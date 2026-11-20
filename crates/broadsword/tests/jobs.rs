//! End-to-end tests for `broadsword build | test | check | projects`. Each test makes a throwaway
//! Broadsword root under the repo's `scratch/` folder with a tiny Cargo project inside its
//! `projects/` folder, runs the real binary there and deletes everything afterwards.
//! The build folder is that throwaway root's own `build/` (a path setting must stay inside root).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

mod support;
use support::bin;

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
        let toml = support::config(&[("disk_floor_gb", &floor_gb.to_string()), ("job_disk_reserve_gb", &reserve_gb.to_string())]);
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
        let mut c = Command::new(bin());
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
    // Deterministic order: the hook says "plenty free" until the pause file is in place; only THEN is it rewritten to "0",
    // so no poll can see a low disk before the pause exists (the hook file is re-read on every poll).
    fs::write(&hook, "99999999999999").unwrap();
    let mut child = r.cmd(&["build", "slow"]).env(HOOK, &hook).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let end = Instant::now() + Duration::from_secs(50);
    while Instant::now() < end && child.try_wait().unwrap().is_none() {
        // wait until the job started its cargo child (the build folder exists), then pause by hand, then drop the free space
        if r.dir.join("build").exists() {
            fs::write(r.dir.join("broadsword.paused"), "reason: paused by hand
").unwrap();
            let tmp = r.dir.join("scratch/free.txt.tmp");
            fs::write(&tmp, "0").unwrap();
            fs::rename(&tmp, &hook).unwrap(); // atomic: a poll never reads a half-written hook
            break;
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

// ---- the test step runs from copies; the build lock covers compiling only ---------------------------------------------

/// A crate `name` with a bin (prints `hi`), a lib with `lib_rs`, and the integration tests in `tests` (file name, source).
fn bin_project(r: &Root, name: &str, version: &str, lib_rs: &str, tests: &[(&str, &str)]) -> PathBuf {
    let p = r.dir.join("projects").join(name);
    fs::create_dir_all(p.join("src")).unwrap();
    fs::create_dir_all(p.join("tests")).unwrap();
    fs::write(p.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nedition = \"2021\"\ndescription = \"a probe\"\nauthors = [\"A\", \"B\"]\n\n[workspace]\n")).unwrap();
    fs::write(p.join("src/main.rs"), "fn main() { println!(\"hi\"); }\n#[cfg(test)] mod t { #[test] fn in_bin() {} }\n").unwrap();
    fs::write(p.join("src/lib.rs"), lib_rs).unwrap();
    for (f, src) in tests {
        fs::write(p.join("tests").join(f), src).unwrap();
    }
    p
}

const ENV_PROBE: &str = r#"
use std::path::Path;
#[test]
fn runs_from_a_copy_with_cargos_environment() {
    let exe = std::env::current_exe().unwrap();
    let bin = std::env::var("CARGO_BIN_EXE_envp").expect("CARGO_BIN_EXE_envp is set at run time");
    for p in [exe.to_string_lossy().replace('\\', "/"), bin.replace('\\', "/")] {
        assert!(p.contains("/scratch/test-bins/"), "not a job copy: {p}");
    }
    assert!(Path::new(&bin).is_file(), "{bin}");
    let out = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    let dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    assert_eq!(Path::new(&dir).canonicalize().unwrap(), std::env::current_dir().unwrap().canonicalize().unwrap(), "runs in the package root");
    assert!(std::env::var("CARGO_MANIFEST_PATH").unwrap().ends_with("Cargo.toml"));
    assert_eq!(std::env::var("CARGO_PKG_NAME").unwrap(), "envp");
    assert_eq!(std::env::var("CARGO_PKG_VERSION").unwrap(), "1.2.3-beta.1");
    assert_eq!(std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap(), "1");
    assert_eq!(std::env::var("CARGO_PKG_VERSION_MINOR").unwrap(), "2");
    assert_eq!(std::env::var("CARGO_PKG_VERSION_PATCH").unwrap(), "3");
    assert_eq!(std::env::var("CARGO_PKG_VERSION_PRE").unwrap(), "beta.1");
    assert_eq!(std::env::var("CARGO_PKG_AUTHORS").unwrap(), "A:B");
    assert_eq!(std::env::var("CARGO_PKG_DESCRIPTION").unwrap(), "a probe");
    assert!(std::env::var("CARGO").is_ok());
    // the same values cargo compiled in
    assert_eq!(std::env::var("CARGO_PKG_VERSION").unwrap(), env!("CARGO_PKG_VERSION"));
}
"#;

#[test]
fn the_test_step_runs_copied_executables_with_cargos_run_time_environment() {
    let r = Root::new("copies", 1, 0);
    bin_project(&r, "envp", "1.2.3-beta.1", "/// ```\n/// assert_eq!(envp::two(), 2);\n/// ```\npub fn two() -> i32 { 2 }\n#[cfg(test)] mod t { #[test] fn in_lib() {} }\n", &[("env.rs", ENV_PROBE)]);
    let out = r.run(&["check", "envp"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    // lib unit test, bin unit test, the integration test and the doc-test
    assert!(so.contains("test: ok — 4 passed, 0 failed"), "{so}");
    assert!(so.contains("build lock: released after") && so.contains("(compiling only); 3 test executable(s) copied to"), "{so}");
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    assert!(log.contains("Running tests/env.rs") || log.contains("Running tests\\env.rs") || log.contains("env.rs ("), "{log}");
    assert!(!log.contains("\"reason\":\"compiler-artifact\""), "cargo's JSON stays out of the log: {log}");
    assert!(!so.contains("\"reason\":"), "cargo's JSON stays off the terminal");
    // the copies are gone once the step ends
    let left: Vec<_> = fs::read_dir(r.dir.join("scratch/test-bins")).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    assert!(left.is_empty(), "copies left behind: {left:?}");
}

/// Sum the `test result:` lines of `text`: (passed, failed, ignored).
fn counts(text: &str) -> (u64, u64, u64) {
    let mut c = (0, 0, 0);
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("test result:") else { continue };
        for part in rest.split([';', '.']) {
            let w: Vec<&str> = part.split_whitespace().collect();
            if let [n, label, ..] = w[..] {
                let n: u64 = n.parse().unwrap_or(0);
                match label {
                    "passed" => c.0 += n,
                    "failed" => c.1 += n,
                    "ignored" => c.2 += n,
                    _ => {}
                }
            }
        }
    }
    c
}

/// `cargo test --offline` run the old way, straight in `dir`, into its own target folder: (counts, exit code).
fn plain_cargo_test(r: &Root, dir: &Path) -> ((u64, u64, u64), Option<i32>) {
    let o = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["test", "--offline"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", r.dir.join("build-plain"))
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (counts(&text(&o.stdout)), o.status.code())
}

#[test]
fn test_results_match_plain_cargo_test() {
    let r = Root::new("same", 1, 0);
    // passing, with ignored tests and a doc-test: every executable runs, so the totals must be equal
    let p = bin_project(&r, "okp", "0.1.0", "/// ```\n/// assert!(true);\n/// ```\npub fn f() {}\n#[cfg(test)] mod t { #[test] fn a() {} #[test] #[ignore] fn skip() {} }\n", &[("a.rs", "#[test] fn x() {}\n#[test] fn y() {}\n"), ("b.rs", "#[test] fn z() {}\n")]);
    let ((pass, fail, ign), code) = plain_cargo_test(&r, &p);
    assert_eq!(code, Some(0));
    let out = r.run(&["test", "okp"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert!(so.contains(&format!("test: ok — {pass} passed, {fail} failed, {ign} ignored")), "plain cargo: {pass}/{fail}/{ign}; broadsword: {so}");
    // failing: cargo stops at the first failing executable, and so does the job (no doc-test here: cargo runs those last)
    let q = bin_project(&r, "badp", "0.1.0", "#[cfg(test)] mod t { #[test] fn a() {} }\n", &[("a.rs", "#[test] fn x() {}\n#[test] fn boom() { panic!(\"no\") }\n"), ("b.rs", "#[test] fn z() {}\n")]);
    let ((pass, fail, _), code) = plain_cargo_test(&r, &q);
    assert_eq!(code, Some(101));
    let out = r.run(&["test", "badp"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "{so}{se}");
    assert!(so.contains(&format!("test: FAILED (exit 101) — {pass} passed, {fail} failed")), "plain cargo: {pass}/{fail}; broadsword: {so}");
    assert!(se.contains("error: test failed, to rerun pass `--test a`"), "{se}");
    // cargo runs doc-tests LAST and only when every executable passed: a failing unit test hides a failing doc-test (the doc-test's
    // pass and fail are not counted); with the unit tests passing, the failing doc-test is counted after them
    let both = "/// ```
/// assert!(false, \"doc boom\");
/// ```
pub fn f() {}
#[cfg(test)] mod t { #[test] fn ok() {} #[test] fn boom() { panic!(\"unit\") } }
";
    let doc_only = "/// ```
/// assert!(false, \"doc boom\");
/// ```
pub fn f() {}
/// ```
/// assert!(true);
/// ```
pub fn g() {}
#[cfg(test)] mod t { #[test] fn ok() {} #[test] fn ok2() {} }
";
    for (name, lib) in [("unitdoc", both), ("onlydoc", doc_only)] {
        let d = bin_project(&r, name, "0.1.0", lib, &[("a.rs", "#[test] fn x() {}
")]);
        let ((pass, fail, ign), code) = plain_cargo_test(&r, &d);
        assert_eq!(code, Some(101), "{name}");
        let out = r.run(&["test", name]);
        let (so, se) = (text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(1), "{name}: {so}{se}");
        let want = format!("test: FAILED (exit 101) — {pass} passed, {fail} failed, {ign} ignored");
        let want_short = format!("test: FAILED (exit 101) — {pass} passed, {fail} failed");
        assert!(so.contains(&want) || so.contains(&want_short), "{name}: plain cargo {pass}/{fail}/{ign}; broadsword: {so}");
        if name == "unitdoc" {
            assert_eq!((pass, fail), (1, 1), "cargo stops at the failing unit test; no doc-test is counted");
            assert!(!so.contains("doc boom") && !se.contains("Doc-tests"), "no doc-test may run after a failing unit test: {so}{se}");
        } else {
            assert_eq!((pass, fail), (5, 1), "lib, bin and integration tests pass, then one doc-test passes and one fails");
        }
    }
}

/// A fake `cargo` (a .cmd file) that logs the environment it was started with, for `cargo metadata` only, then runs the real one.
#[test]
fn every_cargo_child_gets_the_scrubbed_environment_including_cargo_metadata() {
    let r = Root::new("scrubmeta", 1, 0);
    r.project("p", TWO_TESTS, None);
    let dump = r.dir.join("scratch/meta-env.txt");
    let fake = r.dir.join("scratch/fakecargo.cmd");
    let real = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    fs::write(&fake, format!("@echo off
if not \"%1\"==\"metadata\" goto run
echo ---- metadata >> \"{}\"
set >> \"{}\"
:run
\"{real}\" %*
", dump.display(), dump.display())).unwrap();
    let out = r.cmd(&["test", "p"]).env("CARGO", &fake).env("PLANTED_API_KEY", "topsecretvalue123").output().unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{se}");
    let env = fs::read_to_string(&dump).expect("the fake cargo saw a `metadata` call");
    // one call lists the packages for the build-source check, one reads the package variables for the copied tests
    assert!(env.matches("---- metadata").count() >= 2, "{env}");
    assert!(env.contains("CARGO_TARGET_DIR") || env.to_uppercase().contains("PATH="), "the child still gets its ordinary environment: {env}");
    assert!(!env.contains("topsecretvalue123") && !env.contains("PLANTED_API_KEY"), "a secret reached a cargo metadata child: {env}");
}

#[test]
fn a_build_in_the_same_root_does_not_wait_while_another_jobs_tests_run() {
    let r = Root::new("lockfree", 1, 0);
    // job A's only test waits (at most 90 s) for a gate file, after saying it started
    let gate = r.dir.join("scratch/gate.flag");
    let started = r.dir.join("scratch/started.flag");
    let wait_test = format!(
        "#[test]\nfn waits() {{\n    std::fs::write({started:?}, \"\").unwrap();\n    let t = std::time::Instant::now();\n    while !std::path::Path::new({gate:?}).exists() {{\n        assert!(t.elapsed().as_secs() < 90, \"gate never opened\");\n        std::thread::sleep(std::time::Duration::from_millis(50));\n    }}\n}}\n"
    );
    r.project("slow", &wait_test, None);
    r.project("other", TWO_TESTS, None);
    let mut a = r.cmd(&["test", "slow"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let t = Instant::now();
    while !started.exists() {
        assert!(t.elapsed() < Duration::from_secs(120), "job A never reached its tests");
        if let Ok(Some(st)) = a.try_wait() {
            panic!("job A ended early: {st}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // A is inside its tests now. B compiles into the same shared build folder (same lock) without waiting for A.
    let t0 = Instant::now();
    let b = r.run(&["build", "other"]);
    let b_secs = t0.elapsed().as_secs_f64();
    fs::write(&gate, "").unwrap();
    let a_out = a.wait_with_output().unwrap();
    let (bo, be) = (text(&b.stdout), text(&b.stderr));
    assert_eq!(b.status.code(), Some(0), "job B: {bo}{be}");
    let waited: f64 = bo.lines().find_map(|l| l.strip_prefix("build lock: taken after waiting ")).and_then(|w| w.trim_end_matches(" s").parse().ok()).expect("B prints its lock wait");
    assert!(waited < 5.0, "B waited {waited} s for the lock while A ran its tests (B took {b_secs:.1} s)");
    assert_eq!(a_out.status.code(), Some(0), "job A: {}{}", text(&a_out.stdout), text(&a_out.stderr));
    assert!(text(&a_out.stdout).contains("test: ok — 1 passed, 0 failed"), "{}", text(&a_out.stdout));
}

// ---- Python tests behind the job interface (SCALING F9) ----------------------------------------

impl Root {
    /// Replace the value of `python_tests` in this root's broadsword.toml (a raw TOML value, quotes included).
    fn python_tests(&self, value: &str) {
        let path = self.dir.join("broadsword.toml");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("python_tests = \"\""), "{text}");
        fs::write(&path, text.replace("python_tests = \"\"", &format!("python_tests = {value}"))).unwrap();
    }

    /// Write `body` to the Python file `rel` inside project `project`.
    fn py(&self, project: &str, rel: &str, body: &str) {
        let p = self.dir.join("projects").join(project).join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    /// The text of every job record (state/jobs/*.rec).
    fn records(&self) -> String {
        fs::read_dir(self.dir.join("state/jobs")).map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "rec")).map(|e| fs::read_to_string(e.path()).unwrap()).collect()).unwrap_or_default()
    }
}

/// A script that appends `name` to `<project>/ran.txt` (so a test sees which scripts ran, in order), then exits `code`.
fn marker_py(name: &str, code: i32) -> String {
    format!("import sys\nwith open('ran.txt', 'a') as f:\n    f.write('{name}\\n')\nprint('python says {name}')\nsys.exit({code})\n")
}

/// The names `marker_py` scripts of `project` wrote, in the order they ran.
fn ran(r: &Root, project: &str) -> Vec<String> {
    fs::read_to_string(r.dir.join("projects").join(project).join("ran.txt")).unwrap().lines().map(String::from).collect()
}

#[test]
fn python_tests_run_after_cargo_and_the_first_failing_one_fails_the_job() {
    // the done check of SCALING F9: a failing Python test makes the job exit non-zero, and the log names it
    let r = Root::new("pyfail", 1, 0);
    r.project("p", TWO_TESTS, None);
    r.py("p", "py/test_ok.py", &marker_py("ok", 0));
    r.py("p", "py/test_fail.py", &marker_py("fail", 1));
    r.py("p", "py/test_never.py", &marker_py("never", 0));
    r.python_tests("\"p: py/test_ok.py py/test_fail.py py/test_never.py\"");
    let out = r.run(&["check", "p"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "a failing Python test fails the job: {so}{se}");
    assert!(so.contains("build: ok"), "{so}");
    assert!(so.contains("test: FAILED (python py/test_fail.py: exit 1) — 2 passed, 0 failed"), "{so}");
    assert_eq!(ran(&r, "p"), ["ok", "fail"], "in order; the first failure stops the rest");
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    assert!(log.contains("error: python test failed: `python -I -B py/test_fail.py` (exit 1)"), "the log names the failing script: {log}");
    assert!(log.contains("python says ok") && log.contains("python says fail"), "the scripts' output is in the job log: {log}");
    let rec = r.records();
    assert!(rec.contains("status = failed") && rec.contains("python_tests = failed: py/test_fail.py (exit 1)"), "{rec}");
    assert!(rec.contains("python -I -B py/test_ok.py"), "the record's command lists the Python tests: {rec}");
}

#[test]
fn python_tests_pass_in_a_test_job_and_are_named_in_the_test_line() {
    let r = Root::new("pyok", 1, 0);
    r.project("p", TWO_TESTS, None);
    r.py("p", "py/test_a.py", &marker_py("a", 0));
    r.py("p", "py/test_b.py", &marker_py("b", 0));
    r.python_tests("\"P: py/test_a.py py/test_b.py\""); // project names match in any case
    let out = r.run(&["test", "p"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{se}");
    assert!(so.contains("test: ok — 2 passed, 0 failed; python: 2 script(s) ok ("), "{so}");
    assert_eq!(ran(&r, "p"), ["a", "b"]);
    // the build lock was released before the Python tests started (they never need the shared build folder)
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    let released = log.find("build lock: released").expect("the test step releases the build lock");
    let first_py = log.find("Running python -I -B py/test_a.py").expect("the log shows the Python test starting");
    assert!(released < first_py, "{log}");
    assert!(r.records().contains("python_tests = ok: 2 script(s)"), "{}", r.records());
    assert!(!r.dir.join("projects/p/py/__pycache__").exists(), "-B: no .pyc files are written into the checkout");
}

#[test]
fn no_python_test_runs_when_cargo_tests_fail_or_the_project_declares_none() {
    let r = Root::new("pyskip", 1, 0);
    r.project("bad", "#[cfg(test)] mod t { #[test] fn bad() { panic!(\"no\"); } }\n", None);
    r.project("plain", TWO_TESTS, None);
    r.py("bad", "py/test_a.py", &marker_py("a", 0));
    r.py("plain", "py/test_a.py", &marker_py("a", 0));
    r.python_tests("\"bad: py/test_a.py\"");
    let out = r.run(&["test", "bad"]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stdout));
    assert!(text(&out.stdout).contains("test: FAILED (exit 101)"), "{}", text(&out.stdout));
    assert!(!r.dir.join("projects/bad/ran.txt").exists(), "no Python test runs after a cargo test failure");
    // a project not named in python_tests has none: the job says so and its summary is the plain cargo one
    let out = r.run(&["test", "plain"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{se}");
    assert!(so.contains("test: ok — 2 passed, 0 failed\n") || so.contains("test: ok — 2 passed, 0 failed\r\n"), "{so}");
    assert!(se.contains("python tests: none (this project is not named in python_tests)"), "{se}");
    assert!(!r.dir.join("projects/plain/ran.txt").exists());
}

#[test]
fn a_declared_python_test_that_does_not_exist_fails_the_job() {
    let r = Root::new("pymissing", 1, 0);
    r.project("p", TWO_TESTS, None);
    r.python_tests("\"p: py/test_gone.py\"");
    let out = r.run(&["test", "p"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "a missing script is a failure, never a skip: {so}{se}");
    assert!(so.contains("test: FAILED (python py/test_gone.py: script not found)"), "{so}");
}

#[test]
fn a_python_test_gets_the_scrubbed_environment_and_runs_isolated() {
    let r = Root::new("pyscrub", 1, 0);
    r.project("p", TWO_TESTS, None);
    // a FAKE .env in this throwaway root (never the real one), and a parent environment carrying secrets
    fs::write(r.dir.join(".env"), "FAKE_TOKEN=zzpysecret987\n").unwrap();
    // the script reports FACTS about its environment, never the environment itself (a failure must not print a real variable)
    let dump = r.dir.join("scratch/py-env.txt");
    let script = format!(
        "import os, sys\nenv = os.environ\nfacts = [\n    'harmless=' + env.get('HARMLESS_F9', 'MISSING'),\n    'secret_name_seen=' + str('PLANTED_API_KEY' in env),\n    'secret_value_seen=' + str(any('zzplantedkey456' in v for v in env.values())),\n    'dotenv_holder_seen=' + str('CONTAINS_IT' in env),\n    'dotenv_value_seen=' + str(any('zzpysecret987' in v for v in env.values())),\n    'isolated=' + str(sys.flags.isolated),\n    'no_bytecode=' + str(sys.dont_write_bytecode),\n]\nwith open({:?}, 'w') as f:\n    f.write(' '.join(facts))\n",
        dump.display().to_string()
    );
    r.py("p", "py/test_env.py", &script);
    r.python_tests("\"p: py/test_env.py\"");
    let out = r
        .cmd(&["test", "p"])
        .env("PLANTED_API_KEY", "zzplantedkey456")
        .env("CONTAINS_IT", "prefix-zzpysecret987-suffix")
        .env("HARMLESS_F9", "visible")
        .output()
        .unwrap();
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{se}");
    let facts = fs::read_to_string(&dump).expect("the Python test ran and wrote its facts");
    assert_eq!(
        facts,
        "harmless=visible secret_name_seen=False secret_value_seen=False dotenv_holder_seen=False dotenv_value_seen=False isolated=1 no_bytecode=True",
        "harmless=visible proves the facts come from the child's real environment; every *_seen must be False; -I and -B are passed"
    );
}

// ---- test executables and Python tests side by side (test_parallel_binaries) ------------------------------------------

impl Root {
    /// Set `key` (a key of `support::config`) to the raw TOML `value` in this root's broadsword.toml.
    fn set(&self, key: &str, value: &str) {
        let path = self.dir.join("broadsword.toml");
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.lines().any(|l| l.starts_with(&format!("{key} = "))), "{key} not in {text}");
        let new: String = text.lines().map(|l| if l.starts_with(&format!("{key} = ")) { format!("{key} = {value}\n") } else { format!("{l}\n") }).collect();
        fs::write(&path, new).unwrap();
    }
}

/// An integration test `name` that marks itself started in GATE, waits (at most 60 s) until `want` tests have, and then until
/// the job record (in JOBS) names its own pid in `child_pids`. Run one after another it would time out: it proves the
/// executables run at once and that the record names every running one.
fn gated_test(name: &str, gate: &Path, jobs: &Path, want: usize) -> String {
    r#"
use std::time::{Duration, Instant};
#[test]
fn NAME() {
    let gate = std::path::Path::new(r"GATE");
    std::fs::create_dir_all(gate).unwrap();
    std::fs::write(gate.join("NAME"), "").unwrap();
    let t = Instant::now();
    while std::fs::read_dir(gate).unwrap().count() < WANT {
        assert!(t.elapsed().as_secs() < 60, "the other test executables never started beside this one");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::io::Write::write_all(&mut std::io::stdout(), b"NAME saw WANT running\n").unwrap(); // not println!: libtest captures that
    let me = format!("{}:", std::process::id());
    loop {
        let named = std::fs::read_dir(r"JOBS").unwrap().flatten().any(|e| {
            std::fs::read_to_string(e.path()).is_ok_and(|s| s.lines().any(|l| l.starts_with("child_pids = ") && l["child_pids = ".len()..].split('|').any(|p| p.starts_with(&me))))
        });
        if named {
            break;
        }
        assert!(t.elapsed().as_secs() < 60, "the job record never named this running test executable in child_pids");
        std::thread::sleep(Duration::from_millis(20));
    }
}
"#
    .replace("NAME", name)
    .replace("GATE", &gate.display().to_string())
    .replace("JOBS", &jobs.display().to_string())
    .replace("WANT", &want.to_string())
}

/// The position in `log` of the `Running` line of the target whose source ends with `tail`.
fn running_at(log: &str, tail: &str) -> usize {
    log.lines()
        .scan(0usize, |at, l| {
            let here = *at;
            *at += l.len() + 1;
            Some((here, l))
        })
        .find(|(_, l)| l.trim_start().starts_with("Running ") && l.contains(tail))
        .unwrap_or_else(|| panic!("no Running line for {tail} in {log}"))
        .0
}

#[test]
fn test_executables_and_python_tests_run_side_by_side_and_print_in_a_stable_order() {
    let r = Root::new("side", 1, 0);
    r.set("test_parallel_binaries", "4");
    let (gate, jobs) = (r.dir.join("scratch/gate"), r.dir.join("state/jobs"));
    let sep = std::path::MAIN_SEPARATOR;
    let tests: Vec<(String, String)> = ["a", "b", "c"].iter().map(|n| (format!("{n}.rs"), gated_test(n, &gate, &jobs, 3))).collect();
    let tests: Vec<(&str, &str)> = tests.iter().map(|(f, s)| (f.as_str(), s.as_str())).collect();
    bin_project(&r, "side", "0.1.0", "pub fn f() {}\n#[cfg(test)] mod t { #[test] fn in_lib() {} }\n", &tests);
    r.py("side", "py/test_one.py", &marker_py("one", 0));
    r.py("side", "py/test_two.py", &marker_py("two", 0));
    r.python_tests("\"side: py/test_one.py py/test_two.py\"");
    let out = r.run(&["test", "side"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{se}");
    // one summed line: in_lib + in_bin + a + b + c, and the scripts
    assert!(so.contains("test: ok — 5 passed, 0 failed; python: 2 script(s) ok ("), "{so}");
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    assert!(log.contains("up to 4 at once (test_parallel_binaries)"), "{log}");
    // printed whole, in cargo's order and then the declared script order, whatever order they ended in
    let at: Vec<usize> = [format!("src{sep}lib.rs"), format!("src{sep}main.rs"), format!("tests{sep}a.rs"), format!("tests{sep}b.rs"), format!("tests{sep}c.rs"), "py/test_one.py".into(), "py/test_two.py".into()]
        .iter()
        .map(|t| running_at(&log, t))
        .collect();
    assert!(at.windows(2).all(|w| w[0] < w[1]), "Running lines out of order {at:?}: {log}");
    for n in ["a", "b", "c"] {
        let seen = log.find(&format!("{n} saw 3 running")).unwrap_or_else(|| panic!("{n}'s output is in the log: {log}"));
        let next_running = log[running_at(&log, &format!("tests{sep}{n}.rs")) + 1..].find("     Running ").map(|i| i + running_at(&log, &format!("tests{sep}{n}.rs")) + 1).unwrap_or(log.len());
        assert!(seen > running_at(&log, &format!("tests{sep}{n}.rs")) && seen < next_running, "{n}'s output sits under its own Running line: {log}");
    }
    let mut ran = ran(&r, "side");
    ran.sort();
    assert_eq!(ran, ["one", "two"]);
    let rec = r.records();
    assert!(rec.lines().any(|l| l.trim_end() == "child_pids =") && !rec.lines().any(|l| l.starts_with("child_pid") && l.contains(':') && l.chars().any(|c| c.is_ascii_digit())), "the record names no child once the job ended: {rec}");
}

#[test]
fn a_failing_test_executable_among_side_by_side_ones_fails_the_job() {
    let r = Root::new("sidefail", 1, 0);
    r.set("test_parallel_binaries", "4");
    let gate = r.dir.join("scratch/gate");
    // b dies (exit 3, so no `test result:` line: only its exit code can fail the job) while a is still running; a must
    // finish and be printed
    let a = r#"
#[test]
fn a() {
    let gate = std::path::Path::new(r"GATE");
    std::fs::create_dir_all(gate).unwrap();
    std::fs::write(gate.join("a"), "").unwrap();
    let t = std::time::Instant::now();
    while !gate.join("b").exists() {
        assert!(t.elapsed().as_secs() < 60, "b never started beside a");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    std::io::Write::write_all(&mut std::io::stdout(), b"a finished after b died\n").unwrap();
}
"#
    .replace("GATE", &gate.display().to_string());
    let b = r#"
#[test]
fn b() {
    let gate = std::path::Path::new(r"GATE");
    std::fs::create_dir_all(gate).unwrap();
    std::fs::write(gate.join("b"), "").unwrap();
    let t = std::time::Instant::now();
    while !gate.join("a").exists() {
        assert!(t.elapsed().as_secs() < 60, "a never started beside b");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    std::process::exit(3);
}
"#
    .replace("GATE", &gate.display().to_string());
    bin_project(&r, "sidefail", "0.1.0", "pub fn f() {}\n", &[("a.rs", &a), ("b.rs", &b), ("c.rs", "#[test] fn c() {}\n")]);
    let out = r.run(&["test", "sidefail"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "a failing executable fails the job: {so}{se}");
    assert!(so.contains("test: FAILED (exit 101)"), "{so}");
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    assert!(log.contains("error: test failed, to rerun pass `--test b` (exit 3"), "{log}");
    assert!(log.contains("a finished after b died"), "the executable still running at the failure finished and was printed: {log}");
    assert!(r.records().contains("status = failed"), "{}", r.records());
}

#[test]
fn a_failing_python_test_beside_others_fails_the_job_and_is_named() {
    let r = Root::new("sidepy", 1, 0);
    r.set("test_parallel_binaries", "4");
    r.project("p", TWO_TESTS, None);
    r.py("p", "py/test_ok.py", &marker_py("ok", 0));
    r.py("p", "py/test_fail.py", &marker_py("fail", 1));
    r.py("p", "py/test_ok2.py", &marker_py("ok2", 0));
    r.python_tests("\"p: py/test_ok.py py/test_fail.py py/test_ok2.py\"");
    let out = r.run(&["check", "p"]);
    let (so, se) = (text(&out.stdout), text(&out.stderr));
    assert_eq!(out.status.code(), Some(1), "{so}{se}");
    assert!(so.contains("test: FAILED (python py/test_fail.py: exit 1) — 2 passed, 0 failed"), "{so}");
    let log = fs::read_to_string(&r.job_logs()[0]).unwrap();
    assert!(log.contains("error: python test failed: `python -I -B py/test_fail.py` (exit 1)"), "{log}");
    assert!(r.records().contains("python_tests = failed: py/test_fail.py (exit 1)"), "{}", r.records());
}
