//! End-to-end tests for the task board (`broadsword task ...`): the real binary, inside throwaway Broadsword roots under
//! the repo's `scratch/` folder (never the real root). Includes the race tests: the done check "two tasks with
//! overlapping claims never run at once", in miniature.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/json.rs"]
mod j;
use j::{parse, J};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");
const GATE_ENV: &str = "BROADSWORD_INTERNAL_TEST_TASK_START_GATE_FILE";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Root {
    dir: PathBuf,
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Root {
    fn new(name: &str) -> Root {
        Root::with_wait(name, None)
    }

    fn with_wait(name: &str, wait: Option<u64>) -> Root {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("tasks-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        for p in ["p", "q"] {
            fs::create_dir_all(dir.join("projects").join(p)).unwrap();
            fs::write(dir.join("projects").join(p).join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        }
        let toml = format!(
            "root = \".\"\ndisk_floor_gb = 1\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\ndisk_check_every_seconds = 60\nlog_max_mb = 50\n\
             ram_warn_percent = 100\ncpu_warn_percent = 90\nalert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
             state_dir = \"state\"\ninstall_dir = \"build/bin\"\nmcp_call_timeout_seconds = 600\nmcp_output_max_kb = 1024\n\
             checkpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 0\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = {}\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = 10\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
",
            wait.unwrap_or(10)
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args).current_dir(&self.dir).stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Out {
        let o = self.cmd(args).output().unwrap();
        Out { code: o.status.code().unwrap(), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
    }

    /// `task add`, which must succeed; returns the id.
    fn add(&self, project: &str, title: &str, claims: &[&str]) -> String {
        let mut a = vec!["task", "add", project, "--title", title];
        for c in claims {
            a.push("--claim");
            a.push(c);
        }
        let o = self.run(&a);
        assert_eq!(o.code, 0, "add failed: {}", o.stderr);
        o.stdout.trim().to_string()
    }

    fn start(&self, id: &str, owner: &str) -> Out {
        self.run(&["task", "start", id, "--owner", owner])
    }

    fn status_of(&self, id: &str) -> String {
        let o = self.run(&["task", "show", id]);
        assert_eq!(o.code, 0, "{}", o.stderr);
        o.stdout.lines().find_map(|l| l.strip_prefix("status: ")).unwrap().to_string()
    }

    fn list(&self, extra: &[&str]) -> Vec<String> {
        let mut a = vec!["task", "list"];
        a.extend_from_slice(extra);
        let o = self.run(&a);
        assert_eq!(o.code, 0, "{}", o.stderr);
        o.stdout.lines().map(String::from).collect()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn full_lifecycle() {
    let r = Root::new("life");
    let id = r.add("p", "  build   the thing ", &["src", "docs\\guide.md"]);
    assert!(id.starts_with("t-"), "{id}");
    assert!(r.dir.join("state/tasks").join(format!("{id}.task")).is_file());
    assert_eq!(r.status_of(&id), "open");
    let l = r.list(&[]);
    assert_eq!(l.len(), 1);
    let cols: Vec<&str> = l[0].split('\t').collect();
    assert_eq!(cols, [id.as_str(), "open", "-", "p", "build the thing", "src, docs/guide.md"]);

    let s = r.start(&id, "coder-1");
    assert_eq!(s.code, 0, "{}", s.stderr);
    assert_eq!(r.list(&["--status", "running"])[0].split('\t').collect::<Vec<_>>()[..4], [id.as_str(), "running", "coder-1", "p"]);
    assert_eq!(r.start(&id, "coder-2").code, 1, "a running task cannot be started again");

    let d = r.run(&["task", "done", &id, "--note", "merged as abc"]);
    assert_eq!(d.code, 0, "{}", d.stderr);
    let show = r.run(&["task", "show", &id]);
    for want in ["status: done", "owner: coder-1", "claims: src, docs/guide.md", "note: merged as abc", "project: p"] {
        assert!(show.stdout.contains(want), "{want} in {}", show.stdout);
    }
    assert!(show.stdout.contains("started: 20") && show.stdout.contains("ended: 20") && show.stdout.contains("created: 20"), "{}", show.stdout);
    assert_eq!(r.run(&["task", "done", &id]).code, 1, "done twice");
    assert_eq!(r.run(&["task", "fail", &id]).code, 1, "a done task cannot fail");
    assert_eq!(r.run(&["task", "abandon", &id]).code, 1, "a done task cannot be abandoned");
    assert_eq!(r.run(&["task", "start", &id, "--owner", "x"]).code, 1, "a done task cannot restart");
    assert!(r.dir.join("state/tasks").join(format!("{id}.task")).is_file(), "records are kept");
    assert!(hold_is_free(&r), "the board lock is released (the file itself stays)");
}

#[test]
fn only_a_running_task_can_finish_and_open_can_be_abandoned() {
    let r = Root::new("states");
    let a = r.add("p", "a", &["a"]);
    assert_eq!(r.run(&["task", "done", &a]).code, 1, "open -> done is refused");
    assert_eq!(r.run(&["task", "fail", &a]).code, 1, "open -> failed is refused");
    assert_eq!(r.run(&["task", "abandon", &a]).code, 0);
    assert_eq!(r.status_of(&a), "abandoned");
    assert_eq!(r.start(&a, "x").code, 1, "abandoned tasks never restart");
    assert_eq!(r.run(&["task", "abandon", &a]).code, 1);
}

#[test]
fn a_conflicting_start_exits_3_and_names_the_other_task() {
    let r = Root::new("conflict");
    let a = r.add("p", "a", &["src"]);
    let b = r.add("p", "b", &["lib", "src/a.rs"]);
    let c = r.add("p", "c", &["srcx"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    let o = r.start(&b, "coder-2");
    assert_eq!(o.code, 3, "{}", o.stderr);
    for want in [a.as_str(), "coder-1", "`src/a.rs`", "`src`"] {
        assert!(o.stderr.contains(want), "{want} in {}", o.stderr);
    }
    assert_eq!(r.status_of(&b), "open", "a refused task stays open");
    assert_eq!(r.start(&c, "coder-3").code, 0, "srcx does not overlap src");
    // once the holder is done the refused task can start
    assert_eq!(r.run(&["task", "done", &a]).code, 0);
    assert_eq!(r.start(&b, "coder-2").code, 0);
    // two running tasks are named together
    let w = r.add("p", "whole", &["*"]);
    let o = r.start(&w, "boss");
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains(&b) && o.stderr.contains(&c) && o.stderr.contains("coder-2") && o.stderr.contains("coder-3"), "{}", o.stderr);
}

#[test]
fn different_projects_never_conflict() {
    let r = Root::new("projects");
    let a = r.add("p", "a", &["src"]);
    let b = r.add("q", "b", &["src"]);
    let c = r.add("broadsword", "c", &["*"]);
    assert_eq!(r.start(&a, "x").code, 0);
    assert_eq!(r.start(&b, "y").code, 0, "same path in another project is fine");
    assert_eq!(r.start(&c, "z").code, 0, "the whole of broadsword is not the whole of p");
    assert_eq!(r.list(&["--status", "running"]).len(), 3);
    assert_eq!(r.list(&["--project", "q"]).len(), 1);
}

#[test]
fn check_is_read_only_and_reports_conflicts() {
    let r = Root::new("check");
    let a = r.add("p", "a", &["src/engine"]);
    let free = r.run(&["task", "check", "p", "--claim", "src/engine"]);
    assert_eq!(free.code, 0, "open tasks hold nothing: {}", free.stdout);
    assert!(free.stdout.starts_with("free:"));
    assert_eq!(r.start(&a, "coder-1").code, 0);
    let c = r.run(&["task", "check", "p", "--claim", "docs", "--claim", "src"]);
    assert_eq!(c.code, 3, "{}", c.stdout);
    for want in [a.as_str(), "coder-1", "`src`", "`src/engine`"] {
        assert!(c.stdout.contains(want), "{want} in {}", c.stdout);
    }
    assert_eq!(r.run(&["task", "check", "p", "--claim", "docs", "--claim", "srcx"]).code, 0);
    assert_eq!(r.run(&["task", "check", "q", "--claim", "src"]).code, 0, "other project");
    assert_eq!(r.run(&["task", "check", "p", "--claim", "*"]).code, 3);
    assert_eq!(r.run(&["task", "check", "nope", "--claim", "src"]).code, 2);
    assert_eq!(r.run(&["task", "check", "p", "--claim", "../x"]).code, 2);
    assert_eq!(r.status_of(&a), "running", "check changed nothing");
    assert_eq!(r.list(&[]).len(), 1, "check added nothing");
}

#[test]
fn done_fail_and_abandon_all_release_the_claims() {
    for how in ["done", "fail", "abandon"] {
        let r = Root::new(how);
        let a = r.add("p", "a", &["src"]);
        let b = r.add("p", "b", &["src/x.rs"]);
        assert_eq!(r.start(&a, "one").code, 0);
        assert_eq!(r.start(&b, "two").code, 3, "{how}: held before release");
        let o = r.run(&["task", how, &a]);
        assert_eq!(o.code, 0, "{how}: {}", o.stderr);
        assert_eq!(r.start(&b, "two").code, 0, "{how}: released");
        assert_eq!(r.status_of(&a), match how { "fail" => "failed", "abandon" => "abandoned", _ => "done" });
    }
    // abandon also works on a task that never started
    let r = Root::new("abandon-open");
    let a = r.add("p", "a", &["src"]);
    assert_eq!(r.run(&["task", "abandon", &a]).code, 0);
}

#[test]
fn bad_input_is_refused_with_the_documented_codes() {
    let r = Root::new("bad");
    let o = r.run(&["task", "add", "nope", "--title", "t", "--claim", "src"]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("unknown project") && o.stderr.contains("broadsword"), "{}", o.stderr);
    for bad in ["", "..", "../x", "a/../../x", "/abs", "C:\\x", "src/*", "*.rs", "."] {
        let o = r.run(&["task", "add", "p", "--title", "t", "--claim", bad]);
        assert_eq!(o.code, 2, "claim `{bad}`: {}", o.stderr);
        assert!(o.stderr.contains("invalid claim"), "{}", o.stderr);
    }
    assert_eq!(r.run(&["task", "add", "p", "--title", "  ", "--claim", "src"]).code, 2);
    assert_eq!(r.run(&["task", "add", "p", "--title", "t"]).code, 64, "no claims: not a command line");
    assert_eq!(r.run(&["task", "add", "p", "--claim", "src"]).code, 64, "no title");
    assert_eq!(r.list(&[]), ["no tasks"], "nothing was created");
    assert_eq!(r.run(&["task", "list", "--status", "weird"]).code, 2);
    assert_eq!(r.run(&["task", "show", "t-nope"]).code, 1);
    assert_eq!(r.run(&["task", "show", "../x"]).code, 1);
    assert_eq!(r.start("t-nope", "x").code, 1);
    let a = r.add("p", "a", &["*"]);
    assert_eq!(r.run(&["task", "start", &a, "--owner", "  "]).code, 2);
    assert_eq!(r.status_of(&a), "open");
    // `*` as a whole-project claim is fine
    assert_eq!(r.start(&a, "x").code, 0);
}

#[test]
fn ids_are_unique_when_tasks_are_added_at_the_same_time() {
    let r = Root::new("ids");
    let mut kids: Vec<Child> = (0..8)
        .map(|i| r.cmd(&["task", "add", "p", "--title", &format!("t{i}"), "--claim", "src"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap())
        .collect();
    let mut ids = Vec::new();
    for k in kids.drain(..) {
        let o = k.wait_with_output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        ids.push(String::from_utf8(o.stdout).unwrap().trim().to_string());
    }
    let set: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(set.len(), 8, "{ids:?}");
    assert_eq!(r.list(&[]).len(), 8);
}

#[test]
fn worktrees_share_the_main_roots_board() {
    let r = Root::new("wt");
    let wt = r.dir.join("wt").join("a");
    let gitdir = r.dir.join(".git").join("worktrees").join("a");
    fs::create_dir_all(&gitdir).unwrap();
    fs::create_dir_all(&wt).unwrap();
    fs::copy(r.dir.join("broadsword.toml"), wt.join("broadsword.toml")).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
    let from_wt = |args: &[&str]| {
        let o = Command::new(BIN).args(args).current_dir(&wt).output().unwrap();
        (o.status.code().unwrap(), String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let (code, id) = from_wt(&["task", "add", "p", "--title", "from worktree", "--claim", "src"]);
    assert_eq!(code, 0);
    assert!(r.dir.join("state/tasks").join(format!("{id}.task")).is_file(), "the record is in the main root's state");
    assert!(!wt.join("state").exists());
    let b = r.add("p", "from main", &["src/a.rs"]);
    assert_eq!(from_wt(&["task", "start", &id, "--owner", "wt-agent"]).0, 0);
    assert_eq!(r.start(&b, "main-agent").code, 3, "main sees the worktree's running task");
}
fn lock_path(r: &Root) -> PathBuf {
    r.dir.join("state/tasks/board.lock")
}

/// Hold the board lock the way another process would: an OS lock on an open handle.
fn hold(r: &Root) -> fs::File {
    fs::create_dir_all(r.dir.join("state/tasks")).unwrap();
    let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(r)).unwrap();
    f.lock().unwrap();
    f
}

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
    let r = Root::with_wait("kill", Some(2));
    let a = r.add("p", "a", &["src"]);
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_holder_helper", "--nocapture"])
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
    let t = Instant::now();
    let o = r.start(&a, "x");
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(t.elapsed() < Duration::from_secs(2), "no stale timeout after a kill: took {:?}", t.elapsed());
}

#[test]
fn project_names_are_canonical_whatever_the_case_or_trailing_dot() {
    let r = Root::new("canon");
    fs::create_dir_all(r.dir.join("projects/Crash")).unwrap();
    fs::write(r.dir.join("projects/Crash/Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let a = r.add("crash", "a", &["src"]);
    let b = r.add("CRASH", "b", &["src/a.rs"]);
    let c = r.add("Crash", "c", &["src"]);
    for id in [&a, &b, &c] {
        assert!(r.run(&["task", "show", id]).stdout.contains("project: Crash"), "stored under the folder's real name");
    }
    assert_eq!(r.start(&a, "one").code, 0);
    assert_eq!(r.start(&b, "two").code, 3, "`CRASH` and `crash` are the same project");
    assert_eq!(r.start(&c, "three").code, 3);
    for bad in ["Crash.", "crash ", "Crash..", "crash. "] {
        assert_eq!(r.run(&["task", "add", bad, "--title", "t", "--claim", "src"]).code, 2, "`{bad}`");
        assert_eq!(r.run(&["task", "check", bad, "--claim", "src"]).code, 2, "`{bad}`");
    }
    assert_eq!(r.run(&["task", "check", "cRaSh", "--claim", "src/x"]).code, 3);
    assert_eq!(r.list(&["--project", "CRASH"]).len(), 3);
}

#[test]
fn sixteen_gated_starts_with_a_garbage_lock_and_a_big_board_never_give_two_running() {
    // The reviewer's repro: a garbage lock file, 16 simultaneous starts all claiming `src`, and thousands of records so the
    // critical section is slow. Exactly one runs every round and no start is left blocked (no exit 4).
    const ROUNDS: usize = 8;
    const FILLER: usize = 3000;
    let r = Root::with_wait("race16", Some(30));
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


// ---- MCP ---------------------------------------------------------------------------------

/// One `tools/call` over a fresh `broadsword mcp` server. Returns (isError, text, structured exit_code).
fn mcp_call(r: &Root, name: &str, args: &str) -> (bool, String, i64) {
    let mut child = Command::new(BIN).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#).unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let resp = text.lines().filter_map(|l| parse(l).ok()).find(|v| v.get("id") == Some(&J::Num("2".into()))).unwrap_or_else(|| panic!("no response to {name}: {text}"));
    if let Some(e) = resp.get("error") {
        return (true, e.to_json(), -2); // protocol-level error
    }
    let res = resp.get("result").unwrap();
    let Some(J::Arr(c)) = res.get("content") else { panic!("{}", resp.to_json()) };
    let body = c[0].get("text").and_then(J::str).unwrap().to_string();
    let code = match res.get("structuredContent").and_then(|s| s.get("exit_code")) {
        Some(J::Num(n)) => n.parse().unwrap(),
        _ => -1,
    };
    (matches!(res.get("isError"), Some(J::Bool(true))), body, code)
}

#[test]
fn the_task_tools_work_over_mcp() {
    let r = Root::new("mcp");
    let (err, body, code) = mcp_call(&r, "task_add", r#"{"project":"p","title":"first","claims":["src","docs/"]}"#);
    assert!((err, code) == (false, 0), "{body}");
    let id = body.lines().next().unwrap().trim().to_string();
    assert!(id.starts_with("t-"), "{body}");
    let (_, body, _) = mcp_call(&r, "task_add", r#"{"project":"p","title":"second","claims":["src/a.rs"]}"#);
    let id2 = body.lines().next().unwrap().trim().to_string();
    assert_ne!(id, id2);

    let (err, _, code) = mcp_call(&r, "task_check", r#"{"project":"p","claims":["src/a.rs"]}"#);
    assert_eq!((err, code), (false, 0));
    let (err, body, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{id}","owner":"agent-a"}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    let (err, body, code) = mcp_call(&r, "task_check", r#"{"project":"p","claims":["src/a.rs"]}"#);
    assert_eq!((err, code), (true, 3), "{body}");
    assert!(body.contains(&id) && body.contains("agent-a"), "{body}");
    let (err, body, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{id2}","owner":"agent-b"}}"#));
    assert_eq!((err, code), (true, 3), "{body}");
    assert!(body.contains(&id) && body.contains("agent-a") && body.contains("`src/a.rs`") && body.contains("`src`"), "{body}");

    let (_, body, code) = mcp_call(&r, "task_list", r#"{"status":"running"}"#);
    assert_eq!(code, 0);
    assert!(body.contains(&id) && !body.contains(&id2), "{body}");
    let (_, body, _) = mcp_call(&r, "task_show", &format!(r#"{{"id":"{id}"}}"#));
    assert!(body.contains("status: running") && body.contains("owner: agent-a"), "{body}");

    let (err, body, code) = mcp_call(&r, "task_done", &format!(r#"{{"id":"{id}","note":"all good"}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    let (err, body, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{id2}","owner":"agent-b"}}"#));
    assert_eq!((err, code), (false, 0), "released claims: {body}");
    let (err, _, code) = mcp_call(&r, "task_fail", &format!(r#"{{"id":"{id2}","note":"broke"}}"#));
    assert_eq!((err, code), (false, 0));
    let (_, body, _) = mcp_call(&r, "task_list", r#"{"project":"p"}"#);
    assert!(body.contains("done") && body.contains("failed"), "{body}");
    let (_, _, code) = mcp_call(&r, "task_abandon", &format!(r#"{{"id":"{id2}"}}"#));
    assert_eq!(code, 1, "a failed task cannot be abandoned");
    let (err, body, _) = mcp_call(&r, "task_add", r#"{"project":"p","title":"x","claims":[]}"#);
    assert!(err && body.contains("empty"), "{body}");
    let (err, body, code) = mcp_call(&r, "task_add", r#"{"project":"p","title":"x","claims":["../x"]}"#);
    assert_eq!((err, code), (true, 2), "{body}");
    // the CLI sees what MCP did
    assert_eq!(r.status_of(&id), "done");
    assert_eq!(r.status_of(&id2), "failed");
}

// ---- the race: the done check in miniature -----------------------------------------------

/// Spawn one `task start` per (id, owner), all parked on a go-file, then release them together. Returns their exit codes
/// and stderr.
fn start_together(r: &Root, who: &[(&str, String)], round: usize) -> Vec<(i32, String)> {
    let gate = r.dir.join(format!("scratch/go-{round}"));
    let kids: Vec<Child> = who
        .iter()
        .map(|(id, owner)| r.cmd(&["task", "start", id, "--owner", owner]).env(GATE_ENV, &gate).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(150)); // let every process start and park on the gate
    fs::write(&gate, "go").unwrap();
    kids.into_iter()
        .map(|k| {
            let o = k.wait_with_output().unwrap();
            (o.status.code().unwrap(), String::from_utf8_lossy(&o.stderr).into_owned())
        })
        .collect()
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
    let r = Root::new("race8");
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

/// True if nobody holds the board lock right now.
fn hold_is_free(r: &Root) -> bool {
    let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(r)).unwrap();
    f.try_lock().is_ok()
}

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
