//! One Git worktree per task: `broadsword worktree create|remove|prune|list|repair`, plus the pieces `task start
//! --worktree` and `task done` call.
//!
//! A task's worktree is `<root>/<worktrees_dir>/<canonical project>/<task id>` on the branch `task/<task id>`, in the
//! PROJECT's own Git repo (Broadsword's for `broadsword`, `projects/<p>` for the others). The task record keeps
//! `worktree` (the path RELATIVE to root, so the record survives moving the project), `branch`, `base` (the commit the
//! branch started from) and, once removed, `worktree_removed`.
//!
//! Creation is three steps so the board lock is never held through the slow `git worktree add`:
//! 1. under the lock: check, mark the task running with `worktree_creating = <token>` (claims are held from here, so no
//!    conflicting task can start), record `branch` and `base`; release the lock;
//! 2. outside the lock: `git worktree add`;
//! 3. under the lock again: record `worktree` and clear `worktree_creating`, or (start) roll the task back to `open`.
//!
//! While `worktree_creating` is set the task has no `worktree` yet, so `done` refuses, `remove`/`prune` skip it. If the
//! creator crashed between steps the flag stays: `worktree create <id>` finishes the job (it adopts a complete, clean
//! worktree at the recorded base, or starts afresh if nothing was made) and `task fail|abandon <id>` clears it.
//!
//! Removal safety, one function (`remove_for`) used by `task done`, `worktree remove` and `worktree prune`:
//! - the worktree must contain NOTHING that git does not already hold safely: `git status --porcelain --ignored` empty,
//!   so no modified, staged or untracked files AND no ignored files (`.env`, notes, ...: Broadsword cannot know they are
//!   disposable, so it never deletes them and there is no override);
//! - it must still be on its task branch, and every commit in the worktree's own HEAD reflog must be reachable from
//!   some ref (a commit made on a detached HEAD and left behind would lose its last reference);
//! - the branch must be MERGED: its tip is an ancestor of the main checkout's HEAD (`--keep-branch` skips this one rule,
//!   because the branch, which holds the commits, is kept);
//! - then `git worktree remove` (no `--force`), then `git branch -d` (no `-D`, so Git refuses an unmerged branch too, or
//!   one checked out elsewhere: it is then kept), then empty parent folders under worktrees_dir are removed with
//!   `remove_dir` only, and never through a symlink, junction or reparse point.
//!
//! Nothing here deletes files recursively and nothing forces anything: Broadsword can only refuse. All git calls are in
//! wtgit.rs.

use crate::config::Config;
use crate::record::Record;
use crate::task::{self, DONE};
use crate::{git, job, wtgit};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// `worktree remove` could not remove it safely; nothing was changed (the reasons are printed).
pub const EXIT_KEPT: u8 = 3;

/// Test-only hook, same rules as `task::TEST_GATE_ENV`: `git worktree add` waits until this file (inside scratch) exists,
/// to fake a slow creation.
pub const TEST_ADD_GATE_ENV: &str = "BROADSWORD_INTERNAL_TEST_WORKTREE_ADD_GATE_FILE";

// ---- paths ------------------------------------------------------------------------------

/// The task's worktree path relative to root, with `/` separators (that is what the record stores).
fn rel_path(cfg: &Config, project: &str, id: &str) -> String {
    let base: Vec<String> = cfg.worktrees_dir.strip_prefix(&cfg.root).unwrap_or(&cfg.worktrees_dir).components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    format!("{}/{project}/{id}", base.join("/"))
}

/// The record's relative path as a real path, refused unless it is inside the configured worktrees_dir (a hand-edited
/// record must never point removal at some other folder).
fn abs_path(cfg: &Config, rel: &str) -> Result<PathBuf, String> {
    let p = crate::config::normalize(&cfg.root.join(rel));
    if p.starts_with(&cfg.worktrees_dir) && p != cfg.worktrees_dir && !rel.split(['/', '\\']).any(|s| s == "..") {
        Ok(p)
    } else {
        Err(format!("the recorded worktree path `{rel}` is not inside the worktrees_dir setting ({}); it was not touched", cfg.worktrees_dir.display()))
    }
}

fn project_dir(cfg: &Config, t: &Record) -> Result<PathBuf, String> {
    job::resolve_project(&cfg.root, &cfg.projects_dir, t.get("project"))
}

#[cfg(windows)]
fn is_reparse_point(m: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    m.file_attributes() & 0x400 != 0 // FILE_ATTRIBUTE_REPARSE_POINT: symlinks and junctions
}
#[cfg(not(windows))]
fn is_reparse_point(_: &std::fs::Metadata) -> bool {
    false
}

/// A real directory: not a symlink, junction or other reparse point (those may lead somewhere that is not ours).
fn is_real_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink() && !is_reparse_point(&m))
}

/// Remove `start` (if empty) and every empty parent up to, but never including, worktrees_dir. `remove_dir` only, and
/// only real directories.
fn remove_empty_parents(cfg: &Config, start: &Path) {
    let mut p = Some(start);
    while let Some(d) = p {
        if d == cfg.worktrees_dir || !d.starts_with(&cfg.worktrees_dir) || !is_real_dir(d) || std::fs::remove_dir(d).is_err() {
            break;
        }
        p = d.parent();
    }
}

pub fn has_worktree(t: &Record) -> bool {
    !t.get("worktree").is_empty() && t.get("worktree_removed").is_empty()
}

pub fn is_creating(t: &Record) -> bool {
    !t.get("worktree_creating").is_empty()
}

fn branch_of(t: &Record) -> String {
    if t.get("branch").is_empty() {
        format!("task/{}", t.id())
    } else {
        t.get("branch").to_string()
    }
}

// ---- creation ---------------------------------------------------------------------------

pub struct Plan {
    pub token: String,
    pub dir: PathBuf,
    pub branch: String,
    pub rel: String,
    pub path: PathBuf,
    pub base: String,
}

/// Step 1a, no changes made: every check that can refuse, and the fresh base commit.
pub fn plan(cfg: &Config, t: &Record) -> Result<Plan, String> {
    let (id, project) = (t.id().to_string(), t.get("project").to_string());
    let dir = project_dir(cfg, t)?;
    if !git::is_repo(&dir) {
        return Err(format!("project `{project}` is not a Git repo (no .git in {}); a worktree needs one. Run `git init` there and commit first", dir.display()));
    }
    let branch = format!("task/{id}");
    let rel = rel_path(cfg, &project, &id);
    let path = abs_path(cfg, &rel)?;
    if wtgit::branch_exists(&dir, &branch)? {
        return Err(format!("the branch `{branch}` already exists in project `{project}`; it was not touched (merge or delete it yourself if it is leftover work)"));
    }
    if path.exists() {
        return Err(format!("{} already exists; it was not touched", path.display()));
    }
    if wtgit::current_branch(&dir)?.is_none() {
        return Err(format!("the main checkout of `{project}` ({}) is on a detached HEAD; check out a branch first, so the task's base is a branch tip", dir.display()));
    }
    let base = wtgit::head_commit(&dir)?;
    let token = format!("{}-{}", std::process::id(), task::now_secs());
    Ok(Plan { token, dir, branch, rel, path, base })
}

/// Step 1b: the fields that say "a worktree is being made" (the caller saves the record under the lock).
pub fn mark_creating(t: &mut Record, p: &Plan) {
    t.set("worktree_creating", p.token.clone());
    t.set("branch", p.branch.clone());
    t.set("base", p.base.clone());
}

/// Step 2, outside the lock: make the worktree. On failure nothing is left that this call made.
fn add(cfg: &Config, p: &Plan) -> Result<(), String> {
    task::wait_for_gate(cfg, TEST_ADD_GATE_ENV, "worktree add");
    let parent = p.path.parent().ok_or("bad worktree path")?;
    std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    if let Err(e) = wtgit::worktree_add(&p.dir, &p.branch, &p.path, &p.base) {
        // Git cleans up after itself; also drop what this call may have left, but only if it is empty and ours.
        if is_real_dir(&p.path) {
            let _ = std::fs::remove_dir(&p.path); // fails (and keeps it) unless empty
        }
        remove_empty_parents(cfg, parent);
        if !e.contains("already exists") && wtgit::branch_tip(&p.dir, &p.branch).is_ok_and(|tip| tip == p.base) {
            let _ = wtgit::branch_delete_if_merged(&p.dir, &p.branch); // an empty branch at base made by the failed add
        }
        return Err(e);
    }
    Ok(())
}

/// Best-effort undo of a worktree whose record could not be saved: it is clean and its branch is at the base, so the same
/// safe commands remove it. Returns what is left, if anything.
fn undo(cfg: &Config, p: &Plan) -> Option<String> {
    if let Err(e) = wtgit::worktree_remove(&p.dir, &p.path) {
        return Some(format!("{e}; remove {} by hand", p.path.display()));
    }
    let _ = wtgit::branch_delete_if_merged(&p.dir, &p.branch);
    remove_empty_parents(cfg, p.path.parent()?);
    None
}

pub fn describe(p: &Plan) -> String {
    format!("worktree: {}\nbranch: {}\nbase: {}", p.path.display(), p.branch, p.base)
}

pub enum Mode<'a> {
    /// `task start --worktree`: on failure the task goes back to `open`; on success print the running line.
    Start { owner: &'a str },
    /// `worktree create`: the task stays running either way.
    Create,
}

/// The task was marked running but its worktree could not be made: back to `open`, owner and start time cleared, so a task
/// is never left running without the worktree that was asked for.
fn roll_back_start(t: &mut Record, why: &str) -> ExitCode {
    let id = t.id().to_string();
    for k in ["worktree", "branch", "base", "worktree_removed", "worktree_creating"] {
        t.set(k, "");
    }
    t.set("status", task::OPEN);
    t.set("owner", "");
    t.set("started", "");
    match task::save(t) {
        Ok(()) => task::fail(task::EXIT_ERROR, format!("cannot start {id} with a worktree: {why}\ntask {id} was rolled back to `open` (claims not held, nothing half-made); fix the cause and start it again")),
        Err(e) => task::fail(task::EXIT_ERROR, format!("cannot start {id} with a worktree: {why}\nAND the rollback failed: {e}. Task {id} may still say `running` with `worktree_creating` set: run `broadsword task abandon {id}` or fix its record")),
    }
}

/// Steps 2 and 3, called with the board lock NOT held.
pub fn finish(cfg: &Config, id: &str, p: Plan, mode: Mode) -> ExitCode {
    let added = add(cfg, &p);
    let _lock = match task::board_lock(cfg) {
        Ok(l) => l,
        Err(c) => {
            eprintln!(
                "broadsword: task {id}: the worktree step {} but the board was busy, so it could not be recorded. The task stays running with `worktree_creating` set; run `broadsword worktree create {id}` to finish recording it",
                if added.is_ok() { "finished" } else { "failed" }
            );
            return c;
        }
    };
    let mut t = match task::load_task(cfg, id) {
        Ok(t) => t,
        Err(e) => return task::fail(task::EXIT_ERROR, e),
    };
    let mine = t.get("worktree_creating") == p.token;
    match added {
        Ok(()) if !mine => {
            // The task was failed or abandoned while its worktree was being made. Nothing is deleted: record the worktree so
            // that `worktree prune|remove` manage it by the usual safe rules.
            if t.get("worktree").is_empty() {
                t.set("worktree", p.rel.clone());
                t.set("branch", p.branch.clone());
                t.set("base", p.base.clone());
                let _ = task::save(&t);
            }
            task::fail(task::EXIT_ERROR, format!("task {id} is `{}`: it changed while its worktree was being made. The worktree {} was kept and recorded; `broadsword worktree prune` or `worktree remove {id}` will remove it when it is clean and merged", t.status(), p.path.display()))
        }
        Ok(()) => {
            t.set("worktree", p.rel.clone());
            t.set("worktree_creating", "");
            t.set("worktree_removed", "");
            match task::save(&t) {
                Ok(()) => {
                    if let Mode::Start { owner } = mode {
                        println!("task {id} is running (owner {owner}); its claims are held until it is done, failed or abandoned");
                    }
                    println!("{}", describe(&p));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    let left = undo(cfg, &p).map(|l| format!(" The new worktree could not be removed again: {l}")).unwrap_or_default();
                    match mode {
                        Mode::Start { .. } => roll_back_start(&mut t, &format!("{e}.{left}")),
                        Mode::Create => task::fail(task::EXIT_ERROR, format!("{e}; the new worktree was removed again.{left}")),
                    }
                }
            }
        }
        Err(e) if mine => match mode {
            Mode::Start { .. } => roll_back_start(&mut t, &e),
            Mode::Create => {
                t.set("worktree_creating", "");
                for k in ["branch", "base"] {
                    t.set(k, "");
                }
                let saved = task::save(&t);
                task::fail(task::EXIT_ERROR, format!("no worktree was made for {id}: {e}{}", if saved.is_err() { " (and its record could not be reset: run `worktree create` again)" } else { "" }))
            }
        },
        Err(e) => task::fail(task::EXIT_ERROR, format!("no worktree was made for {id}: {e}")),
    }
}

/// `task fail|abandon` of a task whose creation was in progress: clear the flag; if a worktree folder exists, record it so
/// `worktree prune|remove` manage it by the safe rules. Nothing is deleted.
pub fn settle_creating(cfg: &Config, t: &mut Record) {
    let rel = rel_path(cfg, t.get("project"), t.id());
    if abs_path(cfg, &rel).is_ok_and(|p| p.exists()) {
        t.set("worktree", rel);
    }
    t.set("worktree_creating", "");
}

// ---- removal ----------------------------------------------------------------------------

pub enum BranchFate {
    Deleted,
    Kept(String),
    AlreadyGone,
}

pub enum Removal {
    Removed { branch: BranchFate },
    Kept { reasons: Vec<String>, advice: Vec<String> },
}

fn list_some(lines: &[&str]) -> String {
    let shown = lines.iter().take(5).map(|l| l.trim_end()).collect::<Vec<_>>().join("; ");
    format!("{} path(s): {shown}{}", lines.len(), if lines.len() > 5 { "; ..." } else { "" })
}

/// The one safe removal. Does NOT touch the record: on `Removed` the caller records `worktree_removed` (briefly, under the
/// board lock, see `mark_removed`); this function runs without the lock. `Err` means git itself could not be asked.
pub fn remove_for(cfg: &Config, t: &Record, keep_branch: bool) -> Result<Removal, String> {
    let id = t.id().to_string();
    let dir = project_dir(cfg, t)?;
    if !git::is_repo(&dir) {
        return Err(format!("project `{}` is no longer a Git repo (no .git in {}); nothing was removed", t.get("project"), dir.display()));
    }
    let branch = branch_of(t);
    let path = abs_path(cfg, t.get("worktree"))?;
    let exists = path.exists();
    let branch_there = wtgit::branch_exists(&dir, &branch)?;
    let merged = branch_there && wtgit::is_merged_into_head(&dir, &branch)?;
    let into = wtgit::current_branch(&dir)?.unwrap_or_else(|| "HEAD".into());

    let (mut reasons, mut advice) = (Vec::new(), Vec::new());
    if exists {
        match wtgit::current_branch(&path) {
            Ok(Some(b)) if b == branch => {}
            Ok(Some(b)) => reasons.push(format!("the worktree is on branch `{b}`, not `{branch}`, so removing it could lose commits made there")),
            Ok(None) => reasons.push("the worktree is on a detached HEAD, so removing it could lose commits made there".to_string()),
            Err(e) => reasons.push(format!("the worktree cannot be inspected: {e}")),
        }
        match wtgit::status_porcelain(&path) {
            Ok(s) if s.is_empty() => {}
            Ok(s) => {
                let all: Vec<&str> = s.lines().collect();
                let ignored: Vec<&str> = all.iter().copied().filter(|l| l.starts_with("!!")).collect();
                let changed: Vec<&str> = all.iter().copied().filter(|l| !l.starts_with("!!")).collect();
                if !changed.is_empty() {
                    reasons.push(format!("it has uncommitted or untracked changes ({})", list_some(&changed)));
                    advice.push(format!("commit or discard the changes in {} yourself, then run `broadsword worktree remove {id}`", path.display()));
                }
                if !ignored.is_empty() {
                    reasons.push(format!("it contains ignored files, which Broadsword cannot know are disposable (they may be a .env or notes) and never deletes ({})", list_some(&ignored)));
                    advice.push(format!("move or delete those ignored files yourself if you do not need them (there is no override), then run `broadsword worktree remove {id}`"));
                }
            }
            Err(e) => reasons.push(format!("its state cannot be read: {e}")),
        }
        match wtgit::reflog_commits(&path) {
            Ok(commits) => {
                let orphans: Vec<&String> = commits.iter().filter(|c| !wtgit::on_some_ref(&dir, c).unwrap_or(false)).collect();
                if !orphans.is_empty() {
                    let shown: Vec<String> = orphans.iter().take(5).map(|c| c.chars().take(10).collect()).collect();
                    reasons.push(format!("{} commit(s) in the worktree's own history are reachable from no branch (their last reference is the worktree's HEAD log, which removal would delete): {}", orphans.len(), shown.join(", ")));
                    advice.push(format!("keep them with `git branch <name> <commit>` in {} (or discard them deliberately), then run `broadsword worktree remove {id}`", path.display()));
                }
            }
            Err(e) => reasons.push(format!("its HEAD log cannot be read: {e}")),
        }
        if !keep_branch {
            if !branch_there {
                reasons.push(format!("the branch `{branch}` no longer exists, so what the worktree holds cannot be checked as merged"));
            } else if !merged {
                reasons.push(format!("branch `{branch}` has commits that are not merged into `{into}`"));
                advice.push(format!("merge `{branch}` into `{into}` in {}, then run `broadsword worktree remove {id}`", dir.display()));
                advice.push(format!("or, to drop only the folder and keep every commit on the branch, run `broadsword worktree remove {id} --keep-branch` (needs a clean worktree)"));
            }
        }
        if !reasons.is_empty() {
            return Ok(Removal::Kept { reasons, advice });
        }
        if let Err(e) = wtgit::worktree_remove(&dir, &path) {
            let locked = e.to_lowercase().contains("locked");
            let why = if locked { format!("the worktree is locked: {e}") } else { format!("git refused to remove it: {e}") };
            let fix = if locked { format!("it was locked on purpose; if that is no longer intended, unlock it yourself (`git worktree unlock {}`) and retry. Nothing was deleted", path.display()) } else { "fix what git says, or remove it yourself; nothing was deleted".to_string() };
            return Ok(Removal::Kept { reasons: vec![why], advice: vec![fix] });
        }
    } else {
        // The folder is already gone (deleted by hand): only Git's record is left, and there is nothing on disk to lose.
        wtgit::worktree_prune(&dir)?;
    }
    if let Some(parent) = path.parent() {
        remove_empty_parents(cfg, parent);
    }
    let branch = if keep_branch {
        BranchFate::Kept("kept as requested (--keep-branch)".into())
    } else if !branch_there {
        BranchFate::AlreadyGone
    } else if !merged {
        BranchFate::Kept(format!("not merged into `{into}`, so it is kept with its commits"))
    } else {
        match wtgit::branch_delete_if_merged(&dir, &branch) {
            Ok(()) => BranchFate::Deleted,
            Err(e) => BranchFate::Kept(format!("git refused to delete it (its commits stay on it): {e}")),
        }
    };
    Ok(Removal::Removed { branch })
}

/// Record `worktree_removed` for a task, briefly, under the board lock, on a FRESH copy of the record. Only if the record
/// still names the same worktree and has not been marked yet.
fn mark_removed(cfg: &Config, id: &str, rel: &str) -> Result<(), ExitCode> {
    let _lock = task::board_lock(cfg)?;
    let mut t = task::load_task(cfg, id).map_err(|e| task::fail(task::EXIT_ERROR, e))?;
    if t.get("worktree") == rel && t.get("worktree_removed").is_empty() {
        t.set("worktree_removed", task::now_secs().to_string());
        task::save(&t).map_err(|e| task::fail(task::EXIT_ERROR, format!("{e} (the worktree itself was removed)")))?;
    }
    Ok(())
}

fn say_removed(t: &Record, branch: &BranchFate) -> String {
    let b = branch_of(t);
    let fate = match branch {
        BranchFate::Deleted => format!("branch {b} deleted (it was merged)"),
        BranchFate::AlreadyGone => format!("branch {b} was already gone"),
        BranchFate::Kept(why) => format!("branch {b} kept: {why}"),
    };
    format!("worktree removed: {}; {fate}", t.get("worktree"))
}

fn print_kept(reasons: &[String], advice: &[String], to_stderr: bool) {
    for r in reasons {
        if to_stderr {
            eprintln!("  - {r}");
        } else {
            println!("  - {r}");
        }
    }
    for a in advice {
        if to_stderr {
            eprintln!("  to fix: {a}");
        } else {
            println!("  to fix: {a}");
        }
    }
}

/// What `task done` does about the worktree after the task is marked done (called WITHOUT the board lock). Never an
/// error: the task is done either way. Returns the exit code to use (0, or the lock error if recording failed).
pub fn after_done(cfg: &Config, t: &Record) -> ExitCode {
    match remove_for(cfg, t, false) {
        Ok(Removal::Removed { branch }) => {
            if let Err(c) = mark_removed(cfg, t.id(), t.get("worktree")) {
                return c;
            }
            println!("{}", say_removed(t, &branch));
        }
        Ok(Removal::Kept { reasons, advice }) => {
            println!("worktree kept: {}", t.get("worktree"));
            eprintln!("broadsword: WARNING: task {} is {DONE}, but its worktree and branch were kept (nothing was lost):", t.id());
            print_kept(&reasons, &advice, true);
        }
        Err(e) => {
            println!("worktree kept: {}", t.get("worktree"));
            eprintln!("broadsword: WARNING: task {} is {DONE}, but its worktree could not be checked, so it was kept: {e}", t.id());
        }
    }
    ExitCode::SUCCESS
}

// ---- commands ---------------------------------------------------------------------------

pub fn cmd_create(cfg: &Config, id: &str) -> ExitCode {
    let plan = {
        let _lock = match task::board_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        let mut t = match task::load_task(cfg, id) {
            Ok(t) => t,
            Err(e) => return task::fail(task::EXIT_ERROR, e),
        };
        if t.status() != task::RUNNING {
            return task::fail(task::EXIT_ERROR, format!("task {id} is `{}`; a worktree is made for a running task (`task start {id} --owner <name> --worktree` does both)", t.status()));
        }
        if has_worktree(&t) {
            return task::fail(task::EXIT_ERROR, format!("task {id} already has a worktree: {}", t.get("worktree")));
        }
        if is_creating(&t) {
            // A creation was started and never recorded (crashed, or still running in another process).
            let Ok(dir) = project_dir(cfg, &t) else { return task::fail(task::EXIT_ERROR, "unknown project") };
            let rel = rel_path(cfg, t.get("project"), id);
            let path = match abs_path(cfg, &rel) {
                Ok(p) => p,
                Err(e) => return task::fail(task::EXIT_ERROR, e),
            };
            let branch = branch_of(&t);
            let (path_there, branch_there) = (path.exists(), wtgit::branch_exists(&dir, &branch).unwrap_or(true));
            if path_there && branch_there {
                let complete = wtgit::branch_tip(&dir, &branch).is_ok_and(|tip| tip == t.get("base"))
                    && wtgit::current_branch(&path).ok().flatten().as_deref() == Some(branch.as_str())
                    && wtgit::status_porcelain(&path).is_ok_and(|s| s.is_empty());
                if !complete {
                    return task::fail(task::EXIT_ERROR, format!("task {id} has an unfinished worktree creation and {} exists but is not a clean worktree at the recorded base; if no other broadsword is still creating it, inspect it, or `task abandon {id}`", path.display()));
                }
                t.set("worktree", rel);
                t.set("worktree_creating", "");
                if let Err(e) = task::save(&t) {
                    return task::fail(task::EXIT_ERROR, e);
                }
                println!("adopted the complete worktree left by an interrupted creation\nworktree: {}\nbranch: {branch}\nbase: {}", path.display(), t.get("base"));
                return ExitCode::SUCCESS;
            }
            if path_there || branch_there {
                return task::fail(task::EXIT_ERROR, format!("task {id} has an unfinished worktree creation and only part of it exists (folder: {path_there}, branch: {branch_there}); if no other broadsword is still creating it, inspect it, or `task abandon {id}`"));
            }
            // nothing was made: fall through and start afresh
        }
        let p = match plan(cfg, &t) {
            Ok(p) => p,
            Err(e) => return task::fail(task::EXIT_ERROR, format!("no worktree was made for {id}: {e}")),
        };
        mark_creating(&mut t, &p);
        if let Err(e) = task::save(&t) {
            return task::fail(task::EXIT_ERROR, e);
        }
        p
    };
    finish(cfg, id, plan, Mode::Create)
}

pub fn cmd_remove(cfg: &Config, id: &str, keep_branch: bool) -> ExitCode {
    let t = {
        let _lock = match task::board_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        let t = match task::load_task(cfg, id) {
            Ok(t) => t,
            Err(e) => return task::fail(task::EXIT_ERROR, e),
        };
        if t.status() == task::RUNNING {
            return task::fail(task::EXIT_ERROR, format!("task {id} is running; its worktree is in use. Finish the task first (`task done|fail|abandon {id}`)"));
        }
        if is_creating(&t) {
            return task::fail(task::EXIT_ERROR, format!("task {id}'s worktree is still being created"));
        }
        if t.get("worktree").is_empty() {
            return task::fail(task::EXIT_ERROR, format!("task {id} has no worktree"));
        }
        if !has_worktree(&t) {
            println!("the worktree of task {id} was already removed");
            return ExitCode::SUCCESS;
        }
        t
    }; // lock released: the git work below runs without it
    match remove_for(cfg, &t, keep_branch) {
        Ok(Removal::Removed { branch }) => {
            if let Err(c) = mark_removed(cfg, id, t.get("worktree")) {
                return c;
            }
            println!("{}", say_removed(&t, &branch));
            ExitCode::SUCCESS
        }
        Ok(Removal::Kept { reasons, advice }) => {
            eprintln!("broadsword: the worktree of task {id} was NOT removed (nothing was changed):");
            print_kept(&reasons, &advice, true);
            ExitCode::from(EXIT_KEPT)
        }
        Err(e) => task::fail(task::EXIT_ERROR, e),
    }
}

pub fn cmd_prune(cfg: &Config) -> ExitCode {
    let all = {
        let _lock = match task::board_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        match task::load_tasks(cfg) {
            Ok(a) => a,
            Err(e) => return task::fail(task::EXIT_ERROR, e),
        }
    }; // each task below takes the lock only for its own short record update
    let (mut removed, mut kept, mut skipped, mut failed) = (0, 0, 0, false);
    for t in all.into_iter().filter(has_worktree) {
        let id = t.id().to_string();
        if ![task::DONE, task::FAILED, task::ABANDONED].contains(&t.status()) {
            skipped += 1; // a running (or somehow open) task's worktree is never touched
            continue;
        }
        match remove_for(cfg, &t, false) {
            Ok(Removal::Removed { branch }) => {
                if mark_removed(cfg, &id, t.get("worktree")).is_err() {
                    failed = true;
                }
                removed += 1;
                println!("removed {id}: {}", say_removed(&t, &branch));
            }
            Ok(Removal::Kept { reasons, advice }) => {
                kept += 1;
                println!("kept {id} ({}): {}", t.status(), t.get("worktree"));
                print_kept(&reasons, &advice, false);
            }
            Err(e) => {
                kept += 1;
                failed = true;
                eprintln!("broadsword: {id}: {e}");
            }
        }
    }
    let mut pruned = 0;
    for name in job::list_projects(cfg) {
        let Ok(dir) = job::resolve_project(&cfg.root, &cfg.projects_dir, &name) else { continue };
        if !git::is_repo(&dir) {
            continue;
        }
        match wtgit::worktree_prune(&dir) {
            Ok(()) => pruned += 1,
            Err(e) => {
                eprintln!("broadsword: {name}: {e}");
                failed = true;
            }
        }
    }
    println!("prune: {removed} removed, {kept} kept, {skipped} running or open task(s) left alone; stale git worktree records cleared in {pruned} repo(s)");
    ExitCode::from(if failed { task::EXIT_ERROR } else { 0 })
}

pub fn cmd_list(cfg: &Config) -> ExitCode {
    let all = match task::load_tasks(cfg) {
        Ok(a) => a,
        Err(e) => return task::fail(task::EXIT_ERROR, e),
    };
    let rows: Vec<&Record> = all.iter().filter(|t| has_worktree(t) || is_creating(t)).collect();
    if rows.is_empty() {
        println!("no worktrees");
    }
    for t in rows {
        if is_creating(t) {
            println!("{}\t{}\t-\t{}\t{}\tcreating\t-", t.id(), t.status(), branch_of(t), t.get("base"));
            continue;
        }
        let (clean, merged) = state_of(cfg, t);
        println!("{}\t{}\t{}\t{}\t{}\t{}\t{}", t.id(), t.status(), t.get("worktree"), branch_of(t), t.get("base"), clean, merged);
    }
    ExitCode::SUCCESS
}

/// ("clean" | "dirty" | "missing" | "unknown", "merged" | "unmerged" | "no-branch" | "unknown"), read-only. `dirty` includes
/// ignored files, which also block removal.
fn state_of(cfg: &Config, t: &Record) -> (&'static str, &'static str) {
    let (Ok(dir), Ok(path)) = (project_dir(cfg, t), abs_path(cfg, t.get("worktree"))) else { return ("unknown", "unknown") };
    let clean = if !path.exists() {
        "missing"
    } else {
        match wtgit::status_porcelain(&path) {
            Ok(s) if s.is_empty() => "clean",
            Ok(_) => "dirty",
            Err(_) => "unknown",
        }
    };
    let b = branch_of(t);
    let merged = match wtgit::branch_exists(&dir, &b) {
        Ok(false) => "no-branch",
        Ok(true) => match wtgit::is_merged_into_head(&dir, &b) {
            Ok(true) => "merged",
            Ok(false) => "unmerged",
            Err(_) => "unknown",
        },
        Err(_) => "unknown",
    };
    (clean, merged)
}

pub fn cmd_repair(cfg: &Config) -> ExitCode {
    let mut failed = false;
    for name in job::list_projects(cfg) {
        let Ok(dir) = job::resolve_project(&cfg.root, &cfg.projects_dir, &name) else { continue };
        if !git::is_repo(&dir) {
            println!("{name}: not a Git repo, skipped");
            continue;
        }
        let mine = cfg.worktrees_dir.join(&name);
        let folders: Vec<PathBuf> = std::fs::read_dir(&mine).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect()).unwrap_or_default();
        let refs: Vec<&Path> = folders.iter().map(PathBuf::as_path).collect();
        match wtgit::worktree_repair(&dir, &refs) {
            Ok(out) if out.is_empty() => println!("{name}: ok ({} worktree folder(s) checked)", refs.len()),
            Ok(out) => println!("{name}: repaired ({} worktree folder(s) checked)\n{out}", refs.len()),
            Err(e) => {
                eprintln!("broadsword: {name}: {e}");
                failed = true;
            }
        }
    }
    ExitCode::from(if failed { task::EXIT_ERROR } else { 0 })
}
