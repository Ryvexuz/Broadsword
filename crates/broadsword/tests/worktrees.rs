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

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");
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
        let toml = "root = \".\"\ndisk_floor_gb = 1\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\ndisk_check_every_seconds = 60\nlog_max_mb = 50\n\
                    ram_warn_percent = 90\ncpu_warn_percent = 90\nalert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
                    state_dir = \"state\"\ninstall_dir = \"build/bin\"\nmcp_call_timeout_seconds = 600\nmcp_output_max_kb = 1024\n\
                    checkpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 0\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\n";
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn p(&self) -> PathBuf {
        self.dir.join("projects/p")
    }

    fn run(&self, args: &[&str]) -> Out {
        let o = Command::new(BIN).args(args).current_dir(&self.dir).stdin(Stdio::null()).output().unwrap();
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
    assert!(o.stderr.contains("already exists") && o.stderr.contains("rolled back to `open`"), "{}", o.stderr);
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
    assert!(o.stderr.contains("not a Git repo") && o.stderr.contains("rolled back"), "{}", o.stderr);
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
    // an ignored file does not count (build output): clean up the others first, then check this one
    let d = r.started("tests");
    fs::write(r.p().join(".gitignore"), "ignored.bin\n").unwrap();
    git(&r.p(), &["add", ".gitignore"]);
    git(&r.p(), &["commit", "-q", "-m", "ignore"]);
    git(&r.wt(&d), &["merge", "-q", "main"]); // pick up .gitignore in the worktree
    r.merge_into_main(&d);
    fs::write(r.wt(&d).join("ignored.bin"), "x\n").unwrap();
    let o = r.run(&["task", "done", &d]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("worktree removed"), "ignored files are disposable: {}{}", o.stdout, o.stderr);
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

// ---- MCP ---------------------------------------------------------------------------------

fn mcp_call(r: &Root, name: &str, args: &str) -> (bool, String, i64) {
    let mut child = Command::new(BIN).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
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
