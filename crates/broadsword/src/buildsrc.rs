//! Which checkout last built each package into the shared build folder, and the lock that keeps that note true.
//!
//! The problem: every job builds into ONE build folder (`build_dir`). Two checkouts of the same package (the main
//! checkout and a task's worktree) have the same package name but different paths, so cargo keeps a separate
//! fingerprint for each. Build checkout A, then B, then A again, and cargo sees A as fresh, prints `Finished` and does
//! NOT relink: the binary in the build folder is still B's. That happened once, silently.
//!
//! The rule that closes it: `state/build-sources/<package>` holds the source folder (relative to root, `/` separators,
//! `.` for the main checkout) that last built that package. Before a cargo step, every package of the folder about to be
//! built whose note differs (or is missing: whoever built it is unknown) gets `cargo clean -p <package> --offline`, and the
//! same with `--release` (`CLEAN_PROFILES`: a plain `-p` clean leaves the release profile's artifacts, which cargo then calls
//! fresh), which removes that package's own artifacts, its binary included, and nothing else (dependencies stay). Order matters for
//! crashes: clean first, THEN write the note, THEN build. A crash after the clean leaves a note naming the old source and
//! no artifact (harmless: the next build of the old source cleans again or builds fresh). A crash after the note is
//! written leaves the new source named and nothing stale. Writing the note after the build instead could leave a new
//! binary under an old note, which is exactly the hazard.
//!
//! A `run` job (arbitrary command, may build anything) writes the note `run:<folder>` instead: the same folder part, so a
//! later job in that folder needs no clean, but never exactly `.`, so `install` refuses it. A build, test or check in the
//! same folder first runs `cargo clean -p` once for a package with a `run:` note (the run may have replaced the built exe
//! while cargo's fingerprint stays fresh), then writes the plain folder note. A `run` whose packages cannot be listed notes
//! the package names it can read (the root's and the `[workspace] members`'), and when that list may be incomplete or empty it
//! also leaves a marker FILE for that folder (`unlisted-run.d/<folder>`). `install` is refused while ANY marker exists; a plain
//! build of the same folder removes only its own marker, and only after cleaning every package of that folder, so a build of
//! another project never lifts the refusal.
//!
//! Clean, note and build are one step across processes: the job holds `state/build.lock` (an OS file lock, lock.rs) for
//! all of its cargo COMPILING (a build, test or check releases it once its test executables are copied out of the build
//! folder, and runs the tests from those copies: job.rs; a `run` keeps it to the end). Without it, job 2 could read the note, decide no clean is needed, and then job 1 would build its
//! own source over the folder before job 2's cargo step runs.
//!
//! Binary targets: a package note alone cannot say whose exe is in the build folder, because two packages can build the same
//! bin name (a worktree renames the package but keeps `[[bin]] name = "broadsword"`: its build overwrites `broadsword.exe`
//! while main's package note stays valid). So `state/build-sources/bin.<name>` notes each bin target too (a package name has
//! no `.`, so the names never collide). A package is stale when its own note or any of its bin notes names another folder, is
//! missing or is a `run:` note, and `install` refuses when the bin note for `broadsword` names anything but the main checkout.
//! Limit: a missing bin note is accepted by `install` (builds from before bin notes existed left none; every build since
//! writes one), and a bin note is read and cleaned only through a package of the folder being built.
//!
//! Limit: only the packages of the folder's own workspace are cleaned (`cargo metadata --no-deps`). A path dependency
//! that lives outside the workspace is not a member, so it is neither noted nor cleaned and can still be stale.
//!
//! Deleting: `cargo clean -p` is the ONLY deletion here. It is built in one function (`clean_command`) that always passes
//! `-p <package>` (a bare `cargo clean` would wipe the whole folder), always points cargo at `build_dir` through both
//! `--target-dir` and `CARGO_TARGET_DIR`, and refuses if `build_dir` is not strictly inside root or the source folder is
//! inside it. Cargo deletes only regenerable artifacts below the target directory it is given; it never touches sources.

use crate::config::Config;
use crate::lock::{self, Lock};
use crate::{json, task};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Test-only hook (same rules as `task::wait_for_gate`: scratch only, announced, bounded, can only wait): a job pauses
/// here, HOLDING the build lock, after it has decided which packages need cleaning and before it cleans, so a race test can
/// force the interleaving that the lock exists to prevent.
pub const TEST_SYNC_GATE_ENV: &str = "BROADSWORD_INTERNAL_TEST_BUILD_SYNC_GATE_FILE";

/// The main checkout of the Broadsword project itself, as a source folder.
pub const MAIN_SOURCE: &str = ".";

pub fn notes_dir(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("build-sources")
}

/// A cargo package name: letters, digits, `-` and `_`, never empty. Anything else is refused before it reaches a file name
/// or a command line.
pub fn valid_package(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.starts_with('-') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The source folder of `dir` relative to root, `/` separated; `.` for root itself. `dir` must be inside root.
pub fn rel_source(cfg: &Config, dir: &Path) -> Result<String, String> {
    let rel = dir.strip_prefix(&cfg.root).map_err(|_| format!("{} is not inside root", dir.display()))?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| match c {
            Component::Normal(s) => Ok(s.to_string_lossy().into_owned()),
            Component::CurDir => Ok(String::new()),
            _ => Err(format!("{} has a `..` or prefix part", dir.display())),
        })
        .collect::<Result<_, _>>()?;
    let parts: Vec<&str> = parts.iter().map(String::as_str).filter(|p| !p.is_empty()).collect();
    Ok(if parts.is_empty() { MAIN_SOURCE.to_string() } else { parts.join("/") })
}

/// The recorded source of `package`, `None` if there is no (readable) note.
pub fn read_note(cfg: &Config, package: &str) -> Option<String> {
    if !valid_package(package) {
        return None;
    }
    read_file_note(cfg, package)
}

/// The note file name of a binary target: `bin.<name>`. A package name has no `.`, so it never collides with a package note.
fn bin_note_file(bin: &str) -> String {
    format!("bin.{bin}")
}

/// The recorded source of the binary target `bin` (the exe `<build_dir>/debug/<bin>`), `None` if there is no (readable) note.
/// Two packages can build the same bin name, so the package note alone does not say whose exe is in the build folder.
pub fn read_bin_note(cfg: &Config, bin: &str) -> Option<String> {
    if !valid_package(bin) {
        return None;
    }
    read_file_note(cfg, &bin_note_file(bin))
}

fn read_file_note(cfg: &Config, file: &str) -> Option<String> {
    let text = std::fs::read_to_string(notes_dir(cfg).join(file)).ok()?;
    let line = text.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// Record `source` as the last builder of `package`: temp file, flushed to disk, then renamed over the note (atomic: a
/// reader sees the old note or the new one, never half of one).
pub fn write_note(cfg: &Config, package: &str, source: &str) -> Result<(), String> {
    if !valid_package(package) {
        return Err(format!("`{package}` is not a valid package name"));
    }
    write_file_note(cfg, package, source)
}

/// Record `source` as the last builder of the binary target `bin`, the same atomic way as `write_note`.
pub fn write_bin_note(cfg: &Config, bin: &str, source: &str) -> Result<(), String> {
    if !valid_package(bin) {
        return Err(format!("`{bin}` is not a valid binary target name"));
    }
    write_file_note(cfg, &bin_note_file(bin), source)
}

fn write_file_note(cfg: &Config, file: &str, source: &str) -> Result<(), String> {
    let dir = notes_dir(cfg);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let (path, tmp) = (dir.join(file), dir.join(format!("{file}.tmp")));
    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, format!("{source}\n").as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, &path)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write the build-source note {}: {e}", path.display())
    })
}

// ---- the lock ---------------------------------------------------------------------------

pub const EXIT_FAILED: u8 = 1;
pub const EXIT_BUSY: u8 = 3;

/// Take `state/build.lock`. A job holds it while cargo compiles (see the module comment): a build, test or check releases it
/// once its test executables are copied and runs the tests from the copies; a `run` holds it until its command ends. A second job waits up to `build_lock_wait_seconds`, then is
/// refused (exit 3, nothing built, cleaned or recorded) with a message that says how long it waited and who held it.
pub fn lock(cfg: &Config) -> Result<Lock, (u8, String)> {
    let wait = cfg.build_lock_wait_seconds;
    lock::acquire(&cfg.state_dir.join("build.lock"), Duration::from_secs(wait), &move |owner| {
        let who = owner.map(|p| format!(" (pid {p})")).unwrap_or_default();
        format!("build folder busy: another build, test or check job{who} held the build lock for the whole {wait} s wait (build_lock_wait_seconds); nothing was built, cleaned or recorded. Repeat the call later (the lock frees itself when that job ends)")
    })
    .map_err(|e| (if e.starts_with("build folder busy") { EXIT_BUSY } else { EXIT_FAILED }, e))
}

// ---- package names ----------------------------------------------------------------------

/// The package names in `cargo metadata` output (`--no-deps`: the workspace's own packages only).
pub fn parse_packages(metadata: &str) -> Result<Vec<String>, String> {
    let j = json::parse(metadata).map_err(|e| format!("cannot read cargo metadata output: {e}"))?;
    let json::J::Arr(pkgs) = j.get("packages").ok_or("cargo metadata output has no `packages`")? else {
        return Err("cargo metadata output: `packages` is not a list".into());
    };
    let mut names = Vec::new();
    for p in pkgs {
        let n = p.get("name").and_then(json::J::str).ok_or("cargo metadata output: a package has no name")?;
        if !valid_package(n) {
            return Err(format!("cargo metadata gave the package name `{n}`, which is not a plain cargo name; refusing to use it in a file name or command"));
        }
        if !names.iter().any(|x| x == n) {
            names.push(n.to_string());
        }
    }
    if names.is_empty() {
        return Err("cargo metadata lists no packages in this folder".into());
    }
    Ok(names)
}

/// The binary targets (kind `bin`) of each package in `cargo metadata` output: (package, its bin names), in package order. A
/// bin name is held to the same plain-name rule as a package name (it becomes a note file name). The package list itself is
/// checked by `parse_packages` first (plain names, at least one), so both readers agree on which packages there are.
pub fn parse_bins(metadata: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let names = parse_packages(metadata)?;
    let j = json::parse(metadata).map_err(|e| format!("cannot read cargo metadata output: {e}"))?;
    let Some(json::J::Arr(pkgs)) = j.get("packages") else {
        return Err("cargo metadata output: `packages` is not a list".into());
    };
    let mut out: Vec<(String, Vec<String>)> = names.into_iter().map(|n| (n, Vec::new())).collect();
    for p in pkgs {
        let n = p.get("name").and_then(json::J::str).ok_or("cargo metadata output: a package has no name")?;
        // cargo always lists a package's targets; without them its bins cannot be noted, so that is an error (fail closed)
        let Some(json::J::Arr(targets)) = p.get("targets") else {
            return Err(format!("cargo metadata output: package `{n}` has no `targets` list"));
        };
        let bins = &mut out.iter_mut().find(|(name, _)| name == n).ok_or("cargo metadata output: a package changed between reads")?.1;
        for t in targets {
            let is_bin = match t.get("kind") {
                Some(json::J::Arr(k)) => k.iter().any(|x| x.str() == Some("bin")),
                _ => return Err(format!("cargo metadata output: a target of `{n}` has no `kind` list")),
            };
            if !is_bin {
                continue;
            }
            let b = t.get("name").and_then(json::J::str).ok_or_else(|| format!("cargo metadata output: a binary target of `{n}` has no name"))?;
            if !valid_package(b) {
                return Err(format!("cargo metadata gave the binary target name `{b}` (package `{n}`), which is not a plain cargo name; refusing to use it in a file name"));
            }
            if !bins.iter().any(|x| x == b) {
                bins.push(b.to_string());
            }
        }
    }
    Ok(out)
}

/// THE one place this module (and job.rs's test step) starts a cargo child, so every such child gets the scrubbed environment
/// (`scrub::apply`: no secret-named variable, nothing holding a `.env` value) before the caller adds its own variables.
pub fn cargo(cfg: &Config) -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    crate::scrub::apply(&mut c, cfg);
    c
}

/// The stdout of `cargo metadata --no-deps --format-version 1 --offline` in `dir`, run with the scrubbed environment.
pub fn metadata_json(cfg: &Config, dir: &Path) -> Result<String, String> {
    let out = cargo(cfg)
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot start cargo to list the packages in {}: {e}", dir.display()))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The packages of the folder about to be built with their binary targets (from `metadata_json`, see `parse_bins`).
pub fn bins_of(cfg: &Config, dir: &Path) -> Result<Vec<(String, Vec<String>)>, String> {
    parse_bins(&metadata_json(cfg, dir)?)
}

// ---- cleaning ---------------------------------------------------------------------------

#[cfg(windows)]
fn is_reparse_point(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT: symlinks and junctions
}
#[cfg(not(windows))]
fn is_reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

/// The first of `p` and its parents (up to the root of the drive) that is a symlink, junction or reparse point. Missing
/// parts are fine (nothing to follow).
fn link_in_path(p: &Path) -> Option<PathBuf> {
    p.ancestors().find(|a| std::fs::symlink_metadata(a).is_ok_and(|m| m.file_type().is_symlink() || is_reparse_point(&m))).map(Path::to_path_buf)
}

/// A cargo profile whose artifacts a clean removes. `cargo clean -p <package>` alone removes only the dev profile's: a
/// `--release` build from another checkout stays in `build/release`, and cargo then calls it fresh and runs it (found by the
/// merge queue's perf gate: one worktree's `cargo run --release --example bench` ran another worktree's bench).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Dev,
    Release,
}

/// Every profile a stale package is cleaned in, in this order. Limit: a custom `[profile.<name>]` is not cleaned.
pub const CLEAN_PROFILES: [Profile; 2] = [Profile::Dev, Profile::Release];

/// The one and only place a `cargo clean` is made. Always `-p <package>`, always aimed at `build_dir`, for one `profile` (dev, or
/// `--release`). Refuses (Err, nothing run) if the package name is not a plain cargo name, `build_dir` is not strictly inside
/// root, or the source folder lies inside `build_dir` (cleaning would then be pointed at, or beside, sources).
pub fn clean_command(cfg: &Config, dir: &Path, package: &str, profile: Profile) -> Result<Command, String> {
    if !valid_package(package) {
        return Err(format!("refusing to clean: `{package}` is not a plain package name"));
    }
    let b = &cfg.build_dir;
    if !b.starts_with(&cfg.root) || *b == cfg.root || !b.components().all(|c| !matches!(c, Component::ParentDir)) {
        return Err(format!("refusing to clean: the build folder {} is not strictly inside root", b.display()));
    }
    if let Some(l) = link_in_path(b) {
        return Err(format!("refusing to clean: {} is a symlink, junction or reparse point (on the way to the build folder {}), so cleaning could reach somewhere that is not ours", l.display(), b.display()));
    }
    if dir.starts_with(b) {
        return Err(format!("refusing to clean: the source folder {} is inside the build folder {}", dir.display(), b.display()));
    }
    let mut c = cargo(cfg);
    c.arg("clean").arg("-p").arg(package).arg("--offline").arg("--target-dir").arg(b).current_dir(dir).env("CARGO_TARGET_DIR", b).stdin(Stdio::null());
    if profile == Profile::Release {
        c.arg("--release");
    }
    Ok(c)
}

/// The prefix of a note written by a `run` job: `run:<folder>`. A `run` command may build anything, so nothing vouches for
/// what it left in the build folder (`install` refuses any note but exactly `.`), but it was run in `<folder>`, so a later
/// job in the same folder needs no clean (the binary there is that folder's own, or a stale one only the next build in
/// another folder would have to clean).
pub const RUN_NOTE_PREFIX: &str = "run:";

/// The folder a note names, with the `run:` prefix removed.
pub fn note_folder(note: &str) -> &str {
    note.strip_prefix(RUN_NOTE_PREFIX).unwrap_or(note)
}

/// What `prepare` did, one line each, for the job's output and log.
pub struct Prepared {
    pub lines: Vec<String>,
}

/// Make the shared build folder safe to build `dir` into: for every package of `dir` whose note names another folder (or is
/// missing), clean that package's artifacts, then record `source` for it. A note `run:<source>` (a `run` in this same folder)
/// needs no clean; it is rewritten to plain `source`, because the cargo step that follows rebuilds what is out of date and
/// vouches for it. The caller holds the build lock. Err = nothing may be built (the message says why and what to do).
pub fn prepare(cfg: &Config, dir: &Path, source: &str) -> Result<Prepared, String> {
    prepare_packages(cfg, dir, source, bins_of(cfg, dir)?, false)
}

/// The preparation a `run` job makes before its command (the command may be `cargo test`, which would otherwise test the
/// binary another checkout left in the shared build folder): the same clean as `prepare` for a package whose note names
/// another folder or is missing, then the note becomes `run:<source>`, because the command may build anything and nothing
/// may vouch for the result (`install` refuses). Same folder: no clean, so `run -- cargo build` followed by `run -- <the
/// built exe>` keeps the exe. `run` runs arbitrary commands, so a folder whose packages cannot be listed (a broken
/// Cargo.toml the command may be there to diagnose) is NOT refused: the line says protection was skipped. A `cargo clean`
/// that fails (a locked binary) refuses the job, as it does for build, test and check. The caller holds the lock.
pub fn prepare_for_run(cfg: &Config, dir: &Path, source: &str) -> Result<Vec<String>, String> {
    match bins_of(cfg, dir) {
        Ok(packages) => Ok(prepare_packages(cfg, dir, source, packages, true)?.lines),
        Err(e) => {
            // The packages cannot be listed (a broken Cargo.toml the command may be there to repair), but whatever the command
            // builds must still never be installable: note every package name still findable, else mark the whole folder.
            let wanted = format!("{RUN_NOTE_PREFIX}{source}");
            let (names, complete) = manifest_package_names(dir);
            for n in &names {
                write_note(cfg, n, &wanted)?;
                // a package's default binary carries the package's name (its `[[bin]]` names cannot be read here; a renamed
                // bin is still covered for `install` by the folder marker when the list is incomplete, and by nothing when
                // it is complete: a limit, since cargo cannot build a manifest it cannot read)
                write_bin_note(cfg, n, &wanted)?;
            }
            if !complete {
                write_unlisted_marker(cfg, source)?;
            }
            let how = match (names.is_empty(), complete) {
                (true, _) => "no package name found, so the folder is marked (`install` refuses until a plain build of this folder)".to_string(),
                (false, true) => format!("noted `{wanted}` for {}", names.join(", ")),
                (false, false) => format!("noted `{wanted}` for {}, and the folder is marked because a workspace member could not be read", names.join(", ")),
            };
            Ok(vec![format!("build source: not checked (the stale-binary protection is skipped for this run; {how}): {e}")])
        }
    }
}

/// Directory (inside the notes folder) of the folder markers a `run` leaves when its packages could not be listed: ONE file
/// per source folder, named by the encoded folder, holding that folder's source. The name has a `.`, which no package name
/// has, so it can never collide with a package note.
pub const UNLISTED_DIR: &str = "unlisted-run.d";

fn unlisted_dir(cfg: &Config) -> PathBuf {
    notes_dir(cfg).join(UNLISTED_DIR)
}

/// The marker file of a source folder: every character but letters, digits, `-` and `_` becomes `%XX` (per byte).
fn marker_file(cfg: &Config, source: &str) -> PathBuf {
    let mut n = String::new();
    for b in source.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            n.push(b as char);
        } else {
            n.push_str(&format!("%{b:02X}"));
        }
    }
    unlisted_dir(cfg).join(n)
}

fn write_unlisted_marker(cfg: &Config, source: &str) -> Result<(), String> {
    let dir = unlisted_dir(cfg);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let path = marker_file(cfg, source);
    std::fs::write(&path, format!("{source}\n")).map_err(|e| format!("cannot write the build-source marker {}: {e}", path.display()))
}

/// Every folder marker that exists: (file, the folder it holds). An unreadable marker names the folder `?`.
pub fn unlisted_markers(cfg: &Config) -> Vec<(PathBuf, String)> {
    let Ok(rd) = std::fs::read_dir(unlisted_dir(cfg)) else { return Vec::new() };
    let mut v: Vec<(PathBuf, String)> = rd
        .flatten()
        .map(|e| {
            let f = std::fs::read_to_string(e.path()).ok().and_then(|t| t.lines().next().map(|l| l.trim().to_string())).filter(|l| !l.is_empty());
            (e.path(), f.unwrap_or_else(|| "?".into()))
        })
        .collect();
    v.sort();
    v
}

/// A manifest line without its `#` comment, trimmed.
fn toml_line(line: &str) -> &str {
    line.split('#').next().unwrap_or("").trim()
}

/// The `[package] name` of one manifest text, read line by line (the file may be too broken for cargo). `[package.metadata.*]`
/// and other `[package.*]` tables are not `[package]`.
pub fn package_name_in(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let l = toml_line(line);
        if l.starts_with('[') {
            in_package = l == "[package]";
        } else if in_package {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim() == "name" {
                    let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
                    if valid_package(v) {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }
    None
}

/// The quoted strings of the `[workspace] members = [...]` array of one manifest text (it may span lines).
pub fn workspace_members_in(text: &str) -> Vec<String> {
    let (mut in_ws, mut in_arr) = (false, false);
    let mut out = Vec::new();
    for line in text.lines() {
        let l = toml_line(line);
        let mut rest = l;
        if !in_arr {
            if l.starts_with('[') {
                in_ws = l == "[workspace]";
                continue;
            }
            match l.split_once('=') {
                Some((k, v)) if in_ws && k.trim() == "members" => {
                    in_arr = true;
                    rest = v;
                }
                _ => continue,
            }
        }
        let mut quote: Option<char> = None;
        let mut cur = String::new();
        for c in rest.chars() {
            match quote {
                Some(q) if c == q => {
                    out.push(std::mem::take(&mut cur));
                    quote = None;
                }
                Some(_) => cur.push(c),
                None if c == '"' || c == '\'' => quote = Some(c),
                None if c == ']' => in_arr = false,
                None => {}
            }
        }
    }
    out
}

/// The package names findable in `dir/Cargo.toml` and in the manifests of its `[workspace] members` (a trailing `/*` lists that
/// folder's subfolders), plus whether the list is complete: false if the root manifest is unreadable, nothing was found, or a
/// member's manifest could not be read or has no name. Incomplete means the caller marks the whole folder.
fn manifest_package_names(dir: &Path) -> (Vec<String>, bool) {
    let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) else { return (Vec::new(), false) };
    let mut names: Vec<String> = package_name_in(&text).into_iter().collect();
    let mut complete = true;
    for m in workspace_members_in(&text) {
        let folders: Vec<PathBuf> = match m.strip_suffix("/*") {
            Some(parent) => match std::fs::read_dir(dir.join(parent)) {
                Ok(rd) => rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.path()).collect(),
                Err(_) => {
                    complete = false;
                    continue;
                }
            },
            None if m.contains('*') || m.contains('?') || m.contains('[') => {
                complete = false;
                continue;
            }
            None => vec![dir.join(&m)],
        };
        for f in folders {
            match std::fs::read_to_string(f.join("Cargo.toml")).ok().as_deref().and_then(package_name_in) {
                Some(n) => {
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                None => complete = false,
            }
        }
    }
    if names.is_empty() {
        complete = false;
    }
    (names, complete)
}

/// Packages are noted by name AND by each binary target's name (`bin.<name>` notes): two packages can build the same exe
/// name (a renamed package with `[[bin]] name = "broadsword"`), the later build overwrites the exe, and the package note of
/// the first stays valid. A package is stale when its own note OR any of its bin notes differs from it (another folder, a
/// `run:` note, or missing), and cleaning it (`cargo clean -p`, which removes the exes of ITS bin targets) is what removes a
/// same-named exe another package left. Every note of a cleaned package, bins included, is then written before the build.
fn prepare_packages(cfg: &Config, dir: &Path, source: &str, targets: Vec<(String, Vec<String>)>, as_run: bool) -> Result<Prepared, String> {
    let packages: Vec<String> = targets.iter().map(|(p, _)| p.clone()).collect();
    let bins_of_pkg = |p: &str| -> &[String] { targets.iter().find(|(n, _)| n == p).map(|(_, b)| b.as_slice()).unwrap_or(&[]) };
    // A plain build also cleans a package whose note is `run:<this folder>` (the run may have replaced the built exe while
    // cargo's fingerprint stays fresh), and every package of the folder when a run in THIS folder left a folder marker (only
    // that folder's marker is removed, and only after every package here is cleaned).
    let marker = marker_file(cfg, source);
    let marked = !as_run && marker.exists();
    let is_stale = |n: &Option<String>| -> bool {
        n.as_deref().map(note_folder) != Some(source) || (!as_run && (marked || n.as_deref().is_some_and(|x| x.starts_with(RUN_NOTE_PREFIX))))
    };
    // the note that decides a package's staleness, and the one its `cleaned` line names: its own note when that is stale,
    // else the first of its bin notes that is stale (another folder, a `run:` note where a plain build needs a clean, or
    // missing), else its own note
    let stale: Vec<(String, Option<String>)> = packages
        .iter()
        .filter_map(|p| {
            let own = read_note(cfg, p);
            if is_stale(&own) {
                return Some((p.clone(), own));
            }
            bins_of_pkg(p).iter().map(|b| read_bin_note(cfg, b)).find(|n| is_stale(n)).map(|n| (p.clone(), n))
        })
        .collect();
    task::wait_for_gate(cfg, TEST_SYNC_GATE_ENV, "the build-source sync (holding the build lock)");
    let mut lines = vec![format!("build source: {source} (packages: {})", packages.join(", "))];
    let wanted = if as_run { format!("{RUN_NOTE_PREFIX}{source}") } else { source.to_string() };
    for (p, was) in &stale {
        for profile in CLEAN_PROFILES {
            let flag = if profile == Profile::Release { " --release" } else { "" };
            let out = clean_command(cfg, dir, p, profile)?.output().map_err(|e| format!("cannot start `cargo clean -p {p}{flag}`: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "`cargo clean -p {p}{flag}` failed in {}: {}
nothing was built. The old copy of `{p}` in the build folder could not be removed; on Windows a binary that is running cannot be deleted, so stop whatever runs it (run the installed copy under install_dir, not the one in the build folder) and repeat",
                    dir.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
        write_note(cfg, p, &wanted)?;
        for b in bins_of_pkg(p) {
            write_bin_note(cfg, b, &wanted)?;
        }
        lines.push(match was {
            Some(w) if w.starts_with(RUN_NOTE_PREFIX) && note_folder(w) == source => format!("cleaned `{p}` in the shared build folder (a `run` in this folder may have replaced its binary); its dependencies were kept"),
            Some(w) => format!("cleaned `{p}` in the shared build folder (it was last built from {}); its other packages and all dependencies were kept", note_folder(w)),
            None => format!("cleaned `{p}` in the shared build folder (no record of what built it); its dependencies were kept"),
        });
    }
    if marked {
        std::fs::remove_file(&marker).map_err(|e| format!("cannot remove the build-source marker {}: {e}", marker.display()))?;
    }
    // same folder: no clean; only the note's kind may change (run -> vouched by a build step, or a build -> run), for the
    // package note and each bin note alike (read again: a cleaned package's notes are already `wanted`)
    let same_kind_only = |was: Option<String>| was.as_deref().map(note_folder) == Some(source) && was.as_deref() != Some(wanted.as_str());
    for p in &packages {
        if same_kind_only(read_note(cfg, p)) {
            write_note(cfg, p, &wanted)?;
        }
        for b in bins_of_pkg(p) {
            if same_kind_only(read_bin_note(cfg, b)) {
                write_bin_note(cfg, b, &wanted)?;
            }
        }
    }
    Ok(Prepared { lines })
}

// ---- install ----------------------------------------------------------------------------

/// `broadsword install` may only install code built from the main checkout of Broadsword: the installed copy runs the
/// pause hook and the MCP server for everyone. Reads the same note. Err = the message to print (exit 1).
pub fn check_installable(cfg: &Config) -> Result<(), String> {
    let pkg = env!("CARGO_PKG_NAME");
    let markers = unlisted_markers(cfg);
    if !markers.is_empty() {
        let folders: Vec<String> = markers.iter().map(|(_, f)| format!("`{f}`")).collect();
        return Err(format!(
            "refusing to install: a `run` whose packages could not be listed left a folder marker for {} (in {}), so the `{pkg}` binary in the build folder may be its. Install stays refused while ANY such marker exists; a plain build, test or check of that same folder removes its marker, so repeat that for each folder named, then build the main checkout (`broadsword build {pkg}`, with no --task) and run `install` from the binary it built",
            folders.join(", "),
            unlisted_dir(cfg).display()
        ));
    }
    match read_note(cfg, pkg) {
        Some(s) if s == MAIN_SOURCE => check_bin_note(cfg, pkg),
        Some(run) if run.starts_with(RUN_NOTE_PREFIX) => Err(format!(
            "refusing to install: the last job to touch the `{pkg}` binary in the build folder was a `run` ({run}), and a `run` command may have built anything. Build the main checkout first (`broadsword build {pkg}`, with no --task, run from the main checkout), then run `install` from the binary it built"
        )),
        Some(other) => Err(format!(
            "refusing to install: the `{pkg}` binary in the build folder was last built from `{other}`, not from the main checkout. The installed copy runs the pause hook and the MCP server for everyone, so it must never be a task worktree's experiment. Build the main checkout first (`broadsword build {pkg}`, with no --task, run from the main checkout), then run `install` from the binary it built"
        )),
        None => Err(format!(
            "refusing to install: there is no record of which checkout built the `{pkg}` binary in the build folder ({} is missing), so it could be a worktree's. Build the main checkout through Broadsword first (`broadsword build {pkg}`, with no --task), then run `install` from the binary it built",
            notes_dir(cfg).join(pkg).display()
        )),
    }
}

/// The second half of `check_installable`, once the package note says the main checkout built `pkg`: the bin note says whose
/// exe is in the build folder (another package can build the same bin name over it). A missing bin note is accepted: builds
/// from before bin notes existed left none, and every build since writes one for every bin target it builds. A bin note that
/// exists but cannot be read (or is empty) is refused: something wrote it and nothing says what.
fn check_bin_note(cfg: &Config, pkg: &str) -> Result<(), String> {
    let bin_file = notes_dir(cfg).join(bin_note_file(pkg));
    let bin_note = read_bin_note(cfg, pkg);
    let absent = matches!(std::fs::symlink_metadata(&bin_file), Err(e) if e.kind() == std::io::ErrorKind::NotFound);
    match bin_note {
        Some(s) if s == MAIN_SOURCE => Ok(()),
        None if absent => Ok(()),
        None => Err(format!(
            "refusing to install: the binary-target note {} exists but cannot be read or is empty, so nothing says which checkout built the `{pkg}` exe in the build folder. Build the main checkout first (`broadsword build {pkg}`, with no --task, run from the main checkout), then run `install` from the binary it built",
            bin_file.display()
        )),
        Some(other) => Err(format!(
            "refusing to install: the binary target `{pkg}` in the build folder was last built from `{other}` (a package of that folder builds a binary with this name, or a `run` there may have), not from the main checkout. Its package note says the main checkout built the package, but the exe `{pkg}` is not the main checkout's. Build the main checkout first (`broadsword build {pkg}`, with no --task, run from the main checkout), then run `install` from the binary it built"
        )),
    }
}

/// The note says what built the binary in the build folder; `install` copies the RUNNING binary. They are the same file only
/// if the running one is exactly the main root's `<build_dir>/debug/broadsword.exe`. Both paths are normalised (resolved
/// when they exist, lexically otherwise) and compared without regard to case on Windows.
pub fn check_running_binary(cfg: &Config, running: &Path) -> Result<(), String> {
    let exe = format!("{}{}", env!("CARGO_PKG_NAME"), std::env::consts::EXE_SUFFIX);
    let expected = cfg.build_dir.join("debug").join(&exe);
    let norm = |p: &Path| -> String {
        let p = p.canonicalize().unwrap_or_else(|_| {
            let mut out = PathBuf::new();
            for c in p.components() {
                match c {
                    Component::ParentDir => {
                        out.pop();
                    }
                    other => out.push(other),
                }
            }
            crate::config::normalize(&out)
        });
        let t = p.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
        if cfg!(windows) { t.to_lowercase() } else { t }
    };
    if norm(running) == norm(&expected) {
        Ok(())
    } else {
        Err(format!(
            "refusing to install: the running binary is {}, but the build-source note vouches only for {} (what the main checkout builds). Installing would copy a binary nobody has vouched for. Run `install` from {}",
            running.display(),
            expected.display(),
            expected.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_at(root: &Path) -> Config {
        let text = include_str!("../../../broadsword.toml");
        Config::from_text(text, root).unwrap()
    }

    fn tmp(tag: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-buildsrc-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn package_names_are_plain() {
        for ok in ["a", "broadsword", "crash-sim", "x_1", "A9"] {
            assert!(valid_package(ok), "{ok}");
        }
        for bad in ["", "a/b", "a\\b", "..", "a b", "a.b", "a:b", "-p", &"x".repeat(65)] {
            assert!(!valid_package(bad), "{bad}");
        }
    }

    #[test]
    fn parses_cargo_metadata() {
        let m = r#"{"packages":[{"name":"a","version":"0.1.0","deps":[{"name":"zzz"}]},{"name":"b-c"},{"name":"a"}],"workspace_root":"x","version":1}"#;
        assert_eq!(parse_packages(m).unwrap(), ["a", "b-c"]);
        for bad in ["", "{}", r#"{"packages":[]}"#, r#"{"packages":[{"name":"a/b"}]}"#, r#"{"packages":[{}]}"#, r#"{"packages":3}"#, "nope"] {
            assert!(parse_packages(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn source_is_relative_with_slashes() {
        let cfg = cfg_at(Path::new("r"));
        assert_eq!(rel_source(&cfg, &cfg.root).unwrap(), ".");
        assert_eq!(rel_source(&cfg, &cfg.root.join("projects").join("sim")).unwrap(), "projects/sim");
        assert_eq!(rel_source(&cfg, &cfg.worktrees_dir.join("p").join("t-1")).unwrap(), "worktrees/p/t-1");
        assert!(rel_source(&cfg, Path::new("elsewhere")).is_err());
    }

    #[test]
    fn notes_round_trip_atomically() {
        let d = tmp("notes");
        let mut cfg = cfg_at(&d);
        cfg.state_dir = d.join("state");
        assert_eq!(read_note(&cfg, "p"), None);
        write_note(&cfg, "p", "worktrees/p/t-1").unwrap();
        assert_eq!(read_note(&cfg, "p").as_deref(), Some("worktrees/p/t-1"));
        write_note(&cfg, "p", ".").unwrap();
        assert_eq!(read_note(&cfg, "p").as_deref(), Some("."));
        assert!(!notes_dir(&cfg).join("p.tmp").exists(), "no temp file is left");
        assert!(write_note(&cfg, "../x", ".").is_err());
        assert_eq!(read_note(&cfg, "../x"), None);
        std::fs::write(notes_dir(&cfg).join("empty"), "\n").unwrap();
        assert_eq!(read_note(&cfg, "empty"), None, "an empty note means unknown");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn clean_is_always_per_package_and_aimed_at_the_build_folder() {
        let cfg = cfg_at(Path::new("r"));
        let dir = cfg.worktrees_dir.join("p").join("t");
        let c = clean_command(&cfg, &dir, "my-pkg", Profile::Dev).unwrap();
        let args: Vec<String> = c.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert_eq!(args[0], "clean");
        let at = args.iter().position(|a| a == "-p").expect("-p is always there");
        assert_eq!(args[at + 1], "my-pkg");
        assert!(args.contains(&"--offline".to_string()));
        let t = args.iter().position(|a| a == "--target-dir").expect("the target dir is explicit");
        assert_eq!(Path::new(&args[t + 1]), cfg.build_dir);
        let env: Vec<_> = c.get_envs().filter(|(k, _)| *k == "CARGO_TARGET_DIR").collect();
        assert_eq!(env.len(), 1);
        assert_eq!(Path::new(env[0].1.unwrap()), cfg.build_dir);
        // refusals: no package name, a name that is not plain, a build folder that is not inside root, sources inside it
        for bad in ["", "a b", "../x", "a/b"] {
            assert!(clean_command(&cfg, &dir, bad, Profile::Dev).is_err(), "{bad}");
        }
        let mut wild = cfg.clone();
        wild.build_dir = wild.root.clone();
        assert!(clean_command(&wild, &dir, "p", Profile::Dev).is_err(), "build_dir == root");
        wild.build_dir = PathBuf::from("elsewhere").join("build");
        assert!(clean_command(&wild, &dir, "p", Profile::Dev).is_err(), "build_dir outside root");
        wild.build_dir = cfg.root.join("..").join("build");
        assert!(clean_command(&wild, &dir, "p", Profile::Dev).is_err(), "build_dir with ..");
        assert!(clean_command(&cfg, &cfg.build_dir.join("deep"), "p", Profile::Dev).is_err(), "source inside the build folder");
        assert!(clean_command(&cfg, &cfg.root, "p", Profile::Dev).is_ok(), "the main checkout contains the build folder, which is fine");
    }

    #[test]
    fn a_stale_package_is_cleaned_in_the_release_profile_too_with_the_same_guards() {
        let cfg = cfg_at(Path::new("r"));
        let dir = cfg.worktrees_dir.join("p").join("t");
        let args = |p: Profile| -> Vec<String> { clean_command(&cfg, &dir, "my-pkg", p).unwrap().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        let (dev, rel) = (args(Profile::Dev), args(Profile::Release));
        assert!(!dev.contains(&"--release".to_string()), "{dev:?}");
        assert_eq!(rel.iter().filter(|a| *a == "--release").count(), 1, "{rel:?}");
        // the release clean is the same per-package clean aimed at the build folder, plus the flag
        assert_eq!(rel[..rel.len() - 1], dev[..], "{rel:?} vs {dev:?}");
        let at = rel.iter().position(|a| a == "-p").unwrap();
        assert_eq!(rel[at + 1], "my-pkg");
        assert!(clean_command(&cfg, &dir, "a b", Profile::Release).is_err(), "the release clean keeps the refusals");
        // and every stale package is cleaned in both profiles
        assert_eq!(CLEAN_PROFILES, [Profile::Dev, Profile::Release]);
    }

    #[test]
    fn the_only_clean_in_this_file_is_the_guarded_one() {
        let src = include_str!("buildsrc.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(code.matches(".arg(\"clean\")").count(), 1);
        assert_eq!(code.matches("\"clean\"").count(), 1);
    }

    #[test]
    fn install_requires_the_main_roots_own_binary() {
        let cfg = cfg_at(Path::new("r"));
        let exe = format!("broadsword{}", std::env::consts::EXE_SUFFIX);
        assert!(check_running_binary(&cfg, &cfg.build_dir.join("debug").join(&exe)).is_ok());
        assert!(check_running_binary(&cfg, &cfg.build_dir.join("debug").join("..").join("debug").join(&exe)).is_ok(), "normalised");
        let e = check_running_binary(&cfg, &cfg.worktrees_dir.join("p").join("t").join("build").join("debug").join(&exe)).unwrap_err();
        assert!(e.contains("running binary is") && e.contains("vouches only for"), "{e}");
        assert!(check_running_binary(&cfg, &cfg.build_dir.join("release").join(&exe)).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn clean_refuses_a_build_folder_that_is_a_junction() {
        let d = tmp("junction");
        let d = PathBuf::from(d.canonicalize().unwrap().to_string_lossy().trim_start_matches(r"\?\"));
        let mut cfg = cfg_at(&d);
        let real = d.join("elsewhere");
        std::fs::create_dir_all(&real).unwrap();
        let parent = d.join("b");
        std::fs::create_dir_all(&parent).unwrap();
        let link = parent.join("link");
        let ok = {
            use std::os::windows::process::CommandExt;
            Command::new("cmd").arg("/C").raw_arg(format!("mklink /J \"{}\" \"{}\"", link.display(), real.display())).output().unwrap().status.success()
        };
        assert!(ok, "mklink /J failed");
        cfg.build_dir = link.join("build"); // a parent of the build folder is a junction
        let e = clean_command(&cfg, &d, "p", Profile::Dev).unwrap_err();
        assert!(e.contains("junction"), "{e}");
        cfg.build_dir = link.clone();
        assert!(clean_command(&cfg, &d, "p", Profile::Dev).is_err(), "the build folder itself");
        cfg.build_dir = parent;
        assert!(clean_command(&cfg, &d, "p", Profile::Dev).is_ok());
        let _ = std::fs::remove_dir(&link);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn run_notes_name_a_folder_but_are_never_installable() {
        assert_eq!(note_folder("run:worktrees/p/t"), "worktrees/p/t");
        assert_eq!(note_folder("."), ".");
        let d = tmp("runnote");
        let mut cfg = cfg_at(&d);
        cfg.state_dir = d.join("state");
        let pkg = env!("CARGO_PKG_NAME");
        write_note(&cfg, pkg, "run:.").unwrap();
        let e = check_installable(&cfg).unwrap_err();
        assert!(e.contains("a `run`"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn package_table_is_not_a_package_subtable() {
        let t = "[package.metadata.x]\nname = \"decoy\"\n[package]\nname = \"real\" # c\n";
        assert_eq!(package_name_in(t).as_deref(), Some("real"));
        assert_eq!(package_name_in("[package.metadata.x]\nname = \"decoy\"\n"), None);
        assert_eq!(package_name_in("[dependencies]\nname = \"x\"\n"), None);
        let ws = "[workspace]\nmembers = [\"a\", 'b/*',\n  \"c\"] # x\nresolver = \"2\"\n[other]\nmembers = [\"no\"]\n";
        assert_eq!(workspace_members_in(ws), ["a", "b/*", "c"]);
    }

    #[test]
    fn markers_are_per_folder_and_any_one_refuses_install() {
        let d = tmp("markers");
        let mut cfg = cfg_at(&d);
        cfg.state_dir = d.join("state");
        let pkg = env!("CARGO_PKG_NAME");
        write_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        assert!(check_installable(&cfg).is_ok());
        write_unlisted_marker(&cfg, "worktrees/p/t-1").unwrap();
        write_unlisted_marker(&cfg, ".").unwrap();
        assert_eq!(unlisted_markers(&cfg).len(), 2);
        assert!(marker_file(&cfg, "worktrees/p/t-1") != marker_file(&cfg, "worktrees%2Fp/t-1"));
        let e = check_installable(&cfg).unwrap_err();
        assert!(e.contains("folder marker") && e.contains("worktrees/p/t-1"), "{e}");
        std::fs::remove_file(marker_file(&cfg, ".")).unwrap();
        assert!(check_installable(&cfg).is_err(), "a marker for another folder still refuses");
        std::fs::remove_file(marker_file(&cfg, "worktrees/p/t-1")).unwrap();
        assert!(check_installable(&cfg).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn bin_targets_are_read_from_cargo_metadata() {
        let m = r#"{"packages":[{"name":"a","targets":[{"kind":["lib"],"name":"a"},{"kind":["bin"],"name":"tool"},{"kind":["test"],"name":"t"},{"kind":["bin"],"name":"tool"}]},{"name":"b","targets":[]}],"version":1}"#;
        assert_eq!(parse_bins(m).unwrap(), [("a".to_string(), vec!["tool".to_string()]), ("b".to_string(), vec![])]);
        for bad in [
            r#"{"packages":[{"name":"a","targets":[{"kind":["bin"],"name":"x/y"}]}]}"#,
            r#"{"packages":[{"name":"a","targets":[{"kind":["bin"],"name":"x.y"}]}]}"#,
            r#"{"packages":[{"name":"a","targets":[{"kind":["bin"]}]}]}"#,
            r#"{"packages":[{"name":"a","targets":[{"name":"t"}]}]}"#,
            r#"{"packages":[{"name":"a","targets":3}]}"#,
            r#"{"packages":[{"name":"a"}]}"#,
            r#"{"packages":[]}"#,
            "{}",
            "nope",
        ] {
            assert!(parse_bins(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bin_notes_are_their_own_files_and_never_a_package_note() {
        let d = tmp("binnote");
        let mut cfg = cfg_at(&d);
        cfg.state_dir = d.join("state");
        assert_eq!(read_bin_note(&cfg, "tool"), None);
        write_bin_note(&cfg, "tool", "worktrees/p/t-1").unwrap();
        assert_eq!(read_bin_note(&cfg, "tool").as_deref(), Some("worktrees/p/t-1"));
        assert!(notes_dir(&cfg).join("bin.tool").is_file() && !notes_dir(&cfg).join("bin.tool.tmp").exists());
        assert_eq!(read_note(&cfg, "tool"), None, "a bin note is not the package note of the same name");
        assert_eq!(read_note(&cfg, "bin.tool"), None, "and no package name reaches it");
        assert!(write_bin_note(&cfg, "../x", ".").is_err());
        assert_eq!(read_bin_note(&cfg, "a b"), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Finding 39: the package note says main built `broadsword`, but a worktree package of another name built a bin called
    /// `broadsword` over it. Install must look at the bin note too.
    #[test]
    fn install_refuses_when_another_package_built_the_same_bin_name() {
        let d = tmp("instbin");
        let mut cfg = cfg_at(&d);
        cfg.state_dir = d.join("state");
        let pkg = env!("CARGO_PKG_NAME");
        write_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        assert!(check_installable(&cfg).is_ok(), "a main package note and no bin note (an older build): accepted");
        write_bin_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        assert!(check_installable(&cfg).is_ok());
        write_bin_note(&cfg, pkg, "worktrees/broadsword/t-1").unwrap();
        let e = check_installable(&cfg).unwrap_err();
        assert!(e.contains("binary target") && e.contains("worktrees/broadsword/t-1"), "{e}");
        write_bin_note(&cfg, pkg, "run:.").unwrap();
        assert!(check_installable(&cfg).is_err(), "a run note on the bin refuses too");
        std::fs::write(notes_dir(&cfg).join(format!("bin.{pkg}")), "\n").unwrap();
        let e = check_installable(&cfg).unwrap_err();
        assert!(e.contains("cannot be read or is empty"), "an empty bin note refuses: {e}");
        write_bin_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        assert!(check_installable(&cfg).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn install_rule() {
        let d = tmp("inst");
        let mut cfg = cfg_at(&d);
        cfg.state_dir = d.join("state");
        let pkg = env!("CARGO_PKG_NAME");
        assert!(check_installable(&cfg).unwrap_err().contains("no record"));
        write_note(&cfg, pkg, "worktrees/broadsword/t-1").unwrap();
        let e = check_installable(&cfg).unwrap_err();
        assert!(e.contains("worktrees/broadsword/t-1") && e.contains("main checkout"), "{e}");
        write_note(&cfg, pkg, "projects/broadsword").unwrap();
        assert!(check_installable(&cfg).is_err());
        write_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        assert!(check_installable(&cfg).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
