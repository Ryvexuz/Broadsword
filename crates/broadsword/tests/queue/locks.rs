use super::*;

// ---- concurrency --------------------------------------------------------------------------

#[test]
fn a_second_queue_run_gets_busy_while_one_is_running() {
    let r = Root::new("busy", 120);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let mut first = r.spawn(&["queue", "run"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("the first run to reach the reviewer", 120, || r.calls() == 1);
    let t0 = Instant::now();
    let o = r.run(&["queue", "run"]);
    assert_eq!(o.code, 4, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("queue busy"), "{}", o.stderr);
    assert!(t0.elapsed() < Duration::from_secs(30), "it gives up after queue_lock_wait_seconds");
    // cancelling the entry that is running right now is busy too, and changes nothing
    let o = r.run(&["queue", "cancel", &e]);
    assert_eq!(o.code, 4, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&e), "running");
    // read-only commands are never blocked
    assert_eq!(r.run(&["queue", "list"]).code, 0);
    assert_eq!(r.run(&["queue", "show", &e]).code, 0);
    kill_tree(&mut first);
    fs::remove_file(&hang).unwrap();
    // the lock is gone with the process: the entry is resumed and merged
    let o = r.run(&["queue", "run"]);
    // if it ever ends blocked, say WHY (the reason is the evidence): a blocked resume was the flaky failure of this test
    let st = r.status(&e);
    assert!(!st.starts_with("blocked"), "the resumed entry ended blocked: reason = {}
{}{}", r.field(&e, "reason"), o.stdout, o.stderr);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(st, "merged", "reason = {}", r.field(&e, "reason"));
}

/// A root with projects `p` and `q`, one queued entry in each (own files), `queue_per_project` set to `lanes` ("on" / "off").
/// Returns (root, p's entry, q's entry).
fn two_project_root(name: &str, lanes: &str) -> (Root, String, String) {
    let r = Root::new(name, 120);
    train_set(&r, "queue_per_project", &format!("\"{lanes}\""));
    init_project(&r.dir.join(r.projects).join("q"), "q");
    let pt = r.started("src/a.txt");
    r.commit(&r.wt(&pt), "src/a.txt", "x\n", "task: p");
    let pe = r.add(&pt, "low", "sonnet");
    let qt = r.started_in("q", "src/b.txt");
    r.commit(&r.dir.join("worktrees/q").join(&qt), "src/b.txt", "x\n", "task: q");
    let qe = r.add(&qt, "low", "sonnet");
    (r, pe, qe)
}

#[test]
fn per_project_lock_lets_two_projects_drain_at_once_and_refuses_a_second_driver_of_one_project() {
    // SCALING F2, the done check: with queue_per_project "on", drains of p and q run AT THE SAME TIME (both reach the reviewer
    // while both are alive), a second driver of p exits 4 naming the project, and a driver killed while it holds its lock leaves
    // nothing that blocks the next run of its project.
    let (r, pe, qe) = two_project_root("lanes", "on");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let env = [("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())];
    let mut a = r.spawn(&["queue", "run", "--all", "--project", "p"], &env);
    wait_for("p's drain to reach the reviewer", 120, || r.calls() == 1);
    let mut b = r.spawn(&["queue", "run", "--all", "--project", "q"], &env);
    let end = Instant::now() + ceiling(180);
    while r.calls() < 2 {
        if let Some(st) = b.try_wait().unwrap() {
            kill_tree(&mut a);
            let _ = fs::remove_file(&hang);
            panic!("q's drain ended ({st}) while p's drain held its lock: the two projects did not drain at the same time");
        }
        assert!(Instant::now() < end, "timed out waiting for q's drain to reach the reviewer");
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(a.try_wait().unwrap().is_none(), "p's drain is still running while q's reached its reviewer");
    assert_eq!((r.status(&pe), r.status(&qe)), ("running".to_string(), "running".to_string()));
    let qdir = r.dir.join("state/queue");
    assert!(qdir.join("p.queue.lock").is_file() && qdir.join("q.queue.lock").is_file(), "each project has its own lock file");
    // a second driver of p: refused with exit 4, naming the project; the same for one p entry and for cancelling it
    for args in [vec!["queue", "run", "--all", "--project", "p"], vec!["queue", "run", "--project", "p"], vec!["queue", "run", "--entry", pe.as_str()], vec!["queue", "cancel", pe.as_str()]] {
        let o = r.run(&args);
        assert_eq!(o.code, 4, "{args:?}: {}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains("queue busy") && o.stderr.contains("of project `p`"), "{args:?}: {}", o.stderr);
    }
    assert_eq!(r.status(&pe), "running", "the refused calls changed nothing");
    // no project named: refused, nothing run (it could not tell p's live entry from an interrupted one)
    for args in [vec!["queue", "run", "--all"], vec!["queue", "run"]] {
        let o = r.run(&args);
        assert_eq!(o.code, 2, "{args:?}: {}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains("queue_per_project is on") && o.stderr.contains("--project <p>"), "{args:?}: {}", o.stderr);
    }
    // the entries lock stays global and short: read-only and add-type commands are not blocked by either drain
    assert_eq!(r.run(&["queue", "list"]).code, 0);
    assert_eq!(r.calls(), 2, "no refused call reached a reviewer");
    // CRASH: p's driver dies holding p's lock; q's drain finishes on its own
    kill_tree(&mut a);
    fs::remove_file(&hang).unwrap();
    let st = b.wait().unwrap();
    assert_eq!(st.code(), Some(0), "q's drain: reason = {}", r.field(&qe, "reason"));
    assert_eq!(r.status(&qe), "merged", "reason = {}", r.field(&qe, "reason"));
    assert!(r.status(&pe).starts_with("running (interrupted"), "p's entry was interrupted: {}", r.status(&pe));
    // nothing the dead driver left blocks p: the next drain of p takes the lock at once, resumes the entry and merges it
    // no timing bound on the drain: the test root's queue_lock_wait_seconds is 2 s, so a stale holder makes the drain give up after 2 s
    // with exit 4 ("queue busy"), which the exit code and stderr asserts below catch; how long the build and merge take is not the point
    let o = r.run(&["queue", "run", "--all", "--project", "p"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(!o.stderr.contains("queue busy"), "a stale holder blocked p: {}", o.stderr);
    assert!(o.stdout.contains("RESUMING"), "{}", o.stdout);
    assert_eq!(r.status(&pe), "merged", "reason = {}", r.field(&pe, "reason"));
    assert_eq!(r.run(&["queue", "run", "--all", "--project", "p"]).stdout.trim(), "the queue is empty for project `p`: nothing is queued, interrupted or stale");
}

#[test]
fn per_project_drains_take_only_their_project_and_bad_project_input_runs_nothing() {
    let (r, pe, qe) = two_project_root("lanescope", "on");
    for (args, says) in [
        (vec!["queue", "run", "--all", "--project", "nosuch"], "unknown project `nosuch`"),
        (vec!["queue", "run", "--all", "--project", "../p"], "invalid project name"),
        (vec!["queue", "run", "--entry", pe.as_str(), "--project", "q"], "is of project `p`, not `q`"),
        (vec!["queue", "run", "--entry", "q-nope"], "no queue entry with the exact id `q-nope`"),
    ] {
        let o = r.run(&args);
        assert_eq!(o.code, 2, "{args:?}: {}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains(says), "{args:?}: {}", o.stderr);
    }
    assert_eq!((r.status(&pe), r.status(&qe), r.calls()), ("queued".to_string(), "queued".to_string(), 0), "nothing ran");
    // q's drain takes q's entry only
    let o = r.run(&["queue", "run", "--all", "--project", "q"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("--- queue run --all --project q: 1 entry"), "{}", o.stdout);
    assert_eq!((r.status(&pe), r.status(&qe)), ("queued".to_string(), "merged".to_string()));
    // one entry of p, by project (case does not matter), and with --entry its own project's lock is used
    let o = r.run(&["queue", "run", "--project", "P"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&pe), "merged", "reason = {}", r.field(&pe, "reason"));
}

#[test]
fn with_queue_per_project_off_every_run_still_holds_the_one_global_lock() {
    // shipped "off": exactly as before. A drain of q waits for the GLOBAL queue.lock that p's drain holds and exits 4 (no project in
    // the message, no per-project lock file); `--project` then only narrows which entries a run takes.
    let (r, pe, qe) = two_project_root("lanesoff", "off");
    let hang = r.dir.join("hang.flag");
    fs::write(&hang, "1").unwrap();
    let mut a = r.spawn(&["queue", "run", "--all", "--project", "p"], &[("FAKE_CLAUDE_HANG_FILE", hang.to_str().unwrap())]);
    wait_for("p's drain to reach the reviewer", 120, || r.calls() == 1);
    let o = r.run(&["queue", "run", "--all", "--project", "q"]);
    assert_eq!(o.code, 4, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("queue busy: another `queue run` (or a cancel of a running entry)"), "{}", o.stderr);
    let qdir = r.dir.join("state/queue");
    assert!(qdir.join("queue.lock").is_file() && !qdir.join("p.queue.lock").exists() && !qdir.join("q.queue.lock").exists());
    kill_tree(&mut a);
    fs::remove_file(&hang).unwrap();
    // --project q narrows the drain: p's interrupted entry is left for a run that includes p
    let o = r.run(&["queue", "run", "--all", "--project", "q"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(r.status(&pe).starts_with("running (interrupted"), "p's entry is left interrupted: {}", r.status(&pe));
    assert_eq!(r.status(&qe), "merged", "reason = {}", r.field(&qe, "reason"));
    // and with no project named, a run takes every project, as before
    let o = r.run(&["queue", "run", "--all"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status(&pe), "merged", "reason = {}", r.field(&pe, "reason"));
}
