//! `broadsword recover`: deal with jobs whose Broadsword died mid-job.
//!
//! Every record still `running` whose Broadsword process is gone is recovered:
//! - the recorded child process tree is killed first if it is still alive (an orphaned cargo or command),
//!   but only when its recorded creation time matches exactly (a reused pid is never killed);
//! - `build` / `test` / `check` only read and rebuild, so they are resumed: re-run from the step they
//!   were on, as a new job linked to the old one (old record -> `resumed`), at most `max_resumes` times
//!   in a row, then the record is marked `failed`;
//! - `run` may have changed files, so it is rolled back to its checkpoint, after saving the partial work
//!   under `refs/broadsword/interrupted/<id>/<attempt>` (old record -> `rolled_back`). Nothing is deleted.
//!
//! A record whose Broadsword is still alive is another job genuinely running and is left alone.
//! Only an explicit `broadsword recover` rolls back. The automatic recovery before a job
//! (`Mode::Auto`) kills orphans and resumes read-only jobs, but for an interrupted `run` job it only
//! says so, and `blocked_by_interrupted_run` then stops new jobs on that project until it is recovered.
//! A lock file (`state/recover.lock`) keeps two recovers from working at the same time.

use crate::config::Config;
use crate::git;
use crate::job::{self, JobSpec};
use crate::lock::{self, Lock};
use crate::record::{self, Record};
use crate::sys;
use std::process::ExitCode;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// `broadsword recover`: does everything, including rolling back `run` jobs.
    Explicit,
    /// Before a job starts: never rolls anything back.
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

/// An interrupted `run` job on `project`, if any: new jobs there must wait for `broadsword recover`.
pub fn blocked_by_interrupted_run(cfg: &Config, project: &str) -> Option<String> {
    record::load_all(&record::jobs_dir(cfg)).into_iter().find(|r| r.get("project") == project && interrupted_run(r)).map(|r| r.id().to_string())
}

/// Kill the recorded child tree if, and only if, it is verifiably the same process.
fn kill_orphan(r: &Record) {
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
    let _lock = match acquire(cfg, Duration::from_secs(5)) {
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
            if mode == Mode::Auto {
                println!("  this run job may have changed files: run `broadsword recover` to roll it back, or `broadsword recover --abandon {id}` (new jobs on `{project}` are refused until then)");
                continue;
            }
            for entry in std::fs::read_dir(&cfg.state_dir).into_iter().flatten().flatten() {
                if entry.file_name().to_string_lossy().starts_with(&format!("tmp-index-{id}")) {
                    let _ = std::fs::remove_file(entry.path()); // this dead job's leftovers only
                }
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
            let steps = job::steps_for(&kind);
            let first = steps.iter().position(|s| *s == r.get("step")).unwrap_or(0);
            println!("  read-only job: resuming `{kind} {project}` from step `{}`", steps[first]);
            let c = job::run_job(cfg, JobSpec { kind, name: project, command: None, first_step: first, resumed_from: Some(id), resume_count: count + 1 });
            if c != ExitCode::SUCCESS {
                resumed_failed = true;
            }
        }
    }
    if !any {
        println!("recover: nothing to do");
    }
    if !attention.is_empty() {
        eprintln!("broadsword: NEEDS ATTENTION: {} could not be recovered automatically (see above). Records and refs are kept and the work saved under refs/broadsword/ is intact; fix the cause and run `broadsword recover` again, or `broadsword recover --abandon <id>`", attention.join(", "));
        return ExitCode::from(1);
    }
    ExitCode::from(u8::from(resumed_failed))
}

fn rollback_run(cfg: &Config, r: &mut Record, project: &str) -> Result<(), String> {
    let id = r.id().to_string();
    let dir = job::resolve_project(&cfg.root, &cfg.projects_dir, project).map_err(|e| format!("cannot roll back {id}: {e}; record left as running"))?;
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
    let lens: Vec<u64> = r.get("ignored_skipped_len").split('|').filter_map(|n| n.parse().ok()).collect();
    let skipped: Vec<(String, u64)> = r.get("ignored_skipped").split('|').filter(|p| !p.is_empty()).zip(lens).map(|(p, n)| (p.to_string(), n)).collect();
    let rb = git::rollback(&dir, &cfg.state_dir, &id, &ckpt, ign.as_deref(), &skipped, index_tree.as_deref(), &git::Opts::for_project(cfg, &dir))
        .map_err(|e| format!("rollback of {id} failed, record left as running so it can be retried: {e}"))?;
    r.set("interrupted", rb.interrupted_ref.as_str());
    r.set("interrupted_ignored", rb.interrupted_ignored_ref.clone().unwrap_or_default());
    r.set("interrupted_index", rb.interrupted_index_ref.clone().unwrap_or_default());
    r.set("not_restored", rb.not_restored.join("|"));
    r.set("not_protected", rb.not_protected.join("|"));
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
/// The record becomes `abandoned` (which lifts the per-project refusal); the record and every ref stay.
pub fn abandon(cfg: &Config, id: &str) -> ExitCode {
    let _lock = match acquire(cfg, Duration::from_secs(5)) {
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
