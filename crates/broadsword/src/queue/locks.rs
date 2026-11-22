use crate::config::Config;
use crate::job;
use crate::lock::{self, Lock};
use crate::record::{self, Record};
use crate::task::fail;
use crate::pause;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use super::check::main_check_now;
use super::entry::{load_entry, queue_dir};
use super::{EXIT_BAD_INPUT, EXIT_BUSY, EXIT_ERROR, EXIT_RETRY};

/// One of the queue's two lock files, waiting at most `queue_lock_wait_seconds`; `Err` is the exit code, message printed.
fn take(cfg: &Config, file: &str, what: &str) -> Result<Lock, ExitCode> {
    let wait = cfg.queue_lock_wait_seconds;
    let what = what.to_string();
    lock::acquire(&queue_dir(cfg).join(file), Duration::from_secs(wait), &move |owner| {
        let who = owner.map(|p| format!(" (pid {p})")).unwrap_or_default();
        format!("queue busy: {what}{who} held the lock for the whole {wait} s wait; nothing was changed, try again later (the lock frees itself when its holder ends)")
    })
    .map_err(|e| fail(if e.starts_with("queue busy") { EXIT_BUSY } else { EXIT_ERROR }, e))
}

fn queue_lock(cfg: &Config) -> Result<Lock, ExitCode> {
    take(cfg, "queue.lock", "another `queue run` (or a cancel of a running entry)")
}

/// A run LANE (SCALING F2): the entries one `queue run` may take, and the OS lock that proves no other run takes them. Holding it is
/// what makes a `running` entry of the lane an interrupted one. `key` is a lowercase project name (`lane_key`), or None for every
/// project. The lock is held as long as the Lane lives.
pub(super) struct Lane {
    _lock: Lock,
    pub(super) key: Option<String>,
}

impl Lane {
    /// Whether entry `e` is in this lane. Project names are matched without case, like `job::canonical_project` matches them (the
    /// canonical name of a plain name is that name up to case), so this is exact without reading the projects folder.
    pub(super) fn holds(&self, e: &Record) -> bool {
        self.key.as_ref().is_none_or(|k| e.get("project").to_lowercase() == *k)
    }
}

/// The lane key of an entry's project: its name, lowercased. Err when it is not a plain project name (no lock file could carry it). A
/// plain name that is no longer a project keys as itself, so its entries can still be resumed (to end `blocked`) or cancelled.
pub(super) fn lane_key(cfg: &Config, name: &str) -> Result<String, String> {
    match job::canonical_project(&cfg.projects_dir, name) {
        Ok(c) => Ok(c.to_lowercase()),
        Err(m) if m.starts_with("unknown project") => Ok(name.to_lowercase()),
        Err(m) => Err(m),
    }
}

/// LANE HOLD (queue_auto_revert): the file `<state_dir>/queue/hold-<lane key>` holds a project's lane after an automatic revert, or
/// the check after it, failed. While it exists `queue run` merges nothing of that project (`hold_gate`, and `lane_hold` before every
/// fast-forward). Only `hold_gate` removes it, and only after a check of that project's main passed.
fn hold_path(cfg: &Config, project: &str) -> PathBuf {
    let key = lane_key(cfg, project).unwrap_or_else(|_| project.to_lowercase());
    queue_dir(cfg).join(format!("hold-{}", record::sanitize(&key)))
}

/// Write the hold of `project`'s lane: UTC time, the entry, the project and the reason, one `key = value` line each.
pub(super) fn write_hold(cfg: &Config, project: &str, entry: &str, reason: &str) -> Result<PathBuf, String> {
    let path = hold_path(cfg, project);
    let text = format!("time = {}\nentry = {entry}\nproject = {project}\nreason = {}\n", pause::now_string(), reason.replace(['\n', '\r'], " "));
    std::fs::create_dir_all(queue_dir(cfg)).and_then(|_| std::fs::write(&path, text)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// One field of a hold file's text (empty when it is missing).
fn hold_field(text: &str, key: &str) -> String {
    text.lines().find_map(|l| l.strip_prefix(&format!("{key} = "))).unwrap_or("").to_string()
}

/// The reason `project`'s lane is held, or None. A hold file that cannot be read still holds the lane (fail closed).
pub(super) fn lane_hold(cfg: &Config, project: &str) -> Option<String> {
    let path = hold_path(cfg, project);
    if !path.exists() {
        return None;
    }
    Some(match std::fs::read_to_string(&path) {
        Ok(t) => hold_field(&t, "reason"),
        Err(e) => format!("the hold file {} cannot be read: {e}", path.display()),
    })
}

/// HOLD GATE, at the start of every `queue run` (one entry or `--all`), under the lane's run lock: for every held project of the lane, a
/// check of its main runs first; when it passes the hold is removed (the ONE place a hold file is ever deleted), otherwise the run merges
/// and runs nothing (exit 3) and says why.
pub(super) fn hold_gate(cfg: &Config, lane: &Lane) -> Result<(), ExitCode> {
    let Ok(rd) = std::fs::read_dir(queue_dir(cfg)) else { return Ok(()) };
    let mut holds: Vec<PathBuf> = rd.flatten().map(|d| d.path()).filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("hold-"))).collect();
    holds.sort();
    let mut still: Vec<String> = Vec::new();
    for path in holds {
        let key = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("hold-")).unwrap_or("").to_string();
        if lane.key.as_ref().is_some_and(|k| record::sanitize(k) != key) {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                still.push(format!("lane {key} held: the hold file {} cannot be read ({e}); clear with a passing check of main, then queue run", path.display()));
                continue;
            }
        };
        let project = Some(hold_field(&text, "project")).filter(|p| !p.is_empty()).unwrap_or_else(|| key.clone());
        let reason = hold_field(&text, "reason");
        println!("lane {project} is held ({reason}); checking its main first");
        let checked = job::resolve_project(&cfg.root, &cfg.projects_dir, &project).and_then(|dir| main_check_now(cfg, &dir, &project));
        match checked {
            Ok(job) => match std::fs::remove_file(&path) {
                Ok(()) => println!("lane {project}: hold cleared: the check of main passed (job {job})"),
                Err(e) => still.push(format!("lane {project} held: {reason}; the check of main passed (job {job}) but the hold file could not be removed ({e})")),
            },
            Err(m) => {
                println!("lane {project}: the check of main did not pass ({m})");
                still.push(format!("lane {project} held: {reason}; clear with a passing check of main, then queue run"));
            }
        }
    }
    if still.is_empty() {
        return Ok(());
    }
    for s in &still {
        println!("{s}");
    }
    Err(ExitCode::from(EXIT_RETRY))
}

/// The lane key of a `--project` the caller typed: it must be a project (exit 2 otherwise, nothing run).
fn project_key(cfg: &Config, name: &str) -> Result<String, ExitCode> {
    job::canonical_project(&cfg.projects_dir, name).map(|c| c.to_lowercase()).map_err(|m| fail(EXIT_BAD_INPUT, format!("--project: {m}; nothing was run (see `broadsword projects`)")))
}

/// The run lock file of a lane key: `<project>.queue.lock` next to queue.lock. `sanitize` maps any other character to `_`, so two
/// names can only ever share a lock (stricter), never escape the queue folder.
fn lane_lock_file(key: &str) -> String {
    format!("{}.queue.lock", record::sanitize(key))
}

/// THE ONE PLACE a run lock is taken: every `queue run` (one entry or `--all`) and the cancel of a running entry come through here,
/// before the entries lock (order: run lock, then entries lock). `queue_per_project = "off"`: the one global queue.lock, exactly as
/// before (a `key` then only narrows which entries the run takes). `"on"`: the lane's own `<project>.queue.lock` (same OS lock, same
/// `queue_lock_wait_seconds`, exit 4 naming the project), so runs of different projects go at once; without a key it is refused (exit
/// 2), because a run of no named project cannot tell another project's live `running` entries from interrupted ones. A lock that
/// cannot be taken for any other reason fails closed (exit 1).
pub(super) fn run_lock(cfg: &Config, key: Option<String>) -> Result<Lane, ExitCode> {
    if !cfg.queue_per_project {
        return Ok(Lane { _lock: queue_lock(cfg)?, key });
    }
    let Some(k) = key else {
        return Err(fail(EXIT_BAD_INPUT, "queue_per_project is on: every project has its own run lock, so name the project with `--project <p>` (or one entry with `--entry <id>`); nothing was run"));
    };
    let lock = take(cfg, &lane_lock_file(&k), &format!("another `queue run` of project `{k}` (or a cancel of one of its running entries)"))?;
    Ok(Lane { _lock: lock, key: Some(k) })
}

/// The lane of `queue run [--entry <id>] [--project <p>]`. `--project` must name a project, and an `--entry` given with it must be
/// of that project. With `--entry` alone and the setting on, the lane is the entry's own project (an entry's project never changes,
/// so reading it before the lock is safe); with the setting off, `--entry` alone is exactly as before (no lane key).
pub(super) fn lane_for(cfg: &Config, entry: Option<&str>, project: Option<&str>) -> Result<Option<String>, ExitCode> {
    let of_entry = |id: &str| -> Result<String, ExitCode> {
        let e = load_entry(cfg, id).map_err(|m| fail(EXIT_BAD_INPUT, m))?;
        lane_key(cfg, e.get("project")).map_err(|m| fail(EXIT_BAD_INPUT, format!("queue entry {id}: its project `{}` cannot have a run lock: {m}; nothing was run", e.get("project"))))
    };
    match (entry, project) {
        (_, Some(p)) => {
            let k = project_key(cfg, p)?;
            if let Some(id) = entry {
                let ek = of_entry(id)?;
                if ek != k {
                    return Err(fail(EXIT_BAD_INPUT, format!("queue entry {id} is of project `{ek}`, not `{k}`; nothing was run")));
                }
            }
            Ok(Some(k))
        }
        (Some(id), None) if cfg.queue_per_project => Ok(Some(of_entry(id)?)),
        _ => Ok(None),
    }
}

pub(super) fn entries_lock(cfg: &Config) -> Result<Lock, ExitCode> {
    take(cfg, "entries.lock", "another queue command")
}
