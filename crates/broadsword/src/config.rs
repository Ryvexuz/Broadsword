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
    pub job_disk_reserve_gb: u64,
    pub job_disk_poll_seconds: u64,
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
        inside_root(&m, "shared_dir", false)?;
        Ok(Config {
            disk_floor_gb: get_int(&m, "disk_floor_gb")?,
            cleanup_every_hours: get_int(&m, "cleanup_every_hours")?,
            scratch_max_age_hours: get_int(&m, "scratch_max_age_hours")?,
            disk_check_every_seconds: get_int(&m, "disk_check_every_seconds")?,
            log_max_mb: get_int(&m, "log_max_mb")?,
            ram_warn_percent: get_int(&m, "ram_warn_percent")?,
            cpu_warn_percent: get_int(&m, "cpu_warn_percent")?,
            alert_command: get_str(&m, "alert_command")?,
            scratch_dir: root.join(inside_root(&m, "scratch_dir", true)?),
            build_dir: root.join(inside_root(&m, "build_dir", true)?),
            projects_dir: root.join(inside_root(&m, "projects_dir", true)?),
            job_disk_reserve_gb: get_int(&m, "job_disk_reserve_gb")?,
            job_disk_poll_seconds: get_int(&m, "job_disk_poll_seconds")?,
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
    }

    #[test]
    fn worktree_resolves_to_main_root() {
        let base = std::env::temp_dir().join(format!("bs-wt-{}", std::process::id()));
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
        let base = "root = \".\"\ndisk_floor_gb=1\ncleanup_every_hours=1\nscratch_max_age_hours=1\ndisk_check_every_seconds=1\nlog_max_mb=1\nram_warn_percent=1\ncpu_warn_percent=1\nalert_command=\"\"\nbuild_dir=\"build\"\njob_disk_reserve_gb=2\njob_disk_poll_seconds=5\n";
        for bad in ["../x", "C:/abs", "/abs", ".", "a/../../b"] {
            let t = format!("{base}scratch_dir = \"{bad}\"\n");
            assert!(Config::from_text(&t, Path::new("p")).is_err(), "{bad}");
        }
        let t = format!("{base}scratch_dir = \"scratch\"\nshared_dir = \"..\"\n");
        assert!(Config::from_text(&t, Path::new("p")).is_err());
        for bad in ["../p", "/p", "C:/p", "."] {
            let t = format!("{base}scratch_dir = \"scratch\"\nprojects_dir = \"{bad}\"\n");
            assert!(Config::from_text(&t, Path::new("p")).is_err(), "projects_dir {bad}");
        }
        let t = format!("{base}scratch_dir = \"a/b\"\nprojects_dir = \"projects\"\n");
        assert!(Config::from_text(&t, Path::new("p")).is_ok());
    }

    #[test]
    fn missing_key_is_error() {
        assert!(Config::from_text("root = \".\"", Path::new("p")).is_err());
    }

    #[test]
    fn finds_root_walking_up() {
        let base = std::env::temp_dir().join(format!("bs-root-{}", std::process::id()));
        let deep = base.join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(base.join(CONFIG_FILE), "root = \".\"").unwrap();
        assert_eq!(find_config_dir(&deep), Some(base.clone()));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
