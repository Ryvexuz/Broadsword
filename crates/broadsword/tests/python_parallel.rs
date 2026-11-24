//! Python-only pooling through the real job interface, with serial cargo tests.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

mod support;

const HOOK: &str = "BROADSWORD_INTERNAL_TEST_FREE_BYTES_FILE";
const PASS: &str = "#[test] fn cargo_passes() {}\n";

struct Root {
    dir: PathBuf,
}

impl Root {
    fn new(tag: &str, width: &str, scripts: &str, lib: &str) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!("python-parallel-{tag}-{}", std::process::id()));
        // Unique tags per test; refuse to overwrite evidence left by an earlier run.
        assert!(
            !dir.exists(),
            "scratch root already exists: {}",
            dir.display()
        );
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("projects/p/src")).unwrap();
        fs::write(
            dir.join("broadsword.toml"),
            support::config(&[
                ("python_parallel", width),
                ("python_tests", &format!("\"p: {scripts}\"")),
            ]),
        )
        .unwrap();
        fs::write(
            dir.join("projects/p/Cargo.toml"),
            "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n",
        )
        .unwrap();
        fs::write(dir.join("projects/p/src/lib.rs"), lib).unwrap();
        Self { dir }
    }

    fn project(&self) -> PathBuf {
        self.dir.join("projects/p")
    }

    fn py(&self, name: &str, body: &str) {
        fs::write(self.project().join(name), body).unwrap();
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::new(support::bin());
        cmd.args(["test", "p"])
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .env_remove(HOOK)
            .env_remove("BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB");
        cmd
    }

    fn run(&self, mut cmd: Command) -> Output {
        let stdout = self.dir.join("scratch/stdout");
        let stderr = self.dir.join("scratch/stderr");
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
            if start.elapsed() >= Duration::from_secs(120) {
                if cfg!(windows) {
                    let _ = Command::new("taskkill")
                        .args(["/F", "/T", "/PID", &child.id().to_string()])
                        .output();
                }
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "broadsword timed out\nstdout: {}\nstderr: {}",
                    fs::read_to_string(&stdout).unwrap(),
                    fs::read_to_string(&stderr).unwrap()
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }

    fn records(&self) -> String {
        let records: Vec<_> = fs::read_dir(self.dir.join("state/jobs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "rec"))
            .collect();
        assert_eq!(records.len(), 1, "{records:?}");
        fs::read_to_string(&records[0]).unwrap()
    }

    fn log(&self) -> String {
        let logs: Vec<_> = fs::read_dir(self.dir.join("scratch/logs/jobs"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        assert_eq!(logs.len(), 1, "{logs:?}");
        fs::read_to_string(&logs[0]).unwrap()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.dir).unwrap();
        }
    }
}

fn text(out: &Output) -> String {
    format!(
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn marker(name: &str, code: i32) -> String {
    format!(
        "from pathlib import Path\nPath('{name}.ran').write_text('ran')\nprint('{name} output')\nraise SystemExit({code})\n"
    )
}

#[test]
fn python_parallel_runs_the_scripts_at_once() {
    let lib = r#"
/// ```
/// assert!(std::path::Path::new("cargo-passed").is_file());
/// std::fs::write("doc-passed", "ok").unwrap();
/// ```
pub fn documented() {}
#[test]
fn cargo_passes() {
    std::thread::sleep(std::time::Duration::from_secs(1));
    std::fs::write("cargo-passed", "ok").unwrap();
}
"#;
    let r = Root::new("overlap", "3", "a.py b.py c.py", lib);
    for name in ["a", "b", "c"] {
        r.py(
            &format!("{name}.py"),
            &format!(
                "from pathlib import Path\nimport time\nassert Path('cargo-passed').is_file()\nassert Path('doc-passed').is_file()\nwith open('{name}.times', 'a') as f:\n    f.write(str(time.monotonic_ns()) + '\\n')\ntime.sleep(3)\nwith open('{name}.times', 'a') as f:\n    f.write(str(time.monotonic_ns()) + '\\n')\nprint('{name} output')\n"
            ),
        );
    }
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let intervals: Vec<Vec<u128>> = ["a", "b", "c"]
        .iter()
        .map(|name| {
            let times: Vec<u128> = fs::read_to_string(r.project().join(format!("{name}.times")))
                .unwrap()
                .lines()
                .map(|line| line.parse().unwrap())
                .collect();
            assert_eq!(times.len(), 2, "{name}: {times:?}");
            assert!(times[0] < times[1], "{name}: {times:?}");
            times
        })
        .collect();
    let latest_start = intervals.iter().map(|t| t[0]).max().unwrap();
    let earliest_end = intervals.iter().map(|t| t[1]).min().unwrap();
    assert!(
        latest_start < earliest_end,
        "all three script intervals must overlap: {intervals:?}"
    );
    let so = String::from_utf8_lossy(&out.stdout);
    let seconds: f64 = so
        .split("; python: 3 script(s) ok (")
        .nth(1)
        .expect("Python summary")
        .split_once(" s)")
        .expect("Python summary seconds")
        .0
        .parse()
        .unwrap();
    // The brief's speed criterion is strictly under 6 s (not a numeric tolerance).
    assert!(seconds.is_finite() && seconds < 6.0, "{so}");
    let log = r.log();
    assert!(log.contains("up to 3 at once (python_parallel)"), "{log}");
    let positions: Vec<_> = ["a", "b", "c"]
        .iter()
        .map(|n| log.find(&format!("{n} output")).unwrap())
        .collect();
    assert!(positions.windows(2).all(|w| w[0] < w[1]), "{log}");
    assert!(
        r.records().contains("python_tests = ok: 3 script(s), "),
        "{}",
        r.records()
    );
}

#[test]
fn python_parallel_a_failing_script_fails_the_job_and_is_named() {
    let r = Root::new("failure", "3", "ok.py fail.py ok2.py", PASS);
    for (name, code) in [("ok", 0), ("fail", 1), ("ok2", 0)] {
        r.py(&format!("{name}.py"), &marker(name, code));
    }
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("test: FAILED (python fail.py: exit 1)"),
        "{}",
        text(&out)
    );
    let rec = r.records();
    assert!(rec.contains("status = failed"), "{rec}");
    assert!(rec.contains("python_tests = failed: fail.py (exit 1)"), "{rec}");
    assert!(
        r.log()
            .contains("error: python test failed: `python -I -B fail.py` (exit 1)"),
        "{}",
        r.log()
    );
}

#[test]
fn python_scripts_never_start_when_cargo_tests_fail_in_parallel_mode() {
    let r = Root::new(
        "cargo-failure",
        "3",
        "a.py b.py",
        "#[test] fn fails() { panic!(\"cargo failure\"); }\n",
    );
    for name in ["a", "b"] {
        r.py(&format!("{name}.py"), &marker(name, 0));
    }
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("test: FAILED (exit 101)"),
        "{}",
        text(&out)
    );
    for name in ["a", "b"] {
        assert!(!r.project().join(format!("{name}.ran")).exists());
    }
    assert!(!r.log().contains("Running python"), "{}", r.log());
}

#[test]
fn python_parallel_waits_for_doc_tests_to_pass() {
    let r = Root::new(
        "doc-failure",
        "3",
        "a.py b.py",
        "/// ```\n/// panic!(\"doc failure\");\n/// ```\npub fn documented() {}\n",
    );
    for name in ["a", "b"] {
        r.py(&format!("{name}.py"), &marker(name, 0));
    }
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("test: FAILED (exit 101)"),
        "{}",
        text(&out)
    );
    assert!(r.log().contains("doc failure"), "{}", r.log());
    for name in ["a", "b"] {
        assert!(!r.project().join(format!("{name}.ran")).exists());
    }
    assert!(!r.log().contains("Running python"), "{}", r.log());
}

#[test]
fn python_parallel_one_preserves_the_serial_first_failure_path() {
    let r = Root::new("serial", "1", "ok.py fail.py never.py", PASS);
    for (name, code) in [("ok", 0), ("fail", 1), ("never", 0)] {
        r.py(&format!("{name}.py"), &marker(name, code));
    }
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(r.project().join("ok.ran").is_file());
    assert!(r.project().join("fail.ran").is_file());
    assert!(!r.project().join("never.ran").exists());
    assert!(!r.log().contains("at once (python_parallel)"));
}

#[test]
fn python_parallel_preflights_all_scripts_before_starting_any() {
    let r = Root::new("missing", "3", "ok.py missing.py", PASS);
    r.py("ok.py", &marker("ok", 0));
    // A directory is not a script file, either.
    fs::create_dir(r.project().join("missing.py")).unwrap();
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(!r.project().join("ok.ran").exists());
    assert!(
        r.records()
            .contains("python_tests = failed: missing.py (script not found)"),
        "{}",
        r.records()
    );
    assert!(!r.log().contains("Running python"), "{}", r.log());
}

#[test]
fn python_parallel_stops_new_starts_and_names_the_first_failure_in_declared_order() {
    let r = Root::new(
        "ordered-failure",
        "2",
        "first.py second.py never.py",
        PASS,
    );
    // The second script fails first in time; the first must still name the failure.
    // Each waits for the other, so this also proves both initial slots were filled.
    for (name, other, delay, code) in [("first", "second", 1, 2), ("second", "first", 0, 1)] {
        r.py(
            &format!("{name}.py"),
            &format!(
                "from pathlib import Path\nimport time\nPath('{name}.ran').write_text('started')\ndeadline = time.monotonic() + 15\nwhile not Path('{other}.ran').exists():\n    assert time.monotonic() < deadline, 'other script never started'\n    time.sleep(0.02)\ntime.sleep({delay})\nraise SystemExit({code})\n"
            ),
        );
    }
    r.py("never.py", &marker("never", 0));
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(!r.project().join("never.ran").exists());
    assert!(
        r.records().contains("python_tests = failed: first.py (exit 2)"),
        "{}",
        r.records()
    );
    assert!(
        r.log().contains("1 test child(ren) not started after a failure"),
        "{}",
        r.log()
    );
}

#[test]
fn python_parallel_single_script_keeps_the_streamed_path() {
    let r = Root::new("single", "3", "one.py", PASS);
    r.py("one.py", &marker("one", 0));
    let out = r.run(r.cmd());
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    assert!(r.project().join("one.ran").is_file());
    assert!(!r.log().contains("at once (python_parallel)"));
}

#[test]
fn python_parallel_keeps_isolation_scrubbing_and_whole_ordered_output() {
    let r = Root::new("environment-output", "2", "a.py b.py", PASS);
    for name in ["a", "b"] {
        r.py(
            &format!("{name}.py"),
            &format!(
                "import os, sys\nassert sys.flags.isolated == 1\nassert sys.dont_write_bytecode\nassert 'PLANTED_API_KEY' not in os.environ\nassert 'PARALLEL_VALUE_HOLDER' not in os.environ\nassert os.environ.get('HARMLESS_PARALLEL') == 'visible'\nprint('{name}-begin')\nprint('{name}' * 262144)\nprint('{name}-end')\nprint('{name}-stderr', file=sys.stderr)\n"
            ),
        );
    }
    let mut cmd = r.cmd();
    cmd.env("PLANTED_API_KEY", "planted-parallel-value")
        .env("PARALLEL_VALUE_HOLDER", "prefix-planted-parallel-value-suffix")
        .env("HARMLESS_PARALLEL", "visible");
    let out = r.run(cmd);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out));
    let log = r.log();
    let expected = |n: &str| format!("{n}-begin\n{}\n{n}-end\n", n.repeat(262144));
    let normalized = log.replace("\r\n", "\n");
    let a = normalized.find(&expected("a")).expect("whole a output");
    let b = normalized.find(&expected("b")).expect("whole b output");
    assert!(a < b, "outputs must be printed in declared order");
    assert!(log.contains("a-stderr") && log.contains("b-stderr"));
    assert!(!log.contains("planted-parallel-value"));
    assert!(!r.project().join("__pycache__").exists());
}

#[test]
fn python_parallel_disk_drop_kills_scripts_and_their_pipe_holding_children() {
    let r = Root::new("disk-drop", "2", "a.py b.py", PASS);
    let hook = r.dir.join("scratch/free");
    fs::write(&hook, "99999999999999").unwrap();
    for (name, other) in [("a", "b"), ("b", "a")] {
        r.py(
            &format!("{name}.py"),
            &format!(
                "from pathlib import Path\nimport os, subprocess, sys, time\nchild = subprocess.Popen([sys.executable, '-I', '-B', '-c', 'import time; time.sleep(60)'])\nPath('{name}.pids').write_text(str(os.getpid()) + '\\n' + str(child.pid) + '\\n')\ndeadline = time.monotonic() + 15\nwhile not Path('{other}.pids').exists():\n    assert time.monotonic() < deadline, 'other script never started'\n    time.sleep(0.02)\nif '{name}' == 'a':\n    Path('../../scratch/free').write_text('0')\ntime.sleep(60)\n"
            ),
        );
    }
    let mut cmd = r.cmd();
    cmd.env(HOOK, &hook);
    let start = Instant::now();
    let out = r.run(cmd);
    assert_eq!(out.status.code(), Some(4), "{}", text(&out));
    assert!(
        start.elapsed() < Duration::from_secs(50),
        "the disk kill did not interrupt sleeping children: {}",
        text(&out)
    );
    let pids: Vec<u32> = ["a", "b"]
        .iter()
        .flat_map(|name| {
            fs::read_to_string(r.project().join(format!("{name}.pids")))
                .unwrap()
                .lines()
                .map(|s| s.parse::<u32>().unwrap())
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(pids.len(), 4, "both scripts and both children: {pids:?}");
    for pid in pids {
        let alive = if cfg!(windows) {
            let out = Command::new("tasklist")
                .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
                .output()
                .unwrap();
            assert!(out.status.success(), "tasklist failed");
            String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
        } else {
            Command::new("kill")
                .args(["-0", &pid.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        };
        assert!(!alive, "Python process {pid} outlived the disk-floor kill");
    }
    let rec = r.records();
    // The pool's Killed maps to Outcome::DiskDropped in job.rs (as on the serial path), recorded as the job status.
    assert!(rec.lines().any(|l| l.trim_end() == "status = killed_low_disk"), "{rec}");
    assert!(
        rec.lines().any(|l| l.trim_end() == "child_pids ="),
        "job ended with live children: {rec}"
    );
}
