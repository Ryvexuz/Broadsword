use super::*;

// ---- the happy path ---------------------------------------------------------------------

#[test]
fn happy_path_rebases_onto_a_moved_main_checks_reviews_and_merges() {
    let r = Root::new("happy", 30);
    let id = r.started("src");
    let work = r.commit(&r.wt(&id), "src/lib.rs", &lib_with("\npub fn sub(a: i32, b: i32) -> i32 {\n    a - b\n}\n"), "task: add sub");
    r.commit(&r.p(), "NOTES.txt", "main moved on\n", "main: notes");
    let main_before = r.head();
    let entry = r.add(&id, "low", "sonnet");
    assert_eq!(r.status(&entry), "queued");
    let list = r.run(&["queue", "list"]);
    assert!(list.stdout.contains(&entry) && list.stdout.contains("queued") && list.stdout.contains(&id), "{}", list.stdout);

    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "queue run failed:\n{}\n{}", o.stdout, o.stderr);
    assert!(o.stdout.contains(&format!("queue entry {entry}: merged")), "{}", o.stdout);

    // merged: main has the work as a fast-forward (the rebased copy of the commit), no merge commit
    assert_eq!(r.status(&entry), "merged");
    let main_after = r.head();
    assert_ne!(main_after, main_before);
    assert_eq!(git(&r.p(), &["rev-parse", "HEAD~1"]), main_before, "main moved by exactly the task's one commit");
    assert_eq!(git(&r.p(), &["log", "-1", "--format=%s"]), "task: add sub");
    assert!(git(&r.p(), &["rev-list", "--merges", "HEAD"]).is_empty(), "never a merge commit");
    assert!(fs::read_to_string(r.p().join("src/lib.rs")).unwrap().contains("a - b"));
    assert!(git(&r.p(), &["status", "--porcelain"]).is_empty());
    assert_eq!(r.field(&entry, "main commit"), main_after);
    // task done: worktree and branch are gone
    assert_eq!(r.task_status(&id), "done");
    assert!(!r.wt(&id).exists(), "the worktree is removed");
    assert!(!git(&r.p(), &["branch", "--format=%(refname:short)"]).lines().any(|b| b == format!("task/{id}")), "the branch is deleted");
    // the pre-rebase tip is kept, create-only, and the replaced commit stays reachable
    let pre = r.ref_value(&format!("refs/broadsword/queue/{entry}/pre-rebase")).expect("pre-rebase ref");
    assert_eq!(pre, work, "the ref holds the tip the branch had before the rebase");
    assert_eq!(r.field(&entry, "pre-rebase tip"), work);
    assert_ne!(main_after, work, "the rebase really rewrote the commit");
    assert_eq!(git(&r.p(), &["cat-file", "-t", &work]), "commit");
    // the fake reviewer got the diff, the check output and the brief, and was started as documented
    assert_eq!(r.calls(), 1);
    let stdin = fs::read_to_string(r.fake_out.join("stdin.txt")).unwrap();
    assert!(stdin.contains("+pub fn sub(a: i32, b: i32) -> i32") && stdin.contains("diff --git a/src/lib.rs"), "the diff: {stdin}");
    assert!(!stdin.contains("NOTES.txt"), "the diff is main-HEAD...branch, so main's own commit is not in it");
    assert!(stdin.contains("test: ok") && stdin.contains("build: ok"), "the check summary");
    assert!(stdin.contains("test result: ok") || stdin.contains("running 1 test"), "the end of the check output");
    assert!(stdin.contains(&format!("Task {id}: make p better")), "the default brief");
    let args = fs::read_to_string(r.fake_out.join("calls.log")).unwrap();
    for needle in ["-p", "--system-prompt", "independent code reviewer", "--model\u{1f}haiku", "--tools\u{1f}\u{1f}--strict-mcp-config", "--max-turns\u{1f}1", "--output-format\u{1f}json"] {
        assert!(args.contains(needle), "{needle} not in {args:?}");
    }
    let cwd = fs::read_to_string(r.fake_out.join("cwd.txt")).unwrap().replace('\\', "/");
    assert!(cwd.ends_with("/scratch/review-cwd"), "{cwd}");
    assert!(!Path::new(&cwd).join("CLAUDE.md").exists());
    // the review file keeps the raw output, cost and tokens, under state (never scratch)
    let review = fs::read_to_string(r.dir.join("state/queue").join(format!("{entry}.review"))).unwrap();
    assert!(review.contains("VERDICT: PASS") && review.contains("cost_usd: 0.0123") && review.contains("input tokens: 110") && review.contains("output tokens: 5") && review.contains("model: haiku"), "{review}");
    assert_eq!(r.field(&entry, "verdict"), "PASS");
    assert_eq!(r.field(&entry, "review cost (USD)"), "0.0123");
    assert!(r.field(&entry, "review file").starts_with("state/queue/"));
    assert!(r.show(&entry).contains("steps:") && r.show(&entry).contains("fast-forwarded"));
    // an empty queue now
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("queue is empty"), "{}", o.stdout);
}

// ---- rebase conflict ------------------------------------------------------------------------

#[test]
fn a_rebase_conflict_is_aborted_and_the_branch_tip_is_unchanged() {
    let r = Root::new("conflict", 30);
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/lib.rs", &LIB_BASE.replace("a + b", "a + b + 0"), "task: tweak add");
    r.commit(&r.p(), "src/lib.rs", &LIB_BASE.replace("a + b", "b + a"), "main: tweak add differently");
    let main_before = r.head();
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "conflict");
    let reason = r.field(&e, "reason");
    assert!(reason.contains("src/lib.rs") && reason.contains("aborted"), "{reason}");
    assert!(o.stderr.contains(&e), "the entry id is in the message: {}", o.stderr);
    assert_eq!(r.branch_tip(&id), tip, "the branch must end exactly as before");
    assert_eq!(r.head(), main_before);
    let st = git(&r.wt(&id), &["status"]);
    assert!(!st.contains("rebase in progress") && !st.contains("Unmerged"), "{st}");
    assert!(git(&r.wt(&id), &["status", "--porcelain"]).is_empty(), "the worktree is clean again");
    assert_eq!(r.ref_value(&format!("refs/broadsword/queue/{e}/pre-rebase")).as_deref(), Some(tip.as_str()), "the pre-rebase tip is kept");
    assert_eq!(r.calls(), 0);
    assert_eq!(r.task_status(&id), "running");
    // a conflict needs the coder: the entry cannot just be re-run
    let o = r.run(&["queue", "run", "--entry", &e]);
    assert_eq!(o.code, 2, "{}{}", o.stdout, o.stderr);
}

// ---- preconditions -----------------------------------------------------------------------

#[test]
fn a_dirty_main_checkout_blocks_the_entry_and_changes_nothing_then_it_can_be_run_again() {
    let r = Root::new("dirtymain", 30);
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    fs::write(r.p().join("junk.txt"), "uncommitted\n").unwrap();
    let main_before = r.head();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(r.field(&e, "reason").contains("main checkout") && r.field(&e, "reason").contains("junk.txt"), "{}", r.field(&e, "reason"));
    assert!(o.stderr.contains(&e));
    assert_eq!((r.head(), r.branch_tip(&id)), (main_before.clone(), tip), "nothing changed");
    assert!(r.ref_value(&format!("refs/broadsword/queue/{e}/pre-rebase")).is_none());
    assert_eq!(r.calls(), 0);
    // plain `queue run` does not pick a blocked entry, an explicit one is retried
    let o = r.run(&["queue", "run"]);
    assert!(o.stdout.contains("queue is empty"), "{}", o.stdout);
    fs::remove_file(r.p().join("junk.txt")).unwrap();
    let o = r.run(&["queue", "run", "--entry", &e]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged");
    assert_eq!(r.field(&e, "attempt"), "2");
}

// ---- the trunk: the queue merges into `main` or `master`, never into what is checked out ---------------------------------

/// Which branch the project's main checkout has checked out.
fn checked_out(r: &Root) -> String {
    git(&r.p(), &["symbolic-ref", "--short", "HEAD"])
}

fn tip_of(r: &Root, branch: &str) -> String {
    git(&r.p(), &["rev-parse", &format!("refs/heads/{branch}")])
}

fn contains(r: &Root, branch: &str, commit: &str) -> bool {
    git_out(&r.p(), &["merge-base", "--is-ancestor", commit, &format!("refs/heads/{branch}")]).code == 0
}

/// Check out a new branch `feature` in the project with a commit of its own. Returns its tip.
fn feature_checked_out(r: &Root) -> String {
    git(&r.p(), &["switch", "-q", "-c", "feature"]);
    r.commit(&r.p(), "FEATURE.txt", "feature work\n", "feature: own work")
}

/// The entry ended blocked because `feature` is checked out instead of `trunk`, and nothing moved: the checkout, the feature
/// branch, the trunk, the task branch; no reviewer was asked.
fn assert_refused_on_feature(r: &Root, e: &str, trunk: &str, trunk_before: &str, feature_tip: &str, task_tip: &str, task: &str) {
    assert_eq!(r.status(e), "blocked", "reason = {}", r.field(e, "reason"));
    let reason = r.field(e, "reason");
    assert!(reason.contains("`feature`") && reason.contains(&format!("trunk `{trunk}`")), "the reason names HEAD and the trunk: {reason}");
    assert_eq!(checked_out(r), "feature", "the checkout is untouched");
    assert_eq!(tip_of(r, "feature"), feature_tip, "the feature branch is never moved or merged into");
    assert_eq!(tip_of(r, trunk), trunk_before, "nothing merged into the trunk");
    assert_eq!(r.branch_tip(task), task_tip, "the task branch is not rebased onto the feature branch");
    assert!(git(&r.p(), &["status", "--porcelain"]).is_empty());
    assert_eq!(r.calls(), 0, "no review of a diff against the wrong branch");
}

#[test]
fn a_feature_branch_checked_out_in_the_project_is_never_merged_into_and_the_entry_is_refused_until_the_trunk_is_checked_out() {
    let r = Root::new("featco", 30);
    let id = r.started("src");
    let task_tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let main_before = tip_of(&r, "main");
    let feature_tip = feature_checked_out(&r);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_refused_on_feature(&r, &e, "main", &main_before, &feature_tip, &task_tip, &id);
    // with the trunk checked out again it merges into main, and the feature branch stays as it was
    git(&r.p(), &["switch", "-q", "main"]);
    let o = r.run(&["queue", "run", "--entry", &e]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged", "reason = {}", r.field(&e, "reason"));
    assert_eq!(r.field(&e, "main branch"), "main");
    assert_eq!(git(&r.p(), &["log", "-1", "--format=%s", "refs/heads/main"]), "task: new file");
    assert_eq!(r.field(&e, "main commit"), tip_of(&r, "main"));
    assert_eq!(tip_of(&r, "feature"), feature_tip);
    assert!(!contains(&r, "main", &feature_tip), "the feature branch's commit never reaches main");
    assert_eq!(checked_out(&r), "main");
}

#[test]
fn a_merge_train_never_merges_into_a_checked_out_feature_branch() {
    let r = train_root("feattrain", 4);
    let (a, ea) = train_one(&r, "src/a.txt", "x\n");
    let (b, eb) = train_one(&r, "src/b.txt", "y\n");
    let (ta, tb) = (r.branch_tip(&a), r.branch_tip(&b));
    let main_before = tip_of(&r, "main");
    let feature_tip = feature_checked_out(&r);
    let o = r.run(&["queue", "run", "--all"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_refused_on_feature(&r, &ea, "main", &main_before, &feature_tip, &ta, &a);
    assert_refused_on_feature(&r, &eb, "main", &main_before, &feature_tip, &tb, &b);
}

#[test]
fn a_project_whose_trunk_is_master_merges_into_master_and_never_into_a_checked_out_feature_branch() {
    let r = Root::new("master", 30);
    git(&r.p(), &["branch", "-m", "main", "master"]);
    let id = r.started("src");
    let task_tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let master_before = tip_of(&r, "master");
    let feature_tip = feature_checked_out(&r);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_refused_on_feature(&r, &e, "master", &master_before, &feature_tip, &task_tip, &id);
    git(&r.p(), &["switch", "-q", "master"]);
    let o = r.run(&["queue", "run", "--entry", &e]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged", "reason = {}", r.field(&e, "reason"));
    assert_eq!(r.field(&e, "main branch"), "master");
    assert_eq!(git(&r.p(), &["log", "-1", "--format=%s", "refs/heads/master"]), "task: new file");
    assert_eq!(r.field(&e, "main commit"), tip_of(&r, "master"));
    assert!(r.show(&e).contains("fast-forwarded `master`"), "{}", r.show(&e));
    assert_eq!(tip_of(&r, "feature"), feature_tip);
    assert!(git_out(&r.p(), &["rev-parse", "--verify", "-q", "refs/heads/main"]).code != 0, "no `main` was made");
}

#[test]
fn a_project_with_both_main_and_master_blocks_the_entry_with_the_trunk_error_and_merges_nothing() {
    let r = Root::new("bothtrunks", 30);
    let id = r.started("src");
    let task_tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let main_before = tip_of(&r, "main");
    git(&r.p(), &["branch", "master", "main"]);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    let reason = r.field(&e, "reason");
    assert!(reason.contains("both `main` and `master`"), "{reason}");
    assert_eq!((tip_of(&r, "main"), tip_of(&r, "master"), r.branch_tip(&id)), (main_before.clone(), main_before, task_tip));
    assert_eq!(r.calls(), 0);
}

#[test]
fn main_moving_between_the_rebase_and_the_merge_makes_the_entry_stale_and_nothing_is_merged() {
    let r = Root::new("stale", 30);
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    // the fake reviewer commits to main while the review runs
    let o = r.run_env(&["queue", "run"], &[("FAKE_CLAUDE_COMMIT_IN", r.p().to_str().unwrap())]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "stale");
    assert!(r.field(&e, "reason").contains("main moved") && r.field(&e, "reason").contains("nothing was merged"), "{}", r.field(&e, "reason"));
    assert_eq!(git(&r.p(), &["log", "-1", "--format=%s"]), "main moved while the review ran");
    assert!(!r.p().join("src/new.txt").exists(), "nothing merged");
    assert_eq!(r.branch_tip(&id), tip);
    assert_eq!(r.task_status(&id), "running");
    assert!(r.wt(&id).exists());
    // re-run: it rebases again onto the new main (a second pre-rebase ref, the first is never moved), reviews again, merges
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "plain `queue run` picks a stale entry: {}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged");
    assert_eq!(r.ref_value(&format!("refs/broadsword/queue/{e}/pre-rebase-2")).as_deref(), Some(tip.as_str()));
    assert!(r.p().join("src/new.txt").exists() && r.p().join("moved-during-review.txt").exists());
    assert!(git(&r.p(), &["rev-list", "--merges", "HEAD"]).is_empty());
}

// ---- B1: one commit from check to merge ------------------------------------------------------

#[test]
fn a_commit_added_between_check_and_review_is_never_reviewed_or_merged() {
    let r = Root::new("onecommit", 120);
    let id = r.started("src");
    let checked = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    // the run dies in the middle of the review step: the check already passed on `checked`
    let mut child = r.spawn(&["queue", "run"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("the reviewer to start", 120, || r.calls() == 1);
    kill_run_only(&mut child);
    assert_eq!(r.field(&e, "step"), "review");
    assert_eq!(r.field(&e, "checked commit"), checked);
    // a commit that does not even compile lands on the branch before the resume: it was never checked
    let broken = r.commit(&r.wt(&id), "src/lib.rs", &lib_with("\npub fn broken( {\n"), "task: late commit that does not compile");
    fs::remove_file(&hang).unwrap();
    let main_before = r.head();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "stale", "{}{}", o.stdout, o.stderr);
    assert!(r.field(&e, "reason").contains("branch changed after check"), "{}", r.field(&e, "reason"));
    assert_eq!(r.head(), main_before, "nothing merged");
    assert_eq!(r.calls(), 1, "the unchecked commit was never sent to the reviewer");
    assert_eq!(r.branch_tip(&id), broken);
    assert_eq!(r.task_status(&id), "running");
    // run again: the new commit is checked, and the broken build stops it before any reviewer is asked
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "check_failed");
    assert_eq!(r.calls(), 1);
    assert_eq!(r.head(), main_before);
}

#[test]
fn the_merge_refuses_a_tip_that_is_not_the_checked_and_reviewed_commit() {
    let r = Root::new("mergebind", 30);
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    // a recorded PASS whose reviewed commit is not the checked one (a hand-edited or corrupt entry): refused, nothing merged
    let base = git(&r.p(), &["rev-parse", "HEAD"]);
    r.edit_entry(&e, &[("status", "running"), ("step", "merge"), ("attempt", "1"), ("verdict", "PASS"), ("checked_tip", &base), ("reviewed_tip", &tip), ("base_head", &base), ("main_branch", "main")]);
    let main_before = r.head();
    let o = r.run(&["queue", "run"]);
    // ends `stale` (exit 3), never stays `running` at the merge step, where it could be neither cancelled nor skipped
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "stale");
    assert!(o.stderr.contains("without a recorded PASS for the checked commit"), "{}", o.stderr);
    assert_eq!(r.head(), main_before);
    assert_eq!(r.calls(), 0);
}

#[test]
fn an_ignored_file_in_the_worktree_does_not_block_the_queue_and_done_keeps_the_worktree_but_the_entry_is_merged() {
    let r = Root::new("ignored", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    fs::create_dir_all(r.wt(&id).join("target")).unwrap();
    fs::write(r.wt(&id).join("target/x"), "ignored").unwrap();
    fs::write(r.wt(&id).join(".env"), "A=b\n").unwrap();
    fs::write(r.wt(&id).join(".gitignore"), "target\n.env\n").unwrap();
    git(&r.wt(&id), &["add", ".gitignore"]);
    git(&r.wt(&id), &["commit", "-q", "-m", "task: ignore .env"]);
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged");
    assert!(r.field(&e, "reason").contains("KEPT"), "{}", r.field(&e, "reason"));
    assert_eq!(r.task_status(&id), "done");
    assert!(r.wt(&id).exists(), "the worktree with ignored files is kept, never deleted");
    assert!(r.p().join("src/new.txt").exists());
}

// ---- merged_check_failed ----------------------------------------------------------------

#[test]
fn a_check_that_fails_on_main_after_the_merge_is_loud_and_main_is_not_reset() {
    let r = Root::new("mainfail", 30);
    let id = r.started("src");
    // passes in the worktree, fails in the main checkout (which lives under projects/p): the "impossible" case, made on purpose
    let content = LIB_BASE.to_string() + "\n#[cfg(test)]\nmod only_here {\n    #[test]\n    fn not_in_main() {\n        assert!(!env!(\"CARGO_MANIFEST_DIR\").replace('\\\\', \"/\").contains(\"/projects/p\"));\n    }\n}\n";
    let work = r.commit(&r.wt(&id), "src/lib.rs", &content, "task: a test that only passes outside main");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 6, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged_check_failed");
    assert!(o.stderr.contains("LOUD") && o.stderr.contains(&e) && o.stderr.contains("ALREADY on") && o.stderr.contains("NOT reset"), "{}", o.stderr);
    assert_eq!(r.head(), work, "main keeps the commit: it is NOT reset");
    assert!(r.field(&e, "reason").contains(&r.head()), "the loud message names the commit");
    assert_eq!(r.task_status(&id), "running", "the task is not marked done");
    assert!(r.wt(&id).exists());
    assert_eq!(r.run(&["queue", "run", "--entry", &e]).code, 2, "a human decides: it is not run again");
}

/// A worktree stuck in the middle of a rebase, as a dead `queue run` would leave it, and an entry that says it was at the
/// rebase step.
fn stuck_in_a_rebase(r: &Root) -> (String, String, String) {
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/lib.rs", &LIB_BASE.replace("a + b", "a + b + 0"), "task: tweak add");
    let e = r.add(&id, "low", "sonnet");
    let main_head = r.commit(&r.p(), "src/lib.rs", &LIB_BASE.replace("a + b", "b + a"), "main: tweak add differently");
    let pre_ref = format!("refs/broadsword/queue/{e}/pre-rebase");
    git(&r.p(), &["update-ref", &pre_ref, &tip, ""]);
    let o = git_out(&r.wt(&id), &["rebase", &main_head]);
    assert_ne!(o.code, 0, "the rebase must conflict");
    assert!(git(&r.wt(&id), &["status"]).contains("rebase in progress"));
    r.edit_entry(&e, &[("status", "running"), ("step", "rebase"), ("attempt", "1"), ("pre_rebase", &tip), ("pre_rebase_ref", &pre_ref), ("pid", "4000000"), ("main_branch", "main")]);
    (id, e, tip)
}

#[test]
fn resuming_at_the_rebase_step_aborts_the_half_done_rebase_first() {
    let r = Root::new("resumerebase", 30);
    let (id, e, tip) = stuck_in_a_rebase(&r);
    assert!(r.status(&e).contains("interrupted"));
    let o = r.run(&["queue", "run"]);
    // the stale rebase is aborted, the step runs again, and it conflicts for real: the branch is exactly as before
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "conflict");
    assert!(o.stdout.contains("RESUMING"), "{}", o.stdout);
    assert!(r.show(&e).contains("aborted it"), "{}", r.show(&e));
    assert_eq!(r.branch_tip(&id), tip);
    assert!(!git(&r.wt(&id), &["status"]).contains("rebase in progress"));
    assert!(git(&r.wt(&id), &["status", "--porcelain"]).is_empty());
}

#[test]
fn cancelling_an_interrupted_rebase_entry_aborts_the_leftover_rebase_and_touches_nothing_else() {
    let r = Root::new("cancelrebase", 30);
    let (id, e, tip) = stuck_in_a_rebase(&r);
    let main_head = r.head();
    let o = r.run(&["queue", "cancel", &e]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("aborted"), "{}", o.stdout);
    assert_eq!(r.status(&e), "cancelled");
    assert_eq!(r.branch_tip(&id), tip);
    assert_eq!(r.head(), main_head);
    assert!(!git(&r.wt(&id), &["status"]).contains("rebase in progress"));
    assert_eq!(r.task_status(&id), "running");
    // an interrupted entry that reached the merge step cannot be cancelled: main may hold the commit
    let id2 = r.started("docs");
    r.commit(&r.wt(&id2), "docs/a.txt", "x\n", "task: docs");
    let e2 = r.add(&id2, "low", "sonnet");
    r.edit_entry(&e2, &[("status", "running"), ("step", "merge"), ("attempt", "1")]);
    let o = r.run(&["queue", "cancel", &e2]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("merge") && o.stderr.contains("queue run --entry"), "{}", o.stderr);
    assert_eq!(r.status(&e2).split(' ').next().unwrap(), "running");
}

#[test]
fn resuming_at_the_merge_step_records_a_fast_forward_that_already_happened() {
    let r = Root::new("resumemerge", 30);
    let id = r.started("src");
    let tip = r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    // main already holds the reviewed commit (the run died right after `git merge --ff-only`, before recording it)
    git(&r.p(), &["merge", "--ff-only", &tip]);
    r.edit_entry(&e, &[("status", "running"), ("step", "merge"), ("attempt", "1"), ("verdict", "PASS"), ("checked_tip", &tip), ("reviewed_tip", &tip), ("base_head", &git(&r.p(), &["rev-parse", "HEAD~1"])), ("main_branch", "main")]);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged");
    assert_eq!(r.head(), tip);
    assert!(r.show(&e).contains("already is the reviewed commit"));
    assert_eq!(r.task_status(&id), "done");
    assert!(!r.wt(&id).exists());
    assert_eq!(r.calls(), 0, "no reviewer on a resume past the review");
}

#[test]
fn a_test_hook_review_never_merges_into_a_project_outside_scratch() {
    let r = Root::with_projects("hookouter", 30, "projects");
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let main_head = r.head();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "blocked");
    assert!(o.stderr.contains("TEST HOOK") && o.stderr.contains("outside scratch_dir"), "{}", o.stderr);
    assert_eq!(r.field(&e, "test hook"), "true");
    assert_eq!(r.head(), main_head, "nothing merged");
    assert!(!r.p().join("src/new.txt").exists());
    assert_eq!(r.task_status(&id), "running");
}

#[test]
fn the_merge_prints_and_records_the_reviewer_cli_path() {
    let r = Root::new("mergecli", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("merging after a TEST HOOK review through reviewer CLI") && o.stdout.contains("fake_claude"), "{}", o.stdout);
    let shown = r.show(&e);
    assert!(shown.contains("reviewed through CLI") && shown.contains("fake_claude"), "{shown}");
}
