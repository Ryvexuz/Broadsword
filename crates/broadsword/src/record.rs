//! Durable job records: `<state_dir>/jobs/<job id>.rec`, plain `key = value` lines.
//!
//! A record is written when a job starts and rewritten at every step and at the end. Each write goes
//! to `<id>.rec.tmp` and is renamed over the record, so a kill can never leave a half-written record.
//! Records are never deleted by cleanup (they are the evidence recovery works from).
//! Values are one line: a backslash is written `\\` and a newline `\n`.

use crate::config::Config;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
pub const REFUSED: &str = "refused";
pub const KILLED_LOW_DISK: &str = "killed_low_disk";
pub const RESUMED: &str = "resumed";
pub const ROLLED_BACK: &str = "rolled_back";

#[derive(Debug, Clone)]
pub struct Record {
    pub path: PathBuf,
    fields: BTreeMap<String, String>,
}

fn esc(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\r', "").replace('\n', "\\n")
}

fn unesc(v: &str) -> String {
    let mut out = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some(o) => out.push(o),
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

impl Record {
    pub fn get(&self, k: &str) -> &str {
        self.fields.get(k).map(String::as_str).unwrap_or("")
    }

    pub fn get_num(&self, k: &str) -> Option<u64> {
        self.get(k).parse().ok()
    }

    pub fn set(&mut self, k: &str, v: impl Into<String>) {
        self.fields.insert(k.to_string(), v.into());
    }

    pub fn id(&self) -> &str {
        self.get("id")
    }

    pub fn status(&self) -> &str {
        self.get("status")
    }

    pub fn render(&self) -> String {
        self.fields.iter().map(|(k, v)| format!("{k} = {}\n", esc(v))).collect()
    }

    pub fn parse(path: &Path, text: &str) -> Record {
        let fields = text
            .lines()
            .filter_map(|l| l.split_once(" = ").or_else(|| l.strip_suffix(" =").map(|k| (k, ""))))
            .map(|(k, v)| (k.trim().to_string(), unesc(v)))
            .collect();
        Record { path: path.to_path_buf(), fields }
    }

    /// Write atomically: temp file in the same folder, then rename over the record.
    pub fn save(&self) -> std::io::Result<()> {
        let tmp = self.path.with_extension("rec.tmp");
        std::fs::write(&tmp, self.render())?;
        std::fs::rename(&tmp, &self.path)
    }
}

pub fn jobs_dir(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("jobs")
}

/// Keep ids safe for file names and Git ref names.
pub fn sanitize(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// Claim a unique job id: `<base>`, `<base>-2`, ... The claim is an exclusive create of the `.rec`
/// file, so two jobs started in the same second (even by two processes) never share an id.
pub fn create(dir: &Path, base: &str) -> std::io::Result<Record> {
    std::fs::create_dir_all(dir)?;
    let base = sanitize(base);
    for n in 1u32.. {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let path = dir.join(format!("{id}.rec"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => {
                let mut r = Record { path, fields: BTreeMap::new() };
                r.set("id", id);
                return Ok(r);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

pub fn load(path: &Path) -> Result<Record, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    Ok(Record::parse(path, &text))
}

/// Every readable record with an id, oldest first (ids start with a timestamp).
pub fn load_all(dir: &Path) -> Vec<Record> {
    let mut v: Vec<Record> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "rec"))
                .filter_map(|p| load(&p).ok())
                .filter(|r| !r.id().is_empty())
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.id().cmp(b.id()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bs-rec-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn round_trip_with_awkward_values() {
        let d = tmp("rt");
        let mut r = create(&d, "20260101-000000-p-run").unwrap();
        r.set("status", RUNNING);
        r.set("command", "echo a = b\\c\nsecond line");
        r.set("empty", "");
        r.save().unwrap();
        let back = load(&r.path).unwrap();
        assert_eq!(back.id(), "20260101-000000-p-run");
        assert_eq!(back.status(), "running");
        assert_eq!(back.get("command"), "echo a = b\\c\nsecond line");
        assert_eq!(back.get("empty"), "");
        assert_eq!(back.get("missing"), "");
        assert_eq!(std::fs::read_to_string(&r.path).unwrap().lines().count(), 4, "newline must be escaped: one line per field");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp() {
        let d = tmp("atomic");
        let mut r = create(&d, "j").unwrap();
        r.set("status", RUNNING);
        r.save().unwrap();
        r.set("status", DONE);
        r.save().unwrap();
        assert_eq!(load(&r.path).unwrap().status(), "done");
        assert!(!d.join("j.rec.tmp").exists(), "temp file must be renamed away");
        // a half-written temp left by a kill is never read as a record
        std::fs::write(d.join("k.rec.tmp"), "id = k\nstatus = run").unwrap();
        let all = load_all(&d);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id(), "j");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn ids_are_unique_and_safe() {
        let d = tmp("ids");
        let ids: Vec<String> = (0..4).map(|_| create(&d, "20260101-000000-my proj-run").unwrap().id().to_string()).collect();
        assert_eq!(
            ids,
            ["20260101-000000-my_proj-run", "20260101-000000-my_proj-run-2", "20260101-000000-my_proj-run-3", "20260101-000000-my_proj-run-4"]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn status_transitions_persist() {
        let d = tmp("trans");
        let mut r = create(&d, "t").unwrap();
        for st in [RUNNING, ROLLED_BACK] {
            r.set("status", st);
            r.save().unwrap();
            assert_eq!(load_all(&d)[0].status(), st);
        }
        r.set("resumed_as", "t-2");
        r.set("status", RESUMED);
        r.save().unwrap();
        let b = &load_all(&d)[0];
        assert_eq!((b.status(), b.get("resumed_as")), ("resumed", "t-2"));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
