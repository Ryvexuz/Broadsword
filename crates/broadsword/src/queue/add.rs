use crate::config::Config;
use crate::job;
use crate::record;
use crate::reviewer;
use crate::task::{self, fail, now_secs};
use crate::worktree::{self, JobDirError};
use crate::wtgit;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use super::entry::{EXT, is_open, load_entries, process_secrets, queue_dir};
use super::locks::entries_lock;
use super::{EXIT_BAD_INPUT, EXIT_ERROR, EXIT_RETRY, QUEUED};
use super::rebase::{chained_base, left_train_text};
use super::train::{gating_model, trunk_name};
use super::view::{short, some};

// ---- add --------------------------------------------------------------------------------

/// A folder of the config (`state_dir`, `build_dir`), resolved the way `canon_root` was: through the canonical root, and fully
/// canonical when it exists (a junction inside it then resolves too).
fn canon_inside(cfg: &Config, canon_root: &Path, dir: &Path) -> PathBuf {
    let joined = canon_root.join(dir.strip_prefix(&cfg.root).unwrap_or(dir));
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// A brief file must be a small UTF-8 text file INSIDE the root (after canonicalising, so `..` and links count), not inside
/// `state_dir` (queue records, job logs) or `build_dir`, and not a secret file (`git::is_secret_path`, checked on the name given and
/// on the resolved name).
/// A brief is stored, shown and sent to a model, so it may never be any file the caller can read.
fn read_brief(cfg: &Config, path: &str) -> Result<String, String> {
    if crate::git::is_secret_path(path) {
        return Err(format!("`{path}` is a secret file (`.env`, `.envrc`, `.env.<name>`, `.env-*`, `.env_*` or a short name or stream of one); a brief is sent to a model and secrets must never be"));
    }
    let real = std::fs::canonicalize(path).map_err(|e| format!("cannot read the brief file `{path}`: {e}"))?;
    let canon_root = std::fs::canonicalize(&cfg.root).map_err(|e| format!("cannot resolve the project root: {e}"))?;
    if !real.starts_with(&canon_root) {
        return Err(format!("the brief `{path}` is outside the project root; a brief must be a file inside the root (it is sent to a model)"));
    }
    for (name, dir) in [("state_dir", &cfg.state_dir), ("build_dir", &cfg.build_dir)] {
        if real.starts_with(canon_inside(cfg, &canon_root, dir)) {
            return Err(format!("the brief `{path}` is inside {name}; queue records, job logs and build output are never briefs (they are sent to a model)"));
        }
    }
    if crate::git::is_secret_path(&real.to_string_lossy()) {
        return Err(format!("`{path}` resolves to a secret file ({}); a brief is sent to a model and secrets must never be", real.display()));
    }
    let p = real.as_path();
    let meta = std::fs::metadata(p).map_err(|e| format!("cannot read the brief file `{path}`: {e}"))?;
    let max = cfg.reviewer_brief_max_kb * 1024;
    if !meta.is_file() || meta.len() > max {
        return Err(format!("the brief `{path}` must be a file of at most {} KB (reviewer_brief_max_kb); it is {} bytes", cfg.reviewer_brief_max_kb, meta.len()));
    }
    let text = std::fs::read_to_string(p).map_err(|e| format!("the brief file `{path}` must be UTF-8 text: {e}"))?;
    if text.trim().is_empty() {
        return Err(format!("the brief file `{path}` is empty"));
    }
    Ok(text)
}

pub fn cmd_add(cfg: &Config, task_id: &str, risk: &str, coder: &str, brief_file: Option<&str>) -> ExitCode {
    if risk != "low" && risk != "high" {
        return fail(EXIT_BAD_INPUT, format!("--risk must be `low` or `high`, not `{risk}`"));
    }
    let coder = coder.trim();
    if coder.is_empty() || coder.chars().any(char::is_control) {
        return fail(EXIT_BAD_INPUT, "--coder must be the coder's model name on one line, e.g. sonnet");
    }
    // A coder whose family is the low-risk reviewer's has no allowed reviewer at low risk. Derived from `gating_model`, the one place
    // that picks reviewers, and checked before any lock is taken or any entry is written.
    if risk == "low" && gating_model(cfg, risk, coder).is_err() {
        return fail(EXIT_BAD_INPUT, format!("a {} coder has no allowed reviewer at --risk low; queue it with --risk high", reviewer::model_family(coder)));
    }
    let brief_text = match brief_file.map(|f| read_brief(cfg, f)) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => return fail(EXIT_BAD_INPUT, e),
    };
    let t = match task::load_task(cfg, task_id) {
        Ok(t) => t,
        Err(e) => return fail(EXIT_BAD_INPUT, e),
    };
    let refuse = |why: String| fail(EXIT_RETRY, format!("cannot queue task {task_id}: {why}; nothing was queued"));
    let wt = match worktree::job_dir(cfg, task_id, t.get("project")) {
        Ok(p) => p,
        Err(JobDirError::Unknown(m) | JobDirError::Refused(m)) => return refuse(m),
    };
    let dir = match worktree::project_dir(cfg, &t) {
        Ok(d) => d,
        Err(e) => return refuse(e),
    };
    let branch = worktree::branch_of(&t);
    match wtgit::current_branch(&wt) {
        Ok(Some(b)) if b == branch => {}
        Ok(other) => return refuse(format!("its worktree is on {} instead of `{branch}`", other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into()))),
        Err(e) => return refuse(e),
    }
    // Tracked and untracked changes only: a rebase leaves ignored files (an ignored .env, target/) alone. The strict rule, ignored
    // files included, guards worktree REMOVAL at `task done`, which keeps the worktree instead of failing.
    match wtgit::status_porcelain_plain(&wt) {
        Ok(s) if s.is_empty() => {}
        Ok(s) => return refuse(format!("its worktree is not clean (uncommitted or untracked changes; ignored files do not count here): {}", some(&s))),
        Err(e) => return refuse(e),
    }
    let base = t.get("base").to_string();
    if base.is_empty() {
        return refuse("the task record has no base commit".into());
    }
    match wtgit::rev_count(&dir, &format!("{base}..{branch}")) {
        Ok(0) => return refuse(format!("`{branch}` has no commit beyond its base {}; commit the work first", short(&base))),
        Ok(_) => {}
        Err(e) => return refuse(e),
    }
    let brief = process_secrets(cfg).redact(&brief_text.unwrap_or_else(|| {
        format!(
            "Task {}: {}\nProject: {}. The task claims these files and folders: {}.\nNo separate done check was given: judge whether the diff does what the task title says, and flag changes outside the claims.",
            t.id(),
            t.get("title"),
            t.get("project"),
            task::claims_of(&t).join(", ")
        )
    }));
    let _lock = match entries_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let all = match load_entries(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, format!("cannot read the queue, so nothing was queued: {e}")),
    };
    if let Some(x) = all.iter().find(|x| x.get("task") == task_id && is_open(x.status())) {
        return fail(EXIT_RETRY, format!("task {task_id} is already in the queue as {} ({}); a task has one open entry at a time (queued, running, blocked, stale and approved entries are open). Run it with `queue run --entry {}`, or cancel it with `queue cancel {}`, then add again", x.id(), x.status(), x.id(), x.id()));
    }
    // LEFT-TRAIN RULE (see `Run::leave_train`): an earlier entry of this task left a merge train with its branch chained on other
    // entries' commits; while the branch still carries them and main does not, a new entry would review their code as this task's
    for x in all.iter().filter(|x| x.get("task") == task_id) {
        match chained_base(&dir, &branch, x) {
            Ok(None) => {}
            Ok(Some(chained_on)) => {
                let main = trunk_name(&dir);
                return refuse(format!("its earlier queue entry {} {}", x.id(), left_train_text(&main, &branch, &chained_on)));
            }
            Err(e) => return refuse(e),
        }
    }
    let mut e = match record::create_ext(&queue_dir(cfg), &format!("q-{}", job::file_stamp(now_secs())), EXT) {
        Ok(r) => r,
        Err(err) => return fail(EXIT_ERROR, format!("cannot create a queue entry: {err}")),
    };
    e.set("task", task_id);
    e.set("project", t.get("project"));
    e.set("branch", branch);
    e.set("risk", risk);
    e.set("coder", coder);
    e.set("brief", brief);
    e.set("brief_source", if brief_file.is_some() { "file" } else { "task title and claims" });
    e.set("status", QUEUED);
    e.set("attempt", "0");
    e.set("created", now_secs().to_string());
    if let Err(err) = e.save() {
        return fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display()));
    }
    println!("{}", e.id());
    println!("queued task {task_id} ({risk} risk, coder {coder}); `broadsword queue run` will review it with `{}` and merge it if it passes", gating_model(cfg, risk, coder).map_or_else(|_| "no allowed model (the entry will be blocked)".to_string(), |g| if g.why.is_empty() { g.model } else { format!("{} ({})", g.model, g.why) }));
    ExitCode::SUCCESS
}
