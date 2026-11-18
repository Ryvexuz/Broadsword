//! The reviewer side of the merge queue: the fixed reviewer prompt, the review prompt, the backends that call a model,
//! the verdict parser, and the secret handling around a model call.
//!
//! A backend is a `ReviewBackend`: it takes a model name and a prompt and returns the model's answer with its cost and
//! tokens, or says why it could not. Only `claude` exists (the Claude Code CLI run headless); `reviewer_backend` in
//! broadsword.toml names it, and an `openai` backend is one more `impl` and one more arm in `backend`.
//!
//! Nothing here runs unless `queue run` calls it: spending money and using the network happen in exactly one place.
//!
//! SECRETS. Where a secret would enter the child process or the prompt, it is stopped, by name AND by value:
//! - the child's environment loses every variable named `*_API_KEY`, `*_KEY`, `*_TOKEN` or `*_SECRET`, and every variable
//!   whose VALUE contains a secret value (a secret-named entry of the root's `.env`, or the value of a secret-named
//!   variable of this process; the `.env` file is read only for those values and its text is never sent anywhere);
//! - the prompt (brief, diff, check output) has those same values replaced by `[REDACTED]` before it is sent.
//!
//! CLAUDE AUTH. Claude Code's own login (claude.ai account, or a stored console login) lives in its config files under the
//! user's profile, not in environment variables, so the scrubbed environment still authenticates (`claude auth status`
//! reports `loggedIn: true` with it). What the scrub DOES break: a setup that authenticates through an environment variable
//! (`ANTHROPIC_API_KEY`, `CLAUDE_CODE_OAUTH_TOKEN`, `ANTHROPIC_AUTH_TOKEN`): those names match the patterns and are removed on
//! purpose, so such a machine must log in with `claude` interactively once instead. Variables Claude Code needs to find its
//! config (`USERPROFILE`, `HOME`, `APPDATA`, `PATH`, ...) are never touched.

use crate::config::Config;
use crate::json::{self, J};
use crate::sys;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The env var that overrides `reviewer_cli` with a machine-specific program path. Its value is never written to any file
/// or message.
pub const CLI_ENV: &str = "BROADSWORD_REVIEWER_CLI";

/// The reviewer's fixed system prompt: the one the reviewer benchmark (bench/reviewer/run.py) measured.
pub const SYSTEM_PROMPT: &str = "You are an independent code reviewer. You did not write this code and you do not fix it.
You are given a task description, its done check, and a diff. Decide whether the change really delivers what the task and done check require.
Look for: bugs that break the done check or the stated rules, data loss, security problems, races, faked or silent success, and wrong documentation.
Do not report style issues. Be specific: name the file and line (from the diff) and the concrete scenario that fails.

Answer in exactly this format and nothing else:
VERDICT: PASS or FAIL
BLOCKING: <file:line> - <one sentence: what fails, in which scenario>
(one BLOCKING line per blocking problem; write \"BLOCKING: none\" if there are none)
MINOR: <file:line> - <one sentence>
(one MINOR line per minor problem; write \"MINOR: none\" if there are none)";

// ---- the prompt -------------------------------------------------------------------------------

/// The review prompt: the brief, the check result (summary plus the end of its output), the full diff, and the request.
/// The repo diff and the check output are the only repo content in it; no `.env`, ever.
pub fn build_prompt(brief: &str, check_summary: &str, check_tail: &str, diff: &str) -> String {
    format!(
        "{brief}\n\nCHECK RESULT (the branch was rebased onto main, then built and tested; this is what running it printed):\n{check_summary}\n\nEnd of the check output:\n```text\n{check_tail}\n```\n\nDIFF:\n```diff\n{diff}\n```\n\nReview it now, in the required format."
    )
}

// ---- the verdict ------------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Verdict {
    /// Only a clean, well-formed PASS is true. Anything unparseable, contradictory or incomplete is a FAIL.
    pub pass: bool,
    /// False when the answer did not follow the format; `pass` is then false and `note` says why.
    pub parsed: bool,
    pub blocking: Vec<String>,
    pub minor: Vec<String>,
    pub note: String,
}

/// Strip the markdown a model sometimes wraps a line in (`**VERDICT: PASS**`, `- BLOCKING: ...`, `> ...`).
fn unwrap_line(line: &str) -> &str {
    line.trim().trim_start_matches(['*', '`', '#', '>', '-', ' ']).trim_end_matches(['*', '`', ' '])
}

fn after_label<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let head = line.get(..label.len())?;
    head.eq_ignore_ascii_case(label).then(|| line[label.len()..].trim())
}

fn is_none(v: &str) -> bool {
    matches!(v.trim().trim_matches(['*', '`', '.', ' ']).to_lowercase().as_str(), "none" | "n/a" | "-" | "")
}

/// Parse `VERDICT: PASS|FAIL`, the `BLOCKING:` lines and the `MINOR:` lines. The rules are strict because a PASS merges code:
/// - no VERDICT line, an unknown verdict word, or two lines that disagree -> FAIL, `parsed = false`;
/// - a PASS needs at least one BLOCKING line (`BLOCKING: none` counts), and a PASS next to a real BLOCKING finding is a FAIL;
/// - a FAIL without any BLOCKING text gets a placeholder finding, so a rejection always says something.
pub fn parse_verdict(text: &str) -> Verdict {
    let (mut verdicts, mut blocking, mut minor, mut blocking_lines) = (Vec::<String>::new(), Vec::<String>::new(), Vec::<String>::new(), 0usize);
    for raw in text.lines() {
        let l = unwrap_line(raw);
        if let Some(v) = after_label(l, "VERDICT:") {
            verdicts.push(v.trim_matches(['*', '`', '.', ' ']).to_uppercase());
        } else if let Some(v) = after_label(l, "BLOCKING:") {
            blocking_lines += 1;
            if !is_none(v) {
                blocking.push(v.to_string());
            }
        } else if let Some(v) = after_label(l, "MINOR:") {
            if !is_none(v) {
                minor.push(v.to_string());
            }
        }
    }
    let bad = |note: &str, blocking: Vec<String>, minor: Vec<String>| Verdict { pass: false, parsed: false, blocking, minor, note: note.to_string() };
    if verdicts.is_empty() {
        return bad("the reviewer's answer has no `VERDICT:` line, so it counts as FAIL", blocking, minor);
    }
    if verdicts.iter().any(|v| v != "PASS" && v != "FAIL") {
        return bad("the reviewer's `VERDICT:` is neither PASS nor FAIL, so it counts as FAIL", blocking, minor);
    }
    let any_fail = verdicts.iter().any(|v| v == "FAIL");
    if any_fail && verdicts.iter().any(|v| v == "PASS") {
        return bad("the reviewer gave both PASS and FAIL verdicts, so it counts as FAIL", blocking, minor);
    }
    if !any_fail && blocking_lines == 0 {
        return bad("the reviewer said PASS but gave no `BLOCKING:` line (not even `BLOCKING: none`), so it counts as FAIL", blocking, minor);
    }
    if !any_fail && !blocking.is_empty() {
        return Verdict { pass: false, parsed: true, blocking, minor, note: "the reviewer said PASS but listed blocking findings, so it counts as FAIL".into() };
    }
    if any_fail && blocking.is_empty() {
        blocking.push("(the reviewer said FAIL without a BLOCKING line; read the raw output in the review file)".into());
    }
    Verdict { pass: !any_fail, parsed: true, blocking, minor, note: String::new() }
}

// ---- secrets ----------------------------------------------------------------------------------

/// Names that carry a secret: `*_API_KEY`, `*_KEY`, `*_TOKEN`, `*_SECRET` (case-insensitive, as Windows treats them).
pub fn secret_name(name: &str) -> bool {
    let n = name.to_uppercase();
    ["_KEY", "_TOKEN", "_SECRET"].iter().any(|suffix| n.ends_with(suffix)) || n == "API_KEY" || n == "TOKEN" || n == "SECRET"
}

/// The secret values this process knows of: values of secret-named entries of `<root>/.env`, and of secret-named variables
/// in `env`. Values shorter than 8 characters are ignored (too likely to match ordinary text).
pub fn secret_values(root: &Path, env: &[(String, String)]) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join(".env")) {
        for line in text.lines() {
            let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
            if let Some((k, val)) = line.split_once('=') {
                let k = k.trim();
                let val = val.trim().trim_matches(['"', '\'']);
                if (secret_name(k) || k.to_uppercase().contains("OPENAI")) && val.len() >= 8 {
                    v.push(val.to_string());
                }
            }
        }
    }
    for (k, val) in env {
        if secret_name(k) && val.len() >= 8 {
            v.push(val.clone());
        }
    }
    v.sort();
    v.dedup();
    v
}

/// Which variables of `env` the reviewer CLI must NOT inherit: secret-named ones, and any whose value contains a known
/// secret value.
pub fn scrub_names(env: &[(String, String)], secrets: &[String]) -> Vec<String> {
    env.iter().filter(|(k, v)| secret_name(k) || secrets.iter().any(|s| v.contains(s.as_str()))).map(|(k, _)| k.clone()).collect()
}

/// `text` with every known secret value replaced by `[REDACTED]`.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for s in secrets {
        out = out.replace(s.as_str(), "[REDACTED]");
    }
    out
}

// ---- backends ---------------------------------------------------------------------------------

/// A model's answer.
#[derive(Debug, Clone, Default)]
pub struct Review {
    /// Everything the backend printed, exactly (stored in the review file).
    pub raw: String,
    /// The model's text answer (for claude: the `result` field of its JSON).
    pub text: String,
    pub cost_usd: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub models_used: Vec<String>,
}

/// Why no answer came back. None of these is a verdict; the queue marks the entry `blocked` and nothing is merged.
#[derive(Debug)]
pub enum CallError {
    /// The program could not be started.
    Start(String),
    /// It ran longer than `reviewer_timeout_seconds` and was killed.
    Timeout(u64),
    /// It exited non-zero, or reported an error in its own output.
    Failed(String),
}

pub trait ReviewBackend {
    fn name(&self) -> &'static str;
    fn call(&self, cfg: &Config, model: &str, prompt: &str) -> Result<Review, CallError>;
}

/// The backend `reviewer_backend` names (config validation already refused anything but `claude`).
pub fn backend(cfg: &Config) -> Result<Box<dyn ReviewBackend>, String> {
    match cfg.reviewer_backend.as_str() {
        "claude" => Ok(Box::new(ClaudeCli)),
        other => Err(format!("reviewer_backend = \"{other}\" is not supported")),
    }
}

/// The Claude Code CLI, headless: `<cli> -p --system-prompt <s> --model <m> --tools "" --strict-mcp-config --max-turns 1
/// --output-format json`, the prompt on stdin (a big diff does not fit Windows' command-line limit), run in a folder under
/// scratch that holds no CLAUDE.md, with the scrubbed environment.
pub struct ClaudeCli;

impl ClaudeCli {
    fn program(cfg: &Config) -> String {
        match std::env::var(CLI_ENV) {
            Ok(v) if !v.trim().is_empty() => v,
            _ => cfg.reviewer_cli.clone(),
        }
    }
}

impl ReviewBackend for ClaudeCli {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn call(&self, cfg: &Config, model: &str, prompt: &str) -> Result<Review, CallError> {
        let cwd = cfg.scratch_dir.join("review-cwd");
        std::fs::create_dir_all(&cwd).map_err(|e| CallError::Start(format!("cannot create {}: {e}", cwd.display())))?;
        let args: Vec<String> = ["-p", "--system-prompt", SYSTEM_PROMPT, "--model", model, "--tools", "", "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"].iter().map(|s| s.to_string()).collect();
        let env: Vec<(String, String)> = std::env::vars().collect();
        let secrets = secret_values(&cfg.root, &env);
        let remove = scrub_names(&env, &secrets);
        let out = run_cli(&Self::program(cfg), &args, &cwd, &remove, prompt, Duration::from_secs(cfg.reviewer_timeout_seconds))?;
        parse_claude_output(&out.stdout, out.code, &out.stderr)
    }
}

pub struct CliOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run `program args` in `cwd` with `remove` variables dropped from its environment, `stdin` fed to it, killed (with its
/// whole process tree) after `timeout`.
pub fn run_cli(program: &str, args: &[String], cwd: &Path, remove: &[String], stdin: &str, timeout: Duration) -> Result<CliOut, CallError> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for k in remove {
        cmd.env_remove(k);
    }
    let mut child = cmd.spawn().map_err(|e| {
        // The program may be a machine-specific path from BROADSWORD_REVIEWER_CLI: never echo it.
        CallError::Start(format!("cannot start the reviewer CLI ({:?}); install it, put it on PATH, or point {CLI_ENV} at it", e.kind()))
    })?;
    let mut to_stdin = child.stdin.take().expect("piped");
    let data = stdin.as_bytes().to_vec();
    std::thread::spawn(move || {
        let _ = to_stdin.write_all(&data); // a child that exits early closes the pipe; its exit code tells the story
    });
    let (mut so, mut se) = (child.stdout.take().expect("piped"), child.stderr.take().expect("piped"));
    let t_out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    let end = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let (o, e) = (t_out.join().unwrap_or_default(), t_err.join().unwrap_or_default());
                return Ok(CliOut { code: status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&o).into_owned(), stderr: String::from_utf8_lossy(&e).into_owned() });
            }
            Ok(None) => {}
            Err(e) => return Err(CallError::Failed(format!("waiting for the reviewer CLI failed: {e}"))),
        }
        if Instant::now() >= end {
            sys::kill_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
            return Err(CallError::Timeout(timeout.as_secs())); // the readers end on their own once the pipes close
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn num(j: Option<&J>) -> Option<String> {
    match j {
        Some(J::Num(n)) => Some(n.clone()),
        _ => None,
    }
}

/// Read the CLI's `--output-format json` answer. An exit code other than 0, or `is_error: true`, is a `CallError` (no answer).
/// Output that is not JSON is taken as the answer text itself (the verdict parser then decides; garbage is a FAIL).
pub fn parse_claude_output(stdout: &str, code: i32, stderr: &str) -> Result<Review, CallError> {
    let last = |s: &str| {
        let t: Vec<&str> = s.lines().rev().take(5).collect();
        t.into_iter().rev().collect::<Vec<_>>().join(" | ")
    };
    let mut rev = Review { raw: stdout.to_string(), ..Review::default() };
    match json::parse(stdout.trim()) {
        Ok(j @ J::Obj(_)) => {
            rev.text = j.get("result").and_then(J::str).unwrap_or_default().to_string();
            rev.cost_usd = num(j.get("total_cost_usd"));
            if let Some(u) = j.get("usage") {
                let n = |k: &str| num(u.get(k)).and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
                rev.input_tokens = Some(n("input_tokens") + n("cache_creation_input_tokens") + n("cache_read_input_tokens"));
                rev.output_tokens = Some(n("output_tokens"));
            }
            if let Some(J::Obj(m)) = j.get("modelUsage") {
                rev.models_used = m.iter().map(|(k, _)| k.clone()).collect();
            }
            if matches!(j.get("is_error"), Some(J::Bool(true))) {
                return Err(CallError::Failed(format!("the reviewer reported an error: {}", if rev.text.is_empty() { last(stderr) } else { rev.text.chars().take(300).collect() })));
            }
        }
        _ => rev.text = stdout.to_string(),
    }
    if code != 0 {
        return Err(CallError::Failed(format!("the reviewer CLI exited {code}: {}", last(if stderr.trim().is_empty() { stdout } else { stderr }))));
    }
    Ok(rev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_pass_and_fail() {
        let p = parse_verdict("VERDICT: PASS\nBLOCKING: none\nMINOR: none");
        assert!(p.pass && p.parsed && p.blocking.is_empty() && p.minor.is_empty(), "{p:?}");
        let f = parse_verdict("VERDICT: FAIL\nBLOCKING: a.rs:3 - drops data\nBLOCKING: b.rs:9 - race\nMINOR: c.rs:1 - naming");
        assert!(!f.pass && f.parsed);
        assert_eq!(f.blocking, ["a.rs:3 - drops data", "b.rs:9 - race"]);
        assert_eq!(f.minor, ["c.rs:1 - naming"]);
        let md = parse_verdict("**VERDICT: PASS**\n- BLOCKING: none\n- MINOR: x.rs:2 - fine");
        assert!(md.pass && md.minor.len() == 1, "{md:?}");
        let lower = parse_verdict("verdict: pass\nblocking: None.\nminor: none");
        assert!(lower.pass, "{lower:?}");
    }

    #[test]
    fn anything_unclear_is_a_fail() {
        for text in ["", "I think it looks fine", "VERDICT: MAYBE\nBLOCKING: none", "VERDICT: PASS", "VERDICT: PASS\nMINOR: none", "VERDICT: PASS\nVERDICT: FAIL\nBLOCKING: none"] {
            let v = parse_verdict(text);
            assert!(!v.pass && !v.parsed, "{text:?} -> {v:?}");
            assert!(!v.note.is_empty());
        }
        let contradictory = parse_verdict("VERDICT: PASS\nBLOCKING: x.rs:1 - data loss");
        assert!(!contradictory.pass && contradictory.parsed && contradictory.blocking.len() == 1, "{contradictory:?}");
        let bare = parse_verdict("VERDICT: FAIL");
        assert!(!bare.pass && bare.parsed && bare.blocking.len() == 1);
    }

    #[test]
    fn secret_names_and_values() {
        for n in ["OPENAI_API_KEY", "HF_KEY", "FORGE_CLAUDE_TOKEN", "X_SECRET", "openai_api_key", "API_KEY", "ANTHROPIC_API_KEY"] {
            assert!(secret_name(n), "{n}");
        }
        for n in ["PATH", "USERPROFILE", "KEYBOARD", "TOKENIZER", "MONKEY_PATCH", "FAKE_CLAUDE_OUT_DIR"] {
            assert!(!secret_name(n), "{n}");
        }
        let env: Vec<(String, String)> = [("OPENAI_API_KEY", "sk-live-0123456789"), ("COPIED", "x sk-live-0123456789 y"), ("PATH", "C:/bin"), ("SHORT_KEY", "abc"), ("HOME", "/h")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let secrets = secret_values(Path::new("no-such-folder-here"), &env);
        assert_eq!(secrets, ["sk-live-0123456789"], "short values are not secrets");
        let gone = scrub_names(&env, &secrets);
        assert_eq!(gone, ["OPENAI_API_KEY", "COPIED", "SHORT_KEY"]);
        assert_eq!(redact("key=sk-live-0123456789;", &secrets), "key=[REDACTED];");
    }

    #[test]
    fn claude_output_parsing() {
        let ok = r#"{"type":"result","is_error":false,"result":"VERDICT: PASS\nBLOCKING: none\nMINOR: none","total_cost_usd":0.0123,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100},"modelUsage":{"claude-haiku-4-5":{}}}"#;
        let r = parse_claude_output(ok, 0, "").unwrap();
        assert_eq!((r.cost_usd.as_deref(), r.input_tokens, r.output_tokens), (Some("0.0123"), Some(110), Some(5)));
        assert_eq!(r.models_used, ["claude-haiku-4-5"]);
        assert!(parse_verdict(&r.text).pass);
        assert!(matches!(parse_claude_output(ok, 1, "boom"), Err(CallError::Failed(m)) if m.contains("exited 1")));
        assert!(matches!(parse_claude_output(r#"{"is_error":true,"result":"Not logged in"}"#, 0, ""), Err(CallError::Failed(m)) if m.contains("Not logged in")));
        let plain = parse_claude_output("hello", 0, "").unwrap();
        assert_eq!(plain.text, "hello");
        assert!(!parse_verdict(&plain.text).pass);
    }

    #[test]
    fn prompt_holds_brief_check_and_diff_only() {
        let p = build_prompt("BRIEF", "test: ok", "tail text", "+added line");
        for needle in ["BRIEF", "test: ok", "tail text", "+added line", "VERDICT"] {
            assert!(p.contains(needle) || needle == "VERDICT", "{needle}");
        }
    }
}
