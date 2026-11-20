//! The reviewer step and merge queue: `broadsword queue add|list|show|run|cancel`.
//!
//! One durable record per queued branch, `<state_dir>/queue/<entry id>.entry` (the same atomic `key = value` records as
//! jobs and tasks, see record.rs; never deleted by cleanup), and one review file per attempt that reached the reviewer,
//! `<entry id>.review` (the second attempt: `<entry id>.2.review`), holding the reviewer's full raw output, cost and tokens.
//!
//! STATUSES (the exit code of `queue run` in brackets):
//!   queued               waiting for `queue run`
//!   running              a `queue run` is working on it, or died while doing so (see "Crash and resume")
//!   merged               fast-forwarded into the main checkout, main's check passed, `task done` ran [0]
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
//!   merge     if main's HEAD already is the reviewed commit, the fast-forward is recorded as done; otherwise it is attempted;
//!   main_check, done   run again (`task done` on a task that is already done is skipped).
//! To give up instead: `queue cancel <id>` on an interrupted entry aborts a leftover rebase, touches nothing else, and is refused
//! once the merge step began (main may already hold the commit: resume instead).
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

/// An entry is "open" while `queue run` can still act on it (queued, running, blocked, stale): it blocks `queue add` for its task.
/// Finished entries do not: merged, cancelled, rejected, and also check_failed, conflict and merged_check_failed, which `queue run`
/// refuses to run again (the documented way on is a new `queue add`, or a human for merged_check_failed).
fn is_open(status: &str) -> bool {
    matches!(status, QUEUED | RUNNING | BLOCKED | STALE)
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
        return fail(EXIT_RETRY, format!("task {task_id} is already in the queue as {} ({}); a task has one open entry at a time (queued, running, blocked and stale entries are open). Run it with `queue run --entry {}`, or cancel it with `queue cancel {}`, then add again", x.id(), x.status(), x.id(), x.id()));
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
            QUEUED | BLOCKED | STALE => {
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
    let mut e = {
        let _l = match entries_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        let all = match load_entries(cfg) {
            Ok(a) => a,
            Err(m) => return fail(EXIT_ERROR, m),
        };
        let chosen = match entry {
            Some(id) => match all.iter().find(|x| x.id() == id) {
                Some(x) if matches!(x.status(), QUEUED | RUNNING | STALE | BLOCKED) => x.clone(),
                Some(x) => {
                    let hint = match x.status() {
                        REJECTED | CHECK_FAILED | CONFLICT => "fix the code, commit, and queue the task again with `queue add`",
                        _ => "it is finished",
                    };
                    return fail(EXIT_BAD_INPUT, format!("queue entry {id} is `{}` and cannot be run; {hint}", x.status()));
                }
                None => return fail(EXIT_BAD_INPUT, format!("no queue entry with the exact id `{id}` (see `broadsword queue list`)")),
            },
            None => match all.iter().find(|x| x.status() == RUNNING).or_else(|| all.iter().find(|x| matches!(x.status(), QUEUED | STALE))) {
                Some(x) => x.clone(),
                None => {
                    println!("the queue is empty: nothing is queued, interrupted or stale");
                    return ExitCode::SUCCESS;
                }
            },
        };
        let mut e = chosen;
        if e.status() != RUNNING {
            // a fresh attempt: forget what the last attempt learned (its log and review file stay)
            for k in [
                "step", "reason", "error", "pre_rebase", "pre_rebase_ref", "base_head", "rebased_tip", "checked_tip", "reviewed_tip", "reviewer_cli", "test_hook", "check_job", "check_exit", "check_summary", "check_tail", "reviewer_model", "verdict",
                "blocking", "minor", "cost_usd", "tokens_in", "tokens_out", "review_file", "main_sha", "main_branch", "ended",
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
        if let Err(err) = e.save() {
            return fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display()));
        }
        e
    };
    let resumed = !e.get("step").is_empty();
    let dir = match job::resolve_project(&cfg.root, &cfg.projects_dir, e.get("project")) {
        Ok(d) => d,
        Err(m) => {
            let mut run = Run::new(cfg, e, PathBuf::new());
            return run.end(BLOCKED, &format!("the project is unknown: {m}"), EXIT_RETRY);
        }
    };
    println!("queue entry {}: {} (task {}, project {}, risk {}, coder {}, attempt {})", e.id(), if resumed { "RESUMING after an interrupted run, from step ".to_string() + e.get("step") } else { "starting".into() }, e.get("task"), e.get("project"), e.get("risk"), e.get("coder"), e.get("attempt"));
    e.set("error", "");
    let mut run = Run::new(cfg, e, dir);
    run.go()
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
}

impl<'a> Run<'a> {
    fn new(cfg: &'a Config, e: Record, dir: PathBuf) -> Run<'a> {
        Run { cfg, e, dir, wt: PathBuf::new(), secrets: process_secrets(cfg) }
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

    fn pipeline(&mut self) -> R<ExitCode> {
        let start = STEPS.iter().position(|s| *s == self.e.get("step")).unwrap_or(0);
        if start < STEP_MERGE {
            self.s_preconditions(start)?;
        }
        if start <= 1 {
            self.s_rebase()?;
        }
        if start <= 2 {
            self.s_check()?;
        }
        if start <= 3 {
            self.s_review()?;
        }
        if start <= STEP_MERGE {
            self.s_merge()?;
        }
        if start <= 5 {
            self.s_main_check()?;
        }
        self.s_done()
    }

    // -- step 1: preconditions --------------------------------------------------------------

    /// Find the task's worktree (and prove it is the task's), or end blocked.
    fn locate(&mut self) -> R<()> {
        match worktree::job_dir(self.cfg, self.e.get("task"), self.e.get("project")) {
            Ok(p) => {
                self.wt = p;
                Ok(())
            }
            Err(JobDirError::Unknown(m) | JobDirError::Refused(m)) => Err(self.blocked(&m)),
        }
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
        // A stand-in reviewer (a scratch test hook) may only merge into a project that itself lives in scratch.
        let cli_path = self.e.get("reviewer_cli").to_string();
        if self.e.get("test_hook") == "true" {
            let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
            if !canon(&dir).starts_with(canon(&self.cfg.scratch_dir)) {
                return Err(self.blocked(&format!("the review was made by a TEST HOOK reviewer ({cli_path}), not a real one, and project folder {} is outside scratch_dir; a stand-in review never merges into a real project; nothing merged", dir.display())));
            }
        }
        println!("queue entry {}: merging after a {} review through reviewer CLI {}", self.e.id(), if self.e.get("test_hook") == "true" { "TEST HOOK" } else { "real" }, if cli_path.is_empty() { "(none recorded)" } else { &cli_path });
        let main_branch = self.g(wtgit::current_branch(&dir))?;
        if main_branch.as_deref() != Some(self.e.get("main_branch")) {
            let now = main_branch.unwrap_or_else(|| "a detached HEAD".into());
            let was = self.e.get("main_branch").to_string();
            return Err(self.blocked(&format!("the main checkout is on {now}, but this entry was reviewed against `{was}`; nothing merged")));
        }
        let head = self.g(wtgit::head_commit(&dir))?;
        if head == reviewed {
            // an earlier run already fast-forwarded main to exactly this commit and died before recording it
            self.e.set("main_sha", head.clone());
            return self.log("merge", &format!("main's HEAD already is the reviewed commit {}: recorded as merged", short(&head)));
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
        assert!(is_open(QUEUED) && is_open(RUNNING) && is_open(BLOCKED) && is_open(STALE) && !is_open(MERGED) && !is_open(CANCELLED) && !is_open(REJECTED) && !is_open(CHECK_FAILED) && !is_open(CONFLICT));
    }
}
