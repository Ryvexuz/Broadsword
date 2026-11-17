//! Git checkpoints and rollback. Everything runs the `git` program as a child process.
//!
//! A checkpoint records the exact state of a project's working tree (tracked changes plus untracked,
//! non-ignored files) as a commit under `refs/broadsword/checkpoints/<job id>`. It is built in a
//! temporary index file, so the working tree, HEAD, the branch and the real index are never touched.

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub const CHECKPOINT_REFS: &str = "refs/broadsword/checkpoints";
pub const INTERRUPTED_REFS: &str = "refs/broadsword/interrupted";
pub const INDEX_REFS: &str = "refs/broadsword/index";

/// A project is a Git repo of its own when it has its own `.git` (folder, or file for a linked worktree).
pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn git(dir: &Path, args: &[&str], index: Option<&Path>, input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut c = Command::new("git");
    c.args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_AUTHOR_NAME", "Broadsword")
        .env("GIT_AUTHOR_EMAIL", "broadsword@localhost")
        .env("GIT_COMMITTER_NAME", "Broadsword")
        .env("GIT_COMMITTER_EMAIL", "broadsword@localhost")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(i) = index {
        c.env("GIT_INDEX_FILE", i);
    }
    let mut child = c.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "git is not installed or not on PATH (Broadsword needs it for checkpoints and recovery)".to_string()
        } else {
            format!("cannot start git: {e}")
        }
    })?;
    if let (Some(data), Some(mut stdin)) = (input, child.stdin.take()) {
        let data = data.to_vec();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }
    let out = child.wait_with_output().map_err(|e| format!("git {}: {e}", args.join(" ")))?;
    if out.status.success() {
        Ok(out.stdout)
    } else {
        Err(format!("git {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn git_text(dir: &Path, args: &[&str], index: Option<&Path>) -> Result<String, String> {
    git(dir, args, index, None).map(|b| String::from_utf8_lossy(&b).trim().to_string())
}

pub fn ref_exists(dir: &Path, name: &str) -> bool {
    git_text(dir, &["rev-parse", "--verify", "-q", name], None).is_ok()
}

fn head(dir: &Path) -> Option<String> {
    git_text(dir, &["rev-parse", "--verify", "-q", "HEAD"], None).ok()
}

fn commit_tree(dir: &Path, tree: &str, parent: Option<&str>, msg: &str) -> Result<String, String> {
    let mut args = vec!["commit-tree", tree];
    if let Some(p) = parent {
        args.extend(["-p", p]);
    }
    args.extend(["-m", msg]);
    git_text(dir, &args, None)
}

/// Snapshot the working tree as a commit stored at `ref_name`. Uses `<state>/tmp-index-<tag>` and removes it after.
pub fn snapshot(dir: &Path, state: &Path, tag: &str, ref_name: &str, msg: &str) -> Result<String, String> {
    std::fs::create_dir_all(state).map_err(|e| format!("cannot create {}: {e}", state.display()))?;
    let idx = state.join(format!("tmp-index-{tag}"));
    let _ = std::fs::remove_file(&idx);
    let result = (|| {
        let parent = head(dir);
        if parent.is_some() {
            git_text(dir, &["read-tree", "HEAD"], Some(&idx))?;
        }
        git_text(dir, &["add", "-A"], Some(&idx))?;
        let tree = git_text(dir, &["write-tree"], Some(&idx))?;
        let commit = commit_tree(dir, &tree, parent.as_deref(), msg)?;
        git_text(dir, &["update-ref", ref_name, &commit], None)?;
        Ok(commit)
    })();
    let _ = std::fs::remove_file(&idx);
    result
}

pub struct Checkpoint {
    pub ref_name: String,
    pub commit: String,
    /// Tree of the real index at the time, kept reachable under `refs/broadsword/index/<id>`.
    pub index_tree: Option<String>,
}

pub fn checkpoint(dir: &Path, state: &Path, id: &str) -> Result<Checkpoint, String> {
    let ref_name = format!("{CHECKPOINT_REFS}/{id}");
    let commit = snapshot(dir, state, id, &ref_name, &format!("broadsword checkpoint {id}"))?;
    // `write-tree` only writes tree objects; it does not modify the index. It fails on unmerged entries: then none is kept.
    let index_tree = git_text(dir, &["write-tree"], None).ok().filter(|t| !t.is_empty());
    if let Some(t) = &index_tree {
        let c = commit_tree(dir, t, head(dir).as_deref(), &format!("broadsword index at checkpoint {id}"))?;
        git_text(dir, &["update-ref", &format!("{INDEX_REFS}/{id}"), &c], None)?;
    }
    Ok(Checkpoint { ref_name, commit, index_tree })
}

pub struct Rollback {
    pub interrupted_ref: String,
    pub restored: usize,
    pub removed: usize,
}

fn safe_rel(p: &str) -> Option<PathBuf> {
    let path = Path::new(p);
    path.components().all(|c| matches!(c, Component::Normal(_))).then(|| path.to_path_buf())
}

/// Roll a project's working tree back to `checkpoint_ref`.
/// 1. Save the current tree as `refs/broadsword/interrupted/<id>` (kept if it already exists, so a rollback
///    that was itself interrupted can be repeated without losing the partial work).
/// 2. Restore files that differ: tracked files and removed files come back from the checkpoint; files that
///    were not in the checkpoint (new untracked files, now saved in the interrupted ref) are deleted.
///    Ignored files are never touched: they are in neither tree.
/// 3. Put the real index back to what it was at the checkpoint (or to HEAD if that was not recorded).
///
/// HEAD and the branch are never touched.
pub fn rollback(dir: &Path, state: &Path, id: &str, checkpoint_ref: &str, index_tree: Option<&str>) -> Result<Rollback, String> {
    let interrupted_ref = format!("{INTERRUPTED_REFS}/{id}");
    if !ref_exists(dir, &interrupted_ref) {
        snapshot(dir, state, &format!("{id}-int"), &interrupted_ref, &format!("broadsword interrupted work of {id}"))?;
    }
    let ckpt_tree = git_text(dir, &["rev-parse", "--verify", &format!("{checkpoint_ref}^{{tree}}")], None)?;
    let int_tree = git_text(dir, &["rev-parse", "--verify", &format!("{interrupted_ref}^{{tree}}")], None)?;
    // Changes going from the interrupted state back to the checkpoint.
    let diff = git(dir, &["diff-tree", "-r", "--no-renames", "--name-status", "-z", &int_tree, &ckpt_tree], None, None)?;
    let diff = String::from_utf8_lossy(&diff).into_owned();
    let mut toks = diff.split('\0').filter(|t| !t.is_empty());
    let (mut delete, mut restore) = (Vec::new(), Vec::new());
    while let (Some(status), Some(path)) = (toks.next(), toks.next()) {
        if status.starts_with('D') {
            delete.push(path.to_string()); // in the interrupted tree only: added by the job
        } else {
            restore.push(path.to_string());
        }
    }
    for p in &delete {
        let rel = safe_rel(p).ok_or_else(|| format!("refusing unsafe path from git: {p}"))?;
        let full = dir.join(&rel);
        match std::fs::remove_file(&full) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // an earlier, interrupted rollback already removed it
            Err(e) => return Err(format!("cannot remove {}: {e}", full.display())),
        }
        let mut parent = full.parent();
        while let Some(d) = parent {
            if d == dir || std::fs::remove_dir(d).is_err() {
                break; // only ever removes folders that are now empty
            }
            parent = d.parent();
        }
    }
    if !restore.is_empty() {
        if restore.iter().any(|p| safe_rel(p).is_none()) {
            return Err("refusing unsafe path from git".into());
        }
        let idx = state.join(format!("tmp-index-{id}-restore"));
        let _ = std::fs::remove_file(&idx);
        let r = (|| {
            git_text(dir, &["read-tree", &ckpt_tree], Some(&idx))?;
            let mut input = Vec::new();
            for p in &restore {
                input.extend_from_slice(p.as_bytes());
                input.push(0);
            }
            git(dir, &["checkout-index", "-f", "-z", "--stdin"], Some(&idx), Some(&input)).map(|_| ())
        })();
        let _ = std::fs::remove_file(&idx);
        r?;
    }
    match index_tree {
        Some(t) => git_text(dir, &["read-tree", t], None)?,
        None => git_text(dir, &["reset", "-q"], None)?,
    };
    Ok(Rollback { interrupted_ref, restored: restore.len(), removed: delete.len() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A throwaway repo under the repo's scratch folder with one commit: tracked.txt = "one".
    pub fn temp_repo(tag: &str) -> PathBuf {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        let d = scratch.join(format!("git-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        for a in [&["init", "-q"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"]] {
            git_text(&d, a, None).unwrap();
        }
        fs::write(d.join("tracked.txt"), "one").unwrap();
        fs::write(d.join(".gitignore"), "ignored.txt\n").unwrap();
        git_text(&d, &["add", "-A"], None).unwrap();
        git_text(&d, &["commit", "-q", "-m", "init"], None).unwrap();
        d
    }

    fn status(d: &Path) -> String {
        git_text(d, &["status", "--porcelain"], None).unwrap()
    }

    #[test]
    fn checkpoint_captures_changes_without_touching_anything() {
        let d = temp_repo("ckpt");
        fs::write(d.join("tracked.txt"), "modified").unwrap();
        fs::write(d.join("new.txt"), "untracked").unwrap();
        fs::write(d.join("ignored.txt"), "ignored").unwrap();
        let (head_before, status_before) = (head(&d), status(&d));
        let state = d.join("..").join(format!("state-ckpt-{}", std::process::id()));
        let c = checkpoint(&d, &state, "job1").unwrap();
        assert_eq!(c.ref_name, "refs/broadsword/checkpoints/job1");
        assert_eq!(head(&d), head_before);
        assert_eq!(status(&d), status_before);
        assert_eq!(fs::read_to_string(d.join("tracked.txt")).unwrap(), "modified");
        assert!(!state.join("tmp-index-job1").exists(), "temp index removed");
        let show = |p: &str| git_text(&d, &["show", &format!("{}:{p}", c.ref_name)], None);
        assert_eq!(show("tracked.txt").unwrap(), "modified");
        assert_eq!(show("new.txt").unwrap(), "untracked");
        assert!(show("ignored.txt").is_err(), "ignored files are not captured");
        // the checkpoint's parent is HEAD
        assert_eq!(git_text(&d, &["rev-parse", &format!("{}^", c.ref_name)], None).ok(), head_before);
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn checkpoint_works_in_a_repo_without_commits() {
        let d = temp_repo("empty");
        fs::remove_dir_all(d.join(".git")).unwrap();
        git_text(&d, &["init", "-q"], None).unwrap();
        let state = d.join("..").join(format!("state-empty-{}", std::process::id()));
        let c = checkpoint(&d, &state, "j").unwrap();
        assert_eq!(git_text(&d, &["show", &format!("{}:tracked.txt", c.ref_name)], None).unwrap(), "one");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rollback_restores_and_keeps_partial_work() {
        let d = temp_repo("rb");
        fs::write(d.join("tracked.txt"), "before-job").unwrap(); // uncommitted state at checkpoint
        let state = d.join("..").join(format!("state-rb-{}", std::process::id()));
        let c = checkpoint(&d, &state, "j").unwrap();
        // the "job": modifies, adds (nested), deletes a tracked file, touches an ignored file
        fs::write(d.join("tracked.txt"), "during-job").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join("made.txt"), "made").unwrap();
        fs::write(d.join("ignored.txt"), "keep me").unwrap();
        fs::remove_file(d.join(".gitignore")).unwrap();
        fs::write(d.join(".gitignore"), "ignored.txt\n").unwrap();
        let head_before = head(&d);
        let r = rollback(&d, &state, "j", &c.ref_name, c.index_tree.as_deref()).unwrap();
        assert_eq!(fs::read_to_string(d.join("tracked.txt")).unwrap(), "before-job");
        assert!(!d.join("sub").exists(), "new file and its now-empty folder removed");
        assert_eq!(fs::read_to_string(d.join("ignored.txt")).unwrap(), "keep me");
        assert_eq!(head(&d), head_before);
        assert_eq!((r.restored, r.removed), (1, 1));
        let show = |p: &str| git_text(&d, &["show", &format!("{}:{p}", r.interrupted_ref)], None).unwrap();
        assert_eq!(show("tracked.txt"), "during-job");
        assert_eq!(show("sub/made.txt"), "made");
        // repeating a rollback keeps the first interrupted snapshot
        rollback(&d, &state, "j", &c.ref_name, c.index_tree.as_deref()).unwrap();
        assert_eq!(show("sub/made.txt"), "made");
        // the index matches HEAD, as before the job: only the unstaged change shows
        assert_eq!(status(&d), "M tracked.txt");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }
}
