//! Git checkpoints and rollback. Everything runs the `git` program as a child process.
//!
//! A snapshot of a project's working tree (tracked and untracked non-ignored files) is kept TWICE, built in one place
//! (`capture`):
//! - the snapshot COMMIT holds what `git add` of those files would store under the repo's own filters (core.autocrlf,
//!   `.gitattributes`): `update-index --add` on a copy of the real index, so its tree is the tree a normal commit of the
//!   same files would have, and `git diff <base> <checkpoint>` shows only real changes;
//! - a sibling `-raw` commit holds the exact bytes (`hash-object -w --no-filters`, trees built with
//!   `update-index --index-info` in a temporary index), so no line-ending or other filter ever touches them. Restoring
//!   writes these raw blobs back, never the filtered ones (a clean filter is not always undone by its smudge: CRLF and LF
//!   mixed in one file come back all CRLF).
//!
//! Ignored files (minus build, scratch, state and `target` folders, and subject to a size cap) are one more, parentless,
//! raw commit (`git add` never stores them). The working tree, HEAD, the branch and the real index are never changed by
//! taking a snapshot.
//!
//! Refs (a ref cannot be both a file and a folder, so the ignored and raw snapshots live in sibling folders):
//! - `refs/broadsword/checkpoints/<id>`, `refs/broadsword/checkpoints-raw/<id>` and `refs/broadsword/checkpoints-ignored/<id>`
//! - `refs/broadsword/interrupted/<id>/<attempt>`, `refs/broadsword/interrupted-raw/<id>/<attempt>` and
//!   `refs/broadsword/interrupted-ignored/<id>/<attempt>`
//! - a checkpoint commit made before the raw refs existed holds the raw bytes itself; one made since names its raw ref in
//!   its message (`RAW_MARK`), and rollback refuses when that ref is missing
//! - `refs/broadsword/index/<id>`: the real index at checkpoint time, minus secret paths
//! - `refs/broadsword/interrupted-index/<id>/<attempt>`: the real index just before a rollback replaced it, minus secret paths
//!
//! Secret files (see `is_secret_path`) do not exist for any of this: they are never read, stored, restored,
//! overwritten or deleted, by a snapshot or by a rollback. That includes the index: a secret file someone staged is left
//! out of every stored index tree, and a rollback leaves the index entries of secret paths exactly as they are.
//!
//! Refs are only ever created (never overwritten or deleted), so no snapshot is ever lost.

use crate::config::{Config, MB};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

pub const CHECKPOINT_REFS: &str = "refs/broadsword/checkpoints";
pub const CHECKPOINT_IGNORED_REFS: &str = "refs/broadsword/checkpoints-ignored";
pub const CHECKPOINT_RAW_REFS: &str = "refs/broadsword/checkpoints-raw";
pub const INTERRUPTED_RAW_REFS: &str = "refs/broadsword/interrupted-raw";
/// The line of a snapshot commit's message that names the ref holding its exact bytes.
const RAW_MARK: &str = "exact bytes: ";
pub const INTERRUPTED_REFS: &str = "refs/broadsword/interrupted";
pub const INTERRUPTED_IGNORED_REFS: &str = "refs/broadsword/interrupted-ignored";
pub const INDEX_REFS: &str = "refs/broadsword/index";
pub const INTERRUPTED_INDEX_REFS: &str = "refs/broadsword/interrupted-index";

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

/// The secret NAMES (Lazlo's set, 2026-11-19): exactly `.env` or `.envrc` (direnv), `.env.` followed by a name
/// (`.env.local`), or anything starting with `.env-` or `.env_`, the bare `.env-` and `.env_` included (`.env-prod`,
/// `.env_staging`). Letters compare case-insensitively (ASCII) on every
/// platform (Windows file names are, and a `.ENV` is a secret wherever it is), after trailing dots and spaces are dropped
/// (Windows drops them, so `.env ` and `.env.` are the file `.env`). A name is cut at every `:` and each piece is judged,
/// so an NTFS stream (`.env:s`, `.env::$DATA`) and a drive-relative `C:.env` are secrets.
/// Other names that merely start with `.env` (`.envoy.yaml`, `.environment`) are NOT secret.
fn is_secret_name(name: &str) -> bool {
    name.split(':').any(|piece| {
        let n = piece.trim_end_matches([' ', '.']).to_ascii_lowercase();
        // (a bare `.env.` is already `.env` here: the trailing dot was dropped)
        n == ".env" || n == ".envrc" || (n.len() > ".env.".len() && n.starts_with(".env.")) || n.starts_with(".env-") || n.starts_with(".env_")
    })
}

/// Shaped like an NTFS 8.3 short name: `<1-6 chars>~<digits>[.<1-3 chars>]` (`ENV~1`, `ENV~1.LOC`, `ENVOY~1.YAM`),
/// after trailing dots and spaces are dropped.
fn looks_like_short_name(name: &str) -> bool {
    let name = name.trim_end_matches([' ', '.']); // Windows drops these: `ENV~1.` is `ENV~1`
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) => (s, Some(e)),
        None => (name, None),
    };
    if ext.is_some_and(|e| e.is_empty() || e.chars().count() > 3) {
        return false;
    }
    match stem.rsplit_once('~') {
        Some((pre, num)) => (1..=6).contains(&pre.chars().count()) && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// `p` as an absolute `\\?\` path (`\\?\UNC\` for a share), NUL-terminated, so a Win32 call is not cut at MAX_PATH.
/// `std::path::absolute` (GetFullPathNameW) first resolves `.`/`..`, turns `/` into `\` and drops trailing dots and spaces,
/// which a `\\?\` path would otherwise keep literally.
#[cfg(windows)]
fn verbatim_wide(p: &Path) -> Option<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let abs: Vec<u16> = std::path::absolute(p).ok()?.as_os_str().encode_wide().collect();
    let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let mut out = if abs.starts_with(&w(r"\\?\")) || abs.starts_with(&w(r"\\.\")) {
        abs
    } else if abs.starts_with(&w(r"\\")) {
        [w(r"\\?\UNC\"), abs[2..].to_vec()].concat()
    } else {
        [w(r"\\?\"), abs].concat()
    };
    out.push(0);
    Some(out)
}

/// The long (real) name of the last part of `p` as the file system has it, if `p` exists and it can be resolved. On
/// Windows this is `GetLongPathNameW` on the `\\?\` form of the path (any length), which turns an 8.3 alias (`ENV~1`, or a
/// short name an admin set, like `KEYS.TXT`) into the name it stands for (`.env`) and does not follow links.
#[cfg(windows)]
fn long_name(p: &Path) -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetLongPathNameW(short: *const u16, long: *mut u16, len: u32) -> u32;
    }
    let wide = verbatim_wide(p)?;
    let mut buf = vec![0u16; 512];
    loop {
        // SAFETY: `wide` is NUL-terminated and `buf` holds `buf.len()` u16s; the call writes at most that many.
        let n = unsafe { GetLongPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
        if n == 0 {
            return None; // missing, or cannot be resolved
        }
        if n < buf.len() {
            buf.truncate(n);
            break;
        }
        if n > 32_768 + 8 {
            return None; // more than any Windows path can hold: never trusted
        }
        buf.resize(n + 1, 0); // too small: `n` is the size needed (with its NUL); retry with that size
    }
    let long = OsString::from_wide(&buf).to_string_lossy().into_owned();
    long.rsplit(['/', '\\']).next().map(str::to_string)
}

#[cfg(not(windows))]
fn long_name(p: &Path) -> Option<String> {
    std::fs::symlink_metadata(p).ok().and(p.file_name()).map(|n| n.to_string_lossy().into_owned())
}

/// THE one rule for what a secret file is: `path` (relative to `base`; `/` and a backslash separate folders) is a secret
/// when the last part's name is a secret name (`is_secret_name`), OR, when it exists on disk, the long name the file system
/// has for it is one (so `ENV~1`, the 8.3 alias of `.env`, is a secret), OR it exists but its long name cannot be resolved
/// (fail closed), OR it is shaped like an 8.3 alias (`looks_like_short_name`) and does not exist (strict: a missing alias
/// may stand for a secret). A FOLDER named `.env` (a Python venv) does not make its files secret.
///
/// Every place that captures, deletes or lists files calls this function: checkpoint capture (`capture`), the stored index
/// trees (`secret_free_tree`, and the index entries rollback keeps), rollback's deletes (`changes`, `Guard::remove`) and,
/// through `is_secret_path`, the queue's `--brief` reader (`queue.rs`). Rollback's RESTORES use
/// `is_secret_to_restore`: the same rule except the missing-alias clause.
pub fn is_secret_in(base: &Path, path: &str) -> bool {
    secret_check(base, path, Missing::Strict)
}

/// The rule for a path rollback would WRITE from a checkpoint tree: exactly `is_secret_in`, except that a path that does
/// not exist now is not a secret just because its name is shaped like an 8.3 alias. Such a path passed `is_secret_in` when
/// the checkpoint captured it (it existed then and resolved to an ordinary long name), so an ordinary `report~1.txt` the
/// job deleted is restored. The name rule and the long-name rule for a file that exists now still apply in full: rollback
/// never writes through a name that resolves to a secret, and never writes a secret name.
fn is_secret_to_restore(base: &Path, path: &str) -> bool {
    secret_check(base, path, Missing::Captured)
}

/// How `secret_check` judges a path that is missing and cannot be resolved.
#[derive(Clone, Copy, PartialEq)]
enum Missing {
    /// Secret if shaped like an 8.3 alias (it may stand for a secret).
    Strict,
    /// Not a secret: the path comes from a checkpoint tree, which only holds paths that passed `Strict` when captured.
    Captured,
}

fn secret_check(base: &Path, path: &str, missing: Missing) -> bool {
    let (parent, name) = match path.rfind(['/', '\\']) {
        Some(i) => (&path[..i], &path[i + 1..]),
        None => ("", path),
    };
    if is_secret_name(name) {
        return true;
    }
    let file = name.split(':').next().unwrap_or("");
    if file.is_empty() {
        return false;
    }
    let on_disk = if parent.is_empty() { base.join(file) } else { base.join(parent).join(file) };
    match long_name(&on_disk) {
        Some(long) => is_secret_name(&long),
        // Cannot resolve: a file that exists is a secret (its real name is unknown); a missing one only if alias-shaped
        // and judged strictly.
        None => std::fs::symlink_metadata(&on_disk).is_ok() || (missing == Missing::Strict && name.split(':').any(looks_like_short_name)),
    }
}

/// `is_secret_in` for a path relative to the current folder (or absolute).
pub fn is_secret_path(path: &str) -> bool {
    is_secret_in(Path::new(""), path)
}


/// A project is a Git repo of its own when it has its own `.git` (folder, or file for a linked worktree).
pub fn is_repo(dir: &Path) -> bool {
    dir.join(".git").exists()
}

fn git(dir: &Path, args: &[&str], index: Option<&Path>, input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    let mut c = crate::scrub::command_in(dir, "git")?; // the scrubbed environment (scrub.rs); refuses when it cannot be known
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

/// Publish a snapshot: the exact bytes (`raw_tree`, a parentless commit) at `raw_ref` FIRST, then the `git add` tree at
/// `snapshot_ref`, whose message names `raw_ref` (`RAW_MARK`). Every checkpoint and interrupted snapshot goes through here,
/// so a snapshot ref only ever exists once its raw ref was created by this same call: if creating the raw ref fails (or
/// the process dies there), no snapshot ref is made. Returns the snapshot commit.
fn publish_snapshot(
    dir: &Path,
    snapshot_ref: &str,
    raw_ref: &str,
    tree: &str,
    raw_tree: &str,
    parent: Option<&str>,
    title: &str,
) -> Result<String, String> {
    let raw_commit = commit_tree(dir, raw_tree, None, &format!("{title}, exact bytes"))?;
    create_ref(dir, raw_ref, &raw_commit)?;
    let commit = commit_tree(dir, tree, parent, &format!("{title}\n\n{RAW_MARK}{raw_ref}"))?;
    create_ref(dir, snapshot_ref, &commit)?;
    Ok(commit)
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

/// NUL-terminated paths, the `-z --stdin` input of `update-index`.
fn nul_list<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<u8> {
    let mut v = Vec::new();
    for p in paths {
        v.extend_from_slice(p.as_bytes());
        v.push(0);
    }
    v
}

/// The tree `git add` of exactly these files would give, the real index left unchanged: their blobs go through the repo's
/// own filters (core.autocrlf, `.gitattributes`) judged against a COPY of the real index, as `git add` judges them (a path
/// whose index blob already holds CRs keeps them; plain `hash-object` does not read the index and would convert it).
/// Entries of the copy outside `files` (deleted files, secret paths) are removed by name: no file is read for them.
/// `core.safecrlf=false` only silences git's line-ending warning or error; it changes no content.
fn staged_tree_of(dir: &Path, state: &Path, tag: &str, files: &[String]) -> Result<String, String> {
    let idx = unique_index(state, tag);
    let result = (|| {
        let real = PathBuf::from(git_text(dir, &["rev-parse", "--git-path", "index"], None)?);
        let real = if real.is_absolute() { real } else { dir.join(real) };
        match std::fs::copy(&real, &idx) {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {} // no index yet: start from an empty one
            Err(e) => return Err(format!("cannot copy the index {}: {e}", real.display())),
        }
        let keep: HashSet<&str> = files.iter().map(String::as_str).collect();
        let present = String::from_utf8_lossy(&git(dir, &["ls-files", "-z"], Some(&idx), None)?).into_owned();
        let gone: Vec<&str> = present.split('\0').filter(|p| !p.is_empty() && !keep.contains(p)).collect();
        if !gone.is_empty() {
            git(dir, &["update-index", "--force-remove", "-z", "--stdin"], Some(&idx), Some(&nul_list(gone.into_iter())))?;
        }
        if !files.is_empty() {
            let input = nul_list(files.iter().map(String::as_str));
            git(dir, &["-c", "core.safecrlf=false", "update-index", "--add", "-z", "--stdin"], Some(&idx), Some(&input))?;
        }
        git_text(dir, &["write-tree"], Some(&idx))
    })();
    let _ = std::fs::remove_file(&idx);
    result
}

/// The tree that holds a snapshot commit's exact bytes: the tree of the raw ref its message names (`RAW_MARK`), which must
/// exist then, or (a commit made before raw refs existed, which stored raw bytes itself) the commit's own tree.
fn raw_tree_of(dir: &Path, commit_ref: &str) -> Result<String, String> {
    let msg = git_text(dir, &["log", "-1", "--format=%B", commit_ref, "--"], None)?;
    match msg.lines().find_map(|l| l.strip_prefix(RAW_MARK)) {
        Some(raw) => git_text(dir, &["rev-parse", "--verify", &format!("{}^{{tree}}", raw.trim())], None)
            .map_err(|e| format!("{commit_ref} names its exact bytes in {}, which cannot be read: {e}", raw.trim())),
        None => git_text(dir, &["rev-parse", "--verify", &format!("{commit_ref}^{{tree}}")], None),
    }
}

/// NUL-separated `<meta>\t<path>` records of `ls-tree -z` / `ls-files -s -z`, raw (a path is never re-encoded), with the
/// path decoded only to test it.
fn entries(out: &[u8]) -> Vec<(&[u8], String)> {
    out.split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .map(|e| {
            let path = e.iter().position(|b| *b == b'\t').map(|t| &e[t + 1..]).unwrap_or(b"");
            (e, String::from_utf8_lossy(path).into_owned())
        })
        .collect()
}

/// The ONE place an index tree is made fit to store or to restore: `tree` with every secret path (`is_secret_path`)
/// left out, plus `extra` (raw `--index-info` records, kept as given), and the secret paths that were left out.
/// When nothing is left out and nothing added, `tree` itself comes back and nothing is written.
fn secret_free_tree(dir: &Path, state: &Path, tag: &str, tree: &str, extra: &[Vec<u8>]) -> Result<(String, Vec<String>), String> {
    let out = git(dir, &["ls-tree", "-r", "-z", "--full-tree", tree], None, None)?;
    let all = entries(&out);
    let secret: Vec<bool> = all.iter().map(|(_, p)| is_secret_in(dir, p)).collect();
    let dropped: Vec<String> = all.iter().zip(&secret).filter(|(_, s)| **s).map(|((_, p), _)| p.clone()).collect();
    if dropped.is_empty() && extra.is_empty() {
        return Ok((tree.to_string(), dropped));
    }
    let mut input = Vec::new();
    for raw in all.iter().zip(&secret).filter(|(_, s)| !**s).map(|((r, _), _)| *r).chain(extra.iter().map(Vec::as_slice)) {
        input.extend_from_slice(raw); // `ls-tree` lines are a format `--index-info` reads as they are
        input.push(0);
    }
    std::fs::create_dir_all(state).map_err(|e| format!("cannot create {}: {e}", state.display()))?;
    let idx = unique_index(state, tag);
    let result = (|| {
        if !input.is_empty() {
            git(dir, &["update-index", "-z", "--index-info"], Some(&idx), Some(&input))?;
        }
        git_text(dir, &["write-tree"], Some(&idx))
    })();
    let _ = std::fs::remove_file(&idx);
    Ok((result?, dropped))
}

pub struct Captured {
    /// What `git add` of the files would store (`staged_tree_of`): the snapshot commit's tree.
    pub tree: String,
    /// The same files' exact bytes (`tree_of`): what rollback compares and restores.
    pub raw_tree: String,
    pub ignored_tree: Option<String>,
    /// Ignored files left out because the size cap was reached.
    pub skipped: Vec<(String, u64)>,
    /// Secret files left out (names only, never content); no object was ever made from them.
    pub secret_skipped: Vec<String>,
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
    // Secret files are dropped here, before any blob is written: nothing below ever sees their bytes.
    let mut secret_skipped: Vec<String> = Vec::new();
    files.retain(|p| {
        let secret = is_secret_in(dir, p);
        if secret {
            secret_skipped.push(p.clone());
        }
        !secret
    });
    let raw_tree = tree_of(dir, state, tag, &files, &modes)?;
    let tree = staged_tree_of(dir, state, &format!("{tag}-add"), &files)?;

    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    let mut used = 0u64;
    for p in list_files(dir, &["ls-files", "-z", "-o", "-i", "--exclude-standard"])? {
        if excluded(&p, opts) || !is_regular(dir, &p) {
            continue;
        }
        if is_secret_in(dir, &p) {
            if !secret_skipped.contains(&p) {
                secret_skipped.push(p);
            }
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
    secret_skipped.sort();
    Ok(Captured { tree, raw_tree, ignored_tree, skipped, secret_skipped })
}

pub struct Checkpoint {
    pub ref_name: String,
    pub commit: String,
    pub ignored_ref: Option<String>,
    pub skipped: Vec<(String, u64)>,
    /// Secret files not captured, by name.
    pub secret_skipped: Vec<String>,
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
    // The exact bytes first (publish_snapshot): a checkpoint ref that exists always has its raw ref (rollback refuses without it).
    let raw_ref = format!("{CHECKPOINT_RAW_REFS}/{id}");
    let ref_name = format!("{CHECKPOINT_REFS}/{id}");
    let title = format!("broadsword checkpoint {id}");
    let commit = publish_snapshot(dir, &ref_name, &raw_ref, &cap.tree, &cap.raw_tree, parent.as_deref(), &title)?;
    // `write-tree` only writes tree objects; it does not modify the index. It fails on unmerged entries: then none is kept.
    // A secret file someone staged is left out of the stored tree (and listed with the other secret files by name).
    let mut secret_skipped = cap.secret_skipped;
    let index_tree = match git_text(dir, &["write-tree"], None).ok().filter(|t| !t.is_empty()) {
        Some(t) => {
            let (t, dropped) = secret_free_tree(dir, state, &format!("{id}-idx"), &t, &[])?;
            secret_skipped.extend(dropped);
            secret_skipped.sort();
            secret_skipped.dedup();
            let c = commit_tree(dir, &t, parent.as_deref(), &format!("broadsword index at checkpoint {id}"))?;
            create_ref(dir, &format!("{INDEX_REFS}/{id}"), &c)?;
            Some(t)
        }
        None => None,
    };
    Ok(Checkpoint { ref_name, commit, ignored_ref, skipped: cap.skipped, secret_skipped, index_tree })
}

pub struct Rollback {
    pub interrupted_ref: String,
    pub interrupted_ignored_ref: Option<String>,
    /// The real index as it was just before rollback replaced it.
    pub interrupted_index_ref: Option<String>,
    pub restored: usize,
    pub removed: usize,
    pub ignored_restored: usize,
    /// Ignored files rollback did not touch: restoring them would overwrite bytes this attempt did not capture.
    pub not_restored: Vec<String>,
    /// Ignored files the checkpoint itself skipped (over the cap) that are missing or changed now.
    pub not_protected: Vec<String>,
    /// Paths rollback would have restored or deleted but left exactly as they are because they are (or now name) a
    /// secret file. Never silent: `recover` prints each one.
    pub secret_skipped: Vec<String>,
    /// Secret paths whose index entries rollback left exactly as they are (staged now, or in the checkpoint's index tree
    /// and so never restored from it). Never silent: `recover` prints each one.
    pub index_secrets: Vec<String>,
}

fn safe_rel(p: &str) -> Result<PathBuf, String> {
    let path = Path::new(p);
    // A `:` would name an NTFS stream or a drive (`.env:s`, `C:x`): never a plain file inside the project.
    if path.components().all(|c| matches!(c, Component::Normal(n) if !n.to_string_lossy().contains(':'))) {
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

/// A path that is (or now names) a secret is left out and added to `secret_skipped`: a delete (`D`: the path exists now)
/// is judged by `is_secret_in`, a restore from the checkpoint tree by `is_secret_to_restore`.
fn changes(dir: &Path, from: &str, to: &str, secret_skipped: &mut Vec<String>) -> Result<Vec<Change>, String> {
    let out = git(dir, &["diff-tree", "-r", "--no-renames", "--raw", "-z", from, to], None, None)?;
    let out = String::from_utf8_lossy(&out).into_owned();
    let mut toks = out.split('\0').filter(|t| !t.is_empty());
    let mut v = Vec::new();
    while let (Some(meta), Some(path)) = (toks.next(), toks.next()) {
        let f: Vec<&str> = meta.trim_start_matches(':').split(' ').collect();
        if f.len() < 5 {
            return Err(format!("unexpected diff-tree line: {meta}"));
        }
        let status = f[4].chars().next().unwrap_or('M');
        let secret = if status == 'D' { is_secret_in(dir, path) } else { is_secret_to_restore(dir, path) };
        if secret {
            // never restored or deleted (a checkpoint made before this rule may hold one, or the name now resolves to one)
            if !secret_skipped.iter().any(|s| s == path) {
                secret_skipped.push(path.to_string());
            }
            continue;
        }
        v.push(Change { status, mode: f[1].to_string(), sha: f[3].to_string(), path: path.to_string() });
    }
    Ok(v)
}

/// path -> blob id of every file in a tree.
fn tree_blobs(dir: &Path, tree: &str) -> Result<HashMap<String, String>, String> {
    let out = git(dir, &["ls-tree", "-r", "-z", tree], None, None)?;
    let mut m = HashMap::new();
    for entry in String::from_utf8_lossy(&out).split('\0').filter(|e| !e.is_empty()) {
        if let Some((meta, path)) = entry.split_once('\t') {
            if let Some(sha) = meta.split(' ').nth(2) {
                m.insert(path.to_string(), sha.to_string());
            }
        }
    }
    Ok(m)
}

/// The ONE place where rollback changes the working tree. Every file write, file delete and folder
/// delete goes through it. Before acting on a path it checks this attempt's snapshots (tracked,
/// untracked and ignored together): the action is allowed only if the path does not exist on disk, or
/// the file's bytes right now (hashed raw) equal the snapshot's blob for that path, so nothing whose
/// bytes are in no ref is ever overwritten or deleted. Folders are only removed when empty.
/// Anything else is skipped and listed in `skipped`.
struct Guard<'a> {
    dir: &'a Path,
    snapshot: HashMap<String, String>,
    skipped: Vec<String>,
    /// Paths left alone because they are (or now name) a secret file.
    secret_skipped: Vec<String>,
}

impl Guard<'_> {
    /// May the path be changed? Err = could not tell (never treated as "yes").
    fn may_change(&self, rel: &str) -> Result<bool, String> {
        let full = self.dir.join(safe_rel(rel)?);
        match std::fs::symlink_metadata(&full) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(e) => Err(format!("cannot inspect {}: {e}", full.display())),
            Ok(m) if m.is_file() => {
                let now = git_text(self.dir, &["hash-object", "--no-filters", "--", rel], None)?;
                Ok(self.snapshot.get(rel) == Some(&now))
            }
            Ok(_) => Ok(false), // a folder or something else we never captured
        }
    }

    fn secret_skip(&mut self, rel: &str) {
        if !self.secret_skipped.iter().any(|s| s == rel) {
            self.secret_skipped.push(rel.to_string());
        }
    }

    fn skip(&mut self, rel: &str) {
        if !self.skipped.iter().any(|s| s == rel) {
            self.skipped.push(rel.to_string());
        }
    }

    /// Write the blob of `c` to its path. Ok(true) = written, Ok(false) = skipped.
    fn write(&mut self, c: &Change) -> Result<bool, String> {
        if is_secret_to_restore(self.dir, &c.path) {
            self.secret_skip(&c.path); // never written
            return Ok(false);
        }
        if !self.may_change(&c.path)? {
            self.skip(&c.path);
            return Ok(false);
        }
        let full = self.dir.join(safe_rel(&c.path)?);
        let bytes = git(self.dir, &["cat-file", "blob", &c.sha], None, None)?;
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
        Ok(true)
    }

    /// Delete a file the job added (its bytes are in the snapshot), then the folders that leaves empty.
    fn remove(&mut self, rel: &str) -> Result<bool, String> {
        if is_secret_in(self.dir, rel) {
            self.secret_skip(rel); // never deleted
            return Ok(false);
        }
        let full = self.dir.join(safe_rel(rel)?);
        if full.symlink_metadata().is_err() {
            return Ok(false); // already gone
        }
        if !self.may_change(rel)? {
            self.skip(rel);
            return Ok(false);
        }
        std::fs::remove_file(&full).map_err(|e| format!("cannot remove {}: {e}", full.display()))?;
        let mut parent = full.parent();
        while let Some(d) = parent {
            if d == self.dir || !self.remove_empty_dir(d) {
                break;
            }
            parent = d.parent();
        }
        Ok(true)
    }

    /// Remove a folder only if it is empty (never recursive).
    fn remove_empty_dir(&self, d: &Path) -> bool {
        std::fs::read_dir(d).map(|mut e| e.next().is_none()).unwrap_or(false) && std::fs::remove_dir(d).is_ok()
    }
}

/// Roll a project's working tree back to its checkpoint.
/// 1. Snapshot the CURRENT state (ignored files too) into brand-new refs `interrupted/<id>/<attempt>` (what `git add`
///    would store) and `interrupted-raw/<id>/<attempt>` (the exact bytes); every attempt gets new refs, created only if
///    absent and read back before any file is touched. Every comparison and restore below uses the exact bytes: this
///    attempt's raw tree and the checkpoint's (`raw_tree_of`). What to delete and restore is computed from this attempt's snapshot,
///    so edits made after an earlier failed attempt are kept too.
/// 2. Files that exist now but not in the checkpoint (new non-ignored files) are deleted; they are
///    saved. Files that differ from, or are missing since, the checkpoint are restored byte for byte.
/// 3. Ignored files in the checkpoint that changed or were deleted are restored; ignored files the job
///    created are left in place (and are in the snapshot).
/// 4. The real index goes back to what it was at the checkpoint (or to HEAD if none was recorded), except that the
///    entries of secret paths stay as they are now (see `is_secret_path`).
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
    let title = format!("broadsword interrupted work of {id}");
    let mut attempt = 0u32;
    let (interrupted_ref, interrupted_ignored_ref) = loop {
        attempt += 1;
        let r = format!("{INTERRUPTED_REFS}/{id}/{attempt}");
        let raw_ref = format!("{INTERRUPTED_RAW_REFS}/{id}/{attempt}");
        // raw ref first, then the snapshot ref (publish_snapshot), both before any file is touched
        match publish_snapshot(dir, &r, &raw_ref, &cap.tree, &cap.raw_tree, parent.as_deref(), &title) {
            Ok(_) => {
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
            Err(e) if (ref_exists(dir, &raw_ref) || ref_exists(dir, &r)) && attempt < 10_000 => {
                let _ = e; // an earlier attempt owns this number (its raw ref or its snapshot ref): try the next one
            }
            Err(e) => return Err(e),
        }
    };

    let apply = || -> Result<Rollback, String> {
    // Rollback compares and restores EXACT bytes only: the checkpoint's raw tree against this attempt's raw tree.
    let ckpt_tree = raw_tree_of(dir, checkpoint_ref)?;
    // Every secret file in the folder right now (this attempt's capture lists them by name only, before any blob is made:
    // tracked, untracked or ignored, the job's new ones included) is left exactly as it is, and named.
    let mut secret_skipped = cap.secret_skipped.clone();
    let main = changes(dir, &cap.raw_tree, &ckpt_tree, &mut secret_skipped)?;
    let ignored = match (checkpoint_ignored_ref, &cap.ignored_tree) {
        (Some(r), now) => {
            let ckpt_ign = git_text(dir, &["rev-parse", "--verify", &format!("{r}^{{tree}}")], None)?;
            let now = match now {
                Some(t) => t.clone(),
                None => git_text(dir, &["mktree"], None)?, // empty tree: everything in the checkpoint is "missing"
            };
            changes(dir, &now, &ckpt_ign, &mut secret_skipped)?
        }
        (None, _) => Vec::new(),
    };
    for c in main.iter().chain(&ignored) {
        safe_rel(&c.path)?; // validate everything before touching a single file
    }

    // Every change goes through the Guard; see its documentation.
    let mut snapshot = tree_blobs(dir, &cap.raw_tree)?;
    if let Some(t) = &cap.ignored_tree {
        snapshot.extend(tree_blobs(dir, t)?);
    }
    let mut guard = Guard { dir, snapshot, skipped: Vec::new(), secret_skipped };
    let (mut removed, mut restored, mut ignored_restored) = (0, 0, 0);
    for c in main.iter().filter(|c| c.status == 'D') {
        removed += usize::from(guard.remove(&c.path)?);
    }
    for c in main.iter().filter(|c| c.status != 'D') {
        restored += usize::from(guard.write(c)?);
    }
    for c in ignored.iter().filter(|c| c.status != 'D') {
        ignored_restored += usize::from(guard.write(c)?);
    }
    let not_restored = std::mem::take(&mut guard.skipped);
    let mut not_protected = Vec::new();
    for (p, len) in checkpoint_skipped {
        match std::fs::metadata(dir.join(p)) {
            Err(_) => not_protected.push(format!("{p} (missing now)")),
            Ok(m) if m.len() != *len => not_protected.push(format!("{p} (size changed from {len} to {} bytes)", m.len())),
            Ok(_) => {}
        }
    }
    // The real index follows the same rule as the Guard: it is only replaced after its current content
    // is saved. Staged content from after the kill goes to `interrupted-index/<id>/<attempt>` first; an
    // index with unmerged entries cannot be saved, so it is left completely alone and reported.
    let mut not_restored = not_restored;
    let mut interrupted_index_ref = None;
    let mut index_secrets: Vec<String> = Vec::new();
    match git_text(dir, &["write-tree"], None) {
        Ok(tree) => {
            let (tree, _) = secret_free_tree(dir, state, &format!("{id}-int-idx"), &tree, &[])?;
            let c = commit_tree(dir, &tree, parent.as_deref(), &format!("broadsword interrupted index of {id}"))?;
            let r = format!("{INTERRUPTED_INDEX_REFS}/{id}/{attempt}");
            create_ref(dir, &r, &c)?;
            interrupted_index_ref = Some(r);
            // The index goes back to the checkpoint's (or to HEAD's when none was recorded), except for secret paths:
            // their entries are left exactly as they are now (kept if staged, absent if not), never taken from the
            // checkpoint. A checkpoint index tree made before this rule may hold one: it is dropped here.
            let base = match (index_tree, &parent) {
                (Some(t), _) => t.to_string(),
                (None, Some(h)) => git_text(dir, &["rev-parse", "--verify", &format!("{h}^{{tree}}")], None)?,
                (None, None) => git_text(dir, &["mktree"], None)?, // no commit yet: the empty tree
            };
            let now = git(dir, &["ls-files", "-s", "-z"], None, None)?;
            let mut keep: Vec<Vec<u8>> = Vec::new();
            for (raw, p) in entries(&now).into_iter().filter(|(_, p)| is_secret_in(dir, p)) {
                keep.push(raw.to_vec());
                index_secrets.push(p);
            }
            let (restore, dropped) = secret_free_tree(dir, state, &format!("{id}-rst-idx"), &base, &keep)?;
            git_text(dir, &["read-tree", &restore], None)?;
            index_secrets.extend(dropped);
            index_secrets.sort();
            index_secrets.dedup();
        }
        Err(e) => not_restored.push(format!("(the git index: it has unmerged entries and cannot be saved, so it was left untouched: {e})")),
    }
    Ok(Rollback {
        interrupted_ref: interrupted_ref.clone(),
        interrupted_ignored_ref: interrupted_ignored_ref.clone(),
        interrupted_index_ref,
        restored,
        removed,
        ignored_restored,
        not_restored,
        not_protected,
        secret_skipped: std::mem::take(&mut guard.secret_skipped),
        index_secrets,
    })
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

    /// A throwaway repo under the repo's scratch folder with one commit: tracked.txt = "one". It is `repo/` inside a
    /// `scrub::planted::config_root`, so git (started through `scrub::command_in`) loads this build's own settings, never
    /// a main checkout's that may lack a key this branch adds.
    pub fn temp_repo(tag: &str) -> PathBuf {
        let d = crate::scrub::planted::config_root(&format!("git-{tag}")).join("repo");
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

    /// The git child (`git()`, every git.rs call) sees no planted secret: an alias that runs `env` dumps git's own environment.
    #[test]
    fn git_child_sees_no_planted_secret() {
        let root = crate::scrub::planted::root("gitrs");
        let repo = root.join("repo");
        fs::create_dir_all(&repo).unwrap();
        git_text(&repo, &["init", "-q"], None).unwrap();
        let out = git_text(&repo, &["-c", "alias.envdump=!env", "envdump"], None).unwrap();
        crate::scrub::planted::assert_clean("git.rs git()", &out);
        let _ = fs::remove_dir_all(&root);
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

    /// An autocrlf=true repo whose working files are CRLF (as a checkout leaves them), with `lf.txt` committed as an LF blob
    /// (`git add` converts it), `crlfblob.txt` committed as a CRLF blob (`git add` keeps its CRs: git's safe-autocrlf rule,
    /// which only an index-aware capture gets right) and two untracked files, one with mixed endings.
    fn crlf_working_tree(tag: &str) -> PathBuf {
        let d = temp_repo(tag);
        fs::write(d.join("lf.txt"), b"x\ny\n").unwrap();
        fs::write(d.join("crlfblob.txt"), b"a\r\nb\r\n").unwrap();
        git_text(&d, &["add", "-A"], None).unwrap();
        git_text(&d, &["commit", "-q", "-m", "files"], None).unwrap();
        git_text(&d, &["config", "core.autocrlf", "true"], None).unwrap();
        fs::write(d.join("lf.txt"), b"x\r\ny\r\nz\r\n").unwrap();
        fs::write(d.join("crlfblob.txt"), b"a\r\nb\r\nc\r\n").unwrap();
        fs::write(d.join("new.txt"), b"n\r\n").unwrap();
        fs::write(d.join("mixed.txt"), b"m\r\nn\no\r\n").unwrap();
        d
    }

    /// The tree `git add -A` + commit would give right now, made in a copy of the index (the real one is not changed).
    fn add_all_tree(d: &Path, state: &Path) -> String {
        fs::create_dir_all(state).unwrap();
        let idx = state.join("expected-index");
        fs::copy(d.join(".git").join("index"), &idx).unwrap();
        git(d, &["add", "-A"], Some(&idx), None).unwrap();
        let t = git_text(d, &["write-tree"], Some(&idx)).unwrap();
        fs::remove_file(&idx).unwrap();
        t
    }

    fn tree(d: &Path, r: &str) -> String {
        git_text(d, &["rev-parse", &format!("{r}^{{tree}}")], None).unwrap()
    }

    fn blob(d: &Path, r: &str, p: &str) -> Vec<u8> {
        git(d, &["cat-file", "blob", &format!("{r}:{p}")], None, None).unwrap()
    }

    /// A checkpoint of a CRLF working tree under core.autocrlf=true has the tree a normal commit of it would have (crash-sim
    /// 4.6b: the raw checkpoint a819254 differed from its parent in every CRLF file).
    #[test]
    fn checkpoint_tree_equals_a_commit_under_autocrlf() {
        let d = crlf_working_tree("ckadd");
        let state = state_for("ckadd");
        let status_before = status(&d);
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        assert_eq!(status(&d), status_before, "the real index is not changed");
        let expected = add_all_tree(&d, &state);
        assert_eq!(tree(&d, &c.ref_name), expected, "checkpoint tree == tree of git add -A");
        assert_eq!(blob(&d, &c.ref_name, "lf.txt"), b"x\ny\nz\n", "converted like git add");
        assert_eq!(blob(&d, &c.ref_name, "crlfblob.txt"), b"a\r\nb\r\nc\r\n", "a CRLF index blob keeps its CRs, like git add");
        // the exact bytes are kept beside it
        let raw = format!("{CHECKPOINT_RAW_REFS}/j");
        assert_eq!(blob(&d, &raw, "lf.txt"), b"x\r\ny\r\nz\r\n");
        assert_eq!(blob(&d, &raw, "mixed.txt"), b"m\r\nn\no\r\n");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Rollback from such a checkpoint writes back the exact bytes (not the converted blobs), and the interrupted snapshot is
    /// likewise a git-add tree with the job's exact bytes in its raw ref.
    #[test]
    fn rollback_restores_exact_bytes_from_a_filtered_checkpoint() {
        let d = crlf_working_tree("rbadd");
        let state = state_for("rbadd");
        let files = ["tracked.txt", "lf.txt", "crlfblob.txt", "new.txt", "mixed.txt"];
        let before: Vec<Vec<u8>> = files.iter().map(|f| fs::read(d.join(f)).unwrap()).collect();
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        for f in files {
            fs::write(d.join(f), b"job\r\nchanged\n").unwrap();
        }
        fs::write(d.join("made.txt"), b"made\r\n").unwrap();
        let expected_int = add_all_tree(&d, &state);
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        for (f, b) in files.iter().zip(&before) {
            assert_eq!(&fs::read(d.join(f)).unwrap(), b, "{f} restored byte for byte");
        }
        assert!(!d.join("made.txt").exists());
        assert_eq!(tree(&d, &r.interrupted_ref), expected_int, "interrupted tree == tree of git add -A");
        let raw = format!("{INTERRUPTED_RAW_REFS}/j/1");
        assert_eq!(blob(&d, &raw, "lf.txt"), b"job\r\nchanged\n");
        assert_eq!(blob(&d, &raw, "made.txt"), b"made\r\n");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Raw before snapshot (review q-20261123-184131): when the interrupted raw ref cannot be created, rollback stops with no
    /// interrupted snapshot ref and no file touched. A ref `interrupted-raw/j` (a file where the folder must be) makes
    /// `interrupted-raw/j/<n>` impossible to create for every attempt.
    #[test]
    fn rollback_never_publishes_a_snapshot_without_its_raw_ref() {
        let d = crlf_working_tree("rbrawfirst");
        let state = state_for("rbrawfirst");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        assert!(ref_exists(&d, &format!("{CHECKPOINT_RAW_REFS}/j")), "the checkpoint has its raw ref");
        fs::write(d.join("lf.txt"), b"job\r\n").unwrap();
        let blocker = format!("{INTERRUPTED_RAW_REFS}/j");
        create_ref(&d, &blocker, &c.commit).unwrap();
        let e = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).err().expect("refused");
        assert!(!ref_exists(&d, &format!("{INTERRUPTED_REFS}/j/1")), "no interrupted snapshot without its raw ref: {e}");
        assert_eq!(fs::read(d.join("lf.txt")).unwrap(), b"job\r\n", "nothing restored");
        // and a stale raw ref left by an earlier attempt only moves this attempt to the next number
        git_text(&d, &["update-ref", "-d", &blocker], None).unwrap();
        create_ref(&d, &format!("{INTERRUPTED_RAW_REFS}/j/1"), &c.commit).unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(r.interrupted_ref, format!("{INTERRUPTED_REFS}/j/2"));
        assert_eq!(blob(&d, &format!("{INTERRUPTED_RAW_REFS}/j/2"), "lf.txt"), b"job\r\n");
        assert_eq!(raw_tree_of(&d, &r.interrupted_ref).unwrap(), tree(&d, &format!("{INTERRUPTED_RAW_REFS}/j/2")));
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// A checkpoint whose message names a raw ref that is missing is never restored from its filtered tree.
    #[test]
    fn rollback_refuses_when_the_named_raw_ref_is_missing() {
        let d = crlf_working_tree("rbnoraw");
        let state = state_for("rbnoraw");
        let cap = capture(&d, &state, "x", &opts()).unwrap();
        let msg = format!("broadsword checkpoint x\n\n{RAW_MARK}{CHECKPOINT_RAW_REFS}/x");
        let commit = commit_tree(&d, &cap.tree, head(&d).as_deref(), &msg).unwrap();
        create_ref(&d, &format!("{CHECKPOINT_REFS}/x"), &commit).unwrap();
        fs::write(d.join("lf.txt"), b"job\n").unwrap();
        let e = rollback(&d, &state, "x", &format!("{CHECKPOINT_REFS}/x"), None, &[], None, &opts()).err().expect("refused");
        assert!(e.contains("cannot be read"), "{e}");
        assert_eq!(fs::read(d.join("lf.txt")).unwrap(), b"job\n", "nothing restored");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression (cap): rollback must never write over an ignored file whose current bytes were not captured.
    #[test]
    fn cap_never_lets_rollback_overwrite_an_uncaptured_file() {
        let d = temp_repo("capover");
        fs::write(d.join("keep.log"), "original").unwrap();
        fs::write(d.join("big.log"), vec![b'x'; 500]).unwrap(); // 508 bytes in total: fits the checkpoint's cap
        let o = Opts { excludes: vec![], ignored_max_bytes: 1000 };
        let state = state_for("capover");
        let c = checkpoint(&d, &state, "j", &o).unwrap();
        assert!(c.skipped.is_empty());
        // the job adds an ignored file that sorts first and nearly fills the cap
        fs::write(d.join(".aaa.log"), vec![b'y'; 990]).unwrap();
        fs::write(d.join("keep.log"), "changed by job").unwrap();
        // the interrupted snapshot now cannot hold keep.log; the user edits it after the kill
        fs::write(d.join("keep.log"), "USER EDIT, in no snapshot").unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &o).unwrap();
        assert_eq!(fs::read_to_string(d.join("keep.log")).unwrap(), "USER EDIT, in no snapshot", "uncaptured bytes must never be overwritten");
        assert!(r.not_restored.contains(&"keep.log".to_string()), "{:?}", r.not_restored);
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

    /// Regression (reviewer's repro): a tracked/untracked file that the job turned into an ignored one,
    /// pushed over the cap by a big ignored file, must not be overwritten by the restore.
    #[test]
    fn untracked_file_made_ignored_by_the_job_is_not_overwritten() {
        let d = temp_repo("notes");
        fs::write(d.join("notes.txt"), "orig notes").unwrap(); // untracked, in the checkpoint's main tree
        let o = Opts { excludes: vec![], ignored_max_bytes: MB };
        let state = state_for("notes");
        let c = checkpoint(&d, &state, "j", &o).unwrap();
        // the job ignores notes.txt and writes a ~1 MB ignored file that sorts before it
        let gi = fs::read_to_string(d.join(".gitignore")).unwrap();
        fs::write(d.join(".gitignore"), format!("{gi}notes.txt\n")).unwrap();
        fs::write(d.join("-dump.log"), vec![b'd'; 1_048_570]).unwrap();
        fs::write(d.join("notes.txt"), "USER EDIT after the kill").unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &o).unwrap();
        assert_eq!(fs::read_to_string(d.join("notes.txt")).unwrap(), "USER EDIT after the kill", "bytes in no ref are never overwritten");
        assert_eq!(r.not_restored, ["notes.txt"]);
        assert_eq!(fs::read_to_string(d.join(".gitignore")).unwrap(), gi, "captured files are still restored");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// The chokepoint itself: every write and delete is checked against this attempt's snapshot.
    #[test]
    fn guard_allows_only_missing_or_snapshotted_paths() {
        let d = temp_repo("guard");
        let blob = |bytes: &[u8]| String::from_utf8_lossy(&git(&d, &["hash-object", "-w", "--no-filters", "--stdin"], None, Some(bytes)).unwrap()).trim().to_string();
        let change = |p: &str, sha: &str| Change { status: 'M', mode: "100644".into(), sha: sha.into(), path: p.into() };
        let target = blob(b"checkpoint bytes");
        fs::write(d.join("uncaptured.txt"), "in no snapshot").unwrap();
        fs::write(d.join("captured.txt"), "as snapshotted").unwrap();
        fs::write(d.join("changed.txt"), "edited after the snapshot").unwrap();
        fs::create_dir_all(d.join("dir")).unwrap();
        fs::write(d.join("dir").join("f.txt"), "x").unwrap();
        let mut snapshot = HashMap::new();
        snapshot.insert("captured.txt".to_string(), blob(b"as snapshotted"));
        snapshot.insert("changed.txt".to_string(), blob(b"as it was at snapshot time"));
        let mut g = Guard { dir: &d, snapshot, skipped: Vec::new(), secret_skipped: Vec::new() };
        // refuses an existing path that is not in the snapshot
        assert!(!g.write(&change("uncaptured.txt", &target)).unwrap());
        assert!(!g.remove("uncaptured.txt").unwrap());
        assert_eq!(fs::read_to_string(d.join("uncaptured.txt")).unwrap(), "in no snapshot");
        // allows a path that does not exist
        assert!(g.write(&change("missing/new.txt", &target)).unwrap());
        assert_eq!(fs::read_to_string(d.join("missing").join("new.txt")).unwrap(), "checkpoint bytes");
        // allows a snapshotted path whose bytes still match
        assert!(g.write(&change("captured.txt", &target)).unwrap());
        assert_eq!(fs::read_to_string(d.join("captured.txt")).unwrap(), "checkpoint bytes");
        // refuses a snapshotted path whose bytes changed since the snapshot (edit between snapshot and restore)
        assert!(!g.write(&change("changed.txt", &target)).unwrap());
        assert!(!g.remove("changed.txt").unwrap());
        assert_eq!(fs::read_to_string(d.join("changed.txt")).unwrap(), "edited after the snapshot");
        // never writes onto a folder, and only removes empty folders
        assert!(!g.write(&change("dir", &target)).unwrap());
        assert!(!g.remove_empty_dir(&d.join("dir")), "non-empty folder");
        fs::remove_file(d.join("dir").join("f.txt")).unwrap();
        assert!(g.remove_empty_dir(&d.join("dir")));
        assert_eq!(g.skipped, ["uncaptured.txt", "changed.txt", "dir"]);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression: content staged after the kill is saved before the real index is restored.
    #[test]
    fn staged_content_after_the_kill_is_saved_before_the_index_is_restored() {
        let d = temp_repo("idx");
        let state = state_for("idx");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        fs::write(d.join("m.rs"), "fn staged_after_the_kill() {}").unwrap();
        git_text(&d, &["add", "m.rs"], None).unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        let ir = r.interrupted_index_ref.expect("index ref");
        assert_eq!(ir, "refs/broadsword/interrupted-index/j/1");
        assert_eq!(show(&d, &ir, "m.rs"), "fn staged_after_the_kill() {}", "the staged blob is reachable");
        assert!(!git_text(&d, &["ls-files", "--stage"], None).unwrap().contains("m.rs"), "index restored to the checkpoint state");
        assert!(r.not_restored.is_empty());
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// An index with unmerged entries cannot be saved: it is left untouched and reported.
    #[test]
    fn unmerged_index_is_left_untouched_and_reported() {
        let d = temp_repo("unmerged");
        let state = state_for("unmerged");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        let blob = |b: &[u8]| String::from_utf8_lossy(&git(&d, &["hash-object", "-w", "--stdin"], None, Some(b)).unwrap()).trim().to_string();
        let (a, b) = (blob(b"ours"), blob(b"theirs"));
        let entries = format!("0 {z}\ttracked.txt\n100644 {a} 2\ttracked.txt\n100644 {b} 3\ttracked.txt\n", z = "0".repeat(40));
        git(&d, &["update-index", "--index-info"], None, Some(entries.as_bytes())).unwrap();
        assert!(git_text(&d, &["write-tree"], None).is_err(), "the index really is unmerged");
        let before = git_text(&d, &["ls-files", "--stage"], None).unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(git_text(&d, &["ls-files", "--stage"], None).unwrap(), before, "index untouched");
        assert!(r.interrupted_index_ref.is_none());
        assert!(r.not_restored.iter().any(|p| p.contains("git index")), "{:?}", r.not_restored);
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Regression (blocker 3): ignored files the job deleted or changed come back.
    #[test]
    fn ignored_files_deleted_or_changed_by_the_job_are_restored() {
        let d = temp_repo("ign");
        fs::write(d.join("a.log"), "A=1\r\nB=2\n").unwrap();
        fs::write(d.join("ignored.txt"), "OTHER=1").unwrap();
        let state = state_for("ign");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        fs::write(d.join("a.log"), "changed").unwrap();
        fs::remove_file(d.join("ignored.txt")).unwrap();
        fs::write(d.join("made.log"), "log made by job").unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(fs::read(d.join("a.log")).unwrap(), b"A=1\r\nB=2\n");
        assert_eq!(fs::read(d.join("ignored.txt")).unwrap(), b"OTHER=1");
        assert_eq!(fs::read_to_string(d.join("made.log")).unwrap(), "log made by job", "ignored files the job created are never deleted");
        assert_eq!(r.ignored_restored, 2);
        assert_eq!(show(&d, &r.interrupted_ignored_ref.unwrap(), "a.log"), "changed");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_secret_file_rule() {
        for yes in [
            ".env", ".env.local", "sub/.env", "a\\b\\.ENV", ".Env.production", "x/y/.env.example", ".ENV.Local", ".env.", ".env ", "a/.env.x.y",
            ".envrc", "sub/.ENVRC", ".envrc. ", ".env-prod", ".ENV-Staging", "a\\.env_local", ".env_x",
            ".env-", ".env_", ".ENV-", "x/.env_. ",
        ] {
            assert!(is_secret_path(yes), "{yes}");
        }
        for no in [
            "env.txt", "my.env", "a.env/file.txt", "environment", "src/env.rs", "", ".envoy.yaml", ".environment", ".ENVIRONMENT", ".env/key.txt", ".env.d/x",
            "x.env.local", ".envrc2", "envrc", ".env-/x",
        ] {
            assert!(!is_secret_path(no), "{no}");
        }
        // NTFS streams and drive-relative names: every `:` piece is judged
        for yes in [".env:s", ".env::$DATA", ".ENV.local:x:$DATA", "C:.env", "sub/.envrc:s", "a.txt:.env"] {
            assert!(is_secret_path(yes), "{yes}");
        }
        assert!(!is_secret_path("tracked-nowhere.txt:s"));
        // 8.3-alias shapes that do not exist here: strict, they may stand for a secret
        let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("no-such-folder-for-the-rule-test");
        for yes in ["ENV~1", "ENV~1.LOC", "ENVOY~1.YAM", "sub/ABCDEF~12", "env~1:s", "ENV~1. "] {
            assert!(is_secret_in(&missing, yes), "{yes}");
        }
        for no in ["ENV~", "~1", "ABCDEFG~1", "ENV~1.LOCA", "ENV~X", "notes~old.txt"] {
            assert!(!looks_like_short_name(no), "{no}");
            assert!(!is_secret_in(&missing, no), "{no}");
        }
        // a path with a `:` is never written or deleted by rollback
        for bad in [".env:s", "sub/a:b", "C:x"] {
            assert!(safe_rel(bad).is_err(), "{bad}");
        }
        assert!(safe_rel("sub/ok.txt").is_ok());
        // restoring from a checkpoint: a missing alias-shaped name was captured as ordinary, so it is restored; the name
        // rule still holds
        for p in ["report~1.txt", "sub/draft~2.doc", "ENV~1"] {
            assert!(is_secret_in(&missing, p) && !is_secret_to_restore(&missing, p), "{p}");
        }
        for p in [".env", "sub/.env.local", ".envrc:s"] {
            assert!(is_secret_to_restore(&missing, p), "{p}");
        }
    }

    /// Past MAX_PATH the long name is still resolved (the `\\?\` form), so an ordinary deep file is NOT made a secret by the
    /// fail-closed branch, and a deep `.env` is found under its 8.3 alias.
    #[cfg(windows)]
    #[test]
    fn long_names_resolve_past_max_path() {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        let base = scratch.join(format!("git-deep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let deep_rel = vec!["e".repeat(60); 5].join("/");
        let deep = base.join(&deep_rel);
        fs::create_dir_all(&deep).unwrap();
        fs::write(deep.join("ordinary.txt"), "x").unwrap();
        fs::write(deep.join(".env"), "SECRET=deep").unwrap();
        assert!(deep.join("ordinary.txt").display().to_string().len() > 300);
        assert_eq!(long_name(&deep.join("ordinary.txt")).as_deref(), Some("ordinary.txt"), "resolved, not given up on");
        assert!(!is_secret_in(&base, &format!("{deep_rel}/ordinary.txt")));
        match long_name(&deep.join("ENV~1")) {
            Some(long) => assert_eq!(long, ".env"),
            None => {
                use std::io::Write;
                let _ = writeln!(std::io::stderr(), "SKIPPED long_names_resolve_past_max_path (alias part): the volume gives no 8.3 names");
            }
        }
        assert!(is_secret_in(&base, &format!("{deep_rel}/ENV~1")), "an alias is a secret whether or not it resolves");
        fs::remove_dir_all(&base).unwrap();
    }

    /// Paths in a tree, recursively.
    fn tree_paths(d: &Path, r: &str) -> Vec<String> {
        git_text(d, &["ls-tree", "-r", "--name-only", "--full-tree", r], None).unwrap().lines().map(str::to_string).collect()
    }

    /// A secret file someone staged is never stored in `refs/broadsword/index/<id>`; the real index is not changed.
    #[test]
    fn a_staged_secret_file_is_left_out_of_the_stored_index_tree() {
        let d = temp_repo("secret-idx");
        fs::write(d.join(".env"), "SECRET=staged-aaa").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join(".env.local"), "SECRET=staged-bbb").unwrap();
        fs::write(d.join(".envoy.yaml"), "envoy: staged").unwrap();
        fs::write(d.join("staged.txt"), "ordinary staged").unwrap();
        git_text(&d, &["add", "-f", ".env", "sub/.env.local", ".envoy.yaml", "staged.txt"], None).unwrap();
        let index_before = git_text(&d, &["ls-files", "--stage"], None).unwrap();
        let state = state_for("secret-idx");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        let paths = tree_paths(&d, "refs/broadsword/index/j");
        assert!(!paths.iter().any(|p| is_secret_path(p)), "{paths:?}");
        for p in [".envoy.yaml", "staged.txt", "tracked.txt"] {
            assert!(paths.contains(&p.to_string()), "{p} is kept: {paths:?}");
        }
        assert_eq!(c.index_tree.as_deref(), Some(git_text(&d, &["rev-parse", "refs/broadsword/index/j^{tree}"], None).unwrap().as_str()));
        assert_eq!(c.secret_skipped, [".env", "sub/.env.local"], "listed by name once each");
        assert_eq!(git_text(&d, &["ls-files", "--stage"], None).unwrap(), index_before, "the real index is not changed");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Rollback: the saved interrupted index holds no secret path; the restored index takes everything from the checkpoint
    /// except secret entries, which stay as they are now; look-alike names (`.envoy.yaml`, `.environment`) roll back.
    #[test]
    fn rollback_keeps_secret_index_entries_out_of_refs_and_as_they_are() {
        let d = temp_repo("secret-rbidx");
        fs::write(d.join(".envoy.yaml"), "envoy: original").unwrap();
        fs::write(d.join(".environment"), "ENVIRONMENT=original").unwrap();
        fs::write(d.join(".env"), "SECRET=at-checkpoint").unwrap();
        git_text(&d, &["add", "-f", ".env"], None).unwrap();
        let state = state_for("secret-rbidx");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        // the job changes the look-alikes, unstages .env, and stages a new secret and an ordinary file
        fs::write(d.join(".envoy.yaml"), "envoy: job").unwrap();
        fs::remove_file(d.join(".environment")).unwrap();
        git_text(&d, &["rm", "-q", "--cached", ".env"], None).unwrap();
        fs::write(d.join(".env.prod"), "SECRET=staged-by-job").unwrap();
        fs::write(d.join("job.txt"), "job").unwrap();
        git_text(&d, &["add", "-f", ".env.prod", "job.txt"], None).unwrap();
        let staged_by_job = git_text(&d, &["ls-files", "--stage", ".env.prod"], None).unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(fs::read(d.join(".envoy.yaml")).unwrap(), b"envoy: original");
        assert_eq!(fs::read(d.join(".environment")).unwrap(), b"ENVIRONMENT=original");
        assert_eq!(fs::read(d.join(".env.prod")).unwrap(), b"SECRET=staged-by-job", "secret files are never touched");
        let int = tree_paths(&d, &r.interrupted_index_ref.expect("index saved"));
        assert!(int.contains(&"job.txt".to_string()), "{int:?}");
        assert!(!int.iter().any(|p| is_secret_path(p)), "the saved index holds no secret path: {int:?}");
        let staged: Vec<String> = git_text(&d, &["ls-files"], None).unwrap().lines().map(str::to_string).collect();
        assert!(!staged.contains(&"job.txt".to_string()), "ordinary entries go back to the checkpoint: {staged:?}");
        assert!(!staged.contains(&".env".to_string()), "a secret entry is not taken from the checkpoint: {staged:?}");
        assert_eq!(git_text(&d, &["ls-files", "--stage", ".env.prod"], None).unwrap(), staged_by_job, "a staged secret entry stays as it is");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// An index tree stored before this rule may hold a secret entry: a rollback never restores it into the index.
    #[test]
    fn rollback_drops_a_secret_entry_from_an_old_index_tree() {
        let d = temp_repo("secret-oldidx");
        let state = state_for("secret-oldidx");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        let blob = String::from_utf8_lossy(&git(&d, &["hash-object", "-w", "--no-filters", "--stdin"], None, Some(b"SECRET=old-index")).unwrap()).trim().to_string();
        let mut input = git(&d, &["ls-tree", "-z", c.index_tree.as_deref().unwrap()], None, None).unwrap();
        input.extend_from_slice(format!("100644 blob {blob}\t.env.local").as_bytes());
        input.push(0);
        let old = String::from_utf8_lossy(&git(&d, &["mktree", "-z"], None, Some(&input)).unwrap()).trim().to_string();
        assert!(tree_paths(&d, &old).contains(&".env.local".to_string()));
        rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, Some(&old), &opts()).unwrap();
        let staged = git_text(&d, &["ls-files"], None).unwrap();
        assert!(!staged.lines().any(|p| p == ".env.local"), "{staged}");
        assert!(staged.lines().any(|p| p == "tracked.txt"), "{staged}");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// Does the object database hold a blob with exactly these bytes?
    fn has_object(d: &Path, bytes: &[u8]) -> bool {
        let sha = String::from_utf8_lossy(&git(d, &["hash-object", "--no-filters", "--stdin"], None, Some(bytes)).unwrap()).trim().to_string();
        git_text(d, &["cat-file", "-e", &sha], None).is_ok()
    }

    /// Secret files are never captured: no blob is made from them, and capture records only their names.
    #[test]
    fn secret_files_are_never_captured() {
        let d = temp_repo("secret-cap");
        fs::write(d.join(".env"), "SECRET=never-in-git-aaa").unwrap();
        fs::write(d.join(".env.local"), "SECRET=never-in-git-bbb").unwrap();
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("sub").join(".env"), "SECRET=never-in-git-ccc").unwrap();
        fs::write(d.join("env.txt"), "not a secret name").unwrap(); // untracked, not ignored: in the main tree
        fs::write(d.join("other.log"), "other ignored file").unwrap();
        let state = state_for("secret-cap");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        for b in [&b"SECRET=never-in-git-aaa"[..], b"SECRET=never-in-git-bbb", b"SECRET=never-in-git-ccc"] {
            assert!(!has_object(&d, b), "a blob was made from a secret file");
        }
        assert_eq!(c.secret_skipped, [".env", ".env.local", "sub/.env"]);
        assert_eq!(show(&d, &c.ref_name, "env.txt"), "not a secret name");
        assert_eq!(show(&d, &c.ignored_ref.unwrap(), "other.log"), "other ignored file", "other ignored files are still captured");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// The hard case: rollback leaves secret files byte-identical (changed, deleted or created by the job); none is "new".
    #[test]
    fn rollback_never_touches_secret_files() {
        let d = temp_repo("secret-rb");
        fs::write(d.join(".env"), "SECRET=original").unwrap();
        fs::write(d.join(".env.local"), "LOCAL=original").unwrap();
        fs::write(d.join("other.log"), "other original").unwrap();
        let state = state_for("secret-rb");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        fs::write(d.join(".env"), "SECRET=written by the job").unwrap();
        fs::remove_file(d.join(".env.local")).unwrap();
        fs::write(d.join(".env.new"), "NEW=made by the job").unwrap();
        fs::write(d.join("other.log"), "other changed").unwrap();
        fs::write(d.join("tracked.txt"), "job edit").unwrap();
        let r = rollback(&d, &state, "j", &c.ref_name, c.ignored_ref.as_deref(), &c.skipped, c.index_tree.as_deref(), &opts()).unwrap();
        assert_eq!(fs::read(d.join(".env")).unwrap(), b"SECRET=written by the job", "never overwritten");
        assert!(!d.join(".env.local").exists(), "a deleted secret file is not recreated");
        assert_eq!(fs::read(d.join(".env.new")).unwrap(), b"NEW=made by the job", "never deleted as new");
        assert_eq!(fs::read(d.join("other.log")).unwrap(), b"other original");
        assert_eq!(fs::read(d.join("tracked.txt")).unwrap(), b"one");
        assert!(r.not_restored.is_empty(), "secrets are not even reported: {:?}", r.not_restored);
        for b in [&b"SECRET=original"[..], b"SECRET=written by the job", b"LOCAL=original", b"NEW=made by the job"] {
            assert!(!has_object(&d, b), "a blob was made from a secret file");
        }
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }

    /// A checkpoint made before this rule may hold a secret file: rollback must not restore it either.
    #[test]
    fn rollback_ignores_a_secret_file_inside_an_old_checkpoint() {
        let d = temp_repo("secret-old");
        fs::write(d.join("a.log"), "a").unwrap();
        let state = state_for("secret-old");
        let c = checkpoint(&d, &state, "j", &opts()).unwrap();
        let blob = String::from_utf8_lossy(&git(&d, &["hash-object", "-w", "--no-filters", "--stdin"], None, Some(b"SECRET=old")).unwrap()).trim().to_string();
        let old_tree = git_text(&d, &["rev-parse", &format!("{}^{{tree}}", c.ignored_ref.as_ref().unwrap())], None).unwrap();
        let mut input = git(&d, &["ls-tree", "-z", &old_tree], None, None).unwrap();
        input.extend_from_slice(format!("100644 blob {blob}\t.env").as_bytes());
        input.push(0);
        let tree = String::from_utf8_lossy(&git(&d, &["mktree", "-z"], None, Some(&input)).unwrap()).trim().to_string();
        let commit = commit_tree(&d, &tree, None, "old style").unwrap();
        create_ref(&d, "refs/broadsword/checkpoints-ignored/old", &commit).unwrap();
        let main_commit = git_text(&d, &["rev-parse", &c.ref_name], None).unwrap();
        create_ref(&d, "refs/broadsword/checkpoints/old", &main_commit).unwrap();
        // .env is not on disk: the old checkpoint must not bring it back
        let r = rollback(&d, &state, "old", "refs/broadsword/checkpoints/old", Some("refs/broadsword/checkpoints-ignored/old"), &[], c.index_tree.as_deref(), &opts()).unwrap();
        assert!(!d.join(".env").exists());
        assert!(r.not_restored.is_empty());
        assert_eq!(r.secret_skipped, [".env"], "the skip is reported, never silent");
        let _ = fs::remove_dir_all(&state);
        fs::remove_dir_all(&d).unwrap();
    }
}
