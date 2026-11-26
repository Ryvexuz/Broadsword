use crate::config::Config;
use crate::job::JobInfo;
use crate::record;
use crate::task;
use crate::worktree;
use crate::{tools, wtgit};
use std::path::Path;
use std::process::ExitCode;
use super::check::{main_check_now, PassInfo};
use super::entry::load_entry;
use super::locks::{lane_hold, write_hold};
use super::{BLOCKED, code_num, EXIT_ERROR, EXIT_MERGED_BAD, EXIT_RETRY, MERGED, MERGED_CHECK_FAILED, R, Row, Run, STALE, STEP_MERGE};
use super::train::{fail_members, finish_member};
use super::view::{short, some};

/// Did entry `id` end merged_check_failed with its automatic revert DONE and the check after it PASSED, its lane not held? Then
/// main is green again and `queue run --all` goes on with the next entry; anything else (no revert, a failed one, a held lane, an
/// entry that cannot be read) stops the drain as before (fail closed).
pub(super) fn green_revert(cfg: &Config, id: &str) -> bool {
    load_entry(cfg, id).is_ok_and(|e| e.status() == MERGED_CHECK_FAILED && e.get("revert_check") == "passed" && lane_hold(cfg, e.get("project")).is_none())
}

impl<'a> Run<'a> {
    /// Steps 5 to 7: merge, main check, done. An entry whose check was deferred to a merge train that never checked it (a train that
    /// died, or `queue run` of an `approved` entry on its own) is checked alone first: nothing unchecked ever merges.
    pub(super) fn merge_phase(&mut self, start: usize) -> R<ExitCode> {
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

    /// Is the reviewed commit already on the trunk (`refs/heads/<trunk>`: equal to it or an ancestor of it)? False when none is
    /// recorded. A trunk that cannot be told ends the entry `blocked` (`trunk_tip`).
    fn reviewed_on_main(&mut self) -> R<bool> {
        let reviewed = self.e.get("reviewed_tip").to_string();
        if reviewed.is_empty() {
            return Ok(false);
        }
        let dir = self.dir.clone();
        let head = match trunk_tip(&dir, self.e.get("main_branch")) {
            Ok((_, tip)) => tip,
            Err(m) => return Err(self.blocked(&format!("{m}; nothing merged"))),
        };
        self.g(wtgit::is_ancestor(&dir, &reviewed, &head))
    }

    /// `trunk_checkout` for this entry (its recorded trunk): the trunk's tip, or the entry ends `blocked` with the reason.
    pub(super) fn on_trunk(&mut self) -> R<String> {
        match trunk_checkout(&self.dir, self.e.get("main_branch")) {
            Ok((_, tip)) => Ok(tip),
            Err(m) => Err(self.blocked(&format!("{m}; nothing merged"))),
        }
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
        // the trunk (`refs/heads/<trunk>`), checked out in the main checkout, is what is fast-forwarded: never another branch
        let head = self.on_trunk()?;
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
        if let Some(why) = lane_hold(self.cfg, self.e.get("project")) {
            return Err(self.blocked(&format!("lane {} held: {why}; nothing merged. Clear with a passing check of main, then queue run", self.e.get("project"))));
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
        // the last look before the fast-forward: still the trunk checked out, still at `head`
        if self.on_trunk()? != head {
            return Err(self.end(STALE, &format!("main moved from {} just before the merge; nothing was merged. Run `queue run --entry {}` to rebase again", short(&head), self.e.id()), EXIT_RETRY));
        }
        // AUTO-REVERT: the trunk tip before the fast-forward; `pre_merge_sha..main_sha` is exactly what this entry merged
        self.e.set("pre_merge_sha", head.clone());
        self.save()?;
        if let Err(m) = wtgit::merge_ff_only(&dir, &reviewed) {
            let now = self.g(trunk_tip(&dir, "").map(|(_, tip)| tip))?;
            if now != head {
                return Err(self.end(STALE, &format!("main moved from {} to {} during the merge; nothing was merged by this run. Run `queue run --entry {}` to rebase again", short(&head), short(&now), self.e.id()), EXIT_RETRY));
            }
            return Err(self.blocked(&format!("git refused the fast-forward: {m}. Main is unchanged")));
        }
        let mb = self.e.get("main_branch").to_string();
        let after = self.g(trunk_checkout(&dir, &mb).map(|(_, tip)| tip))?;
        if after != reviewed {
            return Err(self.internal(&format!("after `git merge --ff-only` the trunk `{mb}` is at {} but the reviewed commit is {}; it was {} before; look at the project", short(&after), short(&reviewed), short(&head))));
        }
        self.e.set("main_sha", after.clone());
        let note = format!("fast-forwarded `{}` from {} to {}; reviewed through CLI {} (test_hook: {})", self.e.get("main_branch"), short(&head), short(&after), if cli_path.is_empty() { "(none recorded)" } else { &cli_path }, self.e.get("test_hook"));
        self.log("merge", &note)
    }

    /// A stand-in reviewer (a scratch test hook) may only merge into a project that itself lives in scratch. The ONE place this rule
    /// is decided: the single-entry merge (`s_merge`) and the merge train (`train_ready`, before any chaining) both ask it.
    pub(super) fn test_hook_refusal(&self) -> Option<String> {
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
        let spec = self.planned_main_spec()?;
        let full = spec.kind == "check";
        let (code, info) = super::postcheck::run_check(self.cfg, spec);
        self.judge_planned_main_check(full, code, info)
    }

    pub(super) fn prepare_main_check(&mut self) -> R<()> {
        self.set_step("main_check")?;
        let project = self.e.get("project").to_string();
        let sha = self.e.get("main_sha").to_string();
        let mb = self.e.get("main_branch").to_string();
        let loud = |code: String, extra: &str| {
            format!("commit {sha} is ALREADY on `{mb}` of project `{project}`, and `check {project}` there did not pass ({code}){extra}. Main was NOT reset or reverted. A human decides: fix forward, or `git revert` the commit. The task was NOT marked done and its worktree is kept")
        };
        // The main check runs in the main checkout, on its files: only while that checkout is the trunk and holds the merged
        // commit is it a check of the trunk. Otherwise it is refused, loudly (`trunk_checkout`); never run on another branch.
        let on_trunk = trunk_checkout(&self.dir, &mb).and_then(|(trunk, tip)| match wtgit::is_ancestor(&self.dir, &sha, &tip)? {
            true => Ok(()),
            false => Err(format!("the trunk `{trunk}` (at {}) does not hold commit {}", short(&tip), short(&sha))),
        });
        if let Err(m) = on_trunk {
            return Err(self.end(MERGED_CHECK_FAILED, &loud("the check was refused before it started".into(), &format!(": {m}")), EXIT_MERGED_BAD));
        }
        if let Some(code) = tools::guard_refusal(self.cfg, &project, None) {
            return Err(self.end(MERGED_CHECK_FAILED, &loud(format!("the check was refused before it started, exit {code}"), ""), EXIT_MERGED_BAD));
        }
        println!("queue entry {}: check {project} (the main checkout, on its trunk `{mb}`)", self.e.id());
        Ok(())
    }

    pub(super) fn judge_main_check(&mut self, code: u8, info: JobInfo) -> R<()> {
        let (project, sha, mb) = (self.e.get("project"), self.e.get("main_sha"), self.e.get("main_branch"));
        let loud = |code: String, extra: &str| {
            format!("commit {sha} is ALREADY on `{mb}` of project `{project}`, and `check {project}` there did not pass ({code}){extra}. Main was NOT reset or reverted. A human decides: fix forward, or `git revert` the commit. The task was NOT marked done and its worktree is kept")
        };
        if code != 0 {
            let extra = if info.summary.is_empty() { String::new() } else { format!(": {}", info.summary.join("; ")) };
            let job_id = info.id.clone().unwrap_or_else(|| "none".into());
            if !self.cfg.queue_auto_revert {
                return Err(self.end(MERGED_CHECK_FAILED, &loud(format!("exit {code}, job {job_id}"), &extra), EXIT_MERGED_BAD));
            }
            let failed = format!("commit {sha} is on `{mb}` of project `{project}`, and `check {project}` there did not pass (exit {code}, job {job_id}){extra}.");
            let reason = self.auto_revert(&failed, &job_id);
            return Err(self.end(MERGED_CHECK_FAILED, &reason, EXIT_MERGED_BAD));
        }
        self.log("main_check", &format!("check on main passed ({})", info.summary.join("; ")))
    }

    /// AUTO-REVERT (queue_auto_revert = "on"), after a main check that ran and failed: undo exactly the commits this entry (or the
    /// merge train it ended) fast-forwarded, `pre_merge_sha..main_sha`, with ONE new revert commit on the trunk, then check main
    /// again. Never a reset, never a force, never a push. Refused (nothing is touched) when no pre-merge commit is recorded, when the
    /// main checkout is not on the trunk, when the trunk moved since the merge, or when the checkout has changes. Any refusal, a revert
    /// or commit that fails (aborted with `git revert --abort`), or a re-check that fails HOLDS the lane (`write_hold`). Returns the
    /// entry's reason text; `failed` says what failed first.
    fn auto_revert(&mut self, failed: &str, job_id: &str) -> String {
        let project = self.e.get("project").to_string();
        let held = |cfg: &Config, id: &str, why: &str| -> String {
            let note = match write_hold(cfg, &project, id, why) {
                Ok(p) => format!("Lane `{project}` is HELD ({}): `queue run` merges nothing for it until a check of its main passes", p.display()),
                Err(m) => format!("the lane hold could NOT be written ({m})"),
            };
            format!("{failed} The automatic revert did not complete: {why}. {note}. Main was NOT reset. A human decides: fix forward, or `git revert` the commits. The task was NOT marked done and its worktree is kept")
        };
        let id = self.e.id().to_string();
        match self.revert_merged(job_id) {
            Err(why) => held(self.cfg, &id, &why),
            Ok((range, rsha)) => {
                let _ = self.log("main_check", &format!("main check failed; reverted {range} as {rsha}; checking main again"));
                println!("queue entry {id}: main check failed; reverted {range} as {rsha}; check {project} again");
                match main_check_now(self.cfg, &self.dir, &project) {
                    Ok(job) => {
                        // the one mark `green_revert` trusts: main is green again, so `queue run --all` may go on
                        self.e.set("revert_check", "passed");
                        format!("{failed} The merged commits {range} were reverted as {rsha} (one new commit; nothing was reset); main check after the revert passed (job {job}). The task was NOT marked done and its worktree is kept: fix it and queue it again")
                    }
                    Err(m) => held(self.cfg, &id, &format!("the merged commits {range} were reverted as {rsha}, but the main check after the revert did not pass: {m}")),
                }
            }
        }
    }

    /// The revert itself (see `auto_revert`): `Ok((range, revert commit))`, or why nothing (or nothing left) was reverted.
    fn revert_merged(&mut self, job_id: &str) -> Result<(String, String), String> {
        let (dir, mb) = (self.dir.clone(), self.e.get("main_branch").to_string());
        let pre = self.e.get("pre_merge_sha").to_string();
        let last = self.e.get("main_sha").to_string();
        if pre.is_empty() || last.is_empty() {
            // NEVER derived (from the base or anything else): only the queue's own merge step records what it merged
            return Err(format!("no pre-merge commit recorded; not reverting (pre_merge_sha `{pre}`, main_sha `{last}`: the queue's merge step did not record what this entry merged, e.g. main was fast-forwarded by hand)"));
        }
        let (_, tip) = trunk_checkout(&dir, &mb).map_err(|m| format!("{m}; nothing was reverted"))?;
        if tip != last {
            return Err(format!("the trunk `{mb}` moved since the merge: it was {last} after the merge, it is {tip} now (someone else committed); nothing was reverted"));
        }
        if pre == last || !wtgit::is_ancestor(&dir, &pre, &last)? {
            return Err(format!("the recorded pre-merge commit {pre} is not before the merged commit {last}; nothing was reverted"));
        }
        let dirty = wtgit::status_porcelain_plain(&dir)?;
        if !dirty.is_empty() {
            return Err(format!("the main checkout has uncommitted or untracked changes ({}); nothing was reverted", some(&dirty)));
        }
        let range = format!("{pre}..{last}");
        let commits = wtgit::commit_subjects(&dir, &range)?;
        if commits.is_empty() {
            return Err(format!("git lists no commit in {range}; nothing was reverted"));
        }
        let mut msg: String = commits.iter().map(|(_, s)| format!("Revert \"{s}\"\n")).collect();
        msg.push('\n');
        msg.extend(commits.iter().map(|(c, _)| format!("This reverts commit {c}.\n")));
        msg.push_str(&format!("\nReverted-by: broadsword queue {} (main check failed, job {job_id})\n", self.e.id()));
        std::fs::create_dir_all(&self.cfg.scratch_dir).map_err(|e| format!("cannot create {}: {e}; nothing was reverted", self.cfg.scratch_dir.display()))?;
        let msg_file = self.cfg.scratch_dir.join(format!("queue-revert-{}.msg", record::sanitize(self.e.id())));
        std::fs::write(&msg_file, &msg).map_err(|e| format!("cannot write the revert message {}: {e}; nothing was reverted", msg_file.display()))?;
        // the last look before the revert: still the trunk checked out, still at the merged commit
        let (_, now) = trunk_checkout(&dir, &mb).map_err(|m| format!("{m}; nothing was reverted"))?;
        if now != last {
            return Err(format!("the trunk `{mb}` moved just before the revert: it was {last}, it is {now} now; nothing was reverted"));
        }
        let done = wtgit::revert_no_commit(&dir, &range).and_then(|_| wtgit::commit_file(&dir, &msg_file));
        let _ = std::fs::remove_file(&msg_file);
        if let Err(m) = done {
            let aborted = match wtgit::revert_in_progress(&dir) {
                Ok(true) => match wtgit::revert_abort(&dir) {
                    Ok(()) => "the revert was aborted (`git revert --abort`), main is as it was after the merge".to_string(),
                    Err(e) => format!("AND `git revert --abort` failed: {e}; look at the main checkout"),
                },
                Ok(false) => "no revert was left in progress".to_string(),
                Err(e) => format!("whether a revert is left in progress could not be read: {e}; look at the main checkout"),
            };
            return Err(format!("`git revert --no-commit {range}` and its commit failed: {m}; {aborted}"));
        }
        // the revert commit must be exactly ONE new commit on the merged tip, on the trunk that is still checked out; anything else
        // (the trunk moved or was switched during the revert) holds the lane with both shas, and no further git action is taken
        let (_, after) = trunk_checkout(&dir, &mb).map_err(|m| format!("after the revert commit: {m}; expected the trunk at one new commit on {last}; look at the project"))?;
        if after == last || !wtgit::is_ancestor(&dir, &last, &after)? || wtgit::rev_count(&dir, &format!("{last}..{after}"))? != 1 {
            return Err(format!("after the revert the trunk `{mb}` is at {after}, whose parent is not the merged tip {last} checked before the revert; look at the project"));
        }
        self.e.set("revert_sha", after.clone());
        let _ = self.save();
        Ok((range, after))
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

/// TRUNK RULE of the merge queue, part 1 (the ONE place every step asks; part 2 is `trunk_checkout`): the project's trunk
/// (`wtgit::trunk_branch`: the one of `main`/`master` that exists) and the commit at `refs/heads/<trunk>`. `recorded` is the trunk
/// the entry started on (`main_branch`; empty before it is recorded): a different trunk now is refused. Every "is it on main" test
/// asks this, never HEAD. Any error (both or neither trunk, git failing) is returned and the caller fails closed.
fn trunk_tip(dir: &Path, recorded: &str) -> Result<(String, String), String> {
    let (trunk, tip) = wtgit::trunk_commit(dir)?;
    if !recorded.is_empty() && recorded != trunk {
        return Err(format!("the project's trunk is `{trunk}` now, but this entry started on `{recorded}`"));
    }
    Ok((trunk, tip))
}

/// TRUNK RULE of the merge queue, part 2: `trunk_tip`, and the project's main checkout has exactly that trunk checked out.
/// DECISION (t-20261120-134432): a checkout on any other branch (or a detached HEAD) is REFUSED, never worked around. The queue
/// moves the trunk only with `git merge --ff-only` in that checkout (which fast-forwards whatever is checked out) and runs the main
/// check on that checkout's files, so both are only right while the trunk is what is checked out. It never updates a branch ref
/// behind a checkout's back and never moves, merges into or switches the checked-out branch. Called before the rebase (its base),
/// before every fast-forward, right after it, and before the main check.
pub(super) fn trunk_checkout(dir: &Path, recorded: &str) -> Result<(String, String), String> {
    let (trunk, tip) = trunk_tip(dir, recorded)?;
    match wtgit::current_branch(dir)? {
        Some(b) if b == trunk => Ok((trunk, tip)),
        other => {
            let on = other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into());
            Err(format!("the project's main checkout ({}) has {on} checked out, not its trunk `{trunk}`. The merge queue merges only into the trunk, by fast-forwarding it in that checkout, and never moves or merges into any other branch; the checkout was not touched. Check out `{trunk}` there (`git switch {trunk}`), then run the entry again", dir.display()))
        }
    }
}

/// The trunk's name for a message (`<the trunk>` when it cannot be told).
pub(super) fn trunk_name(dir: &Path) -> String {
    wtgit::trunk_branch(dir).unwrap_or_else(|_| "<the trunk>".into())
}

/// Fast-forward main to `tip` (the last of `members`, a chain whose tip passed its check), run ONE main check, mark every member
/// merged. Returns 0, EXIT_ERROR (internal), or EXIT_MERGED_BAD (the main check failed after the fast-forward: loud, main not reset).
#[allow(clippy::too_many_arguments)]
pub(super) fn merge_members(cfg: &Config, train_id: &str, members: Vec<(Run<'_>, std::time::Instant)>, tip: &str, head: &str, pass: &PassInfo, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    let settled = super::postcheck::settle(cfg, rows, worst);
    let n = members.len();
    if n == 0 {
        return settled;
    }
    let dir = members[0].0.dir.clone();
    let trunk = members[0].0.e.get("main_branch").to_string();
    // the trunk, checked out in the main checkout, is what is fast-forwarded (`trunk_checkout`); anything else merges nothing
    let now = match trunk_checkout(&dir, &trunk) {
        Ok((_, h)) => h,
        Err(m) => {
            for (mut r, t) in members {
                let c = r.leave_train(BLOCKED, &format!("{m}; merge train {train_id} merged nothing"), EXIT_RETRY);
                finish_member(cfg, r, code_num(c), t, rows, worst);
            }
            return settled;
        }
    };
    if now != head {
        for (mut r, t) in members {
            let id = r.e.id().to_string();
            let c = r.leave_train(STALE, &format!("main moved from {} to {} while merge train {train_id} was being checked; nothing was merged. Run `queue run --entry {id}` to rebase again", short(head), short(&now)), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return settled;
    }
    let dirty = match wtgit::status_porcelain_plain(&dir) {
        Ok(d) => d,
        Err(m) => return super::postcheck::barrier_error(cfg, members, &m, rows, worst, settled),
    };
    if !dirty.is_empty() {
        for (mut r, t) in members {
            let c = r.leave_train(BLOCKED, &format!("the main checkout got uncommitted or untracked changes while merge train {train_id} was checked ({}); nothing merged", some(&dirty)), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return settled;
    }
    // BEFORE the fast-forward: every member records step `merge`, so a drain killed from here on leaves each one a half-finished
    // merge (`half_merged`) that the next run resumes at its step, never one that is chained into a new train while its commits may
    // already be on main. A member whose record cannot be written stops the train before main moves.
    if let Some(why) = lane_hold(cfg, members[0].0.e.get("project")) {
        for (mut r, t) in members {
            let c = r.leave_train(BLOCKED, &format!("lane {} held: {why}; merge train {train_id} merged nothing. Clear with a passing check of main, then queue run", r.e.get("project")), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return settled;
    }
    if settled != 0 {
        return super::postcheck::stop_members(cfg, members, rows, worst, settled);
    }
    let mut members = members;
    for (r, _) in members.iter_mut() {
        r.e.set("step", "merge");
        // AUTO-REVERT: the trunk tip before the train's fast-forward; `pre_merge_sha..<train tip>` is exactly what the train merged
        r.e.set("pre_merge_sha", head);
        if r.save().is_err() {
            return fail_members(cfg, members, &format!("could not record the merge step of merge train {train_id} before its fast-forward; main was not moved"), rows, worst);
        }
    }
    if let Err(m) = wtgit::merge_ff_only(&dir, tip) {
        let after = trunk_tip(&dir, "").map(|(_, t)| t).unwrap_or_default();
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
    let after = match trunk_checkout(&dir, &trunk) {
        Ok((_, h)) => h,
        Err(m) => return fail_members(cfg, members, &m, rows, worst),
    };
    if after != tip {
        let msg = format!("after `git merge --ff-only` the trunk `{trunk}` is at {} but the train tip is {}; it was {} before; look at the project", short(&after), short(tip), short(head));
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
    if super::postcheck::enabled(cfg, members[0].0.e.get("project")) {
        return super::postcheck::start(cfg, train_id, members, tip, rows, worst);
    }
    // The last entry selects and judges the one planned main job; the others are covered by it.
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
        // the members' rows keep their exit 6; only a revert that left main green lets the drain go on (`green_revert`)
        return if code == EXIT_MERGED_BAD && green_revert(cfg, &last_id) { 0 } else { code };
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
