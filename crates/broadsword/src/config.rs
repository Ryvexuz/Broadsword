//! broadsword.toml: project root discovery and a tiny flat `key = value` parser.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "broadsword.toml";
pub const GB: u64 = 1024 * 1024 * 1024;
pub const MB: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(i64),
    Str(String),
}

/// Remove a trailing `# comment`, ignoring `#` inside double quotes.
fn strip_comment(line: &str) -> &str {
    let mut in_quote = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_quote = !in_quote,
            '#' if !in_quote => return &line[..i],
            _ => {}
        }
    }
    line
}

/// Parse flat `key = value` lines (integers, quoted strings, `#` comments).
pub fn parse_flat(text: &str) -> Result<HashMap<String, Value>, String> {
    let mut map = HashMap::new();
    for (n, raw) in text.lines().enumerate() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let (key, val) = line
            .split_once('=')
            .ok_or_else(|| format!("line {}: expected `key = value`", n + 1))?;
        let key = key.trim();
        let val = val.trim();
        if key.is_empty() {
            return Err(format!("line {}: empty key", n + 1));
        }
        let value = if let Some(rest) = val.strip_prefix('"') {
            let s = rest
                .strip_suffix('"')
                .ok_or_else(|| format!("line {}: unterminated string", n + 1))?;
            Value::Str(s.to_string())
        } else {
            Value::Int(
                val.parse::<i64>()
                    .map_err(|_| format!("line {}: `{}` is not an integer or quoted string", n + 1, val))?,
            )
        };
        map.insert(key.to_string(), value);
    }
    Ok(map)
}

#[derive(Debug, Clone)]
pub struct Config {
    pub root: PathBuf,
    pub disk_floor_gb: u64,
    pub cleanup_every_hours: u64,
    pub scratch_max_age_hours: u64,
    pub disk_check_every_seconds: u64,
    pub log_max_mb: u64,
    pub ram_warn_percent: u64,
    pub cpu_warn_percent: u64,
    pub alert_command: String,
    pub scratch_dir: PathBuf,
    pub build_dir: PathBuf,
    pub projects_dir: PathBuf,
    pub state_dir: PathBuf,
    /// Where `broadsword install` puts the stable copy of the binary.
    pub install_dir: PathBuf,
    pub mcp_call_timeout_seconds: u64,
    pub mcp_output_max_kb: u64,
    pub checkpoint_ignored_max_mb: u64,
    pub max_resumes: u64,
    pub job_disk_reserve_gb: u64,
    pub job_disk_poll_seconds: u64,
    /// How long a `task` command waits for the board lock before exiting "board busy".
    pub task_lock_wait_seconds: u64,
    pub task_archive_days: u64,
    /// How long a build, test or check job waits for the build lock (state/build.lock) before it is refused.
    pub build_lock_wait_seconds: u64,
    /// Where each task's Git worktree lives: `<worktrees_dir>/<project>/<task id>` (broadsword worktree ...).
    pub worktrees_dir: PathBuf,
    /// Which reviewer backend `queue run` calls. Only `claude` (the Claude Code CLI, headless) exists today.
    pub reviewer_backend: String,
    /// The reviewer CLI program (resolved on PATH). The env var BROADSWORD_REVIEWER_CLI overrides it with a machine-specific path
    /// that must lie OUTSIDE the root or inside scratch_dir (a test hook); any other path inside the root is refused.
    pub reviewer_cli: String,
    /// The reviewer model for a `--risk low` / `--risk high` queue entry.
    pub reviewer_model_low: String,
    pub reviewer_model_high: String,
    /// The `--risk high` reviewer when `reviewer_model_high` is the coder's own family.
    pub reviewer_model_high_fallback: String,
    /// The gating reviewer of a `--risk high` entry whose --coder is the `reviewer_model_high` family: an OpenAI model asked through
    /// clients/openai_review.py (same cap and ledger as the shadow review), or `off` for no GPT gate. A skipped call falls back to
    /// `reviewer_model_high_fallback` in the same run.
    pub reviewer_model_high_gpt: String,
    /// The `--effort` of a `--risk high` gating call: low, medium, high, xhigh or max. A low-risk call passes no flag.
    pub reviewer_effort_high: String,
    /// A reviewer call that runs longer than this is killed (no verdict, the entry is `blocked`).
    pub reviewer_timeout_seconds: u64,
    /// A diff bigger than this is never sent to a reviewer (the entry is `blocked`: split the task).
    pub reviewer_diff_max_kb: u64,
    /// A brief file bigger than this is refused by `queue add`.
    pub reviewer_brief_max_kb: u64,
    /// How much of the end of the check output goes into the review prompt.
    pub reviewer_check_tail_kb: u64,
    /// How long a `queue` command waits for a queue lock before it exits 4 "queue busy".
    pub queue_lock_wait_seconds: u64,
    /// The most reviewer calls (real model calls, resumes and re-runs included) one queue entry may make; at the cap it is `blocked`.
    pub reviewer_max_calls_per_entry: u64,
    /// `queue run --all` merge train: the most review-PASSed entries (one project, claims that do not overlap) merged by one check, one
    /// fast-forward and one main check. 1 = off: every entry is checked, merged and main-checked on its own.
    pub merge_train_max: u64,
    /// Concurrent stacked, claim-disjoint merge trains; 1 keeps the serial train.
    pub merge_train_lanes: u64,
    /// `queue run --all` merge train: a project whose oldest approved entry has waited this many minutes gets a train of what it has
    /// (even 1 entry) at the next entry boundary, although entries of that project are still queued. Without it, approved entries of
    /// one project could wait behind another project's stream of entries until the drain ends (the 2026-11-20 starvation).
    pub merge_train_wait_minutes: u64,
    /// `queue run --all` with the merge train on (merge_train_max > 1): the most entries brought to their review verdict at the same
    /// time (PARALLEL REVIEW in queue.rs). Everything after the review stays serial and in queue order. 1 = one at a time, as before.
    pub review_parallel: u64,
    /// The merge queue's perf gate (perfgate.rs): `"on"` runs a project's bench on every checked commit of a project that has a baseline
    /// in `<state_dir>/perf/`, and fails an entry that got slower than a case's `threshold_pct` allows; `"off"` runs no bench.
    pub perf_gate: bool,
    /// How many times the perf gate runs the bench per checked commit; each case is judged on the median of these runs.
    pub perf_runs: u64,
    /// `"on"`: every `queue run` holds its project's own lock, `<state_dir>/queue/<project>.queue.lock`, so drivers of different
    /// projects run at once (and `queue run` needs `--project` or `--entry`); `"off"`: the one global `queue.lock`, as before.
    pub queue_per_project: bool,
    /// `"on"`: a main check that fails after the queue merged an entry or a train reverts exactly those commits on the trunk (`git
    /// revert`, new commits, never a reset), checks main again, and holds the project's lane (`state/queue/hold-<project>`) when the
    /// revert or that check fails. `"off"`: the failure is reported and nothing is reverted, as before.
    pub queue_auto_revert: bool,
    /// Background train main check; the next fast-forward and the drain's end wait for its result.
    pub queue_main_check_async: bool,
    pub main_full_check_every_minutes: u64,
    /// The merge queue's PRE-merge check for the projects named here is a fast safety subset, ONE `run` job: `cargo build --offline &&
    /// cargo test --offline [--lib] [--bins] --test <name> ...` (queue.rs `pre_merge_check`); the main check after the merge stays the full
    /// `check`. Parsed from the required key `merge_fast_tests` by `parse_merge_fast_tests` and checked by `check_merge_fast_tests`;
    /// read it through `merge_fast_subset` (a project not named runs the full check). Its names are checked against cargo metadata
    /// at each pre-merge check (queue.rs `fast_plan`), never trusted from here.
    pub merge_fast_tests: Vec<(String, Vec<String>)>,
    /// The shadow second reviewer: `openai` (clients/openai_review.py, recorded only, never gates a merge) or `off`.
    pub shadow_reviewer: String,
    /// The shadow reviewer's model (an OpenAI model name).
    pub shadow_reviewer_model: String,
    /// A shadow review that runs longer than this is killed and recorded as `ERROR`; the merge goes on.
    pub shadow_reviewer_timeout_seconds: u64,
    /// The most tasks with a working coder at once, across all projects (`task start` refuses at the cap): a `running` task
    /// whose branch has a `queued` or `running` merge-queue entry waits for the queue and is not counted; one whose entry is
    /// `blocked` or `stale` needs its coder, so it counts.
    pub max_agents: u64,
    /// The most open merge-queue entries (any project) at which `task start` still starts a task; at or above it, it refuses,
    /// so finished-but-unmerged work cannot pile up without limit now that it no longer counts against max_agents.
    pub max_queued: u64,
    /// How long `task start` keeps re-reading a merge-queue entry file that cannot be read or is not yet a complete entry
    /// (with backoff) before it fails the start loudly. Well above a queue write (milliseconds), so a concurrent `queue add`
    /// or `queue run` save never fails a legitimate start; short next to task_lock_wait_seconds, since start holds the board.
    pub queue_entry_read_wait_ms: u64,
    /// How long `recover` (and `recover <id>`, `recover --abandon`) waits for the recover lock before it says another recover is running.
    pub recover_lock_wait_seconds: u64,
    /// `cargo sweep --time N`: build-folder files not used for N days are swept (by `cleanup`, and by the cleanup a low-disk kill runs).
    pub sweep_older_than_days: u64,
    /// A `.env` or secret-named value shorter than this many characters is not redacted from text (it would blank ordinary words);
    /// it is still kept out of every child's environment.
    pub redact_min_chars: u64,
    /// SCALING F9: the Python test scripts each project's `test` and `check` run after cargo's tests, as `(project, scripts)`.
    /// Parsed from the required key `python_tests` by `parse_python_tests`; a project not named in it has no Python tests.
    pub python_tests: Vec<(String, Vec<String>)>,
    /// Scripts at once after cargo's tests and doc-tests, when test_parallel_binaries is 1.
    pub python_parallel: usize,
    /// The merge queue's DONE CHECK (donecheck.rs): a done-check command the queue runs is killed (its whole tree) after this long.
    pub done_check_timeout_seconds: u64,
    /// The merge queue's DONE CHECK: the last N lines of each command's output go to the reviewer and the `.donecheck` file.
    pub done_check_tail_lines: u64,
    /// The test step runs up to this many copied test executables (and Python test scripts) at once; 1 = one after another.
    pub test_parallel_binaries: u64,
    /// Side by side, a test child that has exited has its output pipes waited for at most this many seconds (a process it
    /// started may hold them open); then its output is printed as far as it got, marked TRUNCATED. Never blocks the pool.
    pub test_output_wait_seconds: u64,
}

/// Parse the `python_tests` setting: `"<project>: <script> <script> ...; <project>: ..."`, or `""` when no project has Python
/// tests. A project that is not named runs no Python tests (that is how a project declares it has none: the key itself is still
/// required). Every script is a relative `.py` path inside the project folder (no absolute path, no `..`, no whitespace); the
/// job runs it as `python -I -B <script>`. A project named twice (any case), a project with no script, or a bad path is an error.
pub fn parse_python_tests(v: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    use std::path::Component;
    let bad = |why: String| format!("python_tests = \"{v}\": {why}");
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for entry in v.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let (project, scripts) = entry.split_once(':').ok_or_else(|| bad(format!("`{entry}` is not `<project>: <script> ...`")))?;
        let project = project.trim();
        if project.is_empty() || !project.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
            return Err(bad(format!("`{project}` is not a project name")));
        }
        if out.iter().any(|(p, _)| p.eq_ignore_ascii_case(project)) {
            return Err(bad(format!("project `{project}` is named twice")));
        }
        let scripts: Vec<String> = scripts.split_whitespace().map(String::from).collect();
        if scripts.is_empty() {
            return Err(bad(format!("project `{project}` names no script (leave a project with no Python tests out of the list)")));
        }
        for s in &scripts {
            let p = Path::new(s);
            let relative = p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) && p.components().any(|c| matches!(c, Component::Normal(_)));
            if !relative || !s.ends_with(".py") || s.starts_with('-') {
                return Err(bad(format!("`{s}` must be a relative `.py` path inside the project folder (no absolute path, no `..`, not starting with `-`)")));
            }
        }
        out.push((project.to_string(), scripts));
    }
    Ok(out)
}


/// Parse the `merge_fast_tests` setting: `"<project>: <test> <test> ...; <project>: ..."`, or `""` when every project runs the full
/// check before a merge. Each `<test>` is the name of a cargo test binary (`tests/<name>.rs`): letters, digits, `_` and `-`, not
/// starting with `-` (it goes on a command line). A project named twice (any case), a project with no test, or a bad name is an error.
pub fn parse_merge_fast_tests(v: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let bad = |why: String| format!("merge_fast_tests = \"{v}\": {why}");
    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for entry in v.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        let (project, tests) = entry.split_once(':').ok_or_else(|| bad(format!("`{entry}` is not `<project>: <test> ...`")))?;
        let project = project.trim();
        if project.is_empty() || !project.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) {
            return Err(bad(format!("`{project}` is not a project name")));
        }
        if out.iter().any(|(p, _)| p.eq_ignore_ascii_case(project)) {
            return Err(bad(format!("project `{project}` is named twice")));
        }
        let tests: Vec<String> = tests.split_whitespace().map(String::from).collect();
        if tests.is_empty() {
            return Err(bad(format!("project `{project}` names no test binary (leave a project that runs the full check out of the list)")));
        }
        for t in &tests {
            if t.starts_with('-') || !t.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-')) {
                return Err(bad(format!("`{t}` is not a test binary name (letters, digits, `_` and `-`, not starting with `-`)")));
            }
        }
        out.push((project.to_string(), tests));
    }
    Ok(out)
}

/// `merge_fast_tests` checked at config load: a subset is only safe with queue_auto_revert on (a broken merge is reverted after the
/// full main check). The NAMES are not checked here: config load runs no cargo. They are checked against cargo metadata of the very
/// commit being checked, at every pre-merge check (queue.rs `fast_plan`): the one source of valid test target names.
fn check_merge_fast_tests(list: Vec<(String, Vec<String>)>, auto_revert: bool) -> Result<Vec<(String, Vec<String>)>, String> {
    if !list.is_empty() && !auto_revert {
        return Err("merge_fast_tests needs queue_auto_revert = on: a fast subset before the merge is only safe when a merge whose full main check fails is reverted".into());
    }
    Ok(list)
}

fn get_int(m: &HashMap<String, Value>, k: &str) -> Result<u64, String> {
    match m.get(k) {
        Some(Value::Int(i)) if *i >= 0 => Ok(*i as u64),
        Some(_) => Err(format!("{k}: expected a non-negative integer")),
        None => Err(format!("{k}: missing from {CONFIG_FILE}")),
    }
}

fn get_str(m: &HashMap<String, Value>, k: &str) -> Result<String, String> {
    match m.get(k) {
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(_) => Err(format!("{k}: expected a quoted string")),
        None => Err(format!("{k}: missing from {CONFIG_FILE}")),
    }
}

/// A model name goes onto a command line: letters, digits and `.` `_` `-` `:` `[` `]` only, never starting with `-`
/// (it could otherwise be read as a flag).
pub fn valid_model_name(m: &str) -> bool {
    !m.is_empty() && !m.starts_with('-') && m.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']'))
}

/// Drop `.` parts and unify separators without touching the disk (no canonicalize: no verbatim-prefix paths).
pub fn normalize(p: &Path) -> PathBuf {
    p.components().collect()
}

/// A path setting must be relative and stay inside root: no absolute paths, no `..`.
fn inside_root(m: &HashMap<String, Value>, k: &str, required: bool) -> Result<PathBuf, String> {
    use std::path::Component;
    if !required && !m.contains_key(k) {
        return Ok(PathBuf::new());
    }
    let v = get_str(m, k)?;
    let p = Path::new(&v);
    let ok = p.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
        && p.components().any(|c| matches!(c, Component::Normal(_)));
    if ok {
        Ok(p.to_path_buf())
    } else {
        Err(format!("{k} = \"{v}\": must be a relative folder inside root (no absolute path, no `..`)"))
    }
}

/// True when one folder is the other or lies inside the other (either way round).
fn overlaps(a: &Path, b: &Path) -> bool {
    a.starts_with(b) || b.starts_with(a)
}

/// The one place that checks the managed folders against each other, in BOTH directions. A pair is refused when either folder
/// is the other or inside the other, because cleanup of one (scratch sweep, build cleanup) or its records would then delete or
/// shadow the other. install_dir may sit inside build_dir (the shipped default), so that pair is not checked.
fn check_folders_apart(scratch: &Path, build: &Path, state: &Path, install: &Path, worktrees: &Path) -> Result<(), String> {
    // (name, folder, other name, other folder, why it matters)
    let pairs: [(&str, &Path, &str, &Path, &str); 9] = [
        ("build_dir", build, "scratch_dir", scratch, "the scratch sweep would delete build output, or the build cleanup would delete scratch files"),
        ("state_dir", state, "scratch_dir", scratch, "cleanup would delete the job records"),
        ("state_dir", state, "build_dir", build, "cleanup would delete the job records"),
        ("install_dir", install, "scratch_dir", scratch, "that folder is cleaned up, not the stable binary"),
        ("install_dir", install, "state_dir", state, "that folder holds records, not the stable binary"),
        ("worktrees_dir", worktrees, "scratch_dir", scratch, "that folder is cleaned up, and a worktree holds a task's unmerged work"),
        ("worktrees_dir", worktrees, "build_dir", build, "that folder is cleaned up, and a worktree holds a task's unmerged work"),
        ("worktrees_dir", worktrees, "state_dir", state, "that folder holds records, and a worktree holds a task's unmerged work"),
        ("worktrees_dir", worktrees, "install_dir", install, "the stable binary folder must hold only the binary, and a worktree holds a task's unmerged work"),
    ];
    for (name, dir, other_name, other, why) in pairs {
        if !overlaps(dir, other) {
            continue;
        }
        return Err(if dir == other {
            format!("{name} and {other_name} must be different folders: {why}")
        } else if dir.starts_with(other) {
            format!("{name} must not be inside {other_name}: {why}")
        } else {
            format!("{other_name} must not be inside {name}: {why}")
        });
    }
    Ok(())
}

impl Config {
    /// Build a Config from parsed text. `config_dir` is the folder holding broadsword.toml;
    /// `root` and every path in the file are resolved relative to it.
    pub fn from_text(text: &str, config_dir: &Path) -> Result<Config, String> {
        let m = parse_flat(text)?;
        let root = normalize(&config_dir.join(get_str(&m, "root")?));
        let (scratch_dir, build_dir) = (root.join(inside_root(&m, "scratch_dir", true)?), root.join(inside_root(&m, "build_dir", true)?));
        let state_dir = root.join(inside_root(&m, "state_dir", true)?);
        let install_dir = root.join(inside_root(&m, "install_dir", true)?);
        let worktrees_dir = root.join(inside_root(&m, "worktrees_dir", true)?);
        check_folders_apart(&scratch_dir, &build_dir, &state_dir, &install_dir, &worktrees_dir)?;
        let positive = |k: &str| get_int(&m, k).and_then(|v| if v == 0 { Err(format!("{k}: must be at least 1")) } else { Ok(v) });
        let (mcp_call_timeout_seconds, mcp_output_max_kb) = (positive("mcp_call_timeout_seconds")?, positive("mcp_output_max_kb")?);
        // read only by clients/codex_coder.py, which loads this key itself: validated here so a missing or 0 value is refused at load, and not stored (no field nothing reads)
        positive("codex_coder_timeout_seconds")?;
        // read only by scripts/hotspots.py, which loads these keys itself: validated here (missing or 0 refused at load), not stored
        for k in ["hotspot_lines", "hotspot_window_hours", "hotspot_wait_hours", "hotspot_claims", "hotspot_commits"] {
            positive(k)?;
        }
        // read only by scripts/stall_restart.py, which loads these keys itself: validated here (missing or 0 refused at load), not stored
        // stale_claim_hours: read only by scripts/stale_claims.py
        // auto_agent_*: read only by scripts/auto_agent.py
        for k in ["stall_restart_lines", "stall_restart_timeout_seconds", "stale_claim_hours", "auto_agent_timeout_minutes", "auto_agent_min_gap_minutes", "auto_agent_max_starts_per_day"] {
            positive(k)?;
        }
        let reviewer_backend = get_str(&m, "reviewer_backend")?;
        if reviewer_backend != "claude" {
            return Err(format!("reviewer_backend = \"{reviewer_backend}\": only \"claude\" is supported"));
        }
        let reviewer_cli = get_str(&m, "reviewer_cli")?;
        if reviewer_cli.trim().is_empty() || reviewer_cli.contains(['\n', '\r']) {
            return Err("reviewer_cli: must be a non-empty program name (resolved on PATH)".into());
        }
        let model = |k: &str| {
            let v = get_str(&m, k)?;
            if valid_model_name(&v) {
                Ok(v)
            } else {
                Err(format!("{k} = \"{v}\": a model name uses letters, digits and . _ - : [ ] only, and does not start with `-`"))
            }
        };
        let reviewer_effort_high = get_str(&m, "reviewer_effort_high")?;
        if !["low", "medium", "high", "xhigh", "max"].contains(&reviewer_effort_high.as_str()) {
            return Err(format!("reviewer_effort_high = \"{reviewer_effort_high}\": must be low, medium, high, xhigh or max"));
        }
        let perf_gate = match get_str(&m, "perf_gate")?.as_str() {
            "on" => true,
            "off" => false,
            other => return Err(format!("perf_gate = \"{other}\": must be \"on\" or \"off\"")),
        };
        let queue_per_project = match get_str(&m, "queue_per_project")?.as_str() {
            "on" => true,
            "off" => false,
            other => return Err(format!("queue_per_project = \"{other}\": must be \"on\" or \"off\"")),
        };
        let queue_auto_revert = match get_str(&m, "queue_auto_revert")?.as_str() {
            "on" => true,
            "off" => false,
            other => return Err(format!("queue_auto_revert = \"{other}\": must be \"on\" or \"off\"")),
        };
        let queue_main_check_async = match get_str(&m, "queue_main_check_async")?.as_str() {
            "on" => true,
            "off" => false,
            other => return Err(format!("queue_main_check_async = \"{other}\": must be \"on\" or \"off\"")),
        };
        let shadow_reviewer = get_str(&m, "shadow_reviewer")?;
        if shadow_reviewer != "openai" && shadow_reviewer != "off" {
            return Err(format!("shadow_reviewer = \"{shadow_reviewer}\": must be \"openai\" or \"off\""));
        }
        let python_tests = parse_python_tests(&get_str(&m, "python_tests")?)?;
        let projects_dir = root.join(inside_root(&m, "projects_dir", true)?);
        let merge_fast_tests = check_merge_fast_tests(parse_merge_fast_tests(&get_str(&m, "merge_fast_tests")?)?, queue_auto_revert)?;
        Ok(Config {
            merge_fast_tests,
            python_tests,
            python_parallel: usize::try_from(positive("python_parallel")?)
                .map_err(|_| "python_parallel: does not fit this platform's pool width")?,
            done_check_timeout_seconds: positive("done_check_timeout_seconds")?,
            done_check_tail_lines: positive("done_check_tail_lines")?,
            shadow_reviewer,
            shadow_reviewer_model: model("shadow_reviewer_model")?,
            shadow_reviewer_timeout_seconds: positive("shadow_reviewer_timeout_seconds")?,
            reviewer_backend,
            reviewer_cli,
            reviewer_model_low: model("reviewer_model_low")?,
            reviewer_model_high: model("reviewer_model_high")?,
            reviewer_model_high_fallback: model("reviewer_model_high_fallback")?,
            reviewer_model_high_gpt: model("reviewer_model_high_gpt")?,
            reviewer_effort_high,
            reviewer_timeout_seconds: positive("reviewer_timeout_seconds")?,
            reviewer_diff_max_kb: positive("reviewer_diff_max_kb")?,
            reviewer_brief_max_kb: positive("reviewer_brief_max_kb")?,
            reviewer_check_tail_kb: positive("reviewer_check_tail_kb")?,
            queue_lock_wait_seconds: positive("queue_lock_wait_seconds")?,
            reviewer_max_calls_per_entry: positive("reviewer_max_calls_per_entry")?,
            merge_train_max: positive("merge_train_max")?,
            merge_train_lanes: positive("merge_train_lanes")?,
            merge_train_wait_minutes: positive("merge_train_wait_minutes")?,
            review_parallel: positive("review_parallel")?,
            perf_gate,
            perf_runs: positive("perf_runs")?,
            queue_per_project,
            queue_auto_revert,
            queue_main_check_async,
            main_full_check_every_minutes: get_int(&m, "main_full_check_every_minutes")?,
            max_agents: positive("max_agents")?,
            max_queued: positive("max_queued")?,
            queue_entry_read_wait_ms: positive("queue_entry_read_wait_ms")?,
            test_parallel_binaries: positive("test_parallel_binaries")?,
            test_output_wait_seconds: positive("test_output_wait_seconds")?,
            recover_lock_wait_seconds: positive("recover_lock_wait_seconds")?,
            sweep_older_than_days: positive("sweep_older_than_days")?,
            redact_min_chars: positive("redact_min_chars")?,
            disk_floor_gb: get_int(&m, "disk_floor_gb")?,
            cleanup_every_hours: positive("cleanup_every_hours")?,
            scratch_max_age_hours: positive("scratch_max_age_hours")?,
            disk_check_every_seconds: positive("disk_check_every_seconds")?,
            log_max_mb: positive("log_max_mb")?,
            ram_warn_percent: get_int(&m, "ram_warn_percent")?,
            cpu_warn_percent: get_int(&m, "cpu_warn_percent")?,
            alert_command: get_str(&m, "alert_command")?,
            scratch_dir,
            build_dir,
            projects_dir,
            state_dir,
            install_dir,
            mcp_call_timeout_seconds,
            mcp_output_max_kb,
            checkpoint_ignored_max_mb: get_int(&m, "checkpoint_ignored_max_mb")?,
            max_resumes: get_int(&m, "max_resumes")?,
            job_disk_reserve_gb: get_int(&m, "job_disk_reserve_gb")?,
            job_disk_poll_seconds: positive("job_disk_poll_seconds")?,
            task_lock_wait_seconds: positive("task_lock_wait_seconds")?,
            task_archive_days: positive("task_archive_days")?,
            build_lock_wait_seconds: positive("build_lock_wait_seconds")?,
            worktrees_dir,
            root,
        })
    }

    /// THE ONE place the queue reads `merge_fast_tests`: the test names listed for `project` (any case), or None (the full check).
    /// The names are unverified here; queue.rs `fast_plan` checks them against cargo metadata before any subset runs.
    pub fn merge_fast_subset(&self, project: &str) -> Option<&[String]> {
        self.merge_fast_tests.iter().find(|(p, _)| p.eq_ignore_ascii_case(project)).map(|(_, t)| t.as_slice())
    }

    pub fn load_from(config_dir: &Path) -> Result<Config, String> {
        let file = config_dir.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        Config::from_text(&text, config_dir)
    }

    pub fn reserve_bytes(&self) -> u64 {
        self.job_disk_reserve_gb * GB
    }

    pub fn floor_bytes(&self) -> u64 {
        self.disk_floor_gb * GB
    }
}

/// Walk up from `start` to the first folder containing broadsword.toml.
pub fn find_config_dir(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join(CONFIG_FILE).is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// If `dir` is a linked Git worktree (its `.git` is a file reading
/// `gitdir: <main>/.git/worktrees/<name>`), return the main checkout folder.
/// If `.git` is a directory, or absent, `dir` itself is the root.
pub fn main_checkout_root(dir: &Path) -> PathBuf {
    let dot_git = dir.join(".git");
    if dot_git.is_file() {
        if let Ok(text) = std::fs::read_to_string(&dot_git) {
            if let Some(g) = text.lines().find_map(|l| l.trim().strip_prefix("gitdir:")) {
                let gitdir = dir.join(g.trim()); // an absolute gitdir replaces `dir`
                // <main>/.git/worktrees/<name>: three parents up is <main>
                if let Some(main_root) = gitdir.parent().and_then(Path::parent).and_then(Path::parent) {
                    if main_root.join(CONFIG_FILE).is_file() {
                        return normalize(main_root);
                    }
                }
            }
        }
    }
    normalize(dir)
}

pub fn load() -> Result<Config, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?;
    let dir = find_config_dir(&cwd).ok_or_else(|| format!("{CONFIG_FILE} not found in {} or any parent", cwd.display()))?;
    // Pause flag, scratch, logs and build all live in the main checkout, shared by every worktree.
    Config::load_from(&main_checkout_root(&dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ints_strings_comments() {
        let m = parse_flat("# hi\na = 5   # five\nb = \"x # y\"\n\nc = \"\"\n").unwrap();
        assert_eq!(m["a"], Value::Int(5));
        assert_eq!(m["b"], Value::Str("x # y".into()));
        assert_eq!(m["c"], Value::Str(String::new()));
    }

    #[test]
    fn rejects_bad_lines() {
        assert!(parse_flat("novalue").is_err());
        assert!(parse_flat("a = \"open").is_err());
        assert!(parse_flat("a = 1.5").is_err());
    }

    #[test]
    fn config_from_real_file_text() {
        let text = include_str!("../../../broadsword.toml");
        let c = Config::from_text(text, Path::new("proj")).unwrap();
        assert_eq!(c.scratch_dir, Path::new("proj").join(".").join("scratch"));
        assert!(c.disk_floor_gb > 0 && c.log_max_mb > 0);
        assert_eq!(c.projects_dir, Path::new("proj").join(".").join("projects"));
        assert!(c.job_disk_poll_seconds > 0);
        assert_eq!(c.state_dir, Path::new("proj").join(".").join("state"));
        assert!(c.checkpoint_ignored_max_mb > 0 && c.max_resumes > 0);
        assert_eq!(c.install_dir, Path::new("proj").join(".").join("build/bin"));
        assert!(c.mcp_call_timeout_seconds > 0 && c.mcp_output_max_kb > 0);
        assert_eq!(c.worktrees_dir, Path::new("proj").join(".").join("worktrees"));
        assert!(c.build_lock_wait_seconds > 0);
    }

    /// The shipped broadsword.toml with some `key = "value"` lines replaced.
    fn real_toml_with(changes: &[(&str, &str)]) -> String {
        let text = include_str!("../../../broadsword.toml");
        text.lines()
            .map(|l| {
                let key = l.split('=').next().unwrap_or("").trim();
                match changes.iter().find(|(k, _)| *k == key) {
                    Some((k, v)) => format!("{k} = \"{v}\""),
                    None => l.to_string(),
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn folder_overlap_is_refused_when_the_inner_folder_is_the_worktrees_dir() {
        // worktrees_dir inside another managed folder (the old one-way check)
        for (key, val) in [("scratch_dir", "scratch/w"), ("build_dir", "build/w"), ("state_dir", "state/w")] {
            let text = real_toml_with(&[("worktrees_dir", val), (key, &val[..val.find('/').unwrap()])]);
            let e = Config::from_text(&text, Path::new("p")).unwrap_err();
            assert!(e.contains("worktrees_dir must not be inside"), "{key}: {e}");
        }
    }

    #[test]
    fn folder_overlap_is_refused_when_the_other_folder_is_inside_worktrees_dir() {
        // the reverse direction: scratch, build, state or install inside worktrees_dir (the sweep would delete task worktrees)
        for (key, val) in [("scratch_dir", "worktrees/p"), ("build_dir", "worktrees/p"), ("state_dir", "worktrees/p"), ("install_dir", "worktrees/bin")] {
            let text = real_toml_with(&[("worktrees_dir", "worktrees"), (key, val)]);
            let e = Config::from_text(&text, Path::new("p")).unwrap_err();
            assert!(e.contains(&format!("{key} must not be inside worktrees_dir")), "{key}: {e}");
        }
    }

    #[test]
    fn folder_overlap_is_refused_in_both_directions_for_state_and_install() {
        // state vs scratch/build, install vs scratch/state: each way round, and the same folder twice
        let cases: [(&str, &str, &str, &str); 10] = [
            ("build_dir", "scratch/b", "scratch_dir", "scratch"),
            ("scratch_dir", "build/s", "build_dir", "build"),
            ("state_dir", "scratch/s", "scratch_dir", "scratch"),
            ("scratch_dir", "state/x", "state_dir", "state"),
            ("state_dir", "build/s", "build_dir", "build"),
            ("build_dir", "state/b", "state_dir", "state"),
            ("install_dir", "scratch/bin", "scratch_dir", "scratch"),
            ("scratch_dir", "bin/s", "install_dir", "bin"),
            ("install_dir", "state/bin", "state_dir", "state"),
            ("state_dir", "scratch", "scratch_dir", "scratch"),
        ];
        for (a, av, b, bv) in cases {
            let text = real_toml_with(&[(a, av), (b, bv)]);
            assert!(Config::from_text(&text, Path::new("p")).is_err(), "{a}={av} with {b}={bv} must be refused");
        }
        // install_dir inside build_dir is the shipped default and stays allowed; the shipped file loads
        assert!(Config::from_text(&real_toml_with(&[]), Path::new("p")).is_ok());
        assert!(Config::from_text(&real_toml_with(&[("install_dir", "build/bin")]), Path::new("p")).is_ok());
    }

    #[test]
    fn worktrees_dir_is_required_and_validated() {
        let base = "root = \".\"\ndisk_floor_gb=1\ncleanup_every_hours=1\nscratch_max_age_hours=1\ndisk_check_every_seconds=1\nlog_max_mb=1\nram_warn_percent=1\ncpu_warn_percent=1\nalert_command=\"\"\nbuild_dir=\"build\"\njob_disk_reserve_gb=2\njob_disk_poll_seconds=5\nprojects_dir=\"projects\"\nstate_dir=\"state\"\ncheckpoint_ignored_max_mb=50\nmax_resumes=3\nscratch_dir=\"scratch\"\nmcp_call_timeout_seconds=5\nmcp_output_max_kb=5\ntask_lock_wait_seconds = 1\nbuild_lock_wait_seconds = 1\nrecover_lock_wait_seconds = 5
codex_coder_timeout_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_model_high_fallback = \"sonnet\"\nreviewer_model_high_gpt = \"gpt-6.1-sol\"\nreviewer_effort_high = \"high\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
merge_train_max = 4
merge_train_lanes = 1
merge_train_wait_minutes = 20
review_parallel = 1
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
max_queued = 50
queue_entry_read_wait_ms = 2000
perf_gate = \"on\"
perf_runs = 5
queue_per_project = \"off\"
queue_auto_revert = \"on\"
queue_main_check_async = \"off\"
merge_fast_tests = \"\"
python_tests = \"\"
task_archive_days = 30
main_full_check_every_minutes = 60
done_check_timeout_seconds = 600
done_check_tail_lines = 40
test_parallel_binaries = 1
python_parallel = 1
test_output_wait_seconds = 10
hotspot_lines = 1500
hotspot_window_hours = 48
hotspot_wait_hours = 1
hotspot_claims = 5
hotspot_commits = 5
stall_restart_lines = 3
stall_restart_timeout_seconds = 60
stale_claim_hours = 4
auto_agent_timeout_minutes = 90
auto_agent_min_gap_minutes = 150
auto_agent_max_starts_per_day = 6
install_dir=\"build/bin\"\n";
        assert!(Config::from_text(&format!("{base}worktrees_dir=\"worktrees\"\n"), Path::new("p")).is_ok());
        assert!(Config::from_text(&format!("{base}worktrees_dir=\"a/wt\"\n"), Path::new("p")).is_ok());
        assert!(Config::from_text(base, Path::new("p")).is_err(), "worktrees_dir is required");
        for bad in ["../w", "/w", "C:/w", ".", "a/../../w", "scratch", "scratch/w", "build/w", "state/w", "state"] {
            assert!(Config::from_text(&format!("{base}worktrees_dir=\"{bad}\"\n"), Path::new("p")).is_err(), "worktrees_dir {bad}");
        }
    }

    #[test]
    fn task_archive_days_is_required_and_positive() {
        let text = include_str!("../../../broadsword.toml");
        assert_eq!(Config::from_text(text, Path::new("p")).unwrap().task_archive_days, 30);
        let without = text.lines().filter(|l| !l.trim_start().starts_with("task_archive_days")).collect::<Vec<_>>().join("\n");
        let e = Config::from_text(&without, Path::new("p")).unwrap_err();
        assert!(e.contains("task_archive_days"), "{e}");
        for value in ["0", "-1", "\"\""] {
            let bad = format!("{without}\ntask_archive_days = {value}\n");
            let e = Config::from_text(&bad, Path::new("p")).unwrap_err();
            assert!(e.contains("task_archive_days"), "{value}: {e}");
        }
        for value in ["1.5", "18446744073709551616"] {
            assert!(Config::from_text(&format!("{without}\ntask_archive_days = {value}\n"), Path::new("p")).is_err(), "{value}");
        }
    }

    #[test]
    fn build_lock_wait_seconds_is_required_and_positive() {
        let text = include_str!("../../../broadsword.toml");
        assert!(Config::from_text(text, Path::new("p")).is_ok());
        let without: String = text.lines().filter(|l| !l.trim_start().starts_with("build_lock_wait_seconds")).collect::<Vec<_>>().join("\n");
        let e = Config::from_text(&without, Path::new("p")).unwrap_err();
        assert!(e.contains("build_lock_wait_seconds"), "{e}");
        let zero: String = text.lines().map(|l| if l.trim_start().starts_with("build_lock_wait_seconds") { "build_lock_wait_seconds = 0" } else { l }).collect::<Vec<_>>().join("\n");
        assert!(Config::from_text(&zero, Path::new("p")).is_err(), "0 would never wait");
    }

    #[test]
    fn reviewer_and_queue_settings_are_required_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        let c = Config::from_text(text, Path::new("p")).unwrap();
        assert_eq!((c.reviewer_backend.as_str(), c.reviewer_model_low.as_str(), c.reviewer_model_high.as_str()), ("claude", "haiku", "opus"));
        for key in [
            "reviewer_backend", "reviewer_cli", "reviewer_model_low", "reviewer_model_high", "reviewer_model_high_fallback", "reviewer_model_high_gpt", "reviewer_effort_high", "reviewer_timeout_seconds", "reviewer_diff_max_kb", "reviewer_brief_max_kb", "reviewer_check_tail_kb",
            "queue_lock_wait_seconds", "reviewer_max_calls_per_entry", "max_agents", "max_queued", "queue_entry_read_wait_ms", "merge_train_max", "merge_train_wait_minutes", "review_parallel",
        ] {
            let without: String = text.lines().filter(|l| !l.trim_start().starts_with(key)).collect::<Vec<_>>().join("
");
            let e = Config::from_text(&without, Path::new("p")).unwrap_err();
            assert!(e.contains(key), "{key}: {e}");
        }
        let with = |from: &str, to: &str| text.lines().map(|l| if l.trim_start().starts_with(from) { to.to_string() } else { l.to_string() }).collect::<Vec<_>>().join("
");
        assert!(Config::from_text(&with("reviewer_backend", "reviewer_backend = \"openai\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_model_low", "reviewer_model_low = \"--evil\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_model_high", "reviewer_model_high = \"a b\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_model_high_fallback", "reviewer_model_high_fallback = \"a b\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_model_high_gpt", "reviewer_model_high_gpt = \"--evil\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_effort_high", "reviewer_effort_high = \"huge\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_timeout_seconds", "reviewer_timeout_seconds = 0"), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_cli", "reviewer_cli = \"\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("max_agents", "max_agents = 0"), Path::new("p")).is_err(), "max_agents = 0 would start nothing");
        assert!(Config::from_text(&with("max_queued", "max_queued = 0"), Path::new("p")).is_err(), "max_queued = 0 would start nothing");
        assert!(c.max_agents >= 1);
        assert_eq!(c.max_queued, 30, "the shipped max_queued");
        assert!(Config::from_text(&with("queue_entry_read_wait_ms", "queue_entry_read_wait_ms = 0"), Path::new("p")).is_err(), "0 would never wait for an entry being written");
        assert_eq!(c.queue_entry_read_wait_ms, 2000, "the shipped queue_entry_read_wait_ms");
        assert!(Config::from_text(&with("merge_train_max", "merge_train_max = 0"), Path::new("p")).is_err(), "0 would merge nothing: 1 means off");
        assert_eq!(c.merge_train_max, 4, "shipped 4 for the trial (Lazlo 2026-11-19); 1 turns the train off");
        assert!(Config::from_text(&with("merge_train_wait_minutes", "merge_train_wait_minutes = 0"), Path::new("p")).is_err(), "0 would make every approved entry its own train");
        assert!(Config::from_text(&with("review_parallel", "review_parallel = 0"), Path::new("p")).is_err(), "0 would review nothing: 1 means one at a time");
        assert_eq!(c.review_parallel, 3, "the shipped review_parallel");
        assert_eq!(c.merge_train_wait_minutes, 20, "the shipped merge_train_wait_minutes");
        assert!(valid_model_name("claude-haiku-4-5") && valid_model_name("opus[1m]") && !valid_model_name("") && !valid_model_name("-x"));
    }

    #[test]
    fn shadow_reviewer_settings_are_required_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        let c = Config::from_text(text, Path::new("p")).unwrap();
        // The shipped file may have the shadow switched off (safety stop cfadd9e); either valid value is fine here.
        assert!(matches!(c.shadow_reviewer.as_str(), "openai" | "off"), "shipped shadow_reviewer: {}", c.shadow_reviewer);
        assert_eq!(c.shadow_reviewer_model.as_str(), "gpt-6.2-sol");
        for key in ["shadow_reviewer", "shadow_reviewer_model", "shadow_reviewer_timeout_seconds"] {
            let e = Config::from_text(&without_key(text, key), Path::new("p")).unwrap_err();
            assert!(e.contains(key) && e.contains("missing"), "{key}: {e}");
        }
        let with = |key: &str, line: &str| text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == key) { line.to_string() } else { l.to_string() }).collect::<Vec<_>>().join("
");
        assert!(Config::from_text(&with("shadow_reviewer", "shadow_reviewer = \"off\""), Path::new("p")).is_ok());
        assert!(Config::from_text(&with("shadow_reviewer", "shadow_reviewer = \"claude\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("shadow_reviewer_model", "shadow_reviewer_model = \"--evil\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("shadow_reviewer_timeout_seconds", "shadow_reviewer_timeout_seconds = 0"), Path::new("p")).is_err());
    }

    #[test]
    fn perf_gate_settings_are_required_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        let c = Config::from_text(text, Path::new("p")).unwrap();
        assert!(c.perf_gate, "shipped on");
        assert_eq!(c.perf_runs, 5, "the shipped perf_runs");
        for key in ["perf_gate", "perf_runs"] {
            let e = Config::from_text(&without_key(text, key), Path::new("p")).unwrap_err();
            assert!(e.contains(key) && e.contains("missing"), "{key}: {e}");
        }
        let with = |key: &str, line: &str| text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == key) { line.to_string() } else { l.to_string() }).collect::<Vec<_>>().join("\n");
        assert!(!Config::from_text(&with("perf_gate", "perf_gate = \"off\""), Path::new("p")).unwrap().perf_gate);
        for bad in ["perf_gate = \"yes\"", "perf_gate = true", "perf_gate = \"\""] {
            assert!(Config::from_text(&with("perf_gate", bad), Path::new("p")).is_err(), "{bad}");
        }
        assert!(Config::from_text(&with("perf_runs", "perf_runs = 0"), Path::new("p")).is_err(), "0 runs would judge nothing");
    }

    #[test]
    fn queue_per_project_is_required_shipped_off_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        // the shipped value may be "on" or "off" (a trial setting, 2026-11-20); only that it loads is pinned here
        Config::from_text(text, Path::new("p")).unwrap();
        let e =Config::from_text(&without_key(text, "queue_per_project"), Path::new("p")).unwrap_err();
        assert!(e.contains("queue_per_project") && e.contains("missing"), "{e}");
        let with = |line: &str| text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == "queue_per_project") { line.to_string() } else { l.to_string() }).collect::<Vec<_>>().join("\n");
        assert!(Config::from_text(&with("queue_per_project = \"on\""), Path::new("p")).unwrap().queue_per_project);
        for bad in ["queue_per_project = true", "queue_per_project = \"yes\"", "queue_per_project = \"\"", "queue_per_project = 1"] {
            assert!(Config::from_text(&with(bad), Path::new("p")).is_err(), "{bad}");
        }
    }

    #[test]
    fn queue_main_check_async_is_required_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        let e = Config::from_text(&without_key(text, "queue_main_check_async"), Path::new("p")).unwrap_err();
        assert!(e.contains("queue_main_check_async") && e.contains("missing"), "{e}");
        for value in ["maybe", "", "ON", "on\\r", "off\\n"] {
            let e = Config::from_text(&real_toml_with(&[("queue_main_check_async", value)]), Path::new("p")).unwrap_err();
            assert!(e.contains("queue_main_check_async"), "{e}");
        }
        for (value, expected) in [("on", true), ("off", false)] {
            assert_eq!(Config::from_text(&real_toml_with(&[("queue_main_check_async", value)]), Path::new("p")).unwrap().queue_main_check_async, expected);
        }
    }

    #[test]
    fn done_check_settings_are_required_and_positive() {
        let text = include_str!("../../../broadsword.toml");
        let c = Config::from_text(text, Path::new("p")).unwrap();
        assert!(c.done_check_timeout_seconds > c.build_lock_wait_seconds, "a done check waiting for the build lock must not be killed first");
        assert!(c.done_check_tail_lines >= 1);
        for key in ["done_check_timeout_seconds", "done_check_tail_lines"] {
            let e = Config::from_text(&without_key(text, key), Path::new("p")).unwrap_err();
            assert!(e.contains(key), "{key}: {e}");
            let zero: String = text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == key) { format!("{key} = 0") } else { l.to_string() }).collect::<Vec<_>>().join("\n");
            assert!(Config::from_text(&zero, Path::new("p")).is_err(), "{key} = 0");
        }
    }

    #[test]
    fn test_parallel_binaries_is_required_and_positive() {
        let text = include_str!("../../../broadsword.toml");
        assert!(Config::from_text(text, Path::new("p")).unwrap().test_parallel_binaries >= 1);
        let e = Config::from_text(&without_key(text, "test_parallel_binaries"), Path::new("p")).unwrap_err();
        assert!(e.contains("test_parallel_binaries") && e.contains("missing"), "{e}");
        let zero: String = text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == "test_parallel_binaries") { "test_parallel_binaries = 0" } else { l }).collect::<Vec<_>>().join("\n");
        assert!(Config::from_text(&zero, Path::new("p")).is_err(), "0 would run no test");
        assert!(Config::from_text(text, Path::new("p")).unwrap().test_output_wait_seconds >= 1);
        let e = Config::from_text(&without_key(text, "test_output_wait_seconds"), Path::new("p")).unwrap_err();
        assert!(e.contains("test_output_wait_seconds") && e.contains("missing"), "{e}");
    }

    #[test]
    fn python_parallel_is_required_and_positive() {
        let text = include_str!("../../../broadsword.toml");
        assert!(Config::from_text(text, Path::new("p")).unwrap().python_parallel >= 1);
        let without = without_key(text, "python_parallel");
        let e = Config::from_text(&without, Path::new("p")).unwrap_err();
        assert!(e.contains("python_parallel") && e.contains("missing"), "{e}");
        for bad in ["0", "-1", "\"\"", "\"3\"", "\"NaN\"", "\"inf\""] {
            let e = Config::from_text(
                &format!("{without}\npython_parallel = {bad}\n"),
                Path::new("p"),
            )
            .unwrap_err();
            assert!(e.contains("python_parallel"), "{bad}: {e}");
        }
        for width in [1, 3] {
            let c = Config::from_text(
                &format!("{without}\npython_parallel = {width}\n"),
                Path::new("p"),
            )
            .unwrap();
            assert_eq!(c.python_parallel, width);
        }
    }

    #[test]
    fn hotspot_keys_are_required_and_positive() {
        let text = include_str!("../../../broadsword.toml");
        assert!(Config::from_text(text, Path::new("p")).is_ok());
        for key in ["hotspot_lines", "hotspot_window_hours", "hotspot_wait_hours", "hotspot_claims", "hotspot_commits", "stall_restart_lines", "stall_restart_timeout_seconds", "stale_claim_hours", "auto_agent_timeout_minutes", "auto_agent_min_gap_minutes", "auto_agent_max_starts_per_day"] {
            let e = Config::from_text(&without_key(text, key), Path::new("p")).unwrap_err();
            assert!(e.contains(key) && e.contains("missing"), "{key}: {e}");
            let zero: String = text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == key) { format!("{key} = 0") } else { l.to_string() }).collect::<Vec<_>>().join("
");
            let e = Config::from_text(&zero, Path::new("p")).unwrap_err();
            assert!(e.contains(key), "{key} = 0: {e}");
        }
    }

    #[test]
    fn python_tests_is_required_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        let shipped = Config::from_text(text, Path::new("p")).unwrap().python_tests;
        let own = shipped.iter().find(|(p, _)| p == "broadsword").expect("Broadsword declares its own Python tests");
        assert!(
            own.1.iter().all(|s| s.starts_with("scripts/test_")
                || s.starts_with("clients/test_")
                || (s.starts_with("bench/")
                    && Path::new(s).file_name().is_some_and(|name| name.to_string_lossy().starts_with("test_")))),
            "{own:?}"
        );
        let e = Config::from_text(&without_key(text, "python_tests"), Path::new("p")).unwrap_err();
        assert!(e.contains("python_tests") && e.contains("missing"), "{e}");
        assert_eq!(parse_python_tests("").unwrap(), vec![]);
        assert_eq!(parse_python_tests("  ;  ").unwrap(), vec![]);
        assert_eq!(
            parse_python_tests("broadsword: scripts/test_a.py  clients/test_b.py; crash-sim: tests/t.py").unwrap(),
            vec![("broadsword".to_string(), vec!["scripts/test_a.py".to_string(), "clients/test_b.py".to_string()]), ("crash-sim".to_string(), vec!["tests/t.py".to_string()])]
        );
        for bad in ["broadsword", "broadsword:", "p: a.py; P: b.py", "p: ../x.py", "p: /x.py", "p: C:/x.py", "p: a/../../x.py", "p: x.sh", "p: .", ": a.py", "a b: x.py", "p: x.py\" --evil", "p: -c.py", "p: -m.py"] {
            assert!(parse_python_tests(bad).is_err(), "{bad}");
        }
        let with = |v: &str| text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == "python_tests") { format!("python_tests = \"{v}\"") } else { l.to_string() }).collect::<Vec<_>>().join("\n");
        assert!(Config::from_text(&with("p: ../x.py"), Path::new("p")).unwrap_err().contains("python_tests"));
        assert!(Config::from_text(&with(""), Path::new("p")).unwrap().python_tests.is_empty());
    }

    #[test]
    fn merge_train_lanes_is_required_and_positive() {
        let text = include_str!("../../../broadsword.toml");
        let load = |s: &str| Config::from_text(s, Path::new("p"));
        assert_eq!(load(text).unwrap().merge_train_lanes, 2);
        let e = load(&without_key(text, "merge_train_lanes")).unwrap_err();
        assert!(e.contains("merge_train_lanes") && e.contains("missing"), "{e}");
        for bad in ["0", "-1", "\"NaN\"", "\"inf\"", "\"\"", "\"1.5\""] {
            let changed = text.replace("merge_train_lanes = 2", &format!("merge_train_lanes = {bad}"));
            let e = load(&changed).unwrap_err();
            assert!(e.contains("merge_train_lanes"), "{bad}: {e}");
        }
    }

    #[test]
    fn merge_fast_tests_is_required_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        let e = Config::from_text(&without_key(text, "merge_fast_tests"), Path::new("p")).unwrap_err();
        assert!(e.contains("merge_fast_tests") && e.contains("missing"), "{e}");
        assert_eq!(parse_merge_fast_tests("").unwrap(), vec![]);
        assert_eq!(parse_merge_fast_tests("broadsword: queue  wtgit; crash-sim: t_1").unwrap(), vec![("broadsword".to_string(), vec!["queue".to_string(), "wtgit".to_string()]), ("crash-sim".to_string(), vec!["t_1".to_string()])]);
        for bad in ["broadsword", "broadsword:", "a: x; A: y", "a: --lib", "a: ../x", "a: x.rs", "a b: x"] {
            assert!(parse_merge_fast_tests(bad).is_err(), "{bad}");
        }
        // the shipped value lists the self project with test files of this crate (the queue checks the names against cargo metadata
        // at every pre-merge check; this only keeps the shipped value from naming a file that is gone), and needs auto-revert on
        let shipped = Config::from_text(text, Path::new("p")).unwrap();
        let names = shipped.merge_fast_subset("broadsword").expect("the shipped value lists broadsword");
        let tests_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        assert!(!names.is_empty() && names.iter().all(|n| tests_dir.join(format!("{n}.rs")).is_file()), "{names:?}");
        assert_eq!(shipped.merge_fast_subset("not-listed"), None);
        assert_eq!(shipped.merge_fast_subset("BroadSword"), Some(names));
        let off = text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == "queue_auto_revert") { "queue_auto_revert = \"off\"" } else { l }).collect::<Vec<_>>().join("\n");
        assert!(Config::from_text(&off, Path::new("p")).unwrap_err().contains("merge_fast_tests needs queue_auto_revert = on"));
    }

    #[test]
    fn install_dir_and_mcp_settings_are_validated() {
        let base = "root = \".\"\ndisk_floor_gb=1\ncleanup_every_hours=1\nscratch_max_age_hours=1\ndisk_check_every_seconds=1\nlog_max_mb=1\nram_warn_percent=1\ncpu_warn_percent=1\nalert_command=\"\"\nbuild_dir=\"build\"\njob_disk_reserve_gb=2\njob_disk_poll_seconds=5\nprojects_dir=\"projects\"\nstate_dir=\"state\"\ncheckpoint_ignored_max_mb=50\nmax_resumes=3\nscratch_dir=\"scratch\"\nmcp_call_timeout_seconds=5\nmcp_output_max_kb=5\ntask_lock_wait_seconds = 1\nbuild_lock_wait_seconds = 1\nrecover_lock_wait_seconds = 5
codex_coder_timeout_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_model_high_fallback = \"sonnet\"\nreviewer_model_high_gpt = \"gpt-6.1-sol\"\nreviewer_effort_high = \"high\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
merge_train_max = 4
merge_train_lanes = 1
merge_train_wait_minutes = 20
review_parallel = 1
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
max_queued = 50
queue_entry_read_wait_ms = 2000
perf_gate = \"on\"
perf_runs = 5
queue_per_project = \"off\"
queue_auto_revert = \"on\"
queue_main_check_async = \"off\"
merge_fast_tests = \"\"
python_tests = \"\"
task_archive_days = 30
main_full_check_every_minutes = 60
done_check_timeout_seconds = 600
done_check_tail_lines = 40
test_parallel_binaries = 1
python_parallel = 1
test_output_wait_seconds = 10
hotspot_lines = 1500
hotspot_window_hours = 48
hotspot_wait_hours = 1
hotspot_claims = 5
hotspot_commits = 5
stall_restart_lines = 3
stall_restart_timeout_seconds = 60
stale_claim_hours = 4
auto_agent_timeout_minutes = 90
auto_agent_min_gap_minutes = 150
auto_agent_max_starts_per_day = 6
worktrees_dir = \"worktrees\"\n";
        assert!(Config::from_text(&format!("{base}install_dir=\"build/bin\"\n"), Path::new("p")).is_ok());
        for bad in ["../b", "/b", "C:/b", ".", "scratch/bin", "state/x", "a/../../b"] {
            assert!(Config::from_text(&format!("{base}install_dir=\"{bad}\"\n"), Path::new("p")).is_err(), "install_dir {bad}");
        }
        assert!(Config::from_text(base, Path::new("p")).is_err(), "install_dir is required");
        for key in ["mcp_call_timeout_seconds", "mcp_output_max_kb"] {
            let zero = base.replace(&format!("{key}=5"), &format!("{key}=0"));
            assert!(Config::from_text(&format!("{zero}install_dir=\"build/bin\"\n"), Path::new("p")).is_err(), "{key}=0");
        }
    }

    #[test]
    fn worktree_resolves_to_main_root() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-wt-{}", std::process::id()));
        let main = base.join("main");
        let wt = main.join("wt").join("a");
        let gitdir = main.join(".git").join("worktrees").join("a");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::write(main.join(CONFIG_FILE), "root = \".\"").unwrap();
        std::fs::write(wt.join(CONFIG_FILE), "root = \".\"").unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        assert_eq!(main_checkout_root(&wt), main);
        assert_eq!(main_checkout_root(&main), main); // .git is a directory: already main
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn rejects_paths_outside_root() {
        let base = "root = \".\"\ndisk_floor_gb=1\ncleanup_every_hours=1\nscratch_max_age_hours=1\ndisk_check_every_seconds=1\nlog_max_mb=1\nram_warn_percent=1\ncpu_warn_percent=1\nalert_command=\"\"\nbuild_dir=\"build\"\njob_disk_reserve_gb=2\njob_disk_poll_seconds=5\nprojects_dir=\"projects\"\nstate_dir=\"state\"\ncheckpoint_ignored_max_mb=50\nmax_resumes=3\ninstall_dir=\"build/bin\"\nmcp_call_timeout_seconds=5\nmcp_output_max_kb=5\ntask_lock_wait_seconds = 1\nbuild_lock_wait_seconds = 1\nrecover_lock_wait_seconds = 5
codex_coder_timeout_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_model_high_fallback = \"sonnet\"\nreviewer_model_high_gpt = \"gpt-6.1-sol\"\nreviewer_effort_high = \"high\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
merge_train_max = 4
merge_train_wait_minutes = 20
review_parallel = 1
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
max_queued = 50
queue_entry_read_wait_ms = 2000
perf_gate = \"on\"
perf_runs = 5
queue_per_project = \"off\"
queue_auto_revert = \"on\"
merge_fast_tests = \"\"
queue_main_check_async = \"off\"
python_tests = \"\"
task_archive_days = 30
main_full_check_every_minutes = 60
done_check_timeout_seconds = 600
done_check_tail_lines = 40
test_parallel_binaries = 1
python_parallel = 1
test_output_wait_seconds = 10
hotspot_lines = 1500
hotspot_window_hours = 48
hotspot_wait_hours = 1
hotspot_claims = 5
hotspot_commits = 5
stall_restart_lines = 3
stall_restart_timeout_seconds = 60
stale_claim_hours = 4
auto_agent_timeout_minutes = 90
auto_agent_min_gap_minutes = 150
auto_agent_max_starts_per_day = 6
merge_train_lanes = 1
worktrees_dir = \"worktrees\"\n";
        for bad in ["../x", "C:/abs", "/abs", ".", "a/../../b"] {
            let t = format!("{base}scratch_dir = \"{bad}\"\n");
            assert!(Config::from_text(&t, Path::new("p")).is_err(), "{bad}");
        }
        for bad in ["../p", "/p", "C:/p", "."] {
            let t = format!("{base}scratch_dir = \"scratch\"\nprojects_dir = \"{bad}\"\n");
            assert!(Config::from_text(&t, Path::new("p")).is_err(), "projects_dir {bad}");
        }
        for bad in ["../s", "/s", "C:/s", "."] {
            let t = format!("{base}scratch_dir = \"scratch\"\nprojects_dir = \"projects\"\nstate_dir = \"{bad}\"\n");
            assert!(Config::from_text(&t, Path::new("p")).is_err(), "state_dir {bad}");
        }
        for inside in ["scratch/state", "scratch", "build/x"] {
            let t = format!("{base}scratch_dir = \"scratch\"\nstate_dir = \"{inside}\"\n");
            assert!(Config::from_text(&t, Path::new("p")).is_err(), "state_dir {inside}");
        }
        let t = format!("{base}scratch_dir = \"a/b\"\nprojects_dir = \"projects\"\nstate_dir = \"state\"\n");
        assert!(Config::from_text(&t, Path::new("p")).is_ok());
    }

    fn without_key(text: &str, key: &str) -> String {
        text.lines().filter(|l| l.split_once('=').is_none_or(|(k, _)| k.trim() != key)).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn limits_that_were_hard_coded_are_required_positive_settings() {
        let text = include_str!("../../../broadsword.toml");
        let c = Config::from_text(text, Path::new("p")).unwrap();
        assert_eq!((c.recover_lock_wait_seconds, c.sweep_older_than_days, c.redact_min_chars), (5, 7, 6), "the shipped values are the old constants");
        // every setting that used to be clamped silently with `.max(1)`, or fed a loop, is refused at 0 at load
        for key in ["recover_lock_wait_seconds", "sweep_older_than_days", "redact_min_chars", "job_disk_poll_seconds", "disk_check_every_seconds", "cleanup_every_hours", "scratch_max_age_hours", "log_max_mb", "codex_coder_timeout_seconds"] {
            let e = Config::from_text(&without_key(text, key), Path::new("p")).unwrap_err();
            assert!(e.contains(key) && e.contains("missing"), "{key} is required: {e}");
            let zero = text.lines().map(|l| if l.split_once('=').is_some_and(|(k, _)| k.trim() == key) { format!("{key} = 0") } else { l.to_string() }).collect::<Vec<_>>().join("\n");
            let e = Config::from_text(&zero, Path::new("p")).unwrap_err();
            assert!(e.contains(key) && e.contains("at least 1"), "{key} = 0 is refused: {e}");
        }
    }

    #[test]
    fn every_key_in_the_real_toml_is_required_by_the_loader_and_used_by_code() {
        let text = include_str!("../../../broadsword.toml");
        let keys: Vec<String> = parse_flat(text).unwrap().into_keys().collect();
        assert!(keys.len() > 35, "{keys:?}");
        // 1. the loader reads it: with the key removed the config no longer loads (a key nothing reads would still load)
        let unread: Vec<&String> = keys.iter().filter(|k| Config::from_text(&without_key(text, k), Path::new("p")).is_ok()).collect();
        assert!(unread.is_empty(), "keys in broadsword.toml that the loader never reads (delete them, or read them): {unread:?}");
        // 2. some code outside this file uses the field it fills (`cfg.<key>`), so a validated-but-unused key (shared_dir) fails too
        // every .rs file under src/, subfolders included (src/queue/ holds the merge queue), except this file
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut code = String::new();
        let mut dirs = vec![src.clone()];
        while let Some(dir) = dirs.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                } else if p != src.join("config.rs") && p.extension().is_some_and(|x| x == "rs") {
                    code.push_str(&std::fs::read_to_string(&p).unwrap());
                }
            }
        }
        // a key may reach the code through an accessor method of Config instead of the field itself
        fn accessor(k: &str) -> &str {
            match k {
                "job_disk_reserve_gb" => "reserve_bytes",
                "merge_fast_tests" => "merge_fast_subset",
                _ => k,
            }
        }
        let used = |name: &str| code.match_indices(&format!(".{name}")).any(|(i, _)| !code[i + 1 + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'));
        // a key the Rust code never reads may be read by a Python client under clients/, by its exact quoted name
        let mut py = String::new();
        for e in std::fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../clients")).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.ends_with(".py") && !name.starts_with("test_") {
                py.push_str(&std::fs::read_to_string(e.path()).unwrap());
            }
        }
        // the hotspot_* keys are read only by scripts/hotspots.py (exactly that file, exactly those keys)
        let hotspots_py = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/hotspots.py")).unwrap();
        // the stall_restart_* keys are read only by scripts/stall_restart.py (exactly that file, exactly those keys)
        let stall_py = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/stall_restart.py")).unwrap();
        let stale_py = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/stale_claims.py")).unwrap();
        let auto_agent_py = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/auto_agent.py")).unwrap();
        let used_by_python = |k: &str| py.contains(&format!("\"{k}\"")) || (k.starts_with("hotspot_") && hotspots_py.contains(&format!("\"{k}\""))) || (k.starts_with("stall_restart_") && stall_py.contains(&format!("\"{k}\""))) || (k == "stale_claim_hours" && stale_py.contains(&format!("\"{k}\""))) || (k == "auto_agent_timeout_minutes" && auto_agent_py.contains("\"auto_agent_timeout_minutes\"")) || (k == "auto_agent_min_gap_minutes" && auto_agent_py.contains("\"auto_agent_min_gap_minutes\"")) || (k == "auto_agent_max_starts_per_day" && auto_agent_py.contains("\"auto_agent_max_starts_per_day\""));
        let unused: Vec<&String> = keys.iter().filter(|k| !used(k) && !used(accessor(k)) && !used_by_python(k)).collect();
        assert!(unused.is_empty(), "keys in broadsword.toml that no Rust code outside config.rs uses as `.key` and no clients/*.py reads by its quoted name (hotspot_* keys: scripts/hotspots.py; stall_restart_* keys: scripts/stall_restart.py; auto_agent_timeout_minutes, auto_agent_min_gap_minutes, auto_agent_max_starts_per_day: scripts/auto_agent.py): {unused:?}");
    }

    #[test]
    fn missing_key_is_error() {
        assert!(Config::from_text("root = \".\"", Path::new("p")).is_err());
    }

    #[test]
    fn finds_root_walking_up() {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-root-{}", std::process::id()));
        let deep = base.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(base.join(CONFIG_FILE), "root = \".\"").unwrap();
        assert_eq!(find_config_dir(&deep), Some(base.clone()));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
