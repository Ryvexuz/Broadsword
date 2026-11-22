use crate::config::Config;
use crate::record::{self, Record};
use crate::reviewer;
use crate::task::{fail, now_secs};
use crate::pause;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;
use super::{APPROVED, BLOCKED, EXIT_ERROR, EXIT_RETRY, MERGED, MERGED_CHECK_FAILED, QUEUED, R, Run, RUNNING, STALE};

pub(super) const EXT: &str = "entry";

// ---- storage and locks ------------------------------------------------------------------

pub fn queue_dir(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("queue")
}

fn entry_path(cfg: &Config, id: &str) -> PathBuf {
    queue_dir(cfg).join(format!("{id}.{EXT}"))
}

/// Every entry, oldest first. An unreadable entry is an error (never silently skipped: `run` decides on this list); the empty file
/// that exists for an instant while `add` creates an entry is skipped.
pub(super) fn load_entries(cfg: &Config) -> Result<Vec<Record>, String> {
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(queue_dir(cfg)) else { return Ok(v) };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != EXT) {
            continue;
        }
        let mut last = String::new();
        let mut rec = None;
        for _ in 0..5 {
            match record::load(&p) {
                Ok(r) => {
                    rec = Some(r);
                    break;
                }
                Err(e) => {
                    last = e;
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        match rec {
            Some(r) if !r.id().is_empty() => v.push(r),
            Some(_) => {}
            None => return Err(last),
        }
    }
    v.sort_by(|a, b| (a.get_num("created"), a.id().len(), a.id()).cmp(&(b.get_num("created"), b.id().len(), b.id())));
    Ok(v)
}

pub(super) fn load_entry(cfg: &Config, id: &str) -> Result<Record, String> {
    let nope = || format!("no queue entry with the exact id `{id}` (see `broadsword queue list`)");
    if id.is_empty() || record::sanitize(id) != id {
        return Err(nope());
    }
    match record::load(&entry_path(cfg, id)) {
        Ok(r) if r.id() == id => Ok(r),
        _ => Err(nope()),
    }
}

/// An entry is "open" while `queue run` can still act on it (queued, running, blocked, stale, approved): it blocks `queue add` for its task.
/// Finished entries do not: merged, cancelled, rejected, and also check_failed, conflict and merged_check_failed, which `queue run`
/// refuses to run again (the documented way on is a new `queue add`, or a human for merged_check_failed).
/// HALF-FINISHED MERGE, the ONE test: an `approved` entry that a merge train already took into its merge (`merge_members` records
/// step `merge` on every member BEFORE the fast-forward, `main_sha` after it, then `main_check` and `done`). Its commits may be on
/// main already, so it is never chained into a new train (`claim_approved` skips it), never cancelled (`cancel_refusal`), and it is
/// resumed FIRST, at its recorded step, exactly as an interrupted `running` entry is (`resume_first`): `s_merge` takes "the reviewed
/// commit is already on main" as merged, and `s_main_check` checks the merged main.
pub(super) fn half_merged(e: &Record) -> bool {
    e.status() == APPROVED && (matches!(e.get("step"), "merge" | "main_check" | "done") || !e.get("main_sha").is_empty())
}

/// An entry a run takes before any other: an interrupted `running` one, or a half-finished train merge (`half_merged`).
pub(super) fn resume_first(e: &Record) -> bool {
    e.status() == RUNNING || half_merged(e)
}

pub(super) fn is_open(status: &str) -> bool {
    matches!(status, QUEUED | RUNNING | BLOCKED | STALE | APPROVED)
}

/// The entry fields that hold free text (output of the check, the reviewer, git): redacted before they are written.
const TEXT_KEYS: [&str; 12] = ["brief", "check_summary", "check_tail", "done_check", "reason", "error", "log", "blocking", "minor", "shadow_blocking", "shadow_reason", "perf"];

pub(super) fn process_secrets(cfg: &Config) -> reviewer::Secrets {
    let env: Vec<(String, String)> = std::env::vars().collect();
    reviewer::Secrets::collect(&cfg.root, &env, cfg.redact_min_chars as usize)
}

impl<'a> Run<'a> {
    pub(super) fn new(cfg: &'a Config, e: Record, dir: PathBuf) -> Run<'a> {
        Run { cfg, e, dir, wt: PathBuf::new(), secrets: process_secrets(cfg), defer_check: false, claims: Vec::new() }
    }

    pub(super) fn branch(&self) -> String {
        self.e.get("branch").to_string()
    }

    /// Write the entry. A queue that cannot write its own record must not go on: exit 1, loudly.
    pub(super) fn save(&mut self) -> R<()> {
        for k in TEXT_KEYS {
            let v = self.e.get(k);
            if !v.is_empty() {
                let r = self.secrets.redact(v);
                self.e.set(k, r);
            }
        }
        self.e.save().map_err(|err| fail(EXIT_ERROR, format!("queue entry {}: cannot write {}: {err}; stopping. Its last saved step is `{}`; `queue run --entry {}` resumes it", self.e.id(), self.e.path.display(), self.e.get("step"), self.e.id())))
    }

    pub(super) fn log(&mut self, step: &str, text: &str) -> R<()> {
        let line = format!("{} {step}: {}", pause::now_string(), text.replace('\n', " "));
        let log = if self.e.get("log").is_empty() { line } else { format!("{}\n{line}", self.e.get("log")) };
        self.e.set("log", log);
        self.save()
    }

    pub(super) fn set_step(&mut self, step: &str) -> R<()> {
        self.e.set("step", step);
        println!("queue entry {}: step {step}", self.e.id());
        self.log(step, "begin")
    }

    /// The entry is finished with `status`; print the one-line outcome and return the exit code.
    pub(super) fn end(&mut self, status: &str, reason: &str, code: u8) -> ExitCode {
        let reason = &self.secrets.redact(reason);
        self.e.set("status", status);
        self.e.set("reason", reason);
        self.e.set("ended", now_secs().to_string());
        let step = self.e.get("step").to_string();
        let _ = self.log(&step, &format!("{status}: {reason}"));
        for k in TEXT_KEYS {
            let r = self.secrets.redact(self.e.get(k));
            self.e.set(k, r);
        }
        let saved = self.e.save();
        if status == MERGED {
            println!("queue entry {}: merged: {reason}", self.e.id());
        } else if status == MERGED_CHECK_FAILED {
            eprintln!("broadsword: LOUD: queue entry {} is {status}: {reason}", self.e.id());
        } else {
            eprintln!("broadsword: queue entry {} is {status}: {reason}", self.e.id());
        }
        match saved {
            Ok(()) => ExitCode::from(code),
            Err(err) => {
                eprintln!("broadsword: ERROR: queue entry {}: cannot record the final status `{status}`: {err}; its real outcome is `{status}`", self.e.id());
                ExitCode::from(EXIT_ERROR)
            }
        }
    }

    pub(super) fn blocked(&mut self, reason: &str) -> ExitCode {
        self.end(BLOCKED, reason, EXIT_RETRY)
    }

    /// Something unexpected: the entry stays `running` at its step, so a later `queue run` resumes it.
    pub(super) fn internal(&mut self, msg: &str) -> ExitCode {
        let msg = &self.secrets.redact(msg);
        self.e.set("error", msg);
        let _ = self.e.save();
        eprintln!(
            "broadsword: queue entry {} stopped with an error at step `{}`: {msg}\nthe entry is still `running`; `broadsword queue run --entry {}` resumes it from that step, `broadsword queue cancel {}` drops it",
            self.e.id(),
            self.e.get("step"),
            self.e.id(),
            self.e.id()
        );
        ExitCode::from(EXIT_ERROR)
    }

    /// A git (or other) result: an `Err` is an internal error that ends the run.
    pub(super) fn g<T>(&mut self, r: Result<T, String>) -> R<T> {
        r.map_err(|m| self.internal(&m))
    }
}
