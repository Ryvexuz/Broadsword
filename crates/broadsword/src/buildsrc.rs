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
use crate::lock::{self, Lock};
use crate::{json, scrub, task};
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
/// Registered human waiters precede agents; each class is FIFO. Every acquisition, including the nested-run probe,
/// goes through the same priority arbitration. Priority never changes the caller's configured wait budget.
///
/// Nested run (audit 2026-11-21 22Z fix 3): a `run` job holding this lock sets `scrub::RUN_LOCK_MARKER` = `<job id>:<pid>` on
/// its command (job.rs), after `run_proof` proved it. A Broadsword call inside that command that asks for this lock would
/// wait out the whole `build_lock_wait_seconds` for a lock its own parent holds until the call returns: a deadlock. So it is
/// refused at once (exit 3), but only when `nested_in_holder` proves the lock is held by that very run right now. The marker
/// never grants the lock: it is read only after one priority-aware try found the lock unavailable. With an available lock,
/// a malformed marker or any doubt about the holder (a stale marker, a stale record), the call behaves as without it.
pub fn lock(cfg: &Config, priority: lock::Priority) -> Result<Lock, (u8, String)> {
    let started = std::time::Instant::now();
    let wait = cfg.build_lock_wait_seconds;
    let path = cfg.state_dir.join("build.lock");
    let marker = std::env::var(scrub::RUN_LOCK_MARKER).ok();
    if let Some((job, pid)) = marker.as_deref().and_then(scrub::parse_run_lock_marker) {
        if let Some(l) = try_once(&path, priority)? {
            return Ok(l);
        }
        if nested_in_holder(cfg, marker.as_deref().unwrap_or_default()) {
            return Err((EXIT_BUSY, format!("build folder busy: this command runs inside `broadsword run` job {job}, which holds the build lock (pid {pid}); run it directly, not inside another run. Nothing was built, cleaned or recorded")));
        }
    }
    lock::acquire_priority(
        &path,
        Duration::from_secs(wait).saturating_sub(started.elapsed()),
        priority,
        &move |owner| {
            let who = owner.map(|p| format!(" (last recorded holder pid {p})")).unwrap_or_default();
            format!("build folder busy: the build lock stayed unavailable for the whole {wait} s wait (build_lock_wait_seconds){who}; nothing was built, cleaned or recorded. Repeat the call later (the lock frees itself when its holder ends)")
        },
    )
    .map_err(|e| {
        (
            if e.starts_with("build folder busy") {
                EXIT_BUSY
            } else {
                EXIT_FAILED
            },
            e,
        )
    })
}

/// The run proof: `state/build.lock.run.id` holds the marker of the last run that proved it held the build lock, and
/// `state/build.lock.run` is an OS lock that run holds from just AFTER it wrote that id until just BEFORE it lets the build
/// lock go (job.rs `BuildHold` drops its proof first). So the proof lock is held only by a run that holds the build lock, and
/// only one process holds the build lock at a time. (A second file because on Windows a locked file cannot be read.)
fn run_proof_paths(cfg: &Config) -> (PathBuf, PathBuf) {
    (cfg.state_dir.join("build.lock.run.id"), cfg.state_dir.join("build.lock.run"))
}

/// Called by a `run` job that HOLDS the build lock, before it starts its command: write `marker` as the id, then take the proof
/// lock. `Ok` = proved (the caller keeps the lock until just before it releases the build lock, and sets the marker);
/// `Err` = not proved (a write or lock error, named in the message). The caller must then NOT start the command (job.rs fails
/// the run): a command without the marker would bring back the nested-call deadlock silently (review q-20261121-222529).
pub fn run_proof(cfg: &Config, marker: &str) -> Result<Lock, String> {
    let (id, proof) = run_proof_paths(cfg);
    std::fs::write(&id, marker).map_err(|e| format!("cannot write {}: {e}", id.display()))?;
    // a nested call may hold a shared lock on it for an instant while it checks it (`nested_in_holder`)
    lock::acquire(&proof, Duration::from_secs(cfg.build_lock_wait_seconds), &|_| format!("{} stayed busy", proof.display()))
}

/// Is the build lock held right now by the run whose marker is `marker`? Yes only if the id reads `marker`, the proof lock is
/// held exclusively, and the id still reads `marker` afterwards. Why that is exact: only a build-lock holder writes the id, before it
/// takes the proof lock, and the proof holder holds the build lock, so while the proof is held the id is the holder's own.
/// A job id is unique and a run writes it once, so the id cannot read `marker` before and after the proof check unless the
/// run `marker` names held the proof between. A proof lock no run holds (the shared try succeeds), an id written by
/// a run that has let go (with a new holder that writes nothing, as in review q-20261121-220925) or any read error: no.
fn nested_in_holder(cfg: &Config, marker: &str) -> bool {
    let (id, proof) = run_proof_paths(cfg);
    let reads = || std::fs::read_to_string(&id).is_ok_and(|t| t == marker);
    if !reads() {
        return false;
    }
    // a SHARED try: two nested checks never block each other, only a run's exclusive proof blocks it
    let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&proof) else { return false };
    match f.try_lock_shared() {
        Err(std::fs::TryLockError::WouldBlock) => reads(),
        Ok(()) => false, // released when `f` closes: no run proved it holds the build lock
        Err(std::fs::TryLockError::Error(_)) => false,
    }
}

const TRY_BUSY: &str = "busy";

/// One priority-aware try: `Some` when this waiter could take it (and now holds it), `None` when unavailable.
fn try_once(path: &Path, priority: lock::Priority) -> Result<Option<Lock>, (u8, String)> {
    match lock::acquire_priority(path, Duration::ZERO, priority, &|_| {
        TRY_BUSY.to_string()
    }) {
        Ok(l) => Ok(Some(l)),
        Err(e) if e == TRY_BUSY => Ok(None),
        Err(e) => Err((EXIT_FAILED, e)),
    }
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

// ---- main's binaries survive another folder's clean -----------------------------------------

/// THE stable place of main's binaries (people and other projects run these, never cargo's own profile folders): the KEPT copy of
/// each binary the main checkout built: `<build_dir>/main.bins/<project>/<profile dir>/<bin><exe suffix>`, with `<that
/// file>.source` holding the build-source note it was kept under. Cargo does not build there: a profile's folder carries the
/// profile's name, and cargo allows only letters, digits, `-` and `_` in one, so no profile maps to a name with a `.`; `cargo
/// clean -p` removes the named package's artifacts, which live in the profile folders. It stays inside the ONE shared build
/// folder: only final executables are copied, never dependencies.
pub const KEEP_DIR: &str = "main.bins";

/// THE one place the path of a kept copy is made: `bin` for `profile` of the project `project` (its name as job.rs knows it:
/// `broadsword` for the root, the folder name under `projects_dir` otherwise; see `kept_project`).
pub fn kept_exe(cfg: &Config, project: &str, profile: Profile, bin: &str) -> PathBuf {
    cfg.build_dir.join(KEEP_DIR).join(project).join(profile.dir()).join(format!("{bin}{}", std::env::consts::EXE_SUFFIX))
}

/// The project name whose kept copies belong to the main checkout `main` (an output of `main_folder`): `broadsword` for `.`,
/// `<p>` for `<projects>/<p>`. `None` (nothing is kept) for anything else, for a project folder named like Broadsword itself
/// (job.rs always builds that name from the root, so it would share the root's copies) and for a name that is not one plain
/// path part.
pub fn kept_project(cfg: &Config, main: &str) -> Option<String> {
    if main == MAIN_SOURCE {
        return Some(crate::job::SELF_PROJECT.to_string());
    }
    let projects = rel_source(cfg, &cfg.projects_dir).ok().filter(|p| p != MAIN_SOURCE)?;
    let p = main.strip_prefix(&format!("{projects}/"))?;
    let plain = !p.is_empty() && p != "." && p != ".." && !p.contains(['/', '\\', ':']) && !p.eq_ignore_ascii_case(crate::job::SELF_PROJECT);
    plain.then(|| p.to_string())
}

/// The note file beside a kept copy.
fn kept_note(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".source");
    PathBuf::from(s)
}

/// The build-source note a kept copy of `bin` for `profile` of `project` was kept under (`projects/<p>`, `.`, or a `run:` note
/// of one of those), `None` if there is no kept copy or no (readable) note beside it.
pub fn read_kept_note(cfg: &Config, project: &str, profile: Profile, bin: &str) -> Option<String> {
    let exe = kept_exe(cfg, project, profile, bin);
    if !exe.is_file() {
        return None;
    }
    let text = std::fs::read_to_string(kept_note(&exe)).ok()?;
    let line = text.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// The cargo output path of `bin` for `profile` (cargo's cache: what the last build of any checkout left there).
fn built_exe(cfg: &Config, profile: Profile, bin: &str) -> PathBuf {
    cfg.build_dir.join(profile.dir()).join(format!("{bin}{}", std::env::consts::EXE_SUFFIX))
}

/// The main checkout of the project that `source` belongs to: `.` for `.` and for a worktree of Broadsword itself
/// (`<worktrees>/broadsword/<id>`), `<projects>/<p>` for `<projects>/<p>` and for `<worktrees>/<p>/<id>`. `None` for any other
/// folder (nothing is kept for it). An exact allow-list of the two shapes job.rs builds sources from.
pub fn main_folder(cfg: &Config, source: &str) -> Option<String> {
    if source == MAIN_SOURCE {
        return Some(MAIN_SOURCE.to_string());
    }
    let projects = rel_source(cfg, &cfg.projects_dir).ok().filter(|p| p != MAIN_SOURCE)?;
    let worktrees = rel_source(cfg, &cfg.worktrees_dir).ok().filter(|p| p != MAIN_SOURCE)?;
    let one = |s: &str| !s.is_empty() && !s.contains('/') && s != "." && s != "..";
    if let Some(p) = source.strip_prefix(&format!("{projects}/")) {
        return one(p).then(|| source.to_string());
    }
    let (p, id) = source.strip_prefix(&format!("{worktrees}/"))?.split_once('/')?;
    if !one(p) || !one(id) {
        return None;
    }
    Some(if p == crate::job::SELF_PROJECT { MAIN_SOURCE.to_string() } else { format!("{projects}/{p}") })
}

/// Why `copy_atomic` failed: making the full copy beside `to` (disk full, a length mismatch, ...), or putting it in place of `to`
/// (the rename; on Windows a running or open exe cannot be replaced).
enum CopyFail {
    Copy(std::io::Error),
    Replace(std::io::Error),
}

impl std::fmt::Display for CopyFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CopyFail::Copy(e) => write!(f, "{e}"),
            CopyFail::Replace(e) => write!(f, "cannot replace the old file: {e}"),
        }
    }
}

/// The temp file `copy_atomic` writes beside `to` before renaming it over `to`.
fn copy_tmp(to: &Path) -> PathBuf {
    let mut tmp = to.as_os_str().to_os_string();
    tmp.push(".bskeep.tmp");
    PathBuf::from(tmp)
}

/// The temp file `write_atomic` writes beside `path` before renaming it over `path`.
fn atomic_tmp(path: &Path) -> PathBuf {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    PathBuf::from(tmp)
}

/// EVERY path this module creates, writes, renames or removes for the kept exe `exe`: the exe, its `.source` note
/// and `.stale` marker, and the temp file of each (`copy_tmp`, `atomic_tmp`). `guard_kept_files` checks exactly this list, and
/// `copy_atomic` and `write_atomic` build their temp names with the same helpers.
fn kept_files(exe: &Path) -> [PathBuf; 6] {
    let (side, stale) = (kept_note(exe), stale_marker(exe));
    [exe.to_path_buf(), copy_tmp(exe), atomic_tmp(&side), atomic_tmp(&stale), side, stale]
}

/// Create the temp file `tmp` as a NEW file: whatever entry already has that name (a leftover of a crash, a symlink, a hard link
/// to a file elsewhere) is removed first (removing a link removes the link, never its target), and the create fails if anything
/// appears there in between (`create_new`), so writing it never truncates or overwrites any other file.
fn create_fresh(tmp: &Path) -> std::io::Result<std::fs::File> {
    match std::fs::remove_file(tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)
}

/// THE check before any file of a kept copy is read, written or removed: the
/// build-folder guard, then for the exe `exe` and every file written beside it (`.source`, `.stale`, the temp file): neither the
/// file nor any folder on its way (`main.bins`, the profile folder, up to the drive root) may be a symlink, junction or reparse
/// point, and a file or folder that exists must resolve inside `build_dir`. Err = refuse: nothing of it is touched.
fn guard_kept_files(cfg: &Config, what: &str, exe: &Path) -> Result<(), String> {
    guard_build_dir(cfg, what)?;
    let build = cfg.build_dir.canonicalize().map_err(|e| format!("{what}: cannot resolve the build folder {}: {e}", cfg.build_dir.display()))?;
    for p in kept_files(exe) {
        if !p.starts_with(&cfg.build_dir) {
            return Err(format!("{what}: {} is not inside the build folder {}", p.display(), cfg.build_dir.display()));
        }
        if let Some(l) = link_in_path(&p) {
            return Err(format!("{what}: {} is a symlink, junction or reparse point (on the way to {}), so writing there could reach outside the build folder {}; nothing was written", l.display(), p.display(), cfg.build_dir.display()));
        }
        // the deepest part that exists must resolve inside the build folder
        if let Some(a) = p.ancestors().find(|a| std::fs::symlink_metadata(a).is_ok()) {
            let r = a.canonicalize().map_err(|e| format!("{what}: cannot resolve {}: {e}", a.display()))?;
            if !r.starts_with(&build) {
                return Err(format!("{what}: {} resolves to {}, outside the build folder {}; nothing was written", a.display(), r.display(), build.display()));
            }
        }
    }
    Ok(())
}

/// Copy `from` to `to` through a temp file beside `to` and a rename (a reader sees the old file or the new one). Cargo-built
/// executables only, inside the build folder. On any failure the temp file is removed and `to` is left as it was.
fn copy_atomic(from: &Path, to: &Path) -> Result<(), CopyFail> {
    let tmp = copy_tmp(to);
    // flushing needs a handle with write access on Windows (FlushFileBuffers); the copy must be the whole file before it replaces anything
    let copied = (|| {
        if let Some(d) = to.parent() {
            std::fs::create_dir_all(d)?;
        }
        let mut out = create_fresh(&tmp)?;
        let n = std::io::copy(&mut std::fs::File::open(from)?, &mut out)?;
        let meta = std::fs::metadata(from)?;
        let want = meta.len();
        if n != want || out.metadata()?.len() != want {
            return Err(std::io::Error::other(format!("the copy has {n} bytes, the original {want}")));
        }
        // the copy keeps the build's time (readers such as render's tests pick the newest exe)
        out.set_modified(meta.modified()?)?;
        // ... and its permissions (the execute bits on Unix, the read-only flag on Windows): a kept exe must stay runnable
        let writable = out.metadata()?.permissions();
        out.set_permissions(meta.permissions())?;
        out.sync_all()?;
        Ok(writable)
    })();
    let r = copied.map_err(CopyFail::Copy).and_then(|writable| {
        // Windows refuses to replace a read-only file: the old copy was a read-only copy of an older build, so it is made
        // replaceable first (the plain permissions of a new file; elsewhere a rename never needs the old file to be writable).
        // When the rename then fails (the old copy is in use and stays), its read-only flag is set back: the old copy is left
        // exactly as it was.
        #[cfg(windows)]
        {
            let was_readonly = std::fs::metadata(to).ok().map(|m| m.permissions()).filter(|p| p.readonly());
            if was_readonly.is_some() {
                std::fs::set_permissions(to, writable).map_err(CopyFail::Replace)?;
            }
            std::fs::rename(&tmp, to).map_err(|e| match was_readonly.map(|p| std::fs::set_permissions(to, p)) {
                Some(Err(back)) => CopyFail::Replace(std::io::Error::new(e.kind(), format!("{e}; and setting the old file's read-only flag back failed: {back}"))),
                _ => CopyFail::Replace(e),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = writable;
            std::fs::rename(&tmp, to).map_err(CopyFail::Replace)
        }
    });
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
}

/// True if `p` exists and another process has it open or is running it, so it cannot be replaced now: an exclusive open for
/// writing fails with a sharing or lock violation. Only Windows refuses to replace a running exe (elsewhere a rename always
/// replaces it), so elsewhere this is always false.
#[cfg(windows)]
fn in_use(p: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    // ERROR_SHARING_VIOLATION (32), ERROR_LOCK_VIOLATION (33)
    matches!(std::fs::OpenOptions::new().write(true).share_mode(0).open(p), Err(e) if matches!(e.raw_os_error(), Some(32 | 33)))
}
#[cfg(not(windows))]
fn in_use(_: &Path) -> bool {
    false
}

/// The marker beside a kept copy that could not be refreshed because it was in use: `<exe>.stale`, holding `stale: <note of the
/// newer build>, in use at <UTC>`. The `.source` note keeps naming the build the copy really is; the next refresh that can
/// replace the copy removes the marker.
pub fn stale_marker(exe: &Path) -> PathBuf {
    let mut s = exe.as_os_str().to_os_string();
    s.push(".stale");
    PathBuf::from(s)
}

/// When `keep_main_bins` runs, which decides what it may copy and what a kept copy in use (cannot be replaced) does to the job.
#[derive(Clone, Copy)]
enum Keep<'a> {
    /// right after a SUCCESSFUL main step (`refresh_main_bins`), with what cargo's folder held before that step: only an exe the
    /// step itself produced (`StepBins::produced`), or one a stale marker vouches for, is copied. Any other exe there (cached
    /// from an earlier, maybe FAILED, step) keeps its current kept copy and note untouched. A copy in use is main's previous
    /// build and the new one is in cargo's folder, so the job passes, says so loudly and marks the copy stale (with the new
    /// exe's fingerprint)
    AfterMainStep(&'a StepBins),
    /// before another folder's clean: only the exe a stale marker vouches for (a successful build the refresh could not copy:
    /// `vouched_by_marker`) is copied; anything else in cargo's folder may be a failed build's and is left to the clean. A
    /// vouched copy in use refuses the job (the clean would delete main's newer build from cargo's folder with no copy of it;
    /// nothing built or cleaned)
    BeforeForeignClean,
}

/// What the `.stale` marker records of the exe in cargo's folder that a refresh could not copy: its length and modified time.
/// A later build of any checkout writes a new file there (another length or time), so a match means it is still that exe.
fn fingerprint(exe: &Path) -> std::io::Result<String> {
    let m = std::fs::metadata(exe)?;
    let t = m.modified()?.duration_since(std::time::UNIX_EPOCH).map_err(std::io::Error::other)?;
    Ok(format!("{} bytes, modified {}.{:09}", m.len(), t.as_secs(), t.subsec_nanos()))
}

/// The second line of a `.stale` marker: `built: <fingerprint>`.
const STALE_BUILT: &str = "built: ";

/// True if the `.stale` marker beside `kept` names exactly the exe `built` (its fingerprint on the marker's `built:` line): that
/// exe is a successful main build the refresh could not copy. False for no marker, an unreadable one, or any other exe.
fn vouched_by_marker(kept: &Path, built: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(stale_marker(kept)) else { return false };
    let Ok(now) = fingerprint(built) else { return false };
    text.lines().any(|l| l.strip_prefix(STALE_BUILT) == Some(now.as_str()))
}

/// What cargo's folder held for the bin exes of a MAIN checkout's packages just before one step (`snapshot_main_bins`): the
/// fingerprint (`fingerprint`: length and modified time) of each `<profile>/<bin>` exe, or none where there was no exe. After
/// the step, `produced` says which exes the step itself wrote, the only ones `refresh_main_bins` copies. THE one record both
/// the job's step loop and the test step's compile refresh from (job.rs takes it once per step, before the step runs).
pub struct StepBins {
    /// Ok: (profile, bin, fingerprint before the step). Err: the packages could not be listed before the step, so nothing can
    /// be told apart and nothing is refreshed (fail closed).
    before: Result<Vec<(Profile, String, Option<String>)>, String>,
}

impl StepBins {
    /// True if the exe `exe` of `bin` for `profile` was written during the step: it exists now and its fingerprint differs from
    /// the one recorded before (or there was none: it appeared). False when the record could not be made.
    fn produced(&self, profile: Profile, bin: &str, exe: &Path) -> bool {
        let Ok(before) = &self.before else { return false };
        let Ok(now) = fingerprint(exe) else { return false };
        let was = before.iter().find(|(p, b, _)| *p == profile && b == bin).and_then(|(_, _, f)| f.as_deref());
        was != Some(now.as_str())
    }
}

/// The record of cargo's bin exes before a step of `dir` built from `source` (see `StepBins`). Call it with the build lock held,
/// after the build folder was prepared and right before the step runs. For a source that is not a main checkout it records
/// nothing (`refresh_main_bins` refreshes nothing for it anyway).
pub fn snapshot_main_bins(cfg: &Config, dir: &Path, source: &str) -> StepBins {
    if main_folder(cfg, source).as_deref() != Some(source) {
        return StepBins { before: Ok(Vec::new()) };
    }
    let before = bins_of(cfg, dir).map(|targets| {
        let mut v = Vec::new();
        for (_, bins) in &targets {
            for b in bins {
                for profile in CLEAN_PROFILES {
                    v.push((profile, b.clone(), fingerprint(&built_exe(cfg, profile, b)).ok()));
                }
            }
        }
        v
    });
    StepBins { before }
}

/// Copy main's exes of package `pkg` (whose binary targets are `bins`) to their kept place: every exe of those targets whose note
/// (the bin note, else the package note for builds from before bin notes) names `main` (plain or `run:`), in each profile, with
/// that note beside it. `when` ends the error message. Called after a successful main step (`refresh_main_bins`) and before a
/// job of ANOTHER folder cleans `pkg` (`prepare_packages`: nothing is cleaned until this succeeded; Err refuses the job, nothing
/// deleted); `moment` says which, and with it what may be copied (see `Keep`). The caller holds the build lock. An unchanged
/// copy is not rewritten (and gets no line). The note beside the copy is removed first and written last, so a crash in between
/// leaves a copy with no note.
fn keep_main_bins(cfg: &Config, main: &str, pkg: &str, bins: &[String], when: &str, moment: Keep) -> Result<Vec<String>, String> {
    guard_build_dir(cfg, "refusing to keep main's binaries")?;
    let Some(project) = kept_project(cfg, main) else { return Ok(Vec::new()) };
    let mut lines = Vec::new();
    for b in bins {
        let Some(note) = read_bin_note(cfg, b).or_else(|| read_note(cfg, pkg)) else { continue };
        if note_folder(&note) != main {
            continue;
        }
        for profile in CLEAN_PROFILES {
            let (from, to) = (built_exe(cfg, profile, b), kept_exe(cfg, &project, profile, b));
            if !from.is_file() {
                continue;
            }
            guard_kept_files(cfg, "refusing to keep main's binaries", &to)?;
            // after a main step: an exe this step did not write is left alone, copy and note (it may be a failed step's),
            // unless a stale marker vouches for it (an earlier successful step whose refresh found the copy in use)
            if let Keep::AfterMainStep(step) = moment {
                if !step.produced(profile, b, &from) && !vouched_by_marker(&to, &from) {
                    continue;
                }
            }
            let (side, stale) = (kept_note(&to), stale_marker(&to));
            let old = read_kept_note(cfg, &project, profile, b);
            // sameness is the BYTES only: a copy that is current byte for byte is never replaced because its note text differs
            // (a copy in use must not be refused or marked stale for that); only the note beside it is brought up to date
            let same = std::fs::read(&from).ok().is_some_and(|x| std::fs::read(&to).ok().as_deref() == Some(x.as_slice()));
            let fail = |e: &dyn std::fmt::Display| format!("cannot keep main's `{b}` ({}) at {} {when}: {e}\nIf a program is running the kept copy, stop it and repeat", profile.dir(), to.display());
            let remove = |f: &Path| match std::fs::remove_file(f) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(fail(&e)),
                _ => Ok(()),
            };
            if same {
                remove(&stale)?; // up to date after all: no longer stale
                // only a step that wrote this exe may rename its kept copy's builder; before another folder's clean the copy
                // and its note stay exactly as the last main step left them
                if matches!(moment, Keep::AfterMainStep(_)) && old.as_deref() != Some(note.as_str()) {
                    // the binary is not touched (it may be running); the note is replaced atomically beside it
                    write_atomic(&side, &note).map_err(|e| fail(&e))?;
                    lines.push(format!("main.bins: `{b}` ({}) is current; its note now says {} (the binary was not touched)", profile.dir(), note));
                }
                continue;
            }
            // before another folder's clean only a successful build the refresh could not copy is kept: the exe in cargo's folder
            // may be a failed or partial main build's (prepare set its bin note to main before that build ran)
            if matches!(moment, Keep::BeforeForeignClean) && !vouched_by_marker(&to, &from) {
                lines.push(format!("main.bins: `{b}` ({}) not kept from {}: no successful main step vouches for that exe (it may be a failed build's); main.bins keeps the last good copy", profile.dir(), from.display()));
                continue;
            }
            // the note goes first and comes back last, so a crash in between leaves a copy with no note
            remove(&side)?;
            match copy_atomic(&from, &to) {
                Ok(()) => {
                    write_atomic(&side, &note).map_err(|e| fail(&e))?;
                    remove(&stale)?;
                    lines.push(format!("kept main's `{b}` ({}, built from {}) at {}", profile.dir(), note_folder(&note), to.display()));
                }
                Err(CopyFail::Replace(e)) if matches!(moment, Keep::AfterMainStep(_)) && in_use(&to) => {
                    // the old copy stays, still named by its own note; the marker says a newer build exists, and which exe it is
                    if let Some(o) = &old {
                        write_atomic(&side, o).map_err(|e| fail(&e))?;
                    }
                    let built = fingerprint(&from).map_err(|e| fail(&e))?;
                    write_atomic(&stale, &format!("stale: {note}, in use at {}\n{STALE_BUILT}{built}", crate::pause::now_string())).map_err(|e| fail(&e))?;
                    lines.push(format!(
                        "main.bins: `{b}` ({}) NOT refreshed (in use: {e}); the copy is from {}. The new build is only in {} for now; the next main build refreshes the copy once nothing runs it ({} says so meanwhile)",
                        profile.dir(),
                        old.as_deref().unwrap_or("an unknown build (no note)"),
                        from.display(),
                        stale.display()
                    ));
                }
                Err(e) => {
                    if let Some(o) = &old {
                        let _ = write_atomic(&side, o); // the old copy is still there: name it again before failing
                    }
                    return Err(fail(&e));
                }
            }
        }
    }
    Ok(lines)
}

/// After a SUCCESSFUL cargo step (or `run` command) of the main checkout `source` itself, with the build lock still held: every
/// bin target exe of `dir`'s packages whose note names `source` AND that this step produced (`before`, taken by
/// `snapshot_main_bins` right before the step) is copied to its kept place (`keep_main_bins`), so `main.bins` holds main's newest
/// build of every bin a successful step built. An exe the step did not write (cached from an earlier step, which may have
/// failed) is not copied, and its kept copy and note stay as they are. Any other folder (a worktree, an unknown one) refreshes
/// nothing. A `run` whose packages cannot be listed (now, or before the step) refreshes nothing and says so. Err = the copy
/// failed (the caller fails the job: its output was built, but `main.bins` does not hold it).
pub fn refresh_main_bins(cfg: &Config, dir: &Path, source: &str, before: &StepBins) -> Result<Vec<String>, String> {
    if main_folder(cfg, source).as_deref() != Some(source) {
        return Ok(Vec::new());
    }
    if let Err(e) = &before.before {
        return Ok(vec![format!("main's binaries: not kept (the packages could not be listed before this step, so nothing tells which binaries it built: {e})")]);
    }
    let targets = match bins_of(cfg, dir) {
        Ok(t) => t,
        Err(e) => return Ok(vec![format!("main's binaries: not kept (the packages cannot be listed: {e})")]),
    };
    let mut lines = Vec::new();
    for (p, bins) in &targets {
        lines.extend(keep_main_bins(cfg, source, p, bins, "after this build (the build itself succeeded, but main.bins does not hold it)", Keep::AfterMainStep(before))?);
    }
    Ok(lines)
}

/// A folder's packages, each with its bin target names (what `bins_of` gives).
type PackageBins = Vec<(String, Vec<String>)>;

/// THE list of main checkouts and their packages that a clean must consider before it deletes a package's artifacts from the
/// shared build folder: every project job.rs knows (`job::list_projects`: `broadsword` at the root and each project folder with
/// a Cargo.toml), as (its source folder: `.` or `<projects>/<p>`, its packages with their bin targets from `parse_bins`). An exe
/// path in cargo's folder is shared by every package with a bin of that name, so a clean or build from any folder may remove or
/// overwrite an exe another project's main checkout built (`keep_mains_sharing_exe_paths` matches by exe name). A project
/// folder with no Cargo.toml has no packages (cargo built nothing from it) and is left out.
/// An empty workspace is also skipped: it has no bins to protect (2026-11-22 incident).
/// Err = some project's packages cannot be listed: the caller refuses the clean (fail closed), because nothing then tells whether
/// that project's main binaries would be deleted.
fn mains_with_packages(cfg: &Config) -> Result<Vec<(String, PackageBins)>, String> {
    let mut out = Vec::new();
    for name in crate::job::list_projects(cfg) {
        let refuse = |why: String| format!("refusing to clean: the packages of project `{name}` cannot be listed ({why}), so nothing tells whether this clean or build would remove or overwrite a binary its main checkout built at the same path; nothing was built or cleaned. Fix that project's Cargo.toml (or its folder) and repeat");
        let dir = crate::job::resolve_project(&cfg.root, &cfg.projects_dir, &name).map_err(refuse)?;
        if !dir.join("Cargo.toml").is_file() {
            continue;
        }
        let main = rel_source(cfg, &dir).map_err(refuse)?;
        let metadata = metadata_json(cfg, &dir).map_err(refuse)?;
        let parsed = json::parse(&metadata).map_err(|e| refuse(format!("cannot read cargo metadata output: {e}")))?;
        if matches!(parsed.get("packages"), Some(json::J::Arr(packages)) if packages.is_empty()) {
            continue;
        }
        out.push((main, parse_bins(&metadata).map_err(refuse)?));
    }
    Ok(out)
}

/// True if two bin target names give the same exe path in cargo's folder (`<profile>/<bin><EXE_SUFFIX>`): equal names, and on
/// Windows equal without regard to ASCII case (its file names are case-insensitive; a bin name is plain ASCII).
fn same_exe_name(a: &str, b: &str) -> bool {
    if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b }
}

/// THE check before a job of `source` cleans package `pkg` (whose bin targets in the job's folder are `bins`) and then builds:
/// what is shared, and can be lost, is an exe PATH in cargo's folder, `<profile>/<bin><EXE_SUFFIX>`, whichever package produces
/// it. The paths this job can remove or write are its own bins of `pkg`, plus the bins of a package named `pkg` in any main
/// checkout (a package-level clean: the old package-name match is this case). For EVERY other main checkout in `mains`
/// (`mains_with_packages`) that has a bin, in any package, with one of those exe names, keep-or-refuse runs for that bin
/// (`keep_main_bins` with `Keep::BeforeForeignClean`: the bin note says whether that main checkout built the exe there; a vouched
/// exe is kept, one in use refuses the job, anything else is left). Err = the job is refused before anything is cleaned.
fn keep_mains_sharing_exe_paths(cfg: &Config, source: &str, mains: &[(String, PackageBins)], pkg: &str, bins: &[String]) -> Result<Vec<String>, String> {
    let mut names: Vec<&str> = bins.iter().map(String::as_str).collect();
    for (_, targets) in mains {
        for (_, bs) in targets.iter().filter(|(n, _)| n == pkg) {
            names.extend(bs.iter().map(String::as_str));
        }
    }
    let when = format!("before cleaning `{pkg}` (this job is refused: `{pkg}` was not cleaned and nothing was built; a package listed before it may already have been cleaned, with main's binaries kept first)");
    let mut lines = Vec::new();
    for (m, targets) in mains.iter().filter(|(m, _)| m != source) {
        for (owner, bs) in targets {
            let shared: Vec<String> = bs.iter().filter(|b| names.iter().any(|n| same_exe_name(n, b))).cloned().collect();
            if !shared.is_empty() {
                lines.extend(keep_main_bins(cfg, m, owner, &shared, &when, Keep::BeforeForeignClean)?);
            }
        }
    }
    Ok(lines)
}

/// Write `text` and a newline to `path` atomically (temp file, flushed, renamed).
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = atomic_tmp(path);
    let r = (|| {
        let mut f = create_fresh(&tmp)?;
        std::io::Write::write_all(&mut f, format!("{text}\n").as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if r.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    r
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
