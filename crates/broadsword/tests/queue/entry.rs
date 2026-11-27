use super::*;

use std::io::{BufRead, BufReader, Write};
use std::sync::mpsc::{self, Receiver};

/// The `*.entry` files in the queue folder (empty when the folder does not exist).
fn entry_files(r: &Root) -> Vec<String> {
    let Ok(rd) = fs::read_dir(r.dir.join("state/queue")) else { return Vec::new() };
    rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".entry")).collect()
}

#[test]
fn a_haiku_coder_at_low_risk_is_refused_by_queue_add_and_nothing_is_written() {
    let r = Root::new("haikulow", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    for coder in ["haiku", "Haiku", "claude-haiku-4-5-20251001"] {
        let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", coder]);
        assert_eq!(o.code, 2, "{coder}: {}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains("has no allowed reviewer at --risk low; queue it with --risk high"), "{coder}: {}", o.stderr);
        assert!(entry_files(&r).is_empty(), "{coder}: {:?}", entry_files(&r));
    }
    // no lock is left held: the same task queues at high risk straight away
    let e = r.add(&id, "high", "haiku");
    assert_eq!(entry_files(&r).len(), 1);
    assert_eq!(r.status(&e), "queued");
}

#[test]
fn sonnet_at_low_risk_is_still_accepted() {
    let r = Root::new("sonnetlow", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    assert_eq!(r.status(&e), "queued");
    assert_eq!(entry_files(&r).len(), 1);
}

#[test]
fn queue_add_refuses_what_cannot_be_merged_and_is_documented_in_exit_codes() {
    let r = Root::new("addrules", 30);
    // no such task, bad risk, empty coder
    assert_eq!(r.run(&["queue", "add", "nope", "--risk", "low", "--coder", "sonnet"]).code, 2);
    let id = r.started("src");
    assert_eq!(r.run(&["queue", "add", &id, "--risk", "medium", "--coder", "sonnet"]).code, 2);
    assert_eq!(r.run(&["queue", "add", &id, "--risk", "low", "--coder", " "]).code, 2);
    // missing required flags do not parse as a command at all (usage, exit 64)
    assert_eq!(r.run(&["queue", "add", &id, "--risk", "low"]).code, 64);
    // no commit beyond the base
    let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("no commit beyond its base"), "{}", o.stderr);
    // an untracked file blocks
    r.commit(&r.wt(&id), "src/a.txt", "a\n", "task: a");
    fs::write(r.wt(&id).join("loose.txt"), "x").unwrap();
    let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]);
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains("not clean") && o.stderr.contains("loose.txt"), "{}", o.stderr);
    fs::remove_file(r.wt(&id).join("loose.txt")).unwrap();
    // (an ignored file does not block: see `an_ignored_file_in_the_worktree_does_not_block_the_queue...`)
    // a brief that is a secrets file, or too big, or empty
    fs::write(r.dir.join(".env"), "OPENAI_API_KEY=sk-test-abcdefghij\n").unwrap();
    let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet", "--brief", r.dir.join(".env").to_str().unwrap()]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains(".env"), "{}", o.stderr);
    fs::write(r.dir.join("big.md"), "x".repeat(33 * 1024)).unwrap();
    assert_eq!(r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet", "--brief", r.dir.join("big.md").to_str().unwrap()]).code, 2);
    fs::write(r.dir.join("empty.md"), "  \n").unwrap();
    assert_eq!(r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet", "--brief", r.dir.join("empty.md").to_str().unwrap()]).code, 2);
    // a real brief is stored, then a second add of the same task is refused while the first is live
    fs::write(r.dir.join("brief.md"), "Add file a. Done check: a.txt exists in src.\n").unwrap();
    let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet", "--brief", r.dir.join("brief.md").to_str().unwrap()]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    let e = o.stdout.lines().next().unwrap().to_string();
    assert!(r.show(&e).contains("Done check: a.txt exists in src."));
    let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]);
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains(&e) && o.stderr.contains("already in the queue"), "{}", o.stderr);
    // an open (not running) task cannot be queued
    let o = r.run(&["task", "add", "p", "--title", "later", "--claim", "docs"]);
    let open = o.stdout.trim().to_string();
    assert_eq!(r.run(&["queue", "add", &open, "--risk", "low", "--coder", "sonnet"]).code, 3);
    // cancel: queued entries drop, again is fine, finished ones are refused, unknown ones are bad input
    assert_eq!(r.run(&["queue", "cancel", &e]).code, 0);
    assert_eq!(r.status(&e), "cancelled");
    assert_eq!(r.run(&["queue", "cancel", &e]).code, 0, "cancelling twice is not an error");
    assert_eq!(r.run(&["queue", "cancel", "q-nope"]).code, 2);
    assert_eq!(r.run(&["queue", "show", "q-nope"]).code, 2);
    assert_eq!(r.run(&["queue", "run", "--entry", &e]).code, 2, "a cancelled entry cannot be run");
    assert_eq!(r.task_status(&id), "running", "cancel touches nothing else");
    assert!(r.wt(&id).exists());
    // the brief file's text reaches the reviewer
    let e2 = {
        let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet", "--brief", r.dir.join("brief.md").to_str().unwrap()]);
        assert_eq!(o.code, 0, "cancelled entries do not block a new add: {}", o.stderr);
        o.stdout.lines().next().unwrap().to_string()
    };
    assert_eq!(r.run(&["queue", "run"]).code, 0);
    assert!(fs::read_to_string(r.fake_out.join("stdin.txt")).unwrap().contains("Done check: a.txt exists in src."));
    assert_eq!(r.run(&["queue", "cancel", &e2]).code, 3, "a merged entry cannot be cancelled");
}

#[test]
fn a_task_with_a_blocked_or_stale_entry_cannot_be_queued_again() {
    let r = Root::new("twice", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let e = r.add(&id, "low", "sonnet");
    fs::write(r.p().join("junk.txt"), "x\n").unwrap();
    assert_eq!(r.run(&["queue", "run"]).code, 3);
    assert_eq!(r.status(&e), "blocked");
    let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains(&e) && o.stderr.contains("blocked"), "the existing entry is named: {}", o.stderr);
    r.edit_entry(&e, &[("status", "stale")]);
    let o = r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains(&e) && o.stderr.contains("stale"), "{}", o.stderr);
    assert_eq!(r.run(&["queue", "cancel", &e]).code, 0);
    assert_eq!(r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet"]).code, 0, "a cancelled entry does not block");
}

// ---- the MCP tools/call path ----------------------------------------------------------------

struct Mcp {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    rx: Receiver<String>,
}

impl Mcp {
    fn start(r: &Root) -> Mcp {
        let mut child = r.command(&["mcp"], &[]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let stdin = child.stdin.take();
        Mcp { child, stdin, rx }
    }

    fn rpc(&mut self, id: i64, method: &str, params: &str) -> J {
        let s = self.stdin.as_mut().unwrap();
        writeln!(s, r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#).unwrap();
        s.flush().unwrap();
        let end = Instant::now() + ceiling(300);
        loop {
            let line = self.rx.recv_timeout(end.saturating_duration_since(Instant::now())).expect("no response");
            let v = parse(&line).unwrap();
            if v.get("id") == Some(&J::Num(id.to_string())) {
                return v;
            }
        }
    }

    /// `tools/call`: (exit_code, stdout, isError)
    fn call(&mut self, id: i64, name: &str, args: &str) -> (String, String, bool) {
        let v = self.rpc(id, "tools/call", &format!(r#"{{"name":"{name}","arguments":{args}}}"#));
        let res = v.get("result").unwrap_or_else(|| panic!("no result: {}", v.to_json()));
        let sc = res.get("structuredContent").unwrap();
        let code = match sc.get("exit_code") {
            Some(J::Num(n)) => n.clone(),
            other => panic!("exit_code {other:?}"),
        };
        (code, sc.get("stdout").and_then(J::str).unwrap().to_string(), matches!(res.get("isError"), Some(J::Bool(true))))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn queue_add_list_and_run_work_over_mcp_tools_call() {
    let r = Root::new("mcp", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/new.txt", "x\n", "task: new file");
    let mut m = Mcp::start(&r);
    let init = m.rpc(1, "initialize", r#"{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}"#);
    assert!(init.get("result").is_some());
    let list = m.rpc(2, "tools/list", "{}");
    let Some(J::Arr(tools)) = list.get("result").and_then(|x| x.get("tools")) else { panic!("{}", list.to_json()) };
    let find = |n: &str| tools.iter().find(|t| t.get("name").and_then(J::str) == Some(n)).unwrap_or_else(|| panic!("{n} is not listed"));
    for n in ["queue_add", "queue_list", "queue_show", "queue_run", "queue_cancel"] {
        let t = find(n);
        assert!(t.get("description").and_then(J::str).unwrap().len() > 100);
    }
    let run = find("queue_run");
    let ann = run.get("annotations").unwrap();
    assert_eq!((ann.get("destructiveHint"), ann.get("idempotentHint"), ann.get("readOnlyHint")), (Some(&J::Bool(true)), Some(&J::Bool(false)), Some(&J::Bool(false))));
    assert_eq!(find("queue_list").get("annotations").unwrap().get("readOnlyHint"), Some(&J::Bool(true)));
    let desc = run.get("description").and_then(J::str).unwrap();
    for needle in ["--ff-only", "CALLS A MODEL", "BROADSWORD_REVIEWER_CLI", "never uses --force", "NOT IDEMPOTENT"] {
        assert!(desc.to_lowercase().contains(&needle.to_lowercase()), "{needle}");
    }

    let (code, out, err) = m.call(3, "queue_add", &format!(r#"{{"task":"{id}","risk":"low","coder":"sonnet"}}"#));
    assert_eq!((code.as_str(), err), ("0", false), "{out}");
    let entry = out.lines().next().unwrap().to_string();
    let (code, out, _) = m.call(4, "queue_list", "{}");
    assert_eq!(code, "0");
    assert!(out.contains(&entry) && out.contains("queued"), "{out}");
    let (code, out, err) = m.call(5, "queue_add", &format!(r#"{{"task":"{id}","risk":"low","coder":"sonnet"}}"#));
    assert_eq!((code.as_str(), err), ("3", true), "a second add is refused with the documented code: {out}");
    let (code, out, err) = m.call(6, "queue_run", "{}");
    assert_eq!((code.as_str(), err), ("0", false), "{out}");
    assert!(out.contains("merged"), "{out}");
    let (code, out, _) = m.call(7, "queue_list", "{}");
    assert_eq!(code, "0");
    assert!(out.contains(&entry) && out.contains("merged"), "{out}");
    let (code, out, _) = m.call(8, "queue_show", &format!(r#"{{"entry":"{entry}"}}"#));
    assert_eq!(code, "0");
    assert!(out.contains("status: merged"), "{out}");
    let (code, _, err) = m.call(9, "queue_show", r#"{"entry":"q-nope"}"#);
    assert_eq!((code.as_str(), err), ("2", true));
    assert!(r.p().join("src/new.txt").exists());
}

#[test]
fn queue_add_refuses_a_brief_outside_the_root_in_state_or_build_a_secret_file_or_a_link_out() {
    let r = Root::new("briefs", 30);
    let id = r.started("src");
    r.commit(&r.wt(&id), "src/a.txt", "a\n", "task: a");
    let add = |b: &str| r.run(&["queue", "add", &id, "--risk", "low", "--coder", "sonnet", "--brief", b]);
    // outside the root, by absolute path and by `..`
    let outside = r.dir.parent().unwrap().join(format!("outside-brief-{}.md", std::process::id()));
    fs::write(&outside, "outside brief\n").unwrap();
    let o = add(outside.to_str().unwrap());
    assert_eq!(o.code, 2, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("outside the project root"), "{}", o.stderr);
    let dotdot = format!("..{}{}", std::path::MAIN_SEPARATOR, outside.file_name().unwrap().to_str().unwrap());
    let o = add(&dotdot);
    assert_eq!(o.code, 2, "{}{}", o.stdout, o.stderr);
    // inside state_dir and build_dir
    fs::create_dir_all(r.dir.join("build")).unwrap();
    fs::write(r.dir.join("build/b.md"), "build brief\n").unwrap();
    let o = add(r.dir.join("build/b.md").to_str().unwrap());
    assert_eq!(o.code, 2, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("build_dir"), "{}", o.stderr);
    fs::create_dir_all(r.dir.join("state")).unwrap();
    fs::write(r.dir.join("state/s.md"), "state brief\n").unwrap();
    let o = add(r.dir.join("state/s.md").to_str().unwrap());
    assert_eq!(o.code, 2, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("state_dir"), "{}", o.stderr);
    // a secret file, by name
    fs::write(r.dir.join(".env.local"), "K=abcdefghij\n").unwrap();
    let o = add(r.dir.join(".env.local").to_str().unwrap());
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains(".env"), "{}", o.stderr);
    // a directory junction (or symlink) inside the root pointing outside: the brief resolves outside and is refused
    let target = r.dir.parent().unwrap().join(format!("outside-dir-{}", std::process::id()));
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("j.md"), "junction brief\n").unwrap();
    let link = r.dir.join("linkdir");
    let made = if cfg!(windows) {
        Command::new("cmd").args(["/C", "mklink", "/J", link.to_str().unwrap(), target.to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).status().map(|s| s.success()).unwrap_or(false)
    } else {
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &link).is_ok()
        }
        #[cfg(not(unix))]
        {
            false
        }
    };
    if made {
        let o = add(link.join("j.md").to_str().unwrap());
        assert_eq!(o.code, 2, "{}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains("outside the project root"), "{}", o.stderr);
        // remove the junction itself only, never the target's contents through it
        let _ = if cfg!(windows) { fs::remove_dir(&link) } else { fs::remove_file(&link) };
    } else {
        eprintln!("NOTE: could not create a junction or symlink here; that case was not tested");
    }
    let _ = fs::remove_dir_all(&target);
    let _ = fs::remove_file(&outside);
    // nothing was queued by any refusal, and a brief inside the root still works
    fs::write(r.dir.join("ok.md"), "fine brief\n").unwrap();
    assert_eq!(add(r.dir.join("ok.md").to_str().unwrap()).code, 0);
}
