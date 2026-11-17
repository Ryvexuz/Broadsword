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

impl Config {
    /// Build a Config from parsed text. `config_dir` is the folder holding broadsword.toml;
    /// `root` and every path in the file are resolved relative to it.
    pub fn from_text(text: &str, config_dir: &Path) -> Result<Config, String> {
        let m = parse_flat(text)?;
        let root = config_dir.join(get_str(&m, "root")?);
        Ok(Config {
            disk_floor_gb: get_int(&m, "disk_floor_gb")?,
            cleanup_every_hours: get_int(&m, "cleanup_every_hours")?,
            scratch_max_age_hours: get_int(&m, "scratch_max_age_hours")?,
            disk_check_every_seconds: get_int(&m, "disk_check_every_seconds")?,
            log_max_mb: get_int(&m, "log_max_mb")?,
            ram_warn_percent: get_int(&m, "ram_warn_percent")?,
            cpu_warn_percent: get_int(&m, "cpu_warn_percent")?,
            alert_command: get_str(&m, "alert_command")?,
            scratch_dir: root.join(get_str(&m, "scratch_dir")?),
            build_dir: root.join(get_str(&m, "build_dir")?),
            root,
        })
    }

    pub fn load_from(config_dir: &Path) -> Result<Config, String> {
        let file = config_dir.join(CONFIG_FILE);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        Config::from_text(&text, config_dir)
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

pub fn load() -> Result<Config, String> {
    let cwd = std::env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?;
    let dir = find_config_dir(&cwd).ok_or_else(|| format!("{CONFIG_FILE} not found in {} or any parent", cwd.display()))?;
    Config::load_from(&dir)
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
