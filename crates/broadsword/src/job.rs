//! Job runner: `broadsword build | test | check | run <project>` and `broadsword projects`.
//!
//! Every cargo step runs with `CARGO_TARGET_DIR` pointing at the main root's shared build
//! folder, is guarded by the disk reserve before it starts, is watched while it runs, and is
//! tee'd to the terminal and to `scratch/logs/jobs/`.
//!
//! THE TEST STEP RUNS FROM COPIES. `state/build.lock` (buildsrc.rs) is held only while cargo compiles: the test step runs
//! `cargo test --no-run --message-format=json-render-diagnostics` under the lock, reads the test executables (and each package's
//! bin executables) from cargo's JSON, copies
//! the executables to `scratch/test-bins/<job id>/`, and RELEASES the lock. The copies then run one after another, in cargo's
//! order, each in its package root with the environment `cargo test` gives a test (`CARGO`, `CARGO_MANIFEST_DIR`,
//! `CARGO_MANIFEST_PATH`, `CARGO_PKG_*`, and for integration tests `CARGO_BIN_EXE_<bin>` pointing at the COPIED bin), so another
//! job may rebuild the shared folder meanwhile. Results, the summary line and exit codes are those of `cargo test`: the first
//! failing executable stops the step with cargo's exit code 101, and doc-tests run LAST, only if every executable passed.
//! The copies are removed when the step ends; a job killed mid-test leaves them, and the scratch cleanup removes them by age
//! (each copy's modified time is set to the copy time).
//! Every cargo child (including `cargo metadata`) is made by `buildsrc::cargo`, which applies the scrubbed environment.
//!
//! SIDE BY SIDE. With the required setting `test_parallel_binaries` above 1, the copies run up to that many at once, and the
//! project's Python tests run in the SAME pool, after the copies (children start in list order, so every copy has started before
//! any script). Each child's stdout and stderr are collected apart from the others' and printed whole, redacted, in list order
//! (cargo's order, then the declared script order), each with its time; so the log and the summary line read as a serial run's.
//! After any failure nothing new starts; the running children finish and are printed, and a note says how many never started.
//! A failing executable fails the step with exit 101 as before; a failing script fails it as `test: FAILED (python ...)` (the
//! first failing script in the declared order is named). The disk floor is polled for the whole pool and a drop kills every
//! running child's tree. The record names every running child: `child_pid`/`child_start` the first, `child_pids` all of them
//! (`pid:start|...`); recovery kills each one whose creation time matches. A child's exit never makes the pool wait on its output
//! pipes: a process it started that still holds one after `test_output_wait_seconds` gets that child's output cut there, marked
//! TRUNCATED in the log. At 1, everything runs one after another and streams, exactly as before (the first failure stops the rest).
//!
//! PYTHON TESTS (SCALING F9). When cargo's tests pass, the test step (of `test` and `check`) runs the scripts the required setting
//! `python_tests` declares for the project, in order, each as `python -I -B <script>` in the project folder (the task's worktree
//! with `--task`), after the build lock is released. They are ordinary job children: the same disk poll, log, redaction, recorded
//! child pid and scrubbed environment (`python_command`, through `scrub::command`). The first failing script fails the job (exit
//! 1, `test: FAILED (python <script>: exit N) ...`, and the log names it); a pass adds `; python: N script(s) ok` to the test
//! line. The record's `python_tests` field says what ran. A project not named in `python_tests` runs none and says so.
//!
//! Known limits:
//! - Doc-tests are not copied: rustdoc builds them from the shared folder's library as it runs them. So the step takes the build
//!   lock AGAIN for them (and re-checks the source note), after the copies; a package with slow doc-tests holds it meanwhile.
//! - A test that reads the compile-time `env!("CARGO_BIN_EXE_<bin>")` still gets the shared folder's path, which another job
//!   may rebuild while it runs; read the run-time variable first (Broadsword's own tests do).
//! - Only executables are copied, not DLLs beside them (cargo puts the `deps` folders on `PATH` on Windows; the copies get no such
//!   search path), so a test that loads a dylib from the build folder fails or sees another job's files.

use crate::buildsrc;
use crate::config::Config;
use crate::pause;
use crate::reviewer::Secrets;
use crate::scrub;
use crate::record::{self, Record};
use crate::sys;
use crate::worktree::{self, JobDirError};
use std::io::{BufRead, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const SELF_PROJECT: &str = "broadsword";
pub const EXIT_FAILED: u8 = 1;
pub const EXIT_UNKNOWN_PROJECT: u8 = 2;
pub const EXIT_REFUSED: u8 = 3;
pub const EXIT_DISK_DROPPED: u8 = 4;

/// Test-only hook, see `hooked_free_source`. The long name is deliberate: it cannot be set by accident.
pub const TEST_FREE_HOOK_ENV: &str = "BROADSWORD_INTERNAL_TEST_FREE_BYTES_FILE";

/// Test-only hook, see `panic_hook_active`: makes a job panic right after its child has started.
pub const TEST_PANIC_HOOK_ENV: &str = "BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB";

/// TEST ONLY: the env var `BROADSWORD_INTERNAL_TEST_PANIC_IN_JOB` names a gate file; when that file exists inside this
/// root's scratch folder, the job panics after its child has started (to test the panic guard). Without the file, or
/// with a file outside scratch, it does nothing. Use is announced on stderr. It can only make a job fail, never pass.
fn panic_hook_active(cfg: &Config) -> bool {
    let Ok(v) = std::env::var(TEST_PANIC_HOOK_ENV) else { return false };
    let gate = PathBuf::from(&v);
    let inside = match (gate.parent().and_then(|p| p.canonicalize().ok()), cfg.scratch_dir.canonicalize()) {
        (Some(p), Ok(s)) => p.starts_with(s),
        _ => false,
    };
    if !inside || !gate.is_file() {
        return false;
    }
    eprintln!("broadsword: TEST HOOK ACTIVE: the job panics after its child started (gate file {v})");
    true
}

mod projects;
pub use projects::{canonical_project, cmd_projects, guard_decision, hooked_free_source, list_projects, resolve_project};

mod child;
pub use child::{run_child, Outcome, StepRun};
use child::{cargo_command, kill_tree, run_child_opts, Log};

mod steps;
pub use steps::{create_log, file_stamp, step_line_build, step_line_run, step_line_test, steps_for};
use steps::exit_text;
#[cfg(test)]
pub use steps::{parse_test_results, TestCounts};

mod python;
pub use python::{python_scripts, step_line_python, PyTests, PYTHON};
use python::python_command;

mod teststep;
#[cfg(test)]
pub use teststep::parse_artifacts;
use teststep::TestStep;

mod recorded;
use recorded::run_job_recorded;

// ---- commands ---------------------------------------------------------------------------

pub struct JobSpec {
    pub kind: String,
    pub name: String,
    /// Command line, for kind `run` only.
    pub command: Option<String>,
    /// Index into `steps_for(kind)` to start from (recovery resumes a job at the step it was on).
    pub first_step: usize,
    /// Id of the interrupted job this one resumes.
    pub resumed_from: Option<String>,
    /// How many resumes lie behind this job (0 for a job started by the user).
    pub resume_count: u64,
    /// `--task <id>`: run in that task's worktree instead of the project's main checkout.
    pub task: Option<String>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn save_record(rec: &Record) -> Result<(), String> {
    rec.save().map_err(|e| format!("cannot write job record {}: {e}", rec.path.display()))
}

/// Record the final status and return `code`. A record that cannot be written is an error: the job
/// then exits `EXIT_FAILED` so nobody is told it succeeded while its record says `running`.
fn end(rec: &mut Record, status: &str, code: u8) -> u8 {
    rec.set("status", status);
    rec.set("child_pid", "");
    rec.set("child_start", "");
    rec.set("child_pids", "");
    rec.set("ended", now_secs().to_string());
    match save_record(rec) {
        Ok(()) => code,
        Err(e) => {
            eprintln!("broadsword: ERROR: {e}; the job's real status is `{status}`");
            EXIT_FAILED
        }
    }
}

pub fn cmd_job(cfg: &Config, kind: &str, name: &str, task: Option<&str>) -> ExitCode {
    run_job(cfg, JobSpec { kind: kind.into(), name: name.into(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: task.map(String::from) })
}

pub fn cmd_run(cfg: &Config, name: &str, task: Option<&str>, command: &[&str]) -> ExitCode {
    run_job(cfg, JobSpec { kind: "run".into(), name: name.into(), command: Some(command.join(" ")), first_step: 0, resumed_from: None, resume_count: 0, task: task.map(String::from) })
}

/// The folder a job runs in: the project's main checkout, or (`--task`) the task's worktree. `Err` is the exit code and the
/// message to print: 2 for an unknown project or task, 3 for a task whose worktree may not be used.
pub fn job_folder(cfg: &Config, name: &str, task: Option<&str>) -> Result<PathBuf, (u8, String)> {
    let main = resolve_project(&cfg.root, &cfg.projects_dir, name).map_err(|e| (EXIT_UNKNOWN_PROJECT, format!("{e}\nknown projects: {}", list_projects(cfg).join(", "))))?;
    match task {
        None => Ok(main),
        Some(id) => worktree::job_dir(cfg, id, name).map_err(|e| match e {
            JobDirError::Unknown(m) => (EXIT_UNKNOWN_PROJECT, m),
            JobDirError::Refused(m) => (EXIT_REFUSED, m),
        }),
    }
}

/// The source folder (relative to root, as job records store it in `source`) a job of `name` for `task` runs in. The ONE
/// place it is worked out: the job itself and the interrupted-run block (`recover::blocked_by_interrupted_run`) both use it.
pub fn job_source(cfg: &Config, name: &str, task: Option<&str>) -> Result<String, String> {
    let dir = job_folder(cfg, name, task).map_err(|(_, m)| m)?;
    buildsrc::rel_source(cfg, &dir)
}

/// The build lock, held until the job function returns, or until the test step has copied its executables (see the module
/// comment). (A `run` also leaves the note `run:<folder>` for each package: see
/// `buildsrc::prepare_for_run`; it is written before the command, so a crash cannot leave a note that vouches for it.)
struct BuildHold {
    /// A `run`'s proof that it holds the build lock (buildsrc::run_proof). Declared BEFORE `_lock`: fields drop in order, so
    /// the proof is always let go before the build lock, and is never held by a process that no longer holds the build lock.
    proof: Option<crate::lock::Lock>,
    _lock: crate::lock::Lock,
    /// When the lock was taken (the job prints how long it held it when the test step releases it early).
    taken: Instant,
}

/// What a finished job tells its caller besides the exit code (the merge queue reads it; the command line ignores it).
#[derive(Default, Debug, Clone)]
pub struct JobInfo {
    /// The job record's id, once the record exists (a job refused before that has none).
    pub id: Option<String>,
    /// The job's log file (every cargo line, stdout and stderr), once it exists.
    pub log: Option<PathBuf>,
    /// The `--- summary ---` lines (`build: ok (..)`, `test: FAILED ...`); empty when the job never reached its steps.
    pub summary: Vec<String>,
}

/// Run a job: `kind` is build, test, check or run. Writes a durable record, takes a Git checkpoint
/// before the first step (`run` only, and the project must be a Git repo), and runs the steps.
pub fn run_job(cfg: &Config, spec: JobSpec) -> ExitCode {
    ExitCode::from(run_job_recorded(cfg, spec, &mut JobInfo::default(), true))
}

/// Internal queue and performance jobs are agents even when the process has a terminal.
/// Recovery can resume a terminal job, unless the current invocation is `queue run`.
/// The code is the one documented for build, test and check.
pub fn run_job_info(cfg: &Config, spec: JobSpec, info: &mut JobInfo) -> u8 {
    run_job_recorded(cfg, spec, info, false)
}

fn queue_invocation(args: impl IntoIterator<Item = std::ffi::OsString>) -> bool {
    let mut args = args.into_iter().skip(1); // executable, then the command and subcommand
    args.next().as_deref() == Some(std::ffi::OsStr::new("queue"))
        && args.next().as_deref() == Some(std::ffi::OsStr::new("run"))
}

/// The only classification rule, from the job's own record. Missing provenance fails closed to agent.
fn build_priority(rec: &Record) -> crate::lock::Priority {
    if rec.get("started_from_terminal") == "true"
        && rec.get("started_by_queue") == "false"
        && rec.get("internal_job") == "false"
        && rec.get("task").is_empty()
    {
        crate::lock::Priority::Human
    } else {
        crate::lock::Priority::Agent
    }
}

#[cfg(test)]
mod tests {
    use super::teststep::rerun_arg;
    use super::*;
    use crate::config::GB;

    fn tmp(tag: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-job-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn resolves_names() {
        let root = tmp("resolve");
        let projects = root.join("projects");
        std::fs::create_dir_all(projects.join("sim")).unwrap();
        std::fs::write(projects.join("sim/Cargo.toml"), "").unwrap();
        std::fs::create_dir_all(projects.join("nocargo")).unwrap();
        assert_eq!(resolve_project(&root, &projects, "broadsword").unwrap(), root);
        assert_eq!(resolve_project(&root, &projects, "sim").unwrap(), projects.join("sim"));
        for bad in ["nocargo", "nope", "..", "../x", "sim/..", "a/b", "a\\b", "/abs", "C:/abs", ".", ""] {
            assert!(resolve_project(&root, &projects, bad).is_err(), "{bad}");
        }
        assert!(resolve_project(&root, &projects, "..").unwrap_err().contains("invalid"));
        // one canonical name: case and a trailing dot or space never make another project
        for same in ["sim", "SIM", "Sim"] {
            assert_eq!(canonical_project(&projects, same).unwrap(), "sim");
        }
        assert_eq!(canonical_project(&projects, "BroadSword").unwrap(), "broadsword");
        for bad in ["sim.", "sim ", "SIM."] {
            assert!(canonical_project(&projects, bad).unwrap_err().contains("invalid"), "{bad}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn guard_boundaries() {
        let (floor, reserve) = (20 * GB, 2 * GB);
        assert!(guard_decision(true, 100 * GB, reserve, floor).unwrap_err().contains("paused"));
        assert!(guard_decision(false, 22 * GB - 1, reserve, floor).is_err());
        assert!(guard_decision(false, 22 * GB, reserve, floor).is_ok()); // lands exactly on the floor
        assert!(guard_decision(false, 23 * GB, reserve, floor).is_ok());
        assert!(guard_decision(false, 0, 0, 0).is_ok());
        assert!(guard_decision(false, u64::MAX, u64::MAX, u64::MAX).is_ok());
    }

    #[test]
    fn parses_test_results() {
        let out = "running 3 tests\ntest result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s\n\
                   noise\ntest result: FAILED. 8 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.10s\n   test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let c = parse_test_results(out).unwrap();
        assert_eq!(c, TestCounts { passed: 11, failed: 2, ignored: 1, binaries: 3 });
        assert_eq!(parse_test_results("Compiling x\nerror: boom\n"), None);
        assert_eq!(parse_test_results(""), None);
    }

    fn run(code: i32, out: &str) -> StepRun {
        StepRun { outcome: Outcome::Exited(Some(code)), stdout: out.into(), secs: 1.0 }
    }

    #[test]
    fn parse_artifacts_finds_test_and_bin_executables_in_cargos_run_order() {
        let line = |kind: &str, name: &str, test: bool, exe: Option<&str>, doctest: bool| {
            let exe = exe.map(|e| format!("\"{e}\"")).unwrap_or_else(|| "null".into());
            format!("{{\"reason\":\"compiler-artifact\",\"manifest_path\":\"/p/Cargo.toml\",\"target\":{{\"kind\":[\"{kind}\"],\"name\":\"{name}\",\"src_path\":\"/p/src/{name}.rs\",\"doctest\":{doctest},\"test\":true}},\"profile\":{{\"test\":{test}}},\"executable\":{exe},\"fresh\":true}}")
        };
        let out = [
            line("test", "zeta", true, Some("/b/deps/zeta-1"), false),
            "{\"reason\":\"build-script-executed\"}".to_string(),
            line("bin", "tool", false, Some("/b/tool"), false),
            line("test", "alpha", true, Some("/b/deps/alpha-1"), false),
            line("lib", "p", false, None, true),
            line("bin", "tool", true, Some("/b/deps/tool-2"), false),
            line("lib", "p", true, Some("/b/deps/p-3"), true),
            "{\"reason\":\"build-finished\",\"success\":true}".to_string(),
        ]
        .join("\n");
        let a = parse_artifacts(&out).unwrap();
        let order: Vec<(&str, &str)> = a.tests.iter().map(|t| (t.kind.as_str(), t.name.as_str())).collect();
        assert_eq!(order, [("lib", "p"), ("bin", "tool"), ("test", "alpha"), ("test", "zeta")]);
        assert_eq!(a.bins.len(), 1);
        assert_eq!(a.bins[0].exe, PathBuf::from("/b/tool"));
        assert!(a.doctests, "a lib with doctest = true");
        assert_eq!(rerun_arg(&a.tests[0]), "--lib");
        assert_eq!(rerun_arg(&a.tests[2]), "--test alpha");
        let none = parse_artifacts(&line("lib", "p", true, Some("/b/deps/p-3"), false)).unwrap();
        assert!(!none.doctests && none.bins.is_empty());
        assert!(parse_artifacts("{\"reason\":\"compiler-artifact\"").is_err(), "broken JSON is an error, not an empty list");
    }

    #[test]
    fn summary_lines() {
        let ok = "test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";
        assert_eq!(step_line_test(&run(0, ok)), ("test: ok — 11 passed, 0 failed".to_string(), true));
        let bad = "test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n";
        let (l, ok) = step_line_test(&run(101, bad));
        assert!(!ok && l.starts_with("test: FAILED (exit 101)"), "{l}");
        let (l, ok) = step_line_test(&run(0, ""));
        assert!(ok && l.contains("no `test result:`") && !l.contains("0 passed"), "{l}");
        assert_eq!(step_line_build(&run(0, "")).0, "build: ok (1.0 s)");
        assert_eq!(step_line_build(&run(101, "")).0, "build: FAILED (exit 101)");
    }

    #[test]
    fn python_line_joins_the_test_line() {
        let ok = "test: ok — 11 passed, 0 failed".to_string();
        assert_eq!(step_line_python(ok.clone(), true, &PyTests::NoneDeclared), (ok.clone(), true));
        assert_eq!(step_line_python(ok.clone(), true, &PyTests::Ok { count: 2, secs: 1.25 }), ("test: ok — 11 passed, 0 failed; python: 2 script(s) ok (1.2 s)".to_string(), true));
        let failed = PyTests::Failed { script: "scripts/test_x.py".into(), why: "exit 1".into() };
        assert_eq!(step_line_python(ok.clone(), true, &failed), ("test: FAILED (python scripts/test_x.py: exit 1) — 11 passed, 0 failed".to_string(), false));
        // a cargo failure stays a failure, whatever the Python result
        let bad = "test: FAILED (exit 101) — 1 passed, 1 failed".to_string();
        assert_eq!(step_line_python(bad.clone(), false, &PyTests::Ok { count: 1, secs: 0.0 }), (bad, false));
        assert!(!step_line_python(ok, true, &PyTests::Killed(0)).1);
    }

    #[test]
    fn python_scripts_match_the_project_case_insensitively() {
        let mut cfg = Config::from_text(include_str!("../../../broadsword.toml"), Path::new("p")).unwrap();
        cfg.python_tests = crate::config::parse_python_tests("Crash-Sim: t/test_a.py t/test_b.py").unwrap();
        assert_eq!(python_scripts(&cfg, "crash-sim"), ["t/test_a.py", "t/test_b.py"]);
        assert!(python_scripts(&cfg, "other").is_empty());
    }

    #[test]
    fn log_names_never_collide() {
        let d = tmp("logs");
        let (a, _) = create_log(&d, "x").unwrap();
        let (b, _) = create_log(&d, "x").unwrap();
        let (c, _) = create_log(&d, "x").unwrap();
        let names = [a, b, c].map(|p| p.file_name().unwrap().to_string_lossy().into_owned());
        assert_eq!(names, ["x.log", "x-2.log", "x-3.log"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn file_stamp_format() {
        assert_eq!(file_stamp(1_700_000_000), "20231114-221320");
    }

    #[test]
    fn build_priority_uses_only_the_jobs_recorded_provenance_and_task() {
        use crate::lock::Priority::{Agent, Human};
        for (terminal, queue, internal, task, expected) in [
            ("true", "false", "false", "", Human),
            ("true", "false", "false", "t-agent", Agent),
            ("true", "true", "true", "", Agent), // terminal queue main-check, no task
            ("true", "true", "false", "", Agent), // recovery during queue run
            ("true", "false", "true", "", Agent), // internal performance job
            ("false", "false", "false", "", Agent),
            ("", "false", "false", "", Agent),
            ("TRUE", "false", "false", "", Agent),
            ("true\r", "false", "false", "", Agent),
            ("true", "", "false", "", Agent),
            ("true", "false", "", "", Agent),
        ] {
            let mut rec = Record::parse(Path::new("unused.rec"), "");
            rec.set("started_from_terminal", terminal);
            rec.set("started_by_queue", queue);
            rec.set("internal_job", internal);
            rec.set("task", task);
            assert_eq!(
                build_priority(&rec),
                expected,
                "terminal={terminal:?}, queue={queue:?}, internal={internal:?}, task={task:?}"
            );
        }
    }

    #[test]
    fn queue_provenance_comes_from_the_invocations_command_not_argument_substrings() {
        for (args, expected) in [
            (vec!["broadsword", "queue", "run", "--all"], true),
            (vec!["broadsword", "queue", "run", "q-1"], true),
            (vec!["broadsword", "queue", "list"], false),
            (vec!["broadsword", "run", "proj", "--", "queue", "run"], false),
            (vec!["broadsword", "recover", "queue"], false),
            (vec!["broadsword"], false),
            (vec![], false),
        ] {
            assert_eq!(
                queue_invocation(args.iter().map(|s| std::ffi::OsString::from(*s))),
                expected,
                "{args:?}"
            );
        }
    }
}
