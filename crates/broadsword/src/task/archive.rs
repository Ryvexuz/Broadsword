//! Board-only archival. Records are moved, never saved or deleted.
use super::*;
use std::path::Path;

fn plain_path(path: &Path, directory: bool) -> Result<(), String> {
    let m = std::fs::symlink_metadata(path).map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(format!("refusing reparse point {}", path.display()));
        }
    }
    if m.file_type().is_symlink() || (directory && !m.is_dir()) || (!directory && !m.is_file()) {
        return Err(format!("not a plain {}: {}", if directory { "directory" } else { "record" }, path.display()));
    }
    Ok(())
}

pub(super) fn find(cfg: &Config, id: &str) -> Result<Option<PathBuf>, String> {
    let dir = tasks_dir(cfg).join("archive");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    plain_path(&dir, true)?;
    let mut found = None;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        // Inspect the entry itself, without following links: stray files are not archive months.
        if !entry.file_type().map_err(|e| format!("cannot inspect {}: {e}", path.display()))?.is_dir() {
            continue;
        }
        plain_path(&path, true)?;
        let candidate = path.join(format!("{id}.{EXT}"));
        match std::fs::symlink_metadata(&candidate) {
            Ok(_) => {
                plain_path(&candidate, false)?;
                if found.is_some() {
                    return Err(format!("multiple archived records for task `{id}`"));
                }
                found = Some(candidate);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot inspect {}: {e}", candidate.display())),
        }
    }
    Ok(found)
}

/// Reserve an id across the board and archive. The caller holds the board lock until the record is saved.
pub(super) fn create_task(cfg: &Config, base: &str) -> Result<Record, String> {
    let dir = tasks_dir(cfg);
    let base = record::sanitize(base);
    for n in 1u32.. {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        if find(cfg, &id)?.is_some() {
            continue;
        }
        let path = dir.join(format!("{id}.{EXT}"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {
                let mut r = Record::parse(&path, "");
                r.set("id", id);
                return Ok(r);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
        }
    }
    Err(format!("no free task id for `{base}`"))
}

fn plan(cfg: &Config, now: u64) -> Result<(Vec<(Record, String)>, usize), String> {
    let age = cfg.task_archive_days.checked_mul(86400).ok_or("task_archive_days overflows seconds")?;
    let dir = tasks_dir(cfg);
    plain_path(&dir, true)?;
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    let queue = open_queue_entries(cfg)?;
    let mut moves = Vec::new();
    let mut kept = 0;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_none_or(|e| e != EXT) {
            continue;
        }
        plain_path(&path, false)?;
        let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut keys = std::collections::HashSet::new();
        for line in text.lines() {
            let key = line.split_once(" = ").map(|(k, _)| k).or_else(|| line.strip_suffix(" ="));
            if key.is_none_or(|k| k.trim().is_empty() || !keys.insert(k.trim())) {
                return Err(format!("invalid or duplicate field in task record {}", path.display()));
            }
        }
        let r = Record::parse(&path, &text);
        if r.id().is_empty()
            || record::sanitize(r.id()) != r.id()
            || path.file_stem().and_then(|s| s.to_str()) != Some(r.id())
            || !STATUSES.contains(&r.status())
        {
            return Err(format!("invalid task record {}", path.display()));
        }
        let ended = if r.get("ended").is_empty() {
            None
        } else {
            if !r.get("ended").bytes().all(|b| b.is_ascii_digit()) {
                return Err(format!("invalid ended time in {}", path.display()));
            }
            Some(r.get("ended").parse::<u64>().map_err(|_| format!("invalid ended time in {}", path.display()))?)
        };
        if ![DONE, FAILED, ABANDONED].contains(&r.status()) {
            continue;
        }
        let Some(ended) = ended else {
            continue;
        };
        if now.saturating_sub(ended) <= age {
            kept += 1;
            continue;
        }
        if queue.iter().any(|e| e.task == r.id()) || (!r.get("worktree").is_empty() && r.get("worktree_removed").is_empty()) {
            continue;
        }
        let stamp = job::file_stamp(ended);
        let month = format!("{}-{}", &stamp[..4], &stamp[4..6]);
        moves.push((r, month));
    }
    moves.sort_by(|a, b| a.0.id().cmp(b.0.id()));
    Ok((moves, kept))
}

pub fn cmd_archive(cfg: &Config) -> ExitCode {
    let _lock = match board_lock(cfg) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let (moves, kept) = match plan(cfg, now_secs()) {
        Ok(p) => p,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    let mut moved = 0;
    let mut failed = false;
    for (r, month) in moves {
        let relative = format!("state/tasks/archive/{month}/{}.task", r.id());
        let archive = tasks_dir(cfg).join("archive");
        let dir = archive.join(&month);
        let target = dir.join(format!("{}.{EXT}", r.id()));
        let result = (|| -> Result<(), String> {
            for folder in [&archive, &dir] {
                match std::fs::create_dir(folder) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(format!("cannot create {}: {e}", folder.display())),
                }
                plain_path(folder, true)?;
            }
            match std::fs::symlink_metadata(&target) {
                Ok(_) => return Err(format!("target already exists: {} -> {}", r.path.display(), target.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("cannot inspect {}: {e}", target.display())),
            }
            plain_path(&r.path, false)?;
            std::fs::rename(&r.path, &target).map_err(|e| format!("cannot move {} -> {}: {e}", r.path.display(), target.display()))
        })();
        match result {
            Ok(()) => {
                moved += 1;
                println!("archived {} -> {relative}", r.id());
            }
            Err(e) => {
                eprintln!("broadsword: {e}");
                failed = true;
            }
        }
    }
    println!("archive: moved {moved}, kept {kept} ended task(s) younger than {} days", cfg.task_archive_days);
    ExitCode::from(if failed { EXIT_ERROR } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_creation_checks_board_and_archive_for_each_suffix() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!("archive-id-{}", std::process::id()));
        let cfg = Config::from_text(include_str!("../../../../broadsword.toml"), &root).unwrap();
        {
            let _lock = board_lock(&cfg).unwrap();
            let dir = tasks_dir(&cfg).join("archive/2000-01");
            std::fs::create_dir_all(&dir).unwrap();
            let first = dir.join("t-fixed.task");
            let third = dir.join("t-fixed-3.task");
            std::fs::write(&first, b"archived first").unwrap();
            std::fs::write(&third, b"archived third").unwrap();
            let second = tasks_dir(&cfg).join("t-fixed-2.task");
            std::fs::write(&second, b"board second").unwrap();
            let r = create_task(&cfg, "t-fixed").unwrap();
            assert_eq!(r.id(), "t-fixed-4");
            assert_eq!(std::fs::read(&first).unwrap(), b"archived first");
            assert_eq!(std::fs::read(&second).unwrap(), b"board second");
            assert_eq!(std::fs::read(&third).unwrap(), b"archived third");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_plan_uses_strict_age_and_configured_days_without_overflow() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!("archive-plan-{}", std::process::id()));
        let mut cfg = Config::from_text(include_str!("../../../../broadsword.toml"), &root).unwrap();
        cfg.state_dir = root.join("state");
        std::fs::create_dir_all(tasks_dir(&cfg)).unwrap();
        let path = tasks_dir(&cfg).join("t-old.task");
        std::fs::write(&path, "id = t-old\nstatus = done\nended = 0\nworktree = old\nworktree_removed = 1\n").unwrap();
        cfg.task_archive_days = 1;
        assert_eq!(plan(&cfg, 86400).unwrap().0.len(), 0, "exactly one day is not older");
        assert_eq!(plan(&cfg, 86401).unwrap().0.len(), 1, "one second over the threshold");
        cfg.task_archive_days = 2;
        assert_eq!(plan(&cfg, 86401).unwrap().0.len(), 0, "the setting, not a fixed threshold");
        cfg.task_archive_days = u64::MAX;
        assert!(plan(&cfg, u64::MAX).unwrap_err().contains("task_archive_days"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
