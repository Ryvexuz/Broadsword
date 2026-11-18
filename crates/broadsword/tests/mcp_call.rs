//! End-to-end tests for `tools/call` over `broadsword mcp`: the real binary, talked to over pipes, inside throwaway
//! Broadsword roots under the repo's `scratch/` folder (never the real root, never the real crash sim).

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/json.rs"]
mod j;
use j::{parse, J};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Root {
    dir: PathBuf,
}

impl Root {
    /// `floor_gb`/`reserve_gb`: disk guard; `timeout_s`/`out_kb`: the MCP call limits.
    fn new(name: &str, timeout_s: u64, out_kb: u64) -> Root {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("mcpcall-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("projects")).unwrap();
        let toml = format!(
            "root = \".\"\ndisk_floor_gb = 1\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\ndisk_check_every_seconds = 60\nlog_max_mb = 50\n\
             ram_warn_percent = 90\ncpu_warn_percent = 90\nalert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
             state_dir = \"state\"\ninstall_dir = \"build/bin\"\nmcp_call_timeout_seconds = {timeout_s}\nmcp_output_max_kb = {out_kb}\n\
             checkpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 0\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = 10\nreviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
max_agents = 50
"
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    /// A tiny library crate `name`; `git` makes it a repository with one commit (`run` needs that).
    fn project(&self, name: &str, lib_rs: &str, git: bool) {
        let p = self.dir.join("projects").join(name);
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(p.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n")).unwrap();
        fs::write(p.join("src/lib.rs"), lib_rs).unwrap();
        if git {
            fs::write(p.join(".gitignore"), "target\n").unwrap();
            for args in [&["init", "-q"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"], &["add", "-A"], &["commit", "-q", "-m", "init"]] {
                assert!(Command::new("git").args(args).current_dir(&p).output().unwrap().status.success(), "git {args:?}");
            }
        }
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

const PASSING: &str = "#[cfg(test)] mod t { #[test] fn a() {} #[test] fn b() {} }\n";
const FAILING: &str = "#[cfg(test)] mod t { #[test] fn good() {} #[test] fn bad() { panic!(\"no\"); } }\n";

/// A live `broadsword mcp` process. Every stdout line is kept so purity can be checked at the end.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    rx: Receiver<String>,
    all: Arc<Mutex<Vec<String>>>,
    reader: Option<std::thread::JoinHandle<()>>,
    _root: Root,
}

impl Server {
    fn start(root: Root) -> Server {
        let mut child = Command::new(BIN).arg("mcp").current_dir(&root.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let stdin = child.stdin.take();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let all = Arc::new(Mutex::new(Vec::new()));
        let all2 = Arc::clone(&all);
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                all2.lock().unwrap().push(line.clone());
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Server { child, stdin, rx, all, reader: Some(reader), _root: root }
    }

    fn send(&mut self, line: &str) {
        let s = self.stdin.as_mut().expect("stdin already closed");
        writeln!(s, "{line}").unwrap();
        s.flush().unwrap();
    }

    fn recv(&self, secs: u64) -> Option<J> {
        match self.rx.recv_timeout(Duration::from_secs(secs)) {
            Ok(l) => Some(parse(&l).unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {l}"))),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => panic!("server closed stdout"),
        }
    }

    /// Send a request and wait for the response with the same id.
    fn rpc(&mut self, id: i64, method: &str, params: &str) -> J {
        self.send(&format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#));
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            let left = deadline.saturating_duration_since(Instant::now()).as_secs().max(1);
            let r = self.recv(left).unwrap_or_else(|| panic!("no response to {method} {id}"));
            if r.get("id") == Some(&J::Num(id.to_string())) {
                return r;
            }
        }
    }

    fn init(&mut self, version: &str) -> J {
        let r = self.rpc(1, "initialize", &format!(r#"{{"protocolVersion":"{version}","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}"#));
        self.send(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        r
    }

    /// `tools/call`; the whole response (result or error).
    fn tool(&mut self, id: i64, name: &str, args: &str) -> J {
        self.rpc(id, "tools/call", &format!(r#"{{"name":"{name}","arguments":{args}}}"#))
    }

    /// Close stdin, wait for exit, and check that every stdout line was a JSON-RPC message. Returns the exit code.
    fn finish(&mut self) -> i32 {
        self.stdin = None;
        let code = self.child.wait().unwrap().code().unwrap();
        if let Some(r) = self.reader.take() {
            r.join().unwrap();
        }
        for l in self.all.lock().unwrap().iter() {
            let v = parse(l).unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {l}"));
            assert_eq!(v.get("jsonrpc").and_then(J::str), Some("2.0"), "{l}");
        }
        code
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn result(r: &J) -> &J {
    r.get("result").unwrap_or_else(|| panic!("expected a result, got {}", r.to_json()))
}

fn is_error(r: &J) -> bool {
    matches!(result(r).get("isError"), Some(J::Bool(true)))
}

fn text(r: &J) -> String {
    let Some(J::Arr(c)) = result(r).get("content") else { panic!("no content in {}", r.to_json()) };
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].get("type").and_then(J::str), Some("text"));
    c[0].get("text").and_then(J::str).unwrap().to_string()
}

fn structured(r: &J) -> &J {
    result(r).get("structuredContent").unwrap_or_else(|| panic!("no structuredContent in {}", r.to_json()))
}

fn exit_code(r: &J) -> String {
    match structured(r).get("exit_code") {
        Some(J::Num(n)) => n.clone(),
        other => panic!("exit_code: {other:?}"),
    }
}

fn err_code(r: &J) -> String {
    match r.get("error").and_then(|e| e.get("code")) {
        Some(J::Num(n)) => n.clone(),
        other => panic!("expected a JSON-RPC error, got {other:?} in {}", r.to_json()),
    }
}

/// A slow command for `run`: it outlives any test timeout unless it is killed.
fn slow_command() -> &'static str {
    if cfg!(windows) {
        r#"["ping","-n","40","127.0.0.1"]"#
    } else {
        r#"["sleep","40"]"#
    }
}

#[test]
fn status_runs_through_mcp() {
    let mut s = Server::start(Root::new("status", 600, 1024));
    s.init("2025-06-18");
    let r = s.tool(2, "status", "{}");
    assert!(!is_error(&r), "{}", r.to_json());
    let t = text(&r);
    assert!(t.contains("disk:") && t.contains("paused: no") && t.ends_with("exit code: 0"), "{t}");
    assert_eq!(exit_code(&r), "0");
    assert_eq!(structured(&r).get("truncated"), Some(&J::Bool(false)));
    assert!(structured(&r).get("stdout").and_then(J::str).unwrap().contains("ram:"));
    assert_eq!(structured(&r).get("stderr").and_then(J::str), Some(""));
    assert_eq!(s.finish(), 0);
}

#[test]
fn projects_runs_in_the_main_root() {
    let root = Root::new("projects", 600, 1024);
    root.project("alpha", PASSING, false);
    let mut s = Server::start(root);
    s.init("2025-06-18");
    let r = s.tool(2, "projects", "{}");
    assert!(!is_error(&r));
    let t = text(&r);
    assert!(t.contains("broadsword\t") && t.contains("alpha\t"), "{t}");
    assert_eq!(s.finish(), 0);
}

#[test]
fn check_passes_and_fails_with_the_job_exit_codes() {
    let root = Root::new("check", 600, 1024);
    root.project("good", PASSING, false);
    root.project("bad", FAILING, false);
    let mut s = Server::start(root);
    s.init("2025-06-18");
    let ok = s.tool(2, "check", r#"{"project":"good"}"#);
    assert!(!is_error(&ok), "{}", ok.to_json());
    assert_eq!(exit_code(&ok), "0");
    assert!(text(&ok).contains("test: ok") && text(&ok).ends_with("exit code: 0"), "{}", text(&ok));
    let bad = s.tool(3, "check", r#"{"project":"bad"}"#);
    assert!(is_error(&bad), "a failing check is isError");
    assert_eq!(exit_code(&bad), "1");
    let t = text(&bad);
    assert!(t.contains("test: FAILED") && t.contains("stderr:") && t.ends_with("exit code: 1"), "{t}");
    let unknown = s.tool(4, "check", r#"{"project":"nope"}"#);
    assert!(is_error(&unknown));
    assert_eq!(exit_code(&unknown), "2", "an unknown project is exit 2, as documented");
    assert_eq!(s.finish(), 0);
}

#[test]
fn unknown_tool_is_a_protocol_error_and_bad_arguments_are_tool_errors() {
    let mut s = Server::start(Root::new("badargs", 600, 1024));
    s.init("2025-06-18");
    let r = s.tool(2, "nope", "{}");
    assert_eq!(err_code(&r), "-32602");
    assert!(r.get("error").and_then(|e| e.get("message")).and_then(J::str).unwrap().contains("Unknown tool: nope"));
    assert_eq!(err_code(&s.rpc(3, "tools/call", r#"{"arguments":{}}"#)), "-32602");
    for (id, name, args, needle) in [
        (4, "build", "{}", "missing required argument `project`"),
        (5, "build", r#"{"project":7}"#, "`project` must be a string, got a number"),
        (6, "build", r#"{"project":"p","bogus":true}"#, "unknown argument `bogus`"),
        (7, "status", r#"{"x":1}"#, "takes no arguments"),
        (8, "run", r#"{"project":"p","command":[]}"#, "must not be an empty array"),
        (9, "resume", r#"{"force":"yes"}"#, "`force` must be a boolean"),
    ] {
        let r = s.tool(id, name, args);
        assert!(is_error(&r), "{name} {args}: {}", r.to_json());
        assert!(text(&r).contains(needle), "{name} {args}: {}", text(&r));
        assert!(result(&r).get("structuredContent").is_none(), "nothing ran, so there is no exit code to report");
    }
    assert_eq!(s.finish(), 0);
}

#[test]
fn watch_is_refused_and_nothing_runs() {
    let root = Root::new("watch", 600, 1024);
    let dir = root.dir.clone();
    let mut s = Server::start(root);
    s.init("2025-06-18");
    let r = s.tool(2, "watch", "{}");
    assert!(is_error(&r));
    let t = text(&r);
    assert!(t.contains("not available over MCP") && t.contains("background") && t.contains("mcp_callable"), "{t}");
    // tools/list lets a caller tell in advance
    let list = s.rpc(3, "tools/list", "{}");
    let Some(J::Arr(tools)) = result(&list).get("tools") else { panic!() };
    for t in tools {
        let name = t.get("name").and_then(J::str).unwrap();
        let flag = t.get("_meta").and_then(|m| m.get("broadsword")).and_then(|b| b.get("mcp_callable"));
        assert_eq!(flag, Some(&J::Bool(name != "watch")), "{name}");
        if name == "watch" {
            assert!(t.get("description").and_then(J::str).unwrap().contains("NOT AVAILABLE OVER MCP"));
        }
    }
    assert!(!dir.join("scratch/logs/monitor.log").exists(), "watch must not have started");
    assert_eq!(s.finish(), 0);
}

#[test]
fn a_call_that_outlives_the_timeout_is_killed() {
    let root = Root::new("timeout", 2, 1024);
    root.project("p", PASSING, true);
    let mut s = Server::start(root);
    s.init("2025-06-18");
    let t0 = Instant::now();
    let r = s.tool(2, "run", &format!(r#"{{"project":"p","command":{}}}"#, slow_command()));
    let took = t0.elapsed();
    assert!(took < Duration::from_secs(30), "the 40 s command must have been killed, took {took:?}");
    assert!(is_error(&r));
    let t = text(&r);
    assert!(t.contains("timed out after 2 s") && t.contains("exit code: none (killed)"), "{t}");
    assert_eq!(exit_code(&r), "-1");
    // the server is still alive and serving
    assert_eq!(s.rpc(3, "ping", "{}").get("result"), Some(&J::Obj(vec![])));
    assert_eq!(s.finish(), 0);
}

#[test]
fn long_output_is_cut_to_its_tail_and_says_so() {
    let mut s = Server::start(Root::new("trunc", 600, 1));
    s.init("2025-06-18");
    let r = s.tool(2, "tools", "{}");
    assert!(!is_error(&r));
    assert_eq!(structured(&r).get("truncated"), Some(&J::Bool(true)));
    let out = structured(&r).get("stdout").and_then(J::str).unwrap();
    assert!(out.len() <= 1024 && out.len() > 100, "kept {} bytes", out.len());
    assert!(out.ends_with("prints the registry as JSON.\n"), "the END of the output is kept: {out:?}");
    let t = text(&r);
    assert!(t.starts_with("[stdout truncated: only its last 1 KB are shown]\n") && t.ends_with("exit code: 0"), "{t}");
    // and it is complete when it fits
    let mut s2 = Server::start(Root::new("trunc2", 600, 1024));
    s2.init("2025-06-18");
    let full = s2.tool(2, "tools", "{}");
    assert_eq!(structured(&full).get("truncated"), Some(&J::Bool(false)));
    assert!(structured(&full).get("stdout").and_then(J::str).unwrap().starts_with("status "));
    assert_eq!(s.finish() + s2.finish(), 0);
}

#[test]
fn a_paused_root_refuses_check_with_exit_3_through_mcp() {
    let root = Root::new("paused", 600, 1024);
    root.project("good", PASSING, false);
    fs::write(root.dir.join("broadsword.paused"), "time: now\nfree: 0.5 GB\nreason: test\n").unwrap();
    let dir = root.dir.clone();
    let mut s = Server::start(root);
    s.init("2025-06-18");
    let r = s.tool(2, "check", r#"{"project":"good"}"#);
    assert!(is_error(&r));
    assert_eq!(exit_code(&r), "3");
    let t = text(&r);
    assert!(t.contains("paused") && t.ends_with("exit code: 3"), "{t}");
    assert!(!dir.join("build").join("debug").exists(), "cargo must not have started");
    // the read-only tools and resume are still available while paused (the job guard only stops jobs)
    let st = s.tool(3, "status", "{}");
    assert!(text(&st).contains("paused: YES"), "{}", text(&st));
    assert_eq!(s.finish(), 0);
}

#[test]
fn structured_content_and_output_schema_follow_the_protocol_version() {
    for v in ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"] {
        let mut s = Server::start(Root::new("ver", 600, 1024));
        let init = s.init(v);
        assert_eq!(result(&init).get("protocolVersion").and_then(J::str), Some(v));
        let newer = v >= "2025-06-18";
        let r = s.tool(2, "projects", "{}");
        assert!(!is_error(&r));
        assert_eq!(result(&r).get("structuredContent").is_some(), newer, "{v}: structuredContent");
        assert!(text(&r).ends_with("exit code: 0"), "{v}: the text form is always there");
        let list = s.rpc(3, "tools/list", "{}");
        let Some(J::Arr(tools)) = result(&list).get("tools") else { panic!() };
        assert_eq!(tools.len(), 32);
        assert!(tools.iter().all(|t| t.get("outputSchema").is_some() == newer), "{v}: outputSchema");
        assert_eq!(s.finish(), 0, "{v}");
    }
}

#[test]
fn ping_is_answered_and_a_cancel_kills_the_running_call() {
    let root = Root::new("cancel", 600, 1024);
    root.project("p", PASSING, true);
    let mut s = Server::start(root);
    s.init("2025-06-18");
    let t0 = Instant::now();
    s.send(&format!(r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"run","arguments":{{"project":"p","command":{}}}}}}}"#, slow_command()));
    // other requests wait their turn; ping does not
    s.send(r#"{"jsonrpc":"2.0","id":8,"method":"tools/list"}"#);
    s.send(r#"{"jsonrpc":"2.0","id":9,"method":"ping"}"#);
    let first = s.recv(20).expect("ping must be answered while the call runs");
    assert_eq!(first.get("id"), Some(&J::Num("9".into())), "the first thing back is the ping, not the call or the list: {}", first.to_json());
    assert_eq!(first.get("result"), Some(&J::Obj(vec![])));
    assert!(s.recv(1).is_none(), "tools/list waits for the call to end, and the call is still running");
    // cancel the call: it is killed and, as the spec says, gets no response; the queued list is then served
    s.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7,"reason":"test"}}"#);
    let next = s.recv(30).expect("the queued tools/list is answered after the cancel");
    assert_eq!(next.get("id"), Some(&J::Num("8".into())), "{}", next.to_json());
    assert!(result(&next).get("tools").is_some());
    assert!(t0.elapsed() < Duration::from_secs(35), "the 40 s command must have been killed");
    assert_eq!(s.rpc(10, "ping", "{}").get("result"), Some(&J::Obj(vec![])));
    // nobody ever answered request 7
    assert!(s.all.lock().unwrap().iter().all(|l| parse(l).unwrap().get("id") != Some(&J::Num("7".into()))), "a cancelled call gets no response");
    assert_eq!(s.finish(), 0);
}

#[test]
fn closing_stdin_during_a_call_still_delivers_its_response() {
    let mut s = Server::start(Root::new("eof", 600, 1024));
    s.send(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
    s.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"status","arguments":{}}}"#);
    s.send(r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#);
    s.stdin = None; // the client hangs up right after asking
    let ids: Vec<String> = (0..3).map(|_| s.recv(30).expect("a response").get("id").map(J::to_json).unwrap()).collect();
    // ping is answered during the call, so it may come first; the call response must come, and the queue is drained
    assert!(ids.contains(&"2".to_string()) && ids.contains(&"3".to_string()) && ids.contains(&"1".to_string()), "{ids:?}");
    assert_eq!(s.finish(), 0);
}

#[test]
fn the_pause_hook_lets_read_only_mcp_tools_through() {
    let root = Root::new("hook", 600, 1024);
    fs::write(root.dir.join("broadsword.paused"), "time: now\nfree: 0.5 GB\nreason: test\n").unwrap();
    let hook = |tool: &str| {
        let mut c = Command::new(BIN).args(["hook", "pre-tool"]).current_dir(&root.dir).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        c.stdin.take().unwrap().write_all(format!(r#"{{"tool_name":"{tool}","tool_input":{{}}}}"#).as_bytes()).unwrap();
        c.wait().unwrap().code().unwrap()
    };
    for ok in ["status", "tools", "projects", "jobs", "resume", "cleanup"] {
        assert_eq!(hook(&format!("mcp__broadsword__{ok}")), 0, "{ok} is allowed while paused");
    }
    for blocked in ["build", "test", "check", "run", "recover", "abandon", "watch", "install"] {
        assert_eq!(hook(&format!("mcp__broadsword__{blocked}")), 2, "{blocked} is blocked while paused");
    }
    assert_eq!(hook("mcp__other__status"), 2);
}
