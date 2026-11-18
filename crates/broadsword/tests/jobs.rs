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
             job_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = 10\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
max_agents = 50
"
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    /// Set `alert_command` to an `echo` that appends to `scratch/alert-marker.txt`; returns that file.
    fn alert_marker(&self) -> PathBuf {
        let path = self.dir.join("broadsword.toml");
        let text = fs::read_to_string(&path).unwrap();
        let cmd = if cfg!(windows) { "echo alerted>>scratch\\alert-marker.txt" } else { "echo alerted >> scratch/alert-marker.txt" };
        assert!(text.contains("alert_command = \"\""));
        fs::write(&path, text.replace("alert_command = \"\"", &format!("alert_command = \"{cmd}\""))).unwrap();
        self.dir.join("scratch/alert-marker.txt")
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

/// An old file in the throwaway root's scratch folder: the thing a cleanup pass deletes.
fn old_scratch_file(r: &Root) -> PathBuf {
    let p = r.dir.join("scratch/old-file.txt");
    fs::write(&p, "old").unwrap();
    fs::File::options().write(true).open(&p).unwrap().set_modified(std::time::SystemTime::now() - Duration::from_secs(48 * 3600)).unwrap();
    p
}

#[test]
fn a_refused_job_runs_cleanup_first_and_goes_on_when_that_frees_enough_space() {
    let r = Root::new("lowclean-ok", 1, 0);
    r.project("ok", TWO_TESTS, None);
    let old = old_scratch_file(&r);
    // samples: the start guard sees 0; the cleanup pass measures before (0) and after (huge); every later sample is huge
    let hook = r.dir.join("scratch/free.txt");
    fs::write(&hook, "0 0 99999999999999").unwrap();
    let out = r.cmd(&["build", "ok"]).env(HOOK, &hook).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    assert!(so.contains("running one cleanup pass before refusing"), "{so}");
    assert!(so.contains("deleted") && so.contains("old-file.txt"), "what was freed is printed: {so}");
    assert!(so.contains("freed ") && so.contains("free space:"), "{so}");
    assert!(so.contains("cleanup freed enough space") && so.contains("build: ok"), "{so}");
    assert!(!old.exists(), "the cleanup pass really deleted the old scratch file");
    assert!(!r.dir.join("broadsword.paused").exists(), "enough space was freed: no pause");
}

#[test]
fn a_refused_job_pauses_only_if_cleanup_could_not_free_enough_space() {
    let r = Root::new("lowclean-pause", 1, 0);
    r.project("ok", TWO_TESTS, None);
    let old = old_scratch_file(&r);
    let marker = r.alert_marker();
    let hook = r.dir.join("scratch/free.txt");
    fs::write(&hook, "0").unwrap(); // every sample is 0: cleanup cannot help
    let out = r.cmd(&["build", "ok"]).env(HOOK, &hook).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(3), "stdout: {so}\nstderr: {se}");
    assert!(se.contains("refusing to start"), "{se}");
    assert!(so.contains("running one cleanup pass") && so.contains("old-file.txt"), "{so}");
    assert!(!old.exists(), "cleanup ran before the pause");
    let pause = fs::read_to_string(r.dir.join("broadsword.paused")).expect("still below the floor after cleanup: paused");
    assert!(pause.contains("after cleanup") && pause.contains("job refused"), "{pause}");
    assert!(fs::read_to_string(&marker).is_ok_and(|s| s.contains("alerted")), "the configured alert command ran when the refusal paused");
    assert!(!r.dir.join("build").exists(), "cargo must not have run");
}

#[test]
fn an_existing_pause_is_not_a_low_disk_case_and_runs_no_cleanup() {
    let r = Root::new("lowclean-paused", 1, 0);
    r.project("ok", TWO_TESTS, None);
    let old = old_scratch_file(&r);
    fs::write(r.dir.join("broadsword.paused"), "reason: test\n").unwrap();
    let out = r.run(&["build", "ok"]);
    assert_eq!(out.status.code(), Some(3));
    assert!(old.exists(), "a pause that is not about space must not delete anything");
    assert!(!text(&out.stdout).contains("cleanup pass"));
}

#[test]
fn a_job_killed_for_low_disk_runs_cleanup_and_does_not_pause_when_that_frees_enough() {
    let r = Root::new("lowclean-kill-ok", 1, 0);
    r.project("slow", TWO_TESTS, Some("fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }\n"));
    let old = old_scratch_file(&r);
    // start guard: huge; the first poll: 0 (kill); cleanup before: 0, after: huge (and huge from then on)
    let hook = r.dir.join("scratch/free.txt");
    fs::write(&hook, "99999999999999 0 0 99999999999999").unwrap();
    let start = Instant::now();
    let out = r.cmd(&["build", "slow"]).env(HOOK, &hook).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(4), "stdout: {so}\nstderr: {se}");
    assert!(start.elapsed() < Duration::from_secs(50), "job was not killed early");
    assert!(se.contains("job killed; running one cleanup pass before pausing"), "{se}");
    assert!(so.contains("old-file.txt") && so.contains("freed "), "what was freed is printed: {so}");
    assert!(se.contains("Broadsword is NOT paused"), "{se}");
    assert!(!old.exists(), "cleanup really ran");
    assert!(!r.dir.join("broadsword.paused").exists(), "cleanup freed enough: no pause");
}

#[test]
fn a_job_killed_for_low_disk_pauses_after_cleanup_when_space_is_still_below_the_floor() {
    let r = Root::new("lowclean-kill-pause", 1, 0);
    r.project("slow", TWO_TESTS, Some("fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }\n"));
    let old = old_scratch_file(&r);
    let marker = r.alert_marker();
    let hook = r.dir.join("scratch/free.txt");
    fs::write(&hook, "99999999999999 0").unwrap();
    let out = r.cmd(&["build", "slow"]).env(HOOK, &hook).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(4), "stdout: {so}\nstderr: {se}");
    assert!(!old.exists(), "cleanup ran before the pause");
    let pause = fs::read_to_string(r.dir.join("broadsword.paused")).expect("still below the floor: paused");
    assert!(pause.contains("after cleanup") && pause.contains("job killed"), "{pause}");
    assert!(se.contains("Broadsword is paused"), "{se}");
    assert!(fs::read_to_string(&marker).is_ok_and(|s| s.contains("alerted")), "the configured alert command ran when the kill paused: stderr {se}");
}

#[test]
fn a_kill_when_already_paused_does_not_blame_the_disk() {
    let r = Root::new("lowclean-kill-already", 1, 0);
    r.project("slow", TWO_TESTS, Some("fn main() { std::thread::sleep(std::time::Duration::from_secs(60)); }
"));
    let hook = r.dir.join("scratch/free.txt");
    // the pause appears while the job runs (the guard at the start saw none): write it from the hook's first huge sample on
    fs::write(&hook, "99999999999999 0 0 99999999999999").unwrap();
    let mut child = r.cmd(&["build", "slow"]).env(HOOK, &hook).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let end = Instant::now() + Duration::from_secs(50);
    while Instant::now() < end && child.try_wait().unwrap().is_none() && !r.dir.join("broadsword.paused").exists() {
        // wait until the job started its cargo child (the build folder exists), then pause by hand
        if r.dir.join("build").exists() {
            fs::write(r.dir.join("broadsword.paused"), "reason: paused by hand
").unwrap();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    let se = text(&out.stderr);
    assert_eq!(out.status.code(), Some(4), "{se}");
    assert!(se.contains("was already paused before this job was killed") && se.contains("paused by hand"), "{se}");
    assert!(!se.contains("still below the floor after cleanup"), "{se}");
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
fn disk_guard_counts_the_job_reserve_exactly() {
    // floor 1 GB + reserve 2 GB: a job may start only when free space is at least 3 GB. The hook value is used
    // for every sample (the last one repeats), and it can only lower the real free space, so this needs a disk
    // with at least 4 GB free (the real guard would otherwise refuse the second run for a different reason).
    const GB: u64 = 1024 * 1024 * 1024;
    let r = Root::new("reserve", 1, 2);
    // The real disk must have clearly more than 3 GB free, or the "above" case would be refused by the real guard for a
    // reason unrelated to the reserve. On a disk that is too full the test SKIPS with a printed note (the suite must not
    // fail on a nearly full machine); a `status` output with no parsable `disk:` line FAILS, since the guard cannot then be tested.
    let st = text(&r.cmd(&["status"]).output().unwrap().stdout);
    let disk_line = st.lines().find_map(|l| l.strip_prefix("disk:")).unwrap_or_else(|| panic!("`broadsword status` printed no `disk:` line, so the real free space is unknown:
{st}"));
    let real_free_gb: f64 = disk_line
        .split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("cannot parse the free space in the `disk:` line `{disk_line}`:
{st}"));
    if real_free_gb < 4.0 {
        eprintln!("SKIPPED disk_guard_counts_the_job_reserve_exactly: it needs at least 4 GB free on the real disk and it has {real_free_gb} GB");
        return;
    }
    r.project("ok", TWO_TESTS, None);
    let hook = r.dir.join("scratch/free.txt");
    // just below floor + reserve: refused, cargo never started
    fs::write(&hook, (3 * GB - 1).to_string()).unwrap();
    let out = r.cmd(&["build", "ok"]).env(HOOK, &hook).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(3), "stdout: {so}\nstderr: {se}");
    assert!(se.contains("refusing to start") && se.contains("reserve"), "{se}");
    assert!(!r.dir.join("build").exists(), "cargo must not have run");
    // strictly above floor + reserve: allowed, the job runs and ends ok
    fs::write(&hook, (3 * GB + 1024 * 1024).to_string()).unwrap();
    let out = r.cmd(&["build", "ok"]).env(HOOK, &hook).output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    assert!(so.contains("build: ok"), "{so}");
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

/// Make `projects/<name>` a Git repository with one commit (`run` refuses a project without a checkpoint to roll back to).
fn git_init(p: &Path) {
    fs::write(p.join(".gitignore"), "target\n").unwrap();
    for args in [&["init", "-q"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"], &["add", "-A"], &["commit", "-q", "-m", "init"]] {
        assert!(Command::new("git").args(args).current_dir(p).output().unwrap().status.success(), "git {args:?}");
    }
}

#[test]
fn a_run_job_gets_the_scrubbed_environment_and_a_redacted_log() {
    let r = Root::new("scrub", 1, 0);
    r.project("p", TWO_TESTS, None);
    git_init(&r.dir.join("projects/p"));
    // a FAKE .env in this throwaway root (never the real one) and a parent environment that also carries its value
    fs::write(r.dir.join(".env"), "FAKE_TOKEN=zzsecretvalue123\n").unwrap();
    let print_env: &[&str] = if cfg!(windows) { &["run", "p", "--", "set"] } else { &["run", "p", "--", "env"] };
    let mut c = r.cmd(print_env);
    c.env("SOME_API_KEY", "zzsecretvalue123").env("CONTAINS_IT", "prefix-zzsecretvalue123-suffix").env("HARMLESS", "ok").env_remove("OPENAI_API_KEY");
    let out = c.output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    assert!(so.lines().any(|l| l.trim_end() == "HARMLESS=ok"), "a harmless variable must reach the child: {so}");
    assert!(!so.contains("SOME_API_KEY"), "a secret-named variable must not reach the child: {so}");
    assert!(!so.contains("CONTAINS_IT"), "a variable holding a .env value must not reach the child: {so}");
    assert!(!so.contains("zzsecretvalue123"), "{so}");
    assert!(so.lines().any(|l| l.starts_with("CARGO_TARGET_DIR=")), "the shared build folder is still set: {so}");
    let logs = r.job_logs();
    assert_eq!(logs.len(), 1, "{logs:?}");
    let log = fs::read_to_string(&logs[0]).unwrap();
    assert!(!log.contains("zzsecretvalue123") && !log.contains("SOME_API_KEY"), "the job log must hold no secret: {log}");
}

#[test]
fn job_output_that_prints_a_dotenv_value_is_redacted_in_the_log_and_on_the_terminal() {
    let r = Root::new("redact", 1, 0);
    r.project("p", TWO_TESTS, None);
    git_init(&r.dir.join("projects/p"));
    fs::write(r.dir.join(".env"), "FAKE_TOKEN=zzsecretvalue123\n").unwrap();
    let show: &[&str] = if cfg!(windows) { &["run", "p", "--", "type", "..\\..\\.env"] } else { &["run", "p", "--", "cat", "../../.env"] };
    let out = r.run(show);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    assert!(so.contains("[REDACTED]"), "{so}");
    assert!(!so.contains("zzsecretvalue123"), "{so}");
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    assert!(log.contains("[REDACTED]") && !log.contains("zzsecretvalue123"), "{log}");
}
