use super::*;

// ---- check failure: no model is asked ---------------------------------------------------

#[test]
fn a_failing_check_stops_before_the_reviewer() {
    let r = Root::new("checkfail", 30);
    let id = r.started("src");
    let broken = LIB_BASE.replace("assert_eq!(super::add(1, 2), 3)", "assert_eq!(super::add(1, 2), 4)");
    let tip = r.commit(&r.wt(&id), "src/lib.rs", &broken, "task: break a test");
    let main_before = r.head();
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "check_failed");
    assert!(r.field(&e, "reason").contains("test: FAILED"), "{}", r.field(&e, "reason"));
    assert!(r.show(&e).contains("check summary:") && r.show(&e).contains("test: FAILED"));
    assert_eq!(r.calls(), 0, "no model is asked when the check fails");
    assert!(!r.fake_out.join("calls.log").exists());
    assert_eq!(r.head(), main_before);
    assert_eq!(r.branch_tip(&id), tip);
    assert_eq!(r.task_status(&id), "running");
    assert!(r.wt(&id).exists());
}

// ---- queue_auto_revert: a failed main check is reverted, and a lane is held until main is green ---------------------------

/// `src/lib.rs` with a test that passes in a task worktree and fails in the main checkout (which lives under projects/p).
fn only_outside_main() -> String {
    LIB_BASE.to_string() + "\n#[cfg(test)]\nmod only_here {\n    #[test]\n    fn not_in_main() {\n        assert!(!env!(\"CARGO_MANIFEST_DIR\").replace('\\\\', \"/\").contains(\"/projects/p\"));\n    }\n}\n"
}

/// A root with queue_auto_revert on (the test config ships it off).
fn revert_root(name: &str) -> Root {
    let r = Root::new(name, 30);
    train_set(&r, "queue_auto_revert", "\"on\"");
    r
}

fn hold_file(r: &Root) -> PathBuf {
    r.dir.join("state/queue/hold-p")
}

/// Every file git tracks at `rev`, with its blob id: equal lists mean equal files.
fn tree_of(r: &Root, rev: &str) -> String {
    git(&r.p(), &["ls-tree", "-r", rev])
}

#[test]
fn auto_revert_a_merged_entry_whose_main_check_fails_is_reverted_and_main_is_green_again() {
    let r = revert_root("autorevert");
    let id = r.started("src");
    let pre = r.head();
    let work = r.commit(&r.wt(&id), "src/lib.rs", &only_outside_main(), "task: a test that only passes outside main");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 6, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged_check_failed");
    let reason = r.field(&e, "reason");
    assert!(reason.contains("reverted as") && reason.contains("main check after the revert passed"), "{reason}");
    // the trunk holds the merged commit AND one new revert commit on top of it: history kept, nothing reset
    let head = r.head();
    assert_ne!(head, work);
    assert_eq!(git(&r.p(), &["rev-parse", "HEAD~1"]), work, "the revert is a new commit on the merged one");
    assert!(reason.contains(&head[..12]) || reason.contains(&head), "the entry names the revert commit: {reason}");
    assert_eq!(tree_of(&r, "HEAD"), tree_of(&r, &pre), "the files equal main before the merge");
    let msg = git(&r.p(), &["log", "-1", "--format=%B"]);
    assert!(msg.contains("Revert \"task: a test that only passes outside main\"") && msg.contains(&format!("This reverts commit {work}.")), "{msg}");
    assert!(msg.contains(&format!("Reverted-by: broadsword queue {e} (main check failed, job ")), "{msg}");
    assert!(git(&r.p(), &["status", "--porcelain"]).is_empty());
    assert!(!hold_file(&r).exists(), "a revert whose re-check passed leaves the lane open");
    assert_eq!(r.task_status(&id), "running", "the task is not marked done");
    assert!(r.wt(&id).exists(), "its worktree is kept");
}

#[test]
fn auto_revert_a_train_of_two_is_reverted_as_one_range_in_one_commit() {
    let r = train_root("autorevtrain", 2);
    train_set(&r, "queue_auto_revert", "\"on\"");
    let pre = r.head();
    let (a, ea) = train_one(&r, "NOTES.txt", "notes\n");
    let (b, eb) = train_one(&r, "src/lib.rs", &only_outside_main());
    let o = r.run(&["queue", "run", "--all"]);
    assert_eq!(o.code, 6, "{}{}", o.stdout, o.stderr);
    for e in [&ea, &eb] {
        assert_eq!(r.status(e), "merged_check_failed", "{}", r.show(e));
        assert!(r.field(e, "reason").contains("reverted as"), "{}", r.show(e));
    }
    // pre <- a <- b <- ONE revert of both
    assert_eq!(git(&r.p(), &["rev-list", "--count", &format!("{pre}..HEAD")]), "3", "two merged commits and one revert commit");
    assert_eq!(git(&r.p(), &["rev-parse", "HEAD~1"]), r.branch_tip(&b));
    assert_eq!(git(&r.p(), &["rev-parse", "HEAD~2"]), r.branch_tip(&a));
    assert_eq!(tree_of(&r, "HEAD"), tree_of(&r, &pre), "the files equal main before the train");
    let msg = git(&r.p(), &["log", "-1", "--format=%B"]);
    assert!(msg.contains("Revert \"task: NOTES.txt\"") && msg.contains("Revert \"task: src/lib.rs\"") && msg.contains("Reverted-by: broadsword queue "), "{msg}");
    assert!(!hold_file(&r).exists());
}

#[test]
fn auto_revert_refuses_when_the_trunk_moved_after_the_merge_and_holds_the_lane() {
    let r = revert_root("autorevmoved");
    let id = r.started("src");
    let pre = r.head();
    let tip = r.commit(&r.wt(&id), "src/lib.rs", &only_outside_main(), "task: a test that only passes outside main");
    let e = r.add(&id, "low", "sonnet");
    // the run died right after the merge was recorded; then someone else committed on main
    git(&r.p(), &["merge", "--ff-only", &tip]);
    let other = r.commit(&r.p(), "OTHER.txt", "someone else\n", "main: someone else");
    r.edit_entry(&e, &[("status", "running"), ("step", "main_check"), ("attempt", "1"), ("pid", "4000000"), ("verdict", "PASS"), ("checked_tip", &tip), ("reviewed_tip", &tip), ("base_head", &pre), ("pre_merge_sha", &pre), ("main_sha", &tip), ("main_branch", "main")]);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 6, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged_check_failed");
    assert_eq!(r.head(), other, "nothing was reverted: the trunk moved");
    let reason = r.field(&e, "reason");
    assert!(reason.contains(&tip) && reason.contains(&other) && reason.contains("HELD"), "{reason}");
    let hold = fs::read_to_string(hold_file(&r)).expect("the hold file is written");
    assert!(hold.contains(&e) && hold.contains("moved"), "{hold}");
}

#[test]
fn auto_revert_never_derives_a_pre_merge_commit_an_entry_merged_by_hand_is_not_reverted_and_the_lane_is_held() {
    let r = revert_root("autorevnopre");
    let id = r.started("src");
    let base = r.head();
    let tip = r.commit(&r.wt(&id), "src/lib.rs", &only_outside_main(), "task: a test that only passes outside main");
    let e = r.add(&id, "low", "sonnet");
    // main was fast-forwarded by hand (no queue merge step recorded a pre-merge commit); the entry resumes at its merge step
    git(&r.p(), &["merge", "--ff-only", &tip]);
    r.edit_entry(&e, &[("status", "running"), ("step", "merge"), ("attempt", "1"), ("verdict", "PASS"), ("checked_tip", &tip), ("reviewed_tip", &tip), ("base_head", &base), ("main_branch", "main")]);
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 6, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged_check_failed");
    assert_eq!(r.head(), tip, "nothing was reverted");
    let reason = r.field(&e, "reason");
    assert!(reason.contains("no pre-merge commit recorded; not reverting") && reason.contains("HELD"), "{reason}");
    let hold = fs::read_to_string(hold_file(&r)).expect("the hold file is written");
    assert!(hold.contains(&e) && hold.contains("no pre-merge commit recorded"), "{hold}");
}

#[test]
fn auto_revert_a_held_lane_merges_nothing_and_the_hold_clears_after_a_passing_main_check() {
    let r = revert_root("autorevhold");
    // main itself fails its check, and the lane is held
    r.commit(&r.p(), "src/lib.rs", &only_outside_main(), "main: a test that fails on main");
    fs::create_dir_all(r.dir.join("state/queue")).unwrap();
    fs::write(hold_file(&r), "time = 2026-11-22 00:00:00 UTC\nentry = q-test\nproject = p\nreason = the test held it\n").unwrap();
    let id = r.started("NOTES.txt");
    r.commit(&r.wt(&id), "NOTES.txt", "notes\n", "task: notes");
    let e = r.add(&id, "low", "sonnet");
    let before = r.head();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("lane p held: the test held it; clear with a passing check of main, then queue run"), "{}", o.stdout);
    assert_eq!(r.head(), before, "a held lane merges nothing");
    assert_eq!(r.status(&e), "queued");
    assert!(hold_file(&r).exists());
    // main is fixed: the next run's check of main passes, the hold clears, the entry merges
    r.commit(&r.p(), "src/lib.rs", LIB_BASE, "main: fix");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("hold cleared"), "{}", o.stdout);
    assert!(!hold_file(&r).exists());
    assert_eq!(r.status(&e), "merged");
}

#[test]
fn auto_revert_off_keeps_the_old_message_and_reverts_nothing() {
    let r = revert_root("autorevoff");
    train_set(&r, "queue_auto_revert", "\"off\"");
    let id = r.started("src");
    let work = r.commit(&r.wt(&id), "src/lib.rs", &only_outside_main(), "task: a test that only passes outside main");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 6, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged_check_failed");
    assert!(o.stderr.contains("Main was NOT reset or reverted. A human decides"), "{}", o.stderr);
    assert_eq!(r.head(), work, "nothing reverted");
    assert!(!hold_file(&r).exists());
}

// ---- merge_fast_tests: a fast safety subset before the merge, the full check after it ---------------------------------

/// A test binary that passes in the main checkout (under projects/p) and FAILS in a task worktree: only a FULL check of the
/// worktree runs it, so a pre-merge check that passes proves the subset did not run it.
const ONLY_IN_MAIN: &str = "#[test]\nfn only_in_main() {\n    assert!(env!(\"CARGO_MANIFEST_DIR\").replace('\\\\', \"/\").contains(\"/projects/p\"));\n}\n";

/// A root with auto-revert on, whose project `p` has two test binaries on main: `fast` (always passes) and `other` (`ONLY_IN_MAIN`).
fn fast_root(name: &str) -> Root {
    let r = revert_root(name);
    r.commit(&r.p(), "tests/fast.rs", "#[test]\nfn fast_ok() {\n    assert_eq!(p::add(2, 2), 4);\n}\n", "main: test binary fast");
    r.commit(&r.p(), "tests/other.rs", ONLY_IN_MAIN, "main: test binary other");
    r
}

#[test]
fn fast_subset_a_listed_project_runs_the_subset_before_the_merge_and_the_full_check_after_it() {
    let r = fast_root("fastsub");
    train_set(&r, "merge_fast_tests", "\"p: fast\"");
    let id = r.started("NOTES.txt");
    r.commit(&r.wt(&id), "NOTES.txt", "notes\n", "task: notes");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged", "{}", r.show(&e));
    let show = r.show(&e);
    assert!(show.contains("\n  check: fast subset (fast) = `cargo build --offline && cargo test --offline --lib --test fast`, NOT the full suite"), "the record says a subset ran: {show}");
    assert!(!show.lines().any(|l| l.trim_start().starts_with("check: full")), "{show}");
    // the subset is ONE `run` job of the task's worktree; the main check after the merge is the full `check` (it runs `other`)
    assert!(o.stdout.contains("job: run p ") && o.stdout.contains("cargo build --offline && cargo test --offline --lib --test fast"), "{}", o.stdout);
    let main_check = o.stdout.find("check p (the main checkout").expect("a main check ran");
    assert!(o.stdout[main_check..].contains("job: check p "), "the main check is the full check: {}", o.stdout);
    assert!(show.contains("check on main passed (build: ok"), "{show}");
    // the reviewer was told which check ran, in the summary and in the check tail
    let stdin = fs::read_to_string(r.fake_out.join("stdin.txt")).unwrap();
    assert!(stdin.contains("check: fast subset (fast)") && stdin.matches("check: fast subset (fast)").count() >= 2, "{stdin}");
}

#[test]
fn fast_subset_unit_flags_come_only_from_the_packages_cargo_tests() {
    // p's root package has only a binary; its workspace member `inner` (not a default member) has a library. `cargo test` in p
    // tests the root package only, so `--lib` there is refused ("no library targets"): the flags must be `--bins` alone.
    let r = fast_root("fastws");
    let p = r.p();
    git(&p, &["rm", "-q", "src/lib.rs", "tests/fast.rs", "tests/other.rs"]);
    fs::write(p.join("Cargo.toml"), "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\"inner\"]\n").unwrap();
    fs::write(p.join("Cargo.lock"), "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 3\n\n[[package]]\nname = \"inner\"\nversion = \"0.0.0\"\n\n[[package]]\nname = \"p\"\nversion = \"0.0.0\"\n").unwrap();
    fs::create_dir_all(p.join("inner/src")).unwrap();
    fs::write(p.join("inner/Cargo.toml"), "[package]\nname = \"inner\"\nversion = \"0.0.0\"\nedition = \"2021\"\n").unwrap();
    fs::write(p.join("inner/src/lib.rs"), "pub fn one() -> u32 {\n    1\n}\n").unwrap();
    fs::create_dir_all(p.join("src")).unwrap();
    fs::write(p.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::create_dir_all(p.join("tests")).unwrap();
    fs::write(p.join("tests/fast.rs"), "#[test]\nfn fast_ok() {\n    assert_eq!(2 + 2, 4);\n}\n").unwrap();
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "main: binary-only root package with a library workspace member"]);
    train_set(&r, "merge_fast_tests", "\"p: fast\"");
    let id = r.started("NOTES.txt");
    r.commit(&r.wt(&id), "NOTES.txt", "notes\n", "task: notes");
    let e = r.add(&id, "low", "sonnet");
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged", "{}", r.show(&e));
    assert!(r.show(&e).contains("check: fast subset (fast) = `cargo build --offline && cargo test --offline --bins --test fast`"), "{}", r.show(&e));
}

#[test]
fn fast_subset_an_unlisted_project_runs_the_full_check_before_the_merge() {
    let r = fast_root("fastfull");
    let id = r.started("NOTES.txt");
    r.commit(&r.wt(&id), "NOTES.txt", "notes\n", "task: notes");
    let e = r.add(&id, "low", "sonnet");
    let before = r.head();
    let o = r.run(&["queue", "run"]);
    // the full check runs `other` in the worktree, where it fails: nothing merges and no model is asked
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "check_failed", "{}", r.show(&e));
    assert!(r.show(&e).contains("check summary:\n  build: ok") && r.show(&e).contains("\n  check: full\n"), "{}", r.show(&e));
    assert_eq!(r.head(), before);
    assert_eq!(r.calls(), 0);
}

#[test]
fn fast_subset_config_refuses_auto_revert_off() {
    let r = fast_root("fastcfg");
    train_set(&r, "merge_fast_tests", "\"p: fast\"");
    assert_eq!(r.run(&["queue", "list"]).code, 0, "a listed project with auto-revert on loads");
    train_set(&r, "queue_auto_revert", "\"off\"");
    let o = r.run(&["queue", "list"]);
    assert_ne!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("merge_fast_tests needs queue_auto_revert = on"), "{}", o.stderr);
}

/// Queue one notes-only task of `p` with `merge_fast_tests = "p: <names>"` and run it; returns the entry, the run and main before it.
fn fast_entry(r: &Root, names: &str) -> (String, Out, String) {
    train_set(r, "merge_fast_tests", &format!("\"p: {names}\""));
    let id = r.started("NOTES.txt");
    r.commit(&r.wt(&id), "NOTES.txt", "notes\n", "task: notes");
    let e = r.add(&id, "low", "sonnet");
    let before = r.head();
    let o = r.run(&["queue", "run"]);
    (e, o, before)
}

/// The pre-merge check of entry `e` was REFUSED because `name` is not a test target cargo metadata lists: nothing ran, nothing
/// merged, no model was asked.
fn assert_refused(r: &Root, e: &str, o: &Out, before: &str, name: &str) {
    assert_eq!(o.code, 5, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(e), "check_failed", "{}", r.show(e));
    let show = r.show(e);
    assert!(show.contains(&format!("check: REFUSED (merge_fast_tests: `{name}` (project p) is not a test target")), "{show}");
    assert!(!o.stdout.contains("job: run p "), "no subset job ran: {}", o.stdout);
    assert_eq!(r.head(), before);
    assert_eq!(r.calls(), 0);
}

#[test]
fn fast_subset_an_unknown_name_refuses_the_pre_merge_check() {
    let r = fast_root("fastunknown");
    let (e, o, before) = fast_entry(&r, "fast nosuch");
    assert_refused(&r, &e, &o, &before, "nosuch");
}

#[test]
fn fast_subset_names_come_from_cargo_metadata_not_file_names_autotests_off() {
    // autotests = false: tests/stale.rs is a stray file, not a test target, so `stale` is refused
    let r = revert_root("faststale");
    let p = r.p();
    fs::write(p.join("Cargo.toml"), "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\nautotests = false\n\n[workspace]\n").unwrap();
    fs::create_dir_all(p.join("tests")).unwrap();
    fs::write(p.join("tests/stale.rs"), "#[test]\nfn stale() {}\n").unwrap();
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "main: autotests off with a stray tests/stale.rs"]);
    let (e, o, before) = fast_entry(&r, "stale");
    assert_refused(&r, &e, &o, &before, "stale");
}

/// p with one renamed test target: `[[test]] name = "foo"`, path tests/bar.rs.
fn renamed_root(name: &str) -> Root {
    let r = revert_root(name);
    let p = r.p();
    fs::write(p.join("Cargo.toml"), "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[[test]]\nname = \"foo\"\npath = \"tests/bar.rs\"\n\n[workspace]\n").unwrap();
    fs::create_dir_all(p.join("tests")).unwrap();
    fs::write(p.join("tests/bar.rs"), "#[test]\nfn foo_ok() {\n    assert_eq!(p::add(2, 2), 4);\n}\n").unwrap();
    git(&p, &["add", "."]);
    git(&p, &["commit", "-q", "-m", "main: a renamed test target foo at tests/bar.rs"]);
    r
}

#[test]
fn fast_subset_a_renamed_test_target_is_known_by_its_target_name() {
    let r = renamed_root("fastfoo");
    let (e, o, _) = fast_entry(&r, "foo");
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "merged", "{}", r.show(&e));
    assert!(r.show(&e).contains("check: fast subset (foo) = `cargo build --offline && cargo test --offline --lib --test foo`"), "{}", r.show(&e));
    let r = renamed_root("fastbar");
    let (e, o, before) = fast_entry(&r, "bar");
    assert_refused(&r, &e, &o, &before, "bar");
}

#[test]
fn fast_subset_a_green_auto_revert_lets_the_drain_go_on_to_the_next_entry() {
    let r = revert_root("fastdrain");
    let a = r.started("src");
    r.commit(&r.wt(&a), "src/lib.rs", &only_outside_main(), "task: a test that only passes outside main");
    let ea = r.add(&a, "low", "sonnet");
    let b = r.started("NOTES.txt");
    r.commit(&r.wt(&b), "NOTES.txt", "notes\n", "task: notes");
    let eb = r.add(&b, "low", "sonnet");
    let o = r.run(&["queue", "run", "--all"]);
    assert_eq!(o.code, 6, "the worst outcome is still reported: {}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&ea), "merged_check_failed");
    assert!(r.field(&ea, "reason").contains("main check after the revert passed"), "{}", r.show(&ea));
    assert!(!hold_file(&r).exists());
    assert_eq!(r.status(&eb), "merged", "the drain went on after the green revert: {}{}", o.stdout, o.stderr);
    assert!(!o.stderr.contains("stopped after entry"), "{}", o.stderr);
}
