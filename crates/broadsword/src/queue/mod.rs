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
//!   merged              fast-forwarded into the project's trunk (see TRUNK), main's check passed, `task done` ran [0]
//!   blocked              a precondition failed or a step could not run (dirty main, task not running, dirty worktree, reviewer
//!                        model = coder model, diff too big, check could not run, reviewer unreachable or timed out); nothing
//!                        was merged and nothing was changed except what the entry says; fix the cause, `queue run --entry` [3]
//!   conflict             the rebase onto main conflicted; it was aborted and the branch is exactly as before; the coder
//!                        rebases or fixes it, then re-queues with `queue add` [3]
//!   stale                main (or the branch) moved after the rebase and the review; nothing merged; `queue run --entry` rebases again [3]
//!   check_failed         build or tests failed on the rebased branch; no model was asked; fix, commit, `queue add` again [5]
//!   perf_failed          the check passed but the PERF GATE failed (see PERF GATE): a bench case got slower than its baseline's
//!                        threshold_pct allows, or the project's baseline or bench could not be used (fail closed); no model was
//!                        asked; fix, commit, `queue add` again [5]
//!   rejected             the reviewer said FAIL (or gave no usable verdict); the task stays running and its worktree is kept;
//!                        fix, commit, `queue add` again [5]
//!   merged_check_failed  the branch was merged but `check` failed on main; main was NOT reset. With queue_auto_revert on, the queue
//!                        reverted the merged commits with one new commit (`Run::auto_revert`) and checked main again; if that
//!                        could not be done or did not pass, the lane is HELD (`hold_gate`) until a check of main passes [6]
//!   cancelled            dropped by `queue cancel`
//!
//! SHADOW REVIEW. When `shadow_reviewer` is `openai`, a second model reviews the same prompt after the gating verdict is recorded; its
//! verdict and cost are only recorded in the entry (`shadow_*`) and in `<entry id>.shadow.review`; it never changes the outcome (see `s_shadow`).
//!
//! Exit codes of `queue run`: 0 merged, 1 internal error (the entry stays `running`, resumable), 2 bad input, 3 blocked /
//! conflict / stale, 4 queue busy, 5 rejected / check_failed / perf_failed, 6 merged_check_failed.
//!
//! FAST SUBSET (setting `merge_fast_tests`). For a listed project every PRE-merge check (`pre_merge_check`: a single entry's own
//! check, and a merge train's tip, bisect and alone checks) is one `run` job of `cargo build` plus `cargo test [--lib] [--bins] --test <name>...`;
//! the record and the reviewer prompt say `check: fast subset (...)` (else `check: full`). The main check after a merge is always the
//! full `check`, and with queue_auto_revert (required on for a listed project) a failure there is reverted; when that revert leaves
//! main green (`green_revert`), `queue run --all` goes on with the next entry.
//!
//! PERF GATE (perfgate.rs, setting `perf_gate`). It is part of every check of a commit: right after the build and tests pass, the
//! entry's own project is benched on that commit (`perf_runs` times, as `run` jobs in the task's worktree, each holding the build
//! lock), when that project has a baseline `<state_dir>/perf/<project>-baseline.json`; a project without one is skipped (printed and
//! logged in the entry's `perf`), and no other project is ever benched. A case slower than its baseline's `threshold_pct` (per case,
//! from the file), or a baseline or bench that cannot be used, makes the check a FAILURE: `perf_failed`, with each case's baseline,
//! now and change in % in the reason. A bench job that never ran gives no verdict (`blocked`). Single entry: in the check step
//! (`s_check`), before the review. Merge train: ONCE on the chain tip, inside `Run::check_commit`, so a perf failure is bisected
//! exactly like a failed check (each bisect check and the culprit's own check include the gate), and the culprit ends `perf_failed`.
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
//! overlap form a train of up to merge_train_max entries (a lone one at the end of the drain too; and a SHORT train, even of 1, as soon
//! as no entry of that project is left for the drain to run, or its oldest approved entry has waited merge_train_wait_minutes, so one
//! project's approved entries never wait behind another project's stream: see `short_train_why`): their branches are chained one on
//! top of the other (rebase), ONE check runs on the chain tip, main is fast-forwarded ONCE and checked ONCE, and every entry is `merged`
//! with its train id, position and the shared check job. A failing check is bisected (see `run_train`): an entry that fails alone is
//! `check_failed`, the entries proven good merge; entries that only fail together are all held out and none merges (the first of them
//! `conflict`, the ones chained on it `blocked` by the left-train rule; see `run_train` 5). The review stays
//! bound to its commits: an entry whose rebased commits are not the same patches (`git patch-id --stable`) as the reviewed ones goes
//! back to `queued` and is reviewed again (`train_link`). Nothing unchecked ever reaches main: an entry whose check was deferred
//! (`check_deferred = yes`) is checked on its own first if it is ever merged outside a train (`merge_phase`). The two places nb-18
//! (check from a clean checkout of the exact commit) plugs in are `Run::check_commit` and `Run::check_alone`.
//!
//! PARALLEL REVIEW (setting `review_parallel`, only with the merge train on, in `queue run --all`): up to review_parallel entries are
//! brought to their review verdict at once, one worker thread per entry, everything after the review serial and in queue order (see
//! `drive_all`). review_parallel = 1: one entry at a time, exactly as before.
//!
//! LOCKS. `queue.lock` is held for the whole `queue run` (minutes), so one entry is processed at a time (PARALLEL REVIEW: one batch); a second `queue run` waits
//! `queue_lock_wait_seconds`, then exits 4. `entries.lock` is held for milliseconds, around "is this task already queued, create
//! the entry" and around choosing the next entry. Order is always queue.lock then entries.lock. The TASK BOARD lock is never held
//! during the rebase, check, review or merge: it is taken only by `task done` (inside `task::cmd_finish`), which is short.
//! PER-PROJECT RUN LOCK (setting `queue_per_project`, SCALING F2, shipped "off"). Everything above that says queue.lock means THE RUN
//! LOCK, taken in one place (`run_lock`). "off": it is the one global queue.lock. "on": it is `<state_dir>/queue/<project>.queue.lock`,
//! one per project (the same OS lock), and a run only ever takes entries of its own project (`Lane::holds`), so "running while nobody
//! holds the lock" stays exact per project and drivers of different projects run at once; `queue run` then needs `--project` or
//! `--entry` (exit 2 otherwise). entries.lock stays global, merge trains stay per project. LIMIT: a run started under one value does
//! not see the lock of a run started under the other, so the setting must not be switched while a `queue run` is going.
//!
//! ONE COMMIT. The check records `checked_tip`, the commit it ran on. The review diff is `<base>...<checked_tip>`, the reviewer is
//! called only while the branch tip still equals it, and `merge --ff-only` merges exactly that commit. A tip that moved at any
//! point makes the entry `stale`; an unchecked commit is never reviewed or merged.
//!
//! TRUNK. "Main" in this file is the project's TRUNK (`wtgit::trunk_branch`: the one of `main`/`master` that exists; both or
//! neither ends the entry `blocked` with that error), recorded as the entry's `main_branch` when it starts, and "main's HEAD" is
//! `refs/heads/<trunk>`. Every "is it on main" test reads that ref, never HEAD (`trunk_tip`). The rebase base, every fast-forward and
//! the main check also need the main checkout to have the trunk checked out (`trunk_checkout`): a feature branch or a detached HEAD
//! there ends the entry `blocked` naming both, and that branch and the checkout are never moved, merged into or switched.
//!
//! SAFETY. The only commands that move history are `git rebase <trunk tip>` in the task's worktree (aborted on any failure, the tip
//! verified) and `git merge --ff-only <reviewed commit>` in the main checkout while it has the trunk checked out (so the commit
//! merged is exactly the one reviewed, into the trunk only; never a merge commit). Never `--force`, `-D`, `reset --hard` or `push`. Before a rebase the pre-rebase tip is saved in the entry
//! and, create-only, under `refs/broadsword/queue/<entry id>/pre-rebase` (a later attempt: `pre-rebase-<N>`). All git commands are
//! in wtgit.rs.

mod add;
mod cancel;
mod check;
mod entry;
mod locks;
mod merge;
mod rebase;
mod review;
mod train;
mod view;

pub use add::cmd_add;
pub use cancel::cmd_cancel;
pub use entry::queue_dir;
pub use view::{cmd_list, cmd_show};
use crate::config::Config;
use crate::job;
use crate::record::Record;
use crate::reviewer;
use crate::task::{fail, now_secs};
use crate::sys;
use std::path::PathBuf;
use std::process::ExitCode;
use entry::{half_merged, load_entries, load_entry, resume_first};
use locks::{entries_lock, hold_gate, Lane, lane_for, run_lock};
use merge::green_revert;
use review::SHADOW_KEYS;
use train::{claim_approved, run_trains, settle_approved, train_stop_note};

/// The queue's whole source text, every file of src/queue/ (this one first), for the tests in tools.rs and task.rs that check
/// the tool descriptions and the task board's status lists against what the queue's code says. A file added to src/queue/
/// must be listed here: `source_lists_every_file_of_the_queue_folder` fails until it is.
#[cfg(test)]
pub(crate) const SOURCE: &str = concat!(
    include_str!("mod.rs"),
    "\n",
    include_str!("add.rs"),
    "\n",
    include_str!("cancel.rs"),
    "\n",
    include_str!("check.rs"),
    "\n",
    include_str!("entry.rs"),
    "\n",
    include_str!("locks.rs"),
    "\n",
    include_str!("merge.rs"),
    "\n",
    include_str!("rebase.rs"),
    "\n",
    include_str!("review.rs"),
    "\n",
    include_str!("train.rs"),
    "\n",
    include_str!("view.rs"),
);

pub const QUEUED: &str = "queued";
pub const RUNNING: &str = "running";
pub const MERGED: &str = "merged";
pub const APPROVED: &str = "approved";
pub const BLOCKED: &str = "blocked";
pub const CONFLICT: &str = "conflict";
pub const STALE: &str = "stale";
pub const CHECK_FAILED: &str = "check_failed";
pub const PERF_FAILED: &str = "perf_failed";
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
const STEPS: [&str; 7] = ["preconditions", "rebase", "check", "review", "merge", "main_check", "done"];
const STEP_MERGE: usize = 4;

type R<T> = Result<T, ExitCode>;

/// An exit code that stops `queue run --all`: an internal error, or merged_check_failed unless its revert left main green.
fn stops_drain(cfg: &Config, id: &str, code: u8) -> bool {
    code == EXIT_ERROR || (code == EXIT_MERGED_BAD && !green_revert(cfg, id))
}

// ---- run --------------------------------------------------------------------------------

/// `queue run [--entry <id>] [--project <p>]`: one entry. `project` limits the choice to that project's entries (see `run_lock` for
/// the lock it takes).
pub fn cmd_run(cfg: &Config, entry: Option<&str>, project: Option<&str>) -> ExitCode {
    let lane = match lane_for(cfg, entry, project).and_then(|k| run_lock(cfg, k)) {
        Ok(l) => l,
        Err(c) => return c,
    };
    if let Err(c) = hold_gate(cfg, &lane) {
        return c;
    }
    // Choose and claim the entry under the short lock. Holding the lane's run lock already means no other run of the lane exists,
    // so every `running` entry of the lane is an interrupted one.
    let chosen = claim(cfg, |all| match entry {
        Some(id) => match all.iter().find(|x| x.id() == id) {
            Some(x) if !lane.holds(x) => Err(fail(EXIT_BAD_INPUT, format!("queue entry {id} is of project `{}`, outside this run's project; nothing was run", x.get("project")))),
            Some(x) if matches!(x.status(), QUEUED | RUNNING | STALE | BLOCKED | APPROVED) => Ok(Some(x.clone())),
            Some(x) => Err(not_runnable(x)),
            None => Err(fail(EXIT_BAD_INPUT, format!("no queue entry with the exact id `{id}` (see `broadsword queue list`)"))),
        },
        None => {
            let mine: Vec<&Record> = all.iter().filter(|x| lane.holds(x)).collect();
            Ok(mine.iter().find(|x| resume_first(x)).or_else(|| mine.iter().find(|x| matches!(x.status(), QUEUED | STALE))).or_else(|| mine.iter().find(|x| x.status() == APPROVED)).map(|x| (*x).clone()))
        }
    });
    let e = match chosen {
        Ok(Some(e)) => e,
        Ok(None) => {
            println!("the queue is empty{}: nothing is queued, interrupted or stale", in_project(&lane));
            return ExitCode::SUCCESS;
        }
        Err(c) => return c,
    };
    match begin(cfg, e) {
        Ok(mut run) => run.go(),
        Err((_, code)) => code,
    }
}

/// ` for project <p>` when the lane is one project, for the "queue is empty" line.
fn in_project(lane: &Lane) -> String {
    lane.key.as_ref().map(|k| format!(" for project `{k}`")).unwrap_or_default()
}

/// `queue run` refuses a finished entry: exit 2 with the way on.
fn not_runnable(x: &Record) -> ExitCode {
    let hint = match x.status() {
        REJECTED | CHECK_FAILED | PERF_FAILED | CONFLICT => "fix the code, commit, and queue the task again with `queue add`",
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
            "train", "train_pos", "train_job", "train_up", "train_onto", "train_checked", "train_bisect", "alone_tip", "reviewed_orig_tip", "check_deferred", "main_check", "step", "reason", "error", "pre_rebase", "pre_rebase_ref", "base_head", "rebased_tip", "checked_tip", "reviewed_tip", "reviewer_cli", "test_hook", "check_job", "check_exit", "check_summary", "check_tail", "perf", "done_check", "done_check_file",
            "reviewer_model", "gate", "gate_model_used", "verdict", "blocking", "minor", "cost_usd", "tokens_in", "tokens_out", "review_file", "main_sha", "main_branch", "ended", "pre_merge_sha", "revert_sha", "revert_check",
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
fn next_for_all(all: &[Record], taken: &[String], train: bool, lane: &Lane) -> Option<Record> {
    let fresh = |x: &&Record| lane.holds(x) && !taken.iter().any(|t| t == x.id());
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
/// entries are merged in trains (see `run_train`): whenever a full train is ready, whenever a project's short train is due
/// (`short_train_why`: none of its entries left to run, or its oldest approved entry waited merge_train_wait_minutes), and at the
/// end of the drain for whatever is left.
///
/// `project` (`--project <p>`): only that project's entries are drained (with queue_per_project on it is required, and the run holds
/// only that project's lock: see `run_lock`).
pub fn cmd_run_all(cfg: &Config, project: Option<&str>) -> ExitCode {
    let lane = match lane_for(cfg, None, project).and_then(|k| run_lock(cfg, k)) {
        Ok(l) => l,
        Err(c) => return c,
    };
    if let Err(c) = hold_gate(cfg, &lane) {
        return c;
    }
    let train = cfg.merge_train_max > 1;
    if !train && cfg.review_parallel > 1 {
        // PARALLEL REVIEW needs the merge train: without it each entry merges on the main it was rebased on, so a batch's later
        // entries would end stale after the first merge and be reviewed again (see `parallel_ok`, `s_merge`)
        println!("queue run --all: parallel reviews are off because the merge train is off (review_parallel = {}, merge_train_max = {}): entries are reviewed one at a time", cfg.review_parallel, cfg.merge_train_max);
    }
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
        let left: Vec<Record> = match claim_approved(cfg, &lane) {
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
        let e = match claim(cfg, |all| Ok(next_for_all(all, &taken, train, &lane))) {
            Ok(Some(e)) => e,
            Ok(None) => {
                // MERGE TRAIN: nothing else to run: every approved entry left (even a single one) goes into a last train
                if train && !approved.is_empty() {
                    let code = run_trains(cfg, &mut approved, true, &taken, &mut rows);
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
        // PARALLEL REVIEW (review_parallel > 1, merge train on): more entries are claimed now, each exactly as the next turn of this
        // loop would claim it, as long as each is one a train member's worker may take (`parallel_ok`). With review_parallel = 1, or
        // when only one entry is ready, the batch is this one entry and everything below runs as it always did.
        let mut batch = vec![e];
        let mut claim_err = None;
        if train && cfg.review_parallel > 1 && parallel_ok(&batch[0]) {
            while (batch.len() as u64) < cfg.review_parallel {
                match claim(cfg, |all| Ok(next_for_all(all, &taken, train, &lane).filter(parallel_ok))) {
                    Ok(Some(x)) => {
                        taken.push(x.id().to_string());
                        batch.push(x);
                    }
                    Ok(None) => break,
                    Err(c) => {
                        claim_err = Some(c);
                        break;
                    }
                }
            }
        }
        let mut slots: Vec<Slot> = Vec::new();
        for e in batch {
            let t = std::time::Instant::now();
            match begin(cfg, e) {
                Ok(mut r) => {
                    r.defer_check = train;
                    let start = r.start_step();
                    slots.push(Slot::Run(r, t, start, 0));
                }
                Err((id, c)) => slots.push(Slot::Ended(id, code_num(c), t)),
            }
        }
        drive_all(&mut slots, train);
        // From here on everything is serial and in queue order, exactly as for one entry: the summary rows, the approved list, then
        // the trains (and so every train check, fast-forward and main check).
        let mut codes: Vec<(String, u8, usize)> = Vec::new();
        for slot in slots {
            match slot {
                Slot::Ended(id, code, t) => rows.push(row_for(cfg, &id, code, t)),
                Slot::Run(run, t, start, code) => {
                    let id = run.e.id().to_string();
                    if run.e.status() == APPROVED {
                        // approved entries keep their Run until their train has finished with them
                        approved.push((run, t));
                    } else {
                        rows.push(row_for(cfg, &id, code, t));
                    }
                    codes.push((id, code, start));
                }
            }
        }
        // a merged_check_failed whose automatic revert left main green (`green_revert`) does not stop the drain: the next entry is
        // rebased onto that green main, and the drain's exit code still counts the 6 (`worst_exit`)
        let Some(stop_at) = codes.iter().position(|(id, c, _)| stops_drain(cfg, id, *c)).or(if codes.is_empty() { None } else { Some(codes.len() - 1) }) else {
            if let Some(c) = claim_err {
                stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
                rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
                break;
            }
            continue; // every entry of the batch ended before it ran (unknown project): as before, no train for them
        };
        let (id, code, start) = codes[stop_at].clone();
        let mut train_code = 0u8;
        // no new train while a half-finished merge is being finished (`resume_first` entries come first): a train only starts once
        // main's last merge has had its main check
        let stop = stops_drain(cfg, &id, code);
        if train && start < STEP_MERGE && !approved.is_empty() && !stop {
            train_code = run_trains(cfg, &mut approved, false, &taken, &mut rows);
        }
        let hard = if stop { code } else { train_code };
        if hard == EXIT_ERROR || hard == EXIT_MERGED_BAD {
            stop_code = hard;
            stopped = if hard == train_code && train_code != 0 { train_stop_note(hard) } else { format!("stopped after entry {id} (exit {code}): {}", if code == EXIT_ERROR { "it stays `running` at its step; fix the cause, then `queue run` resumes it" } else { "main holds a commit whose check failed; a human decides before anything else merges" }) };
            settle_approved(cfg, &mut approved, &mut rows);
            break;
        }
        if let Some(c) = claim_err {
            stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
            rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
            break;
        }
    }
    let worst = worst_exit(rows.iter().map(|r| r.code), stop_code);
    if rows.is_empty() {
        println!("the queue is empty{}: nothing is queued, interrupted or stale", in_project(&lane));
    } else {
        println!("--- queue run --all{}: {} entr{} in {:.1} s ---", lane.key.as_ref().map(|k| format!(" --project {k}")).unwrap_or_default(), rows.len(), if rows.len() == 1 { "y" } else { "ies" }, t0.elapsed().as_secs_f64());
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

// ---- PARALLEL REVIEW (review_parallel) ------------------------------------------------------------------------------
//
// `queue run --all` with the merge train on (merge_train_max > 1) and review_parallel > 1 claims up to review_parallel entries at
// once (each with the same `claim` and `next_for_all` the serial loop uses, so the same entries in the same order) and brings them
// to their review verdict together, one worker thread per entry (`drive_all`):
//   - each worker owns its entry's Run (its record and its files: <entry>.entry, .review, .shadow.review, .donecheck); no worker
//     ever reads or writes another entry's record, so a verdict can only be written to its own entry;
//   - the steps before the review (preconditions, rebase, the deferred check, the done check) run one worker at a time, in queue
//     order (`Turn`): git writes to the shared repository and done-check jobs never overlap, exactly as in the serial drain;
//   - the reviews (gate call, then shadow call, per entry, with the same rules, models, timeouts and reviewer_max_calls_per_entry)
//     run side by side, except that every call through clients/openai_review.py (GPT gate, GPT shadow) holds `GPT_CALL`, so the GPT
//     daily cap is checked call by call with every earlier call already in the ledger, as in the serial drain;
//   - a worker only ever ends its entry `approved` or finished (rejected, blocked, stale, ...): `parallel_ok` admits only entries
//     whose run starts before the merge step, and with the merge train on such a run stops at `approved` (`drive`). No worker
//     merges, builds a train, checks a train or fast-forwards: all of that runs afterwards, on the main thread, serially, in queue order.
// The run lock (queue.lock) is held by the drain for all of it; entries.lock is taken for each claim as before. A drain killed
// mid-review leaves each claimed entry `running` at its recorded step, and the next `queue run` resumes each one as it always does.

/// Can this entry be one of a parallel batch? Only an entry whose run starts before the merge step and that is not a half-finished
/// train merge: those (and everything after a review) run on the main thread only.
fn parallel_ok(e: &Record) -> bool {
    !half_merged(e) && STEPS.iter().position(|s| *s == e.get("step")).unwrap_or(0) < STEP_MERGE
}

/// One claimed entry of a batch: `Run(run, started, start step, exit code once driven)`, or `Ended(id, exit code, started)` for one
/// `begin` already ended (its project is unknown).
enum Slot<'a> {
    Run(Run<'a>, std::time::Instant, usize, u8),
    Ended(String, u8, std::time::Instant),
}

/// Turns in queue order: `wait(i)` returns once turns 0..i are over; the turn ends when the guard drops (also on an early return).
struct Turn {
    next: std::sync::Mutex<usize>,
    cv: std::sync::Condvar,
}

struct TurnGuard<'t>(&'t Turn);

impl Turn {
    fn wait(&self, i: usize) -> TurnGuard<'_> {
        let mut n = self.next.lock().unwrap_or_else(|p| p.into_inner());
        while *n != i {
            n = self.cv.wait(n).unwrap_or_else(|p| p.into_inner());
        }
        TurnGuard(self)
    }
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        *self.0.next.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.0.cv.notify_all();
    }
}

/// One entry through the steps up to its verdict and, without the merge train, its merge: exactly the serial drain's body. With
/// `turn`, the steps before the review wait for their turn (PARALLEL REVIEW).
fn drive(run: &mut Run<'_>, start: usize, train: bool, turn: Option<(&Turn, usize)>) -> u8 {
    let result = (|| -> R<ExitCode> {
        {
            let _t = turn.map(|(t, i)| t.wait(i));
            run.until_review(start)?;
        }
        run.review_phase(start)?;
        if train && start <= 3 {
            run.approve() // MERGE TRAIN: the entry waits at `approved` for its train
        } else {
            run.merge_phase(start)
        }
    })();
    code_num(match result {
        Ok(c) | Err(c) => c,
    })
}

/// Drive every `Slot::Run` of a batch and record its exit code: one alone on this thread (the serial drain), more side by side,
/// one worker thread each (PARALLEL REVIEW: they are all `parallel_ok` with the train on, so each stops at its verdict).
fn drive_all(slots: &mut [Slot<'_>], train: bool) {
    let runs = slots.iter().filter(|s| matches!(s, Slot::Run(..))).count();
    if runs <= 1 {
        for s in slots.iter_mut() {
            if let Slot::Run(run, _, start, code) = s {
                *code = drive(run, *start, train, None);
            }
        }
        return;
    }
    println!("queue run --all: PARALLEL REVIEW of {runs} entries (review_parallel): their steps up to the review run one after another in queue order, their reviews side by side; trains and merges follow, one at a time");
    let turn = Turn { next: std::sync::Mutex::new(0), cv: std::sync::Condvar::new() };
    std::thread::scope(|sc| {
        let mut i = 0;
        for s in slots.iter_mut() {
            if let Slot::Run(run, _, start, code) = s {
                let (turn, at, start) = (&turn, i, *start);
                i += 1;
                sc.spawn(move || {
                    *code = drive(run, start, train, Some((turn, at)));
                });
            }
        }
    });
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

    /// Step 4, review.
    fn review_phase(&mut self, start: usize) -> R<()> {
        if start <= 3 {
            self.s_review()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::entry::is_open;

    #[test]
    fn steps_are_in_the_documented_order() {
        assert_eq!(STEPS, ["preconditions", "rebase", "check", "review", "merge", "main_check", "done"]);
        assert_eq!(STEPS[STEP_MERGE], "merge");
        assert!(is_open(APPROVED), "an approved entry is open: its task cannot be queued again");
        assert!(is_open(QUEUED) && is_open(RUNNING) && is_open(BLOCKED) && is_open(STALE) && !is_open(MERGED) && !is_open(CANCELLED) && !is_open(REJECTED) && !is_open(CHECK_FAILED) && !is_open(CONFLICT));
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

    /// SOURCE is exactly every .rs file of src/queue/, each once, joined by one line break: a file missing from it would let the
    /// source-text tests in tools.rs and task.rs pass without ever reading that file.
    #[test]
    fn source_lists_every_file_of_the_queue_folder() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("queue");
        let mut files: Vec<String> = Vec::new();
        for e in std::fs::read_dir(&dir).expect("src/queue can be listed") {
            let p = e.expect("src/queue entry").path();
            if p.extension().is_some_and(|x| x == "rs") {
                files.push(std::fs::read_to_string(&p).unwrap_or_else(|err| panic!("{}: {err}", p.display())));
            }
        }
        assert!(files.len() > 1, "src/queue holds the facade and its child modules");
        for text in &files {
            assert!(SOURCE.contains(text.as_str()), "a file of src/queue is not in queue::SOURCE (list it there)");
        }
        let total: usize = files.iter().map(|t| t.len()).sum::<usize>() + files.len() - 1;
        assert_eq!(SOURCE.len(), total, "queue::SOURCE holds something other than the files of src/queue, each once");
    }
}
