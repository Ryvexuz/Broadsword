//! Tests for `wtgit::diff_since`: the review diff lists every changed file whose content it does not show, reads the reviewed
//! commit's `.gitattributes` (not main's), and fails loudly. REAL Git repos in throwaway folders under the repo's `scratch/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

#[allow(dead_code)]
#[path = "../src/wtgit.rs"]
mod wtgit;
use wtgit::{diff_since, diff_since_mode, AttrMode};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@localhost")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@localhost")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

struct Repo {
    root: PathBuf,
    main: PathBuf,
    wt: PathBuf,
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A repo with one commit on `main` (no `.gitattributes`) and a worktree on branch `task` where the test commits.
fn repo(name: &str) -> Repo {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("wtgit-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(&root).unwrap();
    let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
    let main = root.join("main");
    fs::create_dir_all(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    git(&main, &["config", "core.autocrlf", "false"]);
    fs::write(main.join("a.txt"), "one\n").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-q", "-m", "init"]);
    let wt = root.join("wt");
    git(&main, &["worktree", "add", "-q", "-b", "task", wt.to_str().unwrap()]);
    Repo { root, main, wt }
}

fn commit_all(wt: &Path) -> String {
    git(wt, &["add", "-A"]);
    git(wt, &["commit", "-q", "-m", "work"]);
    git(wt, &["rev-parse", "HEAD"])
}

fn attr_branch(name: &str) -> (Repo, String) {
    let r = repo(name);
    fs::write(r.wt.join(".gitattributes"), "*.ppm binary\n*.dat -diff\n").unwrap();
    fs::write(r.wt.join("pic.ppm"), "P6\nLOOKSLIKETEXT\n").unwrap();
    fs::write(r.wt.join("blob.dat"), "SECRETLOOKINGLINE\nsecond\n").unwrap();
    fs::write(r.wt.join("notes.txt"), "visible line\n").unwrap();
    let tip = commit_all(&r.wt);
    (r, tip)
}

#[test]
fn binary_and_minus_diff_files_are_listed_and_not_shown_as_text() {
    let (r, tip) = attr_branch("list");
    for mode in [AttrMode::Auto, AttrMode::AttrSource, AttrMode::Worktree] {
        let out = diff_since_mode(&r.main, "main", &tip, mode).unwrap();
        println!("--- mode {mode:?} ---\n{out}");
        let (header, text) = out.split_once("=== end of list ===").unwrap_or_else(|| panic!("no header block in:\n{out}"));
        assert!(header.contains("A  pic.ppm  old size -  new size 17 bytes  new blob "), "ppm row missing: {header}");
        assert!(header.contains("A  blob.dat  old size -  new size 25 bytes  new blob "), "dat row missing: {header}");
        assert!(!header.contains("notes.txt") && !header.contains(".gitattributes"), "text files must not be listed: {header}");
        assert!(!text.contains("LOOKSLIKETEXT") && !text.contains("SECRETLOOKINGLINE"), "binary content shown as text: {text}");
        assert!(text.contains("+visible line"), "text change missing from the diff: {text}");
    }
}

#[test]
fn attributes_come_from_the_reviewed_commit_not_the_main_checkout() {
    let (r, tip) = attr_branch("attrs");
    // main has no .gitattributes and git is run from main: the ppm is hidden only if the tip's attributes are used.
    assert!(!r.main.join(".gitattributes").exists());
    let out = diff_since(&r.main, "main", &tip).unwrap();
    assert!(out.contains("pic.ppm") && !out.contains("LOOKSLIKETEXT"), "tip attributes were not used: {out}");
}

#[test]
fn a_plain_text_branch_has_no_header() {
    let r = repo("plain");
    fs::write(r.wt.join("b.txt"), "hello\n").unwrap();
    let tip = commit_all(&r.wt);
    let out = diff_since(&r.main, "main", &tip).unwrap();
    assert!(!out.contains("=== "), "unexpected header: {out}");
    assert!(out.contains("+hello"));
}

#[test]
fn a_modified_binary_shows_old_and_new_sizes() {
    let r = repo("mod");
    fs::write(r.wt.join("x.bin"), b"\0\0\0\0").unwrap();
    git(&r.wt, &["add", "-A"]);
    git(&r.wt, &["commit", "-q", "-m", "bin1"]);
    git(&r.main, &["merge", "-q", "--ff-only", "task"]);
    fs::write(r.wt.join("x.bin"), b"\0\0\0\0\0\0\0").unwrap();
    let tip = commit_all(&r.wt);
    let out = diff_since(&r.main, "main", &tip).unwrap();
    assert!(out.contains("M  x.bin  old size 4 bytes  new size 7 bytes"), "{out}");
    fs::remove_file(r.wt.join("x.bin")).unwrap();
    let tip2 = commit_all(&r.wt);
    let out2 = diff_since(&r.main, "main", &tip2).unwrap();
    assert!(out2.contains("D  x.bin  old size 4 bytes  new size -"), "{out2}");
}

#[test]
fn git_failures_are_errors_not_empty_diffs() {
    let (r, tip) = attr_branch("err");
    assert!(diff_since(&r.main, "no-such-base", &tip).is_err());
    assert!(diff_since(&r.main, "main", "no-such-tip").is_err());
    assert!(diff_since(&r.root.join("not-a-repo"), "main", &tip).is_err());
    // worktree route with a tip no checkout has: refused, not diffed with another commit's attributes
    git(&r.wt, &["checkout", "-q", "--detach", "main"]);
    let e = diff_since_mode(&r.main, "main", &tip, AttrMode::Worktree).unwrap_err();
    assert!(e.contains("cannot be read without --attr-source"), "{e}");
}
