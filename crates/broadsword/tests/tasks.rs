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

mod support;
use support::bin;
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
        let toml = support::config(&[("mcp_call_timeout_seconds", "600"), ("task_lock_wait_seconds", &wait.unwrap_or(10).to_string())]);
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(bin());
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
        let o = Command::new(bin()).args(args).current_dir(&wt).output().unwrap();
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


// ---- MCP ---------------------------------------------------------------------------------

/// One `tools/call` over a fresh `broadsword mcp` server. Returns (isError, text, structured exit_code).
fn mcp_call(r: &Root, name: &str, args: &str) -> (bool, String, i64) {
    let mut child = Command::new(bin()).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
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

/// True if nobody holds the board lock right now.
fn hold_is_free(r: &Root) -> bool {
    let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(r)).unwrap();
    f.try_lock().is_ok()
}

#[path = "tasks/archive.rs"]
mod archive;
#[path = "tasks/caps.rs"]
mod caps;
#[path = "tasks/claims.rs"]
mod claims;
#[path = "tasks/lock.rs"]
mod lock;
