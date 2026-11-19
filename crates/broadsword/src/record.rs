//! Durable job records: `<state_dir>/jobs/<job id>.rec`, plain `key = value` lines.
//!
//! A record is written when a job starts and rewritten at every step and at the end. Each write goes
//! to `<id>.rec.tmp` and is renamed over the record, so a kill can never leave a half-written record.
//! Records are never deleted by cleanup (they are the evidence recovery works from).
//! Values are one line: a backslash is written `\\`, a newline `\n` and a carriage return `\r` (none is ever dropped).
//! A value that holds a LIST (file names) is made with `encode_list` and read with `decode_list`, so a name holding `|`,
//! a backslash or a line break comes back whole.

use crate::config::Config;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One record value from a list: the items joined by `|`, with a `\` or `|` inside an item written `\\` or `\|`.
/// (Line breaks need nothing here: the record escapes them when it is saved.)
pub fn encode_list<S: AsRef<str>>(items: &[S]) -> String {
    items.iter().map(|i| i.as_ref().replace('\\', "\\\\").replace('|', "\\|")).collect::<Vec<_>>().join("|")
}

/// The items of an `encode_list` value: split only on a `|` that is not escaped, then `\\` -> `\` and `\|` -> `|`.
/// A record written before lists were escaped still reads the same (its names held no `|`), and any other backslash is
/// kept as it is. An empty value is an empty list.
pub fn decode_list(v: &str) -> Vec<String> {
    if v.is_empty() {
        return Vec::new();
    }
    let mut items = vec![String::new()];
    let mut it = v.chars();
    while let Some(c) = it.next() {
        let cur = items.last_mut().expect("never empty");
        match c {
            '\\' => match it.next() {
                Some(e @ ('\\' | '|')) => cur.push(e),
                Some(o) => {
                    cur.push('\\');
                    cur.push(o);
                }
                None => cur.push('\\'),
            },
            '|' => items.push(String::new()),
            o => cur.push(o),
        }
    }
    items
}

pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
pub const REFUSED: &str = "refused";
pub const KILLED_LOW_DISK: &str = "killed_low_disk";
pub const RESUMED: &str = "resumed";
pub const ROLLED_BACK: &str = "rolled_back";
pub const ABANDONED: &str = "abandoned";

#[derive(Debug, Clone)]
pub struct Record {
    pub path: PathBuf,
    fields: BTreeMap<String, String>,
}

fn esc(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\r', "\\r").replace('\n', "\\n")
}

fn unesc(v: &str) -> String {
    let mut out = String::new();
    let mut it = v.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
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
        let ext = self.path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = self.path.with_extension(format!("{ext}.tmp"));
        {
            let mut f = std::fs::File::create(&tmp)?;
            std::io::Write::write_all(&mut f, self.render().as_bytes())?;
            f.sync_all()?; // on disk before the rename makes it the record
        }
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
    create_ext(dir, base, "rec")
}

/// `create` for another record kind: the file is `<id>.<ext>` (the task board uses `task`).
pub fn create_ext(dir: &Path, base: &str, ext: &str) -> std::io::Result<Record> {
    std::fs::create_dir_all(dir)?;
    let base = sanitize(base);
    for n in 1u32.. {
        let id = if n == 1 { base.clone() } else { format!("{base}-{n}") };
        let path = dir.join(format!("{id}.{ext}"));
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
    load_all_ext(dir, "rec")
}

/// `load_all` for records with the extension `ext`.
pub fn load_all_ext(dir: &Path, ext: &str) -> Vec<Record> {
    let mut v: Vec<Record> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == ext))
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
        let d = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-rec-{tag}-{}", std::process::id()));
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

    /// A list survives names holding `|`, backslashes, a newline and a carriage return, through a saved and loaded record.
    #[test]
    fn lists_round_trip_through_a_record() {
        let d = tmp("list");
        let names = ["plain.txt", ".env.a|b", "a\\b\\", "\\|", "x\ny", "cr\r\nlf", "|", "trailing\\"];
        let mut r = create(&d, "l").unwrap();
        r.set("names", encode_list(&names));
        r.set("raw", "a\r\nb");
        r.save().unwrap();
        let text = std::fs::read_to_string(&r.path).unwrap();
        assert_eq!(text.lines().count(), 3, "one line per field: {text:?}");
        assert!(!text.contains('\r'), "a carriage return is escaped, not written raw");
        let back = load(&r.path).unwrap();
        assert_eq!(decode_list(back.get("names")), names);
        assert_eq!(back.get("raw"), "a\r\nb", "a carriage return is kept, never dropped");
        assert_eq!(decode_list(""), Vec::<String>::new());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Records written before lists were escaped still read the same.
    #[test]
    fn old_unescaped_lists_still_parse() {
        let back = Record::parse(Path::new("old.rec"), "id = old\nsecret_skipped = .env|.env.local|sub/.env\nignored_skipped_len = 600|10\nwin = a\\\\b|c\n");
        assert_eq!(decode_list(back.get("secret_skipped")), [".env", ".env.local", "sub/.env"]);
        assert_eq!(decode_list(back.get("ignored_skipped_len")), ["600", "10"]);
        assert_eq!(decode_list(back.get("win")), ["a\\b", "c"], "an old lone backslash is kept");
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
