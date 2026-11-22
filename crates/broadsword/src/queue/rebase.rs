use crate::record::Record;
use crate::worktree::{self, JobDirError};
use crate::{git, wtgit};
use std::path::Path;
use super::{BLOCKED, CONFLICT, EXIT_RETRY, R, Run};
use super::train::{effort_for, gating_model, trunk_checkout};
use super::view::{short, some};

impl<'a> Run<'a> {
    /// LEFT-TRAIN RULE for a run outside a train (`s_preconditions`, `s_merge`): an entry whose branch is still chained on other
    /// entries' commits that are not on main is never reviewed or merged on that diff; it ends `blocked` (`leave_train`). Once the
    /// branch no longer carries that commit (the coder rebased, or those entries reached main) the record forgets it and the run goes on.
    pub(super) fn refuse_chained(&mut self, step: &str) -> R<()> {
        let (dir, branch) = (self.dir.clone(), self.branch());
        if self.g(chained_base(&dir, &branch, &self.e))?.is_some() {
            return Err(self.leave_train(BLOCKED, "it was run while its branch still carries those entries' commits; nothing was reviewed or merged", EXIT_RETRY));
        }
        let base = chain_bases_of(&self.e).into_iter().next().unwrap_or_default();
        let had = !self.e.get("left_train_base").is_empty() || !self.e.get("train_onto").is_empty();
        if had {
            self.e.set("left_train_base", "");
            self.e.set("train_onto", "");
            self.log(step, &format!("the branch no longer carries {}, the commit a merge train chained it on that is not on main: it runs as it is", short(&base)))?;
        }
        Ok(())
    }

    // -- step 1: preconditions --------------------------------------------------------------

    /// Find the task's worktree (and prove it is the task's), or end blocked.
    pub(super) fn locate(&mut self) -> R<()> {
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

    pub(super) fn s_preconditions(&mut self, start: usize) -> R<()> {
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
        // the entry's `main branch` is the project's TRUNK (never what HEAD happens to be), and it must be what is checked out
        let recorded = if start == 0 { String::new() } else { self.e.get("main_branch").to_string() };
        let (main_branch, trunk_at) = match trunk_checkout(&dir, &recorded) {
            Ok(t) => t,
            Err(m) => return Err(self.blocked(&format!("{m}; nothing merged"))),
        };
        self.e.set("main_branch", main_branch.clone());
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
            Ok(g) => (g.model, if g.why.is_empty() { String::new() } else { format!(" ({})", g.why) }),
            Err(msg) => return Err(self.blocked(&msg)),
        };
        let ahead = self.g(wtgit::rev_count(&dir, &format!("{trunk_at}..{branch}")))?;
        if ahead == 0 {
            return Err(self.blocked(&format!("nothing to merge: `{branch}` has no commit that `{main_branch}` lacks")));
        }
        self.log("preconditions", &format!("ok: main `{main_branch}` clean, worktree clean, task running, {ahead} commit(s) to merge, reviewer `{model}`{why}, effort `{}`, differs from coder `{coder}`", effort_for(self.cfg, self.e.get("risk"))))
    }

    /// Is the rebase in progress in the task's worktree the chain rebase a killed train left (and so the queue's to abort)? Only when
    /// all hold: `train_link` recorded that it began one onto `train_onto` and never finished it, the branch ref is still at the
    /// recorded `pre_rebase` tip (a rebase moves the ref only when it completes), and the worktree's HEAD is on top of `train_onto`
    /// (where that rebase replays the commits). Anything else is the coder's own rebase. A git error answers no (it is not aborted).
    pub(super) fn own_chain_rebase(&mut self) -> R<bool> {
        let (onto, before) = (self.e.get("train_onto").to_string(), self.e.get("pre_rebase").to_string());
        if onto.is_empty() || before.is_empty() {
            return Ok(false);
        }
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        if self.g(wtgit::branch_tip(&dir, &branch))? != before {
            return Ok(false);
        }
        Ok(wtgit::is_ancestor(&wt, &onto, "HEAD").unwrap_or(false))
    }

    pub(super) fn abort_stale_rebase(&mut self) -> R<()> {
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
    pub(super) fn keep_pre_rebase(&mut self, tip: &str) -> R<String> {
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

    pub(super) fn s_rebase(&mut self) -> R<()> {
        self.set_step("rebase")?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let head = self.on_trunk()?;
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
}

/// The commits a merge train may have chained this entry's branch onto, in this order: `left_train_base` (recorded when it left a
/// train, kept across attempts), `train_onto` (a chain rebase begun onto it and never recorded as finished: a drain killed after the
/// rebase moved the branch and before `train_link` saved it, see its KILL WINDOW note; `train_up` is then still the old base) and
/// `train_up` (the base of its own commits while it rides a train). Empty for an entry no train ever chained.
pub(super) fn chain_bases_of(e: &Record) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in ["left_train_base", "train_onto", "train_up"] {
        let v = e.get(k);
        if !v.is_empty() && !out.iter().any(|x| x == v) {
            out.push(v.to_string());
        }
    }
    out
}

/// LEFT-TRAIN RULE, for every caller: the first of `chain_bases_of(e)` that `branch` is still chained on (`chained_onto`), or `None`.
pub(super) fn chained_base(dir: &Path, branch: &str, e: &Record) -> Result<Option<String>, String> {
    for base in chain_bases_of(e) {
        if chained_onto(dir, branch, &base)? {
            return Ok(Some(base));
        }
    }
    Ok(None)
}

/// LEFT-TRAIN RULE, the test: does `branch` still carry `base` (a commit a merge train chained it onto) while main's HEAD does not?
/// Then the branch holds commits of OTHER entries that are not on main, and a review or merge of it would take their code as this
/// entry's. An empty `base` is never chained; a base that main already holds (those entries merged) is not either.
fn chained_onto(dir: &Path, branch: &str, base: &str) -> Result<bool, String> {
    if base.is_empty() {
        return Ok(false);
    }
    let (_, head) = wtgit::trunk_commit(dir)?;
    let tip = wtgit::branch_tip(dir, branch)?;
    Ok(wtgit::is_ancestor(dir, base, &tip)? && !wtgit::is_ancestor(dir, base, &head)?)
}

/// The reason an entry with a chained branch gets (see `Run::leave_train`), with the coder's way out.
pub(super) fn left_train_text(main: &str, branch: &str, base: &str) -> String {
    format!("left a train: rebase your own commits onto main and queue again. `{branch}` is chained on {}, a commit of other entries of a merge train that is not on `{main}`, so a review or merge of it now would take their code as this entry's. In the task's worktree run `git rebase --onto {main} {base} {branch}`, then run the entry again (`queue run --entry <id>`, or `queue cancel` it and `queue add` the task). If those entries reach `{main}` first, the branch needs nothing and runs as it is", short(base))
}
