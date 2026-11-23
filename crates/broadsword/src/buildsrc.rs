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
//! A `run` also invalidates the `broadsword` bin note regardless of its folder's packages: an arbitrary script may build
//! another checkout into that same exe path. An explicit `--manifest-path` must name the job folder's own Cargo.toml in a
//! literal cargo command, otherwise the run is refused before changing notes. Cargo's package/manifest pairs are logged
//! when metadata succeeds; an untraceable run is never evidence that the install binary came from main.
//!
//! Limit: only the packages of the folder's own workspace are cleaned (`cargo metadata --no-deps`). A path dependency
//! that lives outside the workspace is not a member, so it is neither noted nor cleaned and can still be stale.
//!
//! Main's binaries: THE stable place is `<build_dir>/main.bins/<project>/<debug|release>/<bin><exe>` (`kept_exe`; `<project>` is
//! the project name, `broadsword` for the root, so two projects with the same bin name never share a copy. Found 2026-11-20: a
//! worktree's bench build cleaned `crash-sim` and deleted `build/release/crash-sim.exe` that Lazlo had just built from
//! projects/crash-sim; render's drive tests then found no binary). People and other projects' tests run main's binaries from
//! there, NEVER from cargo's own `<build_dir>/<profile>/`: that is a cache which any checkout's build may overwrite or clean, and
//! after another folder's clean it holds only what that job built (a binary it did not build is missing, and a reader fails
//! loudly; this module never copies anything into it, so main's kept copy never sits there under another folder's bin note).
//! A file beside each
//! kept exe (`<exe>.source`) holds the build-source note it was kept under (`projects/<p>`, `.`, or a `run:` note of one).
//! It is written in exactly two places, both under the build lock and both from an exe whose note names the main checkout
//! (so a worktree's exe never lands there), and both only from a SUCCESSFUL main step:
//! - after every successful cargo step or `run` command of the main checkout itself (`refresh_main_bins`, called by job.rs
//!   after a build or run step and after the test step's compile), for ONLY the exes that step produced: job.rs records the
//!   fingerprint (length and modified time) of every bin exe in cargo's folder right before the step (`snapshot_main_bins`,
//!   once per step, the one record both refresh sites use), and an exe whose fingerprint did not change during the step
//!   (cached from an earlier step, maybe a FAILED one: `run -- cargo build --keep-going` can link bin A while bin B fails) is
//!   not copied, and its kept copy and note stay as they are. So main.bins holds main's newest build of every bin a successful
//!   step built;
//! - before a job of another folder cleans the package (`keep_main_bins` in `prepare_packages`), for the main checkout of EVERY
//!   project that has a bin with the same exe file name, in any package (`keep_mains_sharing_exe_paths` over
//!   `mains_with_packages`: the shared thing is the exe PATH `<profile>/<bin><exe>`, so a worktree of project B, with the same
//!   package name or only the same bin name, can remove or overwrite project A's main exe; a package-level clean is found
//!   through its bins; if any project's packages cannot be listed the clean is refused), ONLY for a copy marked stale
//!   (below) whose marker names exactly the exe still in cargo's folder (its length and modified time): that exe is the
//!   successful build the refresh could not copy. Any other exe there (a failed or partial main build's leftovers, whose bin
//!   note prepare already set to main) is never copied: main.bins keeps the last good copy.
//!
//! In use: on Windows a kept copy that is running (Lazlo drives it) cannot be replaced. After a successful main step that does
//! NOT fail the job: the old copy stays, still named by its own `.source` note, a marker `<exe>.stale` beside it says
//! `stale: <note of the new build>, in use at <UTC>` and, on its second line, `built: <length> bytes, modified <time>` of the
//! new build in cargo's folder; the job prints a loud `main.bins: <bin> NOT refreshed (in use)` line, and the next main step
//! that finds the copy free refreshes it and removes the marker (the marker vouches for that exe, so it is copied even when the
//! next step does not rebuild it). Every other copy error (disk full, a length mismatch) still
//! fails the job, after its summary block. Before another folder's clean a stale copy that is still in use refuses that job
//! (nothing built; the package in question is not cleaned, but packages processed earlier in the same job may already be): the
//! clean would delete main's newer build from cargo's folder while main.bins holds only the older one. A copy is "current" by
//! its BYTES alone: when they equal the built exe's, only the `.source` note is rewritten and the binary is never replaced.
//!
//! Known limit (follow-up): nothing reads the `.stale` marker yet (the render lookup task t-20261120-202643-2 will).
//!
//! Limits: a main `build`/`check`/`run` that FAILS refreshes nothing (main.bins keeps the last good build, the cache may hold the
//! failed attempt's leftovers); a `run` of the main checkout counts (its command may build anything there, so its kept note is
//! `run:<folder>`, never `.`); a bin that a failed step linked and the next successful step found fresh (so did not relink)
//! stays out of main.bins until a successful step writes it again (a change to its source, or a clean), and so does the exe of
//! a successful step whose refresh FAILED (that job fails; no marker vouches for the exe); before another folder's clean a
//! copy whose bytes are current keeps its note (only a step that wrote the exe renames its builder); a `run` whose
//! packages cannot be listed (before or after its command) refreshes nothing (its line says so), and a main exe
//! that no refresh copied (that run's, or one built before main.bins existed) is not kept before another folder's clean;
//! copies in the older layout `main.bins/<profile>/` are no longer written or read; cargo-sweep never touches main.bins (it
//! only deletes hash-named files in folders that hold a `.fingerprint`: test in cleanup.rs).
//!
//! Deleting: `cargo clean -p` is the ONLY deletion of build output here. It is built in one function (`clean_command`) that always passes
//! `-p <package>` (a bare `cargo clean` would wipe the whole folder), always points cargo at `build_dir` through both
//! `--target-dir` and `CARGO_TARGET_DIR`, and refuses if `build_dir` is not strictly inside root or the source folder is
//! inside it. Cargo deletes only regenerable artifacts below the target directory it is given; it never touches sources. The
//! kept copies replace their own older versions (a rename over the file, and the `.source` note removed before a new copy is
//! written), inside `<build_dir>/main.bins` only, behind `guard_kept_files`: the build-folder guard, plus no symlink, junction
//! or reparse point at `main.bins`, its profile folder or any file it writes (`kept_files`: the exe, `.source`, `.stale` and
//! each one's temp name), and every existing part resolving inside `build_dir`. A temp file is always created NEW
//! (`create_fresh`: an entry of that name is removed first, then `create_new`), so a hard link planted there is never written
//! through. Limit (accepted residual): a link created between the check and the write, by someone already writing in build/.

use crate::config::Config;
use crate::{json, scrub, task};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

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

mod buildlock;
pub use buildlock::{lock, run_proof, EXIT_BUSY, EXIT_FAILED};

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
fn guard_build_dir(cfg: &Config, what: &str) -> Result<(), String> {
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

mod kept;
pub use kept::{kept_exe, kept_project, main_folder, read_kept_note, stale_marker};
#[cfg(test)]
pub use kept::KEEP_DIR;
#[cfg(test)]
use kept::copy_tmp;
use kept::{atomic_tmp, built_exe, copy_atomic, create_fresh, fingerprint, guard_kept_files, in_use, kept_note, vouched_by_marker, CopyFail, Keep, STALE_BUILT};

mod mainbins;
pub use mainbins::{refresh_main_bins, snapshot_main_bins, StepBins};
use mainbins::{keep_mains_sharing_exe_paths, mains_with_packages, PackageBins};
#[cfg(test)]
use mainbins::keep_main_bins;

/// What `prepare` did, one line each, for the job's output and log.
pub struct Prepared {
    pub lines: Vec<String>,
}

/// Make the shared build folder safe to build `dir` into: for every package of `dir` whose note names another folder (or is
/// missing), clean that package's artifacts, then record `source` for it. A note `run:<source>` also requires cleaning:
/// an arbitrary command may have overwritten the exe without changing Cargo's fingerprint. The caller holds the build
/// lock. Err = nothing may be built (the message says why and what to do).
pub fn prepare(cfg: &Config, dir: &Path, source: &str) -> Result<Prepared, String> {
    let (targets, manifests) = targets_with_manifests(cfg, dir)?;
    let mut prepared = prepare_packages(cfg, dir, source, targets, false)?;
    prepared.lines.extend(manifests);
    Ok(prepared)
}

/// Read package identities from Cargo, not from a second TOML parser. These lines go into the durable job log along with
/// the source note. A package outside the job's checkout must not acquire that checkout's note.
fn targets_with_manifests(cfg: &Config, dir: &Path) -> Result<(PackageBins, Vec<String>), String> {
    let metadata = metadata_json(cfg, dir)?;
    let targets = parse_bins(&metadata)?;
    let parsed =
        json::parse(&metadata).map_err(|e| format!("cannot read cargo metadata output: {e}"))?;
    let Some(json::J::Arr(packages)) = parsed.get("packages") else {
        return Err("cargo metadata output: `packages` is not a list".into());
    };
    let checkout = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve job checkout {}: {e}", dir.display()))?;
    let mut lines = Vec::new();
    for package in packages {
        let name = package
            .get("name")
            .and_then(json::J::str)
            .ok_or("cargo metadata output: a package has no name")?;
        let manifest = package
            .get("manifest_path")
            .and_then(json::J::str)
            .ok_or("cargo metadata output: a package has no manifest_path")?;
        if manifest.is_empty()
            || manifest.chars().any(char::is_control)
            || !Path::new(manifest).is_absolute()
        {
            return Err(format!(
                "cargo metadata output: invalid manifest_path for package `{name}`"
            ));
        }
        let path = Path::new(manifest)
            .canonicalize()
            .map_err(|e| format!("cannot resolve manifest for package `{name}`: {e}"))?;
        if !path.starts_with(&checkout) {
            return Err(format!(
                "refusing to note package `{name}` as this checkout's build: manifest {} is outside {}",
                path.display(),
                checkout.display()
            ));
        }
        lines.push(format!("build manifest: {} (package: {name})", path.display()));
    }
    Ok((targets, lines))
}

/// Only a literal, single cargo command can prove which manifest a shell's `--manifest-path` names. Do not try to evaluate
/// shell expansion, scripts, command substitution or a changed working directory. Ambiguous commands are refused, rather
/// than checking one path and letting the shell build another. Backslashes are literal only on Windows.
fn guard_run_manifest(dir: &Path, command: &str) -> Result<(), String> {
    if !command.contains("--manifest-path") {
        return Ok(());
    }
    let refuse = || {
        "refusing run: cannot attribute --manifest-path to the job checkout; use a literal cargo command naming this checkout's Cargo.toml, or build through the matching Broadsword project/task".to_string()
    };
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    for c in command.chars() {
        if c.is_control()
            || "&|;<>$`%!^()*?[]{}~#".contains(c)
            || (!cfg!(windows) && c == '\\')
            || (cfg!(windows) && c == '\'')
        {
            return Err(refuse());
        }
        if Some(c) == quote {
            quote = None;
        } else if quote.is_none() && (c == '"' || c == '\'') {
            quote = Some(c);
        } else if quote.is_none() && c.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if quote.is_some() {
        return Err(refuse());
    }
    if !word.is_empty() {
        words.push(word);
    }
    if !matches!(words.first().map(String::as_str), Some("cargo" | "cargo.exe")) {
        return Err(refuse());
    }
    let mut manifest = None;
    let mut args = words.iter().skip(1);
    while let Some(arg) = args.next() {
        let value = if arg == "--manifest-path" {
            Some(args.next().ok_or_else(refuse)?.as_str())
        } else {
            arg.strip_prefix("--manifest-path=")
        };
        if let Some(value) = value {
            if value.is_empty() || value.starts_with('-') || manifest.replace(value).is_some() {
                return Err(refuse());
            }
        } else if arg.contains("--manifest-path") {
            return Err(refuse());
        }
    }
    let manifest = manifest.ok_or_else(refuse)?;
    let path = Path::new(manifest);
    if !path.is_absolute()
        && path
            .components()
            .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
    {
        return Err(refuse());
    }
    let requested = dir.join(manifest).canonicalize().map_err(|e| {
        format!("refusing run: cannot resolve --manifest-path {manifest:?}: {e}")
    })?;
    let own = dir
        .join("Cargo.toml")
        .canonicalize()
        .map_err(|e| format!("refusing run: cannot resolve the job's Cargo.toml: {e}"))?;
    if requested != own {
        return Err(format!(
            "refusing run: --manifest-path names {}, not this job checkout's manifest {}; nothing was built or noted as main",
            requested.display(),
            own.display()
        ));
    }
    Ok(())
}

/// The preparation a `run` job makes before its command (the command may be `cargo test`, which would otherwise test the
/// binary another checkout left in the shared build folder): the same clean as `prepare` for a package whose note names
/// another folder or is missing, then the note becomes `run:<source>`, because the command may build anything and nothing
/// may vouch for the result (`install` refuses). Same folder: no clean, so `run -- cargo build` followed by `run -- <the
/// built exe>` keeps the exe. `run` runs arbitrary commands, so a folder whose packages cannot be listed (a broken
/// Cargo.toml the command may be there to diagnose) is NOT refused: the line says protection was skipped. A `cargo clean`
/// that fails (a locked binary) refuses the job, as it does for build, test and check. An explicit foreign or ambiguous
/// `--manifest-path` refuses the command before any notes change. Every accepted run invalidates the install binary's note,
/// even when this folder has no package named `broadsword`: a shell command or script may overwrite that shared exe.
/// The caller holds the lock.
pub fn prepare_for_run(
    cfg: &Config,
    dir: &Path,
    source: &str,
    command: &str,
) -> Result<Vec<String>, String> {
    guard_run_manifest(dir, command)?;
    let mut lines = match targets_with_manifests(cfg, dir) {
        Ok((packages, manifests)) => {
            let mut lines = prepare_packages(cfg, dir, source, packages, true)?.lines;
            lines.extend(manifests);
            lines
        }
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
            vec![format!("build source: not checked (the stale-binary protection is skipped for this run; {how}): {e}")]
        }
    };
    // Last, after preservation/cleaning, but before the caller can spawn the command. This is keyed by the shared exe,
    // not by the package the user selected. A later main build must clean it even if Cargo would call its fingerprint fresh.
    write_bin_note(cfg, env!("CARGO_PKG_NAME"), &format!("{RUN_NOTE_PREFIX}{source}"))?;
    lines.push(
        "build source: install provenance invalidated; an arbitrary run may build packages outside its job checkout".into(),
    );
    Ok(lines)
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
    // main's binaries: before a job of ANOTHER folder cleans a package, a successful main build that main.bins could not take yet
    // (its copy was in use) is kept (see `keep_main_bins`), for EVERY project whose main checkout has a bin with the same exe
    // name (`keep_mains_sharing_exe_paths`: an exe path is shared whichever package builds it, so another project's main exe can
    // be the one this clean removes or the build after it overwrites). Nothing is
    // ever copied back into cargo's folder after the clean: it then holds only what this job builds.
    let mains = if stale.is_empty() { Vec::new() } else { mains_with_packages(cfg)? };
    for (p, was) in &stale {
        lines.extend(keep_mains_sharing_exe_paths(cfg, source, &mains, p, bins_of_pkg(p))?);
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
    fn explicit_run_manifest_must_be_the_job_manifest() {
        let d = tmp("run-manifest");
        std::fs::write(d.join("Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(d.join("copy with spaces")).unwrap();
        std::fs::write(d.join("copy with spaces/Cargo.toml"), "").unwrap();
        for command in [
            "cargo build --manifest-path Cargo.toml",
            "cargo build --manifest-path=\"./Cargo.toml\"",
            "cargo build \"--manifest-path\" \"Cargo.toml\"",
        ] {
            assert!(guard_run_manifest(&d, command).is_ok(), "{command}");
        }
        for command in [
            "cargo build --manifest-path \"copy with spaces/Cargo.toml\"",
            "cargo build --manifest-path=copy/Cargo.toml",
            "cargo build --manifest-path",
            "cargo build --manifest-path=",
            "cargo build --manifest-path \"\"",
            "cargo build --manifest-path Cargo.toml --manifest-path Cargo.toml",
            "cargo build --manifest-path Cargo.toml && cargo build",
            "cd copy && cargo build --manifest-path Cargo.toml",
            "cargo build --manifest-path \"$MANIFEST\"",
            "cargo build --manifest-path \"%MANIFEST%\"",
            "cargo build --manifest-path Cargo.toml *",
            "cargo build --manifest-path Cargo.toml\r",
            "cargo build --manifest-path Cargo.toml\n",
            "cargo build --manifest-path Cargo.toml\0",
            "cargo build --manifest-path \"Cargo.toml",
            "script --manifest-path Cargo.toml",
        ] {
            let error = guard_run_manifest(&d, command).unwrap_err();
            assert!(
                error.contains("refusing run") && error.contains("manifest"),
                "{command:?}: {error}"
            );
        }
        let absolute = d.join("copy with spaces/Cargo.toml").canonicalize().unwrap();
        assert!(guard_run_manifest(
            &d,
            &format!("cargo build --manifest-path \"{}\"", absolute.display())
        )
        .is_err());
        if cfg!(windows) {
            assert!(guard_run_manifest(&d, r"cargo build --manifest-path .\Cargo.toml").is_ok());
            assert!(
                guard_run_manifest(&d, r"cargo build --manifest-path copy\Cargo.toml").is_err()
            );
            assert!(guard_run_manifest(&d, r"cargo build --manifest-path C:Cargo.toml").is_err());
            assert!(guard_run_manifest(&d, r"cargo build --manifest-path \Cargo.toml").is_err());
        }
        let huge = format!(
            "cargo build {} --manifest-path Cargo.toml --manifest-path Cargo.toml",
            "x".repeat(1024 * 1024)
        );
        assert!(
            guard_run_manifest(&d, &huge).is_err(),
            "large commands must not bypass duplicate detection"
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    /// With the shared-exe invalidation removed, this succeeds in preparing the unrelated library but check_installable
    /// incorrectly returns Ok: both Broadsword notes still say main. No command needs to run to exercise the guard.
    #[test]
    fn every_run_invalidates_install_at_the_preparation_entry_point() {
        let d = tmp("run-install-entry");
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname = \"unrelated\"\nversion = \"0.0.0\"\n[workspace]\nmembers = [\"member\"]\n",
        )
        .unwrap();
        std::fs::write(d.join("src/lib.rs"), "").unwrap();
        std::fs::create_dir_all(d.join("member/src")).unwrap();
        std::fs::write(
            d.join("member/Cargo.toml"),
            "[package]\nname = \"unrelated-helper\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        std::fs::write(d.join("member/src/lib.rs"), "").unwrap();
        let cfg = cfg_at(&d);
        let pkg = env!("CARGO_PKG_NAME");
        write_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        write_bin_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        write_note(&cfg, "unrelated", MAIN_SOURCE).unwrap();
        write_note(&cfg, "unrelated-helper", MAIN_SOURCE).unwrap();
        assert!(check_installable(&cfg).is_ok());
        let lines = prepare_for_run(&cfg, &d, MAIN_SOURCE, "some-script").unwrap();
        assert!(
            lines.iter().any(|l| {
                l.contains("build manifest:")
                    && l.contains("Cargo.toml")
                    && l.contains("package: unrelated")
            }),
            "{lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("build manifest:") && l.contains("package: unrelated-helper")),
            "every workspace package has a manifest record: {lines:?}"
        );
        assert_eq!(read_note(&cfg, pkg).as_deref(), Some(MAIN_SOURCE));
        assert_eq!(read_bin_note(&cfg, pkg).as_deref(), Some("run:."));
        assert!(check_installable(&cfg).unwrap_err().contains("binary target"));
        // A failure writing the invalidation must fail setup, not let the caller start its command.
        std::fs::remove_file(notes_dir(&cfg).join(bin_note_file(pkg))).unwrap();
        std::fs::create_dir(notes_dir(&cfg).join(bin_note_file(pkg))).unwrap();
        let error = prepare_for_run(&cfg, &d, MAIN_SOURCE, "some-script").unwrap_err();
        assert!(error.contains("cannot write the build-source note"), "{error}");
        assert!(!notes_dir(&cfg).join(format!("bin.{pkg}.tmp")).exists());
        assert!(check_installable(&cfg).is_err());
        // The metadata-failure fallback must invalidate the same shared exe too.
        std::fs::remove_dir(notes_dir(&cfg).join(bin_note_file(pkg))).unwrap();
        write_bin_note(&cfg, pkg, MAIN_SOURCE).unwrap();
        std::fs::write(d.join("Cargo.toml"), "not a manifest").unwrap();
        let lines = prepare_for_run(&cfg, &d, MAIN_SOURCE, "repair-script").unwrap();
        assert!(lines.iter().any(|l| l.contains("not checked")), "{lines:?}");
        assert_eq!(read_bin_note(&cfg, pkg).as_deref(), Some("run:."));
        assert!(check_installable(&cfg).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn copy_atomic_keeps_the_source_permissions() {
        fn copy(f: &Path, t: &Path) {
            if let Err(e) = copy_atomic(f, t) {
                panic!("copy_atomic failed: {e}");
            }
        }
        let d = tmp("copyperm");
        let (from, to) = (d.join("from.exe"), d.join("to.exe"));
        std::fs::write(&from, b"exe bytes").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&from, std::fs::Permissions::from_mode(0o755)).unwrap();
            copy(&from, &to);
            assert_eq!(std::fs::metadata(&to).unwrap().permissions().mode() & 0o777, 0o755, "the execute bits are kept");
        }
        #[cfg(not(unix))]
        {
            let mut p = std::fs::metadata(&from).unwrap().permissions();
            p.set_readonly(true);
            std::fs::set_permissions(&from, p).unwrap();
            copy(&from, &to);
            assert!(std::fs::metadata(&to).unwrap().permissions().readonly(), "the read-only flag is kept");
            // a read-only kept copy must still be replaceable by the next refresh
            copy(&from, &to);
            assert_eq!(std::fs::read(&to).unwrap(), b"exe bytes");
            // make both writable again (the plain permissions of a new file) so the folder can be removed
            let plain = d.join("plain");
            std::fs::write(&plain, b"").unwrap();
            let plain = std::fs::metadata(&plain).unwrap().permissions();
            for f in [&from, &to] {
                std::fs::set_permissions(f, plain.clone()).unwrap();
            }
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Identical bytes with a different note, the kept copy held open (in use): success, the note updated, the binary untouched.
    #[test]
    fn a_current_copy_in_use_only_gets_its_note_updated() {
        let d = tmp("samebytes");
        let cfg = cfg_at(&d.canonicalize().unwrap());
        let (from, to) = (built_exe(&cfg, Profile::Release, "probe"), kept_exe(&cfg, "p", Profile::Release, "probe"));
        for p in [&from, &to] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"main's exe").unwrap();
        }
        std::fs::write(kept_note(&to), "run:projects/p\n").unwrap();
        write_bin_note(&cfg, "probe", "projects/p").unwrap();
        let modified = std::fs::metadata(&to).unwrap().modified().unwrap();
        // a program running the copy: open for writing with sharing only for reads, so it can be neither replaced nor deleted
        let mut hold = std::fs::OpenOptions::new();
        hold.write(true);
        #[cfg(windows)]
        std::os::windows::fs::OpenOptionsExt::share_mode(&mut hold, 1);
        let _running = hold.open(&to).unwrap();
        // before another folder's clean nothing is renamed: the copy and its note stay as the last main step left them
        let r = keep_main_bins(&cfg, "projects/p", "pkg", &["probe".to_string()], "in the test", Keep::BeforeForeignClean);
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(read_kept_note(&cfg, "p", Profile::Release, "probe").as_deref(), Some("run:projects/p"));
        // after a main step that wrote the exe (no record of it before the step: it appeared), only the note is updated
        let step = StepBins { before: Ok(Vec::new()) };
        let r = keep_main_bins(&cfg, "projects/p", "pkg", &["probe".to_string()], "in the test", Keep::AfterMainStep(&step));
        assert!(r.is_ok(), "{r:?}");
        assert_eq!(read_kept_note(&cfg, "p", Profile::Release, "probe").as_deref(), Some("projects/p"));
        assert_eq!(std::fs::read(&to).unwrap(), b"main's exe");
        assert_eq!(std::fs::metadata(&to).unwrap().modified().unwrap(), modified, "the binary was not rewritten");
        assert!(!stale_marker(&to).exists());
        drop(_running);
        let _ = std::fs::remove_dir_all(&d);
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
    fn the_main_folder_of_a_source_is_an_exact_allow_list() {
        let cfg = cfg_at(Path::new("r"));
        assert_eq!(main_folder(&cfg, ".").as_deref(), Some("."));
        assert_eq!(main_folder(&cfg, "projects/crash-sim").as_deref(), Some("projects/crash-sim"));
        assert_eq!(main_folder(&cfg, "worktrees/crash-sim/t-1").as_deref(), Some("projects/crash-sim"));
        assert_eq!(main_folder(&cfg, "worktrees/broadsword/t-1").as_deref(), Some("."), "Broadsword's own main checkout is the root");
        for bad in ["", "projects", "projects/", "projects/a/b", "projects/..", "worktrees/p", "worktrees/p/", "worktrees/p/t/x", "worktrees/../t", "scratch/x", "run:projects/p", "elsewhere/p/t"] {
            assert_eq!(main_folder(&cfg, bad), None, "{bad}");
        }
        assert_eq!(Profile::Dev.dir(), "debug");
        assert_eq!(Profile::Release.dir(), "release");
        let k = kept_exe(&cfg, "crash-sim", Profile::Release, "crash-sim");
        assert_eq!(k, cfg.build_dir.join(KEEP_DIR).join("crash-sim").join("release").join(format!("crash-sim{}", std::env::consts::EXE_SUFFIX)));
    }

    /// Review q-20261121-011535 (minor at kept_exe): two projects with the same bin name must never share one kept copy. The
    /// kept path is keyed by the project, which comes from the main folder in one place (`kept_project`).
    #[test]
    fn kept_copies_are_keyed_by_project() {
        let cfg = cfg_at(Path::new("r"));
        assert_eq!(kept_project(&cfg, ".").as_deref(), Some("broadsword"));
        assert_eq!(kept_project(&cfg, "projects/crash-sim").as_deref(), Some("crash-sim"));
        assert_eq!(kept_project(&cfg, "projects/other").as_deref(), Some("other"));
        for bad in ["", "projects", "projects/", "projects/a/b", "projects/..", "projects/.", "projects/a\\b", "projects/c:x", "projects/broadsword", "projects/Broadsword", "worktrees/p/t", "run:projects/p", "elsewhere/p"] {
            assert_eq!(kept_project(&cfg, bad), None, "{bad}");
        }
        // the same bin of two projects (and of the root) lands in three different places
        let paths: Vec<PathBuf> = ["broadsword", "crash-sim", "other"].iter().map(|p| kept_exe(&cfg, p, Profile::Dev, "tool")).collect();
        assert!(paths[0] != paths[1] && paths[1] != paths[2] && paths[0] != paths[2], "{paths:?}");
        // and every main folder main_folder gives maps to its own project
        for (source, project) in [(".", "broadsword"), ("worktrees/broadsword/t-1", "broadsword"), ("projects/crash-sim", "crash-sim"), ("worktrees/crash-sim/t-1", "crash-sim")] {
            assert_eq!(main_folder(&cfg, source).and_then(|m| kept_project(&cfg, &m)).as_deref(), Some(project), "{source}");
        }
    }

    /// Review q-20261121-011535 (minor at copy_atomic): when the rename over a read-only old copy fails because the copy is in
    /// use, the old copy stays, and its read-only flag is set back. Windows only (only Windows refuses to replace an open file).
    #[cfg(windows)]
    #[test]
    fn a_failed_replace_of_a_read_only_copy_leaves_it_read_only() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = tmp("roback");
        let (from, to) = (d.join("from.exe"), d.join("to.exe"));
        std::fs::write(&from, b"new build").unwrap();
        std::fs::write(&to, b"old build").unwrap();
        let mut ro = std::fs::metadata(&to).unwrap().permissions();
        ro.set_readonly(true);
        std::fs::set_permissions(&to, ro).unwrap();
        // a program running the copy: open with sharing for reads only, so it can be neither replaced nor deleted
        let running = std::fs::OpenOptions::new().read(true).share_mode(1).open(&to).unwrap();
        let r = copy_atomic(&from, &to);
        assert!(matches!(r, Err(CopyFail::Replace(_))), "the replace must fail while the copy is open");
        drop(running);
        assert_eq!(std::fs::read(&to).unwrap(), b"old build", "the old copy stays");
        assert!(std::fs::metadata(&to).unwrap().permissions().readonly(), "and stays read-only");
        assert!(!copy_tmp(&to).exists(), "no temp file is left");
        let plain = d.join("plain");
        std::fs::write(&plain, b"").unwrap();
        std::fs::set_permissions(&to, std::fs::metadata(&plain).unwrap().permissions()).unwrap();
        let _ = std::fs::remove_dir_all(&d);
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
