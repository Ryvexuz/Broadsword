//! Git checkpoints and rollback. Everything runs the `git` program as a child process.
//!
//! A snapshot records the exact bytes of a project's working tree: tracked and untracked non-ignored
//! files as one commit, and ignored files (minus build, scratch, state and `target` folders, and
//! subject to a size cap) as a second, parentless commit. Files are stored raw (`hash-object -w
//! --no-filters`) and trees are built with `update-index --index-info` in a temporary index, so no
//! line-ending or other filter ever touches the content, and the working tree, HEAD, the branch and
//! the real index are never changed by taking a snapshot. Restoring writes the raw blobs back.
//!
//! Refs (a ref cannot be both a file and a folder, so the ignored snapshots live in sibling folders):
//! - `refs/broadsword/checkpoints/<id>` and `refs/broadsword/checkpoints-ignored/<id>`
//! - `refs/broadsword/interrupted/<id>/<attempt>` and `refs/broadsword/interrupted-ignored/<id>/<attempt>`
//! - `refs/broadsword/index/<id>`: the real index at checkpoint time
//!
//! Refs are only ever created (never overwritten or deleted), so no snapshot is ever lost.

use crate::config::{Config, MB};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

pub const CHECKPOINT_REFS: &str = "refs/broadsword/checkpoints";
pub const CHECKPOINT_IGNORED_REFS: &str = "refs/broadsword/checkpoints-ignored";
pub const INTERRUPTED_REFS: &str = "refs/broadsword/interrupted";
pub const INTERRUPTED_IGNORED_REFS: &str = "refs/broadsword/interrupted-ignored";
pub const INDEX_REFS: &str = "refs/broadsword/index";

/// What a snapshot leaves out of the ignored set, and how much of it it keeps.
pub struct Opts {
    /// Folders (relative to the project) never captured among ignored files: build, scratch, state.
    pub excludes: Vec<PathBuf>,
    pub ignored_max_bytes: u64,
}

impl Opts {
    pub fn for_project(cfg: &Config, dir: &Path) -> Opts {
        Opts {
            excludes: [&cfg.build_dir, &cfg.scratch_dir, &cfg.state_dir].iter().filter_map(|p| p.strip_prefix(dir).ok().map(Path::to_path_buf)).collect(),
            ignored_max_bytes: cfg.checkpoint_ignored_max_mb * MB,
        }
    }
}

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

/// Create a ref that must not exist yet, then check it reads back as `commit`.
fn create_ref(dir: &Path, name: &str, commit: &str) -> Result<(), String> {
    git_text(dir, &["update-ref", name, commit, ""], None)?;
    match git_text(dir, &["rev-parse", "--verify", name], None) {
        Ok(c) if c == commit => Ok(()),
        other => Err(format!("ref {name} did not read back as {commit}: {other:?}")),
    }
}

fn unique_index(state: &Path, tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    state.join(format!("tmp-index-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

fn list_files(dir: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let out = git(dir, args, None, None)?;
    Ok(String::from_utf8_lossy(&out).split('\0').filter(|s| !s.is_empty()).map(str::to_string).collect())
}

fn is_regular(dir: &Path, rel: &str) -> bool {
    std::fs::symlink_metadata(dir.join(rel)).map(|m| m.is_file()).unwrap_or(false)
}

#[cfg(unix)]
fn disk_mode(path: &Path) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(path) {
        Ok(m) if m.permissions().mode() & 0o111 != 0 => "100755",
        _ => "100644",
    }
}

#[cfg(not(unix))]
fn disk_mode(_: &Path) -> &'static str {
    "100644" // Windows has no executable bit; tracked files keep the mode Git recorded
}

/// Store these files raw and build a tree out of them in a temporary index.
fn tree_of(dir: &Path, state: &Path, tag: &str, files: &[String], tracked_modes: &HashMap<String, String>) -> Result<String, String> {
    let idx = unique_index(state, tag);
    let result = (|| {
        if !files.is_empty() {
            if let Some(bad) = files.iter().find(|f| f.contains('\n')) {
                return Err(format!("cannot snapshot a file name containing a newline: {bad:?}"));
            }
            let shas = git(dir, &["hash-object", "-w", "--no-filters", "--stdin-paths"], None, Some(files.join("\n").as_bytes()))?;
            let shas: Vec<String> = String::from_utf8_lossy(&shas).lines().map(str::to_string).collect();
            if shas.len() != files.len() {
                return Err(format!("hash-object returned {} hashes for {} files", shas.len(), files.len()));
            }
            let mut input = Vec::new();
            for (f, sha) in files.iter().zip(&shas) {
                let mode = match tracked_modes.get(f).map(String::as_str) {
                    Some(m @ ("100644" | "100755")) => m,
                    _ => disk_mode(&dir.join(f)),
                };
                input.extend_from_slice(format!("{mode} {sha}\t{f}").as_bytes());
                input.push(0);
            }
            git(dir, &["update-index", "-z", "--index-info"], Some(&idx), Some(&input))?;
        }
        git_text(dir, &["write-tree"], Some(&idx))
    })();
    let _ = std::fs::remove_file(&idx);
    result
}

pub struct Captured {
    pub tree: String,
    pub ignored_tree: Option<String>,
    /// Ignored files left out because the size cap was reached.
    pub skipped: Vec<(String, u64)>,
}

fn excluded(rel: &str, opts: &Opts) -> bool {
    let p = Path::new(rel);
    p.components().any(|c| c.as_os_str() == "target") || opts.excludes.iter().any(|e| p.starts_with(e))
}

/// Read the working tree's raw bytes into Git objects. Touches nothing but the object store.
pub fn capture(dir: &Path, state: &Path, tag: &str, opts: &Opts) -> Result<Captured, String> {
    std::fs::create_dir_all(state).map_err(|e| format!("cannot create {}: {e}", state.display()))?;
    let mut modes = HashMap::new();
    for entry in list_files(dir, &["ls-files", "-s", "-z"])? {
        if let Some((meta, path)) = entry.split_once('\t') {
            modes.insert(path.to_string(), meta.split(' ').next().unwrap_or("").to_string());
        }
    }
    let mut files = list_files(dir, &["ls-files", "-z", "-c", "-o", "--exclude-standard"])?;
    files.sort();
    files.dedup();
    files.retain(|p| is_regular(dir, p)); // tracked files deleted from disk are simply absent from the snapshot
    let tree = tree_of(dir, state, tag, &files, &modes)?;

    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    let mut used = 0u64;
    for p in list_files(dir, &["ls-files", "-z", "-o", "-i", "--exclude-standard"])? {
        if excluded(&p, opts) || !is_regular(dir, &p) {
            continue;
        }
        let len = std::fs::metadata(dir.join(&p)).map(|m| m.len()).unwrap_or(0);
        if used.saturating_add(len) > opts.ignored_max_bytes {
            skipped.push((p, len));
        } else {
            used += len;
            kept.push(p);
        }
    }
    let ignored_tree = if kept.is_empty() { None } else { Some(tree_of(dir, state, &format!("{tag}-ign"), &kept, &HashMap::new())?) };
    Ok(Captured { tree, ignored_tree, skipped })
}

pub struct Checkpoint {
    pub ref_name: String,
    pub commit: String,
    pub ignored_ref: Option<String>,
    pub skipped: Vec<(String, u64)>,
    /// Tree of the real index at the time, kept reachable under `refs/broadsword/index/<id>`.
    pub index_tree: Option<String>,
}

pub fn checkpoint(dir: &Path, state: &Path, id: &str, opts: &Opts) -> Result<Checkpoint, String> {
    let cap = capture(dir, state, id, opts)?;
    let parent = head(dir);
    let ignored_ref = match &cap.ignored_tree {
        Some(t) => {
            let c = commit_tree(dir, t, None, &format!("broadsword checkpoint of ignored files {id}"))?;
            let r = format!("{CHECKPOINT_IGNORED_REFS}/{id}");
            create_ref(dir, &r, &c)?;
            Some(r)
        }
        None => None,
    };
    let commit = commit_tree(dir, &cap.tree, parent.as_deref(), &format!("broadsword checkpoint {id}"))?;
    let ref_name = format!("{CHECKPOINT_REFS}/{id}");
    create_ref(dir, &ref_name, &commit)?;
    // `write-tree` only writes tree objects; it does not modify the index. It fails on unmerged entries: then none is kept.
    let index_tree = git_text(dir, &["write-tree"], None).ok().filter(|t| !t.is_empty());
    if let Some(t) = &index_tree {
        let c = commit_tree(dir, t, parent.as_deref(), &format!("broadsword index at checkpoint {id}"))?;
        create_ref(dir, &format!("{INDEX_REFS}/{id}"), &c)?;
    }
    Ok(Checkpoint { ref_name, commit, ignored_ref, skipped: cap.skipped, index_tree })
}

pub struct Rollback {
    pub interrupted_ref: String,
    pub interrupted_ignored_ref: Option<String>,
    pub restored: usize,
    pub removed: usize,
    pub ignored_restored: usize,
    /// Ignored files rollback did not touch: restoring them would overwrite bytes this attempt did not capture.
    pub not_restored: Vec<String>,
    /// Ignored files the checkpoint itself skipped (over the cap) that are missing or changed now.
    pub not_protected: Vec<String>,
}

fn safe_rel(p: &str) -> Result<PathBuf, String> {
    let path = Path::new(p);
    if path.components().all(|c| matches!(c, Component::Normal(_))) {
        Ok(path.to_path_buf())
    } else {
        Err(format!("refusing unsafe path from git: {p}"))
    }
}

/// One line of `diff-tree --raw -z`: what it takes to go from the first tree to the second.
struct Change {
    status: char,
    #[cfg_attr(not(unix), allow(dead_code))] // only unix sets the executable bit
    mode: String,
    sha: String,
    path: String,
}

fn changes(dir: &Path, from: &str, to: &str) -> Result<Vec<Change>, String> {
    let out = git(dir, &["diff-tree", "-r", "--no-renames", "--raw", "-z", from, to], None, None)?;
    let out = String::from_utf8_lossy(&out).into_owned();
    let mut toks = out.split('\0').filter(|t| !t.is_empty());
    let mut v = Vec::new();
    while let (Some(meta), Some(path)) = (toks.next(), toks.next()) {
        let f: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        if f.len() < 5 {
            return Err(format!("unexpected diff-tree line: {meta}"));
        }
        v.push(Change { status: f[4].chars().next().unwrap_or('M'), mode: f[1].to_string(), sha: f[3].to_string(), path: path.to_string() });
    }
    Ok(v)
}

fn write_blob(dir: &Path, c: &Change) -> Result<(), String> {
    let rel = safe_rel(&c.path)?;
    let full = dir.join(&rel);
    let bytes = git(dir, &["cat-file", "blob", &c.sha], None, None)?;
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(&full, bytes).map_err(|e| format!("cannot write {}: {e}", full.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if c.mode == "100755" { 0o755 } else { 0o644 };
        let _ = std::fs::set_permissions(&full, std::fs::Permissions::from_mode(mode));
    }
    Ok(())
}

/// Roll a project's working tree back to its checkpoint.
/// 1. Snapshot the CURRENT state (raw bytes, ignored files too) into brand-new refs
///    `interrupted/<id>/<attempt>`; every attempt gets a new ref, created only if absent and read back
///    before any file is touched. What to delete and restore is computed from this attempt's snapshot,
///    so edits made after an earlier failed attempt are kept too.
/// 2. Files that exist now but not in the checkpoint (new non-ignored files) are deleted; they are
///    saved. Files that differ from, or are missing since, the checkpoint are restored byte for byte.
/// 3. Ignored files in the checkpoint that changed or were deleted are restored; ignored files the job
///    created are left in place (and are in the snapshot).
/// 4. The real index goes back to what it was at the checkpoint (or to HEAD if none was recorded).
///
/// HEAD and the branch are never touched. Empty folders are neither restored nor removed.
#[allow(clippy::too_many_arguments)] // each argument is one recorded fact about the job; a struct would only rename them
pub fn rollback(
    dir: &Path,
    state: &Path,
    id: &str,
    checkpoint_ref: &str,
    checkpoint_ignored_ref: Option<&str>,
    checkpoint_skipped: &[(String, u64)],
    index_tree: Option<&str>,
    opts: &Opts,
) -> Result<Rollback, String> {
    let cap = capture(dir, state, &format!("{id}-int"), opts)?;
    let parent = head(dir);
    let ignored_commit = match &cap.ignored_tree {
        Some(t) => Some(commit_tree(dir, t, None, &format!("broadsword interrupted ignored files of {id}"))?),
        None => None,
    };
    let commit = commit_tree(dir, &cap.tree, parent.as_deref(), &format!("broadsword interrupted work of {id}"))?;
    let mut attempt = 0u32;
    let (interrupted_ref, interrupted_ignored_ref) = loop {
        attempt += 1;
        let r = format!("{INTERRUPTED_REFS}/{id}/{attempt}");
        match create_ref(dir, &r, &commit) {
            Ok(()) => {
                let ri = match &ignored_commit {
                    Some(c) => {
                        let ri = format!("{INTERRUPTED_IGNORED_REFS}/{id}/{attempt}");
                        create_ref(dir, &ri, c)?;
                        Some(ri)
                    }
                    None => None,
                };
                break (r, ri);
            }
            Err(e) if ref_exists(dir, &r) && attempt < 10_000 => {
                let _ = e; // an earlier attempt owns this ref: try the next number
            }
            Err(e) => return Err(e),
        }
    };

    let apply = || -> Result<Rollback, String> {
    let ckpt_tree = git_text(dir, &["rev-parse", "--verify", &format!("{checkpoint_ref}^{{tree}}")], None)?;
    let main = changes(dir, &cap.tree, &ckpt_tree)?;
    let ignored = match (checkpoint_ignored_ref, &cap.ignored_tree) {
        (Some(r), now) => {
            let ckpt_ign = git_text(dir, &["rev-parse", "--verify", &format!("{r}^{{tree}}")], None)?;
            let now = match now {
                Some(t) => t.clone(),
                None => git_text(dir, &["mktree"], None)?, // empty tree: everything in the checkpoint is "missing"
            };
            changes(dir, &now, &ckpt_ign)?
        }
        (None, _) => Vec::new(),
    };
    for c in main.iter().chain(&ignored) {
        safe_rel(&c.path)?; // validate everything before touching a single file
    }

    let mut removed = 0;
    for c in main.iter().filter(|c| c.status == 'D') {
        let full = dir.join(safe_rel(&c.path)?);
        match std::fs::remove_file(&full) {
            Ok(()) => removed += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("cannot remove {}: {e}", full.display())),
        }
        // Folders the removal left empty go too (deepest first, never recursive: `remove_dir` only
        // succeeds on an empty folder), so a file the checkpoint had can take the place of a folder.
        let mut parent = full.parent();
        while let Some(d) = parent {
            if d == dir || std::fs::remove_dir(d).is_err() {
                break;
            }
            parent = d.parent();
        }
    }
    let mut restored = 0;
    for c in main.iter().filter(|c| c.status != 'D') {
        write_blob(dir, c)?;
        restored += 1;
    }
    // Never write over a path whose current bytes are not in THIS attempt's snapshot (the size cap can leave one out).
    let uncaptured: std::collections::HashSet<&str> = cap.skipped.iter().map(|(p, _)| p.as_str()).collect();
    let (mut ignored_restored, mut not_restored) = (0, Vec::new());
    for c in ignored.iter().filter(|c| c.status != 'D') {
        if uncaptured.contains(c.path.as_str()) {
            not_restored.push(c.path.clone());
            continue;
        }
        write_blob(dir, c)?;
        ignored_restored += 1;
    }
    let mut not_protected = Vec::new();
    for (p, len) in checkpoint_skipped {
        match std::fs::metadata(dir.join(p)) {
            Err(_) => not_protected.push(format!("{p} (missing now)")),
            Ok(m) if m.len() != *len => not_protected.push(format!("{p} (size changed from {len} to {} bytes)", m.len())),
            Ok(_) => {}
        }
    }
    match index_tree {
        Some(t) => git_text(dir, &["read-tree", t], None)?,
        None => git_text(dir, &["reset", "-q"], None)?,
    };
    Ok(Rollback { interrupted_ref: interrupted_ref.clone(), interrupted_ignored_ref: interrupted_ignored_ref.clone(), restored, removed, ignored_restored, not_restored, not_protected })
    };
    apply().map_err(|e| {
        format!("{e}. The work as it was before this attempt is saved in {interrupted_ref}; steps before the failure may already have been applied")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    pub fn opts() -> Opts {
        Opts { excludes: vec![PathBuf::from("state")], ignored_max_bytes: 50 * MB }
    }

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
        fs::write(d.join(".gitignore"), "ignored.txt\n.env*\n*.log\ntarget/\n").unwrap();
        git_text(&d, &["add", "-A"], None).unwrap();
        git_text(&d, &["commit", "-q", "-m", "init"], None).unwrap();
        d
    }

    fn state_for(tag: &str) -> PathBuf {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        scratch.join(format!("state-{tag}-{}", std::process::id()))
    }

    fn status(d: &Path) -> String {
        git_text(d, &["status", "--porcelain"], None).unwrap()
    }

    fn show(d: &Path, r: &str, p: &str) -> String {
        git_text(d, &["show", &format!("{r}:{p}")], None).unwrap_or_else(|e| format!("<{e}>"))
    }

    #[test]
    fn checkpoint_captures_changes_without_touching_anything() {
        let d = temp_repo("ckpt");
        fs::write(d.join("tracked.txt"), "modified").unwrap();
        fs::write(d.join("new.txt"), "untracked").unwrap();
        fs::write(d.join("ignored.txt"), "ignored").unwrap();
        fs::create_dir_all(d.join("target")).unwrap();
        fs::write(d.join("target").join("big.o"), "build output").unwrap();
        let (head_before, status_before) = (head(&d), status(&d));
        let state = state_for("ckpt");
        let c = checkpoint(&d, &state, "job1", &opts()).unwrap();
        assert_eq!(c.ref_name, "refs/broadsword/checkpoints/job1");
        assert_eq!(head(&d), head_before);
        assert_eq!(status(&d), status_before);
        assert_eq!(fs::read_to_string(d.join("tracked.txt")).unwrap(), "modified");
        assert!(fs::read_dir(&state).unwrap().next().is_none(), "temp indexes removed");
        assert_eq!(show(&d, &c.ref_name, "tracked.txt"), "modified");
        assert_eq!(show(&d, &c.ref_name, "new.txt"), "untracked");
        assert!(show(&d, &c.ref_name, "ignored.txt").starts_with('<'), "ignored files are not in the main snapshot");
        let ir = c.ignored_ref.unwrap();
        assert_eq!(show(&d, &ir, "ignored.txt"), "ignored");
        assert!(show(&d, &ir, "target/big.o").starts_with('<'), "target folders are never captured");
        assert_eq!(git_text(&d, &["rev-parse", &format!("{}^", c.ref_name)], None).ok(), head_before);
        // a ref that exists is never overwritten
        assert!(checkpoint(&d, &state, "job1", &opts()).is_err());
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn ignored_size_cap_skips_and_lists() {
        let d = temp_repo("cap");
        fs::write(d.join("a.log"), vec![b'a'; 600]).unwrap();
        fs::write(d.join("b.log"), vec![b'b'; 600]).unwrap();
        let state = state_for("cap");
        let o = Opts { excludes: vec![], ignored_max_bytes: 1000 };
        let c = checkpoint(&d, &state, "j", &o).unwrap();
        assert_eq!(c.skipped, [("b.log".to_string(), 600)]);
        assert_eq!(show(&d, &c.ignored_ref.unwrap(), "a.log").len(), 600);
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn checkpoint_works_in_a_repo_without_commits() {
        let d = temp_repo("empty");
        fs::remove_dir_all(d.join(".git")).unwrap();
        git_text(&d, &["init", "-q"], None).unwrap();
        let state = state_for("empty");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        assert_eq!(show(&d, &c.ref_name, "tracked.txt"), "one");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rollback_restores_and_keeps_partial_work() {
        let d = temp_repo("rb");
        fs::write(d.join("tracked.txt"), "before-job").unwrap(); // uncommitted state at checkpoint
        let state = state_for("rb");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        fs::write(d.join("tracked.txt"), "during-job").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join("made.txt"), "made").unwrap();
        fs::write(d.join("ignored.txt"), "created by job").unwrap();
        let head_before = head(&d);
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(fs::read_to_string(d.join("tracked.txt")).unwrap(), "before-job");
        assert!(!d.join("sub").join("made.txt").exists());
        assert_eq!(fs::read_to_string(d.join("ignored.txt")).unwrap(), "created by job", "ignored files the job created stay");
        assert_eq!(head(&d), head_before);
        assert_eq!((r.restored, r.removed), (1, 1));
        assert_eq!(r.interrupted_ref, "refs/broadsword/interrupted/j/1");
        assert_eq!(show(&d, &r.interrupted_ref, "tracked.txt"), "during-job");
        assert_eq!(show(&d, &r.interrupted_ref, "sub/made.txt"), "made");
        assert_eq!(show(&d, &r.interrupted_ignored_ref.unwrap(), "ignored.txt"), "created by job");
        assert_eq!(status(&d), "M tracked.txt"); // index matches HEAD again: only the unstaged change shows
        // a second rollback is a new attempt with its own ref
        let r2 = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(r2.interrupted_ref, "refs/broadsword/interrupted/j/2");
        assert_eq!(show(&d, "refs/broadsword/interrupted/j/1", "sub/made.txt"), "made");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression (blocker 1): a retried rollback must not overwrite edits made after a failed attempt.
    #[cfg(windows)]
    #[test]
    fn retry_after_failed_rollback_keeps_later_edits() {
        use std::os::windows::fs::OpenOptionsExt;
        let d = temp_repo("retry");
        let state = state_for("retry");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        fs::write(d.join("tracked.txt"), "during-job").unwrap();
        fs::write(d.join("made.txt"), "made").unwrap();
        // lock the file the job added: it can be read but not deleted
        let lock = fs::OpenOptions::new().read(true).share_mode(1).open(d.join("made.txt")).unwrap();
        let first = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts());
        assert!(first.is_err(), "rollback must fail while the file is locked");
        // the user edits a file after the failed attempt, then the lock goes away
        fs::write(d.join("tracked.txt"), "user edit after failure").unwrap();
        drop(lock);
        let second = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(fs::read_to_string(d.join("tracked.txt")).unwrap(), "one");
        assert_eq!(show(&d, &second.interrupted_ref, "tracked.txt"), "user edit after failure", "the edit is saved in this attempt's snapshot");
        assert_ne!(second.interrupted_ref, "refs/broadsword/interrupted/j/1");
        assert_eq!(show(&d, "refs/broadsword/interrupted/j/1", "tracked.txt"), "during-job");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression (blocker 2): bytes come back exactly, even with core.autocrlf=true and text=auto.
    #[test]
    fn rollback_is_byte_exact_under_autocrlf() {
        let d = temp_repo("crlf");
        git_text(&d, &["config", "core.autocrlf", "true"], None).unwrap();
        fs::write(d.join(".gitattributes"), "* text=auto\n").unwrap();
        let originals: [(&str, &[u8]); 4] = [("lf.txt", b"a\nb\nc\n"), ("crlf.txt", b"a\r\nb\r\nc\r\n"), ("mixed.txt", b"a\nb\r\nc\n\r\nd"), ("bin.dat", b"\x00\r\n\x01\n\xff")];
        for (n, b) in originals {
            fs::write(d.join(n), b).unwrap();
        }
        git_text(&d, &["add", "-A"], None).unwrap();
        git_text(&d, &["commit", "-q", "-m", "files"], None).unwrap();
        // the state at checkpoint time: raw bytes written straight to disk (differs from what Git would check out)
        for (n, b) in originals {
            fs::write(d.join(n), b).unwrap();
        }
        fs::write(d.join("untracked-lf.txt"), b"x\ny\n").unwrap();
        let state = state_for("crlf");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        for (n, _) in originals {
            fs::write(d.join(n), b"job changed this\n").unwrap();
        }
        fs::write(d.join("untracked-lf.txt"), b"changed\r\n").unwrap();
        rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        for (n, b) in originals {
            assert_eq!(fs::read(d.join(n)).unwrap(), b, "{n}");
        }
        assert_eq!(fs::read(d.join("untracked-lf.txt")).unwrap(), b"x\ny\n");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression (cap): rollback must never write over an ignored file whose current bytes were not captured.
    #[test]
    fn cap_never_lets_rollback_overwrite_an_uncaptured_file() {
        let d = temp_repo("capover");
        fs::write(d.join(".env"), "SECRET=original").unwrap();
        fs::write(d.join("big.log"), vec![b'x'; 500]).unwrap(); // 515 bytes in total: fits the checkpoint's cap
        let o = Opts { excludes: vec![], ignored_max_bytes: 1000 };
        let state = state_for("capover");
        let c = checkpoint(&d, &state, "j", &o).unwrap();
        assert!(c.skipped.is_empty());
        // the job adds an ignored file that sorts first and nearly fills the cap
        fs::write(d.join(".aaa.log"), vec![b'y'; 990]).unwrap();
        fs::write(d.join(".env"), "SECRET=changed by job").unwrap();
        // the interrupted snapshot now cannot hold .env; the user edits it after the kill
        fs::write(d.join(".env"), "USER EDIT, in no snapshot").unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &o).unwrap();
        assert_eq!(fs::read_to_string(d.join(".env")).unwrap(), "USER EDIT, in no snapshot", "uncaptured bytes must never be overwritten");
        assert!(r.not_restored.contains(&".env".to_string()), "{:?}", r.not_restored);
        assert_eq!(fs::read(d.join(".aaa.log")).unwrap().len(), 990, "ignored files the job created stay");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// The checkpoint's own skipped files are reported when they are missing or changed at rollback time.
    #[test]
    fn rollback_reports_files_the_checkpoint_could_not_protect() {
        let d = temp_repo("unprot");
        fs::write(d.join("a.log"), vec![b'a'; 600]).unwrap();
        fs::write(d.join("b.log"), vec![b'b'; 600]).unwrap();
        fs::write(d.join("c.log"), vec![b'c'; 10]).unwrap();
        let o = Opts { excludes: vec![], ignored_max_bytes: 1000 };
        let state = state_for("unprot");
        let c = checkpoint(&d, &state, "j", &o).unwrap();
        assert_eq!(c.skipped.len(), 1); // b.log
        fs::remove_file(d.join("b.log")).unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &o).unwrap();
        assert_eq!(r.not_protected, ["b.log (missing now)"]);
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression: a file replaced by a folder (and other emptied folders) must not wedge the rollback.
    #[test]
    fn file_replaced_by_directory_can_be_rolled_back() {
        let d = temp_repo("filedir");
        fs::write(d.join("st.txt"), "file").unwrap();
        let state = state_for("filedir");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        fs::remove_file(d.join("st.txt")).unwrap();
        fs::create_dir_all(d.join("st.txt")).unwrap();
        fs::write(d.join("st.txt").join("inner.txt"), "inner").unwrap();
        fs::create_dir_all(d.join("deep").join("er")).unwrap();
        fs::write(d.join("deep").join("er").join("x.txt"), "x").unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(fs::read_to_string(d.join("st.txt")).unwrap(), "file");
        assert!(!d.join("deep").exists(), "folders emptied by the rollback are removed");
        assert_eq!(show(&d, &r.interrupted_ref, "st.txt/inner.txt"), "inner");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression (blocker 3): ignored files (`.env`) the job deleted or changed come back.
    #[test]
    fn ignored_files_deleted_or_changed_by_the_job_are_restored() {
        let d = temp_repo("ign");
        fs::write(d.join(".env"), "SECRET=1\r\nB=2\n").unwrap();
        fs::write(d.join(".env2"), "OTHER=1").unwrap();
        let state = state_for("ign");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        fs::write(d.join(".env"), "SECRET=changed").unwrap();
        fs::remove_file(d.join(".env2")).unwrap();
        fs::write(d.join("made.log"), "log made by job").unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(fs::read(d.join(".env")).unwrap(), b"SECRET=1\r\nB=2\n");
        assert_eq!(fs::read(d.join(".env2")).unwrap(), b"OTHER=1");
        assert_eq!(fs::read_to_string(d.join("made.log")).unwrap(), "log made by job", "ignored files the job created are never deleted");
        assert_eq!(r.ignored_restored, 2);
        assert_eq!(show(&d, &r.interrupted_ignored_ref.unwrap(), ".env"), "SECRET=changed");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }
}
