//! Which checkout last built each package into the shared build folder, and the lock that keeps that note true.
//!
//! The problem: every job builds into ONE build folder (`build_dir`). Two checkouts of the same package (the main
//! checkout and a task's worktree) have the same package name but different paths, so cargo keeps a separate
//! fingerprint for each. Build checkout A, then B, then A again, and cargo sees A as fresh, prints `Finished` and does
//! NOT relink: the binary in the build folder is still B's. That happened once, silently.
//!
//! The rule that closes it: `state/build-sources/<package>` holds the source folder (relative to root, `/` separators,
//! `.` for the main checkout) that last built that package. Before a cargo step, every package of the folder about to be
//! built whose note differs (or is missing: whoever built it is unknown) gets `cargo clean -p <package> --offline`, which
//! removes that package's own artifacts, its binary included, and nothing else (dependencies stay). Order matters for
//! crashes: clean first, THEN write the note, THEN build. A crash after the clean leaves a note naming the old source and
//! no artifact (harmless: the next build of the old source cleans again or builds fresh). A crash after the note is
//! written leaves the new source named and nothing stale. Writing the note after the build instead could leave a new
//! binary under an old note, which is exactly the hazard.
//!
//! Clean, note and build are one step across processes: the job holds `state/build.lock` (an OS file lock, lock.rs) for
//! all of its cargo steps. Without it, job 2 could read the note, decide no clean is needed, and then job 1 would build its
//! own source over the folder before job 2's cargo step runs.
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
    let text = std::fs::read_to_string(notes_dir(cfg).join(package)).ok()?;
    let line = text.lines().next()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}

/// Record `source` as the last builder of `package`: temp file, flushed to disk, then renamed over the note (atomic: a
/// reader sees the old note or the new one, never half of one).
pub fn write_note(cfg: &Config, package: &str, source: &str) -> Result<(), String> {
    if !valid_package(package) {
        return Err(format!("`{package}` is not a valid package name"));
    }
    let dir = notes_dir(cfg);
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let (path, tmp) = (dir.join(package), dir.join(format!("{package}.tmp")));
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

/// Take `state/build.lock`. A job holds it for ALL of its cargo steps (see the module comment), so it is held for as long
/// as the job's builds and tests run: seconds to many minutes. A second job waits up to `build_lock_wait_seconds`, then is
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

fn cargo() -> Command {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
}

/// `cargo metadata --no-deps --format-version 1 --offline` in `dir`: the package names of the folder about to be built.
pub fn packages_of(dir: &Path) -> Result<Vec<String>, String> {
    let out = cargo()
        .args(["metadata", "--no-deps", "--format-version", "1", "--offline"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot start cargo to list the packages in {}: {e}", dir.display()))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed in {}: {}", dir.display(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    parse_packages(&String::from_utf8_lossy(&out.stdout))
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

/// The one and only place a `cargo clean` is made. Always `-p <package>`, always aimed at `build_dir`. Refuses (Err, nothing
/// run) if the package name is not a plain cargo name, `build_dir` is not strictly inside root, or the source folder lies
/// inside `build_dir` (cleaning would then be pointed at, or beside, sources).
pub fn clean_command(cfg: &Config, dir: &Path, package: &str) -> Result<Command, String> {
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
    let mut c = cargo();
    c.arg("clean").arg("-p").arg(package).arg("--offline").arg("--target-dir").arg(b).current_dir(dir).env("CARGO_TARGET_DIR", b).stdin(Stdio::null());
    Ok(c)
}

/// Mark every package of `dir` as "built by someone unknown" by deleting its note (a missing note means unknown, so the
/// next build/test/check cleans it and `install` refuses). Used after a `run` job, whose command may have run cargo.
/// Best effort and never fatal: cargo metadata failing (not a cargo folder) means there is nothing to name. Returns lines
/// for the job's output.
pub fn forget_notes(cfg: &Config, dir: &Path) -> Vec<String> {
    let Ok(packages) = packages_of(dir) else { return Vec::new() };
    let mut lines = Vec::new();
    for p in packages.iter().filter(|p| valid_package(p)) {
        let path = notes_dir(cfg).join(p);
        if path.exists() && std::fs::remove_file(&path).is_ok() {
            lines.push(format!("build-source note of `{p}` cleared: a `run` command may have built it, so the next build/test/check will clean it and `install` will refuse until a `broadsword build`"));
        }
    }
    lines
}

/// What `prepare` did, one line each, for the job's output and log.
pub struct Prepared {
    pub lines: Vec<String>,
}

/// Make the shared build folder safe to build `dir` into: for every package of `dir` whose note is not `source`, clean that
/// package's artifacts, then record `source` for it. The caller holds the build lock. Err = nothing may be built (the
/// message says why and what to do).
pub fn prepare(cfg: &Config, dir: &Path, source: &str) -> Result<Prepared, String> {
    prepare_packages(cfg, dir, source, packages_of(dir)?)
}

/// The preparation a `run` job makes before its command (the command may be `cargo test`, which would otherwise test the
/// binary another checkout left in the shared build folder): the same clean as `prepare`, then the notes of the folder's
/// packages are forgotten again, because the command may build anything and nothing may vouch for the result (`install`
/// refuses, and the next build, test or check cleans). `run` runs arbitrary commands, so a folder whose packages cannot be
/// listed (a broken Cargo.toml the command may be there to diagnose) is NOT refused: the line says protection was skipped.
/// A `cargo clean` that fails (a locked binary) refuses the job, as it does for build, test and check. The caller holds the lock.
pub fn prepare_for_run(cfg: &Config, dir: &Path, source: &str) -> Result<Vec<String>, String> {
    let mut lines = match packages_of(dir) {
        Ok(packages) => prepare_packages(cfg, dir, source, packages)?.lines,
        Err(e) => vec![format!("build source: not checked (the stale-binary protection is skipped for this run): {e}")],
    };
    lines.extend(forget_notes(cfg, dir));
    Ok(lines)
}

fn prepare_packages(cfg: &Config, dir: &Path, source: &str, packages: Vec<String>) -> Result<Prepared, String> {
    let stale: Vec<(String, Option<String>)> = packages.iter().map(|p| (p.clone(), read_note(cfg, p))).filter(|(_, n)| n.as_deref() != Some(source)).collect();
    task::wait_for_gate(cfg, TEST_SYNC_GATE_ENV, "the build-source sync (holding the build lock)");
    let mut lines = vec![format!("build source: {source} (packages: {})", packages.join(", "))];
    for (p, was) in &stale {
        let out = clean_command(cfg, dir, p)?.output().map_err(|e| format!("cannot start `cargo clean -p {p}`: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "`cargo clean -p {p}` failed in {}: {}\nnothing was built. The old copy of `{p}` in the build folder could not be removed; on Windows a binary that is running cannot be deleted, so stop whatever runs it (run the installed copy under install_dir, not the one in the build folder) and repeat",
                dir.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        write_note(cfg, p, source)?;
        lines.push(match was {
            Some(w) => format!("cleaned `{p}` in the shared build folder (it was last built from {w}); its other packages and all dependencies were kept"),
            None => format!("cleaned `{p}` in the shared build folder (no record of what built it); its dependencies were kept"),
        });
    }
    Ok(Prepared { lines })
}

// ---- install ----------------------------------------------------------------------------

/// `broadsword install` may only install code built from the main checkout of Broadsword: the installed copy runs the
/// pause hook and the MCP server for everyone. Reads the same note. Err = the message to print (exit 1).
pub fn check_installable(cfg: &Config) -> Result<(), String> {
    let pkg = env!("CARGO_PKG_NAME");
    match read_note(cfg, pkg) {
        Some(s) if s == MAIN_SOURCE => Ok(()),
        Some(other) => Err(format!(
            "refusing to install: the `{pkg}` binary in the build folder was last built from `{other}`, not from the main checkout. The installed copy runs the pause hook and the MCP server for everyone, so it must never be a task worktree's experiment. Build the main checkout first (`broadsword build {pkg}`, with no --task, run from the main checkout), then run `install` from the binary it built"
        )),
        None => Err(format!(
            "refusing to install: there is no record of which checkout built the `{pkg}` binary in the build folder ({} is missing), so it could be a worktree's. Build the main checkout through Broadsword first (`broadsword build {pkg}`, with no --task), then run `install` from the binary it built",
            notes_dir(cfg).join(pkg).display()
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
        let c = clean_command(&cfg, &dir, "my-pkg").unwrap();
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
            assert!(clean_command(&cfg, &dir, bad).is_err(), "{bad}");
        }
        let mut wild = cfg.clone();
        wild.build_dir = wild.root.clone();
        assert!(clean_command(&wild, &dir, "p").is_err(), "build_dir == root");
        wild.build_dir = PathBuf::from("elsewhere").join("build");
        assert!(clean_command(&wild, &dir, "p").is_err(), "build_dir outside root");
        wild.build_dir = cfg.root.join("..").join("build");
        assert!(clean_command(&wild, &dir, "p").is_err(), "build_dir with ..");
        assert!(clean_command(&cfg, &cfg.build_dir.join("deep"), "p").is_err(), "source inside the build folder");
        assert!(clean_command(&cfg, &cfg.root, "p").is_ok(), "the main checkout contains the build folder, which is fine");
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
        let e = clean_command(&cfg, &d, "p").unwrap_err();
        assert!(e.contains("junction"), "{e}");
        cfg.build_dir = link.clone();
        assert!(clean_command(&cfg, &d, "p").is_err(), "the build folder itself");
        cfg.build_dir = parent;
        assert!(clean_command(&cfg, &d, "p").is_ok());
        let _ = std::fs::remove_dir(&link);
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
