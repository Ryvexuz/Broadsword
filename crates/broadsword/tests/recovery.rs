//! Kill-and-recover tests. Each test makes a throwaway Broadsword root under the repo's `scratch/`
//! folder with a throwaway Git repo as its project, runs the real binary, hard-kills only the
//! broadsword process (a crash: its child is left orphaned), then runs `broadsword recover`.
//! Everything is deleted afterwards. Never touches the real repo or the crash sim.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

mod support;
use support::bin;

// The product's own process-identity code, so the test cleanup applies the very same exact-match rule.
#[allow(dead_code)]
#[path = "../src/sys.rs"]
mod sys;

/// Kill the process tree of a recorded child pid, but only if the live process is verifiably the recorded one:
/// its creation time must equal the recorded `child_start` exactly (`sys::exact_match`). A record without a start
/// time, or a pid that now belongs to another process, is never killed.
fn kill_recorded_if_same(rec: &str) {
    let Some(p) = Proc::recorded(rec) else { return };
    if p.alive() {
        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &p.pid.to_string()]).output();
    }
}

/// One process, named by its pid AND its creation time. Windows hands a freed pid to a new process within seconds, so
/// every "is it still running?" / "is it gone?" check in these tests asks about this pair, never about a bare pid: a bare
/// pid check (`tasklist /FI "PID eq N"`) failed at the old recovery.rs:296 when, after `recover` had killed the orphan,
/// a sibling test's `git.exe` got the same pid (t-20261120-002351). This is the only liveness check in this file.
#[derive(Clone, Copy, Debug)]
struct Proc {
    pid: u32,
    start: Option<u64>,
}

impl Proc {
    /// The child a job record names (`child_pid` + `child_start`); `None` when the record names no child.
    fn recorded(rec: &str) -> Option<Proc> {
        let pid = Root::field(rec, "child_pid").parse().ok()?;
        Some(Proc { pid, start: Root::field(rec, "child_start").parse().ok() })
    }

    /// The process that has `pid` right now (its creation time read now). Only for a pid this test keeps open (a
    /// `Child` it holds, so the pid cannot be reused) or one it has just learned; a pid already gone has no start.
    fn now(pid: u32) -> Proc {
        Proc { pid, start: sys::start_time(pid) }
    }

    /// Is this exact process running? The product's strict identity rule (`sys::exact_match`): the live process with
    /// this pid must have this creation time. A pid that now belongs to another process, or no known start, is not it.
    fn alive(&self) -> bool {
        sys::exact_match(self.start, sys::process_state(self.pid))
    }
}

/// The child a record names, which the test then watches; panics (naming the record) when there is none.
fn recorded_child(rec: &str) -> Proc {
    Proc::recorded(rec).unwrap_or_else(|| panic!("the record names no child process:\n{rec}"))
}

struct Root {
    dir: PathBuf,
    children: Vec<Child>,
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_INDEX_FILE").output().expect("git must be installed");
    assert!(out.status.success(), "git {args:?}: {}", text(&out.stderr));
    text(&out.stdout).trim().to_string()
}

/// Shell line that changes tracked.txt, creates new.txt, then sleeps ~30 s.
fn sleepy_change() -> &'static str {
    if cfg!(windows) {
        "echo changed> tracked.txt & echo partial> new.txt & ping -n 30 127.0.0.1 >nul"
    } else {
        "echo changed > tracked.txt; echo partial > new.txt; sleep 30"
    }
}

fn kill_hard(pid: u32) {
    // No /T: only this process dies, like a crash. Its child is left running.
    let st = Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output().expect("taskkill");
    assert!(st.status.success(), "taskkill: {}", text(&st.stderr));
}

fn wait_until(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for {what}");
}

impl Root {
    fn new(name: &str) -> Root {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        let dir = scratch.join(format!("rec-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("projects")).unwrap();
        let toml = support::config(&[]);
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir, children: Vec::new() }
    }

    fn project_dir(&self) -> PathBuf {
        self.dir.join("projects/p")
    }

    /// Project `p` (tiny lib, `tracked.txt` = original). `lib_rs` is its code; `as_git` makes it a repo with one commit.
    fn project(&self, lib_rs: &str, as_git: bool) {
        let p = self.project_dir();
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(p.join("Cargo.toml"), "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        fs::write(p.join("src/lib.rs"), lib_rs).unwrap();
        fs::write(p.join("tracked.txt"), "original\n").unwrap();
        fs::write(p.join(".gitignore"), ".env\n.env2\n*.log\n").unwrap();
        if as_git {
            git(&p, &["init", "-q"]);
            git(&p, &["config", "user.email", "t@t"]);
            git(&p, &["config", "user.name", "t"]);
            git(&p, &["config", "core.autocrlf", "false"]);
            git(&p, &["add", "-A"]);
            git(&p, &["commit", "-q", "-m", "init"]);
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(bin());
        c.args(args).current_dir(&self.dir).stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().expect("failed to start broadsword")
    }

    /// Start broadsword in the background (stdout/stderr discarded).
    fn spawn(&mut self, args: &[&str]) -> u32 {
        let child = self.cmd(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let pid = child.id();
        self.children.push(child);
        pid
    }

    fn records(&self) -> Vec<(String, String)> {
        let dir = self.dir.join("state/jobs");
        let mut v: Vec<(String, String)> = fs::read_dir(dir)
            .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "rec")).map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read_to_string(e.path()).unwrap())).collect())
            .unwrap_or_default();
        v.sort_by_key(|(name, _)| name.trim_end_matches(".rec").to_string()); // id order, as `broadsword jobs` lists them
        v
    }

    /// All records that contain the line `key = value`; each as its id.
    fn field(rec: &str, key: &str) -> String {
        rec.lines().find_map(|l| l.strip_prefix(&format!("{key} = "))).unwrap_or("").to_string()
    }

    fn only_record(&self) -> String {
        let r = self.records();
        assert_eq!(r.len(), 1, "{r:?}");
        r[0].1.clone()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        for c in &mut self.children {
            // kill the whole tree so no orphan keeps a folder open
            let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &c.id().to_string()]).output();
            let _ = c.kill();
            let _ = c.wait();
        }
        // orphans recorded by a crashed job
        for (_, rec) in self.records() {
            kill_recorded_if_same(&rec);
        }
        std::thread::sleep(Duration::from_millis(300));
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn run_job_checkpoints_and_keeps_its_changes() {
    let mut r = Root::new("ckpt");
    r.project("", true);
    let p = r.project_dir();
    fs::write(p.join("tracked.txt"), "dirty before job\n").unwrap(); // uncommitted work must be captured too
    fs::write(p.join("untracked.txt"), "u\n").unwrap();
    let head = git(&p, &["rev-parse", "HEAD"]);
    let line = if cfg!(windows) { "echo job> tracked.txt" } else { "echo job > tracked.txt" };
    let out = r.run(&["run", "p", "--", line]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    let rec = r.only_record();
    assert_eq!(Root::field(&rec, "status"), "done", "{rec}");
    let ckpt = Root::field(&rec, "checkpoint");
    assert!(ckpt.starts_with("refs/broadsword/checkpoints/"), "{rec}");
    assert_eq!(git(&p, &["show", &format!("{ckpt}:tracked.txt")]), "dirty before job");
    assert_eq!(git(&p, &["show", &format!("{ckpt}:untracked.txt")]), "u");
    assert_eq!(git(&p, &["rev-parse", "HEAD"]), head, "HEAD unchanged");
    assert_eq!(fs::read_to_string(p.join("tracked.txt")).unwrap().trim(), "job", "a finished job keeps its changes");
    // the exit code of the command is the job's
    let out = r.run(&["run", "p", "--", "exit 7"]);
    assert_eq!(out.status.code(), Some(7));
    let statuses: Vec<String> = r.records().iter().map(|(_, c)| Root::field(c, "status")).collect();
    assert_eq!(statuses, ["done", "failed"]);
    r.children.clear();
}

#[test]
fn run_in_a_folder_that_is_not_a_git_repo_is_refused_but_check_still_runs() {
    let r = Root::new("nogit");
    r.project("#[cfg(test)] mod t { #[test] fn a() {} }\n", false);
    let out = r.run(&["run", "p", "--", "echo hi"]);
    assert_eq!(out.status.code(), Some(3), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stderr).contains("not a Git repository"));
    assert_eq!(Root::field(&r.only_record(), "status"), "refused");
    let out = r.run(&["check", "p"]);
    assert_eq!(out.status.code(), Some(0), "{}{}", text(&out.stdout), text(&out.stderr));
    let recs = r.records();
    assert_eq!(recs.len(), 2);
    assert!(recs.iter().any(|(_, c)| Root::field(c, "kind") == "check" && Root::field(c, "checkpoint") == "none" && Root::field(c, "status") == "done"), "{recs:?}");
}

#[test]
fn kill_mid_run_job_then_recover_rolls_back_and_keeps_partial_work() {
    let mut r = Root::new("killrun");
    r.project("", true);
    let p = r.project_dir();
    let head = git(&p, &["rev-parse", "HEAD"]);
    let bs_pid = r.spawn(&["run", "p", "--", sleepy_change()]);
    wait_until("running record with child and partial changes", 60, || {
        let recs = r.records();
        recs.len() == 1 && !Root::field(&recs[0].1, "child_pid").is_empty() && p.join("new.txt").exists() && fs::read_to_string(p.join("tracked.txt")).unwrap().trim() == "changed"
    });
    let rec = r.only_record();
    assert_eq!(Root::field(&rec, "status"), "running");
    let (id, child) = (Root::field(&rec, "id"), recorded_child(&rec));
    assert_eq!(Root::field(&rec, "pid"), bs_pid.to_string());

    kill_hard(bs_pid);
    assert!(child.alive(), "the orphaned command should still be running after the crash: {child:?}");

    // plain `recover` only lists the interrupted run (and kills its verified orphan): no file changes, exit 1
    let listed = r.run(&["recover"]);
    assert_eq!(listed.status.code(), Some(1), "{}{}", text(&listed.stdout), text(&listed.stderr));
    assert!(text(&listed.stdout).contains(&format!("broadsword recover {id}")), "{}", text(&listed.stdout));
    assert!(text(&listed.stdout).contains("killing orphaned process tree"), "{}", text(&listed.stdout));
    assert_eq!(fs::read_to_string(p.join("tracked.txt")).unwrap().trim(), "changed", "plain recover must not roll back");
    assert!(p.join("new.txt").exists(), "plain recover must not roll back");
    assert_eq!(Root::field(&r.only_record(), "status"), "running");
    r.children.clear();

    let out = r.run(&["recover", &id]);
    let so = text(&out.stdout);
    println!("{so}{}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}");
    assert!(so.contains("rolled back"), "{so}");
    assert!(so.contains("git -C"), "tells how to see the partial work: {so}");
    wait_until("orphan to die", 10, || !child.alive()); // killed by the plain recover above

    // working tree is back at the checkpoint state; HEAD and the index are untouched
    assert_eq!(fs::read_to_string(p.join("tracked.txt")).unwrap(), "original\n");
    assert!(!p.join("new.txt").exists());
    assert_eq!(git(&p, &["rev-parse", "HEAD"]), head);
    assert_eq!(git(&p, &["status", "--porcelain"]), "");
    // the partial changes are kept
    let int = format!("refs/broadsword/interrupted/{id}/1");
    assert_eq!(git(&p, &["show", &format!("{int}:tracked.txt")]), "changed");
    assert_eq!(git(&p, &["show", &format!("{int}:new.txt")]), "partial");
    let ckpt = format!("refs/broadsword/checkpoints/{id}");
    assert_eq!(git(&p, &["show", &format!("{ckpt}:tracked.txt")]), "original");
    assert!(git(&p, &["diff", "--stat", &ckpt, &int]).contains("new.txt"));
    let rec = r.only_record();
    assert_eq!(Root::field(&rec, "status"), "rolled_back", "{rec}");
    assert_eq!(Root::field(&rec, "interrupted"), int);

    // a second recover has nothing left to do
    let again = text(&r.run(&["recover"]).stdout);
    assert!(again.contains("nothing to do"), "{again}");
    r.children.clear();
}

#[test]
fn kill_mid_check_job_then_recover_resumes_and_completes() {
    let mut r = Root::new("killcheck");
    // the test sleeps only while sleep.flag exists, so the resumed run can finish quickly
    r.project("#[cfg(test)] mod t { #[test] fn slow() { if std::path::Path::new(\"sleep.flag\").exists() { std::thread::sleep(std::time::Duration::from_secs(60)); } } }\n", false);
    let p = r.project_dir();
    fs::write(p.join("sleep.flag"), "").unwrap();
    let bs_pid = r.spawn(&["check", "p"]);
    wait_until("the test step to be running", 120, || {
        let recs = r.records();
        recs.len() == 1
            && Root::field(&recs[0].1, "step") == "test"
            && !Root::field(&recs[0].1, "child_pid").is_empty()
            && fs::read_to_string(Root::field(&recs[0].1, "log")).map(|l| l.contains("running 1 test")).unwrap_or(false)
    });
    let rec = r.only_record();
    let (old_id, child) = (Root::field(&rec, "id"), recorded_child(&rec));
    kill_hard(bs_pid);
    assert!(child.alive(), "orphaned cargo test should still be running: {child:?}");
    fs::remove_file(p.join("sleep.flag")).unwrap();

    let out = r.run(&["recover"]);
    let so = text(&out.stdout);
    println!("{so}{}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert!(so.contains("killing orphaned process tree"), "{so}");
    assert!(so.contains("resuming `check p` from step `test`"), "{so}");
    assert!(so.contains("test: ok"), "{so}");
    assert!(!child.alive(), "recover must have killed the orphan: {child:?}");

    let recs = r.records();
    assert_eq!(recs.len(), 2, "{recs:?}");
    let old = recs.iter().find(|(_, c)| Root::field(c, "id") == old_id).unwrap();
    let new = recs.iter().find(|(_, c)| Root::field(c, "id") != old_id).unwrap();
    assert_eq!(Root::field(&old.1, "status"), "resumed");
    assert_eq!(Root::field(&old.1, "resumed_as"), Root::field(&new.1, "id"));
    assert_eq!(Root::field(&new.1, "resumed_from"), old_id);
    assert_eq!(Root::field(&new.1, "status"), "done");
    r.children.clear();
}

#[test]
fn recover_leaves_a_live_job_alone() {
    let mut r = Root::new("live");
    r.project("", true);
    let p = r.project_dir();
    let bs_pid = r.spawn(&["run", "p", "--", sleepy_change()]);
    wait_until("running job", 60, || r.records().len() == 1 && !Root::field(&r.records()[0].1, "child_pid").is_empty() && p.join("new.txt").exists());
    let before = r.only_record();
    let out = r.run(&["recover"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{so}");
    assert!(so.contains("still running") && so.contains("left alone"), "{so}");
    assert_eq!(r.only_record(), before, "record untouched");
    assert!(p.join("new.txt").exists(), "working tree untouched");
    assert!(recorded_child(&before).alive());
    let _ = bs_pid;
}

#[test]
fn recover_with_nothing_to_do_is_a_no_op() {
    let r = Root::new("noop");
    let out = r.run(&["recover"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stdout).contains("nothing to do"));
    assert!(r.records().is_empty());
    let out = r.run(&["jobs"]);
    assert_eq!(out.status.code(), Some(0));
}

// ---- regression tests for the review findings ------------------------------------------------

impl Root {
    /// Write a record by hand, as if left behind by a crashed broadsword. A record naming a `pid` without a `pid_start` gets
    /// `pid_start = 1` (a creation time no live process can have): the `dead_pid()` it names may be reused by any new process
    /// under load, and recover's lenient liveness check (kept for old records) would then take the record's job as still
    /// running. With a creation time that cannot match, the record's broadsword reads as gone whoever holds the pid now.
    fn fake_record(&self, id: &str, fields: &[(&str, &str)]) {
        let dir = self.dir.join("state/jobs");
        fs::create_dir_all(&dir).unwrap();
        let mut body: String = fields.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
        if fields.iter().any(|(k, _)| *k == "pid") && !fields.iter().any(|(k, _)| *k == "pid_start") {
            body.push_str("pid_start = 1\n");
        }
        fs::write(dir.join(format!("{id}.rec")), format!("id = {id}\n{body}")).unwrap();
    }

    fn sleeper(&mut self) -> u32 {
        let c = Command::new("ping").args(["-n", "60", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
        let pid = c.id();
        self.children.push(c);
        pid
    }
}

fn dead_pid() -> u32 {
    let mut c = Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
    let pid = c.id();
    c.wait().unwrap();
    pid
}

/// Blocker 3, secrets rule: a killed job that deleted a git-ignored file gets it back, but a secret file (`.env`) is never
/// captured, restored or deleted: it keeps the bytes the job left in it.
#[test]
fn kill_mid_run_restores_ignored_files_but_never_touches_secret_files() {
    let mut r = Root::new("killenv");
    r.project("", true);
    let p = r.project_dir();
    fs::write(p.join(".env"), "SECRET=1\r\n").unwrap();
    fs::write(p.join("data.log"), "OTHER=2\n").unwrap();
    let env_hash = git(&p, &["hash-object", "--no-filters", ".env"]);
    let line = if cfg!(windows) { "echo changed> .env & del data.log & echo made> made.log & ping -n 30 127.0.0.1 >nul" } else { "echo changed > .env; rm data.log; echo made > made.log; sleep 30" };
    let bs_pid = r.spawn(&["run", "p", "--", line]);
    wait_until("the job to have changed the ignored files", 60, || {
        r.records().len() == 1 && !Root::field(&r.records()[0].1, "child_pid").is_empty() && p.join("made.log").exists() && !p.join("data.log").exists()
    });
    let rec = r.only_record();
    let id = Root::field(&rec, "id");
    assert_eq!(Root::field(&rec, "secret_skipped"), ".env", "the record lists the secret file by name: {rec}");
    kill_hard(bs_pid);
    let out = r.run(&["recover", &id]);
    let so = text(&out.stdout);
    println!("{so}");
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert_eq!(fs::read_to_string(p.join(".env")).unwrap().trim(), "changed", "rollback leaves the secret file with the job's bytes");
    assert_eq!(fs::read(p.join("data.log")).unwrap(), b"OTHER=2\n", "another ignored file is restored");
    assert!(p.join("made.log").exists(), "ignored files the job created are never deleted");
    assert!(!so.contains("not restored"), "{so}");
    let ign = format!("refs/broadsword/interrupted-ignored/{id}/1");
    assert_eq!(git(&p, &["show", &format!("{ign}:made.log")]).trim(), "made");
    assert!(git(&p, &["ls-tree", "-r", "--name-only", &ign]).lines().all(|f| f != ".env"), "the interrupted snapshot holds no .env");
    assert_eq!(git(&p, &["rev-parse", "--verify", "-q", &format!("refs/broadsword/checkpoints-ignored/{id}")]).len(), 40);
    assert_no_secret_object(&p, &env_hash);
    let changed_hash = git(&p, &["hash-object", "--no-filters", ".env"]);
    assert_no_secret_object(&p, &changed_hash);
}

/// No object reachable from any refs/broadsword/* ref has this id, and the object store has no such object at all.
fn assert_no_secret_object(p: &Path, hash: &str) {
    let refs = git(p, &["for-each-ref", "--format=%(refname)", "refs/broadsword/"]);
    let names: Vec<&str> = refs.lines().collect();
    if !names.is_empty() {
        let mut args = vec!["rev-list", "--objects"];
        args.extend(names.iter().copied());
        let objs = git(p, &args);
        assert!(!objs.lines().any(|l| l.starts_with(hash)), "blob {hash} of a secret file is reachable from a broadsword ref");
    }
    let out = Command::new("git").args(["cat-file", "-e", hash]).current_dir(p).env_remove("GIT_DIR").output().unwrap();
    assert!(!out.status.success(), "an object with a secret file's bytes ({hash}) exists in the object store");
}

/// A `run` in a repo with ignored secret files: nothing is stored from them, the record lists them by name, other ignored files are kept.
#[test]
fn run_checkpoint_never_stores_secret_files() {
    let mut r = Root::new("secretckpt");
    r.project("", true);
    let p = r.project_dir();
    fs::write(p.join(".gitignore"), ".env\n.env.local\nsub/.env\n*.log\n").unwrap();
    git(&p, &["add", "-A"]);
    git(&p, &["commit", "-q", "-m", "ignore secrets"]);
    fs::write(p.join(".env"), "API_KEY=sk-fake-secret-value-1\n").unwrap();
    fs::write(p.join(".env.local"), "API_KEY=sk-fake-secret-value-2\n").unwrap();
    fs::create_dir_all(p.join("sub")).unwrap();
    fs::write(p.join("sub/.env"), "API_KEY=sk-fake-secret-value-3\n").unwrap();
    fs::write(p.join("keep.log"), "ordinary ignored file\n").unwrap();
    let hashes: Vec<String> = [".env", ".env.local", "sub/.env"].iter().map(|f| git(&p, &["hash-object", "--no-filters", f])).collect();
    let out = r.run(&["run", "p", "--", "echo hi"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    let rec = r.only_record();
    let ign = Root::field(&rec, "checkpoint_ignored");
    assert!(ign.starts_with("refs/broadsword/checkpoints-ignored/"), "{rec}");
    assert_eq!(Root::field(&rec, "secret_skipped"), ".env|.env.local|sub/.env", "{rec}");
    assert_eq!(git(&p, &["show", &format!("{ign}:keep.log")]).trim(), "ordinary ignored file", "other ignored files are still captured");
    for h in &hashes {
        assert_no_secret_object(&p, h);
    }
    assert!(!rec.contains("sk-fake"), "the record never holds secret content: {rec}");
    assert!(!so.contains("sk-fake"), "{so}");
    r.children.clear();
}

/// A `run` that deletes `.env` is interrupted: rollback keeps it deleted and reports no error.
#[test]
fn rollback_keeps_a_deleted_secret_file_deleted() {
    let mut r = Root::new("killdelenv");
    r.project("", true);
    let p = r.project_dir();
    fs::write(p.join(".env"), "SECRET=1\n").unwrap();
    let line = if cfg!(windows) { "del .env & echo changed> tracked.txt & ping -n 30 127.0.0.1 >nul" } else { "rm .env; echo changed > tracked.txt; sleep 30" };
    let bs_pid = r.spawn(&["run", "p", "--", line]);
    wait_until("the job to have deleted .env", 60, || {
        r.records().len() == 1 && !Root::field(&r.records()[0].1, "child_pid").is_empty() && !p.join(".env").exists() && fs::read_to_string(p.join("tracked.txt")).unwrap().trim() == "changed"
    });
    kill_hard(bs_pid);
    let id = Root::field(&r.only_record(), "id");
    let out = r.run(&["recover", &id]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert!(!p.join(".env").exists(), "a secret file the job deleted is not recreated");
    assert_eq!(fs::read_to_string(p.join("tracked.txt")).unwrap().trim(), "original", "tracked files roll back as before");
}

/// Only `run` takes a checkpoint: check (build + test) creates no refs/broadsword/* ref and its record says so.
#[test]
fn check_takes_no_checkpoint_and_creates_no_refs() {
    let r = Root::new("nockpt");
    r.project("#[cfg(test)] mod t { #[test] fn a() {} }\n", true);
    let p = r.project_dir();
    let out = r.run(&["check", "p"]);
    assert_eq!(out.status.code(), Some(0), "{}{}", text(&out.stdout), text(&out.stderr));
    let rec = r.only_record();
    assert_eq!(Root::field(&rec, "kind"), "check");
    assert_eq!(Root::field(&rec, "checkpoint"), "none", "{rec}");
    assert!(Root::field(&rec, "checkpoint_reason").contains("only `run` takes a checkpoint"), "{rec}");
    assert_eq!(git(&p, &["for-each-ref", "refs/broadsword/"]), "", "no checkpoint refs");
    assert!(!text(&out.stdout).contains("checkpoint:"), "{}", text(&out.stdout));
}

/// Item 9: the automatic recovery before a job never rolls back a `run`; it refuses new jobs on that project.
#[test]
fn auto_recover_does_not_roll_back_and_refuses_new_jobs_on_that_project() {
    let mut r = Root::new("auto");
    r.project("", true);
    let p = r.project_dir();
    let bs_pid = r.spawn(&["run", "p", "--", sleepy_change()]);
    wait_until("partial changes", 60, || r.records().len() == 1 && !Root::field(&r.records()[0].1, "child_pid").is_empty() && p.join("new.txt").exists());
    let child = recorded_child(&r.only_record());
    kill_hard(bs_pid);
    let out = r.run(&["run", "p", "--", "echo hi"]);
    let se = text(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{}{se}", text(&out.stdout));
    assert!(se.contains("broadsword recover"), "{se}");
    assert!(p.join("new.txt").exists(), "auto-recover must not touch the working tree");
    assert_eq!(Root::field(&r.only_record(), "status"), "running");
    wait_until("orphan to be killed by auto-recover", 10, || !child.alive());
    let id = Root::field(&r.only_record(), "id");
    assert!(se.contains(&format!("broadsword recover {id}")), "the refusal names the exact command: {se}");
    let out = r.run(&["recover", &id]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stdout));
    assert!(!p.join("new.txt").exists());
    assert_eq!(Root::field(&r.only_record(), "status"), "rolled_back");
}

/// Item 4: a live lock stops a second recover; a stale lock is taken over.
#[test]
fn recover_lock_blocks_a_second_recover_and_stale_locks_are_taken_over() {
    // Changed with the move to OS file locks (the old test planted lock files with a live or dead pid in them and
    // relied on stale-lock takeover; contents now mean nothing): a real holder blocks, garbage content does not.
    let r = Root::new("lock");
    fs::create_dir_all(r.dir.join("state")).unwrap();
    let held = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(r.dir.join("state/recover.lock")).unwrap();
    held.lock().unwrap();
    let out = r.run(&["recover"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("another recover is in progress"), "{}", text(&out.stderr));
    assert!(!text(&out.stderr).contains("delete"), "never tells anyone to delete the lock: {}", text(&out.stderr));
    drop(held);
    fs::write(r.dir.join("state/recover.lock"), format!("{}\n\n", dead_pid())).unwrap(); // stale-looking junk is just junk
    let out = r.run(&["recover"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
}

/// t-20261120-002351: the tests' liveness check is not fooled by pid reuse. A live process stands in for "the new owner
/// of a reused pid": the recorded process had the same pid but an earlier creation time, so it is gone, although a
/// process with that pid is running (which is what made the old bare-pid check fail after a correct `recover`).
#[test]
fn a_reused_pid_is_not_taken_for_the_recorded_process() {
    let mut r = Root::new("reuse");
    let pid = r.sleeper();
    let owner = Proc::now(pid);
    assert!(owner.alive(), "the running process itself is alive: {owner:?}");
    let start = owner.start.expect("a live process has a creation time");
    let rec = format!("child_pid = {pid}\nchild_start = {}\n", start - 1);
    let recorded = recorded_child(&rec);
    assert!(!recorded.alive(), "pid {pid} is running, but as another process than the recorded one: {recorded:?}");
    assert!(Proc::now(pid).alive(), "and the new owner is untouched");
}

/// Item 5: an orphan is killed only when its creation time matches exactly.
#[test]
fn unverifiable_child_pids_are_not_killed() {
    let mut r = Root::new("nokill");
    r.project("", true);
    let (a, b) = (r.sleeper(), r.sleeper());
    for (id, pid, start) in [("20260101-000001-p-run", a, ""), ("20260101-000002-p-run", b, "1")] {
        let dead = dead_pid().to_string();
        r.fake_record(id, &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &dead), ("child_pid", &pid.to_string()), ("child_start", start), ("checkpoint", "pending")]);
    }
    let out = r.run(&["recover"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "the two interrupted runs are listed, not rolled back: {so}{}", text(&out.stderr));
    assert!(so.contains("NOT killed"), "{so}");
    assert!(Proc::now(a).alive() && Proc::now(b).alive(), "neither process may be killed"); // the test holds both Child handles, so neither pid can be reused
}

/// t-20261120-174501: a job that crashed while its test step ran children side by side names them all in `child_pids`; recovery
/// kills every one whose creation time matches, and never one whose recorded time differs (a reused pid). `child_pid` names the
/// first, as the job writes it.
#[test]
fn recovery_kills_every_side_by_side_child_the_record_names() {
    let mut r = Root::new("killall");
    r.project("", true);
    let (a, b, c) = (r.sleeper(), r.sleeper(), r.sleeper());
    let start = |p: u32| Proc::now(p).start.expect("a live process has a creation time");
    let (sa, sb, sc) = (start(a), start(b), start(c));
    let pids = format!("{a}:{sa}|{b}:{sb}|{c}:{}", sc - 1); // c's recorded time is not its own: it stands for a reused pid
    let dead = dead_pid().to_string();
    r.fake_record(
        "20260101-000001-p-run",
        &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &dead), ("child_pid", &a.to_string()), ("child_start", &sa.to_string()), ("child_pids", &pids), ("checkpoint", "pending")],
    );
    let out = r.run(&["recover"]);
    let so = text(&out.stdout);
    let t = Instant::now();
    while Proc::now(a).alive() || Proc::now(b).alive() {
        assert!(t.elapsed() < Duration::from_secs(10), "both recorded children are killed: a alive {}, b alive {}: {so}{}", Proc::now(a).alive(), Proc::now(b).alive(), text(&out.stderr));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(so.contains(&format!("pid {a}")) && so.contains(&format!("pid {b}")), "{so}");
    assert!(Proc::now(c).alive(), "a pid whose creation time differs from the recorded one is never killed: {so}");
}

/// The pid-reuse race in the fake records cannot flip a result: a record whose `pid` is held by a live process right now (this
/// test's own, the worst case of reuse) is still read as interrupted, because `fake_record` gives it a creation time that
/// cannot match. Here it is a check job at the resume cap, so recover marks it failed instead of leaving it alone.
#[test]
fn a_fake_record_whose_pid_a_live_process_holds_is_still_taken_as_interrupted() {
    let r = Root::new("reusedpid");
    r.project("", false);
    let me = std::process::id().to_string();
    r.fake_record("20260101-000001-p-check", &[("project", "p"), ("kind", "check"), ("status", "running"), ("pid", &me), ("step", "build"), ("resume_count", "3")]);
    let out = r.run(&["recover"]);
    let so = text(&out.stdout);
    assert!(!so.contains("still running"), "a live process holding the recorded pid is not taken for the job: {so}");
    assert_eq!(Root::field(&r.only_record(), "status"), "failed", "{so}{}", text(&out.stderr));
}

/// An old record names its one child in `child_pid` only (no `child_pids`): it is still killed.
#[test]
fn a_record_with_only_child_pid_still_has_its_child_killed() {
    let mut r = Root::new("killone");
    r.project("", true);
    let a = r.sleeper();
    let sa = Proc::now(a).start.expect("a live process has a creation time").to_string();
    let dead = dead_pid().to_string();
    r.fake_record("20260101-000001-p-run", &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &dead), ("child_pid", &a.to_string()), ("child_start", &sa), ("checkpoint", "pending")]);
    let out = r.run(&["recover"]);
    let t = Instant::now();
    while Proc::now(a).alive() {
        assert!(t.elapsed() < Duration::from_secs(10), "the child_pid child is killed: {}", text(&out.stdout));
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Item 6: a failed rollback leaves the record running, keeps everything, and exits non-zero.
#[test]
fn failed_rollback_exits_nonzero_and_names_the_job() {
    let r = Root::new("failrb");
    r.project("", true);
    let dead = dead_pid().to_string();
    r.fake_record("20260101-000001-p-run", &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &dead), ("checkpoint", "refs/broadsword/checkpoints/does-not-exist")]);
    let out = r.run(&["recover", "20260101-000001-p-run"]);
    assert_eq!(out.status.code(), Some(1));
    let se = text(&out.stderr);
    assert!(se.contains("NEEDS ATTENTION") && se.contains("20260101-000001-p-run"), "{se}");
    assert_eq!(Root::field(&r.only_record(), "status"), "running");
}

/// Item 7: a job interrupted again and again is not resumed forever.
#[test]
fn resume_is_capped_by_max_resumes() {
    let r = Root::new("cap");
    r.project("", false);
    let dead = dead_pid().to_string();
    r.fake_record("20260101-000001-p-check", &[("project", "p"), ("kind", "check"), ("status", "running"), ("pid", &dead), ("step", "build"), ("resume_count", "3")]);
    let out = r.run(&["recover"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    let rec = r.only_record();
    assert_eq!(Root::field(&rec, "status"), "failed");
    assert!(Root::field(&rec, "failure_reason").contains("max_resumes"), "{rec}");
}

/// Way out: `recover --abandon <exact id>` keeps record and refs, touches no file, and lifts the refusal.
#[test]
fn abandon_lifts_the_refusal_without_touching_files() {
    let mut r = Root::new("abandon");
    r.project("", true);
    let p = r.project_dir();
    let bs_pid = r.spawn(&["run", "p", "--", sleepy_change()]);
    wait_until("partial changes", 60, || r.records().len() == 1 && !Root::field(&r.records()[0].1, "child_pid").is_empty() && p.join("new.txt").exists());
    let rec = r.only_record();
    let id = Root::field(&rec, "id");
    kill_hard(bs_pid);
    assert_eq!(r.run(&["run", "p", "--", "echo hi"]).status.code(), Some(3), "refused while the interrupted run is unresolved");
    // the exact id is required
    let out = r.run(&["recover", "--abandon", &id[..id.len() - 1]]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(Root::field(&r.records()[0].1, "status"), "running");
    let out = r.run(&["recover", "--abandon", &id]);
    let so = text(&out.stdout);
    println!("{so}");
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert!(so.contains("abandoned") && so.contains("refs/broadsword/checkpoints/"), "{so}");
    assert!(p.join("new.txt").exists(), "abandon never touches files");
    let recs = r.records();
    assert_eq!(Root::field(&recs[0].1, "status"), "abandoned");
    assert!(!git(&p, &["rev-parse", "--verify", &format!("refs/broadsword/checkpoints/{id}")]).is_empty(), "refs kept");
    let out = r.run(&["run", "p", "--", "echo hi"]);
    assert_eq!(out.status.code(), Some(0), "{}{}", text(&out.stdout), text(&out.stderr));
}

/// `broadsword run ... | head -1`: the reader closes the pipe while the job is still printing. Broadsword must not die
/// (std's println! would panic), so the job finishes and its record is `done`, not left `running` to block the project.
#[test]
fn closed_stdout_never_leaves_a_job_running() {
    use std::io::BufRead;
    let r = Root::new("pipe");
    r.project("pub fn f() {}\n", true);
    let mut child = r.cmd(&["run", "p", "--", "ping", "-n", "4", "127.0.0.1"]).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.starts_with("job: run p") {
        line.clear();
        assert!(out.read_line(&mut line).unwrap() > 0, "stdout ended before the job line");
    }
    drop(out); // the reader is gone, like `head` exiting; ping keeps printing for ~3 s
    let status = child.wait().unwrap();
    let rec = r.only_record();
    assert_eq!(Root::field(&rec, "status"), "done", "exit {status:?}, record:\n{rec}");
    let again = r.run(&["run", "p", "--", "cmd", "/C", "echo", "ok"]);
    assert_eq!(again.status.code(), Some(0), "the next job on the project is not refused: {}", String::from_utf8_lossy(&again.stderr));
}

/// A panic inside a build/test/check job kills its verified child tree and leaves the record `failed` with a note naming
/// the panic. The test hook (gate file inside the root's scratch folder) makes the job panic after its child started; a
/// build script that sleeps keeps that child alive long enough to prove it is killed.
#[test]
fn a_check_job_that_panics_ends_failed_and_its_child_is_gone() {
    let r = Root::new("panickill");
    r.project("", false);
    let p = r.project_dir();
    fs::write(p.join("build.rs"), "fn main() { std::thread::sleep(std::time::Duration::from_secs(120)); }\n").unwrap();
    fs::write(p.join("Cargo.toml"), "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\nbuild = \"build.rs\"\n\n[workspace]\n").unwrap();
    let gate = r.dir.join("scratch/panic.gate");
    // without the gate file the hook does nothing: the env var alone is ignored
    let out = Command::new(bin()).args(["run", "p", "--", "echo", "hi"]).env("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB", &gate).current_dir(&r.dir).stdin(Stdio::null()).output().unwrap();
    assert!(!text(&out.stderr).contains("TEST HOOK ACTIVE") && !text(&out.stderr).contains("PANIC"), "the hook is ignored without its gate file: {}", text(&out.stderr));
    fs::write(&gate, "").unwrap();
    let out = Command::new(bin()).args(["check", "p"]).env("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB", &gate).current_dir(&r.dir).stdin(Stdio::null()).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "{so}{se}");
    assert!(se.contains("TEST HOOK ACTIVE") && se.contains("PANIC in job"), "{se}");
    let pid = so.split("killing orphaned process tree (pid ").nth(1).and_then(|t| t.split(')').next()).unwrap_or_else(|| panic!("the child must have been killed: {so}"));
    // The record no longer names the child (a failed job clears it), so its creation time is read now: the kill has
    // already returned, so this is normally `Gone` (no start, not alive); a pid already reused here would make the wait
    // fail loudly, never pass by mistake.
    let child = Proc::now(pid.parse().expect("a pid"));
    wait_until("the child to be gone", 10, || !child.alive());
    // the ungated `run` above left its own record; this test is about the check job's
    let recs = r.records();
    let checks: Vec<&String> = recs.iter().map(|(_, t)| t).filter(|t| Root::field(t, "kind") == "check").collect();
    assert_eq!(checks.len(), 1, "{recs:?}");
    let rec = checks[0].clone();
    assert_eq!(Root::field(&rec, "status"), "failed", "{rec}");
    assert!(Root::field(&rec, "failure_reason").contains("panicked") && Root::field(&rec, "failure_reason").contains("test hook"), "{rec}");
    assert_eq!(Root::field(&rec, "child_pid"), "");
    // a failed record blocks nothing, and recover has nothing to do for it
    assert!(text(&r.run(&["recover"]).stdout).contains("nothing to do"));
}

// ---- t-20261120-152958: automatic recovery never hijacks another task's call ------------------------------------------

impl Root {
    /// A task of project `p` started with its own worktree. Returns the id.
    fn task(&self, claim: &str) -> String {
        let o = self.run(&["task", "add", "p", "--title", "t", "--claim", claim]);
        assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
        let id = text(&o.stdout).trim().to_string();
        let o = self.run(&["task", "start", &id, "--owner", "coder", "--worktree"]);
        assert_eq!(o.status.code(), Some(0), "{}{}", text(&o.stdout), text(&o.stderr));
        id
    }

    fn record(&self, id: &str) -> String {
        fs::read_to_string(self.dir.join(format!("state/jobs/{id}.rec"))).unwrap()
    }

    /// Hold the per-record recover lock of job `id`, as a recover working on that record would.
    fn hold_record_lock(&self, id: &str) -> fs::File {
        let dir = self.dir.join("state/recover-locks");
        fs::create_dir_all(&dir).unwrap();
        let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(dir.join(format!("{id}.lock"))).unwrap();
        f.lock().unwrap();
        f
    }
}

/// Task A's interrupted check job exists. A `run --task B` (and a task-less run) neither resumes nor runs it: it is reported
/// on one line and left `running`. Only a call of task A itself resumes it, and says the output above was that job's.
#[test]
fn a_call_for_another_task_never_resumes_an_interrupted_job() {
    let r = Root::new("othertask");
    r.project("", true);
    let (a, b) = (r.task("a.txt"), r.task("b.txt"));
    let dead = dead_pid().to_string();
    let old = "20260101-000001-p-check";
    r.fake_record(old, &[("project", "p"), ("kind", "check"), ("status", "running"), ("pid", &dead), ("step", "build"), ("task", &a), ("resume_count", "0")]);
    for args in [vec!["run", "p", "--task", &b, "--", "echo from-b"], vec!["run", "p", "--", "echo from-main"]] {
        let out = r.run(&args);
        let (so, se) = (text(&out.stdout), text(&out.stderr));
        println!("{args:?}\n{so}{se}");
        assert_eq!(out.status.code(), Some(0), "{so}{se}");
        assert!(!so.contains("resuming") && !so.contains("recovering job"), "another task's job must not be resumed in this call: {so}");
        assert_eq!(so.lines().filter(|l| l.contains(old)).count(), 1, "the interrupted job is reported on exactly one line: {so}");
        assert!(so.lines().any(|l| l.contains(old) && l.contains(&format!("task {a}")) && l.contains("left alone")), "{so}");
        assert!(so.contains(if args.len() == 6 { "from-b" } else { "from-main" }), "the caller's own command ran: {so}");
        let rec = r.record(old);
        assert_eq!(Root::field(&rec, "status"), "running", "{rec}");
        assert_eq!(Root::field(&rec, "resumed_as"), "", "{rec}");
        assert!(r.records().iter().all(|(_, t)| Root::field(t, "kind") == "run" || Root::field(t, "id") == old), "no job of another task was started: {:?}", r.records());
    }
    // task A's own call does resume it, and marks where the resumed job's output ends
    let out = r.run(&["run", "p", "--task", &a, "--", "echo from-a"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    println!("{so}{se}");
    assert_eq!(out.status.code(), Some(0), "{so}{se}");
    assert!(so.contains("resuming `check p` from step `build`"), "{so}");
    let rec = r.record(old);
    assert_eq!(Root::field(&rec, "status"), "resumed", "{rec}");
    let new = Root::field(&rec, "resumed_as");
    assert!(so.contains(&format!("the output above belongs to resumed job {new}")), "{so}");
    let (mark, mine) = (so.find("the output above belongs").unwrap(), so.find("from-a").unwrap());
    assert!(mark < mine, "the marker comes before the caller's own job: {so}");
    assert_eq!(Root::field(&r.record(&new), "task"), a);
}

/// `recover --abandon <id>` of a dead job is not refused by an unrelated recover holding `state/recover.lock`; it is
/// refused (record unchanged) while a recover works on THAT record, and a recover skips a record an abandon holds.
#[test]
fn abandon_does_not_wait_for_an_unrelated_recover_but_never_races_one_on_the_same_record() {
    let r = Root::new("abandonlock");
    r.project("", false);
    let dead = dead_pid().to_string();
    let (one, two) = ("20260101-000001-p-run", "20260101-000002-p-check");
    r.fake_record(one, &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &dead), ("checkpoint", "pending")]);
    r.fake_record(two, &[("project", "p"), ("kind", "check"), ("status", "running"), ("pid", &dead), ("step", "build"), ("resume_count", "0")]);
    fs::create_dir_all(r.dir.join("state")).unwrap();
    let global = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(r.dir.join("state/recover.lock")).unwrap();
    global.lock().unwrap();
    let out = r.run(&["recover", "--abandon", one]);
    assert_eq!(out.status.code(), Some(0), "an unrelated recover must not block the abandon: {}{}", text(&out.stdout), text(&out.stderr));
    assert_eq!(Root::field(&r.record(one), "status"), "abandoned");
    drop(global);

    // a recover working on `two` (its record lock held): abandon is refused and changes nothing
    let held = r.hold_record_lock(two);
    let out = r.run(&["recover", "--abandon", two]);
    assert_eq!(out.status.code(), Some(1), "{}{}", text(&out.stdout), text(&out.stderr));
    assert!(text(&out.stderr).contains("being recovered"), "{}", text(&out.stderr));
    assert_eq!(Root::field(&r.record(two), "status"), "running");
    // and a recover skips a record whose lock another process holds (as an abandon would): nothing is resumed
    let out = r.run(&["recover"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{so}{}", text(&out.stderr));
    assert!(!so.contains("resuming") && so.contains("being recovered"), "{so}");
    assert_eq!(Root::field(&r.record(two), "status"), "running");
    assert_eq!(r.records().len(), 2, "no new job was started: {:?}", r.records());
    drop(held);
    let out = r.run(&["recover", "--abandon", two]);
    assert_eq!(out.status.code(), Some(0), "{}{}", text(&out.stdout), text(&out.stderr));
    assert_eq!(Root::field(&r.record(two), "status"), "abandoned");
}

/// t-20261122-005330: a dead `run` whose record still says `checkpoint = pending` never started its command, so it no longer
/// blocks: the next job in its folder marks it `rolled_back` in place (record kept, a line added to its job log) and runs.
#[test]
fn a_dead_run_that_never_took_its_checkpoint_stops_blocking() {
    let r = Root::new("pendingdead");
    r.project("", true);
    let dead = dead_pid().to_string();
    let id = "20260101-000001-p-run";
    let log = r.dir.join("scratch/old-run.log");
    fs::write(&log, "=== run p ===\n").unwrap();
    let log_s = log.display().to_string().replace('\\', "/");
    r.fake_record(id, &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &dead), ("checkpoint", "pending"), ("log", &log_s)]);
    let out = r.run(&["run", "p", "--", "echo from-new-job"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "a dead run that never started must not block: {so}{se}");
    assert!(so.contains("from-new-job"), "{so}");
    let rec = r.record(id);
    assert_eq!(Root::field(&rec, "status"), "rolled_back", "{rec}");
    assert_eq!(Root::field(&rec, "checkpoint"), "pending", "the record is kept as it was, only settled: {rec}");
    assert!(Root::field(&rec, "rolled_back_reason").contains("never started"), "{rec}");
    let l = fs::read_to_string(&log).unwrap();
    assert!(l.starts_with("=== run p ===\n"), "the old log is kept: {l}");
    assert!(l.contains(&format!("job {id} was interrupted before its checkpoint was taken")) && l.contains("nothing was restored"), "{l}");
}

/// The same pending record is never settled while its broadsword may be alive: a live pid with its own creation time, and a
/// live pid with no recorded creation time (liveness cannot be verified), both stay `running`. A live pid whose recorded
/// creation time differs (the pid was reused by another process) is dead, and is settled.
#[test]
fn a_pending_run_whose_broadsword_may_be_alive_is_never_settled() {
    let mut r = Root::new("pendinglive");
    r.project("", true);
    let (a, b, c) = (r.sleeper(), r.sleeper(), r.sleeper());
    let start = |p: u32| Proc::now(p).start.expect("a live process has a creation time");
    let (sa, sc) = (start(a).to_string(), (start(c) - 1).to_string());
    let (live, unknown, reused) = ("20260101-000001-p-run", "20260101-000002-p-run", "20260101-000003-p-run");
    r.fake_record(live, &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &a.to_string()), ("pid_start", &sa), ("checkpoint", "pending")]);
    r.fake_record(unknown, &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &b.to_string()), ("pid_start", ""), ("checkpoint", "pending")]);
    r.fake_record(reused, &[("project", "p"), ("kind", "run"), ("status", "running"), ("pid", &c.to_string()), ("pid_start", &sc), ("checkpoint", "pending")]);
    let out = r.run(&["run", "p", "--", "echo hi"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{se}");
    for id in [live, unknown] {
        let rec = r.record(id);
        assert_eq!(Root::field(&rec, "status"), "running", "a possibly live job is never marked rolled_back: {rec}\n{so}");
        assert_eq!(Root::field(&rec, "rolled_back_reason"), "", "{rec}");
    }
    assert_eq!(Root::field(&r.record(reused), "status"), "rolled_back", "a reused pid (creation time differs) is dead: {so}");
    assert!(Proc::now(a).alive() && Proc::now(b).alive() && Proc::now(c).alive(), "no live process is killed");
}
