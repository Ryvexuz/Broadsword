//! Exercise the real serial job runner with small, controlled test executables.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

mod support;

const LIST: &str = "state/flaky.txt";
const ALLOW_A: &str = "p --test a flaky\n";
const FREE_HOOK: &str = "BROADSWORD_INTERNAL_TEST_FREE_BYTES_FILE";
const FIRST: &str = "first\n";
const RERAN: &str = "first\nflaky --exact\n";
const FAILED_SUMMARY: &str = concat!(
    "test result: FAILED. 1 passed; 1 failed; 0 ignored; ",
    "0 measured; 0 filtered out; finished in 0.01s"
);
const RECOVERED: &str = concat!(
    "test: ok after flaky rerun — 1 passed, 1 failed on the first run; ",
    "rerun passed: --test a flaky"
);

// These are Cargo-built integration executables, so the fixture exercises real artifact discovery,
// copying, and process creation. The first invocation supplies a controlled libtest failure;
// a normal rerun returns through libtest and must select exactly the named test.
const TEST_EXE: &str = r#"
use std::io::Write;

#[test]
fn flaky() {
    assert_eq!(std::env::var("CARGO_PKG_NAME").unwrap(), "p");
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut trace = std::fs::OpenOptions::new().create(true).append(true)
        .open(format!("{TARGET}.calls")).unwrap();
    if args.is_empty() {
        writeln!(trace, "first").unwrap();
        if MODE == "spawn-error" {
            // Windows permits renaming a running exe, but not deleting or overwriting it.
            let exe = std::env::current_exe().unwrap();
            std::fs::rename(&exe, exe.with_extension("moved")).unwrap();
        }
        let mut out = std::io::stdout().lock();
        let name = if MODE == "option" { "--ignored" } else { "flaky" };
        writeln!(out, "\ntest stable ... ok\ntest {name} ... FAILED\n\nfailures:\n    {name}\n\n\
            test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s").unwrap();
        out.flush().unwrap();
        if MODE == "crash" {
            std::process::abort();
        }
        std::process::exit(101);
    }

    writeln!(trace, "{}", args.join(" ")).unwrap();
    assert_eq!(args, ["flaky", "--exact"]);
    match MODE {
        "consistent" => panic!("still failing"),
        "bad-output" => {
            let mut out = std::io::stdout().lock();
            writeln!(out, "\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s").unwrap();
            writeln!(out, "{}", "x".repeat(256 * 1024)).unwrap();
            out.flush().unwrap();
            std::process::exit(0);
        }
        "disk-drop" => {
            std::fs::write("../../scratch/free.txt", "0").unwrap();
            // Wait for the real disk poll to kill us. If the poll is bypassed, the
            // parent fixture's deadline fails the test instead.
            std::thread::sleep(std::time::Duration::from_secs(FIXTURE_TIMEOUT_SECS));
            panic!("disk poll did not kill the child");
        }
        "flip" => {}
        _ => panic!("this executable must not have started again"),
    }
}

// An option mistakenly passed as a filter would select this different test instead.
#[test]
#[ignore]
fn unrelated() {}
"#;

const BATCH_EXE: &str = r#"
fn attempt(name: &str, pass_on_rerun: bool) {
    use std::io::Write;
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mut trace = std::fs::OpenOptions::new().create(true).append(true)
        .open(format!("{name}.calls")).unwrap();
    if args.is_empty() {
        writeln!(trace, "first").unwrap();
        panic!("first run failed");
    }
    writeln!(trace, "{}", args.join(" ")).unwrap();
    assert_eq!(args, [name, "--exact"]);
    assert!(pass_on_rerun, "still failing");
}

#[test]
fn alpha() { attempt("alpha", true); }

#[test]
fn beta() { attempt("beta", false); }
"#;

struct Root {
    dir: PathBuf,
    resolved: PathBuf,
    timeout_secs: u64,
}

impl Root {
    fn new(tag: &str, parallel: &str, python: &str) -> Self {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        fs::create_dir_all(&scratch).unwrap();
        let name = format!("flake-rerun-{tag}-{}", std::process::id());
        let dir = scratch.join(&name);
        fs::create_dir(&dir).expect("create fresh flake fixture root");
        let resolved = scratch.canonicalize().unwrap().join(&name);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("state")).unwrap();
        fs::create_dir_all(dir.join("projects/p/tests")).unwrap();
        let config = support::config(&[
            ("flaky_tests_file", &format!("\"{LIST}\"")),
            ("test_parallel_binaries", parallel),
            ("python_tests", python),
        ]);
        let timeout_secs = config
            .lines()
            .find_map(|line| line.strip_prefix("done_check_timeout_seconds = "))
            .expect("fixture config gives a deadline")
            .parse()
            .expect("numeric fixture deadline");
        fs::write(dir.join("broadsword.toml"), config).unwrap();
        fs::write(
            dir.join("projects/p/Cargo.toml"),
            "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n",
        )
        .unwrap();
        // A list in the project directory must never be used, even as a fallback.
        fs::create_dir(dir.join("projects/p/state")).unwrap();
        fs::write(dir.join("projects/p").join(LIST), ALLOW_A).unwrap();
        Self {
            dir,
            resolved,
            timeout_secs,
        }
    }

    fn serial(tag: &str) -> Self {
        Self::new(tag, "1", "\"\"")
    }

    fn executable(&self, target: &str, mode: &str) {
        fs::write(
            self.dir.join(format!("projects/p/tests/{target}.rs")),
            format!(
                "const TARGET: &str = {target:?};\nconst MODE: &str = {mode:?};\n\
                 const FIXTURE_TIMEOUT_SECS: u64 = {};\n{TEST_EXE}",
                self.timeout_secs
            ),
        )
        .unwrap();
    }

    fn list(&self, text: impl AsRef<[u8]>) {
        fs::write(self.dir.join(LIST), text).unwrap();
    }

    fn run(&self, drop_disk: bool) -> Output {
        let mut cmd = Command::new(support::bin());
        cmd.args(["test", "p"])
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .env_remove(FREE_HOOK)
            .env_remove("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB");
        if drop_disk {
            let hook = self.dir.join("scratch/free.txt");
            fs::write(&hook, u64::MAX.to_string()).unwrap();
            cmd.env(FREE_HOOK, hook);
        }
        let stdout = self.dir.join("scratch/stdout.txt");
        let stderr = self.dir.join("scratch/stderr.txt");
        let mut child = cmd
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap())
            .spawn()
            .expect("start broadsword");
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() >= Duration::from_secs(self.timeout_secs) {
                if cfg!(windows) {
                    let _ = Command::new("taskkill")
                        .args(["/F", "/T", "/PID", &child.id().to_string()])
                        .output();
                }
                let _ = child.kill();
                let _ = child.wait();
                panic!("broadsword timed out: read {}", self.dir.display());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }

    fn calls(&self, target: &str) -> String {
        fs::read_to_string(self.dir.join(format!("projects/p/{target}.calls"))).unwrap()
    }

    fn read_only_file(&self, dir: &str) -> String {
        let files: Vec<_> = fs::read_dir(self.dir.join(dir))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), 1, "{files:?}");
        fs::read_to_string(&files[0]).unwrap()
    }

    fn record(&self) -> String {
        self.read_only_file("state/jobs")
    }

    fn log(&self) -> String {
        self.read_only_file("scratch/logs/jobs")
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            assert_eq!(self.dir.canonicalize().unwrap(), self.resolved);
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn expect_exit(out: &Output, code: i32) {
    assert_eq!(
        out.status.code(),
        Some(code),
        "stdout:\n{}\nstderr:\n{}",
        stdout(out),
        stderr(out)
    );
}

fn expect_failure(out: &Output, note: &str) {
    expect_exit(out, 1);
    let (so, se) = (stdout(out), stderr(out));
    assert!(so.contains("test: FAILED (exit 101)"), "{so}");
    assert!(!so.lines().any(|line| line.starts_with("test: ok")), "{so}");
    assert!(se.contains(note), "expected {note:?}\nstderr:\n{se}");
}

#[test]
fn listed_flip_passes_after_one_rerun() {
    let r = Root::serial("flip");
    r.executable("a", "flip");
    r.list(format!("# header\r\n{ALLOW_A}p --test a flaky # duplicate\r\n"));
    let out = r.run(false);
    expect_exit(&out, 0);
    let so = stdout(&out);
    assert!(so.lines().any(|line| line == RECOVERED), "{so}");
    assert!(!so.contains("test: ok —"), "{so}");
    let note = "flaky rerun: passed --test a flaky (first run failed)";
    let se = stderr(&out);
    let log = r.log();
    assert_eq!(se.matches(note).count(), 1, "{se}");
    assert_eq!(log.matches(note).count(), 1, "{log}");
    let rec = r.record();
    assert!(
        rec.contains("flaky_rerun = --test a flaky failed then passed"),
        "{rec}"
    );
    assert!(rec.contains("status = done"), "{rec}");
    assert_eq!(r.calls("a"), RERAN);
}

#[test]
fn listed_consistent_failure_still_fails() {
    let r = Root::new("consistent", "1", "\"p: never.py\"");
    r.executable("a", "consistent");
    r.list(ALLOW_A);
    let out = r.run(false);
    expect_failure(&out, "flaky rerun: failed (exit 101) — --test a flaky");
    let rec = r.record();
    assert!(
        rec.contains("flaky_rerun = --test a flaky failed (exit 101)"),
        "{rec}"
    );
    assert!(!rec.contains("python_tests ="), "{rec}");
    assert_eq!(r.calls("a"), RERAN);
}

#[test]
fn crash_after_full_failure_summary_still_fails() {
    let r = Root::serial("crash");
    r.executable("a", "crash");
    r.list(ALLOW_A);
    let out = r.run(false);
    expect_failure(&out, "flaky rerun: none (crash or kill, exit ");
    let so = stdout(&out);
    assert!(so.lines().any(|line| line == FAILED_SUMMARY), "{so}");
    assert_eq!(r.calls("a"), FIRST);
}

#[test]
fn rerun_spawn_error_fails_with_the_note() {
    let r = Root::serial("spawn-error");
    r.executable("a", "spawn-error");
    r.list(ALLOW_A);
    let out = r.run(false);
    expect_failure(&out, "flaky rerun: failed (cannot start `");
    let log = r.log();
    assert!(log.contains("flaky rerun: failed (cannot start `"), "{log}");
    assert!(log.contains(" — --test a flaky"), "{log}");
    let rec = r.record();
    assert!(
        rec.contains("flaky_rerun = --test a flaky failed (cannot start `"),
        "{rec}"
    );
    assert_eq!(r.calls("a"), FIRST);
}

#[test]
fn unlisted_failure_is_not_rerun() {
    let r = Root::serial("unlisted");
    r.executable("a", "flip");
    r.list("p --test a different\np --test b flaky\nother --test a flaky\n");
    let out = r.run(false);
    expect_failure(&out, "flaky rerun: none (unlisted failure: flaky)");
    assert_eq!(r.calls("a"), FIRST);
}

#[test]
fn second_failure_in_a_job_is_not_rerun() {
    let r = Root::serial("second");
    r.executable("a", "flip");
    r.executable("b", "flip");
    r.list(format!("{ALLOW_A}p --test b flaky\n"));
    let out = r.run(false);
    expect_failure(
        &out,
        "flaky rerun: none (one rerun already used in this job)",
    );
    let se = stderr(&out);
    assert!(
        se.contains("flaky rerun: passed --test a flaky (first run failed)"),
        "{se}"
    );
    let rec = r.record();
    assert!(
        rec.contains("flaky_rerun = --test a flaky failed then passed"),
        "{rec}"
    );
    assert_eq!(r.calls("a"), RERAN);
    assert_eq!(r.calls("b"), FIRST);
}

#[test]
fn missing_unreadable_and_invalid_main_lists_are_refused() {
    for (tag, list, note) in [
        ("missing", None, " missing)"),
        ("unreadable", Some(&[0xff][..]), "cannot read list "),
        (
            "invalid",
            Some(&b"p --test a fl\raky\n"[..]),
            "allow-list line 1: invalid entry",
        ),
    ] {
        let r = Root::serial(tag);
        r.executable("a", "flip");
        if let Some(list) = list {
            r.list(list);
        }
        let out = r.run(false);
        expect_failure(&out, "flaky rerun: none (");
        let se = stderr(&out);
        assert!(se.contains("flaky.txt") && se.contains(note), "{se}");
        assert_eq!(r.calls("a"), FIRST);
    }
}

#[test]
fn parallel_failure_only_reports_that_rerun_is_disabled() {
    let r = Root::new("parallel", "2", "\"\"");
    r.executable("a", "flip");
    r.list(ALLOW_A);
    let out = r.run(false);
    expect_failure(&out, "flaky rerun: none (test_parallel_binaries is above 1)");
    assert_eq!(r.calls("a"), FIRST);
}

#[test]
fn exit_zero_with_large_output_after_rerun_summary_still_fails() {
    let r = Root::serial("bad-output");
    r.executable("a", "bad-output");
    r.list(ALLOW_A);
    let out = r.run(false);
    expect_failure(
        &out,
        "flaky rerun: failed (incomplete or invalid rerun summary)",
    );
    assert_eq!(r.calls("a"), RERAN);
}

#[test]
fn disk_drop_during_rerun_fails_and_still_pauses() {
    let r = Root::serial("disk-drop");
    r.executable("a", "disk-drop");
    r.list(ALLOW_A);
    let out = r.run(true);
    expect_failure(&out, "flaky rerun: failed (disk below floor ");
    let pause = fs::read_to_string(r.dir.join("broadsword.paused")).unwrap();
    assert!(pause.contains("job killed"), "{pause}");
    assert_eq!(r.calls("a"), RERAN);
}

#[test]
fn passed_rerun_does_not_mask_a_python_failure() {
    let r = Root::new("python-failure", "1", "\"p: missing.py\"");
    r.executable("a", "flip");
    r.list(ALLOW_A);
    let out = r.run(false);
    expect_exit(&out, 1);
    let so = stdout(&out);
    assert!(
        so.contains("test: FAILED (python missing.py: script not found) after flaky rerun"),
        "{so}"
    );
    let rec = r.record();
    assert!(
        rec.contains("python_tests = failed: missing.py (script not found)"),
        "{rec}"
    );
    assert!(
        rec.contains("flaky_rerun = --test a flaky failed then passed"),
        "{rec}"
    );
    assert_eq!(r.calls("a"), RERAN);
}

#[test]
fn one_passing_rerun_in_a_batch_does_not_excuse_another_failure() {
    let r = Root::serial("batch");
    fs::write(r.dir.join("projects/p/tests/a.rs"), BATCH_EXE).unwrap();
    r.list("p --test a alpha\np --test a beta\n");
    let out = r.run(false);
    expect_failure(&out, "flaky rerun: failed (exit 101) — --test a beta");
    let se = stderr(&out);
    assert!(
        se.contains("flaky rerun: passed --test a alpha (first run failed)"),
        "{se}"
    );
    assert_eq!(r.calls("alpha"), "first\nalpha --exact\n");
    assert_eq!(r.calls("beta"), "first\nbeta --exact\n");
}

#[test]
fn a_listed_option_is_not_used_as_a_test_filter() {
    let r = Root::serial("option");
    r.executable("a", "option");
    r.list("p --test a --ignored\n");
    let out = r.run(false);
    expect_failure(
        &out,
        "flaky rerun: none (a test name would be interpreted as a libtest option)",
    );
    assert_eq!(r.calls("a"), FIRST);
}
