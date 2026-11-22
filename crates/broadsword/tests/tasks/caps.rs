use super::*;

// ---- the agent cap (max_agents) -----------------------------------------------------------

fn set_cap(r: &Root, cap: u64) {
    let p = r.dir.join("broadsword.toml");
    let t: String = fs::read_to_string(&p).unwrap().lines().map(|l| if l.starts_with("max_agents") { format!("max_agents = {cap}
") } else { format!("{l}
") }).collect();
    assert_eq!(t.matches(&format!("max_agents = {cap}
")).count(), 1);
    fs::write(p, t).unwrap();
}

#[test]
fn start_refuses_at_the_agent_cap_across_projects() {
    let r = Root::new("cap");
    set_cap(&r, 2);
    let a = r.add("p", "a", &["src/a.rs"]);
    let b = r.add("q", "b", &["src/b.rs"]);
    let c = r.add("p", "c", &["src/c.rs"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    assert_eq!(r.start(&b, "coder-2").code, 0);
    let o = r.start(&c, "coder-3");
    assert_eq!(o.code, 3, "third start must be refused at cap 2: {}", o.stderr);
    assert!(o.stderr.contains("max_agents") && o.stderr.contains(&a) && o.stderr.contains(&b), "{}", o.stderr);
    assert_eq!(r.status_of(&c), "open", "a refused start changes nothing");
    assert_eq!(r.run(&["task", "done", &a]).code, 0);
    assert_eq!(r.start(&c, "coder-3").code, 0, "a freed slot can be taken");
    // Lowering the cap below the running count stops new starts but leaves running tasks alone.
    set_cap(&r, 1);
    let d = r.add("q", "d", &["docs"]);
    assert_eq!(r.start(&d, "coder-4").code, 3);
    assert_eq!((r.status_of(&b), r.status_of(&c)), ("running".to_string(), "running".to_string()));
}

#[test]
fn eight_gated_starts_never_exceed_a_cap_of_three() {
    let r = Root::new("capr");
    set_cap(&r, 3);
    for round in 0..3 {
        let ids: Vec<String> = (0..8).map(|i| r.add(if i % 2 == 0 { "p" } else { "q" }, "t", &[&format!("r{round}/f{i}.rs")])).collect();
        let who: Vec<(&str, String)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), format!("coder-{i}"))).collect();
        let res = start_together(&r, &who, round);
        let won = res.iter().filter(|(c, _)| *c == 0).count();
        assert_eq!(won, 3, "round {round}: exactly the cap may start: {res:?}");
        assert!(res.iter().all(|(c, _)| *c == 0 || *c == 3), "losers exit 3: {res:?}");
        assert_eq!(r.list(&["--status", "running"]).len(), 3);
        for id in &ids {
            if r.status_of(id) == "running" {
                assert_eq!(r.run(&["task", "abandon", id]).code, 0);
            }
        }
    }
}

// ---- the merge queue and the cap (max_agents counts working coders; max_queued) ------------

fn set_key(r: &Root, key: &str, value: u64) {
    let p = r.dir.join("broadsword.toml");
    let t: String = fs::read_to_string(&p).unwrap().lines().map(|l| if l.split_once(" = ").is_some_and(|(k, _)| k == key) { format!("{key} = {value}\n") } else { format!("{l}\n") }).collect();
    assert_eq!(t.matches(&format!("{key} = {value}\n")).count(), 1);
    fs::write(p, t).unwrap();
}

/// Write a queue entry record the way queue.rs stores it (`<state>/queue/<id>.entry`, `key = value` lines).
fn put_entry(r: &Root, id: &str, task: &str, status: &str) {
    let dir = r.dir.join("state/queue");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(format!("{id}.entry")), format!("id = {id}\ntask = {task}\nproject = p\nstatus = {status}\ncreated = 1\n")).unwrap();
}

#[test]
fn tasks_waiting_in_the_merge_queue_do_not_count_against_max_agents() {
    let r = Root::new("qcap");
    set_cap(&r, 2);
    let a = r.add("p", "a", &["src/a.rs"]);
    let b = r.add("q", "b", &["src/b.rs"]);
    let c = r.add("p", "c", &["src/c.rs"]);
    let d = r.add("q", "d", &["src/d.rs"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    assert_eq!(r.start(&b, "coder-2").code, 0);
    put_entry(&r, "q-1", &a, "queued");
    put_entry(&r, "q-2", &b, "running");
    let o = r.start(&c, "coder-3");
    assert_eq!(o.code, 0, "two running tasks that wait in the queue leave both slots free: {}", o.stderr);
    // a's entry is rejected: a is back in a fix round, so it counts again (a and c count, b still waits): cap 2 is reached.
    put_entry(&r, "q-1", &a, "rejected");
    let o = r.start(&d, "coder-4");
    assert_eq!(o.code, 3, "a task whose entry ended counts again: {}", o.stderr);
    assert!(o.stderr.contains("max_agents") && o.stderr.contains(&format!("{a} (")) && o.stderr.contains(&format!("{c} (")), "{}", o.stderr);
    assert!(!o.stderr.contains(&format!("{b} (")), "b waits in the queue, it is not listed as counted: {}", o.stderr);
    assert_eq!(r.status_of(&d), "open");
    // A stale entry is still open in queue.rs, but its coder must act (move the branch back or re-check it): a counts.
    put_entry(&r, "q-1", &a, "stale");
    let o = r.start(&d, "coder-4");
    assert_eq!(o.code, 3, "a task whose entry is stale needs its coder, so it counts: {}", o.stderr);
    assert!(o.stderr.contains(&format!("{a} (")) && !o.stderr.contains(&format!("{b} (")), "{}", o.stderr);
    // Back to queued: a waits again and d can start.
    put_entry(&r, "q-1", &a, "queued");
    assert_eq!(r.start(&d, "coder-4").code, 0, "a queued entry waits in the queue");
}

#[test]
fn tasks_with_blocked_or_stale_entries_count_against_max_agents() {
    let r = Root::new("qbs");
    set_cap(&r, 2);
    let a = r.add("p", "a", &["src/a.rs"]);
    let b = r.add("q", "b", &["src/b.rs"]);
    let c = r.add("p", "c", &["src/c.rs"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    assert_eq!(r.start(&b, "coder-2").code, 0);
    // Blocked (dirty worktree, changes after review) and stale (branch moved, no checked commit) entries are open, but
    // each needs its coder to act, so both coders are at work: a third start would exceed max_agents.
    put_entry(&r, "q-1", &a, "blocked");
    put_entry(&r, "q-2", &b, "stale");
    let o = r.start(&c, "coder-3");
    assert_eq!(o.code, 3, "two running tasks with blocked/stale entries fill cap 2: {}", o.stderr);
    assert!(o.stderr.contains("max_agents") && o.stderr.contains(&format!("{a} (")) && o.stderr.contains(&format!("{b} (")), "{}", o.stderr);
    assert_eq!(r.status_of(&c), "open", "a refused start changes nothing");
}

#[test]
fn an_approved_entry_frees_its_max_agents_slot_but_counts_for_max_queued() {
    // `approved` (review passed, waiting for a merge train): the coder is not at work, so the task does not count against
    // max_agents, but the entry is open, so it counts for max_queued.
    let r = Root::new("qappr");
    set_cap(&r, 1);
    set_key(&r, "max_queued", 2);
    let a = r.add("p", "a", &["src/a.rs"]);
    let b = r.add("q", "b", &["src/b.rs"]);
    let c = r.add("p", "c", &["src/c.rs"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    put_entry(&r, "q-1", &a, "approved");
    let o = r.start(&b, "coder-2");
    assert_eq!(o.code, 0, "a task whose entry is approved leaves its slot free at max_agents = 1: {}", o.stderr);
    put_entry(&r, "q-2", &b, "approved");
    let o = r.start(&c, "coder-3");
    assert_eq!(o.code, 3, "two approved entries fill max_queued = 2: {}", o.stderr);
    assert!(o.stderr.contains("max_queued") && o.stderr.contains("q-1") && o.stderr.contains("q-2"), "{}", o.stderr);
    assert!(!o.stderr.contains("max_agents"), "no coder is at work, so max_agents is not what refused: {}", o.stderr);
    assert_eq!(r.status_of(&c), "open", "a refused start changes nothing");
    // b's entry is rejected: b is back with its coder (1 working = max_agents 1) and one entry is open.
    put_entry(&r, "q-2", &b, "rejected");
    let o = r.start(&c, "coder-3");
    assert_eq!(o.code, 3, "b's coder is at work again, so max_agents = 1 refuses: {}", o.stderr);
    assert!(o.stderr.contains("max_agents") && o.stderr.contains(&format!("{b} (")) && !o.stderr.contains(&format!("{a} (")), "{}", o.stderr);
}

#[test]
fn start_refuses_at_max_queued_open_entries() {
    let r = Root::new("qmax");
    set_key(&r, "max_queued", 1);
    let a = r.add("p", "a", &["src/a.rs"]);
    let b = r.add("q", "b", &["src/b.rs"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    put_entry(&r, "q-1", &a, "blocked");
    put_entry(&r, "q-0", "t-old", "merged"); // ended entries never count
    let o = r.start(&b, "coder-2");
    assert_eq!(o.code, 3, "one open entry at max_queued = 1 refuses the next start: {}", o.stderr);
    assert!(o.stderr.contains("max_queued") && o.stderr.contains("q-1") && o.stderr.contains(&a), "{}", o.stderr);
    assert!(!o.stderr.contains("q-0"), "{}", o.stderr);
    assert_eq!(r.status_of(&b), "open");
    put_entry(&r, "q-1", &a, "cancelled");
    assert_eq!(r.start(&b, "coder-2").code, 0, "below max_queued again");
}

#[test]
fn an_orphan_open_entry_counts_for_max_queued_and_the_refusal_says_how_to_clear_it() {
    let r = Root::new("qorph");
    set_key(&r, "max_queued", 1);
    let a = r.add("p", "a", &["src/a.rs"]);
    let b = r.add("q", "b", &["src/b.rs"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    put_entry(&r, "q-orph", &a, "queued");
    // a is abandoned while its entry is still open: the entry is an orphan, but still real open queue work (fail closed).
    assert_eq!(r.run(&["task", "abandon", &a]).code, 0);
    let o = r.start(&b, "coder-2");
    assert_eq!(o.code, 3, "the orphan entry still counts for max_queued: {}", o.stderr);
    assert!(o.stderr.contains("max_queued"), "{}", o.stderr);
    assert!(o.stderr.contains(&format!("entry q-orph is open but its task {a} is abandoned")), "the orphan is named with its task's status: {}", o.stderr);
    assert!(o.stderr.contains("broadsword queue cancel q-orph"), "the message names the command that clears it: {}", o.stderr);
    assert_eq!(r.status_of(&b), "open");
    let c = r.run(&["queue", "cancel", "q-orph"]);
    assert_eq!(c.code, 0, "{}", c.stderr);
    let o = r.start(&b, "coder-2");
    assert_eq!(o.code, 0, "after the cancel the start succeeds: {}", o.stderr);
}

#[test]
fn start_waits_for_a_queue_entry_that_is_still_being_written() {
    let r = Root::new("qslow");
    let a = r.add("p", "a", &["src/a.rs"]);
    let dir = r.dir.join("state/queue");
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join("q-slow.entry");
    fs::write(&p, "").unwrap(); // not yet complete
    let w = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        fs::write(&p, "id = q-slow\ntask = t-x\nproject = p\nstatus = merged\ncreated = 1\n").unwrap();
    });
    let o = r.start(&a, "coder-1");
    w.join().unwrap();
    assert_eq!(o.code, 0, "an entry that becomes complete after ~300 ms does not fail the start: {}", o.stderr);
}

#[test]
fn a_malformed_queue_entry_makes_start_fail_naming_the_file() {
    let r = Root::new("qbad");
    let a = r.add("p", "a", &["src/a.rs"]);
    let dir = r.dir.join("state/queue");
    fs::create_dir_all(&dir).unwrap();
    for (name, text) in [("q-bad.entry", "this is not a record\n"), ("q-bad.entry", ""), ("q-bad.entry", "id = q-bad\ntask = t-x\nstatus = no-such-status\n"), ("q-bad.entry", "id = q-bad\nstatus = queued\n")] {
        fs::write(dir.join(name), text).unwrap();
        let o = r.start(&a, "coder-1");
        assert_eq!(o.code, 1, "{text:?}: {}", o.stderr);
        assert!(o.stderr.contains("q-bad.entry"), "{text:?}: the message names the file: {}", o.stderr);
        assert_eq!(r.status_of(&a), "open", "nothing was started");
    }
    fs::write(dir.join("q-bad.entry"), "id = q-bad\ntask = t-x\nstatus = merged\n").unwrap();
    assert_eq!(r.start(&a, "coder-1").code, 0, "a well-formed entry is fine");
}

#[test]
fn missing_max_queued_is_a_config_error() {
    let r = Root::new("qcfg");
    let a = r.add("p", "a", &["src/a.rs"]);
    fs::write(r.dir.join("broadsword.toml"), support::config_without("max_queued")).unwrap();
    let o = r.start(&a, "coder-1");
    assert_ne!(o.code, 0, "{}", o.stdout);
    assert!(o.stderr.contains("max_queued") && o.stderr.contains("missing"), "{}", o.stderr);
    fs::write(r.dir.join("broadsword.toml"), support::config(&[("max_queued", "0")])).unwrap();
    let o = r.start(&a, "coder-1");
    assert!(o.code != 0 && o.stderr.contains("max_queued"), "max_queued = 0 is refused: {}", o.stderr);
}

// ---- the RAM line (ram_warn_percent) ------------------------------------------------------
// Every test root sets ram_warn_percent = 100, so the real RAM level (which may be above 90% on this machine) never
// refuses an unrelated start. The hook can only RAISE the reading, so the refusal cases use hook values at or above the
// line and the "no refusal" cases use no hook, or a hook that must be ignored.

const RAM_HOOK_ENV: &str = "BROADSWORD_INTERNAL_TEST_RAM_USED_PERCENT_FILE";

fn set_warn(r: &Root, pct: u64) {
    let p = r.dir.join("broadsword.toml");
    let t: String = fs::read_to_string(&p).unwrap().lines().map(|l| if l.starts_with("ram_warn_percent") { format!("ram_warn_percent = {pct}\n") } else { format!("{l}\n") }).collect();
    assert_eq!(t.matches(&format!("ram_warn_percent = {pct}\n")).count(), 1);
    fs::write(p, t).unwrap();
}

fn start_with_ram_hook(r: &Root, id: &str, hook_file: &Path) -> Out {
    let o = r.cmd(&["task", "start", id, "--owner", "coder-1"]).env(RAM_HOOK_ENV, hook_file).output().unwrap();
    Out { code: o.status.code().unwrap(), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
}

#[test]
fn start_refuses_when_ram_is_at_or_above_the_warning_line() {
    let r = Root::new("ram95");
    set_warn(&r, 90);
    let a = r.add("p", "a", &["src/a.rs"]);
    let hook = r.dir.join("scratch").join("ram.txt");
    fs::write(&hook, "95").unwrap();
    let o = start_with_ram_hook(&r, &a, &hook);
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert!(o.stderr.contains("TEST HOOK ACTIVE"), "{}", o.stderr);
    assert!(o.stderr.contains("RAM is ") && o.stderr.contains("% used") && o.stderr.contains("ram_warn_percent 90") && o.stderr.contains("memory frees up"), "{}", o.stderr);
    // The reading is max(hook, real): at least the hooked 95, whatever the machine really uses.
    let pct: u64 = o.stderr.split("RAM is ").nth(1).unwrap().split('%').next().unwrap().trim().parse().unwrap();
    assert!(pct >= 95, "{}", o.stderr);
    assert_eq!(r.status_of(&a), "open", "a refused start changes nothing");
    assert!(!r.list(&["--status", "running"]).iter().any(|l| l.contains(&a)), "the refused task must not be running");
}

#[test]
fn start_succeeds_below_the_ram_line() {
    let r = Root::new("ramok");
    // The line is 100 and the hook says 50: RAM is below it whatever the real level is (short of exactly 100%).
    let a = r.add("p", "a", &["src/a.rs"]);
    let hook = r.dir.join("scratch").join("ram.txt");
    fs::write(&hook, "50").unwrap();
    let o = start_with_ram_hook(&r, &a, &hook);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert_eq!(r.status_of(&a), "running");
}

#[test]
fn a_ram_hook_value_below_the_real_reading_cannot_weaken_the_guard() {
    let r = Root::new("ramlow");
    // A line of 1% is below any real RAM use; a hook claiming 0% must not make the start pass.
    set_warn(&r, 1);
    let a = r.add("p", "a", &["src/a.rs"]);
    let hook = r.dir.join("scratch").join("ram.txt");
    fs::write(&hook, "0").unwrap();
    let o = start_with_ram_hook(&r, &a, &hook);
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert!(o.stderr.contains("ram_warn_percent 1"), "{}", o.stderr);
    assert_eq!(r.status_of(&a), "open");
}

#[test]
fn a_ram_hook_file_outside_scratch_is_ignored() {
    let r = Root::new("ramout");
    // Line 100, hook 100: honoured it would refuse; ignored (outside scratch) the real reading (below 100) lets it start.
    let a = r.add("p", "a", &["src/a.rs"]);
    let hook = r.dir.join("ram-outside.txt");
    fs::write(&hook, "100").unwrap();
    let o = start_with_ram_hook(&r, &a, &hook);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stderr.contains("ignoring") && !o.stderr.contains("TEST HOOK ACTIVE"), "{}", o.stderr);
    assert_eq!(r.status_of(&a), "running");
}

#[test]
fn an_unreadable_tasks_folder_is_an_error_never_an_empty_board() {
    let r = Root::new("unreadable-board");
    // a REAL task, then the tasks folder replaced by a FILE: read_dir fails with something other than NotFound
    let id = r.add("p", "real task", &["src"]);
    let tasks = r.dir.join("state").join("tasks");
    fs::remove_dir_all(&tasks).unwrap();
    fs::write(&tasks, "not a folder").unwrap();
    let o = r.run(&["task", "list"]);
    assert_ne!(o.code, 0, "list must not succeed on an unreadable board: {}", o.stdout);
    assert!(o.stderr.contains("cannot read"), "stderr: {}", o.stderr);
    // `start` is refused at the board LOCK step (its lock file lives inside the tasks folder), before load_tasks is reached, so the
    // message is the lock's folder error: `cannot create <root>/state/tasks: Cannot create a file when that file already exists.`
    let o = r.start(&id, "someone");
    assert_ne!(o.code, 0, "start must not succeed on an unreadable board: {}", o.stdout);
    let norm = o.stderr.replace('\\', "/");
    assert!(o.stderr.contains("cannot create") && norm.contains("state/tasks"), "stderr must give the folder error and path: {}", o.stderr);
    assert!(!o.stderr.contains("no task with the exact id"), "must not be the missing-id error: {}", o.stderr);
    // control: on a READABLE board the same start of an id that does not exist gives the missing-id message instead, so the
    // assertions above can only pass because of the unreadable board.
    fs::remove_file(&tasks).unwrap();
    fs::create_dir_all(&tasks).unwrap();
    let c = r.start("t-nonexistent", "someone");
    assert_ne!(c.code, 0);
    assert!(c.stderr.contains("no task with the exact id") && !c.stderr.contains("cannot create"), "control stderr: {}", c.stderr);
    assert!(!o.stdout.contains("running"), "nothing may be marked running: {}", o.stdout);
}

#[test]
fn a_missing_tasks_folder_is_an_empty_board() {
    let r = Root::new("missing-board");
    let o = r.run(&["task", "list"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
}
