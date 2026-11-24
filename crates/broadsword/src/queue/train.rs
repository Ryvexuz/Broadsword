use crate::config::Config;
use crate::job;
use crate::record::Record;
use crate::task::{self, fail, now_secs};
use crate::{sys, wtgit};
use std::process::ExitCode;
use super::check::{PassInfo, TrainCheck};
use super::entry::{half_merged, load_entries, resume_first};
use super::locks::{entries_lock, Lane};
use super::merge::{merge_members, trunk_checkout, trunk_name};
use super::{APPROVED, badness, BLOCKED, CHECK_FAILED, code_num, CONFLICT, EXIT_ERROR, EXIT_MERGED_BAD, EXIT_REJECTED, EXIT_RETRY, PERF_FAILED, QUEUED, R, Row, row_for, Run, RUNNING, STALE};
use super::rebase::{chain_bases_of, chained_base, left_train_text};
use super::view::{short, some};

mod lanes;

impl<'a> Run<'a> {
    /// The claims of this entry's task, as a train compares them. A task that cannot be read claims everything (`*`), so it never
    /// shares a train.
    pub(super) fn load_claims(&mut self) {
        self.claims = match task::load_task(self.cfg, self.e.get("task")) {
            Ok(t) => task::claims_of(&t),
            Err(_) => vec!["*".to_string()],
        };
    }

    /// MERGE TRAIN: the gating review PASSed. The entry stops here as `approved`: nothing is merged, the task stays running, the
    /// worktree is untouched. `run_train` merges it with the other approved entries of its project.
    pub(super) fn approve(&mut self) -> R<ExitCode> {
        self.load_claims();
        let base = self.e.get("base_head").to_string();
        self.e.set("train_up", base);
        self.e.set("status", APPROVED);
        // when it started to wait: the merge_train_wait_minutes rule (`short_train_why`) measures from here
        self.e.set("approved_at", now_secs().to_string());
        self.e.set("reason", "the review PASSed; it waits for a merge train (nothing is merged yet)");
        println!("queue entry {}: approved (the review PASSed); it waits for a merge train", self.e.id());
        self.log("review", "approved: the entry joins the next merge train of its project (claims that do not overlap, at most merge_train_max entries)")?;
        Ok(ExitCode::SUCCESS)
    }

    /// Before an approved entry joins a train: still exactly the reviewed commit, the worktree on its branch and clean, clean main on
    /// the same branch. A chain rebase that a killed drain left half done is aborted first (the branch is then back at its
    /// `pre_rebase` tip); any OTHER rebase in progress is the coder's own and is never aborted: the entry ends `blocked`.
    fn train_ready(&mut self) -> R<()> {
        self.locate()?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        if self.g(wtgit::rebase_in_progress(&wt))? {
            if self.own_chain_rebase()? {
                self.abort_stale_rebase()?;
            } else {
                return Err(self.leave_train(BLOCKED, "a rebase is in progress in the task's worktree that no merge train started (the coder's own, begun after the review); it was left alone. Finish it or abort it (`git rebase --abort`), then run the entry again; nothing merged", EXIT_RETRY));
            }
        }
        // `train_link` rebases whatever the worktree has checked out: it must be the task's own branch (as in `s_preconditions`)
        match self.g(wtgit::current_branch(&wt))? {
            Some(b) if b == branch => {}
            other => {
                let on = other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into());
                return Err(self.leave_train(BLOCKED, &format!("the task's worktree is on {on}, not `{branch}`; it cannot join a merge train (its chain rebase would move what is checked out there); nothing merged. Check out `{branch}` again, then run the entry"), EXIT_RETRY));
            }
        }
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
        if let Err(m) = trunk_checkout(&dir, self.e.get("main_branch")) {
            return Err(self.leave_train(BLOCKED, &format!("{m}; nothing merged"), EXIT_RETRY));
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
        // KILL WINDOW: `train_onto` is SAVED (by the log line below) before the rebase starts and cleared only in the save that
        // records the result. A drain killed after the rebase moved the branch but before that save leaves it set, so the left-train
        // rule (`chained_base`) still sees the branch as chained on `onto`, and `train_ready` knows a rebase in progress is its own.
        self.e.set("train_onto", onto);
        self.log("rebase", &format!("merge train {train_id}: chaining `{branch}` (tip {}) onto {}; its own commits start after {}; the old tip is saved in {r}", short(&tip), short(onto), short(&up)))?;
        match self.g(wtgit::rebase_onto(&wt, onto, &up))? {
            wtgit::Rebase::Done => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if !self.g(wtgit::is_ancestor(&dir, onto, &now))? {
                    return Err(self.internal(&format!("the chain rebase reported success but `{branch}` ({}) does not contain {}; the old tip {} is saved in {r}", short(&now), short(onto), short(&tip))));
                }
                if self.g(wtgit::rev_count(&dir, &format!("{onto}..{now}")))? == 0 {
                    self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                    return Err(self.leave_train(BLOCKED, &format!("after chaining onto the train `{branch}` has no commit left (its changes are already in the train); the old tip {} is saved in {r}", short(&tip)), EXIT_RETRY));
                }
                let now_ids = self.g(wtgit::patch_ids(&dir, &format!("{onto}..{now}")))?;
                if now_ids != reviewed_ids {
                    self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                    self.e.set("rebased_tip", now.clone());
                    self.requeue(&format!("patch changed by the train rebase: merge train {train_id} chained `{branch}` onto the entries before it, and its rebased commits {} are not the same patches (git patch-id --stable) as the reviewed commits {}..{}; the old verdict does not cover them, so the entry goes back to `queued` and is reviewed again (the pre-train tip is saved in {r})", short(&now), short(&base), short(&orig)));
                    return Ok(None);
                }
                if self.e.get("reviewed_orig_tip").is_empty() {
                    self.e.set("reviewed_orig_tip", orig.clone());
                }
                self.e.set("train_up", onto);
                self.e.set("train_onto", "");
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
                self.e.set("train_onto", ""); // the branch is exactly as before: it was never chained onto `onto`
                self.requeue(&format!("merge train {train_id}: chaining `{branch}` onto the entries before it conflicts in: {}. The rebase was aborted and the branch is exactly as before; the entry goes back to `queued` and is reviewed again on its own", files.join(", ")));
                Ok(None)
            }
            wtgit::Rebase::Failed(m) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("{m}; and `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                self.e.set("train_onto", "");
                self.requeue(&format!("merge train {train_id}: `{branch}` could not be chained onto the entries before it: {m}. The branch is unchanged; the entry goes back to `queued`"));
                Ok(None)
            }
        }
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
    pub(super) fn leave_train(&mut self, status: &str, reason: &str, code: u8) -> ExitCode {
        let (dir, branch) = (self.dir.clone(), self.branch());
        let base = match chained_base(&dir, &branch, &self.e) {
            Ok(b) => b,
            // a git error counts as chained, on the first recorded base (an error needs one: no base is never chained)
            Err(_) => Some(chain_bases_of(&self.e).into_iter().next().unwrap_or_default()),
        };
        let Some(base) = base else {
            if status == QUEUED {
                let reason = self.secrets.redact(reason);
                self.e.set("status", QUEUED);
                self.e.set("reason", reason.clone());
                let _ = self.log("train", &format!("back to queued: {reason}"));
                eprintln!("broadsword: queue entry {} goes back to `queued`: {reason}", self.e.id());
                return ExitCode::from(EXIT_RETRY);
            }
            return self.end(status, reason, code);
        };
        let main = if self.e.get("main_branch").is_empty() { trunk_name(&dir) } else { self.e.get("main_branch").to_string() };
        self.e.set("left_train_base", base.clone());
        let text = left_train_text(&main, &branch, &base);
        self.blocked(&format!("{text}. Why it left (it would have been `{status}`): {reason}"))
    }
}

/// What a `check_alone` said about the culprit of a failed train.
enum Alone {
    /// It fails on its own commit too: the entry itself is bad.
    Bad(PassInfo),
    /// It passes on its own commit: the failure only shows when it is combined with the entries before it.
    Fine,
    Unknown(String),
    /// Its own worktree kept the check from a verdict (see `TrainCheck::EntryBlocked`).
    EntryBlocked(String),
}

/// The entries the next train takes out of `items` (one `(project, claims)` per approved entry, oldest first), as indices into
/// `items`. Entries of one project only; an entry whose claims overlap an entry already taken is left out (it waits for a later
/// train); at most `max` entries. A train is ready when it is full, or when `flush` is set (nothing else is coming) and it holds at
/// least one entry. Empty when no project has a ready train.
#[cfg(test)]
fn pick_train(items: &[(String, Vec<String>)], max: usize, flush: bool) -> Vec<usize> {
    pick_train_when(items, max, &|_| flush)
}

/// `pick_train` with readiness per project: a project's train is ready when it is full, or when `ready(project)` and it holds at
/// least one entry (the flush train of the drain's end is `ready` for every project; `short_train_projects` says which projects get
/// a short train before the end). Projects are tried in queue order of their oldest approved entry.
fn pick_train_when(items: &[(String, Vec<String>)], max: usize, ready: &dyn Fn(&str) -> bool) -> Vec<usize> {
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
        if group.len() >= max || (!group.is_empty() && ready(project)) {
            return group;
        }
    }
    Vec::new()
}

/// Take the next train out of `approved` (see `pick_train_when`), in queue order; the others stay.
fn take_group<'a>(approved: &mut Vec<(Run<'a>, std::time::Instant)>, max: usize, ready: &dyn Fn(&str) -> bool) -> Option<Vec<(Run<'a>, std::time::Instant)>> {
    let items: Vec<(String, Vec<String>)> = approved.iter().map(|(r, _)| (r.e.get("project").to_string(), r.claims.clone())).collect();
    let idx = pick_train_when(&items, max, ready);
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
pub(super) fn claim_approved(cfg: &Config, lane: &Lane) -> Result<Vec<Record>, ExitCode> {
    let _l = entries_lock(cfg)?;
    let all: Vec<Record> = load_entries(cfg).map_err(|m| fail(EXIT_ERROR, m))?.into_iter().filter(|x| lane.holds(x)).collect();
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

pub(super) fn train_stop_note(code: u8) -> String {
    if code == EXIT_MERGED_BAD {
        "stopped by a merge train (exit 6): main holds the train's commits and the main check failed; a human decides before anything else merges".into()
    } else {
        "stopped by a merge train (exit 1): an internal error or a check that could not run to a verdict; the entries it was working on are `approved` (or `running`), and the next `queue run --all` takes them again".into()
    }
}

/// One entry of a train is finished (merged, rejected, back to queued, ...): its summary row, and the train's worst outcome so far.
pub(super) fn finish_member(cfg: &Config, r: Run<'_>, code: u8, t: std::time::Instant, rows: &mut Vec<Row>, worst: &mut u8) {
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

/// A train check gave no verdict because of ONE entry's worktree (`TrainCheck::EntryBlocked`): that entry leaves the train and ends
/// `blocked` with the reason (the left-train rule decides the exact text, `leave_train`), as the single-entry path blocks only its own
/// entry. The `others` merged nothing and stay `approved`: the next drain takes them again without it. The drain is NOT stopped.
fn entry_leaves_blocked(cfg: &Config, train_id: &str, member: (Run<'_>, std::time::Instant), msg: &str, others: Vec<(Run<'_>, std::time::Instant)>, rows: &mut Vec<Row>, worst: &mut u8) {
    let (mut r, t) = member;
    let id = r.e.id().to_string();
    println!("merge train {train_id}: entry {id} cannot be checked ({msg}); it leaves the train `blocked`, the other {} entr{} stay approved for the next drain", others.len(), if others.len() == 1 { "y" } else { "ies" });
    let c = r.leave_train(BLOCKED, &format!("merge train {train_id} could not check this entry: {msg}. Put the task's worktree back on its branch at the reviewed commit with nothing else in it, then run the entry again; nothing of it was merged"), EXIT_RETRY);
    finish_member(cfg, r, code_num(c), t, rows, worst);
    leave_approved(cfg, others, &format!("entry {id} left merge train {train_id} (its worktree was not exactly the commit to check); nothing merged, this entry waits for the next drain"), rows, worst);
}

/// An internal error that hits every member (git or the record could not be used): each stays as it is, with the message.
pub(super) fn fail_members(cfg: &Config, members: Vec<(Run<'_>, std::time::Instant)>, msg: &str, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    for (mut r, t) in members {
        let c = r.internal(msg);
        finish_member(cfg, r, code_num(c), t, rows, worst);
    }
    EXIT_ERROR
}

/// Entries still `approved` when the drain ends early (it stopped) or no train took them: a row each; the next drain takes them.
pub(super) fn settle_approved(cfg: &Config, approved: &mut Vec<(Run<'_>, std::time::Instant)>, rows: &mut Vec<Row>) {
    for (r, t) in approved.drain(..) {
        let id = r.e.id().to_string();
        rows.push(row_for(cfg, &id, 0, t));
    }
}

/// An entry this drain may still run (so its project's approved entries may wait for it to fill their train): not taken by this
/// drain yet (`taken`; a taken entry is never run again in the same drain, so waiting for it would be waiting for the end), and
/// queued, running or stale, or a half-finished train merge (`resume_first`): what `next_for_all` can still pick.
fn still_to_run(x: &Record, taken: &[String]) -> bool {
    !taken.iter().any(|t| t == x.id()) && (resume_first(x) || matches!(x.status(), QUEUED | RUNNING | STALE))
}

/// STARVATION RULE, for one project with approved entries waiting: why it gets a train now although the train is not full and the
/// drain has not ended, or None (it keeps waiting for more of its entries). `approved_at` holds each of its approved entries'
/// approval time (unix seconds; None when the entry has none that can be read). A train is due when
///   (a) `pending` is false: no entry of the project is left for this drain to run, so nothing more can join its train; or
///   (b) its oldest approved entry has waited `wait_minutes` (merge_train_wait_minutes) by `now`.
/// An entry with no readable approval time (approved before this rule recorded one) counts as having waited long enough: a short
/// train only merges earlier, through the same chain, check, merge and main check as the flush train of the drain's end. A clock
/// that went back counts as no wait.
fn short_train_why(pending: bool, approved_at: &[Option<u64>], now: u64, wait_minutes: u64) -> Option<String> {
    if approved_at.is_empty() {
        return None;
    }
    if !pending {
        return Some("no other entry of its project is left to run".into());
    }
    if approved_at.iter().any(|a| a.is_none()) {
        return Some("an approved entry has no readable approval time (`approved_at`), so it counts as having waited merge_train_wait_minutes".into());
    }
    let oldest = approved_at.iter().flatten().min().copied().unwrap_or(now);
    let waited = now.saturating_sub(oldest);
    (waited >= wait_minutes.saturating_mul(60)).then(|| format!("its oldest approved entry has waited {} s (merge_train_wait_minutes = {wait_minutes})", waited))
}

/// The projects that get a short train now (`short_train_why`), each with the reason, in the order of `approved` (project, approval
/// time). `all` is every queue entry (to see what is still to run), `taken` the entries this drain took.
fn short_train_projects_in(all: &[Record], approved: &[(String, Option<u64>)], taken: &[String], now: u64, wait_minutes: u64) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (project, _) in approved {
        if out.iter().any(|(p, _)| p == project) {
            continue;
        }
        let times: Vec<Option<u64>> = approved.iter().filter(|(p, _)| p == project).map(|(_, a)| *a).collect();
        let pending = all.iter().any(|x| x.get("project") == project && still_to_run(x, taken));
        if let Some(why) = short_train_why(pending, &times, now, wait_minutes) {
            out.push((project.clone(), why));
        }
    }
    out
}

/// `short_train_projects_in` on the queue as it is now (read under entries.lock) and the clock.
fn short_train_projects(cfg: &Config, approved: &[(Run<'_>, std::time::Instant)], taken: &[String]) -> Result<Vec<(String, String)>, String> {
    let all = {
        let _l = entries_lock(cfg).map_err(|c| format!("the queue's entries lock could not be taken (exit {})", code_num(c)))?;
        load_entries(cfg)?
    };
    let items: Vec<(String, Option<u64>)> = approved.iter().map(|(r, _)| (r.e.get("project").to_string(), r.e.get_num("approved_at"))).collect();
    Ok(short_train_projects_in(&all, &items, taken, now_secs(), cfg.merge_train_wait_minutes))
}

/// Run trains while `approved` holds a ready one. `flush` (the drain's end): every project is ready. Otherwise a project's train is
/// ready when it is full, or when the starvation rule says so (`short_train_projects`: none of its entries is left to run, or its
/// oldest approved entry waited merge_train_wait_minutes). A short train is exactly a flush train of that project. `taken`: the
/// entries this drain took. Returns 0, or EXIT_ERROR / EXIT_MERGED_BAD when the drain must stop.
pub(super) fn run_trains<'a>(cfg: &'a Config, approved: &mut Vec<(Run<'a>, std::time::Instant)>, flush: bool, taken: &[String], rows: &mut Vec<Row>) -> u8 {
    let max = cfg.merge_train_max as usize;
    let mut hard = 0u8;
    while hard == 0 {
        let short = if flush {
            Vec::new()
        } else {
            match short_train_projects(cfg, approved, taken) {
                Ok(s) => s,
                Err(m) => {
                    // fail closed: no train is started on a queue that could not be read; the entries stay approved
                    eprintln!("broadsword: queue run --all: cannot tell which merge trains are due: {m}");
                    return EXIT_ERROR;
                }
            }
        };
        let ready = |p: &str| flush || short.iter().any(|(s, _)| s == p);
        let order = if cfg.merge_train_lanes > 1 {
            lanes::queue_order(approved)
        } else {
            Vec::new()
        };
        let Some(group) = take_group(approved, max, &ready) else { break };
        if !flush && group.len() < max {
            let project = group[0].0.e.get("project").to_string();
            let why = short.iter().find(|(s, _)| *s == project).map(|(_, w)| w.as_str()).unwrap_or("-");
            println!("merge train for project {project}: {} of at most {max} entries, not waiting for more: {why}", group.len());
        }
        let count = lanes::lane_count(cfg, group[0].0.e.get("project"));
        hard = if count > 1 {
            let batch = lanes::take_more(group, approved, max, count, order);
            lanes::run_lanes(cfg, batch, approved, rows)
        } else {
            run_train(cfg, group, rows)
        };
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
///    drain stops. A check that cannot run to a verdict leaves all its entries `approved` and stops the drain, unless the reason is
///    one entry's own worktree (`TrainCheck::EntryBlocked`): then only that entry ends `blocked`, the rest stay `approved` for the
///    next drain, and this drain goes on.
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
        // the chain's base is the trunk's tip, and the trunk must be what the main checkout has checked out (`trunk_checkout`)
        let head = match trunk_checkout(&dir, todo[0].0.e.get("main_branch")) {
            Ok((_, h)) => h,
            Err(m) => {
                for (mut r, t) in std::mem::take(&mut todo) {
                    let c = r.leave_train(BLOCKED, &format!("{m}; nothing merged"), EXIT_RETRY);
                    finish_member(cfg, r, code_num(c), t, rows, &mut worst);
                }
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
        // a check that gave no verdict because of ONE entry's worktree: that entry's 1-based position in the chain, and why
        let mut entry_blocked: Option<(usize, String)> = None;
        let mut bisect_checks = 0u32;
        let mut bisect_secs = 0f64;
        let tip = chain_tip(&chain, m);
        let first = chain[m - 1].0.check_commit(&tip, &format!("merge train {train_id}: all {m} entries combined"));
        checks += 1;
        match first {
            TrainCheck::Pass { job, summary, tail } => {
                lo = m;
                good = PassInfo { job, summary, tail, perf: String::new() };
            }
            TrainCheck::Fail { job, summary, tail, perf } => bad = PassInfo { job, summary, tail, perf },
            TrainCheck::Blocked(msg) => blocked = Some(msg),
            TrainCheck::EntryBlocked(msg) => entry_blocked = Some((m, msg)),
        }
        while blocked.is_none() && entry_blocked.is_none() && hi - lo > 1 {
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
                    good = PassInfo { job, summary, tail, perf: String::new() };
                }
                TrainCheck::Fail { job, summary, tail, perf } => {
                    hi = mid;
                    bad = PassInfo { job, summary, tail, perf };
                }
                TrainCheck::Blocked(msg) => blocked = Some(msg),
                TrainCheck::EntryBlocked(msg) => entry_blocked = Some((mid, msg)),
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
        if let Some((pos, msg)) = entry_blocked {
            // ONE entry's worktree is the reason (see `TrainCheck::EntryBlocked`): only it leaves, the rest wait, the drain goes on
            let member = chain.remove(pos - 1);
            entry_leaves_blocked(cfg, &train_id, member, &msg, chain, rows, &mut worst);
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
                TrainCheck::Fail { job, summary, tail, perf } => Alone::Bad(PassInfo { job, summary, tail, perf }),
                TrainCheck::Pass { job, summary, .. } => {
                    let _ = culprit.log("train", &format!("checked on its own commit: passes (job {job}: {summary}); so the failure only shows when it is combined with the entries before it"));
                    Alone::Fine
                }
                TrainCheck::Blocked(msg) => Alone::Unknown(msg),
                TrainCheck::EntryBlocked(msg) => Alone::EntryBlocked(msg),
            }
        };
        let culprit_id = culprit.e.id().to_string();
        match alone {
            Alone::EntryBlocked(msg) => {
                let mut rest = chain;
                rest.extend(after);
                entry_leaves_blocked(cfg, &train_id, (culprit, ct), &msg, rest, rows, &mut worst);
                break;
            }
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
                let failed = if info.perf.is_empty() { CHECK_FAILED } else { PERF_FAILED };
                println!("merge train {train_id}: entry {culprit_id} fails on its own (job {}): {failed}; the {} entr{} before it merge", info.job, lo, if lo == 1 { "y" } else { "ies" });
                culprit.e.set("check_job", info.job.clone());
                culprit.e.set("check_exit", if info.perf.is_empty() { "1" } else { "0" });
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
                let job_shown = if info.job.is_empty() { "?" } else { info.job.as_str() };
                let why = if info.perf.is_empty() {
                    format!("build or tests failed on this entry on its own (job {job_shown}): {}", info.summary.replace('\n', "; "))
                } else {
                    format!("the build and tests passed on this entry on its own (job {job_shown}) but the perf gate failed: {}", info.perf)
                };
                let c = culprit.leave_train(failed, &format!("{why}. Found by the bisect of merge train {train_id}; nothing of it was merged. Fix it, commit, and queue the task again with `queue add`"), EXIT_REJECTED);
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
                    let perf_note = if bad.perf.is_empty() { String::new() } else { format!(" (it was the perf gate that failed: {})", bad.perf) };
                    let reason = format!("the combined check of merge train {train_id} failed (job {}){perf_note} although each of these entries passes on its own: {}. They only fail together, so none of them was merged (main is unchanged). The entry's check summary and tail are the combined failure's. Rebase or fix them yourself, commit, and queue them again with `queue add`", if bad.job.is_empty() { "?" } else { bad.job.as_str() }, ids.join(", "));
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use super::super::{MERGED, REJECTED};

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
    fn a_train_uses_key_claim_overlap_without_a_separate_rule() {
        for file in ["broadsword.toml", "crates/broadsword/src/config.rs"] {
            let alpha = format!("{file}#alpha");
            let beta = format!("{file}#beta");
            let items = vec![item("p", &[&alpha]), item("p", &[&beta]), item("p", &[&alpha]), item("p", &[file]), item("p", &["*"])];
            assert_eq!(pick_train(&items, 2, false), vec![0, 1], "different keys share a full train");
            assert_eq!(pick_train(&items, 3, true), vec![0, 1], "same key, whole file and wildcard stay out");
            let whole_first = vec![item("p", &[file]), item("p", &[&alpha]), item("p", &[&beta])];
            assert_eq!(pick_train(&whole_first, 3, true), vec![0], "a whole-file claim rides alone");
        }
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

    fn entry(id: &str, project: &str, status: &str) -> Record {
        Record::parse(Path::new(&format!("{id}.entry")), &format!("id = {id}\nproject = {project}\nstatus = {status}\n"))
    }

    #[test]
    fn a_project_with_no_entry_left_to_run_gets_a_short_train_while_another_project_still_has_queued_entries() {
        // crash-sim has 2 approved entries and nothing left to run; broadsword has 1 approved and more queued (the 2026-11-20 case)
        let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c2", "crash-sim", APPROVED), entry("q-b1", "broadsword", APPROVED), entry("q-b2", "broadsword", QUEUED)];
        let approved = vec![("crash-sim".to_string(), Some(1000)), ("crash-sim".to_string(), Some(1010)), ("broadsword".to_string(), Some(1020))];
        let taken: Vec<String> = ["q-c1", "q-c2", "q-b1"].iter().map(|s| s.to_string()).collect();
        let short = short_train_projects_in(&all, &approved, &taken, 1030, 20);
        assert_eq!(short.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["crash-sim"], "{short:?}");
        assert!(short[0].1.contains("no other entry"), "{short:?}");
        // the train that forms is BOTH crash-sim entries (a train of 2 of at most 4), not one, and broadsword's waits
        let items = vec![item("crash-sim", &["a"]), item("crash-sim", &["b"]), item("broadsword", &["c"])];
        let ready = |p: &str| short.iter().any(|(s, _)| s == p);
        assert_eq!(pick_train_when(&items, 4, &ready), vec![0, 1]);
        // what still counts as left to run: queued, stale, running (interrupted) and a half-finished train merge, never a taken entry
        for (status, left) in [(QUEUED, true), (STALE, true), (RUNNING, true), (BLOCKED, false), (REJECTED, false), (MERGED, false), (APPROVED, false)] {
            let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c3", "crash-sim", status)];
            let got = short_train_projects_in(&all, &approved[..1], &["q-c1".to_string()], 1030, 20);
            assert_eq!(got.is_empty(), left, "{status}: {got:?}");
            let got = short_train_projects_in(&all, &approved[..1], &["q-c1".to_string(), "q-c3".to_string()], 1030, 20);
            assert_eq!(got.len(), 1, "{status} but already taken by this drain: not waited for: {got:?}");
        }
        let mut half = entry("q-c3", "crash-sim", APPROVED);
        half.set("step", "main_check");
        assert!(short_train_projects_in(&[entry("q-c1", "crash-sim", APPROVED), half], &approved[..1], &["q-c1".to_string()], 1030, 20).is_empty(), "a half-finished train merge is still to run");
        // another project's queued entry never holds a project back
        assert_eq!(short_train_projects_in(&[entry("q-x", "other", QUEUED)], &approved[..1], &[], 1030, 20).len(), 1);
        // no approved entry: no train
        assert!(short_train_projects_in(&all, &[], &taken, 1030, 20).is_empty());
    }

    #[test]
    fn the_wait_rule_fires_once_the_oldest_approved_entry_has_waited_merge_train_wait_minutes() {
        // entries of the project are still queued, so only the wait rule can start its train; `now` is the injected clock
        let at = [Some(10_000u64), Some(10_500)];
        assert_eq!(short_train_why(true, &at, 10_000 + 20 * 60 - 1, 20), None, "one second short of 20 minutes");
        let why = short_train_why(true, &at, 10_000 + 20 * 60, 20).expect("20 minutes after the OLDEST approval");
        assert!(why.contains("1200 s") && why.contains("merge_train_wait_minutes = 20"), "{why}");
        assert!(short_train_why(true, &at, 10_000 + 5 * 60, 5).is_some(), "the setting is read, not a constant");
        assert!(short_train_why(true, &at, 10_000 + 5 * 60 - 1, 5).is_none());
        assert_eq!(short_train_why(true, &[Some(10_000)], 9_000, 20), None, "a clock that went back is no wait");
        assert!(short_train_why(true, &[Some(10_000), None], 10_001, 20).is_some(), "no readable approval time: counts as waited");
        assert_eq!(short_train_why(true, &[], u64::MAX, 20), None, "nothing approved");
        assert!(short_train_why(true, &[Some(0)], u64::MAX - 1, u64::MAX).is_none(), "a huge setting saturates instead of overflowing");
        // through the projects function and the real decision: queued work of the same project, the wait elapsed
        let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c2", "crash-sim", QUEUED)];
        let approved = vec![("crash-sim".to_string(), Some(10_000))];
        let taken = vec!["q-c1".to_string()];
        assert!(short_train_projects_in(&all, &approved, &taken, 10_000 + 19 * 60, 20).is_empty());
        let got = short_train_projects_in(&all, &approved, &taken, 10_000 + 20 * 60, 20);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].1.contains("has waited"), "{got:?}");
    }
}
