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
//!   shorter than the `redact_min_chars` setting is removed from the environment only when a variable EQUALS it;
//! - the prompt (brief, diff, check output) and everything the queue stores (entry, review file) have the secret values replaced
//!   by `[REDACTED]`; `.env` values shorter than the `redact_min_chars` setting are not (they would blank out ordinary words), and the queue
//!   says how many were skipped.
//!
//! REDACTION LIMITS. Redaction is plain text replacement: (1) a secret split across two diff lines is not redacted; (2) a `.env`
//! value that is also ordinary code or text (`REGION=return`) is blanked everywhere it appears, which hides that code from the
//! reviewer. Keep `.env` values distinctive.
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

// The prompt pieces are macros so both prompts are built from the same text by `concat!` and cannot drift apart.
macro_rules! prompt_head {
    () => {
        "You are an independent code reviewer. You did not write this code and you do not fix it.
You are given a task description, its done check, and a diff. Decide whether the change really delivers what the task and done check require.
Look for: bugs that break the done check or the stated rules, data loss, security problems, races, faked or silent success, and wrong documentation.
Do not report style issues. Be specific: name the file and line (from the diff) and the concrete scenario that fails."
    };
}
macro_rules! prompt_tail {
    () => {
        "Answer in exactly this format and nothing else:
VERDICT: PASS or FAIL
BLOCKING: <file:line> - <one sentence: what fails, in which scenario>
(one BLOCKING line per blocking problem; write \"BLOCKING: none\" if there are none)
MINOR: <file:line> - <one sentence>
(one MINOR line per minor problem; write \"MINOR: none\" if there are none)"
    };
}
/// The "B2" rules block of the 2026-11 prompt-block replay (scratch/prompt-block-replay/b2-block.txt), for the gate only.
macro_rules! gate_block {
    () => {
        "Rules for BLOCKING lines:
1. The brief is the spec. A point it already settles (accepted deviation, documented limit, override, \"Adjudication\" or \"Fix round\" verdict, a named follow-up task) is never BLOCKING. Judge only what changed since the last review.
2. CHECK RESULT covers only what the queue ran (the Rust build and tests). Done-check output for other commands is pasted in the brief; do not block because it is missing from CHECK RESULT.
3. The brief's expected output for a done-check command (for example \"result: 19\") is the pass condition; matching it is not a failure.
4. Never reverse your own point inside the same line. If you wrote \"which is fine\" or \"per spec\", delete the line."
    };
}

/// The reviewer's fixed system prompt: the one the reviewer benchmark (bench/reviewer/run.py) measured. The shadow reviewer uses it.
pub const SYSTEM_PROMPT: &str = concat!(prompt_head!(), "\n\n", prompt_tail!());

/// `SYSTEM_PROMPT` with the rules block inserted before the answer format: for LOW-risk gating calls only (the block was replayed on Haiku).
pub const SYSTEM_PROMPT_GATE: &str = concat!(prompt_head!(), "\n\n", gate_block!(), "\n\n", prompt_tail!());

/// The system prompt for a gating call, chosen by risk (not by model name): `low` gets the block, `high` keeps `SYSTEM_PROMPT`.
pub fn gate_system_prompt(risk: &str) -> &'static str {
    if risk == "high" {
        SYSTEM_PROMPT
    } else {
        SYSTEM_PROMPT_GATE
    }
}

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

/// Every secret value this process knows of.
/// - EVERY value in `<root>/.env` is a secret, whatever its name (`GITHUB_PAT`, `DB_PASSWORD`, `REGION`...);
/// - so is the value of every secret-named variable of the process environment.
///
/// Values of `redact_min_chars` characters or more are replaced by `[REDACTED]` in any text; shorter `.env` values are not (they
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
    /// `min_len` is the `redact_min_chars` setting: values shorter than this are not redacted from text (they would blank out
    /// ordinary words); they are still kept out of the reviewer's environment (see `scrub_names`).
    pub fn collect(root: &Path, env: &[(String, String)], min_len: usize) -> Secrets {
        let mut redact: Vec<String> = Vec::new();
        let mut short_env: Vec<String> = Vec::new();
        for val in env_file_values(root) {
            if val.chars().count() >= min_len {
                redact.push(val);
            } else {
                short_env.push(val);
            }
        }
        for (k, val) in env {
            if secret_name(k) && val.chars().count() >= min_len {
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
    /// The GPT wrapper's `model_reported`: true only when the backend named the answering model (codex 0.170.0 never does, so it is
    /// false there). None for a Claude answer.
    pub model_reported: Option<bool>,
    /// The names the GPT wrapper's `model_used` lists (empty when none was reported).
    pub reported_models: Vec<String>,
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
    fn call(&self, cfg: &Config, cli: &Cli, model: &str, effort: &str, system_prompt: &str, prompt: &str) -> Result<Review, CallError>;
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

    fn call(&self, cfg: &Config, cli: &Cli, model: &str, effort: &str, system_prompt: &str, prompt: &str) -> Result<Review, CallError> {
        let cwd = cfg.scratch_dir.join("review-cwd");
        std::fs::create_dir_all(&cwd).map_err(|e| CallError::Start(format!("cannot create {}: {e}", cwd.display())))?;
        let mut args: Vec<String> = ["-p", "--system-prompt", system_prompt, "--model", model, "--tools", "", "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"].iter().map(|s| s.to_string()).collect();
        if !effort.is_empty() {
            args.extend(["--effort".to_string(), effort.to_string()]);
        }
        let env: Vec<(String, String)> = std::env::vars().collect();
        let remove = Secrets::collect(&cfg.root, &env, cfg.redact_min_chars as usize).scrub_names(&env);
        let out = run_cli(&cli.path, &args, &cwd, &remove, &[], prompt, Duration::from_secs(cfg.reviewer_timeout_seconds))?;
        let mut rev = parse_claude_output(&out.stdout, out.code, &out.stderr)?;
        check_model(&mut rev, model);
        Ok(rev)
    }
}

// ---- the shadow reviewer ----------------------------------------------------------------------

/// The env var that points the shadow reviewer at another script: ONLY a file inside `scratch_dir` is accepted (the test hook, so
/// tests never call the real API). Its value is never written to any file.
pub const SHADOW_SCRIPT_ENV: &str = "BROADSWORD_SHADOW_REVIEWER_SCRIPT";
/// The shadow reviewer program, relative to the root: a Python wrapper that answers like the Claude CLI.
pub const SHADOW_SCRIPT: &str = "clients/openai_review.py";

/// The shadow reviewer's family: every `gpt*` / `o<digit>*` / `openai*` name is `gpt`; anything else its own name.
pub fn shadow_family(name: &str) -> String {
    let n = name.trim().to_lowercase();
    let o_series = n.strip_prefix('o').is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
    if n.starts_with("gpt") || n.starts_with("openai") || o_series {
        "gpt".into()
    } else {
        model_family(&n)
    }
}

/// The shadow program: `python -I -B <script>`. The script is `clients/openai_review.py` under the root, or (test hook) the file
/// `BROADSWORD_SHADOW_REVIEWER_SCRIPT` names, which must lie inside `scratch_dir`.
///
/// S4 (no reviewer CLI inside the root) does NOT apply here: the shadow review never gates a merge, its verdict is only recorded,
/// so a reviewer program inside the root cannot approve anything.
pub fn resolve_shadow(cfg: &Config) -> Result<(String, String), String> {
    let script = match std::env::var(SHADOW_SCRIPT_ENV) {
        Ok(v) if !v.trim().is_empty() => {
            let real = clean_path(std::fs::canonicalize(v.trim()).map_err(|e| format!("the shadow script override is not readable: {e}"))?);
            let scratch = clean_path(std::fs::canonicalize(&cfg.scratch_dir).unwrap_or_else(|_| cfg.scratch_dir.clone()));
            if !real.starts_with(&scratch) {
                return Err(format!("{SHADOW_SCRIPT_ENV} must point inside scratch_dir (a test hook); refused"));
            }
            real
        }
        _ => {
            let p = cfg.root.join(SHADOW_SCRIPT);
            if !p.is_file() {
                return Err(format!("the shadow reviewer program {SHADOW_SCRIPT} was not found under the project root"));
            }
            clean_path(std::fs::canonicalize(&p).unwrap_or(p))
        }
    };
    let python = ["python", "python3"].iter().find_map(|n| find_on_path(n)).ok_or("python was not found on PATH (the shadow reviewer needs it)")?;
    Ok((python.to_string_lossy().replace('\\', "/"), script.to_string_lossy().replace('\\', "/")))
}

/// The shadow answer must come from the requested model: every name `modelUsage` lists starts with the requested name
/// (`gpt-5.6-sol` answers as `gpt-5.6-sol-2026-01-01`).
pub fn check_shadow_model(rev: &mut Review, requested: &str) {
    if rev.invalid.is_some() {
        return;
    }
    let want = requested.trim().to_lowercase();
    if rev.models_used.is_empty() || rev.models_used.iter().any(|m| !m.to_lowercase().starts_with(&want)) {
        rev.invalid = Some(format!("the shadow answer's modelUsage ({}) does not name the requested model `{requested}`", if rev.models_used.is_empty() { "empty".to_string() } else { rev.models_used.join(", ") }));
    }
}

/// Call the shadow reviewer once, with the same prompt, the scrubbed environment and `shadow_reviewer_timeout_seconds`.
pub fn call_shadow(cfg: &Config, python: &str, script: &str, model: &str, prompt: &str) -> Result<Review, CallError> {
    call_gpt(cfg, python, script, model, SYSTEM_PROMPT, prompt, "shadow", cfg.shadow_reviewer_timeout_seconds)
}

/// The env var through which a caller tells clients/openai_review.py what kind of call this is (`gate` or `shadow`), for its ledger.
pub const CALL_TYPE_ENV: &str = "OPENAI_REVIEW_CALL_TYPE";

/// Call clients/openai_review.py once (the shadow review and the GPT gate both come here, so both pass the client's cap and ledger),
/// with the scrubbed environment, `OPENAI_REVIEW_CALL_TYPE=<call_type>` and a kill after `timeout_s`. The answer must come from the
/// requested model: a model named by the backend that is another one makes the review `invalid`.
#[allow(clippy::too_many_arguments)]
pub fn call_gpt(cfg: &Config, python: &str, script: &str, model: &str, system_prompt: &str, prompt: &str, call_type: &str, timeout_s: u64) -> Result<Review, CallError> {
    let cwd = cfg.scratch_dir.join("review-cwd");
    std::fs::create_dir_all(&cwd).map_err(|e| CallError::Start(format!("cannot create {}: {e}", cwd.display())))?;
    let mut args: Vec<String> = vec!["-I".into(), "-B".into(), script.into()];
    args.extend(["-p", "--system-prompt", system_prompt, "--model", model, "--tools", "", "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"].iter().map(|s| s.to_string()));
    let env: Vec<(String, String)> = std::env::vars().collect();
    let remove = Secrets::collect(&cfg.root, &env, cfg.redact_min_chars as usize).scrub_names(&env);
    let out = run_cli(python, &args, &cwd, &remove, &[(CALL_TYPE_ENV, call_type)], prompt, Duration::from_secs(timeout_s))?;
    let mut rev = parse_claude_output(&out.stdout, out.code, &out.stderr)?;
    check_shadow_model(&mut rev, model);
    check_reported_models(&mut rev, model);
    Ok(rev)
}

/// When the backend DID name the answering model(s) (`model_reported`), every name must start with the requested one. When it did
/// not (codex 0.170.0), nothing can be checked and nothing is wrong: the caller records "model not reported".
pub fn check_reported_models(rev: &mut Review, requested: &str) {
    if rev.invalid.is_some() || rev.model_reported != Some(true) {
        return;
    }
    let want = requested.trim().to_lowercase();
    if rev.reported_models.is_empty() || rev.reported_models.iter().any(|m| !m.to_lowercase().starts_with(&want)) {
        rev.invalid = Some(format!("the answer names the model(s) {}, not the requested `{requested}`", if rev.reported_models.is_empty() { "(none, though it says it reported)".to_string() } else { rev.reported_models.join(", ") }));
    }
}

pub struct CliOut {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run `program args` in `cwd` with `remove` variables dropped from its environment, `stdin` fed to it, killed (with its
/// whole process tree) after `timeout`.
pub fn run_cli(program: &str, args: &[String], cwd: &Path, remove: &[String], set: &[(&str, &str)], stdin: &str, timeout: Duration) -> Result<CliOut, CallError> {
    let mut cmd = Command::new(program);
    cmd.args(args).current_dir(cwd).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    for k in remove {
        cmd.env_remove(k);
    }
    for (k, v) in set {
        cmd.env(k, v);
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
            if let Some(J::Bool(b)) = j.get("model_reported") {
                rev.model_reported = Some(*b);
            }
            if let Some(J::Arr(a)) = j.get("model_used") {
                rev.reported_models = a.iter().filter_map(|x| x.str().map(String::from)).collect();
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
    fn gate_prompt_is_shadow_prompt_plus_block() {
        assert!(SYSTEM_PROMPT_GATE.contains("Rules for BLOCKING lines:"));
        assert!(!SYSTEM_PROMPT.contains("Rules for BLOCKING lines:"));
        let at = SYSTEM_PROMPT.find("Answer in exactly this format").expect("format marker");
        let expect = format!("{}{}\n\n{}", &SYSTEM_PROMPT[..at], gate_block!(), &SYSTEM_PROMPT[at..]);
        assert_eq!(SYSTEM_PROMPT_GATE, expect);
        assert!(SYSTEM_PROMPT_GATE.contains("Be specific: name the file and line (from the diff) and the concrete scenario that fails.\n\nRules for BLOCKING lines:"));
    }

    #[test]
    fn a_named_answering_model_must_be_the_requested_one_and_an_unnamed_one_is_not_an_error() {
        let parse = |used: &str, reported: bool| {
            let j = format!(r#"{{"result":"VERDICT: PASS","modelUsage":{{"gpt-6.1-sol":{{}}}},"model_reported":{reported},"model_used":{used}}}"#);
            let mut r = parse_claude_output(&j, 0, "").unwrap();
            check_shadow_model(&mut r, "gpt-6.1-sol");
            check_reported_models(&mut r, "gpt-6.1-sol");
            r
        };
        assert!(parse("[]", false).invalid.is_none(), "codex 0.170.0: nothing reported, nothing wrong");
        assert_eq!(parse("[]", false).model_reported, Some(false));
        assert!(parse(r#"["gpt-6.1-sol-2026-01-01"]"#, true).invalid.is_none());
        assert!(parse(r#"["gpt-5-mini"]"#, true).invalid.as_deref().unwrap_or("").contains("gpt-5-mini"));
        assert!(parse(r#"["gpt-6.1-sol","gpt-5-mini"]"#, true).invalid.is_some(), "one of several names is another model");
        assert!(parse("[]", true).invalid.is_some(), "claims to have reported but names nothing");
    }

    #[test]
    fn gate_prompt_is_chosen_by_risk() {
        assert!(gate_system_prompt("low").contains("Rules for BLOCKING lines:"));
        assert!(!gate_system_prompt("high").contains("Rules for BLOCKING lines:"));
        assert_eq!(gate_system_prompt("high"), SYSTEM_PROMPT);
        assert_eq!(gate_system_prompt("low"), SYSTEM_PROMPT_GATE);
    }

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
        let s = Secrets::collect(Path::new("no-such-folder-here"), &env, 6);
        assert_eq!(s.redact("key=sk-live-0123456789;abc"), "key=[REDACTED];abc", "short values are not redacted");
        assert_eq!(s.scrub_names(&env), ["OPENAI_API_KEY", "COPIED", "SHORT_KEY"]);
    }

    #[test]
    fn every_env_file_value_is_a_secret_whatever_its_name() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-secrets-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(".env"), "# c\nREGION=eu-west-1\nGITHUB_PAT=ghp_abcdef123456\nexport DB_PASSWORD=\"hunter2hunter2\"\nTINY=ab\nEMPTY=\n").unwrap();
        let env: Vec<(String, String)> = [("MY_COPY", "x-ghp_abcdef123456-y"), ("EXACT_SHORT", "ab"), ("HAS_SHORT", "xaby"), ("OTHER", "plain")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let s = Secrets::collect(&d, &env, 6);
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
    fn shadow_families_and_model_check() {
        for n in ["gpt-5.6-sol", "GPT-5.5", "o3", "o4-mini", "openai-x"] {
            assert_eq!(shadow_family(n), "gpt", "{n}");
        }
        assert_ne!(shadow_family("sonnet"), shadow_family("gpt-5.6-sol"));
        assert_eq!(shadow_family("claude-opus-4"), "opus");
        let mut ok = parse_claude_output(r#"{"is_error":false,"result":"x","modelUsage":{"gpt-5.6-sol-2026-01-01":{}}}"#, 0, "").unwrap();
        check_shadow_model(&mut ok, "gpt-5.6-sol");
        assert!(ok.invalid.is_none());
        let mut other = parse_claude_output(r#"{"is_error":false,"result":"x","modelUsage":{"gpt-4o":{}}}"#, 0, "").unwrap();
        check_shadow_model(&mut other, "gpt-5.6-sol");
        assert!(other.invalid.is_some());
    }

    #[test]
    fn prompt_holds_brief_check_and_diff_only() {
        let p = build_prompt("BRIEF", "test: ok", "tail text", "+added line");
        for needle in ["BRIEF", "test: ok", "tail text", "+added line", "VERDICT"] {
            assert!(p.contains(needle) || needle == "VERDICT", "{needle}");
        }
    }
}
