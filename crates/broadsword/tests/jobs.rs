//! End-to-end tests for `broadsword build | test | check | projects`. Each test makes a throwaway
//! Broadsword root under the repo's `scratch/` folder with a tiny Cargo project inside its
//! `projects/` folder, runs the real binary there and deletes everything afterwards.
//! The build folder is that throwaway root's own `build/` (a path setting must stay inside root).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");

struct Root {
    dir: PathBuf,
}

impl Root {
    fn new(name: &str, floor_gb: u64, reserve_gb: u64) -> Root {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        let dir = scratch.join(format!("jobs-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("projects")).unwrap();
        let toml = format!(
            "root = \".\"\n\
             disk_floor_gb = {floor_gb}\n\
             cleanup_every_hours = 1\n\
             scratch_max_age_hours = 24\n\
             disk_check_every_seconds = 60\n\
             log_max_mb = 50\n\
             ram_warn_percent = 90\n\
             cpu_warn_percent = 90\n\
             alert_command = \"\"\n\
             scratch_dir = \"scratch\"\n\
             build_dir = \"build\"\n\
             projects_dir = \"projects\"\n\
             state_dir = \"state\"\n\
             install_dir = \"build/bin\"\n\
             mcp_call_timeout_seconds = 1800\n\
             mcp_output_max_kb = 1024\n\
             checkpoint_ignored_max_mb = 50\n\
             max_resumes = 3\n\
             job_disk_reserve_gb = {reserve_gb}\n\
             job_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\n"
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    /// A tiny library crate named `name` with the given lib.rs (and optional build.rs).
    fn project(&self, name: &str, lib_rs: &str, build_rs: Option<&str>) {
        let p = self.dir.join("projects").join(name);
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(p.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n")).unwrap();
        fs::write(p.join("src/lib.rs"), lib_rs).unwrap();
        if let Some(b) = build_rs {
            fs::write(p.join("build.rs"), b).unwrap();
        }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args).current_dir(&self.dir).env_remove(HOOK).stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().expect("failed to start broadsword")
    }

    fn job_logs(&self) -> Vec<PathBuf> {
        fs::read_dir(self.dir.join("scratch/logs/jobs")).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

const HOOK: &str = "BROADSWORD_INTERNAL_TEST_FREE_BYTES_FILE";
const TWO_TESTS: &str = "#[cfg(test)] mod t { #[test] fn a() {} #[test] fn b() {} }\n";

#[test]
fn passing_project_check_exits_0_with_counts() {
    let r = Root::new("pass", 1, 0);
    // two test binaries: the unit tests (2) and one integration test (1)
    r.project("ok", TWO_TESTS, None);
    fs::create_dir_all(r.dir.join("projects/ok/tests")).unwrap();
    fs::write(r.dir.join("projects/ok/tests/it.rs"), "#[test] fn c() {}\n").unwrap();
    let out = r.run(&["check", "ok"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    assert!(so.contains("build: ok ("), "{so}");
    assert!(so.contains("test: ok — 3 passed, 0 failed"), "{so}");
    // the shared build folder was used, and the job was logged
    assert!(r.dir.join("build/debug").is_dir());
    let logs = r.job_logs();
    assert_eq!(logs.len(), 1, "{logs:?}");
    let name = logs[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.ends_with("-ok-check.log"), "{name}");
    let log = fs::read_to_string(&logs[0]).unwrap();
    assert!(log.contains("test result: ok. 2 passed"), "{log}");
}

#[test]
fn failing_test_exits_1() {
    let r = Root::new("failtest", 1, 0);
    r.project("bad", "#[cfg(test)] mod t { #[test] fn good() {} #[test] fn bad() { panic!(\"no\"); } }\n", None);
    let out = r.run(&["check", "bad"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{so}\n{}", text(&out.stderr));
    assert!(so.contains("build: ok"), "{so}");
    assert!(so.contains("test: FAILED"), "{so}");
    assert!(so.contains("1 passed, 1 failed"), "{so}");
}

#[test]
fn compile_error_exits_1_and_skips_tests() {
    let r = Root::new("nocompile", 1, 0);
    r.project("broken", "fn oops( {\n", None);
    let out = r.run(&["check", "broken"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(1), "{so}\n{}", text(&out.stderr));
    assert!(so.contains("build: FAILED (exit 101)"), "{so}");
    assert!(!so.contains("test:"), "tests must not run after a failed build: {so}");
    assert!(text(&out.stderr).contains("error"), "cargo's stderr should be teed to the terminal");
}

#[test]
fn unknown_project_exits_2_and_lists_known() {
    let r = Root::new("unknown", 1, 0);
    r.project("real", TWO_TESTS, None);
    for bad in ["nope", "..", "../x"] {
        let out = r.run(&["build", bad]);
        assert_eq!(out.status.code(), Some(2), "{bad}");
        assert!(text(&out.stderr).contains("known projects: broadsword, real"), "{}", text(&out.stderr));
    }
    let list = text(&r.run(&["projects"]).stdout);
    assert!(list.lines().any(|l| l.starts_with("broadsword\t")) && list.lines().any(|l| l.starts_with("real\t")), "{list}");
}

#[test]
fn disk_guard_refuses_and_never_starts_cargo() {
    // Floor far above any real disk: free minus reserve can never clear it.
    let r = Root::new("guard", 99_999_999, 2);
    r.project("ok", TWO_TESTS, None);
    let out = r.run(&["build", "ok"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("refusing to start"), "{}", text(&out.stderr));
    assert!(!r.dir.join("build").exists(), "cargo must not have run");
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    assert!(!log.contains("==="), "no step should have started: {log}");
}

#[test]
fn disk_guard_refuses_while_paused() {
    let r = Root::new("paused", 1, 0);
    r.project("ok", TWO_TESTS, None);
    fs::write(r.dir.join("broadsword.paused"), "reason: test\n").unwrap();
    let out = r.run(&["check", "ok"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(text(&out.stderr).contains("paused"));
    assert!(!r.dir.join("build").exists());
}

#[test]
fn disk_dropping_mid_job_kills_cargo_pauses_and_exits_4() {
    let r = Root::new("drop", 1, 0);
    // build.rs keeps cargo busy for a minute: only the kill can end this job quickly.
    r.project("slow", TWO_TESTS, Some("fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }\n"));
    // Test hook: first sample (the start guard) is huge, every later sample (the poll) is 0.
    let hook = r.dir.join("scratch/free.txt");
    fs::write(&hook, "99999999999999 0").unwrap();
    let start = Instant::now();
    let out = r.cmd(&["build", "slow"]).env(HOOK, &hook).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(4), "stdout: {so}\nstderr: {se}");
    assert!(start.elapsed() < Duration::from_secs(50), "job was not killed early");
    assert!(se.contains("TEST HOOK ACTIVE"), "{se}");
    let pause = fs::read_to_string(r.dir.join("broadsword.paused")).expect("pause file written");
    assert!(pause.contains("job killed"), "{pause}");
    assert!(fs::read_to_string(r.dir.join("scratch/logs/monitor.log")).unwrap().contains("PAUSED"));
    assert!(!so.contains("build: ok"), "{so}");
}

#[test]
fn test_hook_outside_scratch_is_ignored() {
    let r = Root::new("hookignored", 1, 0);
    r.project("ok", TWO_TESTS, None);
    let outside = std::env::temp_dir().join(format!("bs-hook-outside-{}.txt", std::process::id()));
    fs::write(&outside, "0").unwrap();
    let out = r.cmd(&["build", "ok"]).env(HOOK, &outside).output().unwrap();
    let _ = fs::remove_file(&outside);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    assert!(text(&out.stderr).contains("ignoring"), "{}", text(&out.stderr));
}

#[test]
fn test_hook_cannot_defeat_the_real_disk_guard() {
    // The real disk cannot clear this floor; the hook claims a huge free space. Still refused.
    let r = Root::new("hookguard", 99_999_999, 2);
    r.project("ok", TWO_TESTS, None);
    let hook = r.dir.join("scratch/free.txt");
    fs::write(&hook, "99999999999999999").unwrap();
    let out = r.cmd(&["build", "ok"]).env(HOOK, &hook).output().unwrap();
    assert_eq!(out.status.code(), Some(3), "{}", text(&out.stderr));
    assert!(!r.dir.join("build").exists(), "cargo must not have run");
}

#[test]
fn same_second_jobs_keep_separate_logs() {
    let r = Root::new("logs", 1, 0);
    r.project("ok", TWO_TESTS, None);
    for _ in 0..3 {
        assert_eq!(r.run(&["build", "ok"]).status.code(), Some(0));
    }
    assert_eq!(r.job_logs().len(), 3, "{:?}", r.job_logs());
}
