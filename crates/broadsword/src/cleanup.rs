//! One cleanup pass: cargo-sweep (if installed), old scratch files, log caps, re-check.

use crate::config::{Config, MB};
use crate::pause;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

#[derive(Debug, Default)]
pub struct Report {
    pub sweep_note: String,
    /// True only if `cargo sweep` really ran on the build folder; false when it was skipped, missing or failed (the note says which).
    pub swept: bool,
    pub deleted: Vec<(PathBuf, u64)>,
    pub truncated: Vec<(PathBuf, u64)>,
    pub free_before: u64,
    pub free_after: u64,
    pub paused_now: bool,
}

impl Report {
    pub fn freed_files(&self) -> u64 {
        self.deleted.iter().map(|d| d.1).sum::<u64>() + self.truncated.iter().map(|d| d.1).sum::<u64>()
    }
}

/// All regular files under `dir` (symlinks are never followed or returned).
pub fn walk_files(dir: &Path, out: &mut Vec<(PathBuf, fs::Metadata)>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let path = entry.path();
        let Ok(md) = fs::symlink_metadata(&path) else { continue };
        if md.is_dir() {
            walk_files(&path, out);
        } else if md.is_file() {
            out.push((path, md));
        }
    }
}

/// Files in `scratch` older than `max_age`; `.gitkeep` is never selected.
pub fn select_expired(scratch: &Path, now: SystemTime, max_age: Duration) -> Vec<(PathBuf, u64)> {
    let mut files = Vec::new();
    walk_files(scratch, &mut files);
    files
        .into_iter()
        .filter(|(p, _)| p.file_name().is_some_and(|n| n != ".gitkeep"))
        .filter(|(p, _)| p.starts_with(scratch))
        .filter(|(_, md)| match md.modified() {
            Ok(m) => now.duration_since(m).is_ok_and(|age| age > max_age),
            Err(_) => false,
        })
        .map(|(p, md)| (p, md.len()))
        .collect()
}

/// Cut `path` down to its last `max_bytes`. Returns bytes removed.
pub fn cap_log(path: &Path, max_bytes: u64) -> std::io::Result<u64> {
    let len = fs::metadata(path)?.len();
    if len <= max_bytes {
        return Ok(0);
    }
    let mut f = fs::File::open(path)?;
    f.seek(SeekFrom::Start(len - max_bytes))?;
    let mut tail = Vec::with_capacity(max_bytes as usize);
    f.read_to_end(&mut tail)?;
    drop(f);
    fs::File::create(path)?.write_all(&tail)?;
    Ok(len - max_bytes)
}

/// What `cleanup` says when cargo-sweep is missing: loud, because the build folder was NOT swept.
pub const SWEEP_MISSING_NOTE: &str = "WARNING: cargo-sweep is NOT installed, so the build folder was NOT swept (install it with `cargo install cargo-sweep`; Broadsword never installs it itself). Scratch files and logs were still cleaned";

/// Sweep the shared build folder. Returns the note and whether the sweep really ran.
fn sweep(cfg: &Config) -> (String, bool) {
    let installed = crate::scrub::command(cfg, "cargo")
        .args(["sweep", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !installed {
        return (SWEEP_MISSING_NOTE.to_string(), false);
    }
    let days = cfg.sweep_older_than_days.to_string();
    match crate::scrub::command(cfg, "cargo")
        .args(["sweep", "--time", days.as_str()])
        .env("CARGO_TARGET_DIR", &cfg.build_dir)
        .current_dir(&cfg.root)
        .output()
    {
        Ok(o) if o.status.success() => (format!("cargo sweep ran (files unused for {days} days): {}", String::from_utf8_lossy(&o.stdout).trim()), true),
        Ok(o) => (format!("WARNING: cargo sweep failed, so the build folder was NOT swept: {}", String::from_utf8_lossy(&o.stderr).trim()), false),
        Err(e) => (format!("WARNING: cargo sweep could not start, so the build folder was NOT swept: {e}"), false),
    }
}

/// Run one pass. `free` reports current free bytes (injected so tests can fake it).
pub fn cleanup_pass(cfg: &Config, now: SystemTime, free: &mut dyn FnMut() -> u64, run_sweep: bool) -> Report {
    cleanup_pass_in(cfg, now, free, run_sweep, "")
}

/// Like `cleanup_pass`; `context` (for example "during `build x`; job killed") is added to the pause reason if the pass still ends below the floor.
pub fn cleanup_pass_in(cfg: &Config, now: SystemTime, free: &mut dyn FnMut() -> u64, run_sweep: bool, context: &str) -> Report {
    let mut r = Report { free_before: free(), ..Default::default() };
    (r.sweep_note, r.swept) = if run_sweep { sweep(cfg) } else { ("cargo-sweep skipped (disabled)".into(), false) };

    let max_age = Duration::from_secs(cfg.scratch_max_age_hours * 3600);
    for (path, size) in select_expired(&cfg.scratch_dir, now, max_age) {
        match fs::remove_file(&path) {
            Ok(()) => r.deleted.push((path, size)),
            Err(e) => eprintln!("could not delete {}: {e}", path.display()),
        }
    }

    let mut files = Vec::new();
    walk_files(&cfg.scratch_dir, &mut files);
    for (path, _) in files {
        if path.extension().is_some_and(|e| e == "log") {
            match cap_log(&path, cfg.log_max_mb * MB) {
                Ok(0) => {}
                Ok(n) => r.truncated.push((path, n)),
                Err(e) => eprintln!("could not cap {}: {e}", path.display()),
            }
        }
    }

    r.free_after = free();
    if pause::below_floor(r.free_after, cfg.floor_bytes()) && !pause::is_paused(&cfg.root) {
        let reason = format!(
            "free space {} is below the {} GB floor after cleanup{}{context}",
            pause::gb(r.free_after),
            cfg.disk_floor_gb,
            if context.is_empty() { "" } else { " " }
        );
        if pause::write_pause(&cfg.root, r.free_after, &reason).is_ok() {
            r.paused_now = true;
            pause::alert(cfg, &format!("Broadsword PAUSED: {reason}"));
        }
    }
    r
}

pub fn print_report(r: &Report) {
    println!("{}", r.sweep_note);
    for (p, s) in &r.deleted {
        println!("deleted {} ({} bytes)", p.display(), s);
    }
    for (p, s) in &r.truncated {
        println!("capped log {} (cut {} bytes)", p.display(), s);
    }
    println!("freed {} bytes in {} deleted + {} capped files", r.freed_files(), r.deleted.len(), r.truncated.len());
    println!("free space: {} before, {} now", pause::gb(r.free_before), pause::gb(r.free_after));
    if r.paused_now {
        println!("PAUSED: still below the floor");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;

    fn tmp(name: &str) -> PathBuf {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!("test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        p
    }

    fn age(path: &Path, hours: u64) {
        let f = File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600)).unwrap();
    }

    /// The real settings with no fast subset: a temp root has a stand-in Cargo.toml but none of the repo's test binaries.
    fn cfg(root: &Path) -> Config {
        let text = include_str!("../../../broadsword.toml");
        let text: String = text
            .lines()
            .map(|l| {
                if l.split_once('=')
                    .is_some_and(|(k, _)| k.trim() == "merge_fast_tests")
                {
                    "merge_fast_tests = \"\""
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        Config::from_text(&text, root).unwrap()
    }

    #[test]
    fn selects_only_old_files_and_skips_gitkeep() {
        let root = tmp("select");
        let scratch = root.join("scratch");
        fs::create_dir_all(scratch.join("sub")).unwrap();
        fs::write(scratch.join("old.txt"), "x").unwrap();
        fs::write(scratch.join("new.txt"), "x").unwrap();
        fs::write(scratch.join(".gitkeep"), "").unwrap();
        fs::write(scratch.join("sub/old2.txt"), "xx").unwrap();
        age(&scratch.join("old.txt"), 48);
        age(&scratch.join("sub/old2.txt"), 25);
        age(&scratch.join(".gitkeep"), 500);
        let sel = select_expired(&scratch, SystemTime::now(), Duration::from_secs(24 * 3600));
        let mut names: Vec<_> = sel.iter().map(|(p, _)| p.file_name().unwrap().to_string_lossy().to_string()).collect();
        names.sort();
        assert_eq!(names, vec!["old.txt", "old2.txt"]);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn cleanup_never_touches_outside_scratch() {
        let root = tmp("outside");
        fs::create_dir_all(root.join("scratch")).unwrap();
        fs::write(root.join("outside.txt"), "keep").unwrap();
        fs::write(root.join("scratch/old.txt"), "gone").unwrap();
        fs::write(root.join("scratch/.gitkeep"), "").unwrap();
        age(&root.join("outside.txt"), 1000);
        age(&root.join("scratch/old.txt"), 1000);
        age(&root.join("scratch/.gitkeep"), 1000);
        let c = cfg(&root);
        let r = cleanup_pass(&c, SystemTime::now(), &mut || 100 * crate::config::GB, false);
        assert_eq!(r.deleted.len(), 1);
        assert!(root.join("outside.txt").exists());
        assert!(root.join("scratch/.gitkeep").exists());
        assert!(!root.join("scratch/old.txt").exists());
        assert!(!r.paused_now && !pause::is_paused(&root));
        fs::remove_dir_all(&root).unwrap();
    }

    /// `build/main.bins` is THE stable place of main's binaries (buildsrc.rs): no cleanup may touch it, cargo-sweep included.
    /// cargo-sweep only deletes hash-named files in folders that hold a `.fingerprint`, so an aged decoy artifact in
    /// `build/debug` (an old fingerprint, an rlib with the same hash) IS swept in the same pass, which proves the sweep ran over
    /// the build folder, while the aged kept copies survive. Without cargo-sweep the pass says so and still leaves them.
    #[test]
    fn cleanup_and_cargo_sweep_never_touch_mains_kept_binaries() {
        let root = tmp("keptbins");
        fs::create_dir_all(root.join("scratch")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        // cargo-sweep finds the target folder through `cargo metadata` in root
        fs::write(root.join("Cargo.toml"), "[package]\nname = \"sweepfix\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        let c = cfg(&root);
        let old = SystemTime::now() - Duration::from_secs((c.sweep_older_than_days + 30) * 24 * 3600);
        let set_old = |p: &Path| {
            let f = File::options().write(true).open(p).unwrap();
            f.set_times(fs::FileTimes::new().set_accessed(old).set_modified(old)).unwrap();
        };
        // a decoy cargo artifact that a sweep removes
        let fp = c.build_dir.join("debug/.fingerprint/decoy-0123456789abcdef");
        fs::create_dir_all(&fp).unwrap();
        fs::write(fp.join("lib-decoy"), "x").unwrap();
        set_old(&fp.join("lib-decoy"));
        // cargo's own profile layout (cargo-sweep 0.8 fails a profile that lacks `build` or `deps`)
        fs::create_dir_all(c.build_dir.join("debug/deps")).unwrap();
        fs::create_dir_all(c.build_dir.join("debug/build")).unwrap();
        let decoy = c.build_dir.join("debug/deps/libdecoy-0123456789abcdef.rlib");
        fs::write(&decoy, "x").unwrap();
        set_old(&decoy);
        // main's kept copies, aged too
        let mut kept = Vec::new();
        for profile in [crate::buildsrc::Profile::Dev, crate::buildsrc::Profile::Release] {
            let exe = crate::buildsrc::kept_exe(&c, "proj", profile, "markprobe");
            fs::create_dir_all(exe.parent().unwrap()).unwrap();
            fs::write(&exe, "main's exe").unwrap();
            let note = PathBuf::from(format!("{}.source", exe.display()));
            fs::write(&note, "projects/proj\n").unwrap();
            set_old(&exe);
            set_old(&note);
            kept.push(exe);
            kept.push(note);
        }
        let r = cleanup_pass(&c, SystemTime::now(), &mut || 100 * crate::config::GB, true);
        let installed = crate::scrub::command(&c, "cargo").args(["sweep", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if installed {
            assert!(r.swept, "{}", r.sweep_note);
            assert!(!decoy.exists(), "the sweep ran over the build folder and removed the aged decoy: {}", r.sweep_note);
        } else {
            assert!(!r.swept && r.sweep_note.contains("NOT installed"), "{}", r.sweep_note);
        }
        for k in &kept {
            assert!(k.is_file(), "{} must survive every cleanup: {}", k.display(), r.sweep_note);
        }
        assert!(r.deleted.iter().all(|(p, _)| !p.starts_with(c.build_dir.join(crate::buildsrc::KEEP_DIR))));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn log_cap_keeps_tail() {
        let root = tmp("log");
        let p = root.join("a.log");
        fs::write(&p, b"0123456789").unwrap();
        assert_eq!(cap_log(&p, 4).unwrap(), 6);
        assert_eq!(fs::read(&p).unwrap(), b"6789");
        assert_eq!(cap_log(&p, 4).unwrap(), 0);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn below_floor_with_nothing_to_delete_pauses_and_alerts() {
        let root = tmp("pause");
        fs::create_dir_all(root.join("scratch")).unwrap();
        let c = cfg(&root);
        let r = cleanup_pass(&c, SystemTime::now(), &mut || crate::config::GB, false);
        assert!(r.paused_now);
        let body = pause::read_pause(&root).unwrap();
        assert!(body.contains("reason:") && body.contains("free:"));
        let log = fs::read_to_string(root.join("scratch/logs/monitor.log")).unwrap();
        assert!(log.contains("PAUSED"));
        fs::remove_dir_all(&root).unwrap();
    }
}
