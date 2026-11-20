//! The reviewer step and merge queue: `broadsword queue add|list|show|run|cancel`.
//!
//! One durable record per queued branch, `<state_dir>/queue/<entry id>.entry` (the same atomic `key = value` records as
//! jobs and tasks, see record.rs; never deleted by cleanup), and one review file per attempt that reached the reviewer,
//! `<entry id>.review` (the second attempt: `<entry id>.2.review`), holding the reviewer's full raw output, cost and tokens.
//!
//! STATUSES (the exit code of `queue run` in brackets):
//!   queued               waiting for `queue run`
//!   running              a `queue run` is working on it, or died while doing so (see "Crash and resume")
//!   approved             MERGE TRAIN only (merge_train_max > 1, `queue run --all`): the review PASSed, nothing is merged, the task stays
//!                        running; the entry waits for a train (see MERGE TRAIN). An open entry (it blocks `queue add` for its task); `queue
//!                        cancel` drops it; `queue run` on it alone resumes it (its deferred check is made first) [0]
//!   merged              fast-forwarded into the main checkout, main's check passed, `task done` ran [0]
//!   blocked              a precondition failed or a step could not run (dirty main, task not running, dirty worktree, reviewer
//!                        model = coder model, diff too big, check could not run, reviewer unreachable or timed out); nothing
//!                        was merged and nothing was changed except what the entry says; fix the cause, `queue run --entry` [3]
//!   conflict             the rebase onto main conflicted; it was aborted and the branch is exactly as before; the coder
//!                        rebases or fixes it, then re-queues with `queue add` [3]
//!   stale                main (or the branch) moved after the rebase and the review; nothing merged; `queue run --entry` rebases again [3]
//!   check_failed         build or tests failed on the rebased branch; no model was asked; fix, commit, `queue add` again [5]
//!   rejected             the reviewer said FAIL (or gave no usable verdict); the task stays running and its worktree is kept;
//!                        fix, commit, `queue add` again [5]
//!   merged_check_failed  the branch IS on main but `check` failed there; main was NOT reset; a human decides [6]
//!   cancelled            dropped by `queue cancel`
//!
//! SHADOW REVIEW. When `shadow_reviewer` is `openai`, a second model reviews the same prompt after the gating verdict is recorded; its
//! verdict and cost are only recorded in the entry (`shadow_*`) and in `<entry id>.shadow.review`; it never changes the outcome (see `s_shadow`).
//!
//! Exit codes of `queue run`: 0 merged, 1 internal error (the entry stays `running`, resumable), 2 bad input, 3 blocked /
//! conflict / stale, 4 queue busy, 5 rejected / check_failed, 6 merged_check_failed.
//!
//! PIPELINE, steps in this order: preconditions, rebase, check, review, merge, main_check, done. Every step is recorded in the
//! entry (`step`, the `log` lines, and the facts each step learned) BEFORE its effect is relied on, and every step is safe to
//! run twice, so any step can be run again from its start.
//!
//! CRASH AND RESUME. `queue run` holds `<state_dir>/queue/queue.lock`, an OS file lock (lock.rs) the operating system releases
//! when the process ends however it ends. So an entry that says `running` while nobody holds the lock is interrupted; there is
//! no stale-lock logic. `queue run` (no argument) resumes the oldest such entry before looking at queued ones; `queue run
//! --entry <id>` resumes a named one. Resuming starts again at the recorded step:
//!   rebase    a rebase the dead run left half done is aborted first (so the branch is back at its `pre_rebase` tip), then the step
//!             runs again (it is skipped when the branch already contains main's HEAD);
//!   check     the check runs again (the job's own recovery handles a check job that died);
//!   review    if the verdict was already recorded it is NOT asked again (no second bill); otherwise the branch tip must still be the
//!             commit the check passed on (`checked_tip`), else the entry is `stale` ("branch changed after check") and nothing is
//!             reviewed; then the reviewer is called again (every real call counts toward reviewer_max_calls_per_entry);
//!   merge     if main's HEAD already contains the reviewed commit (is it, or a merge train's later commit on top of it), the
//!             fast-forward is recorded as done; otherwise it is attempted;
//!   main_check, done   run again (`task done` on a task that is already done is skipped).
//! A HALF-FINISHED TRAIN MERGE (an `approved` entry at step merge, main_check or done, or with `main_sha`: see `half_merged`) is
//! resumed the same way and just as first: `queue run` and `queue run --all` take it before anything else, at its recorded step, so
//! its main check runs on the merged main; it never joins a new train. `--all` starts no train while one is being finished.
//! To give up instead: `queue cancel <id>` on an interrupted entry aborts a leftover rebase, touches nothing else, and is refused
//! once the merge step began, for a half-finished train merge, and for any entry whose reviewed commit is on main (`cancel_refusal`:
//! main already holds it, resume instead).
//!
//! RUN --ALL. `queue run --all` drains the queue under ONE hold of queue.lock: interrupted entries first, then queued and stale ones
//! oldest first, each at most once; an internal error (exit 1) or merged_check_failed (exit 6) stops it; a summary table follows and
//! the exit code is the worst outcome (see `cmd_run_all`). This is the minimal drain the merge train needs; check-ahead and
//! `--blocked` are not part of it.
//!
//! MERGE TRAIN (`merge_train_max` > 1, only in `queue run --all`; 1 = off, every entry as before). The per-entry review is unchanged
//! (own diff, own brief, own gate reviewer, own shadow). What changes is what comes after it: an entry does NOT run its own check
//! (the review prompt says so) and stops at `approved` when its review PASSes. Approved entries of one project whose task claims do not
//! overlap form a train of up to merge_train_max entries (a lone one at the end of the drain too): their branches are chained one on
//! top of the other (rebase), ONE check runs on the chain tip, main is fast-forwarded ONCE and checked ONCE, and every entry is `merged`
//! with its train id, position and the shared check job. A failing check is bisected (see `run_train`): an entry that fails alone is
//! `check_failed`, the entries proven good merge; entries that only fail together are all held out and none merges (the first of them
//! `conflict`, the ones chained on it `blocked` by the left-train rule; see `run_train` 5). The review stays
//! bound to its commits: an entry whose rebased commits are not the same patches (`git patch-id --stable`) as the reviewed ones goes
//! back to `queued` and is reviewed again (`train_link`). Nothing unchecked ever reaches main: an entry whose check was deferred
//! (`check_deferred = yes`) is checked on its own first if it is ever merged outside a train (`merge_phase`). The two places nb-18
//! (check from a clean checkout of the exact commit) plugs in are `Run::check_commit` and `Run::check_alone`.
//!
//! LOCKS. `queue.lock` is held for the whole `queue run` (minutes), so one entry is processed at a time; a second `queue run` waits
//! `queue_lock_wait_seconds`, then exits 4. `entries.lock` is held for milliseconds, around "is this task already queued, create
//! the entry" and around choosing the next entry. Order is always queue.lock then entries.lock. The TASK BOARD lock is never held
//! during the rebase, check, review or merge: it is taken only by `task done` (inside `task::cmd_finish`), which is short.
//!
//! ONE COMMIT. The check records `checked_tip`, the commit it ran on. The review diff is `<base>...<checked_tip>`, the reviewer is
//! called only while the branch tip still equals it, and `merge --ff-only` merges exactly that commit. A tip that moved at any
//! point makes the entry `stale`; an unchecked commit is never reviewed or merged.
//!
//! SAFETY. The only commands that move history are `git rebase <main HEAD>` in the task's worktree (aborted on any failure, the tip
//! verified) and `git merge --ff-only <reviewed commit>` in the main checkout (so the commit merged is exactly the one reviewed;
//! never a merge commit). Never `--force`, `-D`, `reset --hard` or `push`. Before a rebase the pre-rebase tip is saved in the entry
//! and, create-only, under `refs/broadsword/queue/<entry id>/pre-rebase` (a later attempt: `pre-rebase-<N>`). All git commands are
//! in wtgit.rs.

use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec};
use crate::lock::{self, Lock};
use crate::record::{self, Record};
use crate::reviewer::{self, CallError};
use crate::task::{self, fail, now_secs};
use crate::worktree::{self, JobDirError};
use crate::{git, pause, sys, tools, wtgit};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const MERGED: &str = "merged";
pub const APPROVED: &str = "approved";
pub const BLOCKED: &str = "blocked";
pub const CONFLICT: &str = "conflict";
pub const STALE: &str = "stale";
pub const CHECK_FAILED: &str = "check_failed";
pub const REJECTED: &str = "rejected";
pub const MERGED_CHECK_FAILED: &str = "merged_check_failed";
pub const CANCELLED: &str = "cancelled";

pub const EXIT_ERROR: u8 = 1;
pub const EXIT_BAD_INPUT: u8 = 2;
/// blocked, conflict, stale; and for `add` / `cancel`: refused because of the state of the task or entry.
pub const EXIT_RETRY: u8 = 3;
pub const EXIT_BUSY: u8 = 4;
/// rejected, check_failed: the code needs changes.
pub const EXIT_REJECTED: u8 = 5;
pub const EXIT_MERGED_BAD: u8 = 6;

const EXT: &str = "entry";
const STEPS: [&str; 7] = ["preconditions", "rebase", "check", "review", "merge", "main_check", "done"];
const STEP_MERGE: usize = 4;

type R<T> = Result<T, ExitCode>;

// ---- storage and locks ------------------------------------------------------------------

pub fn queue_dir(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("queue")
}

fn entry_path(cfg: &Config, id: &str) -> PathBuf {
    queue_dir(cfg).join(format!("{id}.{EXT}"))
}

/// Every entry, oldest first. An unreadable entry is an error (never silently skipped: `run` decides on this list); the empty file
/// that exists for an instant while `add` creates an entry is skipped.
fn load_entries(cfg: &Config) -> Result<Vec<Record>, String> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(queue_dir(cfg)) else { return Ok(v) };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != EXT) {
            continue;
        }
        let mut last = String::new();
        let mut rec = None;
        for _ in 0..5 {
            match record::load(&p) {
                Ok(r) => {
                    rec = Some(r);
                    break;
                }
                Err(e) => {
                    last = e;
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        match rec {
            Some(r) if !r.id().is_empty() => v.push(r),
            Some(_) => {}
            None => return Err(last),
        }
    }
    v.sort_by(|a, b| (a.get_num("created"), a.id().len(), a.id()).cmp(&(b.get_num("created"), b.id().len(), b.id())));
    Ok(v)
}

fn load_entry(cfg: &Config, id: &str) -> Result<Record, String> {
    let nope = || format!("no queue entry with the exact id `{id}` (see `broadsword queue list`)");
    if id.is_empty() || record::sanitize(id) != id {
        return Err(nope());
    }
    match record::load(&entry_path(cfg, id)) {
        Ok(r) if r.id() == id => Ok(r),
        _ => Err(nope()),
    }
}

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

fn entries_lock(cfg: &Config) -> Result<Lock, ExitCode> {
    take(cfg, "entries.lock", "another queue command")
}

fn when(r: &Record, k: &str) -> String {
    r.get_num(k).map(pause::format_utc).unwrap_or_else(|| "-".into())
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// The first few lines of `s`, for a one-line reason.
fn some(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    format!("{} path(s): {}{}", lines.len(), lines.iter().take(5).map(|l| l.trim_end()).collect::<Vec<_>>().join("; "), if lines.len() > 5 { "; ..." } else { "" })
}

/// The gating reviewer model for a risk level and the note on why (empty unless the fallback was used). For `high`, a high model of
/// the coder's family is replaced by `reviewer_model_high_fallback`; Err names the problem when no allowed model is left.
pub fn gating_model<'a>(cfg: &'a Config, risk: &str, coder: &str) -> Result<(&'a str, String), String> {
    if risk != "high" {
        let m = cfg.reviewer_model_low.as_str();
        return if same_model(coder, m) { Err(format!("the reviewer model for low risk is `{m}`, the same as the coder (`{coder}`); a model never reviews its own code. Queue the task again with another --coder, or change reviewer_model_low in broadsword.toml")) } else { Ok((m, String::new())) };
    }
    let (hi, fb) = (cfg.reviewer_model_high.as_str(), cfg.reviewer_model_high_fallback.as_str());
    if !same_model(coder, hi) {
        Ok((hi, String::new()))
    } else if !same_model(coder, fb) {
        Ok((fb, format!("fallback: coder is {}", reviewer::model_family(coder))))
    } else {
        Err(format!("reviewer_model_high `{hi}` and reviewer_model_high_fallback `{fb}` are both the same as the coder (`{coder}`); a model never reviews its own code. Queue the task again with another --coder, or change reviewer_model_high or reviewer_model_high_fallback in broadsword.toml"))
    }
}

/// The `--effort` the gating call gets: `reviewer_effort_high` for `high` risk, none (empty) for `low`.
fn effort_for<'a>(cfg: &'a Config, risk: &str) -> &'a str {
    if risk == "high" {
        &cfg.reviewer_effort_high
    } else {
        ""
    }
}

/// Same model? Compared by family (`sonnet` and `claude-sonnet-4-5` are the same), case-insensitively.
fn same_model(a: &str, b: &str) -> bool {
    reviewer::model_family(a) == reviewer::model_family(b)
}

/// An entry is "open" while `queue run` can still act on it (queued, running, blocked, stale, approved): it blocks `queue add` for its task.
/// Finished entries do not: merged, cancelled, rejected, and also check_failed, conflict and merged_check_failed, which `queue run`
/// refuses to run again (the documented way on is a new `queue add`, or a human for merged_check_failed).
/// HALF-FINISHED MERGE, the ONE test: an `approved` entry that a merge train already took into its merge (`merge_members` records
/// step `merge` on every member BEFORE the fast-forward, `main_sha` after it, then `main_check` and `done`). Its commits may be on
/// main already, so it is never chained into a new train (`claim_approved` skips it), never cancelled (`cancel_refusal`), and it is
/// resumed FIRST, at its recorded step, exactly as an interrupted `running` entry is (`resume_first`): `s_merge` takes "the reviewed
/// commit is already on main" as merged, and `s_main_check` checks the merged main.
fn half_merged(e: &Record) -> bool {
    e.status() == APPROVED && (matches!(e.get("step"), "merge" | "main_check" | "done") || !e.get("main_sha").is_empty())
}

/// An entry a run takes before any other: an interrupted `running` one, or a half-finished train merge (`half_merged`).
fn resume_first(e: &Record) -> bool {
    e.status() == RUNNING || half_merged(e)
}

fn is_open(status: &str) -> bool {
    matches!(status, QUEUED | RUNNING | BLOCKED | STALE | APPROVED)
}

/// The entry fields that hold free text (output of the check, the reviewer, git): redacted before they are written.
const TEXT_KEYS: [&str; 10] = ["brief", "check_summary", "check_tail", "reason", "error", "log", "blocking", "minor", "shadow_blocking", "shadow_reason"];

/// What a shadow review records in the entry (cleared on a fresh attempt).
const SHADOW_KEYS: [&str; 9] = ["shadow_model", "shadow_model_used", "shadow_verdict", "shadow_reason", "shadow_blocking", "shadow_cost_usd", "shadow_tokens", "shadow_secs", "shadow_file"];

fn process_secrets(cfg: &Config) -> reviewer::Secrets {
    let env: Vec<(String, String)> = std::env::vars().collect();
    reviewer::Secrets::collect(&cfg.root, &env, cfg.redact_min_chars as usize)
}

// ---- add --------------------------------------------------------------------------------

/// A folder of the config (`state_dir`, `build_dir`), resolved the way `canon_root` was: through the canonical root, and fully
/// canonical when it exists (a junction inside it then resolves too).
fn canon_inside(cfg: &Config, canon_root: &Path, dir: &Path) -> PathBuf {
    let joined = canon_root.join(dir.strip_prefix(&cfg.root).unwrap_or(dir));
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// A brief file must be a small UTF-8 text file INSIDE the root (after canonicalising, so `..` and links count), not inside
/// `state_dir` (queue records, job logs) or `build_dir`, and not a secret file (`git::is_secret_path`, checked on the name given and
/// on the resolved name).
/// A brief is stored, shown and sent to a model, so it may never be any file the caller can read.
fn read_brief(cfg: &Config, path: &str) -> Result<String, String> {
    if crate::git::is_secret_path(path) {
        return Err(format!("`{path}` is a secret file (`.env`, `.envrc`, `.env.<name>`, `.env-*`, `.env_*` or a short name or stream of one); a brief is sent to a model and secrets must never be"));
    }
    let real = std::fs::canonicalize(path).map_err(|e| format!("cannot read the brief file `{path}`: {e}"))?;
    let canon_root = std::fs::canonicalize(&cfg.root).map_err(|e| format!("cannot resolve the project root: {e}"))?;
    if !real.starts_with(&canon_root) {
        return Err(format!("the brief `{path}` is outside the project root; a brief must be a file inside the root (it is sent to a model)"));
    }
    for (name, dir) in [("state_dir", &cfg.state_dir), ("build_dir", &cfg.build_dir)] {
        if real.starts_with(canon_inside(cfg, &canon_root, dir)) {
            return Err(format!("the brief `{path}` is inside {name}; queue records, job logs and build output are never briefs (they are sent to a model)"));
        }
    }
    if crate::git::is_secret_path(&real.to_string_lossy()) {
        return Err(format!("`{path}` resolves to a secret file ({}); a brief is sent to a model and secrets must never be", real.display()));
    }
    let p = real.as_path();
    let meta = std::fs::metadata(p).map_err(|e| format!("cannot read the brief file `{path}`: {e}"))?;
    let max = cfg.reviewer_brief_max_kb * 1024;
    if !meta.is_file() || meta.len() > max {
        return Err(format!("the brief `{path}` must be a file of at most {} KB (reviewer_brief_max_kb); it is {} bytes", cfg.reviewer_brief_max_kb, meta.len()));
    }
    let text = std::fs::read_to_string(p).map_err(|e| format!("the brief file `{path}` must be UTF-8 text: {e}"))?;
    if text.trim().is_empty() {
        return Err(format!("the brief file `{path}` is empty"));
    }
    Ok(text)
}

pub fn cmd_add(cfg: &Config, task_id: &str, risk: &str, coder: &str, brief_file: Option<&str>) -> ExitCode {
    if risk != "low" && risk != "high" {
        return fail(EXIT_BAD_INPUT, format!("--risk must be `low` or `high`, not `{risk}`"));
    }
    let coder = coder.trim();
    if coder.is_empty() || coder.chars().any(char::is_control) {
        return fail(EXIT_BAD_INPUT, "--coder must be the coder's model name on one line, e.g. sonnet");
    }
    let brief_text = match brief_file.map(|f| read_brief(cfg, f)) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => return fail(EXIT_BAD_INPUT, e),
    };
    let t = match task::load_task(cfg, task_id) {
        Ok(t) => t,
        Err(e) => return fail(EXIT_BAD_INPUT, e),
    };
    let refuse = |why: String| fail(EXIT_RETRY, format!("cannot queue task {task_id}: {why}; nothing was queued"));
    let wt = match worktree::job_dir(cfg, task_id, t.get("project")) {
        Ok(p) => p,
        Err(JobDirError::Unknown(m) | JobDirError::Refused(m)) => return refuse(m),
    };
    let dir = match worktree::project_dir(cfg, &t) {
        Ok(d) => d,
        Err(e) => return refuse(e),
    };
    let branch = worktree::branch_of(&t);
    match wtgit::current_branch(&wt) {
        Ok(Some(b)) if b == branch => {}
        Ok(other) => return refuse(format!("its worktree is on {} instead of `{branch}`", other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into()))),
        Err(e) => return refuse(e),
    }
    // Tracked and untracked changes only: a rebase leaves ignored files (an ignored .env, target/) alone. The strict rule, ignored
    // files included, guards worktree REMOVAL at `task done`, which keeps the worktree instead of failing.
    match wtgit::status_porcelain_plain(&wt) {
        Ok(s) if s.is_empty() => {}
        Ok(s) => return refuse(format!("its worktree is not clean (uncommitted or untracked changes; ignored files do not count here): {}", some(&s))),
        Err(e) => return refuse(e),
    }
    let base = t.get("base").to_string();
    if base.is_empty() {
        return refuse("the task record has no base commit".into());
    }
    match wtgit::rev_count(&dir, &format!("{base}..{branch}")) {
        Ok(0) => return refuse(format!("`{branch}` has no commit beyond its base {}; commit the work first", short(&base))),
        Ok(_) => {}
        Err(e) => return refuse(e),
    }
    let brief = process_secrets(cfg).redact(&brief_text.unwrap_or_else(|| {
        format!(
            "Task {}: {}\nProject: {}. The task claims these files and folders: {}.\nNo separate done check was given: judge whether the diff does what the task title says, and flag changes outside the claims.",
            t.id(),
            t.get("title"),
            t.get("project"),
            task::claims_of(&t).join(", ")
        )
    }));
    let _lock = match entries_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let all = match load_entries(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, format!("cannot read the queue, so nothing was queued: {e}")),
    };
    if let Some(x) = all.iter().find(|x| x.get("task") == task_id && is_open(x.status())) {
        return fail(EXIT_RETRY, format!("task {task_id} is already in the queue as {} ({}); a task has one open entry at a time (queued, running, blocked, stale and approved entries are open). Run it with `queue run --entry {}`, or cancel it with `queue cancel {}`, then add again", x.id(), x.status(), x.id(), x.id()));
    }
    // LEFT-TRAIN RULE (see `Run::leave_train`): an earlier entry of this task left a merge train with its branch chained on other
    // entries' commits; while the branch still carries them and main does not, a new entry would review their code as this task's
    for x in all.iter().filter(|x| x.get("task") == task_id) {
        let chained_on = chain_base_of(x);
        match chained_onto(&dir, &branch, &chained_on) {
            Ok(false) => {}
            Ok(true) => {
                let main = wtgit::current_branch(&dir).ok().flatten().unwrap_or_else(|| "<main branch>".into());
                return refuse(format!("its earlier queue entry {} {}", x.id(), left_train_text(&main, &branch, &chained_on)));
            }
            Err(e) => return refuse(e),
        }
    }
    let mut e = match record::create_ext(&queue_dir(cfg), &format!("q-{}", job::file_stamp(now_secs())), EXT) {
        Ok(r) => r,
        Err(err) => return fail(EXIT_ERROR, format!("cannot create a queue entry: {err}")),
    };
    e.set("task", task_id);
    e.set("project", t.get("project"));
    e.set("branch", branch);
    e.set("risk", risk);
    e.set("coder", coder);
    e.set("brief", brief);
    e.set("brief_source", if brief_file.is_some() { "file" } else { "task title and claims" });
    e.set("status", QUEUED);
    e.set("attempt", "0");
    e.set("created", now_secs().to_string());
    if let Err(err) = e.save() {
        return fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display()));
    }
    println!("{}", e.id());
    println!("queued task {task_id} ({risk} risk, coder {coder}); `broadsword queue run` will review it with `{}` and merge it if it passes", gating_model(cfg, risk, coder).map_or_else(|_| "no allowed model (the entry will be blocked)".to_string(), |(m, why)| if why.is_empty() { m.to_string() } else { format!("{m} ({why})") }));
    ExitCode::SUCCESS
}

// ---- list, show -------------------------------------------------------------------------

/// `running` entries whose runner is gone are shown as interrupted.
fn shown_status(e: &Record) -> String {
    if e.status() == RUNNING {
        let alive = e.get_num("pid").is_some_and(|p| sys::is_alive(p as u32, e.get_num("pid_start")));
        return if alive { "running".into() } else { "running (interrupted: `queue run` resumes it)".into() };
    }
    e.status().to_string()
}

pub fn cmd_list(cfg: &Config) -> ExitCode {
    let all = match load_entries(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    if all.is_empty() {
        println!("no queue entries");
    }
    for e in &all {
        let note = if e.get("reason").is_empty() { e.get("step") } else { e.get("reason") };
        println!("{}\t{}\t{}\t{}\t{}\t{}\t{}", e.id(), shown_status(e), e.get("task"), e.get("project"), e.get("risk"), e.get("coder"), note);
    }
    ExitCode::SUCCESS
}

pub fn cmd_show(cfg: &Config, id: &str) -> ExitCode {
    let e = match load_entry(cfg, id) {
        Ok(e) => e,
        Err(m) => return fail(EXIT_BAD_INPUT, m),
    };
    let dash = |k: &str| if e.get(k).is_empty() { "-".to_string() } else { e.get(k).to_string() };
    println!("id: {}", e.id());
    println!("status: {}", shown_status(&e));
    println!("reason: {}", dash("reason"));
    println!("task: {}", e.get("task"));
    println!("project: {}", e.get("project"));
    println!("branch: {}", e.get("branch"));
    println!("risk: {}", e.get("risk"));
    println!("coder: {}", e.get("coder"));
    println!("reviewer: {}", dash("reviewer_model"));
    println!("attempt: {}", dash("attempt"));
    println!("step: {}", dash("step"));
    println!("created: {}", when(&e, "created"));
    println!("started: {}", when(&e, "started"));
    println!("ended: {}", when(&e, "ended"));
    for (label, k) in [
        ("main branch", "main_branch"),
        ("base (main HEAD at rebase)", "base_head"),
        ("pre-rebase tip", "pre_rebase"),
        ("pre-rebase ref", "pre_rebase_ref"),
        ("rebased tip", "rebased_tip"),
        ("checked commit", "checked_tip"),
        ("reviewed commit", "reviewed_tip"),
        ("reviewer CLI", "reviewer_cli"),
        ("test hook", "test_hook"),
        ("review calls", "review_calls"),
        ("check job", "check_job"),
        ("check exit", "check_exit"),
        ("verdict", "verdict"),
        ("review cost (USD)", "cost_usd"),
        ("review tokens in", "tokens_in"),
        ("review tokens out", "tokens_out"),
        ("review file", "review_file"),
        ("shadow verdict", "shadow_verdict"),
        ("shadow reason", "shadow_reason"),
        ("shadow model", "shadow_model"),
        ("shadow model used", "shadow_model_used"),
        ("shadow cost (USD)", "shadow_cost_usd"),
        ("shadow tokens (in+out)", "shadow_tokens"),
        ("shadow seconds", "shadow_secs"),
        ("shadow file", "shadow_file"),
        ("main commit", "main_sha"),
        ("merge train", "train"),
        ("position in the train", "train_pos"),
        ("train check job (shared by the whole train)", "train_job"),
        ("train bisect", "train_bisect"),
        ("check deferred to the train", "check_deferred"),
        ("commit that was reviewed (before the train chained it)", "reviewed_orig_tip"),
        ("own commit before the train chained it", "alone_tip"),
        ("left a train chained on", "left_train_base"),
        ("worktree detached for its own check at", "alone_detached"),
        ("main check", "main_check"),
        ("last error", "error"),
    ] {
        if !e.get(k).is_empty() {
            println!("{label}: {}", e.get(k));
        }
    }
    if !e.get("check_summary").is_empty() {
        println!("check summary:\n  {}", e.get("check_summary").replace('\n', "\n  "));
    }
    for (label, k) in [("blocking findings", "blocking"), ("minor findings", "minor"), ("shadow blocking findings (recorded only, never gating)", "shadow_blocking")] {
        if !e.get(k).is_empty() {
            println!("{label}:\n  {}", e.get(k).replace('\n', "\n  "));
        }
    }
    println!("brief ({}):\n  {}", e.get("brief_source"), e.get("brief").replace('\n', "\n  "));
    if !e.get("log").is_empty() {
        println!("steps:\n  {}", e.get("log").trim_end().replace('\n', "\n  "));
    }
    ExitCode::SUCCESS
}

// ---- cancel -----------------------------------------------------------------------------

/// CANCEL RULE, the ONE place: an entry whose commits may already be on main is never cancelled (it would leave main holding code
/// whose main check and `task done` never ran). Refused: a half-finished train merge (`half_merged`), and any entry whose reviewed
/// commit (or recorded main commit) is an ancestor of the project's main HEAD. When that cannot be decided (git or the project
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
        let head = wtgit::head_commit(&dir)?;
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
    // A queued entry: only the entry changes, under the short lock.
    {
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
            RUNNING => {} // below
            other => return fail(EXIT_RETRY, format!("queue entry {id} is `{other}` and cannot be cancelled (it is finished; `queue add` queues the task again if it needs another try)")),
        }
    }
    // A running entry can only be cancelled when nobody is running it: holding the queue lock proves that.
    let _q = match queue_lock(cfg) {
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

// ---- run --------------------------------------------------------------------------------

pub fn cmd_run(cfg: &Config, entry: Option<&str>) -> ExitCode {
    let _q = match queue_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    // Choose and claim the entry under the short lock. Holding queue.lock already means no other run exists, so every
    // `running` entry is an interrupted one.
    let chosen = claim(cfg, |all| match entry {
        Some(id) => match all.iter().find(|x| x.id() == id) {
            Some(x) if matches!(x.status(), QUEUED | RUNNING | STALE | BLOCKED | APPROVED) => Ok(Some(x.clone())),
            Some(x) => Err(not_runnable(x)),
            None => Err(fail(EXIT_BAD_INPUT, format!("no queue entry with the exact id `{id}` (see `broadsword queue list`)"))),
        },
        None => Ok(all.iter().find(|x| resume_first(x)).or_else(|| all.iter().find(|x| matches!(x.status(), QUEUED | STALE))).or_else(|| all.iter().find(|x| x.status() == APPROVED)).cloned()),
    });
    let e = match chosen {
        Ok(Some(e)) => e,
        Ok(None) => {
            println!("the queue is empty: nothing is queued, interrupted or stale");
            return ExitCode::SUCCESS;
        }
        Err(c) => return c,
    };
    match begin(cfg, e) {
        Ok(mut run) => run.go(),
        Err((_, code)) => code,
    }
}

/// `queue run` refuses a finished entry: exit 2 with the way on.
fn not_runnable(x: &Record) -> ExitCode {
    let hint = match x.status() {
        REJECTED | CHECK_FAILED | CONFLICT => "fix the code, commit, and queue the task again with `queue add`",
        _ => "it is finished",
    };
    fail(EXIT_BAD_INPUT, format!("queue entry {} is `{}` and cannot be run; {hint}", x.id(), x.status()))
}

/// Under entries.lock: let `pick` choose an entry from all of them (oldest first) and CLAIM it: a fresh attempt (anything but an
/// interrupted `running` entry or an `approved` one) forgets what the last attempt learned (its log and review files stay) and becomes
/// `running`; the claimer's pid is recorded either way. The caller holds queue.lock.
fn claim(cfg: &Config, pick: impl FnOnce(&[Record]) -> Result<Option<Record>, ExitCode>) -> Result<Option<Record>, ExitCode> {
    let _l = entries_lock(cfg)?;
    let all = load_entries(cfg).map_err(|m| fail(EXIT_ERROR, m))?;
    let Some(mut e) = pick(&all)? else { return Ok(None) };
    // An `approved` entry (review PASSed, waiting for a merge train) that is run on its own is RESUMED at its step `review`, like an
    // interrupted one: its verdict stands, nothing is cleared (a deferred check is made before it can merge: see `merge_phase`).
    let resumed = matches!(e.status(), RUNNING | APPROVED);
    if e.status() == APPROVED {
        e.set("status", RUNNING);
    }
    if !resumed {
        // a fresh attempt: forget what the last attempt learned (its log and review file stay)
        for k in [
            "train", "train_pos", "train_job", "train_up", "train_checked", "train_bisect", "alone_tip", "reviewed_orig_tip", "check_deferred", "main_check", "step", "reason", "error", "pre_rebase", "pre_rebase_ref", "base_head", "rebased_tip", "checked_tip", "reviewed_tip", "reviewer_cli", "test_hook", "check_job", "check_exit", "check_summary", "check_tail",
            "reviewer_model", "verdict", "blocking", "minor", "cost_usd", "tokens_in", "tokens_out", "review_file", "main_sha", "main_branch", "ended",
        ] {
            e.set(k, "");
        }
        for k in SHADOW_KEYS {
            e.set(k, "");
        }
        e.set("attempt", (e.get_num("attempt").unwrap_or(0) + 1).to_string());
        e.set("status", RUNNING);
    }
    let me = std::process::id();
    e.set("pid", me.to_string());
    e.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
    e.set("started", now_secs().to_string());
    e.save().map_err(|err| fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display())))?;
    Ok(Some(e))
}

/// A claimed entry, ready to run: its project resolved (an unknown project ends it `blocked`: `Err((id, exit code))`).
fn begin(cfg: &Config, mut e: Record) -> Result<Run<'_>, (String, ExitCode)> {
    let resumed = !e.get("step").is_empty();
    let dir = match job::resolve_project(&cfg.root, &cfg.projects_dir, e.get("project")) {
        Ok(d) => d,
        Err(m) => {
            let id = e.id().to_string();
            let mut run = Run::new(cfg, e, PathBuf::new());
            return Err((id, run.end(BLOCKED, &format!("the project is unknown: {m}"), EXIT_RETRY)));
        }
    };
    println!("queue entry {}: {} (task {}, project {}, risk {}, coder {}, attempt {})", e.id(), if resumed { "RESUMING after an interrupted run, from step ".to_string() + e.get("step") } else { "starting".into() }, e.get("task"), e.get("project"), e.get("risk"), e.get("coder"), e.get("attempt"));
    e.set("error", "");
    Ok(Run::new(cfg, e, dir))
}

// ---- run --all ----------------------------------------------------------------------------
//
// TP4 SEAM: this is the MINIMAL drain the merge train needs (tp4, t-20261118-095351, is not merged at this base). tp4's own
// `cmd_run_all` adds check-ahead (`queue_check_ahead`) and `--blocked`; when tp4 is ported onto this, it keeps the train hooks marked
// "MERGE TRAIN" below (`approved`, `run_trains`, `settle_approved`, the `train` column) and adds its own parts around them.

/// The exit code a `queue run` returned, as a number (ExitCode hides it).
fn code_num(c: ExitCode) -> u8 {
    (0..=u8::MAX).find(|n| ExitCode::from(*n) == c).unwrap_or(EXIT_ERROR)
}

/// How bad an outcome is, for "the exit code of `--all` is the worst outcome": merged_check_failed (6) is worst, then an internal
/// error (1), rejected / check_failed (5), blocked / conflict / stale (3), bad input (2), busy (4), merged (0).
fn badness(code: u8) -> u8 {
    match code {
        6 => 7,
        1 => 6,
        5 => 5,
        3 => 4,
        2 => 3,
        4 => 2,
        0 => 0,
        _ => 6,
    }
}

/// The next entry `--all` takes: an interrupted one first, then the oldest queued or stale one; never one this drain already took
/// (`taken`), so an entry that ends stale or blocked is not run again in a loop. An `approved` entry (review PASSed, not merged) is
/// picked last, and only when the merge train is off (`train` false): with the train on, the drain itself collects the approved
/// entries into trains (`take_group`).
fn next_for_all(all: &[Record], taken: &[String], train: bool) -> Option<Record> {
    let fresh = |x: &&Record| !taken.iter().any(|t| t == x.id());
    all.iter()
        .filter(fresh)
        .find(|x| resume_first(x))
        .or_else(|| all.iter().filter(fresh).find(|x| matches!(x.status(), QUEUED | STALE)))
        .or_else(|| if train { None } else { all.iter().filter(fresh).find(|x| x.status() == APPROVED) })
        .cloned()
}

/// The exit code of `queue run --all`: the worst (see `badness`) of every row's code and of `stop`, the code that stopped the drain (0
/// when it ran to its end). A stop is counted even when no row carries it, so "the check could not give a verdict" (exit 1) is never
/// reported as the exit 3 of the rows it left `approved`.
fn worst_exit(rows: impl Iterator<Item = u8>, stop: u8) -> u8 {
    rows.chain(std::iter::once(stop)).max_by_key(|c| badness(*c)).unwrap_or(0)
}

/// One line of the `--all` summary table.
struct Row {
    id: String,
    task: String,
    status: String,
    code: u8,
    secs: f64,
    note: String,
    /// `<train id>#<position>` for an entry a merge train handled, else empty.
    train: String,
}

/// `queue run --all`: every runnable entry, in order, under ONE hold of queue.lock (so no second runner can race it and lose with
/// exit 4): interrupted entries first, then queued and stale ones oldest first. Each entry is taken at most once per drain. An entry
/// that ends blocked, conflict, stale, rejected or check_failed does not stop the rest; an internal error (exit 1: the entry stays
/// `running`) and merged_check_failed (exit 6: main holds a commit whose check failed) STOP the drain, because what comes next would
/// build on a main nobody has looked at. Prints a summary table; the exit code is the worst outcome (see `badness`).
///
/// MERGE TRAIN (merge_train_max > 1): an entry stops at `approved` after its review PASSed (its own check deferred), and approved
/// entries are merged in trains (see `run_train`): whenever a full train is ready, and at the end of the drain for whatever is left.
pub fn cmd_run_all(cfg: &Config) -> ExitCode {
    let _q = match queue_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let train = cfg.merge_train_max > 1;
    let t0 = std::time::Instant::now();
    let mut taken: Vec<String> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut stopped = String::new();
    // the exit code that stopped the drain (1 or 6), which may be on no row (a train check without a verdict leaves its entries
    // `approved` with exit 3 rows): it counts for the drain's exit code (`worst_exit`)
    let mut stop_code = 0u8;
    let mut approved: Vec<(Run, std::time::Instant)> = Vec::new();
    if train {
        // entries an earlier drain approved and then died before merging them: they join the trains first (they are the oldest)
        let left: Vec<Record> = match claim_approved(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        for e in left {
            let t = std::time::Instant::now();
            taken.push(e.id().to_string());
            match begin(cfg, e) {
                Ok(mut r) => {
                    r.defer_check = true;
                    r.load_claims();
                    approved.push((r, t));
                }
                Err((id, c)) => rows.push(row_for(cfg, &id, code_num(c), t)),
            }
        }
    }
    loop {
        let e = match claim(cfg, |all| Ok(next_for_all(all, &taken, train))) {
            Ok(Some(e)) => e,
            Ok(None) => {
                // MERGE TRAIN: nothing else to run: every approved entry left (even a single one) goes into a last train
                if train && !approved.is_empty() {
                    let code = run_trains(cfg, &mut approved, true, &mut rows);
                    if code == EXIT_ERROR || code == EXIT_MERGED_BAD {
                        stopped = train_stop_note(code);
                        stop_code = code;
                    }
                    settle_approved(cfg, &mut approved, &mut rows);
                }
                break;
            }
            Err(c) => {
                stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
                rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
                break;
            }
        };
        taken.push(e.id().to_string());
        let t = std::time::Instant::now();
        let mut run = match begin(cfg, e) {
            Ok(mut r) => {
                r.defer_check = train;
                r
            }
            Err((id, c)) => {
                rows.push(row_for(cfg, &id, code_num(c), t));
                continue;
            }
        };
        let start = run.start_step();
        let result = (|| -> R<ExitCode> {
            run.until_review(start)?;
            run.review_phase(start)?;
            if train && start <= 3 {
                run.approve() // MERGE TRAIN: the entry waits at `approved` for its train
            } else {
                run.merge_phase(start)
            }
        })();
        let code = code_num(match result {
            Ok(c) | Err(c) => c,
        });
        let id = run.e.id().to_string();
        if run.e.status() == APPROVED {
            // approved entries keep their Run until their train has finished with them
            approved.push((run, t));
        } else {
            rows.push(row_for(cfg, &id, code, t));
        }
        let mut train_code = 0u8;
        // no new train while a half-finished merge is being finished (`resume_first` entries come first): a train only starts once
        // main's last merge has had its main check
        if train && start < STEP_MERGE && !approved.is_empty() && code != EXIT_ERROR && code != EXIT_MERGED_BAD {
            train_code = run_trains(cfg, &mut approved, false, &mut rows);
        }
        let hard = if code == EXIT_ERROR || code == EXIT_MERGED_BAD { code } else { train_code };
        if hard == EXIT_ERROR || hard == EXIT_MERGED_BAD {
            stop_code = hard;
            stopped = if hard == train_code && train_code != 0 { train_stop_note(hard) } else { format!("stopped after entry {id} (exit {code}): {}", if code == EXIT_ERROR { "it stays `running` at its step; fix the cause, then `queue run` resumes it" } else { "main holds a commit whose check failed; a human decides before anything else merges" }) };
            settle_approved(cfg, &mut approved, &mut rows);
            break;
        }
    }
    let worst = worst_exit(rows.iter().map(|r| r.code), stop_code);
    if rows.is_empty() {
        println!("the queue is empty: nothing is queued, interrupted or stale");
    } else {
        println!("--- queue run --all: {} entr{} in {:.1} s ---", rows.len(), if rows.len() == 1 { "y" } else { "ies" }, t0.elapsed().as_secs_f64());
        // the train column only exists when a merge train handled an entry, so a drain without trains prints the plain table
        let trains = rows.iter().any(|r| !r.train.is_empty());
        println!("entry\ttask\tstatus\texit\tseconds\tnote{}", if trains { "\ttrain" } else { "" });
        for r in &rows {
            println!("{}\t{}\t{}\t{}\t{:.1}\t{}{}", r.id, r.task, r.status, r.code, r.secs, r.note, if trains { format!("\t{}", if r.train.is_empty() { "-" } else { r.train.as_str() }) } else { String::new() });
        }
        if !stopped.is_empty() {
            eprintln!("broadsword: queue run --all {stopped}");
        }
        println!("worst exit: {worst}");
    }
    ExitCode::from(worst)
}

/// A summary row read back from the entry's record (its final status and one-line reason).
fn row_for(cfg: &Config, id: &str, code: u8, t: std::time::Instant) -> Row {
    let e = load_entry(cfg, id).ok();
    let get = |k: &str| e.as_ref().map(|e| e.get(k).to_string()).unwrap_or_default();
    let mut note = get("reason");
    if note.chars().count() > 120 {
        note = note.chars().take(117).collect::<String>() + "...";
    }
    let train = match (get("train"), get("train_pos")) {
        (t, p) if !t.is_empty() => format!("{t}#{p}"),
        _ => String::new(),
    };
    Row { id: id.to_string(), task: get("task"), status: get("status"), code, secs: t.elapsed().as_secs_f64(), note, train }
}

struct Run<'a> {
    cfg: &'a Config,
    e: Record,
    /// The project's main checkout.
    dir: PathBuf,
    /// The task's worktree (set by `locate`).
    wt: PathBuf,
    /// Secret values (every `.env` value, secret-named variables): redacted from the prompt and from everything stored.
    secrets: reviewer::Secrets,
    /// Merge train mode: the entry's own check is deferred to its train (`s_defer_check`), and its review PASS ends it `approved`.
    defer_check: bool,
    /// The claims of the entry's task (what a train compares: entries whose claims overlap never share a train).
    claims: Vec<String>,
}

impl<'a> Run<'a> {
    fn new(cfg: &'a Config, e: Record, dir: PathBuf) -> Run<'a> {
        Run { cfg, e, dir, wt: PathBuf::new(), secrets: process_secrets(cfg), defer_check: false, claims: Vec::new() }
    }

    fn branch(&self) -> String {
        self.e.get("branch").to_string()
    }

    /// Write the entry. A queue that cannot write its own record must not go on: exit 1, loudly.
    fn save(&mut self) -> R<()> {
        for k in TEXT_KEYS {
            let v = self.e.get(k);
            if !v.is_empty() {
                let r = self.secrets.redact(v);
                self.e.set(k, r);
            }
        }
        self.e.save().map_err(|err| fail(EXIT_ERROR, format!("queue entry {}: cannot write {}: {err}; stopping. Its last saved step is `{}`; `queue run --entry {}` resumes it", self.e.id(), self.e.path.display(), self.e.get("step"), self.e.id())))
    }

    fn log(&mut self, step: &str, text: &str) -> R<()> {
        let line = format!("{} {step}: {}", pause::now_string(), text.replace('\n', " "));
        let log = if self.e.get("log").is_empty() { line } else { format!("{}\n{line}", self.e.get("log")) };
        self.e.set("log", log);
        self.save()
    }

    fn set_step(&mut self, step: &str) -> R<()> {
        self.e.set("step", step);
        println!("queue entry {}: step {step}", self.e.id());
        self.log(step, "begin")
    }

    /// The entry is finished with `status`; print the one-line outcome and return the exit code.
    fn end(&mut self, status: &str, reason: &str, code: u8) -> ExitCode {
        let reason = &self.secrets.redact(reason);
        self.e.set("status", status);
        self.e.set("reason", reason);
        self.e.set("ended", now_secs().to_string());
        let step = self.e.get("step").to_string();
        let _ = self.log(&step, &format!("{status}: {reason}"));
        for k in TEXT_KEYS {
            let r = self.secrets.redact(self.e.get(k));
            self.e.set(k, r);
        }
        let saved = self.e.save();
        if status == MERGED {
            println!("queue entry {}: merged: {reason}", self.e.id());
        } else if status == MERGED_CHECK_FAILED {
            eprintln!("broadsword: LOUD: queue entry {} is {status}: {reason}", self.e.id());
        } else {
            eprintln!("broadsword: queue entry {} is {status}: {reason}", self.e.id());
        }
        match saved {
            Ok(()) => ExitCode::from(code),
            Err(err) => {
                eprintln!("broadsword: ERROR: queue entry {}: cannot record the final status `{status}`: {err}; its real outcome is `{status}`", self.e.id());
                ExitCode::from(EXIT_ERROR)
            }
        }
    }

    fn blocked(&mut self, reason: &str) -> ExitCode {
        self.end(BLOCKED, reason, EXIT_RETRY)
    }

    /// Something unexpected: the entry stays `running` at its step, so a later `queue run` resumes it.
    fn internal(&mut self, msg: &str) -> ExitCode {
        let msg = &self.secrets.redact(msg);
        self.e.set("error", msg);
        let _ = self.e.save();
        eprintln!(
            "broadsword: queue entry {} stopped with an error at step `{}`: {msg}\nthe entry is still `running`; `broadsword queue run --entry {}` resumes it from that step, `broadsword queue cancel {}` drops it",
            self.e.id(),
            self.e.get("step"),
            self.e.id(),
            self.e.id()
        );
        ExitCode::from(EXIT_ERROR)
    }

    /// A git (or other) result: an `Err` is an internal error that ends the run.
    fn g<T>(&mut self, r: Result<T, String>) -> R<T> {
        r.map_err(|m| self.internal(&m))
    }

    fn go(&mut self) -> ExitCode {
        match self.pipeline() {
            Ok(code) | Err(code) => code,
        }
    }

    /// The step to start from: the recorded one (a resumed run), else the first.
    fn start_step(&self) -> usize {
        STEPS.iter().position(|s| *s == self.e.get("step")).unwrap_or(0)
    }

    fn pipeline(&mut self) -> R<ExitCode> {
        let start = self.start_step();
        self.until_review(start)?;
        self.review_phase(start)?;
        self.merge_phase(start)
    }

    /// Steps 1 to 3: preconditions, rebase, check (each only when `start` is at or before it).
    fn until_review(&mut self, start: usize) -> R<()> {
        if start < STEP_MERGE {
            self.s_preconditions(start)?;
        }
        if start <= 1 {
            self.s_rebase()?;
        }
        if start <= 2 {
            self.check_step()?;
        }
        Ok(())
    }

    /// The check step: the real check, or (merge train mode) the deferral that leaves the check to the train.
    fn check_step(&mut self) -> R<()> {
        if self.defer_check {
            self.s_defer_check()
        } else {
            self.s_check()
        }
    }

    /// Step 4, review.
    fn review_phase(&mut self, start: usize) -> R<()> {
        if start <= 3 {
            self.s_review()?;
        }
        Ok(())
    }

    /// Steps 5 to 7: merge, main check, done. An entry whose check was deferred to a merge train that never checked it (a train that
    /// died, or `queue run` of an `approved` entry on its own) is checked alone first: nothing unchecked ever merges.
    fn merge_phase(&mut self, start: usize) -> R<ExitCode> {
        if start <= STEP_MERGE {
            // a commit already on main (a train that died after its fast-forward) is not checked alone: it is merged, and the main
            // check below checks the main that holds it
            if self.e.get("check_deferred") == "yes" && self.e.get("train_checked") != self.e.get("reviewed_tip") && !self.reviewed_on_main()? {
                self.log("merge", "this entry's check was deferred to a merge train that has not checked this commit: it is checked on its own before it can merge")?;
                self.s_check()?;
            }
            self.s_merge()?;
        }
        if start <= 5 {
            self.s_main_check()?;
        }
        self.s_done()
    }

    /// Is the reviewed commit already on the main checkout's HEAD (equal to it or an ancestor of it)? False when none is recorded.
    fn reviewed_on_main(&mut self) -> R<bool> {
        let reviewed = self.e.get("reviewed_tip").to_string();
        if reviewed.is_empty() {
            return Ok(false);
        }
        let dir = self.dir.clone();
        let head = self.g(wtgit::head_commit(&dir))?;
        self.g(wtgit::is_ancestor(&dir, &reviewed, &head))
    }

    // -- merge train (see `run_train`) ------------------------------------------------------

    /// MERGE TRAIN, check step: the entry's own check is NOT run. The commit that is reviewed is recorded as `checked_tip` (review and
    /// merge are bound to it as always); `check_deferred = yes` says that no check has run on it, and the review prompt says so too.
    /// The build and tests run once, on the whole train; a deferred commit never merges before a check of it has passed (`merge_phase`).
    fn s_defer_check(&mut self) -> R<()> {
        self.set_step("check")?;
        let (dir, branch) = (self.dir.clone(), self.branch());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        self.e.set("check_job", "");
        self.e.set("check_exit", "");
        self.e.set("check_summary", "NOT RUN for this change alone: with a merge train the build and tests run once on the combined branches of every approved change, after the reviews, and nothing merges before that check passes. Judge the diff on its own merits.");
        self.e.set("check_tail", "");
        self.e.set("checked_tip", tip.clone());
        self.e.set("check_deferred", "yes");
        self.e.set("train_checked", "");
        self.save()?;
        self.log("check", &format!("deferred to the merge train: commit {} is reviewed without a check of its own", short(&tip)))
    }

    /// The claims of this entry's task, as a train compares them. A task that cannot be read claims everything (`*`), so it never
    /// shares a train.
    fn load_claims(&mut self) {
        self.claims = match task::load_task(self.cfg, self.e.get("task")) {
            Ok(t) => task::claims_of(&t),
            Err(_) => vec!["*".to_string()],
        };
    }

    /// MERGE TRAIN: the gating review PASSed. The entry stops here as `approved`: nothing is merged, the task stays running, the
    /// worktree is untouched. `run_train` merges it with the other approved entries of its project.
    fn approve(&mut self) -> R<ExitCode> {
        self.load_claims();
        let base = self.e.get("base_head").to_string();
        self.e.set("train_up", base);
        self.e.set("status", APPROVED);
        self.e.set("reason", "the review PASSed; it waits for a merge train (nothing is merged yet)");
        println!("queue entry {}: approved (the review PASSed); it waits for a merge train", self.e.id());
        self.log("review", "approved: the entry joins the next merge train of its project (claims that do not overlap, at most merge_train_max entries)")?;
        Ok(ExitCode::SUCCESS)
    }

    /// Before an approved entry joins a train: still exactly the reviewed commit, clean worktree, clean main on the same branch. A
    /// chain rebase that a killed drain left half done is aborted first (the branch is then back at its `pre_rebase` tip).
    fn train_ready(&mut self) -> R<()> {
        self.locate()?;
        self.abort_stale_rebase()?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let (reviewed, checked) = (self.e.get("reviewed_tip").to_string(), self.e.get("checked_tip").to_string());
        if self.e.get("verdict") != "PASS" || reviewed.is_empty() || reviewed != checked {
            return Err(self.leave_train(STALE, &format!("an approved entry without a recorded PASS for its checked commit (reviewed `{}`, checked `{}`) cannot join a merge train. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&checked), self.e.id()), EXIT_RETRY));
        }
        if let Some(why) = self.test_hook_refusal() {
            return Err(self.leave_train(BLOCKED, &why, EXIT_RETRY));
        }
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if tip != reviewed {
            return Err(self.leave_train(STALE, &format!("branch changed after the review: `{branch}` was reviewed at {} but is at {} now; nothing was merged. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let wt_status = self.g(wtgit::status_porcelain_plain(&wt))?;
        if !wt_status.is_empty() {
            return Err(self.leave_train(BLOCKED, &format!("the task's worktree got changes after the review ({}); it cannot join a merge train; nothing merged", some(&wt_status)), EXIT_RETRY));
        }
        let main_branch = self.g(wtgit::current_branch(&dir))?;
        if main_branch.as_deref() != Some(self.e.get("main_branch")) {
            return Err(self.leave_train(BLOCKED, "the main checkout is not on the branch this entry was reviewed against; nothing merged", EXIT_RETRY));
        }
        let dirty = self.g(wtgit::status_porcelain_plain(&dir))?;
        if !dirty.is_empty() {
            return Err(self.leave_train(BLOCKED, &format!("the main checkout has uncommitted or untracked changes ({}); commit or discard them, then run again; nothing merged", some(&dirty)), EXIT_RETRY));
        }
        if self.e.get("alone_tip").is_empty() {
            self.e.set("alone_tip", reviewed);
            self.save()?;
        }
        Ok(())
    }

    /// Put this entry's own commits (everything after `train_up`) on top of `onto`, the previous entry of the chain (or main's HEAD
    /// for the first). `Some(new tip)`; `None` when the entry cannot ride in this train and went back to `queued`: git could not apply
    /// its commits (its branch exactly as it was), or the rebased commits are not the same patches as the reviewed ones. The old tip is
    /// saved under refs/broadsword/queue/ first, as for every rebase.
    ///
    /// THE REVIEW BINDS ITS COMMITS: the review and the merge follow the entry to its new commit only when `git patch-id --stable` of
    /// the rebased commits (`onto..new tip`) equals that of the commits that were reviewed (`base_head..reviewed_orig_tip`), commit for
    /// commit. Anything else (a hunk whose context the entries before it changed) sends the entry back to `queued` ("patch changed by
    /// the train rebase"): it is reviewed again and never merges on the old verdict.
    fn train_link(&mut self, onto: &str, train_id: &str) -> R<Option<String>> {
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        let up = if self.e.get("train_up").is_empty() { self.e.get("base_head").to_string() } else { self.e.get("train_up").to_string() };
        let orig = if self.e.get("reviewed_orig_tip").is_empty() { self.e.get("reviewed_tip").to_string() } else { self.e.get("reviewed_orig_tip").to_string() };
        let base = self.e.get("base_head").to_string();
        let reviewed_ids = self.g(wtgit::patch_ids(&dir, &format!("{base}..{orig}")))?;
        self.e.set("train", train_id);
        let r = self.keep_pre_rebase(&tip)?;
        self.e.set("pre_rebase", tip.clone());
        self.e.set("pre_rebase_ref", r.clone());
        self.log("rebase", &format!("merge train {train_id}: chaining `{branch}` (tip {}) onto {}; its own commits start after {}; the old tip is saved in {r}", short(&tip), short(onto), short(&up)))?;
        match self.g(wtgit::rebase_onto(&wt, onto, &up))? {
            wtgit::Rebase::Done => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if !self.g(wtgit::is_ancestor(&dir, onto, &now))? {
                    return Err(self.internal(&format!("the chain rebase reported success but `{branch}` ({}) does not contain {}; the old tip {} is saved in {r}", short(&now), short(onto), short(&tip))));
                }
                if self.g(wtgit::rev_count(&dir, &format!("{onto}..{now}")))? == 0 {
                    self.e.set("train_up", onto);
                    return Err(self.leave_train(BLOCKED, &format!("after chaining onto the train `{branch}` has no commit left (its changes are already in the train); the old tip {} is saved in {r}", short(&tip)), EXIT_RETRY));
                }
                let now_ids = self.g(wtgit::patch_ids(&dir, &format!("{onto}..{now}")))?;
                if now_ids != reviewed_ids {
                    self.e.set("train_up", onto);
                    self.e.set("rebased_tip", now.clone());
                    self.requeue(&format!("patch changed by the train rebase: merge train {train_id} chained `{branch}` onto the entries before it, and its rebased commits {} are not the same patches (git patch-id --stable) as the reviewed commits {}..{}; the old verdict does not cover them, so the entry goes back to `queued` and is reviewed again (the pre-train tip is saved in {r})", short(&now), short(&base), short(&orig)));
                    return Ok(None);
                }
                if self.e.get("reviewed_orig_tip").is_empty() {
                    self.e.set("reviewed_orig_tip", orig.clone());
                }
                self.e.set("train_up", onto);
                self.e.set("rebased_tip", now.clone());
                self.e.set("checked_tip", now.clone());
                self.e.set("reviewed_tip", now.clone());
                self.e.set("check_deferred", "yes");
                self.e.set("train_checked", "");
                self.save()?;
                self.log("rebase", &format!("chained: tip {} -> {}; the same {} patch(es) as reviewed (git patch-id --stable)", short(&tip), short(&now), now_ids.len()))?;
                Ok(Some(now))
            }
            wtgit::Rebase::Conflict(files) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("the chain rebase conflicted and was aborted, but `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                self.requeue(&format!("merge train {train_id}: chaining `{branch}` onto the entries before it conflicts in: {}. The rebase was aborted and the branch is exactly as before; the entry goes back to `queued` and is reviewed again on its own", files.join(", ")));
                Ok(None)
            }
            wtgit::Rebase::Failed(m) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("{m}; and `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                self.requeue(&format!("merge train {train_id}: `{branch}` could not be chained onto the entries before it: {m}. The branch is unchanged; the entry goes back to `queued`"));
                Ok(None)
            }
        }
    }

    /// LEFT-TRAIN RULE for a run outside a train (`s_preconditions`, `s_merge`): an entry whose branch is still chained on other
    /// entries' commits that are not on main is never reviewed or merged on that diff; it ends `blocked` (`leave_train`). Once the
    /// branch no longer carries that commit (the coder rebased, or those entries reached main) the record forgets it and the run goes on.
    fn refuse_chained(&mut self, step: &str) -> R<()> {
        let base = chain_base_of(&self.e);
        if base.is_empty() {
            return Ok(());
        }
        let (dir, branch) = (self.dir.clone(), self.branch());
        if self.g(chained_onto(&dir, &branch, &base))? {
            return Err(self.leave_train(BLOCKED, "it was run while its branch still carries those entries' commits; nothing was reviewed or merged", EXIT_RETRY));
        }
        if !self.e.get("left_train_base").is_empty() {
            self.e.set("left_train_base", "");
            self.log(step, &format!("the branch no longer carries {}, the commit a merge train chained it on that is not on main: it runs as it is", short(&base)))?;
        }
        Ok(())
    }

    /// An entry a train cannot take goes back to `queued` with the reason; the next `queue run` checks and reviews it afresh. Unless
    /// its branch is chained on other entries (see `leave_train`): then it ends `blocked`.
    /// Both outcomes are exit 3 (the caller records that); a record that cannot be written has printed its own error.
    fn requeue(&mut self, reason: &str) {
        let _ = self.leave_train(QUEUED, reason, EXIT_RETRY);
    }

    /// LEFT-TRAIN RULE, the ONE place it is applied: every way an entry leaves a merge train without merging (back to queued,
    /// check_failed, conflict, stale, blocked) ends here, and so does a solo run of an entry still chained (`s_preconditions`,
    /// `s_merge`). An entry whose branch still carries commits of other entries that are not on main (`chained_onto`) never goes on to
    /// a review or merge of that chained diff: it ends `blocked` ("left a train: ..."), and `left_train_base` keeps the commit it is
    /// chained on across attempts, so a later `queue run` or `queue add` refuses it too until the coder has rebased its own commits
    /// onto main (or those entries reached main). Otherwise the entry gets `status` as asked. A git error counts as chained.
    fn leave_train(&mut self, status: &str, reason: &str, code: u8) -> ExitCode {
        let (dir, branch, base) = (self.dir.clone(), self.branch(), chain_base_of(&self.e));
        let chained = chained_onto(&dir, &branch, &base).unwrap_or(true);
        if !chained {
            if status == QUEUED {
                let reason = self.secrets.redact(reason);
                self.e.set("status", QUEUED);
                self.e.set("reason", reason.clone());
                let _ = self.log("train", &format!("back to queued: {reason}"));
                eprintln!("broadsword: queue entry {} goes back to `queued`: {reason}", self.e.id());
                return ExitCode::from(EXIT_RETRY);
            }
            return self.end(status, reason, code);
        }
        let main = if self.e.get("main_branch").is_empty() { wtgit::current_branch(&dir).ok().flatten().unwrap_or_else(|| "<main branch>".into()) } else { self.e.get("main_branch").to_string() };
        self.e.set("left_train_base", base.clone());
        let text = left_train_text(&main, &branch, &base);
        self.blocked(&format!("{text}. Why it left (it would have been `{status}`): {reason}"))
    }

    /// NB-18 PLUG-IN POINT 1 of 2. Check `commit` of this entry's project: the build and tests, as `check <project>` runs them.
    /// Interim implementation (nb-18 is not merged): the check runs in this entry's task worktree (`check <project> --task <id>`), and
    /// only for a commit that worktree has checked out right now. nb-18 (check from a clean checkout of the exact commit) replaces the
    /// body with "check `commit` in a clean checkout", and then `check_alone` no longer needs to move the worktree.
    fn check_commit(&mut self, commit: &str, what: &str) -> TrainCheck {
        let (project, task_id) = (self.e.get("project").to_string(), self.e.get("task").to_string());
        if let Some(code) = tools::guard_refusal(self.cfg, &project, Some(&task_id)) {
            return TrainCheck::Blocked(format!("the check was refused before it started (exit {code}; the reason is printed above, e.g. Broadsword paused or an interrupted run to recover)"));
        }
        let head = match wtgit::head_commit(&self.wt) {
            Ok(h) => h,
            Err(m) => return TrainCheck::Blocked(m),
        };
        if head != commit {
            return TrainCheck::Blocked(format!("the worktree of task {task_id} is at {} instead of {}: the interim check can only check a commit its worktree has checked out", short(&head), short(commit)));
        }
        println!("queue entry {}: check {project} --task {task_id} ({what}; commit {})", self.e.id(), short(commit));
        let mut info = JobInfo::default();
        let code = job::run_job_info(self.cfg, JobSpec { kind: "check".into(), name: project.clone(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: Some(task_id) }, &mut info);
        let summary = info.summary.join("\n");
        let tail = info.log.as_deref().map(|p| read_tail(p, self.cfg.reviewer_check_tail_kb * 1024)).unwrap_or_default();
        let job_id = info.id.clone().unwrap_or_default();
        match code {
            0 => TrainCheck::Pass { job: job_id, summary, tail },
            1 if !info.summary.is_empty() => TrainCheck::Fail { job: job_id, summary, tail },
            c => TrainCheck::Blocked(format!("the check could not run to a verdict (exit {c}; job {})", if job_id.is_empty() { "none" } else { job_id.as_str() })),
        }
    }

    /// NB-18 PLUG-IN POINT 2 of 2. Check this entry's OWN commit (`alone_tip`: the reviewed commit on main as it was, before a train
    /// chained it) without the entries before it. Interim: the worktree is put on that commit (detached, no branch moves), checked, and
    /// put back on its branch. With nb-18 this is simply `check_commit(alone_tip)` in a clean checkout.
    fn check_alone(&mut self) -> TrainCheck {
        let alone = self.e.get("alone_tip").to_string();
        if alone.is_empty() {
            return TrainCheck::Blocked("no alone commit is recorded for this entry".into());
        }
        let (wt, branch) = (self.wt.clone(), self.branch());
        // recorded BEFORE the worktree is detached: a run killed while it is detached leaves this key, and the next run's `locate`
        // puts the worktree back on its branch (`restore_alone_checkout`)
        self.e.set("alone_detached", alone.clone());
        if self.save().is_err() {
            return TrainCheck::Blocked("could not record that the worktree is about to be detached; it was not moved".into());
        }
        if let Err(m) = wtgit::checkout_detach(&wt, &alone) {
            return TrainCheck::Blocked(format!("cannot put the worktree on the entry's own commit {}: {m}", short(&alone)));
        }
        let res = self.check_commit(&alone, "alone: the entry's own commit without the entries before it");
        if let Err(m) = wtgit::checkout_branch(&wt, &branch) {
            return TrainCheck::Blocked(format!("the worktree could NOT be put back on `{branch}` ({m}); do it by hand: git -C {} checkout {branch}", wt.display()));
        }
        self.e.set("alone_detached", "");
        if self.save().is_err() {
            return TrainCheck::Blocked("the worktree is back on its branch, but the record could not be written".into());
        }
        res
    }

    // -- step 1: preconditions --------------------------------------------------------------

    /// Find the task's worktree (and prove it is the task's), or end blocked.
    fn locate(&mut self) -> R<()> {
        match worktree::job_dir(self.cfg, self.e.get("task"), self.e.get("project")) {
            Ok(p) => {
                self.wt = p;
                self.restore_alone_checkout()
            }
            Err(JobDirError::Unknown(m) | JobDirError::Refused(m)) => Err(self.blocked(&m)),
        }
    }

    /// A run killed inside `check_alone` left the task's worktree detached on the entry's own commit (`alone_detached` records it).
    /// Every run that uses the worktree passes `locate`, which calls this: the worktree goes back on its branch, and only when it is
    /// exactly where the queue put it (detached at that commit); a worktree the coder moved since is never touched (the entry ends
    /// `blocked`, with the command to run by hand). Nothing recorded: nothing done.
    fn restore_alone_checkout(&mut self) -> R<()> {
        let at = self.e.get("alone_detached").to_string();
        if at.is_empty() {
            return Ok(());
        }
        let (wt, branch) = (self.wt.clone(), self.branch());
        match self.g(wtgit::current_branch(&wt))? {
            Some(b) if b == branch => {}
            Some(other) => return Err(self.blocked(&format!("a killed merge-train check left a note that the worktree was detached at {}, but it is on branch `{other}` now, not `{branch}`; it was not touched. Put it back with `git -C {} checkout {branch}`", short(&at), wt.display()))),
            None => {
                let head = self.g(wtgit::head_commit(&wt))?;
                if head != at {
                    return Err(self.blocked(&format!("the worktree is detached at {}, not at {} where a killed merge-train check put it; it was not touched. Put it back with `git -C {} checkout {branch}`", short(&head), short(&at), wt.display())));
                }
                if let Err(m) = wtgit::checkout_branch(&wt, &branch) {
                    return Err(self.blocked(&format!("a killed merge-train check left the worktree detached at {}, and it could not be put back on `{branch}`: {m}. Do it by hand: git -C {} checkout {branch}", short(&at), wt.display())));
                }
                self.e.set("alone_detached", "");
                return self.log("train", &format!("a killed merge-train check had left the worktree detached on the entry's own commit {}: put it back on `{branch}`", short(&at)));
            }
        }
        self.e.set("alone_detached", "");
        self.save()
    }

    fn s_preconditions(&mut self, start: usize) -> R<()> {
        self.set_step("preconditions")?;
        self.locate()?;
        // A run that died in the middle of ITS rebase (the recorded step is `rebase`) left the worktree half-rebased: put it back
        // before judging anything. A fresh attempt never aborts a rebase: one in progress is the coder's own and blocks as dirty.
        if start == 1 {
            self.abort_stale_rebase()?;
        }
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        if !git::is_repo(&dir) {
            return Err(self.blocked(&format!("project `{}` is not a Git repo", self.e.get("project"))));
        }
        let main_branch = match self.g(wtgit::current_branch(&dir))? {
            Some(b) => b,
            None => return Err(self.blocked("the main checkout is on a detached HEAD; check out a branch first")),
        };
        if start == 0 || self.e.get("main_branch").is_empty() {
            self.e.set("main_branch", main_branch.clone());
        } else if self.e.get("main_branch") != main_branch {
            let was = self.e.get("main_branch").to_string();
            return Err(self.blocked(&format!("the main checkout is on `{main_branch}` but this entry started on `{was}`")));
        }
        let dirty = self.g(wtgit::status_porcelain_plain(&dir))?;
        if !dirty.is_empty() {
            return Err(self.blocked(&format!("the main checkout has uncommitted or untracked changes ({}); commit or discard them, then run again", some(&dirty))));
        }
        match self.g(wtgit::current_branch(&wt))? {
            Some(b) if b == branch => {}
            other => {
                let on = other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into());
                return Err(self.blocked(&format!("the task's worktree is on {on}, not `{branch}`")));
            }
        }
        // tracked and untracked only: a rebase leaves ignored files alone (see `cmd_add`)
        let wt_status = self.g(wtgit::status_porcelain_plain(&wt))?;
        if !wt_status.is_empty() {
            return Err(self.blocked(&format!("the task's worktree is not clean ({}); commit or remove those changes, then run again (ignored files do not matter here)", some(&wt_status))));
        }
        self.refuse_chained("preconditions")?;
        let coder = self.e.get("coder").to_string();
        let (model, why) = match gating_model(self.cfg, self.e.get("risk"), &coder) {
            Ok((m, why)) => (m.to_string(), if why.is_empty() { String::new() } else { format!(" ({why})") }),
            Err(msg) => return Err(self.blocked(&msg)),
        };
        let ahead = self.g(wtgit::rev_count(&dir, &format!("HEAD..{branch}")))?;
        if ahead == 0 {
            return Err(self.blocked(&format!("nothing to merge: `{branch}` has no commit that `{main_branch}` lacks")));
        }
        self.log("preconditions", &format!("ok: main `{main_branch}` clean, worktree clean, task running, {ahead} commit(s) to merge, reviewer `{model}`{why}, effort `{}`, differs from coder `{coder}`", effort_for(self.cfg, self.e.get("risk"))))
    }

    fn abort_stale_rebase(&mut self) -> R<()> {
        let wt = self.wt.clone();
        if self.g(wtgit::rebase_in_progress(&wt))? {
            self.g(wtgit::rebase_abort(&wt))?;
            let (tip, before) = (self.g(wtgit::branch_tip(&self.dir.clone(), &self.branch()))?, self.e.get("pre_rebase").to_string());
            if !before.is_empty() && tip != before {
                return Err(self.internal(&format!("after aborting the rebase an interrupted run left, the branch is at {} but was at {} before it; the old tip is saved in {}", short(&tip), short(&before), self.e.get("pre_rebase_ref"))));
            }
            self.log("rebase", "found a rebase left half done by an interrupted run and aborted it; the branch is back where it was")?;
        }
        Ok(())
    }

    // -- step 2: rebase ---------------------------------------------------------------------

    /// Save `tip` under refs/broadsword/queue/<id>/pre-rebase[-N], create-only (an existing ref at another commit is never
    /// moved: a numbered or suffixed name is used instead). Returns the ref name.
    fn keep_pre_rebase(&mut self, tip: &str) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let leaf = if attempt <= 1 { "pre-rebase".to_string() } else { format!("pre-rebase-{attempt}") };
        let name = format!("refs/broadsword/queue/{}/{leaf}", self.e.id());
        let dir = self.dir.clone();
        if wtgit::keep_ref(&dir, &name, tip).is_ok() {
            return Ok(name);
        }
        let alt = format!("{name}-{}", &tip[..tip.len().min(12)]);
        self.g(wtgit::keep_ref(&dir, &alt, tip).map(|_| alt))
    }

    fn s_rebase(&mut self) -> R<()> {
        self.set_step("rebase")?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let head = self.g(wtgit::head_commit(&dir))?;
        self.e.set("base_head", head.clone());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if self.g(wtgit::is_ancestor(&dir, &head, &tip))? {
            self.e.set("rebased_tip", tip.clone());
            return self.log("rebase", &format!("`{branch}` already contains main's HEAD {}: no rebase needed (tip {})", short(&head), short(&tip)));
        }
        self.e.set("pre_rebase", tip.clone());
        let r = self.keep_pre_rebase(&tip)?;
        self.e.set("pre_rebase_ref", r.clone());
        self.log("rebase", &format!("rebasing `{branch}` (tip {}) onto main's HEAD {}; the old tip is saved in {r}", short(&tip), short(&head)))?;
        match self.g(wtgit::rebase(&wt, &head))? {
            wtgit::Rebase::Done => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if !self.g(wtgit::is_ancestor(&dir, &head, &now))? {
                    return Err(self.internal(&format!("the rebase reported success but `{branch}` ({}) does not contain main's HEAD {}; the old tip {} is saved in {r}", short(&now), short(&head), short(&tip))));
                }
                if self.g(wtgit::rev_count(&dir, &format!("{head}..{now}")))? == 0 {
                    return Err(self.blocked(&format!("after the rebase `{branch}` has no commit that main lacks (its changes are already in main); the old tip {} is saved in {r}", short(&tip))));
                }
                self.e.set("rebased_tip", now.clone());
                self.log("rebase", &format!("rebased: tip {} -> {}", short(&tip), short(&now)))
            }
            wtgit::Rebase::Conflict(files) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("the rebase conflicted and was aborted, but `{branch}` is at {} instead of {}: the old tip is saved in {r}; look at the worktree", short(&now), short(&tip))));
                }
                Err(self.end(CONFLICT, &format!("rebasing `{branch}` onto main {} conflicts in: {}. The rebase was aborted and the branch is exactly as before (tip {}). Rebase or fix it yourself, commit, and queue it again with `queue add`", short(&head), files.join(", "), short(&tip)), EXIT_RETRY))
            }
            wtgit::Rebase::Failed(m) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("{m}; and `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                Err(self.blocked(&format!("the rebase could not run: {m}. The branch is unchanged")))
            }
        }
    }

    // -- step 3: check ----------------------------------------------------------------------

    fn s_check(&mut self) -> R<()> {
        self.set_step("check")?;
        let (project, task_id) = (self.e.get("project").to_string(), self.e.get("task").to_string());
        if let Some(code) = tools::guard_refusal(self.cfg, &project, Some(&task_id)) {
            return Err(self.blocked(&format!("the check was refused before it started (exit {code}; the reason is printed above, e.g. Broadsword paused or an interrupted run to recover); nothing changed")));
        }
        println!("queue entry {}: check {project} --task {task_id}", self.e.id());
        let (dir, branch) = (self.dir.clone(), self.branch());
        let tip_before = self.g(wtgit::branch_tip(&dir, &branch))?;
        let mut info = JobInfo::default();
        let code = job::run_job_info(self.cfg, JobSpec { kind: "check".into(), name: project.clone(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: Some(task_id) }, &mut info);
        let summary = info.summary.join("\n");
        let tail = info.log.as_deref().map(|p| read_tail(p, self.cfg.reviewer_check_tail_kb * 1024)).unwrap_or_default();
        self.e.set("check_job", info.id.clone().unwrap_or_default());
        self.e.set("check_exit", code.to_string());
        self.e.set("check_summary", summary.clone());
        self.e.set("check_tail", tail);
        self.save()?;
        match code {
            0 => {
                // The one commit the check ran on. Review and merge are bound to exactly this commit.
                let tip_after = self.g(wtgit::branch_tip(&dir, &branch))?;
                if tip_after != tip_before {
                    return Err(self.end(STALE, &format!("branch changed during the check: `{branch}` was at {} when the check started and is at {} now; nothing was reviewed or merged. Run `queue run --entry {}` to check it again", short(&tip_before), short(&tip_after), self.e.id()), EXIT_RETRY));
                }
                self.e.set("checked_tip", tip_before.clone());
                self.e.set("check_deferred", "");
                self.save()?;
                self.log("check", &format!("passed on commit {} ({})", short(&tip_before), info.summary.join("; ")))
            }
            1 if !info.summary.is_empty() => Err(self.end(CHECK_FAILED, &format!("build or tests failed on the rebased branch (job {}): {}. No model was asked. Fix it, commit, and queue the task again with `queue add`", info.id.as_deref().unwrap_or("?"), info.summary.join("; ")), EXIT_REJECTED)),
            c => Err(self.blocked(&format!("the check could not run to a verdict (exit {c}; job {}); nothing was merged; see its output above, fix the cause, then `queue run --entry {}`", info.id.as_deref().unwrap_or("none"), self.e.id()))),
        }
    }

    // -- step 4: review ---------------------------------------------------------------------

    fn s_review(&mut self) -> R<()> {
        self.set_step("review")?;
        if !self.e.get("verdict").is_empty() {
            self.log("review", "the verdict is already recorded (this run resumed): the reviewer is not asked again")?;
            return self.recorded_verdict_ok();
        }
        let (dir, branch) = (self.dir.clone(), self.branch());
        let model = match gating_model(self.cfg, self.e.get("risk"), self.e.get("coder")) {
            Ok((m, _)) => m.to_string(),
            Err(msg) => return Err(self.blocked(&msg)),
        };
        // Check, review and merge are bound to ONE commit: the one the check passed on. A branch that moved since (a commit added
        // after the check, live or before a resume) was never checked, so it is never reviewed and never merged.
        let checked = self.e.get("checked_tip").to_string();
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if checked.is_empty() {
            return Err(self.end(STALE, &format!("no checked commit is recorded for this entry, so nothing was reviewed or merged. Run `queue run --entry {}` to check it", self.e.id()), EXIT_RETRY));
        }
        if tip != checked {
            return Err(self.end(STALE, &format!("branch changed after check: `{branch}` was checked at {} but is at {} now; the new commit(s) were never checked, so nothing was reviewed or merged. Run `queue run --entry {}` to rebase and check it again", short(&checked), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let base = self.e.get("base_head").to_string();
        let diff = self.g(wtgit::diff_since(&dir, &base, &checked))?;
        if diff.trim().is_empty() {
            return Err(self.blocked("the diff against main is empty: nothing to review or merge"));
        }
        let max = self.cfg.reviewer_diff_max_kb * 1024;
        if diff.len() as u64 > max {
            return Err(self.blocked(&format!("the diff is {} KB, over reviewer_diff_max_kb = {}: it is never cut, so it is never sent. Split the task into smaller ones", diff.len() / 1024, self.cfg.reviewer_diff_max_kb)));
        }
        let backend = match reviewer::backend(self.cfg) {
            Ok(b) => b,
            Err(m) => return Err(self.blocked(&m)),
        };
        let cli = match backend.prepare(self.cfg) {
            Ok(c) => c,
            Err(m) => return Err(self.blocked(&format!("the reviewer CLI was refused: {m}. No model was called, nothing merged"))),
        };
        let calls = self.e.get_num("review_calls").unwrap_or(0);
        if calls >= self.cfg.reviewer_max_calls_per_entry {
            return Err(self.blocked(&format!("review call cap reached: this entry already made {calls} reviewer call(s) (reviewer_max_calls_per_entry = {}); no model was called, nothing merged. Cancel it and queue the task again if more reviews are wanted", self.cfg.reviewer_max_calls_per_entry)));
        }
        self.e.set("reviewed_tip", checked.clone());
        self.e.set("reviewer_model", model.clone());
        let effort = effort_for(self.cfg, self.e.get("risk")).to_string();
        self.e.set("reviewer_effort", effort.clone());
        self.e.set("reviewer_cli", cli.path.clone());
        self.e.set("test_hook", if cli.test_hook { "true" } else { "false" });
        // counted BEFORE the call, so a run that dies in the middle of it still paid for one
        self.e.set("review_calls", (calls + 1).to_string());
        self.save()?;
        if cli.test_hook {
            println!("TEST HOOK ACTIVE: reviewer CLI is {}", cli.path);
        } else {
            println!("queue entry {}: reviewer CLI is {}", self.e.id(), cli.path);
        }
        let prompt = self.secrets.redact(&reviewer::build_prompt(self.e.get("brief"), self.e.get("check_summary"), self.e.get("check_tail"), &diff));
        let short_note = if self.secrets.short_count() > 0 { format!("; {} value(s) of .env are shorter than {} characters: not redacted from text (too common) but kept out of the reviewer's environment", self.secrets.short_count(), self.cfg.redact_min_chars) } else { String::new() };
        self.log("review", &format!("calling the {} reviewer `{model}` through {}{} on {} (diff {} bytes, commit {}, call {} of at most {}); this spends money and uses the network{short_note}", backend.name(), cli.path, if cli.test_hook { " [TEST HOOK]" } else { "" }, if self.e.get("risk") == "high" { "a high-risk change" } else { "a low-risk change" }, diff.len(), short(&checked), calls + 1, self.cfg.reviewer_max_calls_per_entry))?;
        let review = match backend.call(self.cfg, &cli, &model, &effort, &prompt) {
            Ok(r) => r,
            Err(CallError::Timeout(s)) => return Err(self.blocked(&format!("the reviewer `{model}` gave no answer within {s} s (reviewer_timeout_seconds) and was killed: no verdict, nothing merged. Run `queue run --entry {}` to ask again", self.e.id()))),
            Err(CallError::Start(m) | CallError::Failed(m)) => return Err(self.blocked(&format!("the reviewer `{model}` could not be used: {m}. No verdict, nothing merged. Fix it, then `queue run --entry {}`", self.e.id()))),
        };
        let verdict = reviewer::judge(&review);
        let file = self.write_review(&model, &checked, &base, prompt.len(), &cli, &review, &verdict)?;
        self.e.set("verdict", if verdict.pass { "PASS" } else { "FAIL" });
        self.e.set("blocking", verdict.blocking.join("\n"));
        self.e.set("minor", verdict.minor.join("\n"));
        self.e.set("cost_usd", review.cost_usd.clone().unwrap_or_default());
        self.e.set("tokens_in", review.input_tokens.map(|n| n.to_string()).unwrap_or_default());
        self.e.set("tokens_out", review.output_tokens.map(|n| n.to_string()).unwrap_or_default());
        self.e.set("review_file", file.clone());
        self.save()?;
        self.log("review", &format!("{} by `{model}`; {} blocking, {} minor; cost {} USD; full output in {file}", self.e.get("verdict"), verdict.blocking.len(), verdict.minor.len(), review.cost_usd.as_deref().unwrap_or("unknown")))?;
        self.s_shadow(&prompt)?;
        if verdict.pass {
            return Ok(());
        }
        let why = if verdict.note.is_empty() { verdict.blocking.join("; ") } else { format!("{} ({})", verdict.note, verdict.blocking.join("; ")) };
        Err(self.end(REJECTED, &format!("the `{model}` reviewer FAILED it: {why}. The task is still running and its worktree is kept: fix, commit, and queue it again with `queue add`. Full review: {file}"), EXIT_REJECTED))
    }

    /// The SHADOW review: the same prompt to a second model (`shadow_reviewer`), run once after the gating verdict is recorded.
    /// It is only RECORDED: it never changes the entry's outcome, so no failure of it (timeout, API error, bad JSON, no python, no
    /// script) ends or blocks the entry; each is recorded as `shadow_verdict = ERROR` with the reason. It runs at most once per
    /// review call, because a resumed run that already has a gating verdict never reaches the review call again.
    /// S4 (no reviewer program inside the root) does not apply to it: it never gates a merge.
    fn s_shadow(&mut self, prompt: &str) -> R<()> {
        if self.cfg.shadow_reviewer == "off" {
            return Ok(());
        }
        let model = self.cfg.shadow_reviewer_model.clone();
        self.e.set("shadow_model", model.clone());
        let coder = self.e.get("coder").to_string();
        if reviewer::shadow_family(&coder) == reviewer::shadow_family(&model) {
            return self.shadow_end("SKIPPED", &format!("the coder `{coder}` and the shadow model `{model}` are the same family (a model never reviews its own code)"));
        }
        let (python, script) = match reviewer::resolve_shadow(self.cfg) {
            Ok(p) => p,
            Err(m) => return self.shadow_end("ERROR", &m),
        };
        self.log("shadow", &format!("calling the shadow reviewer `{model}` (recorded only, never gating; at most {} s); this spends money and uses the network", self.cfg.shadow_reviewer_timeout_seconds))?;
        let t0 = std::time::Instant::now();
        let result = reviewer::call_shadow(self.cfg, &python, &script, &model, prompt);
        let secs = t0.elapsed().as_secs_f64();
        self.e.set("shadow_secs", format!("{secs:.1}"));
        let review = match result {
            Ok(r) => r,
            Err(CallError::Timeout(s)) => return self.shadow_end("ERROR", &format!("no answer within {s} s (shadow_reviewer_timeout_seconds); it was killed")),
            Err(CallError::Start(m) | CallError::Failed(m)) => return self.shadow_end("ERROR", &m),
        };
        let v = reviewer::judge(&review);
        let file = self.write_shadow_file(&model, &review, &v)?;
        self.e.set("shadow_file", file);
        self.e.set("shadow_model_used", review.models_used.join(", "));
        self.e.set("shadow_cost_usd", review.cost_usd.clone().unwrap_or_default());
        self.e.set("shadow_tokens", match (review.input_tokens, review.output_tokens) {
            (Some(i), Some(o)) => (i + o).to_string(),
            _ => String::new(),
        });
        if review.invalid.is_some() || !v.parsed {
            return self.shadow_end("ERROR", &v.note);
        }
        self.e.set("shadow_blocking", v.blocking.join("
"));
        self.shadow_end(if v.pass { "PASS" } else { "FAIL" }, "")
    }

    fn shadow_end(&mut self, verdict: &str, reason: &str) -> R<()> {
        self.e.set("shadow_verdict", verdict);
        self.e.set("shadow_reason", reason);
        self.save()?;
        println!("queue entry {}: shadow review {verdict}{} (recorded only; the gating review decides)", self.e.id(), if reason.is_empty() { String::new() } else { format!(": {reason}") });
        self.log("shadow", &format!("{verdict}{}", if reason.is_empty() { String::new() } else { format!(": {reason}") }))
    }

    fn write_shadow_file(&mut self, model: &str, r: &reviewer::Review, v: &reviewer::Verdict) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 { format!("{}.shadow.review", self.e.id()) } else { format!("{}.{attempt}.shadow.review", self.e.id()) };
        let path = queue_dir(self.cfg).join(&name);
        let text = format!(
            "entry: {}
attempt: {attempt}
shadow review: recorded only, never gating
model: {model}
models reported: {}
verdict: {} (answer followed the format: {})
{}blocking:
{}
--- raw shadow output (exactly as printed) ---
{}
",
            self.e.id(),
            r.models_used.join(", "),
            if v.pass { "PASS" } else { "FAIL" },
            if v.parsed && r.invalid.is_none() { "yes" } else { "NO" },
            if v.note.is_empty() { String::new() } else { format!("note: {}
", v.note) },
            v.blocking.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("
"),
            r.raw
        );
        let text = self.secrets.redact(&text);
        let tmp = path.with_extension("review.tmp");
        if let Err(e) = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path)) {
            return Err(self.internal(&format!("cannot write the shadow review file {}: {e}", path.display())));
        }
        Ok(path.strip_prefix(&self.cfg.root).unwrap_or(&path).to_string_lossy().replace('\\', "/"))
    }

    /// A verdict is already recorded (a resumed run). Returns Ok only for a PASS on exactly the checked commit; anything else ends
    /// the entry the way a fresh run would, so no resumed entry can be stuck `running` (which would block `queue run` for all):
    /// not PASS -> `rejected` exactly as a fresh FAIL (task running, worktree kept, `queue add` works again);
    /// a PASS without a recorded reviewed or checked commit, or for another commit -> `stale` (`queue run --entry` starts a fresh attempt).
    fn recorded_verdict_ok(&mut self) -> R<()> {
        let id = self.e.id().to_string();
        if self.e.get("verdict") != "PASS" {
            let model = self.e.get("reviewer_model").to_string();
            let file = self.e.get("review_file").to_string();
            let blocking = self.e.get("blocking").replace('\n', "; ");
            let why = if blocking.is_empty() { format!("verdict `{}`", self.e.get("verdict")) } else { blocking };
            return Err(self.end(REJECTED, &format!("the `{model}` reviewer FAILED it: {why}. The task is still running and its worktree is kept: fix, commit, and queue it again with `queue add`. Full review: {file}"), EXIT_REJECTED));
        }
        let (checked, reviewed) = (self.e.get("checked_tip").to_string(), self.e.get("reviewed_tip").to_string());
        if checked.is_empty() || reviewed.is_empty() || checked != reviewed {
            return Err(self.end(STALE, &format!("the recorded PASS does not name the checked commit (checked `{}`, reviewed `{}`), so it cannot be trusted; nothing was merged. Run `queue run --entry {id}` to check and review it again", short(&checked), short(&reviewed)), EXIT_RETRY));
        }
        Ok(())
    }

    /// The review file: header lines, then the reviewer's raw output. Returns its path relative to root. Written atomically.
    #[allow(clippy::too_many_arguments)]
    fn write_review(&mut self, model: &str, tip: &str, base: &str, prompt_bytes: usize, cli: &reviewer::Cli, r: &reviewer::Review, v: &reviewer::Verdict) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 { format!("{}.review", self.e.id()) } else { format!("{}.{attempt}.review", self.e.id()) };
        let path = queue_dir(self.cfg).join(&name);
        let or_unknown = |o: Option<String>| o.unwrap_or_else(|| "unknown".into());
        let text = format!(
            "entry: {}\nattempt: {attempt}\ntask: {}\nbackend: {}\nreviewer cli: {}\ntest_hook: {}\nmodel: {model}\nmodels reported by the backend: {}\nrisk: {}\ncoder: {}\nreviewed commit: {tip}\ndiff base: {base}\nprompt bytes: {prompt_bytes}\ncost_usd: {}\ninput tokens: {}\noutput tokens: {}\nverdict: {} (answer followed the format: {})\n{}blocking:\n{}\nminor:\n{}\n--- raw reviewer output (exactly as printed) ---\n{}\n",
            self.e.id(),
            self.e.get("task"),
            self.cfg.reviewer_backend,
            cli.path,
            cli.test_hook,
            if r.models_used.is_empty() { "-".to_string() } else { r.models_used.join(", ") },
            self.e.get("risk"),
            self.e.get("coder"),
            or_unknown(r.cost_usd.clone()),
            or_unknown(r.input_tokens.map(|n| n.to_string())),
            or_unknown(r.output_tokens.map(|n| n.to_string())),
            if v.pass { "PASS" } else { "FAIL" },
            if v.parsed { "yes" } else { "NO" },
            if v.note.is_empty() { String::new() } else { format!("note: {}\n", v.note) },
            v.blocking.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("\n"),
            v.minor.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("\n"),
            r.raw
        );
        let text = self.secrets.redact(&text);
        let tmp = path.with_extension("review.tmp");
        let written = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            return Err(self.internal(&format!("cannot write the review file {}: {e}", path.display())));
        }
        Ok(path.strip_prefix(&self.cfg.root).unwrap_or(&path).to_string_lossy().replace('\\', "/"))
    }

    // -- step 5: merge ----------------------------------------------------------------------

    fn s_merge(&mut self) -> R<()> {
        self.set_step("merge")?;
        self.locate()?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let reviewed = self.e.get("reviewed_tip").to_string();
        let base = self.e.get("base_head").to_string();
        // A resume at this step skips the review step: the recorded state is judged here, and every bad state ENDS the entry
        // (`rejected` or `stale`), never leaves it `running` where it would block `queue run` for every other entry.
        if self.e.get("verdict") != "PASS" {
            self.recorded_verdict_ok()?; // always an Err here: rejected
        }
        if reviewed.is_empty() || base.is_empty() || self.e.get("checked_tip") != reviewed {
            return Err(self.end(STALE, &format!("the merge step was reached without a recorded PASS for the checked commit (reviewed `{}`, checked `{}`, base `{}`); nothing was merged. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(self.e.get("checked_tip")), short(&base), self.e.id()), EXIT_RETRY));
        }
        let cli_path = self.e.get("reviewer_cli").to_string();
        if let Some(why) = self.test_hook_refusal() {
            return Err(self.blocked(&why));
        }
        self.refuse_chained("merge")?;
        println!("queue entry {}: merging after a {} review through reviewer CLI {}", self.e.id(), if self.e.get("test_hook") == "true" { "TEST HOOK" } else { "real" }, if cli_path.is_empty() { "(none recorded)" } else { &cli_path });
        let main_branch = self.g(wtgit::current_branch(&dir))?;
        if main_branch.as_deref() != Some(self.e.get("main_branch")) {
            let now = main_branch.unwrap_or_else(|| "a detached HEAD".into());
            let was = self.e.get("main_branch").to_string();
            return Err(self.blocked(&format!("the main checkout is on {now}, but this entry was reviewed against `{was}`; nothing merged")));
        }
        let head = self.g(wtgit::head_commit(&dir))?;
        if head == reviewed || self.g(wtgit::is_ancestor(&dir, &reviewed, &head))? {
            // an earlier run already fast-forwarded main to this commit (a merge train: to a later commit that contains it) and died
            // before recording it
            self.e.set("main_sha", reviewed.clone());
            if head == reviewed {
                return self.log("merge", &format!("main's HEAD already is the reviewed commit {}: recorded as merged", short(&head)));
            }
            return self.log("merge", &format!("the reviewed commit {} is already on main (HEAD {} contains it): recorded as merged", short(&reviewed), short(&head)));
        }
        let dirty = self.g(wtgit::status_porcelain_plain(&dir))?;
        if !dirty.is_empty() {
            return Err(self.blocked(&format!("the main checkout got uncommitted or untracked changes while the review ran ({}); nothing merged", some(&dirty))));
        }
        if head != base {
            return Err(self.end(STALE, &format!("main moved from {} to {} after the rebase and the review; nothing was merged. Run `queue run --entry {}` to rebase again", short(&base), short(&head), self.e.id()), EXIT_RETRY));
        }
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if tip != reviewed {
            return Err(self.end(STALE, &format!("branch changed after check and review: `{branch}` moved from the checked and reviewed commit {} to {}; nothing was merged. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let wt_status = self.g(wtgit::status_porcelain_plain(&wt))?;
        if !wt_status.is_empty() {
            return Err(self.blocked(&format!("the task's worktree got changes after the review ({}); nothing merged", some(&wt_status))));
        }
        if let Err(m) = wtgit::merge_ff_only(&dir, &reviewed) {
            let now = self.g(wtgit::head_commit(&dir))?;
            if now != head {
                return Err(self.end(STALE, &format!("main moved from {} to {} during the merge; nothing was merged by this run. Run `queue run --entry {}` to rebase again", short(&head), short(&now), self.e.id()), EXIT_RETRY));
            }
            return Err(self.blocked(&format!("git refused the fast-forward: {m}. Main is unchanged")));
        }
        let after = self.g(wtgit::head_commit(&dir))?;
        if after != reviewed {
            return Err(self.internal(&format!("after `git merge --ff-only` main's HEAD is {} but the reviewed commit is {}; main was {} before; look at the project", short(&after), short(&reviewed), short(&head))));
        }
        self.e.set("main_sha", after.clone());
        let note = format!("fast-forwarded `{}` from {} to {}; reviewed through CLI {} (test_hook: {})", self.e.get("main_branch"), short(&head), short(&after), if cli_path.is_empty() { "(none recorded)" } else { &cli_path }, self.e.get("test_hook"));
        self.log("merge", &note)
    }

    /// A stand-in reviewer (a scratch test hook) may only merge into a project that itself lives in scratch. The ONE place this rule
    /// is decided: the single-entry merge (`s_merge`) and the merge train (`train_ready`, before any chaining) both ask it.
    fn test_hook_refusal(&self) -> Option<String> {
        if self.e.get("test_hook") != "true" {
            return None;
        }
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        if canon(&self.dir).starts_with(canon(&self.cfg.scratch_dir)) {
            return None;
        }
        Some(format!("the review was made by a TEST HOOK reviewer ({}), not a real one, and project folder {} is outside scratch_dir; a stand-in review never merges into a real project; nothing merged", self.e.get("reviewer_cli"), self.dir.display()))
    }

    // -- step 6: main check -----------------------------------------------------------------

    fn s_main_check(&mut self) -> R<()> {
        self.set_step("main_check")?;
        let project = self.e.get("project").to_string();
        let sha = self.e.get("main_sha").to_string();
        let mb = self.e.get("main_branch").to_string();
        let loud = |code: String, extra: &str| {
            format!("commit {sha} is ALREADY on `{mb}` of project `{project}`, and `check {project}` there did not pass ({code}){extra}. Main was NOT reset or reverted. A human decides: fix forward, or `git revert` the commit. The task was NOT marked done and its worktree is kept")
        };
        if let Some(code) = tools::guard_refusal(self.cfg, &project, None) {
            return Err(self.end(MERGED_CHECK_FAILED, &loud(format!("the check was refused before it started, exit {code}"), ""), EXIT_MERGED_BAD));
        }
        println!("queue entry {}: check {project} (the main checkout)", self.e.id());
        let mut info = JobInfo::default();
        let code = job::run_job_info(self.cfg, JobSpec { kind: "check".into(), name: project.clone(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: None }, &mut info);
        if code != 0 {
            let extra = if info.summary.is_empty() { String::new() } else { format!(": {}", info.summary.join("; ")) };
            return Err(self.end(MERGED_CHECK_FAILED, &loud(format!("exit {code}, job {}", info.id.as_deref().unwrap_or("none")), &extra), EXIT_MERGED_BAD));
        }
        self.log("main_check", &format!("check on main passed ({})", info.summary.join("; ")))
    }

    // -- step 7: done -----------------------------------------------------------------------

    fn s_done(&mut self) -> R<ExitCode> {
        self.set_step("done")?;
        let id = self.e.get("task").to_string();
        let t = self.g(task::load_task(self.cfg, &id))?;
        if t.status() == task::RUNNING {
            let note = format!("merged by queue entry {} as {}", self.e.id(), short(self.e.get("main_sha")));
            let _ = task::cmd_finish(self.cfg, &id, task::Action::Done, Some(&note)); // prints its own result; judged below from the record
        }
        let t = self.g(task::load_task(self.cfg, &id))?;
        if t.status() != task::DONE {
            return Err(self.internal(&format!("`task done {id}` did not complete (the task is `{}`; it may be that the board was busy); the commit {} IS on main. Resume with `queue run --entry {}`", t.status(), short(self.e.get("main_sha")), self.e.id())));
        }
        // Removal keeps a worktree that holds anything git would lose (ignored files included): the entry is still `merged`.
        let wt_note = if worktree::has_worktree(&t) {
            let why = worktree::abs_path(self.cfg, t.get("worktree")).ok().and_then(|p| wtgit::status_porcelain(&p).ok()).filter(|s| !s.is_empty()).map(|s| format!(": it holds files removal will not delete ({})", some(&s))).unwrap_or_else(|| " (the reason is in the warning `task done` printed)".into());
            format!("its worktree and branch were KEPT{why}; nothing was lost")
        } else {
            "its worktree and branch are removed".to_string()
        };
        Ok(self.end(MERGED, &format!("commit {} is on `{}`, check on main passed, task {id} is done; {wt_note}", short(self.e.get("main_sha")), self.e.get("main_branch")), 0))
    }
}

// ---- merge train ----------------------------------------------------------------------------

/// The result of one check of a commit (see `Run::check_commit`).
enum TrainCheck {
    Pass { job: String, summary: String, tail: String },
    Fail { job: String, summary: String, tail: String },
    /// The check could not run to a verdict (refused, paused, wrong checkout, job could not start): no conclusion about the code.
    Blocked(String),
}

/// What a check recorded: the job id, its summary lines and the end of its output.
#[derive(Default, Clone)]
struct PassInfo {
    job: String,
    summary: String,
    tail: String,
}

/// What a `check_alone` said about the culprit of a failed train.
enum Alone {
    /// It fails on its own commit too: the entry itself is bad.
    Bad(PassInfo),
    /// It passes on its own commit: the failure only shows when it is combined with the entries before it.
    Fine,
    Unknown(String),
}

/// The entries the next train takes out of `items` (one `(project, claims)` per approved entry, oldest first), as indices into
/// `items`. Entries of one project only; an entry whose claims overlap an entry already taken is left out (it waits for a later
/// train); at most `max` entries. A train is ready when it is full, or when `flush` is set (nothing else is coming) and it holds at
/// least one entry. Empty when no project has a ready train.
fn pick_train(items: &[(String, Vec<String>)], max: usize, flush: bool) -> Vec<usize> {
    let mut seen: Vec<&str> = Vec::new();
    for (first, (project, _)) in items.iter().enumerate() {
        if seen.contains(&project.as_str()) {
            continue;
        }
        seen.push(project.as_str());
        let mut group: Vec<usize> = Vec::new();
        for (i, (p, claims)) in items.iter().enumerate().skip(first) {
            if p != project || group.len() >= max {
                continue;
            }
            let clash = group.iter().any(|g| items[*g].1.iter().any(|a| claims.iter().any(|b| task::overlaps(a, b))));
            if !clash {
                group.push(i);
            }
        }
        if group.len() >= max || (flush && !group.is_empty()) {
            return group;
        }
    }
    Vec::new()
}

/// The commit a merge train chained this entry's branch onto: `left_train_base` (recorded when it left a train, kept across attempts),
/// else `train_up` (the base of its own commits while it rides a train). Empty for an entry no train ever chained.
fn chain_base_of(e: &Record) -> String {
    if e.get("left_train_base").is_empty() { e.get("train_up").to_string() } else { e.get("left_train_base").to_string() }
}

/// LEFT-TRAIN RULE, the test: does `branch` still carry `base` (a commit a merge train chained it onto) while main's HEAD does not?
/// Then the branch holds commits of OTHER entries that are not on main, and a review or merge of it would take their code as this
/// entry's. An empty `base` is never chained; a base that main already holds (those entries merged) is not either.
fn chained_onto(dir: &Path, branch: &str, base: &str) -> Result<bool, String> {
    if base.is_empty() {
        return Ok(false);
    }
    let head = wtgit::head_commit(dir)?;
    let tip = wtgit::branch_tip(dir, branch)?;
    Ok(wtgit::is_ancestor(dir, base, &tip)? && !wtgit::is_ancestor(dir, base, &head)?)
}

/// The reason an entry with a chained branch gets (see `Run::leave_train`), with the coder's way out.
fn left_train_text(main: &str, branch: &str, base: &str) -> String {
    format!("left a train: rebase your own commits onto main and queue again. `{branch}` is chained on {}, a commit of other entries of a merge train that is not on `{main}`, so a review or merge of it now would take their code as this entry's. In the task's worktree run `git rebase --onto {main} {base} {branch}`, then run the entry again (`queue run --entry <id>`, or `queue cancel` it and `queue add` the task). If those entries reach `{main}` first, the branch needs nothing and runs as it is", short(base))
}

/// Take the next train out of `approved` (see `pick_train`), in queue order; the others stay.
fn take_group<'a>(approved: &mut Vec<(Run<'a>, std::time::Instant)>, max: usize, flush: bool) -> Option<Vec<(Run<'a>, std::time::Instant)>> {
    let items: Vec<(String, Vec<String>)> = approved.iter().map(|(r, _)| (r.e.get("project").to_string(), r.claims.clone())).collect();
    let idx = pick_train(&items, max, flush);
    if idx.is_empty() {
        return None;
    }
    let mut group = Vec::new();
    for i in idx.iter().rev() {
        group.push(approved.remove(*i));
    }
    group.reverse();
    Some(group)
}

/// Entries an earlier `queue run --all` approved and never merged (it died, or stopped): claimed by this process (their pid), oldest
/// first. Under entries.lock; their status stays `approved`.
fn claim_approved(cfg: &Config) -> Result<Vec<Record>, ExitCode> {
    let _l = entries_lock(cfg)?;
    let all = load_entries(cfg).map_err(|m| fail(EXIT_ERROR, m))?;
    let me = std::process::id();
    let mut out = Vec::new();
    // a half-finished merge is not a train candidate: the drain resumes it on its own first (`resume_first`)
    for mut e in all.into_iter().filter(|x| x.status() == APPROVED && !half_merged(x)) {
        e.set("pid", me.to_string());
        e.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
        e.save().map_err(|err| fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display())))?;
        out.push(e);
    }
    Ok(out)
}

fn train_stop_note(code: u8) -> String {
    if code == EXIT_MERGED_BAD {
        "stopped by a merge train (exit 6): main holds the train's commits and the main check failed; a human decides before anything else merges".into()
    } else {
        "stopped by a merge train (exit 1): an internal error or a check that could not run to a verdict; the entries it was working on are `approved` (or `running`), and the next `queue run --all` takes them again".into()
    }
}

/// One entry of a train is finished (merged, rejected, back to queued, ...): its summary row, and the train's worst outcome so far.
fn finish_member(cfg: &Config, r: Run<'_>, code: u8, t: std::time::Instant, rows: &mut Vec<Row>, worst: &mut u8) {
    let id = r.e.id().to_string();
    rows.push(row_for(cfg, &id, code, t));
    if badness(code) > badness(*worst) {
        *worst = code;
    }
}

/// The entries stay `approved` (the reason says why), and the next drain takes them again.
fn leave_approved(cfg: &Config, members: Vec<(Run<'_>, std::time::Instant)>, why: &str, rows: &mut Vec<Row>, worst: &mut u8) {
    for (mut r, t) in members {
        r.e.set("reason", format!("approved, not merged: {why}"));
        let _ = r.log("train", why);
        finish_member(cfg, r, EXIT_RETRY, t, rows, worst);
    }
}

/// An internal error that hits every member (git or the record could not be used): each stays as it is, with the message.
fn fail_members(cfg: &Config, members: Vec<(Run<'_>, std::time::Instant)>, msg: &str, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    for (mut r, t) in members {
        let c = r.internal(msg);
        finish_member(cfg, r, code_num(c), t, rows, worst);
    }
    EXIT_ERROR
}

/// Entries still `approved` when the drain ends early (it stopped) or no train took them: a row each; the next drain takes them.
fn settle_approved(cfg: &Config, approved: &mut Vec<(Run<'_>, std::time::Instant)>, rows: &mut Vec<Row>) {
    for (r, t) in approved.drain(..) {
        let id = r.e.id().to_string();
        rows.push(row_for(cfg, &id, 0, t));
    }
}

/// Run trains while `approved` holds a ready one (`flush`: any entry is ready). Returns 0, or EXIT_ERROR / EXIT_MERGED_BAD when the
/// drain must stop.
fn run_trains<'a>(cfg: &'a Config, approved: &mut Vec<(Run<'a>, std::time::Instant)>, flush: bool, rows: &mut Vec<Row>) -> u8 {
    let max = cfg.merge_train_max as usize;
    let mut hard = 0u8;
    while hard == 0 {
        let Some(group) = take_group(approved, max, flush) else { break };
        hard = run_train(cfg, group, rows);
    }
    hard
}

/// The commit a chain entry's own check proves (its tip in the chain), for the 1-based position `i`.
fn chain_tip(chain: &[(Run<'_>, std::time::Instant)], i: usize) -> String {
    chain[i - 1].0.e.get("checked_tip").to_string()
}

/// ONE MERGE TRAIN. `group` are approved entries of one project with claims that do not overlap (`pick_train`).
///
/// 1. Each entry is checked to still be exactly its reviewed commit (`train_ready`).
/// 2. CHAIN: each entry's own commits are rebased onto the previous entry's tip, the first onto main's HEAD (`train_link`). The result
///    is one straight line `main <- e1 <- e2 <- ... <- en`, every entry's branch tip is on it, and the tip of entry i contains the
///    entries before it. An entry that cannot be chained (a textual conflict), or whose rebased commits are not the same patches as
///    the reviewed ones, goes back to `queued`; the rest go on.
/// 3. ONE CHECK of the chain tip (`Run::check_commit`).
/// 4. PASS: ONE fast-forward of main to the tip (every entry's commits as they are, in order, no squash), ONE main check (`s_main_check`,
///    run by the last entry), then every entry is `merged` (`task done`), recording train id, position and the shared check job.
/// 5. FAIL: BISECT. The check of entry i's tip is a check of the first i entries, so a binary search over the prefixes finds the first
///    entry k whose prefix fails (about log2(n) checks; the prefix before it passed). The culprit is then checked on its own commit
///    (`check_alone`): if it fails alone it is `check_failed` with that check's log, the proven-good entries before it merge as a
///    smaller train, and the entries after it start a new round on the new main. If it passes alone the failure only shows when it is
///    combined with the entries before it: the culprit AND every entry before it in the chain (the partner is not searched for) are
///    held out with the combined failure's log, and NOTHING is merged for them. COMBINED-ONLY PAIR, DECIDED (round 3, kept as the brief
///    says: "neither merges"; the proven prefix is NOT merged): each held entry asks for `conflict`, and the left-train rule
///    (`leave_train`) decides what it ends as. The first entry of the chain sits on main's HEAD and ends a plain `conflict`; every later
///    one still carries the earlier held entries' commits, which are not on main, so it ends `blocked` ("left a train: ... it would
///    have been `conflict`", with the rebase command). So for a pair e1, e2: e1 `conflict`, e2 `blocked`, main unchanged. Merging the
///    proven prefix instead would turn e2 into a plain `conflict` but put e1 on main while its partner is unknown; that is a later
///    choice for the measurement window, not this phase. Every round removes at least one entry, so it ends.
/// 6. A main check that fails after the fast-forward: every entry of the merge is `merged_check_failed` (exit 6), main is NOT reset, the
///    drain stops. A check that cannot run to a verdict leaves all its entries `approved` and stops the drain.
///
/// Returns 0, EXIT_ERROR or EXIT_MERGED_BAD (the drain stops on the last two).
fn run_train<'a>(cfg: &'a Config, group: Vec<(Run<'a>, std::time::Instant)>, rows: &mut Vec<Row>) -> u8 {
    static TRAIN_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let train_id = format!("tr-{}-{}", job::file_stamp(now_secs()), TRAIN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let started = std::time::Instant::now();
    let size = group.len();
    println!("merge train {train_id}: {size} approved entr{} (merge_train_max = {})", if size == 1 { "y" } else { "ies" }, cfg.merge_train_max);
    let mut worst = 0u8;
    let mut hard = 0u8;
    let mut checks = 0u32;
    let mut todo: Vec<(Run<'a>, std::time::Instant)> = Vec::new();
    for (mut r, t) in group {
        match r.train_ready() {
            Ok(()) => todo.push((r, t)),
            Err(c) => {
                let code = code_num(c);
                finish_member(cfg, r, code, t, rows, &mut worst);
            }
        }
    }
    let mut round = 0u32;
    while !todo.is_empty() && hard == 0 {
        round += 1;
        let dir = todo[0].0.dir.clone();
        let head = match wtgit::head_commit(&dir) {
            Ok(h) => h,
            Err(m) => {
                hard = fail_members(cfg, std::mem::take(&mut todo), &m, rows, &mut worst);
                break;
            }
        };
        // ---- chain
        let mut chain: Vec<(Run<'a>, std::time::Instant)> = Vec::new();
        let mut prev = head.clone();
        let mut pending: std::collections::VecDeque<(Run<'a>, std::time::Instant)> = std::mem::take(&mut todo).into();
        while let Some((mut r, t)) = pending.pop_front() {
            match r.train_link(&prev, &train_id) {
                Ok(Some(tip)) => {
                    prev = tip;
                    chain.push((r, t));
                }
                Ok(None) => finish_member(cfg, r, EXIT_RETRY, t, rows, &mut worst),
                Err(c) => {
                    let code = code_num(c);
                    finish_member(cfg, r, code, t, rows, &mut worst);
                    if code == EXIT_ERROR {
                        hard = EXIT_ERROR;
                        break;
                    }
                }
            }
        }
        if hard != 0 {
            let rest: Vec<(Run<'a>, std::time::Instant)> = chain.into_iter().chain(pending).collect();
            leave_approved(cfg, rest, "the merge train stopped on an internal error of an earlier entry", rows, &mut worst);
            break;
        }
        if chain.is_empty() {
            continue;
        }
        let m = chain.len();
        for (i, (r, _)) in chain.iter_mut().enumerate() {
            r.e.set("train", train_id.clone());
            r.e.set("train_pos", (i + 1).to_string());
            let _ = r.save();
        }
        // ---- one check of the whole chain, then a bisect if it fails
        let mut lo = 0usize; // the first `lo` entries are proven good by a check that passed (their tip's)
        let mut hi = m; // the check of the first `hi` entries failed
        let mut good = PassInfo::default();
        let mut bad = PassInfo::default();
        let mut blocked: Option<String> = None;
        let mut bisect_checks = 0u32;
        let mut bisect_secs = 0f64;
        let tip = chain_tip(&chain, m);
        let first = chain[m - 1].0.check_commit(&tip, &format!("merge train {train_id}: all {m} entries combined"));
        checks += 1;
        match first {
            TrainCheck::Pass { job, summary, tail } => {
                lo = m;
                good = PassInfo { job, summary, tail };
            }
            TrainCheck::Fail { job, summary, tail } => bad = PassInfo { job, summary, tail },
            TrainCheck::Blocked(msg) => blocked = Some(msg),
        }
        while blocked.is_none() && hi - lo > 1 {
            let mid = (lo + hi) / 2;
            let tip_mid = chain_tip(&chain, mid);
            let tb = std::time::Instant::now();
            let res = chain[mid - 1].0.check_commit(&tip_mid, &format!("bisect of merge train {train_id}: the first {mid} entries"));
            bisect_checks += 1;
            checks += 1;
            bisect_secs += tb.elapsed().as_secs_f64();
            match res {
                TrainCheck::Pass { job, summary, tail } => {
                    lo = mid;
                    good = PassInfo { job, summary, tail };
                }
                TrainCheck::Fail { job, summary, tail } => {
                    hi = mid;
                    bad = PassInfo { job, summary, tail };
                }
                TrainCheck::Blocked(msg) => blocked = Some(msg),
            }
        }
        if bisect_checks > 0 {
            println!("merge train {train_id}: bisect: {bisect_checks} extra check(s) in {bisect_secs:.1} s; the first {lo} entr{} pass, entry {hi} makes it fail", if lo == 1 { "y" } else { "ies" });
        }
        let stats = format!("round {round}: {} check(s) of the chain, {bisect_checks} of them bisect, {bisect_secs:.1} s of bisect", 1 + bisect_checks);
        for (r, _) in chain.iter_mut() {
            r.e.set("train_bisect", stats.clone());
        }
        if let Some(msg) = blocked {
            println!("merge train {train_id}: the check could not give a verdict ({msg}); the {m} entries stay approved");
            leave_approved(cfg, chain, &format!("the merge train's check could not run to a verdict: {msg}"), rows, &mut worst);
            hard = EXIT_ERROR;
            break;
        }
        if lo == m {
            // ---- the whole chain passed
            let c = merge_members(cfg, &train_id, chain, &tip, &head, &good, rows, &mut worst);
            if c != 0 {
                hard = c;
            }
            continue;
        }
        // ---- entry `hi` (1-based) is the first whose prefix fails
        let k = hi;
        let after: Vec<(Run<'a>, std::time::Instant)> = chain.split_off(k);
        let Some((mut culprit, ct)) = chain.pop() else {
            hard = EXIT_ERROR;
            break;
        };
        let alone = if k == 1 {
            Alone::Bad(bad.clone()) // its tip IS its own commit on main: the check that failed was already the check on its own
        } else {
            checks += 1;
            match culprit.check_alone() {
                TrainCheck::Fail { job, summary, tail } => Alone::Bad(PassInfo { job, summary, tail }),
                TrainCheck::Pass { job, summary, .. } => {
                    let _ = culprit.log("train", &format!("checked on its own commit: passes (job {job}: {summary}); so the failure only shows when it is combined with the entries before it"));
                    Alone::Fine
                }
                TrainCheck::Blocked(msg) => Alone::Unknown(msg),
            }
        };
        let culprit_id = culprit.e.id().to_string();
        match alone {
            Alone::Unknown(msg) => {
                println!("merge train {train_id}: entry {culprit_id} could not be checked on its own ({msg}); every entry stays approved");
                let mut all = chain;
                all.push((culprit, ct));
                all.extend(after);
                leave_approved(cfg, all, &format!("the merge train's check of {culprit_id} on its own could not run to a verdict: {msg}"), rows, &mut worst);
                hard = EXIT_ERROR;
                break;
            }
            Alone::Bad(info) => {
                println!("merge train {train_id}: entry {culprit_id} fails on its own (job {}): check_failed; the {} entr{} before it merge", info.job, lo, if lo == 1 { "y" } else { "ies" });
                culprit.e.set("check_job", info.job.clone());
                culprit.e.set("check_exit", "1");
                culprit.e.set("check_summary", info.summary.clone());
                culprit.e.set("check_tail", info.tail.clone());
                // the proven-good entries before it merge FIRST: then a culprit chained only on them sits on main and ends a plain
                // `check_failed`; if they could not merge, its branch still carries them and it ends `blocked` (`leave_train`)
                if lo > 0 {
                    let tip_good = chain_tip(&chain, lo);
                    let c = merge_members(cfg, &train_id, chain, &tip_good, &head, &good, rows, &mut worst);
                    if c != 0 {
                        hard = c;
                    }
                }
                let c = culprit.leave_train(CHECK_FAILED, &format!("build or tests failed on this entry on its own (job {}): {}. Found by the bisect of merge train {train_id}; nothing of it was merged. Fix it, commit, and queue the task again with `queue add`", if info.job.is_empty() { "?" } else { info.job.as_str() }, info.summary.replace('\n', "; ")), EXIT_REJECTED);
                finish_member(cfg, culprit, code_num(c), ct, rows, &mut worst);
                if hard != 0 {
                    leave_approved(cfg, after, "the merge train stopped before this entry's turn", rows, &mut worst);
                    break;
                }
                todo = after;
            }
            Alone::Fine => {
                // COMBINED-ONLY (see 5 above): all held out; the first ends `conflict`, the ones chained on it `blocked` (left-train rule)
                println!("merge train {train_id}: entry {culprit_id} passes on its own, but fails combined with the entries before it: conflict for it and for those {lo} entr{}; nothing of them is merged", if lo == 1 { "y" } else { "ies" });
                let mut held = chain;
                held.push((culprit, ct));
                let ids: Vec<String> = held.iter().map(|(r, _)| r.e.id().to_string()).collect();
                for (mut r, t) in held {
                    r.e.set("check_job", bad.job.clone());
                    r.e.set("check_exit", "1");
                    r.e.set("check_summary", bad.summary.clone());
                    r.e.set("check_tail", bad.tail.clone());
                    let reason = format!("the combined check of merge train {train_id} failed (job {}) although each of these entries passes on its own: {}. They only fail together, so none of them was merged (main is unchanged). The entry's check summary and tail are the combined failure's. Rebase or fix them yourself, commit, and queue them again with `queue add`", if bad.job.is_empty() { "?" } else { bad.job.as_str() }, ids.join(", "));
                    let c = r.leave_train(CONFLICT, &reason, EXIT_RETRY);
                    finish_member(cfg, r, code_num(c), t, rows, &mut worst);
                }
                todo = after;
            }
        }
    }
    println!("merge train {train_id}: finished in {:.1} s; {size} entr{} taken, {checks} check(s) of the train, worst exit {worst}", started.elapsed().as_secs_f64(), if size == 1 { "y" } else { "ies" });
    hard
}

/// Fast-forward main to `tip` (the last of `members`, a chain whose tip passed its check), run ONE main check, mark every member
/// merged. Returns 0, EXIT_ERROR (internal), or EXIT_MERGED_BAD (the main check failed after the fast-forward: loud, main not reset).
#[allow(clippy::too_many_arguments)]
fn merge_members(cfg: &Config, train_id: &str, members: Vec<(Run<'_>, std::time::Instant)>, tip: &str, head: &str, pass: &PassInfo, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    let n = members.len();
    if n == 0 {
        return 0;
    }
    let dir = members[0].0.dir.clone();
    let now = match wtgit::head_commit(&dir) {
        Ok(h) => h,
        Err(m) => return fail_members(cfg, members, &m, rows, worst),
    };
    if now != head {
        for (mut r, t) in members {
            let id = r.e.id().to_string();
            let c = r.leave_train(STALE, &format!("main moved from {} to {} while merge train {train_id} was being checked; nothing was merged. Run `queue run --entry {id}` to rebase again", short(head), short(&now)), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return 0;
    }
    let dirty = match wtgit::status_porcelain_plain(&dir) {
        Ok(d) => d,
        Err(m) => return fail_members(cfg, members, &m, rows, worst),
    };
    if !dirty.is_empty() {
        for (mut r, t) in members {
            let c = r.leave_train(BLOCKED, &format!("the main checkout got uncommitted or untracked changes while merge train {train_id} was checked ({}); nothing merged", some(&dirty)), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return 0;
    }
    // BEFORE the fast-forward: every member records step `merge`, so a drain killed from here on leaves each one a half-finished
    // merge (`half_merged`) that the next run resumes at its step, never one that is chained into a new train while its commits may
    // already be on main. A member whose record cannot be written stops the train before main moves.
    let mut members = members;
    for (r, _) in members.iter_mut() {
        r.e.set("step", "merge");
        if r.save().is_err() {
            return fail_members(cfg, members, &format!("could not record the merge step of merge train {train_id} before its fast-forward; main was not moved"), rows, worst);
        }
    }
    if let Err(m) = wtgit::merge_ff_only(&dir, tip) {
        let after = wtgit::head_commit(&dir).unwrap_or_default();
        for (mut r, t) in members {
            let id = r.e.id().to_string();
            let c = if after != head {
                r.leave_train(STALE, &format!("main moved from {} to {} during the merge of train {train_id}; nothing was merged by this run. Run `queue run --entry {id}` to rebase again", short(head), short(&after)), EXIT_RETRY)
            } else {
                r.leave_train(BLOCKED, &format!("git refused the fast-forward of train {train_id}: {m}. Main is unchanged"), EXIT_RETRY)
            };
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return 0;
    }
    let after = match wtgit::head_commit(&dir) {
        Ok(h) => h,
        Err(m) => return fail_members(cfg, members, &m, rows, worst),
    };
    if after != tip {
        let msg = format!("after `git merge --ff-only` main's HEAD is {} but the train tip is {}; main was {} before; look at the project", short(&after), short(tip), short(head));
        return fail_members(cfg, members, &msg, rows, worst);
    }
    let last = n - 1;
    let last_id = members[last].0.e.id().to_string();
    for (i, (r, _)) in members.iter_mut().enumerate() {
        let own = r.e.get("reviewed_tip").to_string();
        let cli = r.e.get("reviewer_cli").to_string();
        r.e.set("main_sha", own);
        r.e.set("train", train_id);
        r.e.set("train_pos", (i + 1).to_string());
        r.e.set("train_job", pass.job.clone());
        r.e.set("check_job", pass.job.clone());
        r.e.set("check_exit", "0");
        r.e.set("check_summary", pass.summary.clone());
        r.e.set("check_tail", pass.tail.clone());
        r.e.set("check_deferred", "");
        r.e.set("train_checked", tip);
        r.e.set("main_check", if i == last { format!("full check of the train tip {} on main, run by this entry for the whole merge train {train_id}", short(tip)) } else { format!("one main check for the whole merge train {train_id}, run by its last entry {last_id}") });
        r.e.set("step", "merge");
        let _ = r.log("merge", &format!("merge train {train_id}, position {} of {n}: main fast-forwarded {} -> {} (the train tip, checked by job {}); reviewed through CLI {}", i + 1, short(head), short(tip), if pass.job.is_empty() { "?" } else { pass.job.as_str() }, if cli.is_empty() { "(none recorded)" } else { cli.as_str() }));
    }
    println!("merge train {train_id}: main fast-forwarded {} -> {} ({n} entr{}, one check, one fast-forward)", short(head), short(tip), if n == 1 { "y" } else { "ies" });
    // TP2 SEAM: tp2 (main == checked commit -> only a build on main) is not merged at this base, so this ONE main check is the full
    // `check` of main, run by the last entry (its commit is the tip); the others record that it covers them.
    if let Err(c) = members[last].0.s_main_check() {
        let code = code_num(c);
        let reason = members[last].0.e.get("reason").to_string();
        for (i, (mut r, t)) in members.into_iter().enumerate() {
            if i == last {
                finish_member(cfg, r, code, t, rows, worst);
                continue;
            }
            let c2 = if code == EXIT_MERGED_BAD { r.end(MERGED_CHECK_FAILED, &reason, EXIT_MERGED_BAD) } else { r.internal(&format!("the main check of merge train {train_id} did not complete: {reason}")) };
            finish_member(cfg, r, code_num(c2), t, rows, worst);
        }
        return code;
    }
    let mut hard = 0u8;
    for (i, (mut r, t)) in members.into_iter().enumerate() {
        if i != last {
            r.e.set("step", "main_check");
            let _ = r.log("main_check", &format!("covered by the one main check of merge train {train_id} (run by entry {last_id}), which passed"));
        }
        let code = code_num(match r.s_done() {
            Ok(c) | Err(c) => c,
        });
        if code == EXIT_ERROR {
            hard = EXIT_ERROR;
        }
        finish_member(cfg, r, code, t, rows, worst);
    }
    hard
}

/// The last `max` bytes of a file as text (empty if it cannot be read). Cut at a line start when it cut anything.
fn read_tail(path: &Path, max: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let cut = len > max;
    if cut && f.seek(SeekFrom::Start(len - max)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let s = String::from_utf8_lossy(&buf).into_owned();
    if cut {
        let rest = s.split_once('\n').map(|(_, r)| r.to_string()).unwrap_or(s);
        format!("[... earlier output cut: only the last {} KB are shown ...]\n{rest}", max / 1024)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end_and_says_it_cut() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-queue-tail-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.log");
        std::fs::write(&f, "line one\nline two\nline three\n").unwrap();
        assert_eq!(read_tail(&f, 1024), "line one\nline two\nline three\n");
        let t = read_tail(&f, 20);
        assert!(t.starts_with("[... earlier output cut") && t.ends_with("line three\n") && !t.contains("line one"), "{t}");
        assert_eq!(read_tail(&d.join("missing"), 10), "");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn same_model_ignores_case_and_space() {
        assert!(same_model("Opus", " opus "));
        assert!(!same_model("haiku", "opus"));
    }

    #[test]
    fn steps_are_in_the_documented_order() {
        assert_eq!(STEPS, ["preconditions", "rebase", "check", "review", "merge", "main_check", "done"]);
        assert_eq!(STEPS[STEP_MERGE], "merge");
        assert!(is_open(APPROVED), "an approved entry is open: its task cannot be queued again");
        assert!(is_open(QUEUED) && is_open(RUNNING) && is_open(BLOCKED) && is_open(STALE) && !is_open(MERGED) && !is_open(CANCELLED) && !is_open(REJECTED) && !is_open(CHECK_FAILED) && !is_open(CONFLICT));
    }

    fn item(project: &str, claims: &[&str]) -> (String, Vec<String>) {
        (project.to_string(), claims.iter().map(|c| c.to_string()).collect())
    }

    #[test]
    fn a_train_takes_only_entries_with_claims_that_do_not_overlap_and_only_from_one_project() {
        let items = vec![item("p", &["src/a.rs"]), item("p", &["src"]), item("q", &["src/a.rs"]), item("p", &["docs"]), item("p", &["src/b.rs"])];
        // `src` overlaps `src/a.rs` (and `src/b.rs`), so the second entry is left out; the other project's entry never joins
        assert_eq!(pick_train(&items, 3, false), vec![0, 3, 4], "full train: a, docs, b");
        assert_eq!(pick_train(&items, 4, false), Vec::<usize>::new(), "only 3 compatible entries: not full, so not ready without flush");
        assert_eq!(pick_train(&items, 4, true), vec![0, 3, 4], "flush takes what there is");
        // a task that could not be read claims everything and rides alone
        let items = vec![item("p", &["*"]), item("p", &["src/a.rs"])];
        assert_eq!(pick_train(&items, 2, false), Vec::<usize>::new());
        assert_eq!(pick_train(&items, 2, true), vec![0]);
        // two projects: the first project with a ready train goes first
        let items = vec![item("p", &["a"]), item("q", &["a"]), item("q", &["b"])];
        assert_eq!(pick_train(&items, 2, false), vec![1, 2]);
        assert_eq!(pick_train(&[], 2, true), Vec::<usize>::new());
    }

    #[test]
    fn a_train_never_exceeds_merge_train_max() {
        let items: Vec<(String, Vec<String>)> = (0..6)
            .map(|i| {
                let f = format!("f{i}");
                item("p", &[f.as_str()])
            })
            .collect();
        assert_eq!(pick_train(&items, 4, false), vec![0, 1, 2, 3]);
        assert_eq!(pick_train(&items, 1, false), vec![0], "1 = every entry on its own");
    }

    #[test]
    fn run_all_badness_orders_exit_codes_worst_first() {
        let mut codes = vec![0u8, 4, 2, 3, 5, 1, 6];
        codes.sort_by_key(|c| std::cmp::Reverse(badness(*c)));
        assert_eq!(codes, vec![6, 1, 5, 3, 2, 4, 0]);
    }

    #[test]
    fn run_all_exit_counts_the_stop_code_even_when_no_row_carries_it() {
        // a train check without a verdict: its entries stay approved (rows exit 3), the drain stopped with 1
        assert_eq!(worst_exit([3u8, 3, 0].into_iter(), EXIT_ERROR), EXIT_ERROR);
        assert_eq!(worst_exit([5u8, 3].into_iter(), EXIT_ERROR), EXIT_ERROR, "1 is worse than rejected / check_failed");
        assert_eq!(worst_exit([6u8, 3].into_iter(), EXIT_ERROR), EXIT_MERGED_BAD, "merged_check_failed stays the worst");
        assert_eq!(worst_exit([3u8, 0].into_iter(), 0), 3, "no stop: the worst row");
        assert_eq!(worst_exit(std::iter::empty(), 0), 0);
    }
}
