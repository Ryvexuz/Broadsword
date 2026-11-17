//! `broadsword mcp`: a Model Context Protocol server over stdio (JSON-RPC 2.0, one JSON message per line).
//!
//! stdout carries protocol messages ONLY. Diagnostics go to stderr and to `<scratch>/logs/mcp.log`.
//!
//! This is the transport, not a tool: like `hook pre-tool` it lives outside the `TOOLS` registry.
//!
//! `tools/call` runs the tool as a CHILD PROCESS: this same binary (`current_exe()`), started with the tool's
//! command-line arguments (`Tool::to_args`) in the main root. A handler that `println!`s therefore cannot touch
//! the protocol stream, and a hung tool can be killed with its whole process tree.
//!
//! One call at a time: the call runs on the thread that serves the connection. While it runs, a second thread keeps
//! reading stdin (so the pipe never fills) and this thread watches it: `ping` is answered at once,
//! `notifications/cancelled` for the running request kills the child and sends no response (as the spec says), and
//! every other message waits (at most `MAX_PENDING` of them; beyond that the pipe backs up) and is handled, in
//! order, after the call. End of input during a call does not kill it: the call finishes, is answered, then the
//! server exits.
//!
//! Protocol errors (unknown tool, malformed request) are JSON-RPC errors. Everything about running the tool
//! (bad arguments, refusal, non-zero exit, timeout) is a normal result with `isError: true`.

use crate::config::Config;
use crate::json::{self, J};
use crate::sys;
use crate::tools::{self, Inputs, Ty, Val};
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Protocol versions this server speaks, oldest first. The last one is the newest. (From the author's
/// knowledge of the MCP spec; not checked against the live spec.)
pub const SUPPORTED_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];

/// `outputSchema` (in tools/list) and `structuredContent` (in tools/call results) exist from this version on.
/// Versions are ISO dates, so string order is date order.
pub const STRUCTURED_FROM: &str = "2025-06-18";

/// Longest request line accepted, in bytes. Longer lines are refused and skipped, so a hostile client cannot exhaust memory.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// Messages queued while a call runs. More than this and the reader thread blocks, so the pipe backs up instead of memory growing.
const MAX_PENDING: usize = 256;

/// How often a running call looks at its child and at new input.
const TICK: Duration = Duration::from_millis(20);

/// After the tool process has exited, how long to wait for its output pipes to close (a grandchild that outlived it
/// can hold them open) before taking what was read.
const PIPE_GRACE: Duration = Duration::from_secs(2);

const INSTRUCTIONS: &str = "Broadsword is a local development ecosystem: it builds, tests and checks projects as tracked jobs, \
guards disk space, takes Git checkpoints and recovers interrupted work. The tools/list result already carries every tool's full \
description, inputs, exit codes and flags: read it before calling. Every tool except `watch` can be called with tools/call. `watch` \
never ends, so it is not available over MCP (its _meta.broadsword.mcp_callable is false): start it yourself from a terminal. \
Calls run one at a time, each as a separate process, and block until the tool ends: a build or check can take minutes, and a call is \
killed after the mcp_call_timeout_seconds setting (1800 s as shipped). The result's text is the tool's stdout, then a `stderr:` \
section if there was any, then `exit code: N`; isError is true when the exit code is not 0 (each tool lists its exit codes: for \
example 3 means refused because Broadsword is paused or the disk is low). From protocol version 2025-06-18 the same data is also in \
structuredContent as {exit_code, stdout, stderr, truncated}. Output beyond mcp_output_max_kb is cut to its last part and the result \
says so. While a call runs, ping is still answered and notifications/cancelled for that call stops it.";

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

/// What a tool call needs from the outside: the settings and the binary to run it with.
pub struct Ctx {
    pub cfg: Option<Config>,
    pub exe: Option<PathBuf>,
}

impl Ctx {
    pub fn new(cfg: Option<&Config>) -> Ctx {
        Ctx { cfg: cfg.cloned(), exe: std::env::current_exe().ok() }
    }

    #[cfg(test)]
    pub fn none() -> Ctx {
        Ctx { cfg: None, exe: None }
    }
}

/// Per-connection state: the protocol version agreed in `initialize` (`None` until then, which counts as the oldest).
#[derive(Default)]
pub struct Session {
    version: Option<&'static str>,
}

impl Session {
    /// May `outputSchema` / `structuredContent` be used on this connection?
    fn structured(&self) -> bool {
        self.version.is_some_and(|v| v >= STRUCTURED_FROM)
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
/// wrapped as `{"tools": [...]}`. Parsed and re-serialised so it fits on one line. For a protocol version older than
/// `STRUCTURED_FROM` the `outputSchema` member is left out of every tool.
fn tools_list(structured: bool) -> J {
    let mut tools = json::parse(&tools::registry_json()).expect("registry_json is always valid JSON (tested)");
    if !structured {
        if let J::Arr(items) = &mut tools {
            for t in items {
                if let J::Obj(members) = t {
                    members.retain(|(k, _)| k != "outputSchema");
                }
            }
        }
    }
    obj(vec![("tools", tools)])
}

// ---- tools/call: checking the request ----------------------------------------------------

/// Why a call was not run.
#[derive(Debug, PartialEq)]
pub enum Reject {
    /// A JSON-RPC error (-32602): the request itself is malformed or names no such tool.
    Protocol(String),
    /// A tool execution error: a normal result with `isError: true`.
    Tool(String),
}

/// A validated call: the tool and the command-line arguments to run it with.
pub struct Prepared {
    pub tool: &'static tools::Tool,
    pub args: Vec<String>,
}

impl std::fmt::Debug for Prepared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Prepared({} {:?})", self.tool.name, self.args)
    }
}

fn kind(v: &J) -> &'static str {
    match v {
        J::Null => "null",
        J::Bool(_) => "a boolean",
        J::Num(_) => "a number",
        J::Str(_) => "a string",
        J::Arr(_) => "an array",
        J::Obj(_) => "an object",
    }
}

/// Check `arguments` against the tool's inputSchema (the registry's `Param`s: what tools/list publishes) and
/// convert it to typed inputs. The message says what is wrong and what is accepted.
pub fn validate_args(tool: &tools::Tool, members: &[(String, J)]) -> Result<Inputs, String> {
    let accepted = || if tool.params.is_empty() { "it takes no arguments".to_string() } else { format!("it accepts: {}", tool.params.iter().map(|p| p.name).collect::<Vec<_>>().join(", ")) };
    for (i, (k, _)) in members.iter().enumerate() {
        if !tool.params.iter().any(|p| p.name == k) {
            return Err(format!("unknown argument `{k}` for tool `{}`; {}", tool.name, accepted()));
        }
        if members[..i].iter().any(|(k2, _)| k2 == k) {
            return Err(format!("argument `{k}` is given more than once"));
        }
    }
    let mut inputs = Inputs::default();
    for p in tool.params {
        let Some((_, v)) = members.iter().find(|(k, _)| k == p.name) else {
            if p.required {
                return Err(format!("missing required argument `{}` ({}) for tool `{}`", p.name, p.ty.name(), tool.name));
            }
            continue;
        };
        let wrong = |want: &str| format!("argument `{}` must be {want}, got {}", p.name, kind(v));
        let val = match (p.ty, v) {
            (Ty::String, J::Str(s)) => Val::Str(s.clone()),
            (Ty::String, _) => return Err(wrong("a string")),
            (Ty::Boolean, J::Bool(b)) => Val::Bool(*b),
            (Ty::Boolean, _) => return Err(wrong("a boolean")),
            (Ty::Integer, J::Num(n)) => Val::Int(n.parse::<i64>().map_err(|_| wrong("an integer"))?),
            (Ty::Integer, _) => return Err(wrong("an integer")),
            (Ty::StringArray, J::Arr(items)) => {
                let strings: Option<Vec<String>> = items.iter().map(|x| x.str().map(String::from)).collect();
                let strings = strings.ok_or_else(|| format!("argument `{}` must be an array of strings; at least one item is not a string", p.name))?;
                if p.required && strings.is_empty() {
                    return Err(format!("argument `{}` must not be an empty array", p.name));
                }
                Val::List(strings)
            }
            (Ty::StringArray, _) => return Err(wrong("an array of strings")),
        };
        inputs = inputs.with(p.name, val);
    }
    Ok(inputs)
}

/// Everything about a `tools/call` that can be decided without running anything.
pub fn prepare_call(params: Option<&J>) -> Result<Prepared, Reject> {
    let Some(name) = params.and_then(|p| p.get("name")).and_then(J::str) else {
        return Err(Reject::Protocol("tools/call needs params.name (a string: the tool to call)".into()));
    };
    let Some(tool) = tools::find(name) else {
        let known: Vec<&str> = tools::TOOLS.iter().map(|t| t.name).collect();
        return Err(Reject::Protocol(format!("Unknown tool: {name}; known tools: {}", known.join(", "))));
    };
    let members: &[(String, J)] = match params.and_then(|p| p.get("arguments")) {
        None | Some(J::Null) => &[],
        Some(J::Obj(m)) => m,
        Some(_) => return Err(Reject::Protocol("params.arguments must be an object".into())),
    };
    if !tool.mcp_callable {
        return Err(Reject::Tool(format!(
            "`{name}` is not available over MCP: it never ends on its own, and a tool call must end. It is a background service: start it yourself outside MCP, from a terminal, as a background process (`{}`), and stop it by killing that process. (Callers can tell in advance: tools/list marks it with _meta.broadsword.mcp_callable = false.)",
            tool.cli
        )));
    }
    let inputs = validate_args(tool, members).map_err(Reject::Tool)?;
    let args = (tool.to_args)(&inputs);
    // Whatever was accepted must come back as this very tool from the command line, or the child would run something else.
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    match tools::resolve(&refs) {
        Some((t, parsed)) if t.name == tool.name && parsed == inputs.canonical() => Ok(Prepared { tool, args }),
        _ => Err(Reject::Tool(format!("these arguments cannot be passed to `{name}` on its command line (for example a value that looks like an option); command line would be: {}", args.join(" ")))),
    }
}

// ---- tools/call: running the child ---------------------------------------------------------

/// The last `cap` bytes of a stream, and how many bytes it had in all.
struct Tail {
    buf: Vec<u8>,
    total: u64,
    cap: usize,
}

impl Tail {
    fn new(cap: usize) -> Tail {
        Tail { buf: Vec::new(), total: 0, cap }
    }

    fn push(&mut self, chunk: &[u8]) {
        self.total += chunk.len() as u64;
        self.buf.extend_from_slice(chunk);
        if self.buf.len() > self.cap.saturating_mul(2) {
            let cut = self.buf.len() - self.cap;
            self.buf.drain(..cut);
        }
    }

    /// (text, was anything cut). A cut never starts in the middle of a UTF-8 character.
    fn text(&self) -> (String, bool) {
        let mut b = &self.buf[..];
        if b.len() > self.cap {
            b = &b[b.len() - self.cap..];
        }
        let cut = self.total > b.len() as u64;
        if cut {
            while !b.is_empty() && b[0] & 0xC0 == 0x80 {
                b = &b[1..];
            }
        }
        (String::from_utf8_lossy(b).into_owned(), cut)
    }
}

fn pump(mut src: impl Read, tail: Arc<Mutex<Tail>>) {
    let mut chunk = [0u8; 8192];
    loop {
        match src.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                if let Ok(mut t) = tail.lock() {
                    t.push(&chunk[..n]);
                }
            }
        }
    }
}

/// What a finished (or killed) call produced.
#[derive(Debug, Default, PartialEq)]
pub struct Captured {
    /// `None`: the process was killed on timeout, or ended without an exit code.
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_cut: bool,
    pub stderr_cut: bool,
    /// Set when the call was killed for taking longer than this many seconds.
    pub timed_out_after: Option<u64>,
}

impl Captured {
    pub fn is_error(&self) -> bool {
        self.timed_out_after.is_some() || self.code != Some(0)
    }
}

/// The result's text: what the tool printed, then `stderr:` if any, then the exit code.
pub fn render_text(c: &Captured, cap_kb: u64) -> String {
    let mut t = String::new();
    if let Some(n) = c.timed_out_after {
        t.push_str(&format!("timed out after {n} s (the mcp_call_timeout_seconds setting): the tool and everything it started were killed. It may have left a job interrupted; `recover` resumes it. Output up to that point follows.\n"));
    }
    for (cut, name) in [(c.stdout_cut, "stdout"), (c.stderr_cut, "stderr")] {
        if cut {
            t.push_str(&format!("[{name} truncated: only its last {cap_kb} KB are shown]\n"));
        }
    }
    t.push_str(&c.stdout);
    if !c.stdout.is_empty() && !c.stdout.ends_with('\n') {
        t.push('\n');
    }
    if !c.stderr.is_empty() {
        t.push_str("stderr:\n");
        t.push_str(&c.stderr);
        if !c.stderr.ends_with('\n') {
            t.push('\n');
        }
    }
    match c.code {
        Some(n) if c.timed_out_after.is_none() => t.push_str(&format!("exit code: {n}")),
        _ => t.push_str("exit code: none (killed)"),
    }
    t
}

fn text_content(text: &str) -> J {
    J::Arr(vec![obj(vec![("type", J::Str("text".into())), ("text", J::Str(text.into()))])])
}

/// A tool execution error that never ran the tool: no exit code, so no structuredContent.
fn tool_error(message: &str) -> J {
    obj(vec![("content", text_content(message)), ("isError", J::Bool(true))])
}

/// The `tools/call` result. `structured` adds `structuredContent` (only for protocol versions that have it).
pub fn call_result(c: &Captured, cap_kb: u64, structured: bool) -> J {
    let mut members = vec![("content", text_content(&render_text(c, cap_kb))), ("isError", J::Bool(c.is_error()))];
    if structured {
        members.push((
            "structuredContent",
            obj(vec![
                ("exit_code", J::Num(c.code.filter(|_| c.timed_out_after.is_none()).unwrap_or(-1).to_string())),
                ("stdout", J::Str(c.stdout.clone())),
                ("stderr", J::Str(c.stderr.clone())),
                ("truncated", J::Bool(c.stdout_cut || c.stderr_cut)),
            ]),
        ));
    }
    obj(members)
}

enum Ran {
    Done(Captured),
    Cancelled,
    /// stdout closed: the client is gone.
    Gone,
}

/// Something a running call noticed on stdin.
enum Side {
    Nothing,
    Cancel,
    Gone,
}

enum Event {
    Line(Vec<u8>),
    TooLong,
    Eof,
    ReadError(String),
}

pub enum Handled {
    Reply(Option<String>),
    Call { id: J, params: Option<J> },
}

/// Handle one request line. `what` is set to a short description for the log. A `tools/call` is not answered here:
/// it comes back as `Handled::Call` for the server to run.
pub fn handle(line: &str, what: &mut String, sess: &mut Session) -> Handled {
    let msg = match json::parse(line) {
        Ok(m) => m,
        Err(e) => {
            *what = format!("parse error: {e}");
            return Handled::Reply(Some(error(&J::Null, PARSE_ERROR, &format!("Parse error: {e}"))));
        }
    };
    if matches!(msg, J::Arr(_)) {
        *what = "batch refused".into();
        return Handled::Reply(Some(error(&J::Null, INVALID_REQUEST, "Invalid Request: JSON-RPC batches are not supported; send one message per line")));
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
        return Handled::Reply(None);
    }
    let (true, Some(method), true) = (versioned, method, id.is_none_or(id_ok)) else {
        *what = "invalid request".into();
        return Handled::Reply(Some(error(&reply_id, INVALID_REQUEST, "Invalid Request: need an object with jsonrpc \"2.0\", a string method, and a string id or an integer id of at most 15 digits (or no id for a notification)")));
    };
    *what = format!("{method}{}", if id.is_none() { " (notification)" } else { "" });
    if id.is_none() {
        return Handled::Reply(None); // a notification (no id) never gets a response, known method or not
    }
    Handled::Reply(Some(match method {
        "initialize" => match initialize(msg.get("params")) {
            Ok(r) => {
                let asked = msg.get("params").and_then(|p| p.get("protocolVersion")).and_then(J::str).unwrap_or("");
                let chose = r.get("protocolVersion").and_then(J::str).unwrap_or("");
                sess.version = SUPPORTED_VERSIONS.iter().find(|v| **v == chose).copied();
                *what = format!("initialize: client asked {asked:?}, negotiated {chose}");
                result(&reply_id, r)
            }
            Err(e) => error(&reply_id, INVALID_PARAMS, &e),
        },
        "ping" => result(&reply_id, obj(vec![])),
        "tools/list" => result(&reply_id, tools_list(sess.structured())),
        "tools/call" => {
            let params = msg.get("params").cloned();
            *what = format!("tools/call {}", params.as_ref().and_then(|p| p.get("name")).and_then(J::str).unwrap_or("?"));
            return Handled::Call { id: reply_id, params };
        }
        _ => error(&reply_id, METHOD_NOT_FOUND, &format!("Method not found: {method}")),
    }))
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

/// Read `input` on its own thread, one event per line, through a bounded channel.
fn spawn_reader(mut input: impl BufRead + Send + 'static) -> Receiver<Event> {
    let (tx, rx) = mpsc::sync_channel(64);
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        loop {
            let ev = match read_line(&mut input, &mut buf) {
                Ok(Ok(None)) => Event::Eof,
                Ok(Ok(Some(()))) => Event::Line(buf.clone()),
                Ok(Err(())) => Event::TooLong,
                Err(e) => Event::ReadError(e.to_string()),
            };
            let last = matches!(ev, Event::Eof | Event::ReadError(_));
            if tx.send(ev).is_err() || last {
                break;
            }
        }
    });
    rx
}

enum Flow {
    Continue,
    Stop,
}

struct Server<'a, W: Write> {
    out: W,
    log: &'a mut Log,
    ctx: &'a Ctx,
    rx: Receiver<Event>,
    pending: VecDeque<Event>,
    eof: bool,
    sess: Session,
}

impl<W: Write> Server<'_, W> {
    /// Write one response line. False means the client is gone.
    fn send(&mut self, line: &str) -> bool {
        if writeln!(self.out, "{line}").and_then(|()| self.out.flush()).is_err() {
            self.log.line("stdout closed; exiting");
            return false;
        }
        true
    }

    fn process(&mut self, ev: Event) -> io::Result<Flow> {
        let mut what = String::new();
        let handled = match ev {
            Event::Eof => {
                self.eof = true;
                return Ok(Flow::Continue);
            }
            Event::ReadError(e) => return Err(io::Error::other(e)),
            Event::TooLong => {
                what = format!("line longer than {MAX_LINE_BYTES} bytes refused");
                Handled::Reply(Some(error(&J::Null, PARSE_ERROR, &format!("Parse error: line longer than {MAX_LINE_BYTES} bytes"))))
            }
            Event::Line(buf) => match std::str::from_utf8(&buf) {
                Err(_) => {
                    what = "invalid UTF-8".into();
                    Handled::Reply(Some(error(&J::Null, PARSE_ERROR, "Parse error: input is not valid UTF-8")))
                }
                Ok(text) => {
                    let text = text.trim_end_matches('\r');
                    if text.trim().is_empty() {
                        return Ok(Flow::Continue);
                    }
                    handle(text, &mut what, &mut self.sess)
                }
            },
        };
        match handled {
            Handled::Reply(reply) => {
                self.log.line(&format!("{what}{}", if reply.is_some() { "" } else { " -> no reply" }));
                if let Some(r) = reply {
                    if !self.send(&r) {
                        return Ok(Flow::Stop);
                    }
                }
            }
            Handled::Call { id, params } => {
                self.log.line(&what);
                return Ok(self.call(&id, params.as_ref()));
            }
        }
        Ok(Flow::Continue)
    }

    fn call(&mut self, id: &J, params: Option<&J>) -> Flow {
        let response = match prepare_call(params) {
            Err(Reject::Protocol(m)) => {
                self.log.line(&format!("tools/call refused: {m}"));
                error(id, INVALID_PARAMS, &m)
            }
            Err(Reject::Tool(m)) => {
                self.log.line(&format!("tools/call refused (isError): {m}"));
                result(id, tool_error(&m))
            }
            Ok(p) => {
                let ctx = self.ctx;
                let (Some(cfg), Some(exe)) = (&ctx.cfg, &ctx.exe) else {
                    let m = "cannot run tools: broadsword.toml could not be loaded when the server started (or the server's own binary could not be found); fix it and restart the MCP server";
                    self.log.line(m);
                    return if self.send(&result(id, tool_error(m))) { Flow::Continue } else { Flow::Stop };
                };
                match self.run_child(id, &p, cfg, exe) {
                    Ok(Ran::Done(c)) => {
                        self.log.line(&format!("tools/call {} -> {}", p.tool.name, c.code.map_or("killed".to_string(), |n| format!("exit {n}"))));
                        result(id, call_result(&c, cfg.mcp_output_max_kb, self.sess.structured()))
                    }
                    Ok(Ran::Cancelled) => {
                        self.log.line(&format!("tools/call {} cancelled by the client: child killed, no response sent", p.tool.name));
                        return Flow::Continue;
                    }
                    Ok(Ran::Gone) => return Flow::Stop,
                    Err(m) => {
                        self.log.line(&m);
                        result(id, tool_error(&m))
                    }
                }
            }
        };
        if self.send(&response) {
            Flow::Continue
        } else {
            Flow::Stop
        }
    }

    /// Start the tool as a child process, collect its output and wait for it to end, time out or be cancelled.
    fn run_child(&mut self, id: &J, p: &Prepared, cfg: &Config, exe: &std::path::Path) -> Result<Ran, String> {
        let mut cmd = Command::new(exe);
        cmd.args(&p.args).current_dir(&cfg.root).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window flashes up for a windowless server
        }
        let mut child = cmd.spawn().map_err(|e| format!("cannot start `{} {}`: {e}", exe.display(), p.args.join(" ")))?;
        let cap = (cfg.mcp_output_max_kb as usize).saturating_mul(1024);
        let (out_tail, err_tail) = (Arc::new(Mutex::new(Tail::new(cap))), Arc::new(Mutex::new(Tail::new(cap))));
        let t_out = child.stdout.take().map(|s| {
            let t = Arc::clone(&out_tail);
            std::thread::spawn(move || pump(s, t))
        });
        let t_err = child.stderr.take().map(|s| {
            let t = Arc::clone(&err_tail);
            std::thread::spawn(move || pump(s, t))
        });
        let readers_done = || t_out.as_ref().is_none_or(|t| t.is_finished()) && t_err.as_ref().is_none_or(|t| t.is_finished());
        let timeout = Duration::from_secs(cfg.mcp_call_timeout_seconds);
        let start = Instant::now();
        let mut exited: Option<(Option<i32>, Instant)> = None;
        let mut timed_out = false;
        let kill = |child: &mut std::process::Child| {
            // The tool starts cargo, rustc and shells: take the whole tree down. The pid cannot have been reused:
            // the child has not been waited on yet, so its handle is still ours (as in job.rs).
            sys::kill_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
        };
        loop {
            if exited.is_none() {
                match child.try_wait() {
                    Ok(Some(status)) => exited = Some((status.code(), Instant::now())),
                    Ok(None) => {}
                    Err(e) => {
                        kill(&mut child);
                        return Err(format!("waiting for the tool failed: {e}"));
                    }
                }
            }
            if let Some((_, at)) = exited {
                if readers_done() || at.elapsed() >= PIPE_GRACE {
                    break;
                }
            } else if start.elapsed() >= timeout {
                kill(&mut child);
                timed_out = true;
                break;
            }
            match self.poll_input(id) {
                Side::Nothing => {}
                Side::Cancel => {
                    kill(&mut child);
                    return Ok(Ran::Cancelled);
                }
                Side::Gone => {
                    kill(&mut child);
                    return Ok(Ran::Gone);
                }
            }
        }
        let (stdout, stdout_cut) = out_tail.lock().map(|t| t.text()).unwrap_or_default();
        let (stderr, stderr_cut) = err_tail.lock().map(|t| t.text()).unwrap_or_default();
        Ok(Ran::Done(Captured {
            code: if timed_out { None } else { exited.and_then(|(c, _)| c) },
            stdout,
            stderr,
            stdout_cut,
            stderr_cut,
            timed_out_after: timed_out.then_some(cfg.mcp_call_timeout_seconds),
        }))
    }

    /// Wait up to one tick for input during a call (this is also the call's sleep). Ping is answered, a cancel of
    /// the running request ends the call, anything else is queued for after it.
    fn poll_input(&mut self, running: &J) -> Side {
        if self.eof || self.pending.len() >= MAX_PENDING {
            std::thread::sleep(TICK);
            return Side::Nothing;
        }
        let ev = match self.rx.recv_timeout(TICK) {
            Ok(e) => e,
            Err(RecvTimeoutError::Timeout) => return Side::Nothing,
            Err(RecvTimeoutError::Disconnected) => {
                self.eof = true;
                return Side::Nothing;
            }
        };
        let Event::Line(bytes) = &ev else {
            if matches!(ev, Event::Eof) {
                self.eof = true; // the client closed its end: finish this call, answer it, then stop
            } else {
                self.pending.push_back(ev);
            }
            return Side::Nothing;
        };
        let parsed = std::str::from_utf8(bytes).ok().and_then(|t| json::parse(t.trim()).ok());
        let Some(msg) = parsed else {
            self.pending.push_back(ev);
            return Side::Nothing;
        };
        let (method, id) = (msg.get("method").and_then(J::str), msg.get("id"));
        match (method, id) {
            (Some("notifications/cancelled"), None) => {
                if msg.get("params").and_then(|p| p.get("requestId")) == Some(running) {
                    return Side::Cancel;
                }
                self.log.line("notifications/cancelled for a request that is not running: ignored");
                Side::Nothing
            }
            (Some("ping"), Some(_)) => {
                let mut what = String::new();
                let line = std::str::from_utf8(bytes).unwrap_or_default().trim().to_string();
                match handle(&line, &mut what, &mut self.sess) {
                    Handled::Reply(Some(r)) => {
                        self.log.line("ping answered during a tool call");
                        if self.send(&r) {
                            Side::Nothing
                        } else {
                            Side::Gone
                        }
                    }
                    _ => Side::Nothing,
                }
            }
            _ => {
                self.pending.push_back(ev);
                Side::Nothing
            }
        }
    }
}

/// Serve until EOF. Returns Ok on EOF and on a closed output pipe; Err only for an unreadable input.
pub fn serve(input: impl BufRead + Send + 'static, output: impl Write, log: &mut Log, ctx: &Ctx) -> io::Result<()> {
    let mut s = Server { out: output, log, ctx, rx: spawn_reader(input), pending: VecDeque::new(), eof: false, sess: Session::default() };
    loop {
        let ev = match s.pending.pop_front() {
            Some(e) => e,
            None if s.eof => return Ok(()),
            None => match s.rx.recv() {
                Ok(e) => e,
                Err(_) => return Ok(()),
            },
        };
        if let Flow::Stop = s.process(ev)? {
            return Ok(());
        }
    }
}

/// Entry point for `broadsword mcp`. Exit code 0 on EOF or a closed stdout.
pub fn run(cfg: Option<&Config>) -> std::process::ExitCode {
    let mut log = Log::open(cfg);
    log.line(&format!("started, version {}, protocol versions {}", env!("CARGO_PKG_VERSION"), SUPPORTED_VERSIONS.join(" ")));
    let ctx = Ctx::new(cfg);
    match &ctx.exe {
        Some(e) => log.line(&format!("tools run as {}", e.display())),
        None => log.line("cannot find this program's own path: tools/call will report an error"),
    }
    let code = match serve(io::BufReader::new(io::stdin()), io::stdout().lock(), &mut log, &ctx) {
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

    fn call_with(line: &str, sess: &mut Session) -> Option<J> {
        match handle(line, &mut String::new(), sess) {
            Handled::Reply(r) => r.map(|r| json::parse(&r).expect("response must be valid JSON")),
            Handled::Call { .. } => panic!("unexpected tools/call"),
        }
    }

    fn call(line: &str) -> Option<J> {
        call_with(line, &mut Session::default())
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
        let ins = res.get("instructions").and_then(J::str).unwrap();
        assert!(ins.contains("one at a time") && ins.contains("`watch`") && ins.contains("structuredContent") && !ins.contains("not yet called"), "{ins}");
        assert_eq!(err_code(&call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#).unwrap()), "-32602");
        assert_eq!(err_code(&call(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":5}}"#).unwrap()), "-32602");
    }

    #[test]
    fn versions_are_oldest_first() {
        let mut sorted = SUPPORTED_VERSIONS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, SUPPORTED_VERSIONS);
        assert!(SUPPORTED_VERSIONS.contains(&STRUCTURED_FROM));
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

    fn init_session(version: &str) -> Session {
        let mut s = Session::default();
        call_with(&format!(r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"{version}"}}}}"#), &mut s).unwrap();
        s
    }

    #[test]
    fn tools_list_matches_the_registry_and_gates_output_schema_by_version() {
        let list = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{"cursor":"whatever"}}"#;
        let full = json::parse(&tools::registry_json()).unwrap();
        for v in SUPPORTED_VERSIONS {
            let r = call_with(list, &mut init_session(v)).unwrap();
            let res = r.get("result").unwrap();
            assert_eq!(res.get("nextCursor"), None);
            let J::Arr(items) = res.get("tools").unwrap() else { panic!() };
            assert_eq!(items.len(), tools::TOOLS.len());
            if *v >= STRUCTURED_FROM {
                assert_eq!(res.get("tools"), Some(&full), "{v}: the same as `tools --json`");
                assert!(items.iter().all(|t| t.get("outputSchema").is_some()), "{v}");
            } else {
                assert!(items.iter().all(|t| t.get("outputSchema").is_none()), "{v}: outputSchema does not exist yet");
                assert!(items.iter().all(|t| t.get("inputSchema").is_some() && t.get("name").is_some()));
            }
        }
        // before initialize nothing newer than the oldest version may be assumed
        let r = call(list).unwrap();
        let J::Arr(items) = r.get("result").and_then(|r| r.get("tools")).unwrap().clone() else { panic!() };
        assert!(items.iter().all(|t| t.get("outputSchema").is_none()));
    }

    #[test]
    fn tools_call_is_handed_to_the_server_not_answered_by_handle() {
        let mut what = String::new();
        let h = handle(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"status"}}"#, &mut what, &mut Session::default());
        assert!(matches!(h, Handled::Call { ref id, .. } if *id == J::Num("3".into())));
        assert_eq!(what, "tools/call status");
    }

    fn prep(params: &str) -> Result<Prepared, Reject> {
        prepare_call(json::parse(params).ok().as_ref())
    }

    fn tool_msg(r: Result<Prepared, Reject>) -> String {
        match r {
            Err(Reject::Tool(m)) => m,
            other => panic!("expected a tool error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tool_and_malformed_calls_are_protocol_errors() {
        for bad in [r#"{"name":"nope"}"#, r#"{}"#, r#"{"name":5}"#, r#"{"name":"status","arguments":[]}"#, r#"{"name":"status","arguments":"x"}"#] {
            assert!(matches!(prep(bad), Err(Reject::Protocol(_))), "{bad}");
        }
        assert!(prepare_call(None).is_err());
        let Err(Reject::Protocol(m)) = prep(r#"{"name":"nope"}"#) else { panic!() };
        assert!(m.contains("Unknown tool: nope") && m.contains("status") && m.contains("check"), "{m}");
    }

    #[test]
    fn bad_arguments_are_tool_errors_with_a_clear_message() {
        let m = tool_msg(prep(r#"{"name":"build","arguments":{}}"#));
        assert!(m.contains("missing required argument `project`"), "{m}");
        let m = tool_msg(prep(r#"{"name":"build"}"#));
        assert!(m.contains("missing required argument `project`"), "{m}");
        let m = tool_msg(prep(r#"{"name":"build","arguments":{"project":5}}"#));
        assert!(m.contains("`project` must be a string, got a number"), "{m}");
        let m = tool_msg(prep(r#"{"name":"build","arguments":{"project":"p","extra":1}}"#));
        assert!(m.contains("unknown argument `extra`") && m.contains("it accepts: project"), "{m}");
        let m = tool_msg(prep(r#"{"name":"status","arguments":{"x":1}}"#));
        assert!(m.contains("unknown argument `x`") && m.contains("takes no arguments"), "{m}");
        let m = tool_msg(prep(r#"{"name":"resume","arguments":{"force":"yes"}}"#));
        assert!(m.contains("`force` must be a boolean, got a string"), "{m}");
        let m = tool_msg(prep(r#"{"name":"resume","arguments":{"force":null}}"#));
        assert!(m.contains("got null"), "{m}");
        let m = tool_msg(prep(r#"{"name":"run","arguments":{"project":"p","command":"cargo --version"}}"#));
        assert!(m.contains("`command` must be an array of strings, got a string"), "{m}");
        let m = tool_msg(prep(r#"{"name":"run","arguments":{"project":"p","command":["a",1]}}"#));
        assert!(m.contains("at least one item is not a string"), "{m}");
        let m = tool_msg(prep(r#"{"name":"run","arguments":{"project":"p","command":[]}}"#));
        assert!(m.contains("must not be an empty array"), "{m}");
        let m = tool_msg(prep(r#"{"name":"run","arguments":{"project":"p"}}"#));
        assert!(m.contains("missing required argument `command`"), "{m}");
        let m = tool_msg(prep(r#"{"name":"build","arguments":{"project":"a","project":"b"}}"#));
        assert!(m.contains("more than once"), "{m}");
        let m = tool_msg(prep(r#"{"name":"tools","arguments":{"name":"--json"}}"#));
        assert!(m.contains("cannot be passed"), "{m}");
    }

    #[test]
    fn watch_is_refused_with_an_explanation() {
        let m = tool_msg(prep(r#"{"name":"watch","arguments":{}}"#));
        assert!(m.contains("not available over MCP") && m.contains("background") && m.contains("mcp_callable"), "{m}");
        assert!(m.contains("watch"), "{m}");
    }

    #[test]
    fn valid_calls_become_command_lines() {
        let args = |p: &str| prep(p).unwrap().args;
        assert_eq!(args(r#"{"name":"status"}"#), ["status"]);
        assert_eq!(args(r#"{"name":"status","arguments":null}"#), ["status"]);
        assert_eq!(args(r#"{"name":"check","arguments":{"project":"crash-sim"}}"#), ["check", "crash-sim"]);
        assert_eq!(args(r#"{"name":"resume","arguments":{"force":true}}"#), ["resume", "--force"]);
        assert_eq!(args(r#"{"name":"resume","arguments":{"force":false}}"#), ["resume"]);
        assert_eq!(args(r#"{"name":"run","arguments":{"project":"p","command":["cargo","run","--release"]}}"#), ["run", "p", "--", "cargo", "run", "--release"]);
        assert_eq!(args(r#"{"name":"abandon","arguments":{"id":"j1"}}"#), ["recover", "--abandon", "j1"]);
        assert_eq!(args(r#"{"name":"tools","arguments":{"name":"check","json":true}}"#), ["tools", "check", "--json"]);
    }

    /// For every tool, arguments built from its own published schema pass validation and come back as that tool: the
    /// schema (tools/list), the validator and `to_args`/`parse` cannot drift apart.
    #[test]
    fn arguments_built_from_each_schema_validate_and_resolve() {
        let J::Arr(list) = json::parse(&tools::registry_json()).unwrap() else { panic!() };
        for j in &list {
            let name = j.get("name").and_then(J::str).unwrap();
            let Some(J::Obj(props)) = j.get("inputSchema").and_then(|s| s.get("properties")) else { panic!() };
            let sample: Vec<String> = props
                .iter()
                .map(|(k, v)| {
                    let val = match v.get("type").and_then(J::str) {
                        Some("string") => r#""x""#,
                        Some("boolean") => "true",
                        Some("integer") => "7",
                        Some("array") => r#"["a","b"]"#,
                        other => panic!("{name}.{k}: type {other:?}"),
                    };
                    format!("\"{k}\":{val}")
                })
                .collect();
            let p = format!(r#"{{"name":"{name}","arguments":{{{}}}}}"#, sample.join(","));
            let tool = tools::find(name).unwrap();
            if tool.mcp_callable {
                let got = prep(&p).unwrap_or_else(|e| panic!("{p}: {e:?}"));
                assert_eq!(got.tool.name, name);
            } else {
                assert!(matches!(prep(&p), Err(Reject::Tool(_))));
            }
        }
    }

    fn captured(code: Option<i32>, out: &str, err: &str) -> Captured {
        Captured { code, stdout: out.into(), stderr: err.into(), ..Captured::default() }
    }

    #[test]
    fn result_text_is_stdout_then_stderr_then_exit_code() {
        assert_eq!(render_text(&captured(Some(0), "a\nb\n", ""), 1), "a\nb\nexit code: 0");
        assert_eq!(render_text(&captured(Some(0), "no newline", ""), 1), "no newline\nexit code: 0");
        assert_eq!(render_text(&captured(Some(1), "out\n", "bad\n"), 1), "out\nstderr:\nbad\nexit code: 1");
        assert_eq!(render_text(&captured(Some(3), "", "refused"), 1), "stderr:\nrefused\nexit code: 3");
        let cut = Captured { stdout_cut: true, ..captured(Some(0), "tail\n", "") };
        assert_eq!(render_text(&cut, 7), "[stdout truncated: only its last 7 KB are shown]\ntail\nexit code: 0");
        let to = Captured { timed_out_after: Some(5), ..captured(None, "partial\n", "") };
        let t = render_text(&to, 1);
        assert!(t.starts_with("timed out after 5 s") && t.contains("partial") && t.ends_with("exit code: none (killed)"), "{t}");
    }

    #[test]
    fn structured_content_only_when_asked_and_iserror_follows_the_exit_code() {
        let ok = call_result(&captured(Some(0), "o", ""), 1, true);
        assert_eq!(ok.get("isError"), Some(&J::Bool(false)));
        let s = ok.get("structuredContent").unwrap();
        assert_eq!(s.to_json(), r#"{"exit_code":0,"stdout":"o","stderr":"","truncated":false}"#);
        let J::Arr(c) = ok.get("content").unwrap() else { panic!() };
        assert_eq!(c[0].get("type").and_then(J::str), Some("text"));
        assert!(call_result(&captured(Some(0), "o", ""), 1, false).get("structuredContent").is_none());
        let bad = call_result(&captured(Some(1), "", "e"), 1, true);
        assert_eq!(bad.get("isError"), Some(&J::Bool(true)));
        assert_eq!(bad.get("structuredContent").and_then(|s| s.get("exit_code")), Some(&J::Num("1".into())));
        let to = call_result(&Captured { timed_out_after: Some(1), ..captured(None, "", "") }, 1, true);
        assert_eq!((to.get("isError"), to.get("structuredContent").and_then(|s| s.get("exit_code"))), (Some(&J::Bool(true)), Some(&J::Num("-1".into()))));
        let cut = call_result(&Captured { stderr_cut: true, ..captured(Some(0), "", "x") }, 1, true);
        assert_eq!(cut.get("structuredContent").and_then(|s| s.get("truncated")), Some(&J::Bool(true)));
        assert_eq!(tool_error("m").get("isError"), Some(&J::Bool(true)));
        assert!(tool_error("m").get("structuredContent").is_none());
    }

    #[test]
    fn session_structured_by_version() {
        assert!(!Session::default().structured());
        for v in SUPPORTED_VERSIONS {
            assert_eq!(init_session(v).structured(), *v >= "2025-06-18", "{v}");
        }
    }

    #[test]
    fn tail_keeps_the_end_and_never_splits_a_character() {
        let mut t = Tail::new(10);
        t.push(b"hello");
        assert_eq!(t.text(), ("hello".to_string(), false));
        for _ in 0..100 {
            t.push(b"0123456789");
        }
        t.push(b"END");
        let (text, cut) = t.text();
        assert!(cut && text.len() == 10 && text.ends_with("789END"), "{text}");
        assert_eq!(t.total, 5 + 1000 + 3);
        let mut t = Tail::new(5);
        t.push("aé€😀".as_bytes()); // 1 + 2 + 3 + 4 bytes
        let (text, cut) = t.text();
        assert!(cut && text == "😀", "{text:?}");
        let mut t = Tail::new(0);
        t.push(b"abc");
        assert_eq!(t.text(), (String::new(), true));
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
            assert!(matches!(handle(m, &mut what, &mut Session::default()), Handled::Reply(None)), "{m}");
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
        handle(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2099-01-01"}}"#, &mut what, &mut Session::default());
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
        serve(Cursor::new(input.to_vec()), &mut out, &mut Log::none(), &Ctx::none()).unwrap();
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
    fn tools_call_without_a_loadable_config_is_a_tool_error_not_a_crash() {
        let out = run_lines(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"status\"}}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"nope\"}}\n");
        assert_eq!(out[0].get("result").and_then(|r| r.get("isError")), Some(&J::Bool(true)));
        assert!(out[0].to_json().contains("broadsword.toml could not be loaded"));
        assert_eq!(err_code(&out[1]), "-32602");
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
        assert!(serve(Cursor::new(input.to_vec()), Broken, &mut Log::none(), &Ctx::none()).is_ok());
    }

    #[test]
    fn mcp_is_not_in_the_registry() {
        assert!(tools::resolve(&["mcp"]).is_none());
        assert!(tools::find("mcp").is_none());
    }
}
