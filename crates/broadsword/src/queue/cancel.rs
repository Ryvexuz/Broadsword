use crate::config::Config;
use crate::job;
use crate::record::Record;
use crate::task::{self, fail, now_secs};
use crate::worktree;
use crate::wtgit;
use std::process::ExitCode;
use super::entry::{half_merged, load_entry};
use super::locks::{entries_lock, lane_key, run_lock};
use super::{APPROVED, BLOCKED, CANCELLED, EXIT_BAD_INPUT, EXIT_ERROR, EXIT_RETRY, QUEUED, RUNNING, STALE, STEP_MERGE, STEPS};
use super::view::short;

// ---- cancel -----------------------------------------------------------------------------

/// CANCEL RULE, the ONE place: an entry whose commits may already be on main is never cancelled (it would leave main holding code
/// whose main check and `task done` never ran). Refused: a half-finished train merge (`half_merged`), and any entry whose reviewed
/// commit (or recorded main commit) is an ancestor of the project's trunk (`refs/heads/<trunk>`, never HEAD). When that cannot be decided (git or the project
/// fails), the cancel is refused too: fail closed. `None` = may be cancelled.
fn cancel_refusal(cfg: &Config, e: &Record) -> Option<String> {
    let id = e.id();
    if half_merged(e) {
        return Some(format!("queue entry {id} is a half-finished merge (a merge train took it into its merge at step `{}`): its commits may be on main already. Not cancelled; finish it with `broadsword queue run --all` or `broadsword queue run --entry {id}`", e.get("step")));
    }
    let mut commits: Vec<&str> = [e.get("reviewed_tip"), e.get("main_sha")].into_iter().filter(|c| !c.is_empty()).collect();
    commits.dedup();
    if commits.is_empty() {
        return None;
    }
    let on_main = (|| -> Result<Option<String>, String> {
        let dir = job::resolve_project(&cfg.root, &cfg.projects_dir, e.get("project"))?;
        let (_, head) = wtgit::trunk_commit(&dir)?;
        for c in &commits {
            if wtgit::is_ancestor(&dir, c, &head)? {
                return Ok(Some(c.to_string()));
            }
        }
        Ok(None)
    })();
    match on_main {
        Ok(None) => None,
        Ok(Some(c)) => Some(format!("queue entry {id}: its reviewed commit {} is already on main of project `{}`. Not cancelled (main would hold it with no main check and no `task done`); finish it with `broadsword queue run --entry {id}`", short(&c), e.get("project"))),
        Err(m) => Some(format!("queue entry {id}: cannot tell whether its reviewed commit is already on main ({m}); not cancelled")),
    }
}

pub fn cmd_cancel(cfg: &Config, id: &str) -> ExitCode {
    // A queued entry: only the entry changes, under the short lock. A running one: its project, for its run lock below.
    let project = {
        let _l = match entries_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        let mut e = match load_entry(cfg, id) {
            Ok(e) => e,
            Err(m) => return fail(EXIT_BAD_INPUT, m),
        };
        match e.status() {
            CANCELLED => {
                println!("queue entry {id} was already cancelled");
                return ExitCode::SUCCESS;
            }
            QUEUED | BLOCKED | STALE | APPROVED => {
                if let Some(why) = cancel_refusal(cfg, &e) {
                    return fail(EXIT_RETRY, why);
                }
                let was = e.status().to_string();
                e.set("status", CANCELLED);
                e.set("reason", format!("cancelled while {was}"));
                e.set("ended", now_secs().to_string());
                if let Err(err) = e.save() {
                    return fail(EXIT_ERROR, format!("cannot write the queue entry: {err}"));
                }
                println!("queue entry {id} is cancelled; nothing else was touched (task {} stays as it was)", e.get("task"));
                return ExitCode::SUCCESS;
            }
            RUNNING => e.get("project").to_string(), // below
            other => return fail(EXIT_RETRY, format!("queue entry {id} is `{other}` and cannot be cancelled (it is finished; `queue add` queues the task again if it needs another try)")),
        }
    };
    // A running entry can only be cancelled when nobody is running it: holding its run lock (`run_lock`: the global queue.lock, or
    // with queue_per_project its project's own) proves that. An entry's project never changes, so it was safe to read it first.
    // (with the setting off the key is not needed: the global lock guards every entry, exactly as before)
    let key = match cfg.queue_per_project.then(|| lane_key(cfg, &project)).transpose() {
        Ok(k) => k,
        Err(m) => return fail(EXIT_ERROR, format!("queue entry {id}: its project `{project}` cannot have a run lock ({m}); not cancelled")),
    };
    let _q = match run_lock(cfg, key) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let _l = match entries_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let mut e = match load_entry(cfg, id) {
        Ok(e) => e,
        Err(m) => return fail(EXIT_BAD_INPUT, m),
    };
    if e.status() != RUNNING {
        return fail(EXIT_RETRY, format!("queue entry {id} changed to `{}` while waiting; look at `queue show {id}`", e.status()));
    }
    let step = STEPS.iter().position(|s| *s == e.get("step")).unwrap_or(0);
    if step >= STEP_MERGE {
        return fail(EXIT_RETRY, format!("queue entry {id} was interrupted at step `{}`, at or after the merge: main may already hold the commit. Not cancelled; finish it with `broadsword queue run --entry {id}`", e.get("step")));
    }
    if let Some(why) = cancel_refusal(cfg, &e) {
        return fail(EXIT_RETRY, why);
    }
    let mut note = String::new();
    if let Ok(t) = task::load_task(cfg, e.get("task")) {
        if let (Ok(dir), true) = (worktree::project_dir(cfg, &t), worktree::has_worktree(&t)) {
            if let Ok(wt) = worktree::abs_path(cfg, t.get("worktree")) {
                if wt.exists() && wtgit::is_worktree_of(&dir, &wt) && wtgit::rebase_in_progress(&wt).unwrap_or(false) {
                    match wtgit::rebase_abort(&wt) {
                        Ok(()) => note = "; a rebase the dead run left half done was aborted".into(),
                        Err(m) => return fail(EXIT_ERROR, format!("queue entry {id}: the leftover rebase could not be aborted: {m}; the entry was not cancelled")),
                    }
                    let tip = wtgit::branch_tip(&dir, e.get("branch")).unwrap_or_default();
                    if !e.get("pre_rebase").is_empty() && tip != e.get("pre_rebase") {
                        return fail(EXIT_ERROR, format!("queue entry {id}: after the abort the branch is at {} but was at {} before the rebase; the entry was not cancelled; the old tip is also in {}", short(&tip), short(e.get("pre_rebase")), e.get("pre_rebase_ref")));
                    }
                }
            }
        }
    }
    e.set("status", CANCELLED);
    e.set("reason", format!("cancelled while interrupted at step `{}`", e.get("step")));
    e.set("ended", now_secs().to_string());
    if let Err(err) = e.save() {
        return fail(EXIT_ERROR, format!("cannot write the queue entry: {err}"));
    }
    println!("queue entry {id} is cancelled{note}; nothing else was touched");
    ExitCode::SUCCESS
}
