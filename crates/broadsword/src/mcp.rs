//! `broadsword mcp`: a Model Context Protocol server over stdio (JSON-RPC 2.0, one JSON message per line).
//!
//! stdout carries protocol messages ONLY. Diagnostics go to stderr and to `<scratch>/logs/mcp.log`.
//!
//! This is the transport, not a tool: like `hook pre-tool` it lives outside the `TOOLS` registry. It only
//! lists tools (milestone 1); `tools/call` arrives in Stage 2 milestone 2 and is refused until then.
//! Nothing here changes files or starts jobs, so the low-disk pause has nothing to guard.

use crate::config::Config;
use crate::json::{self, J};
use crate::tools;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};

/// Protocol versions this server speaks, oldest first. The last one is the newest. (From the author's
/// knowledge of the MCP spec; not checked against the live spec.)
pub const SUPPORTED_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// Longest request line accepted, in bytes. Longer lines are refused and skipped, so a hostile client cannot exhaust memory.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

const INSTRUCTIONS: &str = "Broadsword is a local development ecosystem: it builds, tests and checks projects as tracked jobs, \
guards disk space, takes Git checkpoints and recovers interrupted work. The tools/list result already carries every tool's full \
description, inputs, exit codes and flags. In this version tools can be listed but not yet called: tools/call returns an error \
until a later milestone.";

/// Longest numeric id echoed: a finite integer of at most this many digits (no fraction, no exponent).
const MAX_ID_DIGITS: usize = 15;

fn id_ok(id: &J) -> bool {
    match id {
        J::Str(_) => true,
        J::Num(n) => !n.contains(['.', 'e', 'E']) && n.trim_start_matches('-').len() <= MAX_ID_DIGITS,
        _ => false,
    }
}

// JSON-RPC 2.0 error codes.
const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// Where to send diagnostics besides stderr. Best effort: a missing log never stops the server.
pub struct Log(Option<File>);

impl Log {
    pub fn open(cfg: Option<&Config>) -> Log {
        let file = cfg.and_then(|c| {
            let dir = c.scratch_dir.join("logs");
            fs::create_dir_all(&dir).ok()?;
            OpenOptions::new().create(true).append(true).open(dir.join("mcp.log")).ok()
        });
        Log(file)
    }

    #[cfg(test)]
    pub fn none() -> Log {
        Log(None)
    }

    fn line(&mut self, msg: &str) {
        eprintln!("broadsword mcp: {msg}");
        if let Some(f) = &mut self.0 {
            let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = writeln!(f, "{t} {msg}");
        }
    }
}

fn obj(members: Vec<(&str, J)>) -> J {
    J::Obj(members.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn error(id: &J, code: i64, message: &str) -> String {
    obj(vec![("jsonrpc", J::Str("2.0".into())), ("id", id.clone()), ("error", obj(vec![("code", J::Num(code.to_string())), ("message", J::Str(message.into()))]))]).to_json()
}

fn result(id: &J, result: J) -> String {
    obj(vec![("jsonrpc", J::Str("2.0".into())), ("id", id.clone()), ("result", result)]).to_json()
}

/// The `initialize` result: echo the client's version if we support it, else offer our newest.
fn initialize(params: Option<&J>) -> Result<J, String> {
    let wanted = params.and_then(|p| p.get("protocolVersion")).and_then(J::str).ok_or("initialize needs params.protocolVersion (a string)")?;
    let version = SUPPORTED_VERSIONS.iter().find(|v| **v == wanted).or(SUPPORTED_VERSIONS.last()).copied().unwrap_or(wanted);
    Ok(obj(vec![
        ("protocolVersion", J::Str(version.into())),
        ("capabilities", obj(vec![("tools", obj(vec![("listChanged", J::Bool(false))]))])),
        ("serverInfo", obj(vec![("name", J::Str("broadsword".into())), ("version", J::Str(env!("CARGO_PKG_VERSION").into()))])),
        ("instructions", J::Str(INSTRUCTIONS.into())),
    ]))
}

/// `tools/list`: the same registry `broadsword tools --json` prints (one function, `tools::registry_json`),
/// wrapped as `{"tools": [...]}`. Parsed and re-serialised so it fits on one line.
fn tools_list() -> J {
    let tools = json::parse(&tools::registry_json()).expect("registry_json is always valid JSON (tested)");
    obj(vec![("tools", tools)])
}

/// Handle one request line. Returns the response line, or `None` when nothing may be sent (notifications).
/// `what` is set to a short description for the log.
pub fn handle(line: &str, what: &mut String) -> Option<String> {
    let msg = match json::parse(line) {
        Ok(m) => m,
        Err(e) => {
            *what = format!("parse error: {e}");
            return Some(error(&J::Null, PARSE_ERROR, &format!("Parse error: {e}")));
        }
    };
    if matches!(msg, J::Arr(_)) {
        *what = "batch refused".into();
        return Some(error(&J::Null, INVALID_REQUEST, "Invalid Request: JSON-RPC batches are not supported; send one message per line"));
    }
    let id = msg.get("id");
    // An id must be a string or a number; anything else cannot be echoed meaningfully.
    let reply_id = match id {
        Some(i) if id_ok(i) => i.clone(),
        _ => J::Null,
    };
    let method = msg.get("method").and_then(J::str);
    let versioned = msg.get("jsonrpc").and_then(J::str) == Some("2.0");
    // A message with an id, a result or error and no method is a JSON-RPC response (to a request we never send): ignore it.
    if versioned && method.is_none() && id.is_some_and(id_ok) && (msg.get("result").is_some() || msg.get("error").is_some()) {
        *what = "response from client ignored".into();
        return None;
    }
    let (true, Some(method), true) = (versioned, method, id.is_none_or(id_ok)) else {
        *what = "invalid request".into();
        return Some(error(&reply_id, INVALID_REQUEST, "Invalid Request: need an object with jsonrpc \"2.0\", a string method, and a string id or an integer id of at most 15 digits (or no id for a notification)"));
    };
    *what = format!("{method}{}", if id.is_none() { " (notification)" } else { "" });
    id?; // a notification (no id) never gets a response, known method or not
    Some(match method {
        "initialize" => match initialize(msg.get("params")) {
            Ok(r) => {
                let asked = msg.get("params").and_then(|p| p.get("protocolVersion")).and_then(J::str).unwrap_or("");
                let chose = r.get("protocolVersion").and_then(J::str).unwrap_or("");
                *what = format!("initialize: client asked {asked:?}, negotiated {chose}");
                result(&reply_id, r)
            }
            Err(e) => error(&reply_id, INVALID_PARAMS, &e),
        },
        "ping" => result(&reply_id, obj(vec![])),
        "tools/list" => result(&reply_id, tools_list()),
        "tools/call" => error(&reply_id, METHOD_NOT_FOUND, "tools/call is not implemented yet: tool calls arrive in Stage 2 milestone 2. Use tools/list to see the tools."),
        _ => error(&reply_id, METHOD_NOT_FOUND, &format!("Method not found: {method}")),
    })
}

/// One line from `r` without ever holding more than `MAX_LINE_BYTES` + 1 bytes. `Err(())` means the line was too
/// long; the rest of it has been skipped. `Ok(None)` is end of input.
fn read_line(r: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<Result<Option<()>, ()>> {
    buf.clear();
    // Room for the limit plus a `\r\n`, which do not count toward it.
    let cap = MAX_LINE_BYTES as u64 + 2;
    let n = r.by_ref().take(cap).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(Ok(None));
    }
    let complete = buf.last() == Some(&b'\n');
    if complete || (n as u64) < cap {
        // a whole line (or the final line without a newline)
        if complete {
            buf.pop();
        }
        if buf.last() == Some(&b'\r') {
            buf.pop();
        }
        return Ok(if buf.len() <= MAX_LINE_BYTES { Ok(Some(())) } else { Err(()) });
    }
    // Too long: discard through the next newline (or EOF) in bounded chunks.
    buf.clear();
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            break;
        }
        match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => {
                r.consume(i + 1);
                break;
            }
            None => {
                let l = chunk.len();
                r.consume(l);
            }
        }
    }
    Ok(Err(()))
}

/// Serve until EOF. Returns Ok on EOF and on a closed output pipe; Err only for an unreadable input.
pub fn serve(mut input: impl BufRead, mut output: impl Write, log: &mut Log) -> io::Result<()> {
    let mut buf = Vec::new();
    loop {
        let mut what = String::new();
        let reply = match read_line(&mut input, &mut buf)? {
            Ok(None) => return Ok(()),
            Err(()) => {
                what = format!("line longer than {MAX_LINE_BYTES} bytes refused");
                Some(error(&J::Null, PARSE_ERROR, &format!("Parse error: line longer than {MAX_LINE_BYTES} bytes")))
            }
            Ok(Some(())) => match std::str::from_utf8(&buf) {
                Err(_) => {
                    what = "invalid UTF-8".into();
                    Some(error(&J::Null, PARSE_ERROR, "Parse error: input is not valid UTF-8"))
                }
                Ok(text) => {
                    let text = text.trim_end_matches('\r');
                    if text.trim().is_empty() {
                        continue;
                    }
                    handle(text, &mut what)
                }
            },
        };
        log.line(&format!("{what}{}", if reply.is_some() { "" } else { " -> no reply" }));
        if let Some(r) = reply {
            // Any write failure (a closed pipe) means the client is gone: stop quietly.
            if writeln!(output, "{r}").and_then(|()| output.flush()).is_err() {
                log.line("stdout closed; exiting");
                return Ok(());
            }
        }
    }
}

/// Entry point for `broadsword mcp`. Exit code 0 on EOF or a closed stdout.
pub fn run(cfg: Option<&Config>) -> std::process::ExitCode {
    let mut log = Log::open(cfg);
    log.line(&format!("started, version {}, protocol versions {}", env!("CARGO_PKG_VERSION"), SUPPORTED_VERSIONS.join(" ")));
    let stdin = io::stdin();
    let stdout = io::stdout();
    let code = match serve(stdin.lock(), stdout.lock(), &mut log) {
        Ok(()) => 0,
        Err(e) => {
            log.line(&format!("stdin error: {e}"));
            1
        }
    };
    log.line("stopped");
    std::process::ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn call(line: &str) -> Option<J> {
        handle(line, &mut String::new()).map(|r| json::parse(&r).expect("response must be valid JSON"))
    }

    fn err_code(v: &J) -> String {
        match v.get("error").and_then(|e| e.get("code")) {
            Some(J::Num(n)) => n.clone(),
            other => panic!("no error code: {other:?}"),
        }
    }

    #[test]
    fn initialize_echoes_supported_versions_and_falls_back_to_newest() {
        for v in SUPPORTED_VERSIONS {
            let r = call(&format!(r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{v}"}}}}"#)).unwrap();
            assert_eq!(r.get("result").and_then(|x| x.get("protocolVersion")).and_then(J::str), Some(*v));
        }
        let r = call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#).unwrap();
        let res = r.get("result").unwrap();
        assert_eq!(res.get("protocolVersion").and_then(J::str), SUPPORTED_VERSIONS.last().copied());
        assert_eq!(res.get("capabilities").unwrap().to_json(), r#"{"tools":{"listChanged":false}}"#);
        assert_eq!(res.get("serverInfo").and_then(|s| s.get("name")).and_then(J::str), Some("broadsword"));
        assert!(res.get("instructions").and_then(J::str).unwrap().contains("not yet called"));
        assert_eq!(err_code(&call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#).unwrap()), "-32602");
        assert_eq!(err_code(&call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":5}}"#).unwrap()), "-32602");
    }

    #[test]
    fn versions_are_oldest_first() {
        let mut sorted = SUPPORTED_VERSIONS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, SUPPORTED_VERSIONS);
    }

    #[test]
    fn notifications_never_get_a_response() {
        for m in ["notifications/initialized", "notifications/cancelled", "ping", "tools/list", "tools/call", "no/such/method", "initialize"] {
            assert!(call(&format!(r#"{{"jsonrpc":"2.0","method":"{m}"}}"#)).is_none(), "{m} without id must not be answered");
        }
    }

    #[test]
    fn ping_and_ids_are_echoed_exactly() {
        for id in ["1", "0", "-7", "123456789012345", r#""abc""#, r#""""#, r#""a\"bé😀""#] {
            let line = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#);
            let r = call(&line).unwrap();
            assert_eq!(r.get("id"), json::parse(id).ok().as_ref(), "id {id}");
            assert_eq!(r.get("result"), Some(&J::Obj(vec![])));
        }
    }

    #[test]
    fn tools_list_matches_the_registry() {
        let r = call(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"cursor":"whatever"}}"#).unwrap();
        let res = r.get("result").unwrap();
        assert_eq!(res.get("nextCursor"), None);
        assert_eq!(res.get("tools"), Some(&json::parse(&tools::registry_json()).unwrap()));
        let J::Arr(items) = res.get("tools").unwrap() else { panic!() };
        assert_eq!(items.len(), tools::TOOLS.len());
    }

    #[test]
    fn tools_call_is_refused_honestly() {
        let r = call(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"status"}}"#).unwrap();
        assert_eq!(err_code(&r), "-32601");
        assert!(r.get("error").unwrap().get("message").and_then(J::str).unwrap().contains("milestone 2"));
    }

    #[test]
    fn unknown_method() {
        let r = call(r#"{"jsonrpc":"2.0","id":"x","method":"resources/list"}"#).unwrap();
        assert_eq!((err_code(&r).as_str(), r.get("id")), ("-32601", Some(&J::Str("x".into()))));
    }

    #[test]
    fn client_responses_are_ignored() {
        for m in [r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, r#"{"jsonrpc":"2.0","id":"a","error":{"code":-1,"message":"x"}}"#] {
            let mut what = String::new();
            assert!(handle(m, &mut what).is_none(), "{m}");
            assert!(what.contains("ignored"));
        }
    }

    #[test]
    fn ids_must_be_strings_or_small_integers() {
        for id in ["1.5", "1e3", "1E3", "1234567890123456", "-1234567890123456", "1.0", "true", "[]"] {
            let r = call(&format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"ping"}}"#)).unwrap();
            assert_eq!((err_code(&r).as_str(), r.get("id")), ("-32600", Some(&J::Null)), "id {id}");
        }
    }

    #[test]
    fn trailing_cr_does_not_count_toward_the_limit() {
        let mut line = vec![b' '; MAX_LINE_BYTES - 2];
        line.extend_from_slice(b"{}\r\n");
        let out = run_lines(&line);
        assert_eq!(err_code(&out[0]), "-32600", "an exactly-at-limit line with CRLF is read, not refused as too long");
        let mut over = vec![b' '; MAX_LINE_BYTES - 1];
        over.extend_from_slice(b"{}\r\n");
        let out = run_lines(&over);
        assert!(out[0].get("error").unwrap().get("message").and_then(J::str).unwrap().contains("longer"));
    }

    #[test]
    fn initialize_logs_the_negotiated_version() {
        let mut what = String::new();
        handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#, &mut what);
        assert!(what.contains("2099-01-01") && what.contains("negotiated 2025-11-25"), "{what}");
    }

    #[test]
    fn bad_messages() {
        for bad in ["{", "nope", r#"{"jsonrpc":"2.0","id":1,"method":"ping"} x"#, r#"{"jsonrpc":"2.0","id":1,"method":"pi\x"}"#] {
            let r = call(bad).unwrap();
            assert_eq!((err_code(&r).as_str(), r.get("id")), ("-32700", Some(&J::Null)), "{bad}");
        }
        for bad in [
            r#"[]"#,
            r#"[{"jsonrpc":"2.0","id":1,"method":"ping"}]"#,
            r#"{"id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"1.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":2,"id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","id":1,"method":5}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":{},"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","method":7}"#,
            "42",
            r#""str""#,
            "null",
        ] {
            let r = call(bad).unwrap();
            assert_eq!(err_code(&r), "-32600", "{bad}");
        }
        // a valid id is still echoed on an invalid request
        let r = call(r#"{"jsonrpc":"2.0","id":9}"#).unwrap();
        assert_eq!(r.get("id"), Some(&J::Num("9".into())));
        // deeply nested hostile input is a parse error, not a crash
        assert_eq!(err_code(&call(&"[".repeat(100_000)).unwrap()), "-32700");
    }

    fn run_lines(input: &[u8]) -> Vec<J> {
        let mut out = Vec::new();
        serve(Cursor::new(input.to_vec()), &mut out, &mut Log::none()).unwrap();
        String::from_utf8(out).unwrap().lines().map(|l| json::parse(l).unwrap()).collect()
    }

    #[test]
    fn serve_keeps_going_after_errors() {
        let input = "{bad\n\n   \r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n[1]\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}";
        let out = run_lines(input.as_bytes());
        let ids: Vec<Option<&J>> = out.iter().map(|r| r.get("id")).collect();
        assert_eq!(ids, [Some(&J::Null), Some(&J::Num("1".into())), Some(&J::Null), Some(&J::Num("2".into()))]);
    }

    #[test]
    fn invalid_utf8_is_a_parse_error() {
        let out = run_lines(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"\xff\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n");
        assert_eq!(err_code(&out[0]), "-32700");
        assert_eq!(out[1].get("id"), Some(&J::Num("2".into())));
    }

    #[test]
    fn oversize_line_is_refused_and_skipped() {
        let mut input = vec![b'['; MAX_LINE_BYTES + 10];
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"ping\"}\n");
        let out = run_lines(&input);
        assert_eq!(out.len(), 2);
        assert_eq!(err_code(&out[0]), "-32700");
        assert!(out[0].get("error").unwrap().get("message").and_then(J::str).unwrap().contains("longer"));
        assert_eq!(out[1].get("id"), Some(&J::Num("5".into())));
        // a line of exactly the limit is accepted (and is just invalid JSON here)
        let mut exact = vec![b' '; MAX_LINE_BYTES - 1];
        exact.push(b'x');
        exact.push(b'\n');
        assert!(run_lines(&exact)[0].get("error").unwrap().get("message").and_then(J::str).unwrap().contains("Parse error: unexpected input"));
        // an oversize line with no newline before EOF still ends cleanly
        assert_eq!(run_lines(&vec![b'a'; MAX_LINE_BYTES * 2]).len(), 1);
    }

    #[test]
    fn closed_stdout_ends_serving_cleanly() {
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n";
        assert!(serve(Cursor::new(&input[..]), Broken, &mut Log::none()).is_ok());
    }

    #[test]
    fn mcp_is_not_in_the_registry() {
        assert!(tools::resolve(&["mcp"]).is_none());
        assert!(tools::find("mcp").is_none());
    }
}
