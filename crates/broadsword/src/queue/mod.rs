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
mod entry;
mod locks;
mod train;

pub use add::cmd_add;
pub use entry::queue_dir;
pub use train::{cmd_cancel, cmd_list, cmd_run, cmd_run_all, cmd_show};
use crate::config::Config;
use crate::record::Record;
use crate::reviewer;
use std::path::PathBuf;
use std::process::ExitCode;

/// The queue's whole source text, every file of src/queue/ (this one first), for the tests in tools.rs and task.rs that check
/// the tool descriptions and the task board's status lists against what the queue's code says. A file added to src/queue/
/// must be listed here: `source_lists_every_file_of_the_queue_folder` fails until it is.
#[cfg(test)]
pub(crate) const SOURCE: &str = concat!(
    include_str!("mod.rs"),
    "\n",
    include_str!("add.rs"),
    "\n",
    include_str!("entry.rs"),
    "\n",
    include_str!("locks.rs"),
    "\n",
    include_str!("train.rs"),
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

#[cfg(test)]
mod tests {
    use super::*;

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
