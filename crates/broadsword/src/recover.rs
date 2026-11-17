//! `broadsword recover`: deal with jobs whose Broadsword died mid-job.
//!
//! Every record still `running` whose Broadsword process is gone is recovered:
//! - the recorded child process tree is killed first if it is still alive (an orphaned cargo or command);
//! - `build` / `test` / `check` only read and rebuild, so they are resumed: re-run from the step they
//!   were on, as a new job linked to the old one (old record -> `resumed`);
//! - `run` may have changed files, so it is rolled back to its checkpoint, after saving the partial work
//!   as `refs/broadsword/interrupted/<id>` (old record -> `rolled_back`). Nothing is deleted.
//!
//! A record whose Broadsword is still alive is another job genuinely running and is left alone.

use crate::config::Config;
use crate::git;
use crate::job::{self, JobSpec};
use crate::record::{self, Record};
use crate::sys;
use std::process::ExitCode;

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
        Some(pid) => sys::is_alive(pid as u32, r.get_num("pid_start")),
        None => false, // a record with no pid can only be one that died before it started
    }
}

fn kill_orphan(r: &Record) {
    if let Some(pid) = r.get_num("child_pid") {
        if sys::is_alive(pid as u32, r.get_num("child_start")) {
            println!("  killing orphaned process tree (pid {pid})");
            sys::kill_tree(pid as u32);
        }
    }
}

fn save(r: &Record) {
    if let Err(e) = r.save() {
        eprintln!("broadsword: warning: cannot write job record {}: {e}", r.path.display());
    }
}

/// Recover every dead running job. Returns the first non-zero exit code of a resumed job, else 0.
pub fn recover(cfg: &Config) -> ExitCode {
    let mut code = 0u8;
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
        for leftover in [format!("tmp-index-{id}"), format!("tmp-index-{id}-int"), format!("tmp-index-{id}-restore")] {
            let _ = std::fs::remove_file(cfg.state_dir.join(leftover));
        }
        let kind = r.get("kind").to_string();
        let project = r.get("project").to_string();
        if kind == "run" {
            rollback_run(cfg, &mut r, &project);
        } else {
            let steps = job::steps_for(&kind);
            let first = steps.iter().position(|s| *s == r.get("step")).unwrap_or(0);
            println!("  read-only job: resuming `{kind} {project}` from step `{}`", steps[first]);
            let c = job::run_job(cfg, JobSpec { kind, name: project, command: None, first_step: first, resumed_from: Some(id) });
            if code == 0 {
                code = if c == ExitCode::SUCCESS { 0 } else { 1 };
            }
        }
    }
    if !any {
        println!("recover: nothing to do");
    }
    ExitCode::from(code)
}

fn rollback_run(cfg: &Config, r: &mut Record, project: &str) {
    let id = r.id().to_string();
    let Ok(dir) = job::resolve_project(&cfg.root, &cfg.projects_dir, project) else {
        eprintln!("  cannot roll back: unknown project `{project}`; record left as running");
        return;
    };
    let ckpt = r.get("checkpoint").to_string();
    if ckpt == "pending" || ckpt == "none" || ckpt.is_empty() {
        if ckpt == "pending" {
            // killed before the checkpoint existed, which is before the command could start: nothing changed
            println!("  no checkpoint was taken yet, so the command never started: nothing to roll back");
            r.set("status", record::ROLLED_BACK);
            save(r);
        } else {
            eprintln!("  cannot roll back: record has no checkpoint; record left as running");
        }
        return;
    }
    let index_tree = Some(r.get("index_tree").to_string()).filter(|t| !t.is_empty());
    match git::rollback(&dir, &cfg.state_dir, &id, &ckpt, index_tree.as_deref()) {
        Ok(rb) => {
            r.set("interrupted", rb.interrupted_ref.as_str());
            r.set("status", record::ROLLED_BACK);
            save(r);
            println!("  rolled back to the checkpoint: restored {} file(s), removed {} new file(s)", rb.restored, rb.removed);
            println!("  checkpoint (state before the job):  {ckpt}");
            println!("  interrupted (partial work, kept):   {}", rb.interrupted_ref);
            println!("  see the partial work: git -C {} diff {ckpt} {}", dir.display(), rb.interrupted_ref);
        }
        Err(e) => eprintln!("  rollback failed, record left as running so it can be retried: {e}"),
    }
}
