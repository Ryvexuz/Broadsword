//! One Git worktree per task: `broadsword worktree create|remove|prune|list|repair`, plus the pieces `task start
//! --worktree` and `task done` call.
//!
//! A task's worktree is `<root>/<worktrees_dir>/<canonical project>/<task id>` on the branch `task/<task id>`, in the
//! PROJECT's own Git repo (Broadsword's for `broadsword`, `projects/<p>` for the others). The task record keeps
//! `worktree` (the path RELATIVE to root, so the record survives moving the project), `branch`, `base` (the commit the
//! branch started from) and, once removed, `worktree_removed`.
//!
//! Removal safety, one function (`remove_for`) used by `task done`, `worktree remove` and `worktree prune`:
//! - the worktree must be CLEAN: `git status --porcelain --ignored=no` empty (no modified, staged or untracked files;
//!   ignored files such as build output are regenerable and do not count);
//! - it must still be on its task branch (a detached HEAD or another branch would hide commits that removal loses);
//! - the branch must be MERGED: its tip is an ancestor of the main checkout's HEAD (`--keep-branch` skips this one rule,
//!   because the branch, which holds the commits, is kept);
//! - then `git worktree remove` (no `--force`), then `git branch -d` (no `-D`, so Git refuses an unmerged branch too),
//!   then empty parent folders under worktrees_dir are removed with `remove_dir` only.
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

/// Remove `start` (if empty) and every empty parent up to, but never including, worktrees_dir. `remove_dir` only.
fn remove_empty_parents(cfg: &Config, start: &Path) {
    let mut p = Some(start);
    while let Some(d) = p {
        if d == cfg.worktrees_dir || !d.starts_with(&cfg.worktrees_dir) || std::fs::remove_dir(d).is_err() {
            break;
        }
        p = d.parent();
    }
}

pub fn has_worktree(t: &Record) -> bool {
    !t.get("worktree").is_empty() && t.get("worktree_removed").is_empty()
}

fn branch_of(t: &Record) -> String {
    if t.get("branch").is_empty() {
        format!("task/{}", t.id())
    } else {
        t.get("branch").to_string()
    }
}

// ---- creation ---------------------------------------------------------------------------

pub struct Created {
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
}

/// Make the task's worktree and fill `worktree`, `branch`, `base` into the record (the caller saves it). On any failure
/// nothing is left behind: Git cleans its own half-made state and the empty folders made here are removed.
pub fn create_for(cfg: &Config, t: &mut Record) -> Result<Created, String> {
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
    let on = wtgit::current_branch(&dir)?.ok_or_else(|| format!("the main checkout of `{project}` ({}) is on a detached HEAD; check out a branch first, so the task's base is a branch tip", dir.display()))?;
    let base = wtgit::head_commit(&dir)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    if let Err(e) = wtgit::worktree_add(&dir, &branch, &path, &base) {
        remove_empty_parents(cfg, &path);
        return Err(format!("{e} (the main checkout is on `{on}`)"));
    }
    t.set("worktree", rel);
    t.set("branch", branch.clone());
    t.set("base", base.clone());
    t.set("worktree_removed", "");
    Ok(Created { path, branch, base })
}

/// Best-effort undo of a `create_for` whose record could not be saved: the new worktree is clean and its branch is at
/// the base, so the same safe commands remove it. Returns what is left, if anything.
pub fn undo_create(cfg: &Config, t: &Record, c: &Created) -> Option<String> {
    let dir = project_dir(cfg, t).ok()?;
    if let Err(e) = wtgit::worktree_remove(&dir, &c.path) {
        return Some(format!("{e}; remove {} by hand", c.path.display()));
    }
    let _ = wtgit::branch_delete_if_merged(&dir, &c.branch);
    remove_empty_parents(cfg, c.path.parent()?);
    None
}

pub fn describe_created(c: &Created) -> String {
    format!("worktree: {}\nbranch: {}\nbase: {}", c.path.display(), c.branch, c.base)
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

/// The one safe removal. On `Removed` the record has `worktree_removed` set (the caller saves it); on `Kept` the record is
/// unchanged. `Err` means git itself could not be asked (nothing was changed).
pub fn remove_for(cfg: &Config, t: &mut Record, keep_branch: bool) -> Result<Removal, String> {
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
                let lines: Vec<&str> = s.lines().collect();
                let shown = lines.iter().take(5).map(|l| l.trim_end()).collect::<Vec<_>>().join("; ");
                reasons.push(format!("it has uncommitted or untracked changes ({} path(s): {shown}{})", lines.len(), if lines.len() > 5 { "; ..." } else { "" }));
            }
            Err(e) => reasons.push(format!("its state cannot be read: {e}")),
        }
        if !reasons.is_empty() {
            advice.push(format!("commit or discard the changes in {} yourself, then run `broadsword worktree remove {id}`", path.display()));
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
            return Ok(Removal::Kept { reasons: vec![format!("git refused to remove it: {e}")], advice: vec!["fix what git says, or remove it yourself; nothing was deleted".into()] });
        }
    } else {
        // The folder is already gone (deleted by hand): only Git's record is left, and there is nothing on disk to lose.
        wtgit::worktree_prune(&dir)?;
    }
    if let Some(parent) = path.parent() {
        remove_empty_parents(cfg, parent);
    }
    t.set("worktree_removed", task::now_secs().to_string());
    let branch = if keep_branch {
        BranchFate::Kept("kept as requested (--keep-branch)".into())
    } else if !branch_there {
        BranchFate::AlreadyGone
    } else if !merged {
        BranchFate::Kept(format!("not merged into `{into}`, so it is kept with its commits"))
    } else {
        match wtgit::branch_delete_if_merged(&dir, &branch) {
            Ok(()) => BranchFate::Deleted,
            Err(e) => BranchFate::Kept(format!("git refused to delete it: {e}")),
        }
    };
    Ok(Removal::Removed { branch })
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

/// What `task done` prints about the worktree after the task is marked done. Never an error: the task is done either way.
pub fn after_done(cfg: &Config, t: &mut Record) -> bool {
    match remove_for(cfg, t, false) {
        Ok(Removal::Removed { branch }) => {
            println!("{}", say_removed(t, &branch));
            true
        }
        Ok(Removal::Kept { reasons, advice }) => {
            println!("worktree kept: {}", t.get("worktree"));
            eprintln!("broadsword: WARNING: task {} is {DONE}, but its worktree and branch were kept (nothing was lost):", t.id());
            for r in &reasons {
                eprintln!("  - {r}");
            }
            for a in &advice {
                eprintln!("  to fix: {a}");
            }
            false
        }
        Err(e) => {
            println!("worktree kept: {}", t.get("worktree"));
            eprintln!("broadsword: WARNING: task {} is {DONE}, but its worktree could not be checked, so it was kept: {e}", t.id());
            false
        }
    }
}

// ---- commands ---------------------------------------------------------------------------

pub fn cmd_create(cfg: &Config, id: &str) -> ExitCode {
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
    let c = match create_for(cfg, &mut t) {
        Ok(c) => c,
        Err(e) => return task::fail(task::EXIT_ERROR, format!("no worktree was made for {id}: {e}")),
    };
    if let Err(e) = task::save(&t) {
        let left = undo_create(cfg, &t, &c).map(|l| format!(" and it could not be undone: {l}")).unwrap_or_default();
        return task::fail(task::EXIT_ERROR, format!("{e}; the new worktree was removed again{left}"));
    }
    println!("{}", describe_created(&c));
    ExitCode::SUCCESS
}

pub fn cmd_remove(cfg: &Config, id: &str, keep_branch: bool) -> ExitCode {
    let _lock = match task::board_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let mut t = match task::load_task(cfg, id) {
        Ok(t) => t,
        Err(e) => return task::fail(task::EXIT_ERROR, e),
    };
    if t.status() == task::RUNNING {
        return task::fail(task::EXIT_ERROR, format!("task {id} is running; its worktree is in use. Finish the task first (`task done|fail|abandon {id}`)"));
    }
    if t.get("worktree").is_empty() {
        return task::fail(task::EXIT_ERROR, format!("task {id} has no worktree"));
    }
    if !has_worktree(&t) {
        println!("the worktree of task {id} was already removed");
        return ExitCode::SUCCESS;
    }
    match remove_for(cfg, &mut t, keep_branch) {
        Ok(Removal::Removed { branch }) => {
            if let Err(e) = task::save(&t) {
                return task::fail(task::EXIT_ERROR, format!("{e} (the worktree itself was removed)"));
            }
            println!("{}", say_removed(&t, &branch));
            ExitCode::SUCCESS
        }
        Ok(Removal::Kept { reasons, advice }) => {
            eprintln!("broadsword: the worktree of task {id} was NOT removed (nothing was changed):");
            for r in &reasons {
                eprintln!("  - {r}");
            }
            for a in &advice {
                eprintln!("  to fix: {a}");
            }
            ExitCode::from(EXIT_KEPT)
        }
        Err(e) => task::fail(task::EXIT_ERROR, e),
    }
}

pub fn cmd_prune(cfg: &Config) -> ExitCode {
    let _lock = match task::board_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let all = match task::load_tasks(cfg) {
        Ok(a) => a,
        Err(e) => return task::fail(task::EXIT_ERROR, e),
    };
    let (mut removed, mut kept, mut skipped, mut failed) = (0, 0, 0, false);
    for mut t in all.into_iter().filter(has_worktree) {
        let id = t.id().to_string();
        if ![task::DONE, task::FAILED, task::ABANDONED].contains(&t.status()) {
            skipped += 1; // a running (or somehow open) task's worktree is never touched
            continue;
        }
        match remove_for(cfg, &mut t, false) {
            Ok(Removal::Removed { branch }) => {
                if let Err(e) = task::save(&t) {
                    eprintln!("broadsword: {e} (the worktree of {id} was removed)");
                    failed = true;
                }
                removed += 1;
                println!("removed {id}: {}", say_removed(&t, &branch));
            }
            Ok(Removal::Kept { reasons, advice }) => {
                kept += 1;
                println!("kept {id} ({}): {}", t.status(), t.get("worktree"));
                for r in &reasons {
                    println!("  - {r}");
                }
                for a in &advice {
                    println!("  to fix: {a}");
                }
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
    let rows: Vec<&Record> = all.iter().filter(|t| has_worktree(t)).collect();
    if rows.is_empty() {
        println!("no worktrees");
    }
    for t in rows {
        let (clean, merged) = state_of(cfg, t);
        println!("{}\t{}\t{}\t{}\t{}\t{}\t{}", t.id(), t.status(), t.get("worktree"), branch_of(t), t.get("base"), clean, merged);
    }
    ExitCode::SUCCESS
}

/// ("clean" | "dirty" | "missing" | "unknown", "merged" | "unmerged" | "no-branch" | "unknown"), read-only.
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
