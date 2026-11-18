//! Kill-and-recover tests. Each test makes a throwaway Broadsword root under the repo's `scratch/`
//! folder with a throwaway Git repo as its project, runs the real binary, hard-kills only the
//! broadsword process (a crash: its child is left orphaned), then runs `broadsword recover`.
//! Everything is deleted afterwards. Never touches the real repo or the crash sim.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");

// The product's own process-identity code, so the test cleanup applies the very same exact-match rule.
#[allow(dead_code)]
#[path = "../src/sys.rs"]
mod sys;

/// Kill the process tree of a recorded child pid, but only if the live process is verifiably the recorded one:
/// its creation time must equal the recorded `child_start` exactly (`sys::exact_match`). A record without a start
/// time, or a pid that now belongs to another process, is never killed.
fn kill_recorded_if_same(rec: &str) {
    let (pid, start) = (Root::field(rec, "child_pid"), Root::field(rec, "child_start"));
    let Ok(pid) = pid.parse::<u32>() else { return };
    if sys::exact_match(start.parse::<u64>().ok(), sys::process_state(pid)) {
        let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output();
    }
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

fn pid_alive(pid: &str) -> bool {
    let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().expect("tasklist");
    text(&out.stdout).split_whitespace().any(|w| w == pid)
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
        let toml = "root = \".\"\ndisk_floor_gb = 1\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\n\
                    disk_check_every_seconds = 60\nlog_max_mb = 50\nram_warn_percent = 100\ncpu_warn_percent = 90\n\
                    alert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
                    state_dir = \"state\"\ninstall_dir = \"build/bin\"\nmcp_call_timeout_seconds = 1800\nmcp_output_max_kb = 1024\ncheckpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 0\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = 10\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
light_check_paths = \"docs/,*.md\"
light_check_timeout_seconds = 300
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
";
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
        let mut c = Command::new(BIN);
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
    let (id, child) = (Root::field(&rec, "id"), Root::field(&rec, "child_pid"));
    assert_eq!(Root::field(&rec, "pid"), bs_pid.to_string());

    kill_hard(bs_pid);
    assert!(pid_alive(&child), "the orphaned command should still be running after the crash");

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
    wait_until("orphan to die", 10, || !pid_alive(&child)); // killed by the plain recover above

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
    let (old_id, child) = (Root::field(&rec, "id"), Root::field(&rec, "child_pid"));
    kill_hard(bs_pid);
    assert!(pid_alive(&child), "orphaned cargo test should still be running");
    fs::remove_file(p.join("sleep.flag")).unwrap();

    let out = r.run(&["recover"]);
    let so = text(&out.stdout);
    println!("{so}{}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert!(so.contains("killing orphaned process tree"), "{so}");
    assert!(so.contains("resuming `check p` from step `test`"), "{so}");
    assert!(so.contains("test: ok"), "{so}");
    assert!(!pid_alive(&child));

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
    assert!(pid_alive(&Root::field(&before, "child_pid")));
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
    /// Write a record by hand, as if left behind by a crashed broadsword.
    fn fake_record(&self, id: &str, fields: &[(&str, &str)]) {
        let dir = self.dir.join("state/jobs");
        fs::create_dir_all(&dir).unwrap();
        let body: String = fields.iter().map(|(k, v)| format!("{k} = {v}\n")).collect();
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
    let child = Root::field(&r.only_record(), "child_pid");
    kill_hard(bs_pid);
    let out = r.run(&["run", "p", "--", "echo hi"]);
    let se = text(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "{}{se}", text(&out.stdout));
    assert!(se.contains("broadsword recover"), "{se}");
    assert!(p.join("new.txt").exists(), "auto-recover must not touch the working tree");
    assert_eq!(Root::field(&r.only_record(), "status"), "running");
    wait_until("orphan to be killed by auto-recover", 10, || !pid_alive(&child));
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
    assert!(pid_alive(&a.to_string()) && pid_alive(&b.to_string()), "neither process may be killed");
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
    let out = Command::new(BIN).args(["run", "p", "--", "echo", "hi"]).env("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB", &gate).current_dir(&r.dir).stdin(Stdio::null()).output().unwrap();
    assert!(!text(&out.stderr).contains("TEST HOOK ACTIVE") && !text(&out.stderr).contains("PANIC"), "the hook is ignored without its gate file: {}", text(&out.stderr));
    fs::write(&gate, "").unwrap();
    let out = Command::new(BIN).args(["check", "p"]).env("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB", &gate).current_dir(&r.dir).stdin(Stdio::null()).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "{so}{se}");
    assert!(se.contains("TEST HOOK ACTIVE") && se.contains("PANIC in job"), "{se}");
    let pid = so.split("killing orphaned process tree (pid ").nth(1).and_then(|t| t.split(')').next()).unwrap_or_else(|| panic!("the child must have been killed: {so}")).to_string();
    wait_until("the child to be gone", 10, || !pid_alive(&pid));
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
