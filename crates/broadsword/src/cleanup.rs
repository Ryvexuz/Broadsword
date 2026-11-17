//! One cleanup pass: cargo-sweep (if installed), old scratch files, log caps, re-check.

use crate::config::{Config, MB};
use crate::pause;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

#[derive(Debug, Default)]
pub struct Report {
    pub sweep_note: String,
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

fn sweep(cfg: &Config) -> String {
    let installed = Command::new("cargo")
        .args(["sweep", "--version"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !installed {
        return "cargo-sweep not installed, skipped".to_string();
    }
    match Command::new("cargo")
        .args(["sweep", "--time", "7"])
        .env("CARGO_TARGET_DIR", &cfg.build_dir)
        .current_dir(&cfg.root)
        .output()
    {
        Ok(o) if o.status.success() => format!("cargo sweep ran: {}", String::from_utf8_lossy(&o.stdout).trim()),
        Ok(o) => format!("cargo sweep failed: {}", String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => format!("cargo sweep could not start: {e}"),
    }
}

/// Run one pass. `free` reports current free bytes (injected so tests can fake it).
pub fn cleanup_pass(cfg: &Config, now: SystemTime, free: &mut dyn FnMut() -> u64, run_sweep: bool) -> Report {
    let mut r = Report { free_before: free(), ..Default::default() };
    r.sweep_note = if run_sweep { sweep(cfg) } else { "cargo-sweep skipped (disabled)".into() };

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
            "free space {} is below the {} GB floor after cleanup",
            pause::gb(r.free_after),
            cfg.disk_floor_gb
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

    fn cfg(root: &Path) -> Config {
        let text = include_str!("../../../broadsword.toml");
        Config::from_text(text, root).unwrap()
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
