//! `broadsword recover`: deal with jobs whose Broadsword died mid-job.
//!
//! Every record still `running` whose Broadsword process is gone is recovered:
//! - the recorded child process tree is killed first if it is still alive (an orphaned cargo or command),
//!   but only when its recorded creation time matches exactly (a reused pid is never killed);
//! - `build` / `test` / `check` only read and rebuild, so they are resumed (they take no checkpoint): re-run from the step they
//!   were on, as a new job linked to the old one (old record -> `resumed`), at most `max_resumes` times
//!   in a row, then the record is marked `failed`;
//! - `run` may have changed files, so it is rolled back to its checkpoint (the only kind that has one; secret files are
//!   never part of a checkpoint or of the rollback, and each one it leaves alone is named in the output: `.env`, `.envrc`,
//!   `.env.<name>` and names starting with `.env-` or `.env_`, any case, plus an NTFS short name or stream of one, as
//!   `git::is_secret_in` decides; `.envoy.yaml` and `.environment` are ordinary files), after saving the partial work
//!   under `refs/broadsword/interrupted/<id>/<attempt>` (old record -> `rolled_back`). Nothing is deleted.
//!
//! A record whose Broadsword is still alive is another job genuinely running and is left alone.
//! A rollback reverts files, and an agent may still be working in that folder, so it only ever happens on request, for the
//! job named: `broadsword recover <job id>` rolls back exactly that one interrupted `run`; `broadsword recover --all` rolls
//! back every one (a human who means it). Plain `broadsword recover` (`Mode::List`) does the automatic part and only LISTS
//! interrupted `run` jobs, with the command to roll each back; it changes no project file. The automatic recovery before
//! a job (`Mode::Auto`) likewise kills orphans and resumes read-only jobs but never rolls back, and
//! `blocked_by_interrupted_run` then stops new jobs in that run's own source folder until it is recovered.
//! A lock file (`state/recover.lock`) keeps two recovers from working at the same time.

use crate::config::Config;
use crate::git;
use crate::job::{self, JobSpec};
use crate::lock::{self, Lock};
use crate::pause;
use crate::record::{self, Record};
use crate::sys;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `broadsword recover --all`: does everything, including rolling back EVERY interrupted `run` job.
    All,
    /// `broadsword recover`: the automatic part, then lists interrupted `run` jobs; never rolls back.
    List,
    /// Before a job starts: never rolls anything back, prints no list.
    Auto,
}

pub fn cmd_list(cfg: &Config) -> ExitCode {
    let all = record::load_all(&record::jobs_dir(cfg));
    if all.is_empty() {
        println!("no job records in {}", record::jobs_dir(cfg).display());
    }
    for r in &all {
        let note = match r.status() {
            record::RESUMED => format!(" -> {}", r.get("resumed_as")),
            record::ROLLED_BACK => format!(" (partial work: {})", r.get("interrupted")),
            _ => String::new(),
        };
        println!("{}\t{}\t{} {}\t{}{note}", r.id(), r.status(), r.get("kind"), r.get("project"), r.get("command"));
    }
    ExitCode::SUCCESS
}

fn broadsword_alive(r: &Record) -> bool {
    match r.get_num("pid") {
        Some(pid) => sys::is_alive(pid as u32, r.get_num("pid_start")), // lenient on purpose: never call a live job dead
        None => false, // a record with no pid can only be one that died before it started
    }
}

/// Is this a `run` job that was interrupted and not yet rolled back?
fn interrupted_run(r: &Record) -> bool {
    r.status() == record::RUNNING && r.get("kind") == "run" && !broadsword_alive(r)
}

/// An interrupted `run` job that blocks a new job of `project` running for `task` (`None` = the project's main checkout),
/// if any: that new job must wait for `broadsword recover`. Returns the blocking job's id and its source folder.
/// The block is per folder: the new job's source folder comes from `job::job_source` (the same function the job records
/// it with) and must equal the interrupted run's recorded `source` exactly. A record with no `source` (older records)
/// blocks the whole project, and so does a new job whose source cannot be worked out: when in doubt, block.
pub fn blocked_by_interrupted_run(cfg: &Config, project: &str, task: Option<&str>) -> Option<(String, String)> {
    let mine = job::job_source(cfg, project, task).ok();
    record::load_all(&record::jobs_dir(cfg))
        .into_iter()
        .find(|r| r.get("project") == project && interrupted_run(r) && blocks_source(r.get("source"), mine.as_deref()))
        .map(|r| (r.id().to_string(), shown_source(&r)))
}

/// Does an interrupted run recorded with `recorded` (its source folder, possibly empty) block a job running in `mine`?
fn blocks_source(recorded: &str, mine: Option<&str>) -> bool {
    recorded.is_empty() || mine.is_none_or(|m| m == recorded)
}

/// Kill the recorded child tree if, and only if, it is verifiably the same process.
pub fn kill_orphan(r: &Record) {
    let Some(pid) = r.get_num("child_pid") else { return };
    let recorded = r.get_num("child_start");
    match sys::process_state(pid as u32) {
        sys::ProcState::Gone => {}
        state if sys::exact_match(recorded, state) => {
            println!("  killing orphaned process tree (pid {pid})");
            sys::kill_tree(pid as u32);
        }
        sys::ProcState::Alive { start: Some(_) } if recorded.is_some() => {} // same pid, different process: not ours
        _ => println!("  pid {pid} is running but cannot be verified as this job's child (no exact creation time), so it is NOT killed; check it by hand"),
    }
}

fn save(r: &Record) -> Result<(), String> {
    r.save().map_err(|e| format!("cannot write job record {}: {e}", r.path.display()))
}

/// Take `state/recover.lock` (an OS file lock, see lock.rs): waited for briefly, then an error says another recover
/// is in progress. The lock frees itself when its holder ends, so the message never suggests deleting anything.
fn acquire(cfg: &Config, wait: Duration) -> Result<Lock, String> {
    let path = cfg.state_dir.join("recover.lock");
    lock::acquire(&path, wait, &|owner| match owner {
        Some(pid) => format!("another recover is in progress (pid {pid}); wait for it to finish (the lock is released when it ends)"),
        None => "another recover is in progress; wait for it to finish (the lock is released when it ends)".to_string(),
    })
}

/// Recover every dead running job. Returns non-zero if any job needs attention or a resumed job failed.
pub fn recover(cfg: &Config, mode: Mode) -> ExitCode {
    let _lock = match acquire(cfg, Duration::from_secs(cfg.recover_lock_wait_seconds)) {
        Ok(l) => l,
        Err(e) => {
            if mode == Mode::Auto {
                eprintln!("broadsword: warning: automatic recovery skipped: {e}");
                return ExitCode::SUCCESS;
            }
            eprintln!("broadsword: {e}");
            return ExitCode::from(1);
        }
    };
    let mut resumed_failed = false;
    let mut attention: Vec<String> = Vec::new();
    let mut any = false;
    let mut listed: Vec<String> = Vec::new();
    for mut r in record::load_all(&record::jobs_dir(cfg)) {
        if r.status() != record::RUNNING {
            continue;
        }
        any = true;
        let id = r.id().to_string();
        if broadsword_alive(&r) {
            println!("job {id}: still running (broadsword pid {}), left alone", r.get("pid"));
            continue;
        }
        println!("recovering job {id}: {} {} was interrupted (broadsword pid {} is gone)", r.get("kind"), r.get("project"), r.get("pid"));
        kill_orphan(&r);
        let kind = r.get("kind").to_string();
        let project = r.get("project").to_string();
        if kind == "run" {
            if mode != Mode::All {
                if mode == Mode::List {
                    let started = r.get_num("started").map(pause::format_utc).unwrap_or_else(|| "unknown".into());
                    println!("  source folder: {}", shown_source(&r));
                    println!("  started: {started}");
                    listed.push(id.clone());
                }
                println!("  this run job may have changed files, so nothing was changed: roll it back with `broadsword recover {id}`, or give up on it with `broadsword recover --abandon {id}` (new jobs in the same folder of `{project}` are refused until then)");
                continue;
            }
            if let Err(e) = rollback_run(cfg, &mut r, &project) {
                eprintln!("  {e}");
                attention.push(id);
            }
        } else {
            let count = r.get_num("resume_count").unwrap_or(0);
            if count >= cfg.max_resumes {
                let why = format!("interrupted and already resumed {count} time(s) in a row (max_resumes = {}); not resuming again", cfg.max_resumes);
                println!("  {why}");
                r.set("status", record::FAILED);
                r.set("failure_reason", why);
                if let Err(e) = save(&r) {
                    eprintln!("  {e}");
                }
                attention.push(id);
                continue;
            }
            let task = Some(r.get("task").to_string()).filter(|t| !t.is_empty());
            if let Some(t) = &task {
                // A job that ran in a task's worktree resumes there or not at all (never in the main checkout instead).
                if let Err((_, why)) = job::job_folder(cfg, &project, Some(t)) {
                    let why = format!("it ran in the worktree of task {t}, which can no longer be used: {why}");
                    println!("  {why}");
                    r.set("status", record::FAILED);
                    r.set("failure_reason", why);
                    if let Err(e) = save(&r) {
                        eprintln!("  {e}");
                    }
                    attention.push(id);
                    continue;
                }
            }
            let steps = job::steps_for(&kind);
            let first = steps.iter().position(|s| *s == r.get("step")).unwrap_or(0);
            println!("  read-only job: resuming `{kind} {project}` from step `{}`", steps[first]);
            let c = job::run_job(cfg, JobSpec { kind, name: project, command: None, first_step: first, resumed_from: Some(id), resume_count: count + 1, task });
            if c != ExitCode::SUCCESS {
                resumed_failed = true;
            }
        }
    }
    if !any {
        println!("recover: nothing to do");
    }
    if !attention.is_empty() {
        eprintln!("broadsword: NEEDS ATTENTION: {} could not be recovered automatically (see above). Records and refs are kept and the work saved under refs/broadsword/ is intact; fix the cause and run `broadsword recover <id>` again, or `broadsword recover --abandon <id>`", attention.join(", "));
        return ExitCode::from(1);
    }
    if !listed.is_empty() {
        eprintln!("broadsword: NEEDS ATTENTION: {} interrupted run job(s) were NOT rolled back (no file was changed): {}. Roll one back with `broadsword recover <id>` (only if nobody is still working in its source folder), or give up on it with `broadsword recover --abandon <id>`", listed.len(), listed.join(", "));
        return ExitCode::from(1);
    }
    ExitCode::from(u8::from(resumed_failed))
}

fn shown_source(r: &Record) -> String {
    if r.get("source").is_empty() { "unknown (older record: the whole project)".to_string() } else { r.get("source").to_string() }
}

/// `broadsword recover <id>`: roll back exactly this one interrupted `run` job and nothing else. Anything else (no such
/// record, not a `run`, not running, its Broadsword still alive) is refused with exit 2 and changes nothing.
pub fn recover_one(cfg: &Config, id: &str) -> ExitCode {
    let refuse = |why: String| {
        eprintln!("broadsword: refusing to recover `{id}`: {why}");
        ExitCode::from(2)
    };
    if id.is_empty() || record::sanitize(id) != id {
        return refuse("not a job id (see `broadsword jobs`)".into());
    }
    let _lock = match acquire(cfg, Duration::from_secs(cfg.recover_lock_wait_seconds)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("broadsword: {e}");
            return ExitCode::from(1);
        }
    };
    let path = record::jobs_dir(cfg).join(format!("{id}.rec"));
    let mut r = match record::load(&path) {
        Ok(r) if r.id() == id => r,
        _ => return refuse("no job record with that exact id (see `broadsword jobs`)".into()),
    };
    if r.status() != record::RUNNING || r.get("kind") != "run" {
        return refuse(format!("it is a `{}` job with status `{}`: only an interrupted `run` job can be rolled back", r.get("kind"), r.status()));
    }
    if broadsword_alive(&r) {
        return refuse(format!("its broadsword (pid {}) is still running, so the job is not interrupted; nothing was changed", r.get("pid")));
    }
    println!("recovering job {id}: run {} was interrupted (broadsword pid {} is gone)", r.get("project"), r.get("pid"));
    kill_orphan(&r);
    let project = r.get("project").to_string();
    match rollback_run(cfg, &mut r, &project) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("  {e}");
            eprintln!("broadsword: NEEDS ATTENTION: {id} could not be rolled back (see above). Records and refs are kept and the work saved under refs/broadsword/ is intact; fix the cause and run `broadsword recover {id}` again, or `broadsword recover --abandon {id}`");
            ExitCode::from(1)
        }
    }
}

fn rollback_run(cfg: &Config, r: &mut Record, project: &str) -> Result<(), String> {
    let id = r.id().to_string();
    for entry in std::fs::read_dir(&cfg.state_dir).into_iter().flatten().flatten() {
        if entry.file_name().to_string_lossy().starts_with(&format!("tmp-index-{id}")) {
            let _ = std::fs::remove_file(entry.path()); // this dead job's leftovers only
        }
    }
    let task = Some(r.get("task").to_string()).filter(|t| !t.is_empty());
    // A run that ran in a task's worktree is rolled back THERE: its checkpoint is of that folder, never of the main checkout.
    let dir = job::job_folder(cfg, project, task.as_deref()).map_err(|(_, e)| format!("cannot roll back {id}: {e}; record left as running"))?;
    let ckpt = r.get("checkpoint").to_string();
    if ckpt == "pending" {
        // killed before the checkpoint existed, which is before the command could start: nothing changed
        println!("  no checkpoint was taken yet, so the command never started: nothing to roll back");
        r.set("status", record::ROLLED_BACK);
        return save(r);
    }
    if ckpt == "none" || ckpt.is_empty() {
        return Err(format!("cannot roll back {id}: record has no checkpoint; record left as running"));
    }
    let nonempty = |k: &str| Some(r.get(k).to_string()).filter(|t| !t.is_empty());
    let (index_tree, ign) = (nonempty("index_tree"), nonempty("checkpoint_ignored"));
    // Lists in a record are `record::encode_list` values (a name may hold `|` or a line break); old records read the same.
    let lens = record::decode_list(r.get("ignored_skipped_len"));
    let skipped: Vec<(String, u64)> = record::decode_list(r.get("ignored_skipped"))
        .into_iter()
        .zip(lens)
        .filter(|(p, _)| !p.is_empty())
        .filter_map(|(p, n)| n.parse().ok().map(|n| (p, n)))
        .collect();
    let rb = git::rollback(&dir, &cfg.state_dir, &id, &ckpt, ign.as_deref(), &skipped, index_tree.as_deref(), &git::Opts::for_project(cfg, &dir))
        .map_err(|e| format!("rollback of {id} failed, record left as running so it can be retried: {e}"))?;
    r.set("interrupted", rb.interrupted_ref.as_str());
    r.set("interrupted_ignored", rb.interrupted_ignored_ref.clone().unwrap_or_default());
    r.set("interrupted_index", rb.interrupted_index_ref.clone().unwrap_or_default());
    r.set("not_restored", record::encode_list(&rb.not_restored));
    r.set("not_protected", record::encode_list(&rb.not_protected));
    r.set("secret_left", record::encode_list(&rb.secret_skipped));
    r.set("index_secrets_left", record::encode_list(&rb.index_secrets));
    let partial = !rb.not_restored.is_empty();
    if !partial {
        r.set("status", record::ROLLED_BACK);
    }
    save(r)?;
    println!(
        "  {}: restored {} file(s), removed {} new file(s), restored {} ignored file(s)",
        if partial { "PARTIAL rollback" } else { "rolled back to the checkpoint" },
        rb.restored,
        rb.removed,
        rb.ignored_restored
    );
    println!("  checkpoint (state before the job):  {ckpt}");
    println!("  interrupted (partial work, kept):   {}", rb.interrupted_ref);
    if let Some(i) = &rb.interrupted_ignored_ref {
        println!("  interrupted ignored files, kept:    {i}");
    }
    if let Some(i) = &rb.interrupted_index_ref {
        println!("  index before rollback, kept:        {i}");
    }
    for p in &rb.not_restored {
        println!("  not restored (current copy not captured, left exactly as it is): {p}");
    }
    for p in &rb.not_protected {
        println!("  not protected by the checkpoint (it was over the ignored-file size cap): {p}");
    }
    // Secret files are never saved, restored, overwritten or deleted, and never silently: each one is named (names only,
    // never content; control characters escaped, as a name can hold a line break).
    for p in record::decode_list(r.get("secret_skipped")) {
        println!("  secret file the checkpoint did not save, left as it is: {}", p.escape_debug());
    }
    for p in &rb.secret_skipped {
        println!("  secret file left exactly as it is (a rollback never restores, overwrites or deletes one): {}", p.escape_debug());
    }
    for p in &rb.index_secrets {
        println!("  secret index entry left as it is: {}", p.escape_debug());
    }
    println!("  see the partial work: git -C {} diff {ckpt} {}", dir.display(), rb.interrupted_ref);
    if partial {
        return Err(format!(
            "rollback of {id} is PARTIAL: {} file(s) were left untouched because their current bytes are in no snapshot (listed above); the record stays `running`. Save or move those files, then run `broadsword recover` again, or run `broadsword recover --abandon {id}` to give up on this rollback",
            rb.not_restored.len()
        ));
    }
    Ok(())
}

/// `broadsword recover --abandon <id>`: give up on an interrupted job without touching any file.
/// The record becomes `abandoned` (which lifts the per-folder refusal); the record and every ref stay.
pub fn abandon(cfg: &Config, id: &str) -> ExitCode {
    let _lock = match acquire(cfg, Duration::from_secs(cfg.recover_lock_wait_seconds)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("broadsword: {e}");
            return ExitCode::from(1);
        }
    };
    if id.is_empty() || record::sanitize(id) != id {
        eprintln!("broadsword: `{id}` is not a job id (see `broadsword jobs`)");
        return ExitCode::from(1);
    }
    let path = record::jobs_dir(cfg).join(format!("{id}.rec"));
    let mut r = match record::load(&path) {
        Ok(r) if r.id() == id => r,
        _ => {
            eprintln!("broadsword: no job record with the exact id `{id}` (see `broadsword jobs`)");
            return ExitCode::from(1);
        }
    };
    if r.status() != record::RUNNING {
        eprintln!("broadsword: job {id} is `{}`, not `running`: nothing to abandon", r.status());
        return ExitCode::from(1);
    }
    if broadsword_alive(&r) {
        eprintln!("broadsword: job {id} is still running (broadsword pid {}); not abandoning it", r.get("pid"));
        return ExitCode::from(1);
    }
    kill_orphan(&r);
    r.set("status", record::ABANDONED);
    r.set("ended", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string());
    if let Err(e) = save(&r) {
        eprintln!("broadsword: ERROR: {e}");
        return ExitCode::from(1);
    }
    println!("job {id} abandoned: no file was changed, and the record and all refs are kept");
    for k in ["checkpoint", "checkpoint_ignored", "interrupted", "interrupted_ignored", "interrupted_index"] {
        if !r.get(k).is_empty() && r.get(k) != "pending" && r.get(k) != "none" {
            println!("  {k}: {}", r.get(k));
        }
    }
    println!("  see everything saved for this job: git -C <project folder> for-each-ref \"refs/broadsword/*/{id}*\"");
    println!("  compare the files with the checkpoint: git -C <project folder> diff {}", r.get("checkpoint"));
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::blocks_source;

    #[test]
    fn a_run_blocks_only_its_own_source_folder() {
        assert!(blocks_source("worktrees/p/t-1", Some("worktrees/p/t-1")));
        assert!(!blocks_source("worktrees/p/t-1", Some("worktrees/p/t-2")));
        assert!(!blocks_source("worktrees/p/t-1", Some("projects/p")), "main is not the worktree");
        assert!(!blocks_source("projects/p", Some("worktrees/p/t-1")));
        assert!(!blocks_source("worktrees/p/t-1", Some("worktrees/p/t-10")), "exact match, not a prefix");
        assert!(!blocks_source("worktrees/p/t-1", Some("Worktrees/p/t-1")), "compared exactly as stored");
    }

    #[test]
    fn when_in_doubt_it_blocks() {
        assert!(blocks_source("", Some("worktrees/p/t-1")), "an older record with no source blocks the whole project");
        assert!(blocks_source("", None));
        assert!(blocks_source("worktrees/p/t-1", None), "a new job whose folder cannot be worked out is blocked");
    }
}
