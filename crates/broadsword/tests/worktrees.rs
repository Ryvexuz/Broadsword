//! End-to-end tests for per-task worktrees (`task start --worktree`, `task done`, `broadsword worktree ...`): the real
//! binary and REAL Git repos, inside throwaway Broadsword roots under the repo's `scratch/` folder. The project `p` in
//! each root is its own Git repo under `projects/`; the real Broadsword repo and the crash sim are never touched.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

#[allow(dead_code)]
#[path = "../src/json.rs"]
mod j;
use j::{parse, J};

mod support;
use support::bin;
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// Run git in `dir` (must succeed unless `allow_fail`); returns trimmed stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let o = git_out(dir, args);
    assert_eq!(o.code, 0, "git {args:?} in {} failed: {}", dir.display(), o.stderr);
    o.stdout.trim().to_string()
}

fn git_out(dir: &Path, args: &[&str]) -> Out {
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
    Out { code: o.status.code().unwrap(), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
}

struct Root {
    dir: PathBuf,
}

impl Root {
    fn new(name: &str) -> Root {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("wt-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        let dir = fs::canonicalize(&dir).unwrap();
        let dir = PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"));
        // p: a Git repo of its own with one commit on `main`. q: a project that is NOT a Git repo.
        for p in ["p", "q"] {
            fs::create_dir_all(dir.join("projects").join(p)).unwrap();
            fs::write(dir.join("projects").join(p).join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        }
        let p = dir.join("projects/p");
        fs::write(p.join("a.txt"), "one\n").unwrap();
        git(&p, &["init", "-q", "-b", "main"]);
        git(&p, &["add", "."]);
        git(&p, &["commit", "-q", "-m", "first"]);
        let toml = support::config(&[("mcp_call_timeout_seconds", "600")]);
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn p(&self) -> PathBuf {
        self.dir.join("projects/p")
    }

    fn run(&self, args: &[&str]) -> Out {
        let o = Command::new(bin()).args(args).current_dir(&self.dir).stdin(Stdio::null()).output().unwrap();
        Out { code: o.status.code().unwrap(), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
    }

    /// A new open task in project `project` with its own claim (so tasks never conflict). Returns the id.
    fn add(&self, project: &str, claim: &str) -> String {
        let o = self.run(&["task", "add", project, "--title", "t", "--claim", claim]);
        assert_eq!(o.code, 0, "{}", o.stderr);
        o.stdout.trim().to_string()
    }

    /// A task in `p`, started with `--worktree`. Returns the id.
    fn started(&self, claim: &str) -> String {
        let id = self.add("p", claim);
        let o = self.run(&["task", "start", &id, "--owner", "coder", "--worktree"]);
        assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
        id
    }

    fn wt(&self, id: &str) -> PathBuf {
        self.dir.join("worktrees/p").join(id)
    }

    fn status_of(&self, id: &str) -> String {
        self.show(id, "status").unwrap()
    }

    fn show(&self, id: &str, key: &str) -> Option<String> {
        let o = self.run(&["task", "show", id]);
        assert_eq!(o.code, 0, "{}", o.stderr);
        o.stdout.lines().find_map(|l| l.strip_prefix(&format!("{key}: "))).map(String::from)
    }

    /// Commit a new file inside the task's worktree.
    fn commit_in_wt(&self, id: &str, file: &str) -> String {
        fs::write(self.wt(id).join(file), "work\n").unwrap();
        git(&self.wt(id), &["add", file]);
        git(&self.wt(id), &["commit", "-q", "-m", &format!("work {file}")]);
        git(&self.wt(id), &["rev-parse", "HEAD"])
    }

    fn merge_into_main(&self, id: &str) {
        git(&self.p(), &["merge", "-q", "--no-edit", &format!("task/{id}")]);
    }

    fn branches(&self) -> Vec<String> {
        git(&self.p(), &["branch", "--format=%(refname:short)"]).lines().map(String::from).collect()
    }

    fn worktree_dirs(&self) -> Vec<String> {
        git(&self.p(), &["worktree", "list", "--porcelain"]).lines().filter_map(|l| l.strip_prefix("worktree ")).map(String::from).collect()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn start_with_worktree_bases_the_branch_on_the_current_main_tip() {
    let r = Root::new("base");
    let first = git(&r.p(), &["rev-parse", "HEAD"]);
    // main moves on BEFORE the task starts: the base must be the new tip, not the first commit
    fs::write(r.p().join("a.txt"), "two\n").unwrap();
    git(&r.p(), &["commit", "-q", "-am", "second"]);
    let tip = git(&r.p(), &["rev-parse", "HEAD"]);
    assert_ne!(first, tip);

    let id = r.add("p", "src");
    let o = r.run(&["task", "start", &id, "--owner", "coder-1", "--worktree"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains(&format!("branch: task/{id}")) && o.stdout.contains(&format!("base: {tip}")), "{}", o.stdout);
    assert_eq!(r.status_of(&id), "running");
    let wt = r.wt(&id);
    assert!(wt.join("a.txt").is_file());
    assert_eq!(fs::read_to_string(wt.join("a.txt")).unwrap().replace("\r\n", "\n"), "two\n");
    assert_eq!(git(&wt, &["branch", "--show-current"]), format!("task/{id}"));
    assert_eq!(git(&r.p(), &["rev-parse", &format!("task/{id}")]), tip);
    assert_eq!(git(&r.p(), &["branch", "--show-current"]), "main", "the main checkout is untouched");
    assert_eq!(r.show(&id, "branch").unwrap(), format!("task/{id}"));
    assert_eq!(r.show(&id, "base").unwrap(), tip);
    assert_eq!(r.show(&id, "worktree").unwrap(), format!("worktrees/p/{id}"), "recorded relative to root");
    assert_eq!(r.worktree_dirs().len(), 2);

    // without --worktree nothing is made
    let id2 = r.add("p", "docs");
    assert_eq!(r.run(&["task", "start", &id2, "--owner", "coder-2"]).code, 0);
    assert!(!r.wt(&id2).exists() && r.show(&id2, "worktree").is_none());
}

#[test]
fn a_failed_worktree_rolls_the_task_back_to_open_with_no_half_state() {
    let r = Root::new("rollback");
    // 1. the branch already exists
    let id = r.add("p", "src");
    git(&r.p(), &["branch", &format!("task/{id}")]);
    let o = r.run(&["task", "start", &id, "--owner", "coder-1", "--worktree"]);
    assert_eq!(o.code, 1, "{}", o.stdout);
    assert!(o.stderr.contains("already exists") && o.stderr.contains("is still `open`"), "{}", o.stderr);
    assert_eq!(r.status_of(&id), "open");
    assert_eq!(r.show(&id, "owner").unwrap(), "-");
    assert_eq!(r.show(&id, "started").unwrap(), "-");
    assert!(r.show(&id, "worktree").is_none());
    assert!(!r.dir.join("worktrees/p").exists(), "no leftover folders");
    assert_eq!(r.worktree_dirs().len(), 1);
    // its claims are not held: a task with the same claim starts fine
    let other = r.add("p", "src");
    assert_eq!(r.run(&["task", "start", &other, "--owner", "x"]).code, 0);
    assert_eq!(r.run(&["task", "abandon", &other]).code, 0);
    // after fixing the cause (delete the stray branch) the same task starts
    git(&r.p(), &["branch", "-d", &format!("task/{id}")]);
    assert_eq!(r.run(&["task", "start", &id, "--owner", "coder-1", "--worktree"]).code, 0);

    // 2. the project is not a Git repo
    let q = r.add("q", "src");
    let o = r.run(&["task", "start", &q, "--owner", "c", "--worktree"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("not a Git repo") && o.stderr.contains("is still `open`"), "{}", o.stderr);
    assert_eq!(r.status_of(&q), "open");

    // 3. the main checkout is on a detached HEAD
    git(&r.p(), &["checkout", "-q", "--detach"]);
    let d = r.add("p", "docs");
    let o = r.run(&["task", "start", &d, "--owner", "c", "--worktree"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("detached"), "{}", o.stderr);
    assert_eq!(r.status_of(&d), "open");

    // 4. the folder already exists
    git(&r.p(), &["checkout", "-q", "main"]);
    fs::create_dir_all(r.wt(&d)).unwrap();
    let o = r.run(&["task", "start", &d, "--owner", "c", "--worktree"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("already exists"), "{}", o.stderr);
    assert_eq!(r.status_of(&d), "open");
    assert!(r.wt(&d).is_dir(), "a pre-existing folder is never touched");
}

#[test]
fn done_after_merging_removes_the_worktree_its_record_and_the_branch() {
    let r = Root::new("merged");
    let id = r.started("src");
    r.commit_in_wt(&id, "b.txt");
    r.merge_into_main(&id);
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("worktree removed") && o.stdout.contains("deleted"), "{}", o.stdout);
    assert!(o.stderr.is_empty(), "no warning: {}", o.stderr);
    assert!(!r.wt(&id).exists(), "folder gone");
    assert_eq!(r.worktree_dirs().len(), 1, "git's record gone: {:?}", r.worktree_dirs());
    assert!(!r.branches().iter().any(|b| b.starts_with("task/")), "branch gone: {:?}", r.branches());
    assert!(!r.dir.join("worktrees/p").exists(), "no empty folder left");
    assert!(r.dir.join("worktrees").is_dir(), "the configured worktrees_dir itself stays");
    assert_eq!(r.status_of(&id), "done");
    assert!(r.show(&id, "worktree").unwrap().ends_with("(removed)"));
    assert!(r.p().join("b.txt").is_file(), "the merged work is in main");
    // a second task in the same project still works afterwards
    let id2 = r.started("docs");
    assert!(r.wt(&id2).is_dir());
}

#[test]
fn done_with_an_unmerged_branch_keeps_everything_and_warns() {
    let r = Root::new("unmerged");
    let id = r.started("src");
    let sha = r.commit_in_wt(&id, "b.txt");
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 0, "done still succeeds: {}", o.stderr);
    assert_eq!(r.status_of(&id), "done");
    assert!(o.stdout.contains("worktree kept"), "{}", o.stdout);
    assert!(o.stderr.contains("WARNING") && o.stderr.contains("not merged") && o.stderr.contains("to fix:") && o.stderr.contains("--keep-branch"), "{}", o.stderr);
    assert!(r.wt(&id).is_dir());
    assert_eq!(git(&r.p(), &["rev-parse", &format!("task/{id}")]), sha);
    assert!(r.show(&id, "worktree").unwrap().ends_with(&id), "still recorded as existing");
    // after merging, the on-demand removal finishes the job
    r.merge_into_main(&id);
    let o = r.run(&["worktree", "remove", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(!r.wt(&id).exists() && !r.branches().iter().any(|b| b.starts_with("task/")));
    assert_eq!(r.run(&["worktree", "remove", &id]).code, 0, "repeating is harmless");
}

#[test]
fn uncommitted_or_untracked_changes_keep_the_worktree_even_when_merged() {
    let r = Root::new("dirty");
    // tracked file modified (branch has no commits of its own, so it IS merged)
    let a = r.started("src");
    fs::write(r.wt(&a).join("a.txt"), "edited\n").unwrap();
    let o = r.run(&["task", "done", &a]);
    assert_eq!(o.code, 0);
    assert!(o.stderr.contains("uncommitted or untracked") && o.stderr.contains("a.txt"), "{}", o.stderr);
    assert!(r.wt(&a).is_dir() && r.branches().contains(&format!("task/{a}")));
    // an untracked file
    let b = r.started("docs");
    fs::write(r.wt(&b).join("new.txt"), "x\n").unwrap();
    let o = r.run(&["task", "done", &b]);
    assert_eq!(o.code, 0);
    assert!(o.stderr.contains("uncommitted or untracked") && o.stderr.contains("new.txt"), "{}", o.stderr);
    assert!(r.wt(&b).join("new.txt").is_file());
    // a staged-only change
    let c = r.started("lib");
    fs::write(r.wt(&c).join("s.txt"), "x\n").unwrap();
    git(&r.wt(&c), &["add", "s.txt"]);
    assert_eq!(r.run(&["task", "done", &c]).code, 0);
    assert!(r.wt(&c).join("s.txt").is_file());
    // nothing of the three kept ones was lost, and `remove` agrees they are not removable
    for id in [&a, &b, &c] {
        let o = r.run(&["worktree", "remove", id]);
        assert_eq!(o.code, 3, "{}", o.stderr);
        assert!(o.stderr.contains("NOT removed"), "{}", o.stderr);
        assert!(r.wt(id).is_dir());
    }
    let o = r.run(&["worktree", "remove", &a, "--keep-branch"]);
    assert_eq!(o.code, 3, "--keep-branch still needs a clean worktree: {}", o.stderr);
    assert_eq!(fs::read_to_string(r.wt(&a).join("a.txt")).unwrap().replace("\r\n", "\n"), "edited\n");
}

#[test]
fn keep_branch_removes_only_the_folder_of_an_abandoned_unmerged_task() {
    let r = Root::new("keep");
    let id = r.started("src");
    let sha = r.commit_in_wt(&id, "b.txt");
    // running: refused outright
    let o = r.run(&["worktree", "remove", &id, "--keep-branch"]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("running"), "{}", o.stderr);
    assert_eq!(r.run(&["task", "abandon", &id]).code, 0);
    // abandoned and unmerged: plain remove is refused, nothing changes
    let o = r.run(&["worktree", "remove", &id]);
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains("not merged"), "{}", o.stderr);
    assert!(r.wt(&id).is_dir());
    // --keep-branch: folder and git record gone, branch and its commit stay
    let o = r.run(&["worktree", "remove", &id, "--keep-branch"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("kept"), "{}", o.stdout);
    assert!(!r.wt(&id).exists());
    assert_eq!(r.worktree_dirs().len(), 1);
    assert!(r.branches().contains(&format!("task/{id}")));
    assert_eq!(git(&r.p(), &["rev-parse", &format!("task/{id}")]), sha);
    assert!(git(&r.p(), &["log", "--format=%s", &format!("task/{id}")]).contains("work b.txt"), "the commit is reachable");
    assert!(!r.dir.join("worktrees/p").exists());
    assert!(r.show(&id, "worktree").unwrap().ends_with("(removed)"));
}

#[test]
fn a_worktree_that_left_its_branch_is_never_removed() {
    let r = Root::new("detached");
    let id = r.started("src");
    git(&r.wt(&id), &["checkout", "-q", "--detach"]);
    fs::write(r.wt(&id).join("c.txt"), "x\n").unwrap();
    git(&r.wt(&id), &["add", "c.txt"]);
    git(&r.wt(&id), &["commit", "-q", "-m", "orphan work"]); // on no branch
    assert_eq!(r.run(&["task", "done", &id]).code, 0);
    assert!(r.wt(&id).is_dir(), "commits on a detached HEAD would be lost");
    let o = r.run(&["worktree", "remove", &id, "--keep-branch"]);
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains("detached HEAD"), "{}", o.stderr);
}

#[test]
fn prune_removes_only_clean_merged_finished_worktrees_and_never_a_running_one() {
    let r = Root::new("prune");
    let running = r.started("a");
    let done_clean = r.started("b");
    assert_eq!(r.run(&["task", "done", &done_clean]).code, 0); // already removed by done
    let failed_clean = r.started("c"); // will fail, clean and merged (no commits)
    assert_eq!(r.run(&["task", "fail", &failed_clean, "--note", "x"]).code, 0);
    assert!(r.wt(&failed_clean).is_dir(), "fail leaves the worktree for prune");
    let done_unmerged = r.started("d");
    r.commit_in_wt(&done_unmerged, "d.txt");
    assert_eq!(r.run(&["task", "done", &done_unmerged]).code, 0);
    let abandoned_dirty = r.started("e");
    fs::write(r.wt(&abandoned_dirty).join("u.txt"), "x\n").unwrap();
    assert_eq!(r.run(&["task", "abandon", &abandoned_dirty]).code, 0);
    let done_merged_later = r.started("f");
    r.commit_in_wt(&done_merged_later, "f.txt");
    assert_eq!(r.run(&["task", "done", &done_merged_later]).code, 0); // kept: unmerged at that time
    r.merge_into_main(&done_merged_later); // ... merged afterwards

    let o = r.run(&["worktree", "list"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let row = |id: &str| o.stdout.lines().find(|l| l.starts_with(id)).map(|l| l.split('\t').map(String::from).collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(row(&running)[1..3], ["running".to_string(), format!("worktrees/p/{running}")]);
    assert_eq!(row(&running)[3], format!("task/{running}"));
    assert_eq!(row(&running)[5..], ["clean", "merged"]);
    assert_eq!(row(&done_unmerged)[5..], ["clean", "unmerged"]);
    assert_eq!(row(&abandoned_dirty)[5..], ["dirty", "merged"]);
    assert!(row(&done_clean).is_empty(), "already removed ones are not listed");

    let o = r.run(&["worktree", "prune"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains(&format!("removed {failed_clean}")) && o.stdout.contains(&format!("removed {done_merged_later}")), "{}", o.stdout);
    assert!(o.stdout.contains(&format!("kept {done_unmerged}")) && o.stdout.contains(&format!("kept {abandoned_dirty}")), "{}", o.stdout);
    assert!(o.stdout.contains("1 running or open task(s) left alone"), "{}", o.stdout);
    assert!(o.stdout.contains("not merged") && o.stdout.contains("uncommitted or untracked"), "{}", o.stdout);
    assert!(r.wt(&running).is_dir(), "a running task's worktree is untouched");
    assert!(r.branches().contains(&format!("task/{running}")));
    assert!(!r.wt(&failed_clean).exists() && !r.wt(&done_merged_later).exists());
    assert!(r.wt(&done_unmerged).is_dir() && r.wt(&abandoned_dirty).join("u.txt").is_file());
    assert!(r.branches().contains(&format!("task/{done_unmerged}")) && r.branches().contains(&format!("task/{abandoned_dirty}")));
    assert!(!r.branches().contains(&format!("task/{failed_clean}")) && !r.branches().contains(&format!("task/{done_merged_later}")));
    assert_eq!(r.worktree_dirs().len(), 4, "main + running + two kept: {:?}", r.worktree_dirs());
    // a second prune changes nothing
    let o = r.run(&["worktree", "prune"]);
    assert!(o.stdout.contains("0 removed, 2 kept"), "{}", o.stdout);
}

#[test]
fn a_worktree_deleted_by_hand_is_cleared_by_prune() {
    let r = Root::new("byhand");
    let id = r.started("src");
    assert_eq!(r.run(&["task", "fail", &id]).code, 0);
    fs::remove_dir_all(r.wt(&id)).unwrap(); // the user's own deletion, in the throwaway root
    assert_eq!(r.worktree_dirs().len(), 2, "git still lists the vanished worktree");
    assert!(r.run(&["worktree", "list"]).stdout.contains("missing"));
    let o = r.run(&["worktree", "prune"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.worktree_dirs().len(), 1, "git's record is cleared");
    assert!(o.stdout.contains(&format!("removed {id}")), "{}", o.stdout);
    assert!(!r.branches().contains(&format!("task/{id}")), "its branch was merged (no commits), so it is deleted");
    assert!(r.show(&id, "worktree").unwrap().ends_with("(removed)"));
    assert!(!r.dir.join("worktrees/p").exists());

    // the same by hand, but with unmerged commits: the branch must survive
    let id = r.started("docs");
    let sha = r.commit_in_wt(&id, "w.txt");
    assert_eq!(r.run(&["task", "fail", &id]).code, 0);
    fs::remove_dir_all(r.wt(&id)).unwrap();
    assert_eq!(r.run(&["worktree", "prune"]).code, 0);
    assert_eq!(git(&r.p(), &["rev-parse", &format!("task/{id}")]), sha, "unmerged commits are kept");
    assert_eq!(r.worktree_dirs().len(), 1);
}

#[test]
fn create_on_demand_for_a_running_task_and_its_refusals() {
    let r = Root::new("create");
    let id = r.add("p", "src");
    let o = r.run(&["worktree", "create", &id]);
    assert_eq!(o.code, 1, "an open task gets none");
    assert!(o.stderr.contains("open"), "{}", o.stderr);
    assert_eq!(r.run(&["task", "start", &id, "--owner", "c"]).code, 0);
    let o = r.run(&["worktree", "create", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(r.wt(&id).join("a.txt").is_file() && o.stdout.contains(&format!("branch: task/{id}")), "{}", o.stdout);
    assert_eq!(r.status_of(&id), "running");
    let o = r.run(&["worktree", "create", &id]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("already has a worktree"), "{}", o.stderr);
    // failure leaves the task running, with no worktree and no leftovers
    let id2 = r.add("p", "docs");
    git(&r.p(), &["branch", &format!("task/{id2}")]);
    assert_eq!(r.run(&["task", "start", &id2, "--owner", "c"]).code, 0);
    let o = r.run(&["worktree", "create", &id2]);
    assert_eq!(o.code, 1);
    assert!(!r.wt(&id2).exists() && r.show(&id2, "worktree").is_none() && r.status_of(&id2) == "running");
    assert_eq!(r.run(&["worktree", "create", "nope"]).code, 1);
}

#[test]
fn a_tampered_record_cannot_point_removal_outside_worktrees_dir() {
    let r = Root::new("tamper");
    let id = r.started("src");
    assert_eq!(r.run(&["task", "fail", &id]).code, 0);
    let rec = r.dir.join("state/tasks").join(format!("{id}.task"));
    let text = fs::read_to_string(&rec).unwrap();
    let bad = text.lines().map(|l| if l.starts_with("worktree = ") { "worktree = scratch/..".to_string() } else { l.to_string() }).collect::<Vec<_>>().join("\n");
    fs::write(&rec, bad + "\n").unwrap();
    let o = r.run(&["worktree", "remove", &id]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("not inside the worktrees_dir"), "{}", o.stderr);
    assert!(r.dir.join("scratch").is_dir() && r.wt(&id).is_dir());
}

#[test]
fn repair_fixes_the_links_after_the_root_moves() {
    let mut r = Root::new("repair");
    let id = r.started("src");
    let old = r.dir.clone();
    let new = old.with_file_name(format!("{}-moved", old.file_name().unwrap().to_string_lossy()));
    fs::rename(&old, &new).unwrap();
    r.dir = new.clone();
    assert!(git_out(&r.wt(&id), &["status"]).code != 0, "after the move git cannot find the repo from the worktree");
    let o = r.run(&["worktree", "repair"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("p: repaired") || o.stdout.contains("p: ok"), "{}", o.stdout);
    assert!(o.stdout.contains("q: not a Git repo, skipped"), "{}", o.stdout);
    assert_eq!(git_out(&r.wt(&id), &["status"]).code, 0, "{}", git_out(&r.wt(&id), &["status"]).stderr);
    let listed = r.worktree_dirs().join("\n").replace('\\', "/");
    assert!(listed.contains(&format!("-moved/worktrees/p/{id}")), "{listed}");
    // and the repaired worktree is fully usable: a clean merged task is removed normally
    assert_eq!(r.run(&["task", "done", &id]).code, 0);
    assert!(!r.wt(&id).exists());
    // repair again is harmless
    assert_eq!(r.run(&["worktree", "repair"]).code, 0);
}

// ---- review fixes ------------------------------------------------------------------------

const ADD_GATE_ENV: &str = "BROADSWORD_INTERNAL_TEST_WORKTREE_ADD_GATE_FILE";

/// Start `task start <id> --owner a --worktree` with `git worktree add` parked on a gate file (scratch only), and wait until
/// the task shows `running` (step 1 done, lock released). Returns the child and the gate path.
#[allow(clippy::zombie_processes)] // the caller waits on (or kills and waits on) the returned child
fn parked_start(r: &Root, id: &str, tag: &str) -> (std::process::Child, PathBuf) {
    let gate = r.dir.join("scratch").join(format!("gate-{tag}"));
    let child = Command::new(bin())
        .args(["task", "start", id, "--owner", "a", "--worktree"])
        .env(ADD_GATE_ENV, &gate)
        .current_dir(&r.dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for _ in 0..2000 {
        if r.status_of(id) == "running" {
            return (child, gate);
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the parked start never reached running");
}

#[test]
fn a_slow_worktree_creation_does_not_hold_the_board_lock() {
    let r = Root::new("slow");
    let id = r.add("p", "src");
    let (child, gate) = parked_start(&r, &id, "slow");
    // creation is parked in `git worktree add`; the board is free:
    let other = r.add("p", "docs");
    let o = r.run(&["task", "start", &other, "--owner", "b"]);
    assert_eq!(o.code, 0, "another start must not get exit 4 board busy: {}", o.stderr);
    // the claims are held from the first step
    let clash = r.add("p", "src/x.rs");
    assert_eq!(r.run(&["task", "start", &clash, "--owner", "c"]).code, 3, "claims of the creating task are held");
    // done refuses while the worktree is being created; remove and prune leave it alone
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 1);
    assert!(o.stderr.contains("still being created"), "{}", o.stderr);
    assert_eq!(r.status_of(&id), "running");
    assert_eq!(r.run(&["worktree", "remove", &id]).code, 1);
    assert_eq!(r.run(&["worktree", "prune"]).code, 0);
    assert!(r.run(&["worktree", "list"]).stdout.contains("creating"));
    // release: the start finishes and records the worktree
    fs::write(&gate, "go").unwrap();
    let o = child.wait_with_output().unwrap();
    let out = String::from_utf8_lossy(&o.stdout).into_owned();
    assert_eq!(o.status.code(), Some(0), "{out}{}", String::from_utf8_lossy(&o.stderr));
    assert!(out.contains("is running") && out.contains(&format!("branch: task/{id}")), "{out}");
    assert!(r.wt(&id).join("a.txt").is_file());
    assert_eq!(r.show(&id, "worktree").unwrap(), format!("worktrees/p/{id}"));
    assert_eq!(r.run(&["task", "done", &id]).code, 0);
    assert!(!r.wt(&id).exists());
}

#[test]
fn a_creation_that_fails_after_the_lock_was_released_rolls_back_and_never_says_running() {
    let r = Root::new("latefail");
    let id = r.add("p", "src");
    let (child, gate) = parked_start(&r, &id, "late");
    git(&r.p(), &["branch", &format!("task/{id}")]); // someone takes the branch while the add is parked
    fs::write(&gate, "go").unwrap();
    let o = child.wait_with_output().unwrap();
    let (out, err) = (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned());
    assert_eq!(o.status.code(), Some(1), "{out}{err}");
    assert!(err.contains("rolled back to `open`"), "{err}");
    assert!(!out.contains("is running"), "stdout must not claim running: {out}");
    assert_eq!(r.status_of(&id), "open");
    assert_eq!(r.show(&id, "owner").unwrap(), "-");
    assert!(r.show(&id, "worktree").is_none() && !r.dir.join("worktrees/p").exists());
    assert!(r.branches().contains(&format!("task/{id}")), "the other party's branch is not ours to delete");
    // a plain refusal (before running) also never prints running
    let id2 = r.add("p", "docs");
    git(&r.p(), &["branch", &format!("task/{id2}")]);
    let o = r.run(&["task", "start", &id2, "--owner", "a", "--worktree"]);
    assert_eq!(o.code, 1);
    assert!(!o.stdout.contains("running"), "{}", o.stdout);
}

#[test]
fn a_stale_creating_state_is_recovered_by_worktree_create_or_ended_by_abandon() {
    let r = Root::new("stale");
    let id = r.add("p", "src");
    let (mut child, _gate) = parked_start(&r, &id, "stale");
    child.kill().unwrap(); // the creator dies between step 1 and step 2
    let _ = child.wait();
    assert_eq!(r.status_of(&id), "running");
    assert_eq!(r.run(&["task", "done", &id]).code, 1, "done refuses while the state is stale");
    assert_eq!(r.run(&["task", "start", &r.add("p", "src"), "--owner", "x"]).code, 3, "its claims stay held");
    // nothing was made: `worktree create` starts afresh and finishes the job
    let o = r.run(&["worktree", "create", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(r.wt(&id).join("a.txt").is_file() && r.show(&id, "worktree").is_some());
    assert_eq!(r.run(&["task", "done", &id]).code, 0);
    assert!(!r.wt(&id).exists());

    // the creator died AFTER `git worktree add` but before recording: the complete worktree is adopted
    let id = r.add("p", "docs");
    assert_eq!(r.run(&["task", "start", &id, "--owner", "a"]).code, 0);
    let base = git(&r.p(), &["rev-parse", "HEAD"]);
    git(&r.p(), &["worktree", "add", "-q", "-b", &format!("task/{id}"), r.wt(&id).to_str().unwrap(), &base]);
    let rec = r.dir.join("state/tasks").join(format!("{id}.task"));
    let text = fs::read_to_string(&rec).unwrap();
    fs::write(&rec, format!("{}\nworktree_creating = 1-1\nbranch = task/{id}\nbase = {base}\n", text.trim_end())).unwrap();
    let o = r.run(&["worktree", "create", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("adopted"), "{}", o.stdout);
    assert_eq!(r.show(&id, "worktree").unwrap(), format!("worktrees/p/{id}"));
    assert_eq!(r.run(&["task", "done", &id]).code, 0);
    assert!(!r.wt(&id).exists());

    // abandon clears the state and keeps any folder, recorded so prune manages it
    let id = r.add("p", "lib");
    let (mut child, _g) = parked_start(&r, &id, "stale2");
    child.kill().unwrap();
    let _ = child.wait();
    assert_eq!(r.run(&["task", "abandon", &id]).code, 0);
    assert_eq!(r.status_of(&id), "abandoned");
    assert!(r.run(&["worktree", "list"]).stdout.contains("no worktrees"));
}

#[test]
fn ignored_files_block_removal_and_are_never_deleted() {
    let r = Root::new("ignored");
    fs::write(r.p().join(".gitignore"), ".env\nnotes/\n").unwrap();
    git(&r.p(), &["add", ".gitignore"]);
    git(&r.p(), &["commit", "-q", "-m", "ignore"]);
    let id = r.started("src"); // merged: no commits of its own
    fs::write(r.wt(&id).join(".env"), "KEY=secret\n").unwrap();
    fs::create_dir_all(r.wt(&id).join("notes")).unwrap();
    fs::write(r.wt(&id).join("notes/n.md"), "my notes\n").unwrap();
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 0);
    assert!(o.stdout.contains("worktree kept"), "{}", o.stdout);
    assert!(o.stderr.contains("ignored files") && o.stderr.contains(".env") && o.stderr.contains("notes"), "{}", o.stderr);
    assert!(!o.stderr.contains("--force"), "{}", o.stderr);
    assert_eq!(fs::read_to_string(r.wt(&id).join(".env")).unwrap().trim(), "KEY=secret");
    assert!(r.wt(&id).join("notes/n.md").is_file() && r.branches().contains(&format!("task/{id}")));
    assert!(r.run(&["worktree", "list"]).stdout.contains("dirty"));
    let o = r.run(&["worktree", "remove", &id, "--keep-branch"]);
    assert_eq!(o.code, 3, "no override, not even --keep-branch: {}", o.stderr);
    assert_eq!(r.run(&["worktree", "prune"]).code, 0);
    assert!(r.wt(&id).join(".env").is_file());
    // the owner deletes them deliberately, then it works
    fs::remove_file(r.wt(&id).join(".env")).unwrap();
    fs::remove_dir_all(r.wt(&id).join("notes")).unwrap();
    assert_eq!(r.run(&["worktree", "remove", &id]).code, 0);
    assert!(!r.wt(&id).exists());
}

#[test]
fn done_is_flagged_destructive_and_says_it_deletes_the_worktree_and_branch() {
    let r = Root::new("destructive");
    let o = r.run(&["tools", "task_done", "--json"]);
    assert!(o.stdout.contains("\"destructiveHint\": true"), "{}", o.stdout);
    assert!(o.stdout.contains("DELETES the worktree folder and the `task/<id>` branch"), "{}", o.stdout);
}

fn kept_ref(id: &str, sha: &str) -> String {
    format!("refs/broadsword/kept/{id}/{}", &sha[..12])
}

#[test]
fn a_commit_whose_last_reference_is_the_worktrees_head_log_is_saved_then_the_worktree_goes() {
    let r = Root::new("orphan");
    let id = r.started("src");
    git(&r.wt(&id), &["checkout", "-q", "--detach"]);
    fs::write(r.wt(&id).join("c.txt"), "x\n").unwrap();
    git(&r.wt(&id), &["add", "c.txt"]);
    git(&r.wt(&id), &["commit", "-q", "-m", "orphan work"]);
    let sha = git(&r.wt(&id), &["rev-parse", "HEAD"]);
    git(&r.wt(&id), &["checkout", "-q", &format!("task/{id}")]); // back on the branch, clean, branch merged
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("worktree removed") && o.stdout.contains(&kept_ref(&id, &sha)) && o.stdout.contains("git update-ref -d"), "{}", o.stdout);
    assert!(!r.wt(&id).exists());
    assert_eq!(git(&r.p(), &["rev-parse", &kept_ref(&id, &sha)]), sha, "the commit is kept under a ref");
    assert!(git(&r.p(), &["log", "--format=%s", &kept_ref(&id, &sha)]).contains("orphan work"));
    assert!(r.show(&id, "kept refs").unwrap().contains(&kept_ref(&id, &sha)));
}

#[test]
fn rebase_then_merge_and_amend_remove_the_worktree_and_keep_the_old_commits() {
    let r = Root::new("rebase");
    // rebase-then-merge, as a merge queue does
    let id = r.started("src");
    let old = r.commit_in_wt(&id, "b.txt");
    fs::write(r.p().join("a.txt"), "moved on\n").unwrap();
    git(&r.p(), &["commit", "-q", "-am", "main moved"]);
    git(&r.wt(&id), &["rebase", "-q", "main"]);
    let new = git(&r.wt(&id), &["rev-parse", "HEAD"]);
    assert_ne!(old, new);
    r.merge_into_main(&id);
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("worktree removed") && o.stdout.contains("deleted"), "{}", o.stdout);
    assert!(!r.wt(&id).exists() && !r.branches().iter().any(|b| b.starts_with("task/")));
    assert_eq!(git(&r.p(), &["rev-parse", &kept_ref(&id, &old)]), old, "the pre-rebase commit is kept");
    assert!(git_out(&r.p(), &["rev-parse", "--verify", "-q", &kept_ref(&id, &new)]).code != 0, "the merged commit needs no kept ref");
    // amended away
    let id = r.started("docs");
    let first = r.commit_in_wt(&id, "d.txt");
    git(&r.wt(&id), &["commit", "-q", "--amend", "-m", "better"]);
    r.merge_into_main(&id);
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(!r.wt(&id).exists());
    assert_eq!(git(&r.p(), &["rev-parse", &kept_ref(&id, &first)]), first);
    // a clean worktree whose history is all on branches gets no kept refs
    let id = r.started("lib");
    r.commit_in_wt(&id, "e.txt");
    r.merge_into_main(&id);
    assert_eq!(r.run(&["task", "done", &id]).code, 0);
    assert!(git(&r.p(), &["for-each-ref", &format!("refs/broadsword/kept/{id}")]).is_empty());
}

#[test]
fn a_stray_folder_at_the_worktree_path_is_never_inspected_as_a_worktree() {
    let r = Root::new("stray");
    let id = r.started("src");
    assert_eq!(r.run(&["task", "fail", &id]).code, 0);
    // the worktree vanishes (removed with git itself) and a plain folder takes its place, inside an enclosing repo
    git(&r.p(), &["worktree", "remove", r.wt(&id).to_str().unwrap()]);
    fs::create_dir_all(r.wt(&id)).unwrap();
    fs::write(r.wt(&id).join("mine.txt"), "not git's\n").unwrap();
    let o = r.run(&["worktree", "remove", &id]);
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert!(o.stderr.contains("stray folder, not a git worktree"), "{}", o.stderr);
    assert!(!o.stderr.contains("merged") && !o.stderr.contains("uncommitted"), "no misleading clean/merged reasons: {}", o.stderr);
    assert!(r.run(&["worktree", "list"]).stdout.contains("stray"));
    let o = r.run(&["worktree", "prune"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains(&format!("kept {id}")) && o.stdout.contains("stray folder, not a git worktree") && o.stdout.contains("left in place"), "{}", o.stdout);
    assert!(r.wt(&id).join("mine.txt").is_file(), "left in place");
    // and done on a task whose folder was swapped for a stray one only warns
    let id2 = r.started("docs");
    git(&r.p(), &["worktree", "remove", r.wt(&id2).to_str().unwrap()]);
    fs::create_dir_all(r.wt(&id2)).unwrap();
    let o = r.run(&["task", "done", &id2]);
    assert_eq!(o.code, 0);
    assert!(o.stderr.contains("stray folder") && r.wt(&id2).is_dir());
}

#[test]
fn a_locked_worktree_is_refused_without_suggesting_force() {
    let r = Root::new("locked");
    let id = r.started("src");
    git(&r.p(), &["worktree", "lock", r.wt(&id).to_str().unwrap()]);
    assert_eq!(r.run(&["task", "fail", &id]).code, 0);
    let o = r.run(&["worktree", "remove", &id]);
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert!(o.stderr.contains("locked") && o.stderr.contains("unlock"), "{}", o.stderr);
    for bad in ["-f", "--force", "-D"] {
        assert!(!o.stderr.split_whitespace().any(|w| w.trim_matches(|c| c == '\'' || c == '`') == bad), "no force hint `{bad}`: {}", o.stderr);
    }
    assert!(r.wt(&id).is_dir());
    git(&r.p(), &["worktree", "unlock", r.wt(&id).to_str().unwrap()]);
    assert_eq!(r.run(&["worktree", "remove", &id]).code, 0);
}

#[test]
fn a_branch_checked_out_in_two_places_keeps_its_commits_and_the_record_says_kept() {
    let r = Root::new("twoplaces");
    let id = r.started("src");
    let other = r.dir.join("scratch").join("second-checkout");
    git(&r.p(), &["worktree", "add", "-q", "-f", other.to_str().unwrap(), &format!("task/{id}")]); // the test's own force, to set up
    let o = r.run(&["task", "done", &id]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("worktree removed") && o.stdout.contains(&format!("branch task/{id} kept")), "{}", o.stdout);
    assert!(r.branches().contains(&format!("task/{id}")), "git refuses -d for a branch checked out elsewhere");
    assert!(other.join("a.txt").is_file(), "the other checkout is untouched");
    assert_eq!(git(&r.p(), &["rev-parse", &format!("task/{id}")]), git(&r.p(), &["rev-parse", "main"]));
}

#[cfg(windows)]
#[test]
fn empty_parent_cleanup_never_removes_a_junction() {
    let r = Root::new("junction");
    let ext = r.dir.join("scratch").join("elsewhere");
    fs::create_dir_all(&ext).unwrap();
    fs::write(ext.join("keep.txt"), "mine\n").unwrap();
    fs::create_dir_all(r.dir.join("worktrees")).unwrap();
    let link = r.dir.join("worktrees/p");
    let (l, e) = (link.to_string_lossy().replace('/', "\\"), ext.to_string_lossy().replace('/', "\\"));
    let o = Command::new("cmd").args(["/C", "mklink", "/J", &l, &e]).output().unwrap();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stdout));
    let id = r.started("src");
    assert_eq!(r.run(&["task", "done", &id]).code, 0);
    assert!(!ext.join(&id).exists(), "the worktree was removed");
    assert!(link.exists(), "the junction itself must stay");
    assert_eq!(fs::read_to_string(ext.join("keep.txt")).unwrap().trim(), "mine");
    // remove the link by hand so the root's own cleanup never walks through it
    fs::remove_dir(&link).unwrap();
}

// ---- MCP ---------------------------------------------------------------------------------

fn mcp_call(r: &Root, name: &str, args: &str) -> (bool, String, i64) {
    let mut child = Command::new(bin()).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#).unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let resp = text.lines().filter_map(|l| parse(l).ok()).find(|v| v.get("id") == Some(&J::Num("2".into()))).unwrap_or_else(|| panic!("no response to {name}: {text}"));
    if let Some(e) = resp.get("error") {
        return (true, e.to_json(), -2);
    }
    let res = resp.get("result").unwrap();
    let Some(J::Arr(c)) = res.get("content") else { panic!("{}", resp.to_json()) };
    let body = c[0].get("text").and_then(J::str).unwrap().to_string();
    let code = match res.get("structuredContent").and_then(|s| s.get("exit_code")) {
        Some(J::Num(n)) => n.parse().unwrap(),
        _ => -1,
    };
    (matches!(res.get("isError"), Some(J::Bool(true))), body, code)
}

#[test]
fn the_worktree_tools_work_over_mcp() {
    let r = Root::new("mcp");
    let (_, body, _) = mcp_call(&r, "task_add", r#"{"project":"p","title":"a","claims":["src"]}"#);
    let a = body.lines().next().unwrap().trim().to_string();
    let (_, body, _) = mcp_call(&r, "task_add", r#"{"project":"p","title":"b","claims":["docs"]}"#);
    let b = body.lines().next().unwrap().trim().to_string();

    // task_start with worktree:true, and worktree_create for the other (started plain)
    let (err, body, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{a}","owner":"agent","worktree":true}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    assert!(body.contains(&format!("branch: task/{a}")), "{body}");
    assert!(r.wt(&a).is_dir());
    let (err, _, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{b}","owner":"agent"}}"#));
    assert_eq!((err, code), (false, 0));
    let (err, body, code) = mcp_call(&r, "worktree_create", &format!(r#"{{"id":"{b}"}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    assert!(r.wt(&b).is_dir());
    let (err, body, code) = mcp_call(&r, "worktree_create", &format!(r#"{{"id":"{b}"}}"#));
    assert_eq!((err, code), (true, 1), "{body}");

    let (err, body, code) = mcp_call(&r, "worktree_list", "{}");
    assert_eq!((err, code), (false, 0));
    assert!(body.contains(&a) && body.contains(&b) && body.contains("clean"), "{body}");

    // a running task's worktree: remove refused (1), prune leaves it alone
    let (err, _, code) = mcp_call(&r, "worktree_remove", &format!(r#"{{"id":"{a}"}}"#));
    assert_eq!((err, code), (true, 1));
    let (_, body, code) = mcp_call(&r, "worktree_prune", "{}");
    assert_eq!(code, 0);
    assert!(body.contains("2 running or open task(s) left alone"), "{body}");
    assert!(r.wt(&a).is_dir() && r.wt(&b).is_dir());

    // an unmerged commit: done keeps it; remove is refused with exit 3; keep_branch removes the folder only
    r.commit_in_wt(&a, "x.txt");
    let (err, body, code) = mcp_call(&r, "task_done", &format!(r#"{{"id":"{a}"}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    assert!(body.contains("worktree kept") && body.contains("WARNING"), "{body}");
    let (err, body, code) = mcp_call(&r, "worktree_remove", &format!(r#"{{"id":"{a}"}}"#));
    assert_eq!((err, code), (true, 3), "{body}");
    let (err, body, code) = mcp_call(&r, "worktree_remove", &format!(r#"{{"id":"{a}","keep_branch":true}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    assert!(!r.wt(&a).exists() && r.branches().contains(&format!("task/{a}")));

    // done on a clean merged one removes it; prune and repair run
    let (err, body, code) = mcp_call(&r, "task_done", &format!(r#"{{"id":"{b}"}}"#));
    assert_eq!((err, code), (false, 0));
    assert!(body.contains("worktree removed"), "{body}");
    let (err, body, code) = mcp_call(&r, "worktree_prune", "{}");
    assert_eq!((err, code), (false, 0), "{body}");
    let (err, body, code) = mcp_call(&r, "worktree_repair", "{}");
    assert_eq!((err, code), (false, 0), "{body}");
    assert!(body.contains("p: ok"), "{body}");
    let (_, body, _) = mcp_call(&r, "worktree_list", "{}");
    assert!(body.contains("no worktrees"), "{body}");

    // the registry says the right things about them
    let o = r.run(&["tools", "worktree_remove", "--json"]);
    assert!(o.stdout.contains("\"destructiveHint\": true"), "{}", o.stdout);
    let o = r.run(&["tools", "worktree_prune", "--json"]);
    assert!(o.stdout.contains("\"destructiveHint\": true"), "{}", o.stdout);
    let o = r.run(&["tools", "worktree_list", "--json"]);
    assert!(o.stdout.contains("\"readOnlyHint\": true"), "{}", o.stdout);
}

// ---- an interrupted `run` blocks only the source folder it worked in --------------------------------------------------

/// A pid that belonged to a process that has already exited.
fn dead_pid() -> u32 {
    let mut c = Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
    let pid = c.id();
    c.wait().unwrap();
    pid
}

impl Root {
    /// A `run` job record left behind by a crashed broadsword (its pid is dead). `source` is the folder the job ran in as
    /// the record stores it (`None` = an older record with no `source` line at all). Returns the record's id.
    fn interrupted_run(&self, n: u32, task: &str, source: Option<&str>) -> String {
        let id = format!("20260101-00000{n}-p-run");
        let dir = self.dir.join("state/jobs");
        fs::create_dir_all(&dir).unwrap();
        let src = source.map(|s| format!("source = {s}\n")).unwrap_or_default();
        let body = format!("id = {id}\nproject = p\nkind = run\n{src}task = {task}\nstatus = running\npid = {}\npid_start = 1\ncheckpoint = pending\n", dead_pid()); // pid_start = 1: a reused pid belongs to a process with another start time, so the record still reads as dead
        fs::write(dir.join(format!("{id}.rec")), body).unwrap();
        id
    }

    fn run_echo(&self, task: Option<&str>) -> Out {
        match task {
            Some(t) => self.run(&["run", "p", "--task", t, "--", "echo", "hi"]),
            None => self.run(&["run", "p", "--", "echo", "hi"]),
        }
    }

    fn status_in_record(&self, id: &str) -> String {
        let text = fs::read_to_string(self.dir.join(format!("state/jobs/{id}.rec"))).unwrap();
        text.lines().find_map(|l| l.strip_prefix("status = ")).unwrap_or("").to_string()
    }
}

#[test]
fn an_interrupted_run_in_one_worktree_blocks_only_that_worktree() {
    let r = Root::new("blockscope");
    let (a, b) = (r.started("a"), r.started("b"));
    let id = r.interrupted_run(1, &a, Some(&format!("worktrees/p/{a}")));
    // another worktree of the same project is not blocked
    let o = r.run_echo(Some(&b));
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    // the main checkout is not blocked
    let o = r.run_echo(None);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    // the interrupted run's own folder is: every kind of job, naming the job, the folder and both recover commands
    for args in [vec!["run", "p", "--task", &a, "--", "echo", "hi"], vec!["check", "p", "--task", &a], vec!["build", "p", "--task", &a], vec!["test", "p", "--task", &a]] {
        let o = r.run(&args);
        assert_eq!(o.code, 3, "{args:?}: {}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains(&id) && o.stderr.contains(&format!("worktrees/p/{a}")), "{}", o.stderr);
        assert!(o.stderr.contains(&format!("broadsword recover {id}")) && o.stderr.contains(&format!("recover --abandon {id}")), "the refusal names the exact commands: {}", o.stderr);
    }
    assert_eq!(r.status_in_record(&id), "running", "the automatic recovery never rolls a run back");
}

#[test]
fn an_interrupted_run_on_main_blocks_main_but_not_a_worktree() {
    let r = Root::new("blockmain");
    let a = r.started("a");
    let id = r.interrupted_run(1, "", Some("projects/p"));
    let o = r.run_echo(Some(&a));
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    let o = r.run_echo(None);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
    assert!(o.stderr.contains(&id) && o.stderr.contains("projects/p"), "{}", o.stderr);
    let o = r.run(&["check", "p"]);
    assert_eq!(o.code, 3, "{}{}", o.stdout, o.stderr);
}

#[test]
fn a_record_without_a_source_line_still_blocks_the_whole_project() {
    let r = Root::new("blockold");
    let (a, b) = (r.started("a"), r.started("b"));
    let id = r.interrupted_run(1, "", None);
    for task in [Some(a.as_str()), Some(b.as_str()), None] {
        let o = r.run_echo(task);
        assert_eq!(o.code, 3, "{task:?}: {}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains(&id), "{}", o.stderr);
    }
}

#[test]
fn abandoning_the_interrupted_run_of_one_worktree_lets_its_jobs_run_again() {
    let r = Root::new("blockabandon");
    let (a, b) = (r.started("a"), r.started("b"));
    let id_a = r.interrupted_run(1, &a, Some(&format!("worktrees/p/{a}")));
    let id_b = r.interrupted_run(2, &b, Some(&format!("worktrees/p/{b}")));
    assert_eq!(r.run_echo(Some(&a)).code, 3);
    assert_eq!(r.run_echo(Some(&b)).code, 3);
    let o = r.run(&["recover", "--abandon", &id_a]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.status_in_record(&id_a), "abandoned");
    let o = r.run_echo(Some(&a));
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.run_echo(Some(&b)).code, 3, "B's own interrupted run still blocks B ({id_b})");
}

#[test]
fn a_run_job_that_panics_stays_running_and_blocks_only_its_own_folder() {
    let r = Root::new("panicrun");
    let (a, b) = (r.started("a"), r.started("b"));
    let gate = r.dir.join("scratch/panic.gate");
    fs::write(&gate, "").unwrap();
    let line = "echo changed> a.txt & ping -n 30 127.0.0.1 >nul";
    let o = Command::new(bin())
        .args(["run", "p", "--task", &a, "--", line])
        .env("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB", &gate)
        .current_dir(&r.dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let (so, se) = (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned());
    assert_eq!(o.status.code(), Some(1), "{so}{se}");
    assert!(se.contains("TEST HOOK ACTIVE") && se.contains("PANIC in job"), "{se}");
    assert!(so.contains("killing orphaned process tree"), "the verified child is killed: {so}");
    assert!(se.contains("broadsword recover"), "prints the recover hint: {se}");
    let id = fs::read_dir(r.dir.join("state/jobs")).unwrap().flatten().next().unwrap().file_name().to_string_lossy().trim_end_matches(".rec").to_string();
    assert_eq!(r.status_in_record(&id), "running", "a panicked run is left running so recover offers the rollback");
    let rec = fs::read_to_string(r.dir.join(format!("state/jobs/{id}.rec"))).unwrap();
    assert!(rec.contains(&format!("source = worktrees/p/{a}")) && rec.contains("panic = ") && rec.contains("test hook"), "{rec}");
    // the panicked run's pid is gone from the record, so it reads as interrupted even while this process were alive
    assert!(rec.lines().any(|l| l == "pid = " || l == "pid ="), "{rec}");
    // blocks its own folder only
    assert_eq!(r.run_echo(Some(&a)).code, 3);
    assert_eq!(r.run_echo(Some(&b)).code, 0);
    assert_eq!(r.run_echo(None).code, 0);
    // recover rolls the worktree back as before
    // a.txt is tracked ("one\n"); the panicked run overwrote it
    let read = |p: PathBuf| fs::read_to_string(p).unwrap().replace("\r\n", "\n");
    assert_eq!(read(r.wt(&a).join("a.txt")).trim(), "changed");
    let o = r.run(&["recover", &id]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(read(r.wt(&a).join("a.txt")), "one\n", "rolled back in its own worktree: {}", o.stdout);
    assert_eq!(read(r.wt(&b).join("a.txt")), "one\n", "the other worktree is untouched");
    assert_eq!(r.status_in_record(&id), "rolled_back");
    assert_eq!(r.run_echo(Some(&a)).code, 0);
}

impl Root {
    /// Run a `run` job in task `task`'s worktree that overwrites a.txt and is then left interrupted by the panic test hook
    /// (a real checkpoint, a real change, a dead broadsword). Returns its job id.
    fn panicked_run(&self, task: &str) -> String {
        let before: Vec<String> = self.job_ids();
        let gate = self.dir.join("scratch/panic.gate");
        fs::write(&gate, "").unwrap();
        let o = Command::new(bin())
            .args(["run", "p", "--task", task, "--", "echo changed> a.txt & ping -n 30 127.0.0.1 >nul"])
            .env("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB", &gate)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let _ = fs::remove_file(&gate);
        assert_eq!(o.status.code(), Some(1), "{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
        let id = self.job_ids().into_iter().find(|i| !before.contains(i)).expect("the panicked run left a record");
        assert_eq!(self.status_in_record(&id), "running");
        id
    }

    fn job_ids(&self) -> Vec<String> {
        fs::read_dir(self.dir.join("state/jobs")).into_iter().flatten().flatten().map(|e| e.file_name().to_string_lossy().trim_end_matches(".rec").to_string()).collect()
    }

    fn a_txt(&self, task: &str) -> String {
        fs::read_to_string(self.wt(task).join("a.txt")).unwrap().trim().to_string()
    }
}

/// Audit S1: a manual rollback reverts only the job it names. Two interrupted runs in two worktrees; plain `recover`
/// lists them and changes no file; `recover <id-A>` rolls back A only; B is left alone and still listed; the automatic
/// recovery before a job never rolls anything back.
#[test]
fn recover_with_an_id_rolls_back_only_that_run_and_plain_recover_changes_no_file() {
    let r = Root::new("recone");
    let (a, b) = (r.started("a"), r.started("b"));
    let (id_a, id_b) = (r.panicked_run(&a), r.panicked_run(&b));
    assert_eq!((r.a_txt(&a).as_str(), r.a_txt(&b).as_str()), ("changed", "changed"));

    // the automatic recovery that every job runs first (here: a job in the main checkout) never rolls back
    assert_eq!(r.run_echo(None).code, 0);
    assert_eq!((r.a_txt(&a).as_str(), r.a_txt(&b).as_str()), ("changed", "changed"), "auto recovery must not roll back");
    assert_eq!((r.status_in_record(&id_a).as_str(), r.status_in_record(&id_b).as_str()), ("running", "running"));

    // plain `recover`: lists both with their source folder and the command, changes no file, exits non-zero
    let o = r.run(&["recover"]);
    assert_eq!(o.code, 1, "{}{}", o.stdout, o.stderr);
    for (id, t) in [(&id_a, &a), (&id_b, &b)] {
        assert!(o.stdout.contains(&format!("job {id}")) && o.stdout.contains(&format!("worktrees/p/{t}")), "{}", o.stdout);
        assert!(o.stdout.contains(&format!("broadsword recover {id}")), "{}", o.stdout);
        assert!(o.stderr.contains(id.as_str()), "{}", o.stderr);
    }
    assert!(o.stdout.contains("started:"), "{}", o.stdout);
    assert_eq!((r.a_txt(&a).as_str(), r.a_txt(&b).as_str()), ("changed", "changed"), "plain recover must change no file");
    assert_eq!((r.status_in_record(&id_a).as_str(), r.status_in_record(&id_b).as_str()), ("running", "running"));

    // recover <id-A>: only A
    let o = r.run(&["recover", &id_a]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.a_txt(&a), "one", "A is rolled back: {}", o.stdout);
    assert_eq!(r.a_txt(&b), "changed", "B's folder is not touched");
    assert_eq!((r.status_in_record(&id_a).as_str(), r.status_in_record(&id_b).as_str()), ("rolled_back", "running"));
    assert!(!o.stdout.contains(&id_b), "B is not even mentioned by a rollback of A: {}", o.stdout);

    // B is still listed, A no longer is
    let o = r.run(&["recover"]);
    assert_eq!(o.code, 1, "{}{}", o.stdout, o.stderr);
    assert!(o.stdout.contains(&format!("broadsword recover {id_b}")) && !o.stdout.contains(&id_a), "{}", o.stdout);
    assert_eq!(r.a_txt(&b), "changed");
    let o = r.run(&["recover", &id_b]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!(r.a_txt(&b), "one");
}

/// `recover --all` keeps the old all-at-once behaviour for a human who means it.
#[test]
fn recover_all_rolls_back_every_interrupted_run() {
    let r = Root::new("recall");
    let (a, b) = (r.started("a"), r.started("b"));
    let (id_a, id_b) = (r.panicked_run(&a), r.panicked_run(&b));
    let o = r.run(&["recover", "--all"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    assert_eq!((r.a_txt(&a).as_str(), r.a_txt(&b).as_str()), ("one", "one"));
    assert_eq!((r.status_in_record(&id_a).as_str(), r.status_in_record(&id_b).as_str()), ("rolled_back", "rolled_back"));
}

/// `recover <id>` is refused (exit 2, nothing changed) for anything that is not an interrupted `run`: no such record, a
/// check job, an already finished record, a run whose broadsword is still alive, and `--all` together with an id.
#[test]
fn recover_with_an_id_refuses_anything_but_an_interrupted_run() {
    let r = Root::new("recrefuse");
    let a = r.started("a");
    let id = r.panicked_run(&a);
    let rec_path = |i: &str| r.dir.join(format!("state/jobs/{i}.rec"));
    let jobs = r.dir.join("state/jobs");
    let alive = format!("id = 20260101-000009-p-run\nproject = p\nkind = run\nsource = worktrees/p/{a}\ntask = {a}\nstatus = running\npid = {}\ncheckpoint = pending\n", std::process::id());
    fs::write(jobs.join("20260101-000009-p-run.rec"), &alive).unwrap(); // this very test process: alive
    fs::write(jobs.join("20260101-000008-p-check.rec"), format!("id = 20260101-000008-p-check\nproject = p\nkind = check\nstatus = running\npid = {}\nstep = build\n", dead_pid())).unwrap();
    fs::write(jobs.join("20260101-000007-p-run.rec"), format!("id = 20260101-000007-p-run\nproject = p\nkind = run\nstatus = done\npid = {}\ncheckpoint = none\n", dead_pid())).unwrap();
    let snapshot = |ids: &[&str]| ids.iter().map(|i| fs::read_to_string(rec_path(i)).unwrap()).collect::<Vec<_>>();
    let ids = [id.as_str(), "20260101-000009-p-run", "20260101-000008-p-check", "20260101-000007-p-run"];
    let before = snapshot(&ids);
    for (arg, why) in [
        ("20260101-000009-p-run", "still running"),
        ("20260101-000008-p-check", "only an interrupted `run`"),
        ("20260101-000007-p-run", "only an interrupted `run`"),
        ("no-such-job", "no job record"),
        ("../x", "not a valid job id"),
    ] {
        let o = r.run(&["recover", arg]);
        assert_eq!(o.code, 2, "{arg}: {}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains(why), "{arg}: {}", o.stderr);
    }
    assert_eq!(snapshot(&ids), before, "a refusal changes no record");
    assert_eq!(r.a_txt(&a), "changed", "a refusal changes no file");
    let o = r.run(&["recover", "--all", &id]);
    assert_eq!(o.code, 2, "--all together with an id is refused, nothing rolled back: {}{}", o.stdout, o.stderr);
    assert_eq!(r.a_txt(&a), "changed");
    assert_eq!(r.status_in_record(&id), "running");
}

/// A `run` job's command sees CARGO_TARGET_DIR = the main root's shared build folder, so a `cargo ...` inside `run` from
/// a worktree never builds into the worktree (found when `run -- cargo test` in a Broadsword worktree made its own build/).
#[test]
fn a_run_job_in_a_worktree_uses_the_shared_build_folder() {
    let r = Root::new("runtarget");
    let a = r.started("a");
    let o = r.run(&["run", "p", "--task", &a, "--", "echo", "TD=%CARGO_TARGET_DIR%"]);
    assert_eq!(o.code, 0, "{}{}", o.stdout, o.stderr);
    let td = o.stdout.lines().find_map(|l| l.trim().strip_prefix("TD=")).unwrap_or_else(|| panic!("no TD= line: {}", o.stdout)).trim().to_string();
    let want = r.dir.join("build");
    let norm = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    assert_eq!(norm(&td), norm(&want.to_string_lossy()), "run must point cargo at the main root's build folder");
    assert!(!r.wt(&a).join("build").join("debug").exists());
}
