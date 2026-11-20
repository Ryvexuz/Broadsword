//! Mode dependency rule check (scaling step S3): which workspace crates may depend on which.
//!
//! Input: the text of `cargo metadata --format-version 1` (the workspace's own packages are the ones whose `source` is
//! `null`; registry crates are ignored), a group map (crate name -> group name) and an allow table (group -> the other
//! groups its crates may depend on). A dependency between two crates of the SAME group is always allowed. Every other
//! edge must be listed in the allow table, so a mode-to-mode edge (render -> physics) is forbidden in the shipped table
//! simply by not being listed.
//!
//! Failure rules (fail closed): a workspace crate missing from the group map is an error, whatever its edges; a group
//! named in the map but absent from the allow table is an error; unreadable metadata is an error. All problems of one
//! kind are listed together, sorted, so the output is stable.
//!
//! Limits: both normal, dev and build dependencies count (cargo metadata does not separate them in `dependencies`
//! here, and a dev-dependency edge still couples the crates at test time). Only edges between workspace crates are
//! checked; an external crate is never in the map.

use crate::json::{self, J};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One forbidden dependency: `from` depends on `to`, and `from_group` may not depend on `to_group`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub from: String,
    pub from_group: String,
    pub to: String,
    pub to_group: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepRuleError {
    /// The metadata text could not be read.
    Metadata(String),
    /// Workspace crates with no entry in the group map.
    Unmapped(Vec<String>),
    /// (crate, group) pairs whose group has no row in the allow table.
    UnknownGroup(Vec<(String, String)>),
    /// Every forbidden edge.
    Forbidden(Vec<Violation>),
}

impl fmt::Display for DepRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DepRuleError::Metadata(m) => write!(f, "cannot read cargo metadata: {m}"),
            DepRuleError::Unmapped(v) => {
                write!(f, "crate(s) missing from the group map: {}", v.join(", "))
            }
            DepRuleError::UnknownGroup(v) => {
                let items: Vec<String> = v
                    .iter()
                    .map(|(c, g)| format!("{c} (group `{g}`)"))
                    .collect();
                write!(
                    f,
                    "group missing from the allow table: {}",
                    items.join(", ")
                )
            }
            DepRuleError::Forbidden(v) => {
                let items: Vec<String> = v
                    .iter()
                    .map(|x| {
                        format!(
                            "{} (group `{}`) depends on {} (group `{}`)",
                            x.from, x.from_group, x.to, x.to_group
                        )
                    })
                    .collect();
                write!(f, "forbidden dependency: {}", items.join("; "))
            }
        }
    }
}

impl std::error::Error for DepRuleError {}

/// The group map and the allow table.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    /// crate name -> group name.
    pub groups: BTreeMap<String, String>,
    /// group name -> the other groups its crates may depend on. A group with an empty set may depend on nothing else.
    pub allow: BTreeMap<String, BTreeSet<String>>,
}

impl Rules {
    pub fn new(groups: &[(&str, &str)], allow: &[(&str, &[&str])]) -> Rules {
        Rules {
            groups: groups
                .iter()
                .map(|(c, g)| (c.to_string(), g.to_string()))
                .collect(),
            allow: allow
                .iter()
                .map(|(g, to)| (g.to_string(), to.iter().map(|t| t.to_string()).collect()))
                .collect(),
        }
    }

    /// The shipped table. `core` depends on nothing else; each mode (`render`, `physics`, `game`) may depend on `core`
    /// only, so no mode depends on another mode; `app` (the binaries) may depend on everything.
    pub fn shipped() -> Rules {
        Rules::new(
            &[("broadsword", "app")],
            &[
                ("core", &[]),
                ("render", &["core"]),
                ("physics", &["core"]),
                ("game", &["core"]),
                ("app", &["core", "render", "physics", "game"]),
            ],
        )
    }
}

/// Workspace crate names and the (from, to) edges between them.
pub type WorkspaceGraph = (BTreeSet<String>, BTreeSet<(String, String)>);

/// The workspace crates of `cargo metadata` output and their dependency edges (target names that are also workspace
/// crates), both sorted and de-duplicated.
pub fn workspace_edges(metadata: &str) -> Result<WorkspaceGraph, DepRuleError> {
    let bad = |m: &str| DepRuleError::Metadata(m.to_string());
    let j = json::parse(metadata).map_err(DepRuleError::Metadata)?;
    let J::Arr(pkgs) = j.get("packages").ok_or_else(|| bad("no `packages`"))? else {
        return Err(bad("`packages` is not an array"));
    };
    let mut local: BTreeSet<String> = BTreeSet::new();
    let mut deps_of: Vec<(String, Vec<String>)> = Vec::new();
    for p in pkgs {
        let name = p
            .get("name")
            .and_then(J::str)
            .ok_or_else(|| bad("a package has no string `name`"))?;
        let source = p
            .get("source")
            .ok_or_else(|| bad(&format!("package {name} has no `source`")))?;
        match source {
            J::Null => {}
            J::Str(_) => continue,
            _ => {
                return Err(bad(&format!(
                    "package {name}: `source` is neither null nor a string"
                )))
            }
        }
        let J::Arr(deps) = p
            .get("dependencies")
            .ok_or_else(|| bad(&format!("package {name} has no `dependencies`")))?
        else {
            return Err(bad(&format!(
                "package {name}: `dependencies` is not an array"
            )));
        };
        let mut names = Vec::new();
        for d in deps {
            let dn = d.get("name").and_then(J::str).ok_or_else(|| {
                bad(&format!(
                    "package {name}: a dependency has no string `name`"
                ))
            })?;
            names.push(dn.to_string());
        }
        local.insert(name.to_string());
        deps_of.push((name.to_string(), names));
    }
    if local.is_empty() {
        return Err(bad("no workspace packages (source null) found"));
    }
    let mut edges = BTreeSet::new();
    for (from, names) in deps_of {
        for to in names {
            if local.contains(&to) {
                edges.insert((from.clone(), to));
            }
        }
    }
    Ok((local, edges))
}

/// Check workspace crates and edges against the rules. Order of errors: unmapped crates, then unknown groups, then
/// forbidden edges.
pub fn check_edges(
    crates: &BTreeSet<String>,
    edges: &BTreeSet<(String, String)>,
    rules: &Rules,
) -> Result<(), DepRuleError> {
    let unmapped: Vec<String> = crates
        .iter()
        .filter(|c| !rules.groups.contains_key(*c))
        .cloned()
        .collect();
    if !unmapped.is_empty() {
        return Err(DepRuleError::Unmapped(unmapped));
    }
    let unknown: Vec<(String, String)> = crates
        .iter()
        .map(|c| (c.clone(), rules.groups[c].clone()))
        .filter(|(_, g)| !rules.allow.contains_key(g))
        .collect();
    if !unknown.is_empty() {
        return Err(DepRuleError::UnknownGroup(unknown));
    }
    let mut bad = Vec::new();
    for (from, to) in edges {
        let (fg, tg) = (&rules.groups[from], &rules.groups[to]);
        if fg != tg && !rules.allow[fg].contains(tg) {
            bad.push(Violation {
                from: from.clone(),
                from_group: fg.clone(),
                to: to.clone(),
                to_group: tg.clone(),
            });
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        bad.sort();
        Err(DepRuleError::Forbidden(bad))
    }
}

/// `cargo metadata` text -> checked against `rules`.
pub fn check(metadata: &str, rules: &Rules) -> Result<(), DepRuleError> {
    let (crates, edges) = workspace_edges(metadata)?;
    check_edges(&crates, &edges, rules)
}
