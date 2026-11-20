//! Affected-package selection (docs/SCALING.md section 2): from `cargo metadata --format-version 1` output (WITH the
//! dependency graph, so not `--no-deps`) and the list of changed files, work out which workspace packages a check must
//! build and test.
//!
//! - D: the workspace members a changed file belongs to. A file belongs to the member whose manifest folder is the longest
//!   prefix of its path, compared component by component and case-sensitively (a case mismatch maps to nothing, which
//!   fails closed to a full check).
//! - R(D): D plus every workspace member that depends on a package of D, transitively, through dependencies of every kind
//!   (normal, build, dev) and for every target platform, also through packages that are not members.
//! - Each selected package carries a reason: the files that changed it, or the dependency chain that reaches a changed one.
//!
//! Fail closed, always: a global file (below), a file that maps to no member, or an empty change list gives
//! `Selection::Full` with one reason per cause. Metadata that cannot be read, parsed or trusted, and a changed path that is
//! not a plain relative path, give `Err`, and the caller then runs the full check.
//!
//! Global files (the complete rule; names compared ASCII-case-insensitively): the workspace manifest (`Cargo.toml` at the
//! workspace root); a file named `Cargo.lock`, `broadsword.toml`, `rust-toolchain` or `rust-toolchain.toml` at any depth;
//! any path with a `.cargo` folder in it.
//!
//! The function runs no cargo and touches no file: JSON in, list out. The metadata must describe the tip of the change
//! (so new edges are in the graph); a file of a package that exists only at the base maps to nothing and gives `Full`.
//! Changed paths are relative to the metadata's `workspace_root`, `/` or `\` separated (as `git diff --name-only -z`
//! gives them when the repo root is the workspace root).
//!
//! Known limit: a folder excluded from the workspace but lying inside a member's folder maps to that member.

use crate::json::{parse, J};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// One selected package and why it was selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub name: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Run the full check; one reason per cause (a global file, an unmapped file, no changed files).
    Full(Vec<String>),
    /// `direct` is D; `affected` is R(D), which includes D. Both sorted by package name.
    Packages {
        direct: Vec<Picked>,
        affected: Vec<Picked>,
    },
}

/// How many changed files a reason names before it says "and N more".
const FILES_IN_REASON: usize = 3;

const GLOBAL_NAMES: [&str; 4] = [
    "cargo.lock",
    "broadsword.toml",
    "rust-toolchain",
    "rust-toolchain.toml",
];

struct Pkg {
    name: String,
    manifest: String,
}

struct Member {
    id: String,
    name: String,
    dir: Vec<String>,
}

/// Select D and R(D) for `changed` from `metadata` (see the module docs).
pub fn select(metadata: &str, changed: &[&str]) -> Result<Selection, String> {
    let meta = parse(metadata).map_err(|e| format!("cannot read cargo metadata: {e}"))?;
    let (members, reverse, packages) = graph(&meta)?;

    // Validate every path before judging any, so a bad path is never hidden behind a Full.
    let mut paths = Vec::new();
    for p in changed {
        paths.push((*p, components(p)?));
    }
    if paths.is_empty() {
        return Ok(Selection::Full(vec!["no changed files were given".into()]));
    }

    let mut full = Vec::new();
    let mut direct: BTreeMap<String, BTreeSet<String>> = BTreeMap::new(); // member id -> changed files
    for (raw, comps) in &paths {
        if is_global(comps) {
            full.push(format!("global file changed: {raw}"));
            continue;
        }
        let owner = members
            .iter()
            .filter(|m| m.dir.len() < comps.len() && comps.starts_with(&m.dir))
            .max_by_key(|m| m.dir.len());
        match owner {
            Some(m) => {
                direct
                    .entry(m.id.clone())
                    .or_default()
                    .insert(comps.join("/"));
            }
            None => full.push(format!("changed file maps to no workspace package: {raw}")),
        }
    }
    if !full.is_empty() {
        return Ok(Selection::Full(full));
    }

    let name_of = |id: &str| packages.get(id).map(|p| p.name.clone()).unwrap_or_default();
    let mut d = Vec::new();
    for (id, files) in &direct {
        let shown: Vec<&str> = files
            .iter()
            .map(String::as_str)
            .take(FILES_IN_REASON)
            .collect();
        let mut reason = format!("changed: {}", shown.join(", "));
        if files.len() > FILES_IN_REASON {
            reason.push_str(&format!(" and {} more", files.len() - FILES_IN_REASON));
        }
        d.push(Picked {
            name: name_of(id),
            reason,
        });
    }

    // Breadth-first over reverse edges from every package of D at once; `via` records the package each one was reached from,
    // so a reason names one shortest chain back to a changed package.
    let mut via: BTreeMap<String, Option<String>> =
        direct.keys().map(|id| (id.clone(), None)).collect();
    let mut todo: VecDeque<String> = direct.keys().cloned().collect();
    while let Some(id) = todo.pop_front() {
        for up in reverse.get(&id).into_iter().flatten() {
            if !via.contains_key(up) {
                via.insert(up.clone(), Some(id.clone()));
                todo.push_back(up.clone());
            }
        }
    }
    let mut affected = d.clone();
    for m in &members {
        let Some(Some(first)) = via.get(&m.id) else {
            continue;
        };
        let mut chain = vec![name_of(first)];
        let mut at = first.clone();
        while let Some(Some(next)) = via.get(&at) {
            chain.push(name_of(next));
            at = next.clone();
        }
        affected.push(Picked {
            name: m.name.clone(),
            reason: format!("depends on {} (changed)", chain.join(" -> ")),
        });
    }
    d.sort_by(|a, b| a.name.cmp(&b.name));
    affected.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Selection::Packages {
        direct: d,
        affected,
    })
}

type Graph = (
    Vec<Member>,
    BTreeMap<String, BTreeSet<String>>,
    BTreeMap<String, Pkg>,
);

/// Members (with their folders relative to the workspace root), reverse edges (package id -> ids that depend on it) and
/// every package by id. Anything missing, mistyped or inconsistent is an error: a graph we cannot trust selects nothing.
fn graph(meta: &J) -> Result<Graph, String> {
    let root = text(meta, "workspace_root", "the metadata")?;
    let root_parts = split_abs(root);
    if root_parts.is_empty() {
        return Err("cargo metadata: `workspace_root` is empty".into());
    }

    let mut packages = BTreeMap::new();
    for p in list(meta, "packages", "the metadata")? {
        let id = text(p, "id", "a package")?;
        let name = text(p, "name", "a package")?;
        let manifest = text(p, "manifest_path", "a package")?;
        if packages
            .insert(
                id.to_string(),
                Pkg {
                    name: name.to_string(),
                    manifest: manifest.to_string(),
                },
            )
            .is_some()
        {
            return Err(format!("cargo metadata lists the package id `{id}` twice"));
        }
    }

    let mut members = Vec::new();
    for m in list(meta, "workspace_members", "the metadata")? {
        let id = m
            .str()
            .ok_or("cargo metadata: a `workspace_members` entry is not a string")?;
        let p = packages.get(id).ok_or_else(|| {
            format!("cargo metadata: workspace member `{id}` is not in `packages`")
        })?;
        if !plain_name(&p.name) {
            return Err(format!(
                "cargo metadata gave the member name `{}`, which is not a plain cargo name",
                p.name
            ));
        }
        let mut parts = split_abs(&p.manifest);
        if !parts
            .last()
            .is_some_and(|f| f.eq_ignore_ascii_case("Cargo.toml"))
        {
            return Err(format!(
                "cargo metadata: the manifest of `{}` is not a Cargo.toml: {}",
                p.name, p.manifest
            ));
        }
        parts.pop();
        let inside = parts.len() >= root_parts.len()
            && parts
                .iter()
                .zip(&root_parts)
                .all(|(a, b)| a.eq_ignore_ascii_case(b));
        if !inside {
            return Err(format!(
                "cargo metadata: the manifest of `{}` is outside the workspace root {root}: {}",
                p.name, p.manifest
            ));
        }
        let dir = parts[root_parts.len()..].to_vec();
        if let Some(other) = members
            .iter()
            .find(|o: &&Member| o.dir == dir || o.id == id)
        {
            return Err(format!(
                "cargo metadata: members `{}` and `{}` share a folder or an id",
                other.name, p.name
            ));
        }
        members.push(Member {
            id: id.to_string(),
            name: p.name.clone(),
            dir,
        });
    }
    if members.is_empty() {
        return Err("cargo metadata lists no workspace members".into());
    }

    let resolve = meta
        .get("resolve")
        .ok_or("cargo metadata has no `resolve` graph")?;
    if *resolve == J::Null {
        return Err("cargo metadata has no dependency graph (`resolve` is null: was it made with --no-deps?)".into());
    }
    let mut reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut seen = BTreeSet::new();
    for node in list(resolve, "nodes", "`resolve`")? {
        let id = text(node, "id", "a `resolve` node")?;
        if !packages.contains_key(id) {
            return Err(format!(
                "cargo metadata: `resolve` node `{id}` is not in `packages`"
            ));
        }
        if !seen.insert(id.to_string()) {
            return Err(format!(
                "cargo metadata: `resolve` lists the node `{id}` twice"
            ));
        }
        // Both forms of a node's dependencies are read and joined: `dependencies` (ids, every kind) and `deps[].pkg`.
        let mut deps = Vec::new();
        let plain = node.get("dependencies");
        let rich = node.get("deps");
        if plain.is_none() && rich.is_none() {
            return Err(format!(
                "cargo metadata: `resolve` node `{id}` has neither `dependencies` nor `deps`"
            ));
        }
        if plain.is_some() {
            for d in list(node, "dependencies", "a `resolve` node")? {
                deps.push(d.str().ok_or_else(|| {
                    format!("cargo metadata: a dependency of `{id}` is not a string")
                })?);
            }
        }
        if rich.is_some() {
            for d in list(node, "deps", "a `resolve` node")? {
                deps.push(text(d, "pkg", "a `deps` entry")?);
            }
        }
        for dep in deps {
            if !packages.contains_key(dep) {
                return Err(format!(
                    "cargo metadata: `{id}` depends on `{dep}`, which is not in `packages`"
                ));
            }
            reverse
                .entry(dep.to_string())
                .or_default()
                .insert(id.to_string());
        }
    }
    if let Some(m) = members.iter().find(|m| !seen.contains(&m.id)) {
        return Err(format!(
            "cargo metadata: workspace member `{}` has no node in `resolve`",
            m.name
        ));
    }
    Ok((members, reverse, packages))
}

fn text<'a>(v: &'a J, key: &str, what: &str) -> Result<&'a str, String> {
    v.get(key)
        .and_then(J::str)
        .ok_or_else(|| format!("cargo metadata: {what} has no string `{key}`"))
}

fn list<'a>(v: &'a J, key: &str, what: &str) -> Result<&'a [J], String> {
    match v.get(key) {
        Some(J::Arr(a)) => Ok(a),
        _ => Err(format!("cargo metadata: {what} has no list `{key}`")),
    }
}

/// An absolute path from cargo split into components (either separator; empty parts dropped, so the root `/` and a
/// trailing separator vanish on both sides of the comparison alike).
fn split_abs(p: &str) -> Vec<String> {
    p.split(['/', '\\'])
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// A changed path as components. Only plain relative paths pass; anything else is a named error.
fn components(p: &str) -> Result<Vec<String>, String> {
    let bad = |why: &str| Err(format!("changed path {p:?} {why}"));
    if p.is_empty() {
        return bad("is empty");
    }
    if p.chars().any(char::is_control) {
        return bad("holds a control character (a line break?)");
    }
    if p.starts_with('"') {
        return bad("is quoted (use `git diff --name-only -z`)");
    }
    if p.starts_with(['/', '\\']) {
        return bad("is absolute");
    }
    if p.contains(':') {
        return bad("holds `:` (a drive letter or a stream name)");
    }
    let mut out = Vec::new();
    for c in p.split(['/', '\\']) {
        match c {
            "" => return bad("has an empty component"),
            "." | ".." => return bad("has a `.` or `..` component"),
            c => out.push(c.to_string()),
        }
    }
    Ok(out)
}

fn is_global(comps: &[String]) -> bool {
    if comps.len() == 1 && comps[0].eq_ignore_ascii_case("Cargo.toml") {
        return true;
    }
    if comps.iter().any(|c| c.eq_ignore_ascii_case(".cargo")) {
        return true;
    }
    comps
        .last()
        .is_some_and(|f| GLOBAL_NAMES.iter().any(|g| f.eq_ignore_ascii_case(g)))
}

/// Same rule as buildsrc::valid_package (a name later passed to `cargo -p`).
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}
