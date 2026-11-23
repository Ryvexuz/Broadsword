use super::*;

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
pub(super) fn link_in_path(p: &Path) -> Option<PathBuf> {
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

impl Profile {
    /// The folder of this profile's artifacts under `build_dir` (cargo's layout).
    pub fn dir(self) -> &'static str {
        match self {
            Profile::Dev => "debug",
            Profile::Release => "release",
        }
    }
}

/// Every profile a stale package is cleaned in, in this order. Limit: a custom `[profile.<name>]` is not cleaned.
pub const CLEAN_PROFILES: [Profile; 2] = [Profile::Dev, Profile::Release];

/// The checks every write or delete in the build folder passes first: `build_dir` strictly inside root, no `..`, and no
/// symlink, junction or reparse point on the way to it. `what` starts the message ("refusing to clean", ...).
pub(super) fn guard_build_dir(cfg: &Config, what: &str) -> Result<(), String> {
    let b = &cfg.build_dir;
    if !b.starts_with(&cfg.root) || *b == cfg.root || !b.components().all(|c| !matches!(c, Component::ParentDir)) {
        return Err(format!("{what}: the build folder {} is not strictly inside root", b.display()));
    }
    if let Some(l) = link_in_path(b) {
        return Err(format!("{what}: {} is a symlink, junction or reparse point (on the way to the build folder {}), so cleaning could reach somewhere that is not ours", l.display(), b.display()));
    }
    Ok(())
}

/// The one and only place a `cargo clean` is made. Always `-p <package>`, always aimed at `build_dir`, for one `profile` (dev, or
/// `--release`). Refuses (Err, nothing run) if the package name is not a plain cargo name, `build_dir` is not strictly inside
/// root, or the source folder lies inside `build_dir` (cleaning would then be pointed at, or beside, sources).
pub fn clean_command(cfg: &Config, dir: &Path, package: &str, profile: Profile) -> Result<Command, String> {
    if !valid_package(package) {
        return Err(format!("refusing to clean: `{package}` is not a plain package name"));
    }
    let b = &cfg.build_dir;
    guard_build_dir(cfg, "refusing to clean")?;
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

