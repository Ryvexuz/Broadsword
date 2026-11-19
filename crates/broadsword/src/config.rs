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
    /// The shadow second reviewer: `openai` (clients/openai_review.py, recorded only, never gates a merge) or `off`.
    pub shadow_reviewer: String,
    /// The shadow reviewer's model (an OpenAI model name).
    pub shadow_reviewer_model: String,
    /// A shadow review that runs longer than this is killed and recorded as `ERROR`; the merge goes on.
    pub shadow_reviewer_timeout_seconds: u64,
    /// The most tasks that may be `running` on the board at once, across all projects (`task start` refuses at the cap).
    pub max_agents: u64,
    /// How long `recover` (and `recover <id>`, `recover --abandon`) waits for the recover lock before it says another recover is running.
    pub recover_lock_wait_seconds: u64,
    /// `cargo sweep --time N`: build-folder files not used for N days are swept (by `cleanup`, and by the cleanup a low-disk kill runs).
    pub sweep_older_than_days: u64,
    /// A `.env` or secret-named value shorter than this many characters is not redacted from text (it would blank ordinary words);
    /// it is still kept out of every child's environment.
    pub redact_min_chars: u64,
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

impl Config {
    /// Build a Config from parsed text. `config_dir` is the folder holding broadsword.toml;
    /// `root` and every path in the file are resolved relative to it.
    pub fn from_text(text: &str, config_dir: &Path) -> Result<Config, String> {
        let m = parse_flat(text)?;
        let root = normalize(&config_dir.join(get_str(&m, "root")?));
        let (scratch_dir, build_dir) = (root.join(inside_root(&m, "scratch_dir", true)?), root.join(inside_root(&m, "build_dir", true)?));
        let state_dir = root.join(inside_root(&m, "state_dir", true)?);
        for (name, other) in [("scratch_dir", &scratch_dir), ("build_dir", &build_dir)] {
            if state_dir.starts_with(other) {
                return Err(format!("state_dir must not be inside {name}: cleanup would delete the job records"));
            }
        }
        let install_dir = root.join(inside_root(&m, "install_dir", true)?);
        for (name, other) in [("scratch_dir", &scratch_dir), ("state_dir", &state_dir)] {
            if install_dir.starts_with(other) {
                return Err(format!("install_dir must not be inside {name}: that folder is cleaned up or holds records, not the stable binary"));
            }
        }
        let worktrees_dir = root.join(inside_root(&m, "worktrees_dir", true)?);
        for (name, other) in [("scratch_dir", &scratch_dir), ("build_dir", &build_dir), ("state_dir", &state_dir)] {
            if worktrees_dir.starts_with(other) {
                return Err(format!("worktrees_dir must not be inside {name}: that folder is cleaned up or holds records, and a worktree holds a task's unmerged work"));
            }
        }
        let positive = |k: &str| get_int(&m, k).and_then(|v| if v == 0 { Err(format!("{k}: must be at least 1")) } else { Ok(v) });
        let (mcp_call_timeout_seconds, mcp_output_max_kb) = (positive("mcp_call_timeout_seconds")?, positive("mcp_output_max_kb")?);
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
        let shadow_reviewer = get_str(&m, "shadow_reviewer")?;
        if shadow_reviewer != "openai" && shadow_reviewer != "off" {
            return Err(format!("shadow_reviewer = \"{shadow_reviewer}\": must be \"openai\" or \"off\""));
        }
        Ok(Config {
            shadow_reviewer,
            shadow_reviewer_model: model("shadow_reviewer_model")?,
            shadow_reviewer_timeout_seconds: positive("shadow_reviewer_timeout_seconds")?,
            reviewer_backend,
            reviewer_cli,
            reviewer_model_low: model("reviewer_model_low")?,
            reviewer_model_high: model("reviewer_model_high")?,
            reviewer_timeout_seconds: positive("reviewer_timeout_seconds")?,
            reviewer_diff_max_kb: positive("reviewer_diff_max_kb")?,
            reviewer_brief_max_kb: positive("reviewer_brief_max_kb")?,
            reviewer_check_tail_kb: positive("reviewer_check_tail_kb")?,
            queue_lock_wait_seconds: positive("queue_lock_wait_seconds")?,
            reviewer_max_calls_per_entry: positive("reviewer_max_calls_per_entry")?,
            max_agents: positive("max_agents")?,
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
            projects_dir: root.join(inside_root(&m, "projects_dir", true)?),
            state_dir,
            install_dir,
            mcp_call_timeout_seconds,
            mcp_output_max_kb,
            checkpoint_ignored_max_mb: get_int(&m, "checkpoint_ignored_max_mb")?,
            max_resumes: get_int(&m, "max_resumes")?,
            job_disk_reserve_gb: get_int(&m, "job_disk_reserve_gb")?,
            job_disk_poll_seconds: positive("job_disk_poll_seconds")?,
            task_lock_wait_seconds: positive("task_lock_wait_seconds")?,
            build_lock_wait_seconds: positive("build_lock_wait_seconds")?,
            worktrees_dir,
            root,
        })
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

    #[test]
    fn worktrees_dir_is_required_and_validated() {
        let base = "root = \".\"\ndisk_floor_gb=1\ncleanup_every_hours=1\nscratch_max_age_hours=1\ndisk_check_every_seconds=1\nlog_max_mb=1\nram_warn_percent=1\ncpu_warn_percent=1\nalert_command=\"\"\nbuild_dir=\"build\"\njob_disk_reserve_gb=2\njob_disk_poll_seconds=5\nprojects_dir=\"projects\"\nstate_dir=\"state\"\ncheckpoint_ignored_max_mb=50\nmax_resumes=3\nscratch_dir=\"scratch\"\nmcp_call_timeout_seconds=5\nmcp_output_max_kb=5\ntask_lock_wait_seconds = 1\nbuild_lock_wait_seconds = 1\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
install_dir=\"build/bin\"\n";
        assert!(Config::from_text(&format!("{base}worktrees_dir=\"worktrees\"\n"), Path::new("p")).is_ok());
        assert!(Config::from_text(&format!("{base}worktrees_dir=\"a/wt\"\n"), Path::new("p")).is_ok());
        assert!(Config::from_text(base, Path::new("p")).is_err(), "worktrees_dir is required");
        for bad in ["../w", "/w", "C:/w", ".", "a/../../w", "scratch", "scratch/w", "build/w", "state/w", "state"] {
            assert!(Config::from_text(&format!("{base}worktrees_dir=\"{bad}\"\n"), Path::new("p")).is_err(), "worktrees_dir {bad}");
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
            "reviewer_backend", "reviewer_cli", "reviewer_model_low", "reviewer_model_high", "reviewer_timeout_seconds", "reviewer_diff_max_kb", "reviewer_brief_max_kb", "reviewer_check_tail_kb",
            "queue_lock_wait_seconds", "reviewer_max_calls_per_entry", "max_agents",
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
        assert!(Config::from_text(&with("reviewer_timeout_seconds", "reviewer_timeout_seconds = 0"), Path::new("p")).is_err());
        assert!(Config::from_text(&with("reviewer_cli", "reviewer_cli = \"\""), Path::new("p")).is_err());
        assert!(Config::from_text(&with("max_agents", "max_agents = 0"), Path::new("p")).is_err(), "max_agents = 0 would start nothing");
        assert!(c.max_agents >= 1);
        assert!(valid_model_name("claude-haiku-4-5") && valid_model_name("opus[1m]") && !valid_model_name("") && !valid_model_name("-x"));
    }

    #[test]
    fn shadow_reviewer_settings_are_required_and_validated() {
        let text = include_str!("../../../broadsword.toml");
        let c = Config::from_text(text, Path::new("p")).unwrap();
        // The shipped file may have the shadow switched off (safety stop cfadd9e); either valid value is fine here.
        assert!(matches!(c.shadow_reviewer.as_str(), "openai" | "off"), "shipped shadow_reviewer: {}", c.shadow_reviewer);
        assert_eq!(c.shadow_reviewer_model.as_str(), "gpt-5.6-sol");
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
    fn install_dir_and_mcp_settings_are_validated() {
        let base = "root = \".\"\ndisk_floor_gb=1\ncleanup_every_hours=1\nscratch_max_age_hours=1\ndisk_check_every_seconds=1\nlog_max_mb=1\nram_warn_percent=1\ncpu_warn_percent=1\nalert_command=\"\"\nbuild_dir=\"build\"\njob_disk_reserve_gb=2\njob_disk_poll_seconds=5\nprojects_dir=\"projects\"\nstate_dir=\"state\"\ncheckpoint_ignored_max_mb=50\nmax_resumes=3\nscratch_dir=\"scratch\"\nmcp_call_timeout_seconds=5\nmcp_output_max_kb=5\ntask_lock_wait_seconds = 1\nbuild_lock_wait_seconds = 1\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
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
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
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
        for key in ["recover_lock_wait_seconds", "sweep_older_than_days", "redact_min_chars", "job_disk_poll_seconds", "disk_check_every_seconds", "cleanup_every_hours", "scratch_max_age_hours", "log_max_mb"] {
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
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut code = String::new();
        for e in std::fs::read_dir(&src).unwrap().flatten() {
            if e.file_name() != "config.rs" && e.path().extension().is_some_and(|x| x == "rs") {
                code.push_str(&std::fs::read_to_string(e.path()).unwrap());
            }
        }
        // a key may reach the code through an accessor method of Config instead of the field itself
        fn accessor(k: &str) -> &str {
            if k == "job_disk_reserve_gb" { "reserve_bytes" } else { k }
        }
        let used = |name: &str| code.match_indices(&format!(".{name}")).any(|(i, _)| !code[i + 1 + name.len()..].starts_with(|c: char| c.is_alphanumeric() || c == '_'));
        let unused: Vec<&String> = keys.iter().filter(|k| !used(k) && !used(accessor(k))).collect();
        assert!(unused.is_empty(), "keys in broadsword.toml that no code outside config.rs uses as `.key`: {unused:?}");
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
