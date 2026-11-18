//! Every Git command Broadsword runs for per-task worktrees, in this one file. Each function is one `git` call with a
//! one-line "what and why" comment above it (AGENTS.md "Learning Git": the orchestrator copies these into LAB_LOG.md).
//!
//! Hard rules, enforced by what exists here: there is no function that passes `--force` to `git worktree remove`, no
//! `git branch -D`, and nothing that deletes files itself. A worktree or branch with work in it can only be refused.

use std::path::Path;
use std::process::{Command, Stdio};

pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Run `git <args>` in `dir`. `Err` only when git cannot be started; a non-zero exit is an `Out` with its code.
fn run(dir: &Path, args: &[&str]) -> Result<Out, String> {
    run_with(dir, args, None)
}

/// Same, feeding `input` to git's stdin (used for long lists of commits, which would not fit on a command line).
fn run_with(dir: &Path, args: &[&str], input: Option<String>) -> Result<Out, String> {
    let mut child = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { "git is not installed or not on PATH".to_string() } else { format!("cannot start git: {e}") })?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        std::thread::spawn(move || {
            let _ = std::io::Write::write_all(&mut stdin, data.as_bytes());
        });
    }
    let o = child.wait_with_output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    Ok(Out { code: o.status.code().unwrap_or(-1), stdout: String::from_utf8_lossy(&o.stdout).trim().to_string(), stderr: String::from_utf8_lossy(&o.stderr).trim().to_string() })
}

/// Git's own hints about forcing (`use 'remove -f -f'`, `use -D to force deletion`) are dropped from its messages:
/// Broadsword never forces, and must not suggest it.
fn without_force_hints(msg: &str) -> String {
    msg.lines()
        .filter(|l| !l.split_whitespace().any(|w| matches!(w.trim_matches(|c: char| c == '\'' || c == '"' || c == ',' || c == '.'), "-f" | "-ff" | "--force" | "-D")) && !l.to_lowercase().contains("force"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exit code 0 -> trimmed stdout; anything else -> an error naming the command and git's own message.
fn ok(dir: &Path, args: &[&str]) -> Result<String, String> {
    let o = run(dir, args)?;
    if o.code == 0 {
        Ok(o.stdout)
    } else {
        Err(format!("git {} failed: {}", args.join(" "), without_force_hints(&o.stderr)))
    }
}

/// `git symbolic-ref -q --short HEAD`: which branch is checked out here; `None` means detached HEAD (exit 1). Why: a task's
/// worktree must start from the tip of a real branch, and removal must know the worktree is still on its task branch.
pub fn current_branch(dir: &Path) -> Result<Option<String>, String> {
    let o = run(dir, &["symbolic-ref", "-q", "--short", "HEAD"])?;
    match o.code {
        0 => Ok(Some(o.stdout)),
        1 => Ok(None),
        _ => Err(format!("git symbolic-ref -q --short HEAD failed: {}", o.stderr)),
    }
}

/// `git rev-parse HEAD`: the commit at the tip of the checked-out branch, resolved fresh. Why: it is the base of the new
/// task branch, so a task never starts from a stale commit.
pub fn head_commit(dir: &Path) -> Result<String, String> {
    ok(dir, &["rev-parse", "HEAD"])
}

/// `git show-ref --verify --quiet refs/heads/<branch>`: does the local branch exist (exit 0) or not (exit 1)? Why: refuse to
/// create a task branch that is already there, and notice a branch deleted by hand.
pub fn branch_exists(dir: &Path, branch: &str) -> Result<bool, String> {
    let r = format!("refs/heads/{branch}");
    let o = run(dir, &["show-ref", "--verify", "--quiet", &r])?;
    match o.code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(format!("git show-ref --verify --quiet {r} failed: {}", o.stderr)),
    }
}

/// `git worktree add -b <branch> <path> <base>`: make a new branch at `base` and check it out in a new folder that shares
/// this repo's history. Why: the task's private working copy; the main checkout is never touched.
pub fn worktree_add(dir: &Path, branch: &str, path: &Path, base: &str) -> Result<(), String> {
    let p = path.to_string_lossy();
    ok(dir, &["worktree", "add", "-b", branch, &p, base]).map(|_| ())
}

/// `git status --porcelain --ignored`: one line per modified, staged, untracked (`??`) or IGNORED (`!!`) path; empty output =
/// the worktree holds nothing git does not already have. Ignored files are listed on purpose: `.env` files and notes are
/// ignored too and Broadsword cannot tell them from build output, so any of them blocks removal. Why: removal must never
/// throw away uncommitted, untracked or ignored files.
pub fn status_porcelain(worktree: &Path) -> Result<String, String> {
    ok(worktree, &["status", "--porcelain", "--ignored"])
}

/// `git rev-parse --verify refs/heads/<branch>`: the commit a local branch points at. Why: tell whether a branch is still
/// at its base (empty), and finish or undo an interrupted creation.
pub fn branch_tip(dir: &Path, branch: &str) -> Result<String, String> {
    ok(dir, &["rev-parse", "--verify", &format!("refs/heads/{branch}")])
}

/// `git log -g --format=%H HEAD`: every commit the worktree's own HEAD has pointed at (its reflog, which dies with the
/// worktree). Why: a commit made on a detached HEAD and left behind would lose its last reference on removal.
pub fn reflog_commits(worktree: &Path) -> Result<Vec<String>, String> {
    let out = ok(worktree, &["log", "-g", "--format=%H", "HEAD"])?;
    let mut v: Vec<String> = Vec::new();
    for l in out.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !v.iter().any(|x| x == l) {
            v.push(l.to_string());
        }
    }
    Ok(v)
}

/// `git rev-list --stdin --not --all` (run in the PROJECT's repo, commits fed on stdin): of the given commits, which are
/// reachable from no branch, tag or other ref? It is one git call for the whole list (not one per commit): rev-list walks
/// from the given commits, hides everything reachable from any ref (`--all`, which includes the main checkout's HEAD), and
/// prints what is left; the answer is the given commits that appear in that output. Why: a commit on no ref would be lost
/// once the worktree's HEAD log is deleted.
pub fn on_no_ref(dir: &Path, commits: &[String]) -> Result<Vec<String>, String> {
    if commits.is_empty() {
        return Ok(Vec::new());
    }
    let o = run_with(dir, &["rev-list", "--stdin", "--not", "--all"], Some(commits.join("\n") + "\n"))?;
    if o.code != 0 {
        return Err(format!("git rev-list --stdin --not --all failed: {}", without_force_hints(&o.stderr)));
    }
    let unreachable: Vec<&str> = o.stdout.lines().map(str::trim).collect();
    Ok(commits.iter().filter(|c| unreachable.contains(&c.as_str())).cloned().collect())
}

/// `git update-ref <ref> <commit> ""` then `git rev-parse --verify <ref>`: create a ref that must not exist yet (the empty
/// old value means "only if absent"), and read it back. An existing ref already at `commit` counts as done. Why: gives a
/// commit that would lose its last reference a permanent home under refs/broadsword/kept/.
pub fn keep_ref(dir: &Path, name: &str, commit: &str) -> Result<(), String> {
    if ok(dir, &["rev-parse", "--verify", "-q", name]).is_ok_and(|c| c == commit) {
        return Ok(());
    }
    ok(dir, &["update-ref", name, commit, ""])?;
    match ok(dir, &["rev-parse", "--verify", name]) {
        Ok(c) if c == commit => Ok(()),
        other => Err(format!("ref {name} did not read back as {commit}: {other:?}")),
    }
}

/// Make a path comparable: absolute, `/` separators, no `\\?\` prefix, no trailing slash, case-folded on Windows.
fn norm(p: &str) -> String {
    let p = std::fs::canonicalize(p).map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string());
    let p = p.trim_start_matches(r"\\?\").replace('\\', "/");
    let p = p.trim_end_matches('/').to_string();
    if cfg!(windows) {
        p.to_lowercase()
    } else {
        p
    }
}

/// `git rev-parse --show-toplevel` and `git rev-parse --path-format=absolute --git-common-dir` in the folder, and in the
/// project: is `path` really the root of a worktree of THIS project's repo? A plain folder (or one in some other repo)
/// would otherwise make every later git command run in the enclosing repo. Why: never judge or touch the wrong repo.
pub fn is_worktree_of(project_dir: &Path, path: &Path) -> bool {
    let Ok(top) = ok(path, &["rev-parse", "--show-toplevel"]) else { return false };
    if norm(&top) != norm(&path.to_string_lossy()) {
        return false;
    }
    let common = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
    match (ok(path, &common), ok(project_dir, &common)) {
        (Ok(a), Ok(b)) => norm(&a) == norm(&b),
        _ => false,
    }
}

/// `git merge-base --is-ancestor <commit> HEAD`: is the commit already part of the main checkout's current branch (exit 0)
/// or not (exit 1)? Why: "merged" means the branch tip is reachable from where the project is now.
pub fn is_merged_into_head(main_dir: &Path, commit_or_branch: &str) -> Result<bool, String> {
    let o = run(main_dir, &["merge-base", "--is-ancestor", commit_or_branch, "HEAD"])?;
    match o.code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(format!("git merge-base --is-ancestor {commit_or_branch} HEAD failed: {}", o.stderr)),
    }
}

/// `git worktree remove <path>` (never `--force`): delete the worktree folder and Git's record of it. Git itself refuses
/// when the worktree has changes or untracked files. Why: the automatic cleanup once a task's work is merged.
pub fn worktree_remove(dir: &Path, path: &Path) -> Result<(), String> {
    let p = path.to_string_lossy();
    ok(dir, &["worktree", "remove", &p]).map(|_| ())
}

/// `git branch -d <branch>` (never `-D`): delete a branch only if it is fully merged into the current branch; Git refuses
/// otherwise. Why: a second, independent guard so an unmerged branch (the only copy of its commits) cannot be deleted.
pub fn branch_delete_if_merged(dir: &Path, branch: &str) -> Result<(), String> {
    ok(dir, &["branch", "-d", branch]).map(|_| ())
}

/// `git worktree prune`: forget the records of worktree folders that no longer exist (deleted by hand). Why: keeps Git's
/// list honest and frees the branch of a vanished worktree.
pub fn worktree_prune(dir: &Path) -> Result<(), String> {
    ok(dir, &["worktree", "prune"]).map(|_| ())
}

/// `git worktree repair [<path>...]`: rewrite the full paths Git stores in both directions between the repo and each
/// worktree. Why: Git stores absolute paths, so after the project folder moves to another drive they all point nowhere.
/// The worktree folders are passed explicitly because that is how Git finds them when the main repo itself moved.
pub fn worktree_repair(dir: &Path, worktrees: &[&Path]) -> Result<String, String> {
    let paths: Vec<String> = worktrees.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    let mut args = vec!["worktree", "repair"];
    args.extend(paths.iter().map(String::as_str));
    let o = run(dir, &args)?;
    if o.code == 0 {
        Ok(format!("{}\n{}", o.stdout, o.stderr).trim().to_string()) // repair reports what it fixed on stderr
    } else {
        Err(format!("git {} failed: {}", args.join(" "), o.stderr))
    }
}

// ---- reviewer step and merge queue (queue.rs) -------------------------------------------------------------------------
//
// Same hard rules as above: no `--force`, no `-D`, no `reset --hard`, no `push`. The two commands that move a branch are
// `rebase` (only ever inside a task's own worktree, and always aborted on any failure) and `merge --ff-only` (only ever in
// the main checkout, and it can only move HEAD forward along a line it already contains). Neither can drop a commit
// without it staying reachable: queue.rs saves the pre-rebase tip under refs/broadsword/queue/ first.

/// `git status --porcelain` (NO `--ignored`): one line per modified, staged or untracked path; empty = clean. Why: the MAIN
/// checkout must be clean before a merge; unlike a worktree about to be deleted, ignored files there (build folders,
/// `.env`) are normal and not at risk.
pub fn status_porcelain_plain(dir: &Path) -> Result<String, String> {
    ok(dir, &["status", "--porcelain"])
}

/// `git rev-list --count <range>`: how many commits a range holds, e.g. `<base>..<branch>` = commits on the branch that
/// the base does not have. Why: "at least one commit beyond its base" and "nothing left to merge".
pub fn rev_count(dir: &Path, range: &str) -> Result<u64, String> {
    let out = ok(dir, &["rev-list", "--count", range])?;
    out.trim().parse::<u64>().map_err(|_| format!("git rev-list --count {range} printed `{out}`, not a number"))
}

/// `git merge-base --is-ancestor <ancestor> <descendant>`: is the first commit part of the history of the second (exit 0)
/// or not (exit 1)? Why: "the branch already contains main's HEAD" (skip the rebase) and "the rebase really landed on it".
pub fn is_ancestor(dir: &Path, ancestor: &str, descendant: &str) -> Result<bool, String> {
    let o = run(dir, &["merge-base", "--is-ancestor", ancestor, descendant])?;
    match o.code {
        0 => Ok(true),
        1 => Ok(false),
        _ => Err(format!("git merge-base --is-ancestor {ancestor} {descendant} failed: {}", o.stderr)),
    }
}

/// `git diff --no-color --no-ext-diff <base>...<tip>`: the changes on `tip` since it split from `base` (three dots = from
/// their merge base), nothing excluded. Why: exactly what a merge would bring in is what the reviewer reads.
pub fn diff_since(dir: &Path, base: &str, tip: &str) -> Result<String, String> {
    ok(dir, &["diff", "--no-color", "--no-ext-diff", &format!("{base}...{tip}")])
}

/// `git diff --name-only -z --no-renames <base>...<tip>`: every path the branch changes since it split from `base` (a rename lists
/// both its old and new path; `-z` so odd names are never quoted). Why: the merge queue picks the light or full check from the paths.
pub fn diff_names(dir: &Path, base: &str, tip: &str) -> Result<Vec<String>, String> {
    let range = format!("{base}...{tip}");
    let o = run(dir, &["diff", "--name-only", "-z", "--no-renames", "--no-ext-diff", &range])?;
    if o.code != 0 {
        return Err(format!("git diff --name-only {range} failed: {}", without_force_hints(&o.stderr)));
    }
    Ok(o.stdout.split('\0').filter(|s| !s.is_empty()).map(String::from).collect())
}

/// `git rev-parse --path-format=absolute --git-path rebase-merge` (and `rebase-apply`), then does that folder exist? Git
/// keeps the state of a rebase that is stopped or was killed there. Why: a crashed `queue run` may have left a worktree
/// in the middle of a rebase, which must be aborted before anything else touches it.
pub fn rebase_in_progress(worktree: &Path) -> Result<bool, String> {
    for name in ["rebase-merge", "rebase-apply"] {
        let p = ok(worktree, &["rev-parse", "--path-format=absolute", "--git-path", name])?;
        if Path::new(&p).exists() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `git rebase --abort`: stop a rebase and put the branch back exactly where it was before it began. Why: a conflict (or
/// any failure) must leave the branch as it was.
pub fn rebase_abort(worktree: &Path) -> Result<(), String> {
    ok(worktree, &["rebase", "--abort"]).map(|_| ())
}

/// `git diff --name-only --diff-filter=U`: the paths with unresolved conflicts. Why: the entry names them for the coder.
fn unmerged_files(worktree: &Path) -> Result<Vec<String>, String> {
    Ok(ok(worktree, &["diff", "--name-only", "--diff-filter=U"])?.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

pub enum Rebase {
    /// The branch now sits on top of `onto`.
    Done,
    /// Conflicts: the rebase was aborted, the branch is as it was; these are the conflicting files.
    Conflict(Vec<String>),
    /// Git failed for another reason; any half-done rebase was aborted (the text says whether that worked).
    Failed(String),
}

/// `git rebase <onto>` in the task's worktree (its branch is checked out there): replay the branch's own commits on top of
/// `onto`. On ANY failure it then runs `git rebase --abort` (a rebase that stopped is never left behind), so the branch is
/// back where it started; the caller still verifies the tip. Why: the merge into main must be a pure fast-forward.
pub fn rebase(worktree: &Path, onto: &str) -> Result<Rebase, String> {
    let o = run(worktree, &["rebase", onto])?;
    if o.code == 0 {
        return Ok(Rebase::Done);
    }
    let files = unmerged_files(worktree).unwrap_or_default();
    let msg = without_force_hints(if o.stderr.is_empty() { &o.stdout } else { &o.stderr });
    let aborted = match rebase_in_progress(worktree) {
        Ok(true) => rebase_abort(worktree).map(|_| "the rebase was aborted".to_string()),
        Ok(false) => Ok("no rebase was left running".to_string()),
        Err(e) => Err(e),
    };
    Ok(match (files.is_empty(), aborted) {
        (false, Ok(_)) => Rebase::Conflict(files),
        (_, Ok(a)) => Rebase::Failed(format!("git rebase {onto} failed ({a}): {msg}")),
        (_, Err(e)) => Rebase::Failed(format!("git rebase {onto} failed AND it could not be aborted: {e}; look at the worktree: {msg}")),
    })
}

/// `git merge --ff-only <commit>` in the main checkout: move the current branch forward to `commit`, only if that is a pure
/// fast-forward (the branch tip is an ancestor of it); never a merge commit, git refuses otherwise. Why: the one way task
/// work enters main, and it cannot rewrite or lose anything main already has.
pub fn merge_ff_only(main_dir: &Path, commit: &str) -> Result<(), String> {
    ok(main_dir, &["merge", "--ff-only", commit]).map(|_| ())
}
