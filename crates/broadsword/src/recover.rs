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
use crate::record::{self, Record};
use crate::sys;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, Instant};

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

struct Lock {
    path: PathBuf,
    content: String,
}

impl Drop for Lock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).map(|c| c == self.content).unwrap_or(false) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Take `state/recover.lock` (exclusive create holding pid and creation time). A lock whose owner is
/// gone is taken over; a live one is waited for briefly, then an error says who holds it.
fn acquire(cfg: &Config, wait: Duration) -> Result<Lock, String> {
    use std::io::Write;
    std::fs::create_dir_all(&cfg.state_dir).map_err(|e| format!("cannot create {}: {e}", cfg.state_dir.display()))?;
    let path = cfg.state_dir.join("recover.lock");
    let me = std::process::id();
    let content = format!("{me}\n{}\n", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
    let deadline = Instant::now() + wait;
    let mut takeovers = 0u32;
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                f.write_all(content.as_bytes()).and_then(|_| f.sync_all()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                return Ok(Lock { path, content });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
        }
        let seen = std::fs::read_to_string(&path).unwrap_or_default();
        let mut lines = seen.lines();
        let owner = lines.next().and_then(|l| l.parse::<u32>().ok());
        let start = lines.next().and_then(|l| l.parse::<u64>().ok());
        if let Some(pid) = owner {
            if !sys::is_alive(pid, start) && takeovers < 5 {
                // Stale: move it aside (only one taker can win the rename), and put it back if what we moved was not what we saw.
                takeovers += 1;
                let aside = path.with_extension(format!("stale-{me}-{takeovers}"));
                if std::fs::rename(&path, &aside).is_ok() {
                    if std::fs::read_to_string(&aside).unwrap_or_default() == seen {
                        let _ = std::fs::remove_file(&aside);
                    } else if !path.exists() {
                        let _ = std::fs::rename(&aside, &path);
                    }
                }
                continue;
            }
        }
        if Instant::now() >= deadline {
            return Err(match owner {
                Some(pid) => format!("another recover is in progress (pid {pid}); wait for it, or if it is stuck delete {}", path.display()),
                None => format!("{} exists and cannot be read; if no recover is running, delete it", path.display()),
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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
                println!("  this run job may have changed files: run `broadsword recover` to roll it back (new jobs on `{project}` are refused until then)");
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
        eprintln!("broadsword: NEEDS ATTENTION: {} could not be recovered; nothing was deleted, fix the cause and run `broadsword recover` again", attention.join(", "));
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
    let rb = git::rollback(&dir, &cfg.state_dir, &id, &ckpt, ign.as_deref(), index_tree.as_deref(), &git::Opts::for_project(cfg, &dir))
        .map_err(|e| format!("rollback of {id} failed, record left as running so it can be retried (nothing was deleted): {e}"))?;
    r.set("interrupted", rb.interrupted_ref.as_str());
    r.set("interrupted_ignored", rb.interrupted_ignored_ref.clone().unwrap_or_default());
    r.set("status", record::ROLLED_BACK);
    save(r)?;
    println!("  rolled back to the checkpoint: restored {} file(s), removed {} new file(s), restored {} ignored file(s)", rb.restored, rb.removed, rb.ignored_restored);
    println!("  checkpoint (state before the job):  {ckpt}");
    println!("  interrupted (partial work, kept):   {}", rb.interrupted_ref);
    if let Some(i) = &rb.interrupted_ignored_ref {
        println!("  interrupted ignored files, kept:    {i}");
    }
    println!("  see the partial work: git -C {} diff {ckpt} {}", dir.display(), rb.interrupted_ref);
    Ok(())
}
