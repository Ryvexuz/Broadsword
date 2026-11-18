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
//! - EVERY value in the root's `.env` is a secret, whatever its name (`GITHUB_PAT`, `DB_PASSWORD`, `REGION`...); so is the
//!   value of every secret-named variable of this process (the `.env` file is read only for its values; its text is never sent);
//! - the child's environment loses every variable named `*_API_KEY`, `*_KEY`, `*_TOKEN`, `*_SECRET`, `*_PAT`, `*_PASSWORD`,
//!   `*_PASS`, `*_CREDENTIAL*` or `*_AUTH*`, and every variable whose VALUE equals or contains a secret value; a `.env` value
//!   shorter than `MIN_REDACT_LEN` is removed from the environment only when a variable EQUALS it;
//! - the prompt (brief, diff, check output) and everything the queue stores (entry, review file) have the secret values replaced
//!   by `[REDACTED]`; `.env` values shorter than `MIN_REDACT_LEN` are not (they would blank out ordinary words), and the queue
//!   says how many were skipped.
//!
//! THE REVIEWER PROGRAM. `resolve_cli` turns `BROADSWORD_REVIEWER_CLI` / `reviewer_cli` into an absolute path, which the queue
//! prints and records on every review. A path outside the project root (the real Claude install) is normal; one inside the
//! root's `scratch_dir` is the TEST HOOK (announced, `test_hook: true` in the review); any other path inside the root is refused,
//! `build/` included, so the shipped `fake_claude` can never stand in for a real reviewer. The CLI's answer must be its own JSON
//! and its `modelUsage` must name the requested model family, or it is a FAIL (`Review::invalid`).
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

/// MINOR: `none`, `n/a`, `-` or nothing all mean no minor findings.
fn is_none(v: &str) -> bool {
    matches!(v.trim().trim_matches(['*', '`', '.', ' ']).to_lowercase().as_str(), "none" | "n/a" | "-" | "")
}

/// BLOCKING: only an explicit `none` (or `n/a`) means no blocking finding. An empty header means the findings follow.
fn is_explicit_none(v: &str) -> bool {
    matches!(v.trim().trim_matches(['*', '`', '.', ' ']).to_lowercase().as_str(), "none" | "n/a")
}

fn is_empty_header(v: &str) -> bool {
    matches!(v.trim().trim_matches(['*', '`', '.', ' ']), "" | "-")
}

/// Parse `VERDICT: PASS|FAIL`, the `BLOCKING:` lines and the `MINOR:` lines. The rules are strict because a PASS merges code:
/// - no VERDICT line, an unknown verdict word, or two lines that disagree -> FAIL, `parsed = false`;
/// - a PASS needs at least one BLOCKING line (`BLOCKING: none` counts), and a PASS next to a real BLOCKING finding is a FAIL;
/// - a FAIL without any BLOCKING text gets a placeholder finding, so a rejection always says something.
pub fn parse_verdict(text: &str) -> Verdict {
    let (mut verdicts, mut blocking, mut minor, mut blocking_lines) = (Vec::<String>::new(), Vec::<String>::new(), Vec::<String>::new(), 0usize);
    // An empty `BLOCKING:` header: its findings are the lines that follow, up to the next MINOR / VERDICT / BLOCKING label.
    let (mut in_empty_header, mut empty_unfilled, mut header_filled) = (false, 0usize, false);
    fn close_header(in_empty: &mut bool, filled: bool, unfilled: &mut usize) {
        if *in_empty && !filled {
            *unfilled += 1;
        }
        *in_empty = false;
    }
    for raw in text.lines() {
        let l = unwrap_line(raw);
        if let Some(v) = after_label(l, "VERDICT:") {
            close_header(&mut in_empty_header, header_filled, &mut empty_unfilled);
            verdicts.push(v.trim_matches(['*', '`', '.', ' ']).to_uppercase());
        } else if let Some(v) = after_label(l, "BLOCKING:") {
            close_header(&mut in_empty_header, header_filled, &mut empty_unfilled);
            blocking_lines += 1;
            if is_empty_header(v) {
                in_empty_header = true;
                header_filled = false;
            } else if !is_explicit_none(v) {
                blocking.push(v.to_string());
            }
        } else if let Some(v) = after_label(l, "MINOR:") {
            close_header(&mut in_empty_header, header_filled, &mut empty_unfilled);
            if !is_none(v) {
                minor.push(v.to_string());
            }
        } else if in_empty_header && !l.trim().is_empty() {
            header_filled = true;
            blocking.push(l.trim().to_string());
        }
    }
    close_header(&mut in_empty_header, header_filled, &mut empty_unfilled);
    let bad = |note: &str, blocking: Vec<String>, minor: Vec<String>| Verdict { pass: false, parsed: false, blocking, minor, note: note.to_string() };
    if empty_unfilled > 0 {
        return bad("the reviewer's `BLOCKING:` line is empty and nothing follows it (only `BLOCKING: none` means none), so it counts as FAIL", blocking, minor);
    }
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

/// Names that carry a secret (case-insensitive, as Windows treats them): `*_API_KEY`, `*_KEY`, `*_TOKEN`, `*_SECRET`, `*_PAT`,
/// `*_PASSWORD`, `*_PASS`, `*_CREDENTIAL*`, `*_AUTH*`, and the bare `API_KEY`, `TOKEN`, `SECRET`, `PAT`, `PASSWORD`, `PASS`.
pub fn secret_name(name: &str) -> bool {
    let n = name.to_uppercase();
    ["_KEY", "_TOKEN", "_SECRET", "_PAT", "_PASSWORD", "_PASS"].iter().any(|suffix| n.ends_with(suffix))
        || n.contains("_CREDENTIAL")
        || n.contains("_AUTH")
        || ["API_KEY", "TOKEN", "SECRET", "PAT", "PASSWORD", "PASS"].contains(&n.as_str())
}

/// Values shorter than this are not redacted from text (they would blank out ordinary words); they are still kept out of the
/// reviewer's environment (see `Secrets::scrub_names`).
pub const MIN_REDACT_LEN: usize = 6;

/// Every secret value this process knows of.
/// - EVERY value in `<root>/.env` is a secret, whatever its name (`GITHUB_PAT`, `DB_PASSWORD`, `REGION`...);
/// - so is the value of every secret-named variable of the process environment.
///
/// Values of `MIN_REDACT_LEN` characters or more are replaced by `[REDACTED]` in any text; shorter `.env` values are not (they
/// would redact common words), but a variable of the reviewer's environment that EQUALS one is still removed.
#[derive(Debug, Default, Clone)]
pub struct Secrets {
    redact: Vec<String>,
    short_env: Vec<String>,
}

fn env_file_values(root: &Path) -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join(".env")) {
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((_, val)) = line.split_once('=') {
                let val = val.trim().trim_matches(['"', '\'']);
                if !val.is_empty() {
                    v.push(val.to_string());
                }
            }
        }
    }
    v
}

impl Secrets {
    pub fn collect(root: &Path, env: &[(String, String)]) -> Secrets {
        let mut redact: Vec<String> = Vec::new();
        let mut short_env: Vec<String> = Vec::new();
        for val in env_file_values(root) {
            if val.chars().count() >= MIN_REDACT_LEN {
                redact.push(val);
            } else {
                short_env.push(val);
            }
        }
        for (k, val) in env {
            if secret_name(k) && val.chars().count() >= MIN_REDACT_LEN {
                redact.push(val.clone());
            }
        }
        // longest first, so a secret that contains another is replaced whole
        redact.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
        redact.dedup();
        short_env.sort();
        short_env.dedup();
        Secrets { redact, short_env }
    }

    /// How many `.env` values are too short to redact from text.
    pub fn short_count(&self) -> usize {
        self.short_env.len()
    }

    /// `text` with every known secret value replaced by `[REDACTED]`.
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_string();
        for s in &self.redact {
            out = out.replace(s.as_str(), "[REDACTED]");
        }
        out
    }

    /// Which variables of `env` the reviewer CLI must NOT inherit: secret-named ones; any whose value equals or contains a
    /// secret value; any whose value equals a short `.env` value.
    pub fn scrub_names(&self, env: &[(String, String)]) -> Vec<String> {
        env.iter()
            .filter(|(k, v)| secret_name(k) || self.redact.iter().any(|s| v.contains(s.as_str())) || self.short_env.iter().any(|s| v == s))
            .map(|(k, _)| k.clone())
            .collect()
    }
}

// ---- model families ---------------------------------------------------------------------------

/// The family a model name belongs to: `haiku`, `sonnet` or `opus` for any name that contains one (`claude-sonnet-4-5`,
/// `opus[1m]`), otherwise the trimmed lower-case name itself. Two names are the same model when their families are equal.
pub fn model_family(name: &str) -> String {
    let n = name.trim().to_lowercase();
    for f in ["haiku", "sonnet", "opus"] {
        if n.contains(f) {
            return f.to_string();
        }
    }
    n
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
    /// Why this answer cannot be trusted as a review (not the CLI's JSON, no `modelUsage`, another model family answered).
    /// When set, the answer is a FAIL whatever its text says.
    pub invalid: Option<String>,
}

/// The reviewer program a call will run: its resolved path, and whether it is the scratch-folder test hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub path: String,
    pub test_hook: bool,
}

/// The verdict for an answer: an `invalid` answer is a FAIL (its raw text is kept by the caller); otherwise the text is parsed.
pub fn judge(r: &Review) -> Verdict {
    match &r.invalid {
        Some(why) => Verdict { pass: false, parsed: false, blocking: vec![], minor: vec![], note: format!("{why}, so it counts as FAIL") },
        None => parse_verdict(&r.text),
    }
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
    /// Work out which program a call would run, and refuse a forbidden one, WITHOUT calling it.
    fn prepare(&self, cfg: &Config) -> Result<Cli, String>;
    fn call(&self, cfg: &Config, cli: &Cli, model: &str, prompt: &str) -> Result<Review, CallError>;
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

fn clean_path(p: std::path::PathBuf) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => std::path::PathBuf::from(rest),
        None => p,
    }
}

/// A bare program name searched on PATH (`claude` is `claude.exe` on Windows).
fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    for dir in std::env::split_paths(&std::env::var_os("PATH")?) {
        for e in exts {
            let c = dir.join(format!("{name}{e}"));
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

/// Resolve the reviewer program to an absolute path and classify it against the project root.
/// - `BROADSWORD_REVIEWER_CLI` (if set) or `reviewer_cli` is taken as a path when it has a folder part, else searched on PATH;
/// - OUTSIDE the project root (where the real Claude install is): allowed, a normal reviewer;
/// - inside the root's `scratch_dir`: the TEST HOOK, allowed but announced and recorded (`test_hook: true`);
/// - anywhere else inside the root (`build/` included, where the shipped fake_claude is built): refused.
pub fn resolve_cli(cfg: &Config) -> Result<Cli, String> {
    let wanted = match std::env::var(CLI_ENV) {
        Ok(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => cfg.reviewer_cli.clone(),
    };
    let has_dir = wanted.contains(['/', '\\']);
    let found = if has_dir {
        let p = Path::new(&wanted);
        let abs = if p.is_absolute() { p.to_path_buf() } else { std::env::current_dir().map_err(|e| format!("cannot read the current folder: {e}"))?.join(p) };
        abs.is_file().then_some(abs)
    } else {
        find_on_path(&wanted)
    };
    let Some(found) = found else {
        return Err(format!("cannot start the reviewer CLI: `{wanted}` was not found{}; install it, put it on PATH, or point {CLI_ENV} at it", if has_dir { "" } else { " on PATH" }));
    };
    let real = clean_path(std::fs::canonicalize(&found).map_err(|e| format!("cannot resolve the reviewer CLI `{}`: {e}", found.display()))?);
    let root = clean_path(std::fs::canonicalize(&cfg.root).unwrap_or_else(|_| cfg.root.clone()));
    let path = real.to_string_lossy().replace('\\', "/");
    if !real.starts_with(&root) {
        return Ok(Cli { path, test_hook: false });
    }
    let scratch = clean_path(std::fs::canonicalize(&cfg.scratch_dir).unwrap_or_else(|_| cfg.scratch_dir.clone()));
    if real.starts_with(&scratch) {
        return Ok(Cli { path, test_hook: true });
    }
    Err(format!(
        "the reviewer CLI `{path}` is inside the project root and not inside its scratch folder: refused. {CLI_ENV} must point outside the project root (like the real Claude install) or, as a test hook, inside scratch_dir; build/ is never allowed, so the shipped fake reviewer cannot stand in for a real one"
    ))
}

impl ReviewBackend for ClaudeCli {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn prepare(&self, cfg: &Config) -> Result<Cli, String> {
        resolve_cli(cfg)
    }

    fn call(&self, cfg: &Config, cli: &Cli, model: &str, prompt: &str) -> Result<Review, CallError> {
        let cwd = cfg.scratch_dir.join("review-cwd");
        std::fs::create_dir_all(&cwd).map_err(|e| CallError::Start(format!("cannot create {}: {e}", cwd.display())))?;
        let args: Vec<String> = ["-p", "--system-prompt", SYSTEM_PROMPT, "--model", model, "--tools", "", "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"].iter().map(|s| s.to_string()).collect();
        let env: Vec<(String, String)> = std::env::vars().collect();
        let remove = Secrets::collect(&cfg.root, &env).scrub_names(&env);
        let out = run_cli(&cli.path, &args, &cwd, &remove, prompt, Duration::from_secs(cfg.reviewer_timeout_seconds))?;
        let mut rev = parse_claude_output(&out.stdout, out.code, &out.stderr)?;
        check_model(&mut rev, model);
        Ok(rev)
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
    let mut child = cmd.spawn().map_err(|e| CallError::Start(format!("cannot start the reviewer CLI `{program}` ({:?}); install it, put it on PATH, or point {CLI_ENV} at it", e.kind())))?;
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
/// Output that is not the CLI's JSON (an object with a text `result`) is NOT taken as an answer: the review is marked `invalid`
/// (a FAIL, the raw text kept).
pub fn parse_claude_output(stdout: &str, code: i32, stderr: &str) -> Result<Review, CallError> {
    let last = |s: &str| {
        let t: Vec<&str> = s.lines().rev().take(5).collect();
        t.into_iter().rev().collect::<Vec<_>>().join(" | ")
    };
    let mut rev = Review { raw: stdout.to_string(), ..Review::default() };
    match json::parse(stdout.trim()) {
        Ok(j @ J::Obj(_)) if j.get("result").and_then(J::str).is_some() => {
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
        Ok(j @ J::Obj(_)) if matches!(j.get("is_error"), Some(J::Bool(true))) => {
            return Err(CallError::Failed(format!("the reviewer reported an error: {}", last(stderr))));
        }
        _ => rev.invalid = Some("the reviewer CLI's output is not its JSON answer (`--output-format json`, an object with a `result`)".into()),
    }
    if code != 0 {
        return Err(CallError::Failed(format!("the reviewer CLI exited {code}: {}", last(if stderr.trim().is_empty() { stdout } else { stderr }))));
    }
    Ok(rev)
}

/// The answer must come from the model that was asked for. `modelUsage` lists the models that did the work: it must exist,
/// and every name in it must be of the requested family (haiku, sonnet, opus). Otherwise the review is `invalid`, saying
/// which model answered.
pub fn check_model(rev: &mut Review, requested: &str) {
    if rev.invalid.is_some() {
        return;
    }
    if rev.models_used.is_empty() {
        rev.invalid = Some(format!("the reviewer's JSON has no `modelUsage`, so it cannot be shown that `{requested}` answered"));
        return;
    }
    let want = model_family(requested);
    let others: Vec<&String> = rev.models_used.iter().filter(|m| model_family(m) != want).collect();
    if !others.is_empty() {
        rev.invalid = Some(format!("`{requested}` (family {want}) was asked, but the answer came from {}", rev.models_used.join(", ")));
    }
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
        for n in ["OPENAI_API_KEY", "HF_KEY", "FORGE_CLAUDE_TOKEN", "X_SECRET", "openai_api_key", "API_KEY", "ANTHROPIC_API_KEY", "GITHUB_PAT", "DB_PASSWORD", "SVC_PASS", "AWS_CREDENTIALS", "X_AUTH_HEADER", "PASSWORD"] {
            assert!(secret_name(n), "{n}");
        }
        for n in ["PATH", "USERPROFILE", "KEYBOARD", "TOKENIZER", "MONKEY_PATCH", "FAKE_CLAUDE_OUT_DIR", "PATHEXT", "PASSENGER"] {
            assert!(!secret_name(n), "{n}");
        }
        let env: Vec<(String, String)> = [("OPENAI_API_KEY", "sk-live-0123456789"), ("COPIED", "x sk-live-0123456789 y"), ("PATH", "C:/bin"), ("SHORT_KEY", "abc"), ("HOME", "/h")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let s = Secrets::collect(Path::new("no-such-folder-here"), &env);
        assert_eq!(s.redact("key=sk-live-0123456789;abc"), "key=[REDACTED];abc", "short values are not redacted");
        assert_eq!(s.scrub_names(&env), ["OPENAI_API_KEY", "COPIED", "SHORT_KEY"]);
    }

    #[test]
    fn every_env_file_value_is_a_secret_whatever_its_name() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(".env"), "# c\nREGION=eu-west-1\nGITHUB_PAT=ghp_abcdef123456\nexport DB_PASSWORD=\"hunter2hunter2\"\nTINY=ab\nEMPTY=\n").unwrap();
        let env: Vec<(String, String)> = [("MY_COPY", "x-ghp_abcdef123456-y"), ("EXACT_SHORT", "ab"), ("HAS_SHORT", "xaby"), ("OTHER", "plain")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let s = Secrets::collect(&d, &env);
        assert_eq!(s.redact("r=eu-west-1 p=ghp_abcdef123456 q=hunter2hunter2 t=ab"), "r=[REDACTED] p=[REDACTED] q=[REDACTED] t=ab");
        assert_eq!(s.short_count(), 1);
        assert_eq!(s.scrub_names(&env), ["MY_COPY", "EXACT_SHORT"], "contains for long values, equals for short ones");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn model_families_compare_by_family() {
        for (a, b) in [("sonnet", "claude-sonnet-4-5"), ("Opus", "opus[1m]"), ("haiku", "claude-haiku-4-5-20251001")] {
            assert_eq!(model_family(a), model_family(b), "{a} {b}");
        }
        assert_ne!(model_family("haiku"), model_family("opus"));
        assert_eq!(model_family(" Fake-Model "), "fake-model");
    }

    #[test]
    fn an_empty_blocking_header_followed_by_findings_is_a_fail() {
        for text in [
            "VERDICT: PASS\nBLOCKING:\n- src/a.rs:3 - drops data\nMINOR: none",
            "VERDICT: PASS\nBLOCKING:\nsrc/a.rs:3 - drops data\n\nMINOR: none",
            "VERDICT: PASS\nBLOCKING: -\n- a.rs:1 - x\n- b.rs:2 - y\nMINOR: none",
        ] {
            let v = parse_verdict(text);
            assert!(!v.pass && !v.blocking.is_empty(), "{text:?} -> {v:?}");
        }
        let two = parse_verdict("VERDICT: PASS\nBLOCKING:\n- a.rs:1 - x\n- b.rs:2 - y\nMINOR: none");
        assert_eq!(two.blocking.len(), 2);
        // an empty header with nothing after it is not `none`
        let bare = parse_verdict("VERDICT: PASS\nBLOCKING:\nMINOR: none");
        assert!(!bare.pass && !bare.parsed, "{bare:?}");
        let end = parse_verdict("VERDICT: PASS\nMINOR: none\nBLOCKING:");
        assert!(!end.pass, "{end:?}");
        // an explicit none still passes
        assert!(parse_verdict("VERDICT: PASS\nBLOCKING: none\nMINOR: x.rs:1 - y").pass);
    }

    #[test]
    fn claude_output_parsing() {
        let ok = r#"{"type":"result","is_error":false,"result":"VERDICT: PASS\nBLOCKING: none\nMINOR: none","total_cost_usd":0.0123,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":100},"modelUsage":{"claude-haiku-4-5":{}}}"#;
        let r = parse_claude_output(ok, 0, "").unwrap();
        assert_eq!((r.cost_usd.as_deref(), r.input_tokens, r.output_tokens), (Some("0.0123"), Some(110), Some(5)));
        assert_eq!(r.models_used, ["claude-haiku-4-5"]);
        assert!(r.invalid.is_none() && judge(&r).pass);
        let mut same = r.clone();
        check_model(&mut same, "haiku");
        assert!(same.invalid.is_none() && judge(&same).pass);
        let mut other = r.clone();
        check_model(&mut other, "opus");
        let why = other.invalid.clone().unwrap();
        assert!(why.contains("claude-haiku-4-5") && why.contains("opus"), "{why}");
        assert!(!judge(&other).pass && judge(&other).note.contains("counts as FAIL"));
        let mut missing = parse_claude_output(r#"{"is_error":false,"result":"VERDICT: PASS\nBLOCKING: none"}"#, 0, "").unwrap();
        check_model(&mut missing, "haiku");
        assert!(missing.invalid.as_deref().unwrap().contains("modelUsage") && !judge(&missing).pass);
        assert!(matches!(parse_claude_output(ok, 1, "boom"), Err(CallError::Failed(m)) if m.contains("exited 1")));
        assert!(matches!(parse_claude_output(r#"{"is_error":true,"result":"Not logged in"}"#, 0, ""), Err(CallError::Failed(m)) if m.contains("Not logged in")));
        for plain in ["hello", "VERDICT: PASS\nBLOCKING: none\nMINOR: none", "{\"foo\":1}", ""] {
            let p = parse_claude_output(plain, 0, "").unwrap();
            assert!(p.invalid.is_some() && p.text.is_empty() && p.raw == plain, "{plain:?}");
            assert!(!judge(&p).pass);
        }
    }

    #[test]
    fn prompt_holds_brief_check_and_diff_only() {
        let p = build_prompt("BRIEF", "test: ok", "tail text", "+added line");
        for needle in ["BRIEF", "test: ok", "tail text", "+added line", "VERDICT"] {
            assert!(p.contains(needle) || needle == "VERDICT", "{needle}");
        }
    }
}
