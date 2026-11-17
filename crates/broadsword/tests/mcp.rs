//! End-to-end tests for `broadsword mcp`: spawn the real binary and talk to it over pipes.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");

fn spawn() -> Child {
    // Run from the crate dir (inside the repo) so broadsword.toml is found and the log goes to the repo's scratch/.
    Command::new(BIN).arg("mcp").current_dir(env!("CARGO_MANIFEST_DIR")).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap()
}

/// Send `lines`, close stdin, and return (stdout lines, exit code).
fn session(lines: &[&str]) -> (Vec<String>, i32) {
    let mut child = spawn();
    let mut stdin = child.stdin.take().unwrap();
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    (text.lines().map(str::to_string).collect(), out.status.code().unwrap())
}

// Reuse the crate's own parser (std-only) by path; the binary crate has no lib target.
#[allow(dead_code)]
#[path = "../src/json.rs"]
mod j;
use j::{parse, J};

fn num(v: &J) -> String {
    match v {
        J::Num(n) => n.clone(),
        other => panic!("not a number: {other:?}"),
    }
}

const INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#;

#[test]
fn initialize_echoes_a_supported_version() {
    for v in ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"] {
        let req = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{v}"}}}}"#);
        let (out, code) = session(&[&req]);
        assert_eq!(code, 0);
        assert_eq!(out.len(), 1);
        let r = parse(&out[0]).unwrap();
        let res = r.get("result").unwrap();
        assert_eq!(res.get("protocolVersion").and_then(J::str), Some(v));
        assert_eq!(res.get("capabilities").unwrap().to_json(), r#"{"tools":{"listChanged":false}}"#);
        let info = res.get("serverInfo").unwrap();
        assert_eq!(info.get("name").and_then(J::str), Some("broadsword"));
        assert_eq!(info.get("version").and_then(J::str), Some(env!("CARGO_PKG_VERSION")));
        assert!(res.get("instructions").and_then(J::str).unwrap().contains("Broadsword"));
    }
}

#[test]
fn initialize_falls_back_to_the_newest_version() {
    let (out, _) = session(&[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#]);
    let r = parse(&out[0]).unwrap();
    assert_eq!(r.get("result").and_then(|x| x.get("protocolVersion")).and_then(J::str), Some("2025-11-25"));
}

#[test]
fn initialized_notification_gets_no_response() {
    let (out, code) = session(&[r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, r#"{"jsonrpc":"2.0","method":"anything/else","params":{}}"#]);
    assert_eq!((out.len(), code), (0, 0), "stdout: {out:?}");
}

#[test]
fn ping_returns_an_empty_object() {
    let (out, _) = session(&[r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#]);
    assert_eq!(out, [r#"{"jsonrpc":"2.0","id":7,"result":{}}"#]);
}

#[test]
fn tools_list_has_every_tool_with_the_same_schemas_as_tools_json() {
    let cli = Command::new(BIN).args(["tools", "--json"]).current_dir(env!("CARGO_MANIFEST_DIR")).output().unwrap();
    assert!(cli.status.success());
    let J::Arr(cli_tools) = parse(&String::from_utf8(cli.stdout).unwrap()).unwrap() else { panic!("tools --json is an array") };
    let (out, code) = session(&[INIT, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#]);
    assert_eq!((out.len(), code), (2, 0));
    let r = parse(&out[1]).unwrap();
    let res = r.get("result").unwrap();
    assert_eq!(res.get("nextCursor"), None);
    let Some(J::Arr(mcp_tools)) = res.get("tools") else { panic!("result.tools array") };
    assert_eq!(mcp_tools.len(), 13);
    assert_eq!(mcp_tools, &cli_tools, "tools/list must equal `tools --json` exactly");
    for t in mcp_tools {
        assert!(t.get("name").and_then(J::str).is_some());
        assert!(t.get("description").and_then(J::str).is_some());
        assert_eq!(t.get("inputSchema").and_then(|s| s.get("type")).and_then(J::str), Some("object"));
    }
    let names: Vec<&str> = mcp_tools.iter().filter_map(|t| t.get("name").and_then(J::str)).collect();
    assert!(names.contains(&"status") && names.contains(&"tools") && !names.contains(&"mcp"), "{names:?}");
}

#[test]
fn tools_call_is_not_yet_implemented() {
    let (out, _) = session(&[r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"status","arguments":{}}}"#]);
    let r = parse(&out[0]).unwrap();
    let e = r.get("error").unwrap();
    assert_eq!(num(e.get("code").unwrap()), "-32601");
    assert!(e.get("message").and_then(J::str).unwrap().contains("milestone 2"));
    assert_eq!(r.get("result"), None);
}

#[test]
fn unknown_method_and_malformed_input() {
    let (out, code) = session(&[
        r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#,
        "this is not json",
        r#"{"id":2,"method":"ping"}"#,
        r#"[{"jsonrpc":"2.0","id":3,"method":"ping"}]"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"ping"}"#,
    ]);
    assert_eq!(code, 0);
    let r: Vec<J> = out.iter().map(|l| parse(l).unwrap()).collect();
    assert_eq!(r.len(), 5);
    let code_of = |v: &J| num(v.get("error").and_then(|e| e.get("code")).unwrap());
    assert_eq!(code_of(&r[0]), "-32601");
    assert_eq!((code_of(&r[1]).as_str(), r[1].get("id")), ("-32700", Some(&J::Null)));
    assert_eq!(code_of(&r[2]), "-32600");
    assert_eq!((code_of(&r[3]).as_str(), r[3].get("id")), ("-32600", Some(&J::Null)));
    // still serving after every error
    assert_eq!(r[4].get("result"), Some(&J::Obj(vec![])));
}

#[test]
fn string_and_number_ids_are_echoed_exactly() {
    let (out, _) = session(&[
        r#"{"jsonrpc":"2.0","id":"abc-1","method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":42,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":1.50,"method":"ping"}"#,
        r#"{"jsonrpc":"2.0","id":"\u00e9\ud83d\ude00","method":"ping"}"#,
    ]);
    assert_eq!(out[0], r#"{"jsonrpc":"2.0","id":"abc-1","result":{}}"#);
    assert_eq!(out[1], r#"{"jsonrpc":"2.0","id":42,"result":{}}"#);
    assert_eq!(out[2], r#"{"jsonrpc":"2.0","id":1.50,"result":{}}"#);
    assert_eq!(parse(&out[3]).unwrap().get("id").and_then(J::str), Some("\u{e9}\u{1F600}"));
}

#[test]
fn several_requests_on_one_live_connection() {
    // Interactive: write one request, read its response, repeat, on the same process.
    let mut child = spawn();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for (i, req) in [INIT, r#"{"jsonrpc":"2.0","id":2,"method":"ping"}"#, r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#, r#"{"jsonrpc":"2.0","id":"four","method":"ping"}"#].iter().enumerate() {
        writeln!(stdin, "{req}").unwrap();
        stdin.flush().unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let r = parse(line.trim_end()).unwrap_or_else(|e| panic!("response {i} not JSON ({e}): {line}"));
        assert!(r.get("result").is_some(), "{line}");
    }
    drop(stdin);
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn eof_exits_zero_and_stdout_is_only_json_lines() {
    let (out, code) = session(&[INIT, "garbage", r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, r#"{"jsonrpc":"2.0","id":3,"method":"tools/call"}"#, "[]"]);
    assert_eq!(code, 0);
    assert_eq!(out.len(), 5);
    for l in &out {
        let v = parse(l).unwrap_or_else(|e| panic!("stdout line is not JSON ({e}): {l}"));
        assert_eq!(v.get("jsonrpc").and_then(J::str), Some("2.0"), "{l}");
        assert!(!l.contains('\n'));
    }
    // empty input: no output, exit 0
    assert_eq!(session(&[]), (vec![], 0));
}

#[test]
fn closing_stdout_early_exits_cleanly() {
    let mut child = spawn();
    let mut stdin = child.stdin.take().unwrap();
    drop(child.stdout.take()); // the client is gone
    // Keep sending a large reply's worth of requests until the server notices; it must exit 0, not crash.
    for _ in 0..50 {
        if writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/list"}}"#).is_err() {
            break;
        }
    }
    drop(stdin);
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

#[test]
fn oversize_line_is_refused_then_serving_continues() {
    let mut child = spawn();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let big = vec![b'a'; 1024 * 1024 + 100];
        stdin.write_all(&big).unwrap();
        stdin.write_all(b"\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n").unwrap();
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let lines: Vec<String> = String::from_utf8(out.stdout).unwrap().lines().map(str::to_string).collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(num(parse(&lines[0]).unwrap().get("error").and_then(|e| e.get("code")).unwrap()), "-32700");
    assert_eq!(parse(&lines[1]).unwrap().get("result"), Some(&J::Obj(vec![])));
}

#[test]
fn responses_are_valid_for_python_if_available() {
    let py = Command::new("python").args(["-I", "-c", "import sys"]).stdin(Stdio::null()).output();
    if !matches!(py, Ok(ref o) if o.status.success()) {
        eprintln!("SKIPPED: python not available; responses were validated with the crate's own parser only");
        return;
    }
    let (out, _) = session(&[INIT, r#"{"jsonrpc":"2.0","id":"s","method":"tools/list"}"#, "garbage", r#"{"jsonrpc":"2.0","id":3,"method":"tools/call"}"#]);
    let mut child = Command::new("python")
        .args(["-I", "-c", "import json,sys\nn=0\nfor l in sys.stdin.read().split('\\n'):\n  if l:\n    d=json.loads(l); assert d['jsonrpc']=='2.0'; n+=1\n    if d.get('id')=='s': assert len(d['result']['tools'])==13\nprint(n)"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(out.join("\n").as_bytes()).unwrap();
    let o = child.wait_with_output().unwrap();
    assert!(o.status.success(), "python rejected stdout: {}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "4");
    eprintln!("python validated {} response lines", out.len());
}
