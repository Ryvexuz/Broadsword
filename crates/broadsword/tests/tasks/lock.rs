use super::*;

#[test]
fn garbage_in_the_lock_file_is_ignored_and_a_live_holder_means_board_busy() {
    let r = Root::with_wait("lock", Some(1));
    let a = r.add("p", "a", &["src"]);
    // whatever the file says, only a real OS lock counts: no stale detection exists
    for junk in ["4000000000\n\n", "", "garbage", &format!("{}\n\n", std::process::id())] {
        fs::write(lock_path(&r), junk).unwrap();
        assert_eq!(r.run(&["task", "show", &a]).code, 0);
    }
    fs::write(lock_path(&r), "4000000000\n\n").unwrap();
    assert_eq!(r.start(&a, "x").code, 0, "a garbage lock file does not block");
    assert!(lock_path(&r).exists(), "the lock file is never deleted");
    let b = r.add("p", "b", &["lib"]);
    let held = hold(&r);
    let t = Instant::now();
    let o = r.start(&b, "y");
    assert_eq!(o.code, 4, "{}", o.stderr);
    assert!(o.stderr.contains("board busy") && !o.stderr.contains("delete"), "{}", o.stderr);
    assert!(t.elapsed() >= Duration::from_millis(900), "it waited for the lock");
    assert_eq!(r.status_of(&b), "open", "nothing changed");
    drop(held);
    assert_eq!(r.start(&b, "y").code, 0);
}

/// The test binary doubles as the lock holder for the kill test: with the env var set, this "test" takes the board lock,
/// announces it with a marker file and sleeps until killed. Without the env var it does nothing.
#[test]
fn lock_holder_helper() {
    let Ok(dir) = std::env::var("BROADSWORD_TEST_HOLD_LOCK_ROOT") else { return };
    let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(Path::new(&dir).join("state/tasks/board.lock")).unwrap();
    f.lock().unwrap();
    fs::write(Path::new(&dir).join("scratch/held"), "x").unwrap();
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn a_killed_lock_holder_frees_the_board_at_once() {
    // Probe the OS lock directly after the kill: timing a task start also counts process startup under load.
    // Keep the 20 s wait and exit-code checks to prove a live holder blocks and a start after the kill succeeds.
    let r = Root::with_wait("kill", Some(20));
    let a = r.add("p", "a", &["src"]);
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock::lock_holder_helper", "--nocapture"])
        .env("BROADSWORD_TEST_HOLD_LOCK_ROOT", &r.dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while !r.dir.join("scratch/held").exists() {
        assert!(Instant::now() < end, "the holder never took the lock");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(r.start(&a, "x").code, 4, "a live holder blocks");
    let k = Command::new("taskkill").args(["/F", "/PID", &holder.id().to_string()]).output().unwrap();
    assert!(k.status.success(), "taskkill: {}", String::from_utf8_lossy(&k.stderr));
    let _ = holder.wait();
    let f = fs::OpenOptions::new().read(true).write(true).open(lock_path(&r)).unwrap();
    if let Err(err) = f.try_lock() {
        panic!("the killed holder's board lock was still held: {err}");
    }
    drop(f);
    let o = r.start(&a, "x");
    assert_eq!(o.code, 0, "{}", o.stderr);
}

#[test]
fn sixteen_gated_starts_with_a_garbage_lock_and_a_big_board_never_give_two_running() {
    // The reviewer's repro: a garbage lock file, 16 simultaneous starts all claiming `src`, and thousands of records so the
    // critical section is slow. Exactly one runs every round and no start is left blocked (no exit 4).
    // All 16 starts serialize on one lock, each scanning 3000+ records; under load the line exceeded 30 s.
    // A garbage-lock regression still blocks every start and still fails after the 120 s wait.
    const ROUNDS: usize = 8;
    const FILLER: usize = 3000;
    let r = Root::with_wait("race16", Some(120));
    let dir = r.dir.join("state/tasks");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..FILLER {
        fs::write(dir.join(format!("filler-{i}.task")), format!("id = filler-{i}\nproject = p\nstatus = done\nclaims = src\ncreated = {i}\ntitle = filler\n")).unwrap();
    }
    let t0 = Instant::now();
    let mut blocked = 0;
    for round in 0..ROUNDS {
        fs::write(lock_path(&r), "4000000000\n\n").unwrap(); // stale-looking garbage
        let ids: Vec<String> = (0..16).map(|i| r.add("p", &format!("t{i}"), &["src", &format!("own/f{i}")])).collect();
        let who: Vec<(&str, String)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), format!("agent-{i}"))).collect();
        let res = start_together(&r, &who, round);
        let codes: Vec<i32> = res.iter().map(|x| x.0).collect();
        blocked += codes.iter().filter(|c| **c == 4).count();
        assert_eq!(codes.iter().filter(|c| **c == 0).count(), 1, "round {round}: exactly one runs: {codes:?}");
        assert_eq!(codes.iter().filter(|c| **c == 3).count(), 15, "round {round}: fifteen refused with exit 3: {codes:?}");
        assert_eq!(r.list(&["--status", "running"]).len(), 1, "round {round}: running must be exactly 1");
        for id in &ids {
            fs::remove_file(dir.join(format!("{id}.task"))).unwrap();
        }
    }
    assert_eq!(blocked, 0, "no start may be left blocked");
    eprintln!("race16: {ROUNDS} rounds x 16 gated starts, {FILLER} filler records, garbage lock file each round: exactly one running every round, {blocked} blocked starts, {:.1} s", t0.elapsed().as_secs_f64());
}

#[test]
fn two_overlapping_starts_in_a_race_one_wins_every_round() {
    const ROUNDS: usize = 60;
    let r = Root::new("race2");
    let t0 = Instant::now();
    let mut wins_first = 0;
    for round in 0..ROUNDS {
        let _ = fs::remove_dir_all(r.dir.join("state/tasks")); // reset the board
        let (ca, cb) = if round % 2 == 0 { (["src"], ["src/a.rs"]) } else { (["src/a.rs"], ["src"]) };
        let a = r.add("p", "a", &ca);
        let b = r.add("p", "b", &cb);
        let res = start_together(&r, &[(&a, "agent-a".into()), (&b, "agent-b".into())], round);
        let mut codes: Vec<i32> = res.iter().map(|x| x.0).collect();
        assert_eq!(res.iter().filter(|x| x.0 == 0).count(), 1, "round {round}: exactly one must win, got {codes:?}: {res:?}");
        let loser = res.iter().find(|x| x.0 != 0).unwrap();
        assert_eq!(loser.0, 3, "round {round}: the loser must get exit 3: {loser:?}");
        assert!(loser.1.contains("agent-") && loser.1.contains("overlaps"), "round {round}: {}", loser.1);
        let running = r.list(&["--status", "running"]);
        assert_eq!(running.len(), 1, "round {round}: exactly one task is running: {running:?}");
        if res[0].0 == 0 {
            wins_first += 1;
        }
        codes.sort_unstable();
        assert_eq!(codes, [0, 3]);
    }
    eprintln!("race2: {ROUNDS} rounds of 2 simultaneous `task start`, exactly one winner each round, in {:.1} s ({wins_first} won by the first-launched process)", t0.elapsed().as_secs_f64());
}

#[test]
fn eight_mutually_overlapping_starts_in_a_race_exactly_one_runs() {
    const ROUNDS: usize = 10;
    let r = Root::with_wait("race8", Some(30));
    let t0 = Instant::now();
    for round in 0..ROUNDS {
        let _ = fs::remove_dir_all(r.dir.join("state/tasks"));
        // every task claims `shared` plus its own file, so each pair overlaps through `shared` only
        let ids: Vec<String> = (0..8).map(|i| r.add("p", &format!("t{i}"), &["shared", &format!("own/f{i}.rs")])).collect();
        let who: Vec<(&str, String)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), format!("agent-{i}"))).collect();
        let res = start_together(&r, &who, round);
        let codes: Vec<i32> = res.iter().map(|x| x.0).collect();
        assert_eq!(codes.iter().filter(|c| **c == 0).count(), 1, "round {round}: exactly one runs: {res:?}");
        assert_eq!(codes.iter().filter(|c| **c == 3).count(), 7, "round {round}: the other seven get exit 3: {res:?}");
        assert_eq!(r.list(&["--status", "running"]).len(), 1, "round {round}");
        assert_eq!(r.list(&["--status", "open"]).len(), 7, "round {round}");
    }
    eprintln!("race8: {ROUNDS} rounds of 8 simultaneous `task start` on 8 mutually overlapping tasks, exactly one running each round, in {:.1} s", t0.elapsed().as_secs_f64());
}

#[test]
fn non_overlapping_starts_in_a_race_all_run() {
    let r = Root::new("race-free");
    let ids: Vec<String> = (0..6).map(|i| r.add("p", &format!("t{i}"), &[&format!("mod{i}")])).collect();
    let who: Vec<(&str, String)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), format!("agent-{i}"))).collect();
    let res = start_together(&r, &who, 0);
    assert!(res.iter().all(|x| x.0 == 0), "disjoint claims must all start: {res:?}");
    assert_eq!(r.list(&["--status", "running"]).len(), 6);
}
