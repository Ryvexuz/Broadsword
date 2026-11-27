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
use crate::git;
use crate::pause;
use crate::reviewer::Secrets;
use crate::scrub;
use crate::record::{self, Record};
use crate::sys;
use crate::worktree::{self, JobDirError};
use std::io::{BufRead, IsTerminal, Read, Write};
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
use projects::low_disk_cleanup;

mod child;
pub use child::{run_child, Outcome, StepRun};
use child::{cargo_command, kill_tree, run_child_opts, shell_command, Log};

mod steps;
pub use steps::{create_log, file_stamp, step_line_build, step_line_run, step_line_test, steps_for};
use steps::exit_text;
#[cfg(test)]
pub use steps::{parse_test_results, TestCounts};

mod python;
pub use python::{python_scripts, step_line_python, PyTests, PYTHON};
use python::{python_command, python_record};

mod teststep;
#[cfg(test)]
pub use teststep::parse_artifacts;
use teststep::{refresh_main_bins, TestStep};

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

/// The text of a panic payload.
fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into())
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

fn run_job_recorded(cfg: &Config, spec: JobSpec, info: &mut JobInfo, direct: bool) -> u8 {
    let kind = spec.kind.as_str();
    let project_dir = match job_folder(cfg, &spec.name, spec.task.as_deref()) {
        Ok(d) => d,
        Err((code, msg)) => {
            eprintln!("broadsword: {msg}");
            return code;
        }
    };
    let source = match job_source(cfg, &spec.name, spec.task.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("broadsword: {e}");
            return EXIT_FAILED;
        }
    };
    let name = canonical_project(&cfg.projects_dir, &spec.name).unwrap_or_else(|_| spec.name.clone()); // records store the canonical name
    let name = name.as_str();
    let steps = steps_for(kind);
    let mut free_src = hooked_free_source(cfg);
    let poll = Duration::from_secs(cfg.job_disk_poll_seconds);

    let secs = now_secs();
    let log_dir = cfg.scratch_dir.join("logs").join("jobs");
    let log_opened = std::fs::create_dir_all(&log_dir).and_then(|_| create_log(&log_dir, &format!("{}-{name}-{kind}", file_stamp(secs))));
    let (log_path, log): (PathBuf, Log) = match log_opened {
        Ok((p, f)) => (p, Arc::new(Mutex::new(f))),
        Err(e) => {
            eprintln!("broadsword: cannot create job log in {}: {e}", log_dir.display());
            return EXIT_FAILED;
        }
    };

    // The record comes first: from here on a kill at any moment leaves something to recover from.
    let command_text = match &spec.command {
        Some(c) => c.clone(),
        None => {
            let mut parts: Vec<String> = steps.iter().map(|s| format!("cargo {s} --offline")).collect();
            if steps.contains(&"test") {
                parts.extend(python_scripts(cfg, name).iter().map(|s| format!("{PYTHON} -I -B {s}")));
            }
            parts.join("; ")
        }
    };
    let mut rec = match record::create(&record::jobs_dir(cfg), &format!("{}-{name}-{kind}", file_stamp(secs))) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("broadsword: cannot create job record in {}: {e}", record::jobs_dir(cfg).display());
            return EXIT_FAILED;
        }
    };
    let me = std::process::id();
    rec.set("project", name);
    rec.set("kind", kind);
    rec.set("source", source.as_str());
    rec.set("task", spec.task.as_deref().unwrap_or(""));
    // Observe our own invocation, never a child's environment. Internal queue/performance calls are agents even in
    // a terminal; a resumed job can be human when recovery was itself invoked outside `queue run`.
    rec.set("started_from_terminal", std::io::stdin().is_terminal().to_string());
    rec.set("started_by_queue", queue_invocation(std::env::args_os()).to_string());
    rec.set("internal_job", (!direct && spec.resumed_from.is_none()).to_string());
    rec.set("command", command_text);
    rec.set("status", record::RUNNING);
    rec.set("pid", me.to_string());
    rec.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
    rec.set("child_pid", "");
    rec.set("child_start", "");
    rec.set("child_pids", "");
    rec.set("started", secs.to_string());
    rec.set("step", steps.get(spec.first_step).copied().unwrap_or(steps[0]));
    rec.set("checkpoint", "pending");
    rec.set("log", log_path.display().to_string());
    rec.set("resume_count", spec.resume_count.to_string());
    if let Some(old) = &spec.resumed_from {
        rec.set("resumed_from", old.as_str());
    }
    if let Err(e) = save_record(&rec) {
        eprintln!("broadsword: {e}");
        return EXIT_FAILED;
    }
    let job_id = rec.id().to_string();
    let priority = build_priority(&rec);
    info.id = Some(job_id.clone());
    info.log = Some(log_path.clone());
    let panic_hook = panic_hook_active(cfg);
    let secrets = Arc::new(scrub::secrets(cfg));
    // From here the record exists: a panic must leave a truthful record (see the handling after this block).
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
    if let Some(old) = &spec.resumed_from {
        // Hand over before running: the old record now points at this job.
        let path = record::jobs_dir(cfg).join(format!("{old}.rec"));
        let handed = record::load(&path).and_then(|mut o| {
            o.set("status", record::RESUMED);
            o.set("resumed_as", job_id.as_str());
            save_record(&o)
        });
        if let Err(e) = handed {
            eprintln!("broadsword: ERROR: cannot mark {old} as resumed: {e}");
            return end(&mut rec, record::FAILED, EXIT_FAILED);
        }
    }
    println!("job: {kind} {name} (id {job_id}, log: {})", log_path.display());
    println!("source: {source}{}", spec.task.as_deref().map(|t| format!(" (task {t})")).unwrap_or_default());

    if kind == "run" && !git::is_repo(&project_dir) {
        eprintln!("broadsword: refusing to run: project `{name}` is not a Git repository, so there is no checkpoint to roll back to (run `git init` in {} first)", project_dir.display());
        return end(&mut rec, record::REFUSED, EXIT_REFUSED);
    }

    let mut summary: Vec<String> = Vec::new();
    let mut all_ok = true;
    // the first failure to refresh main.bins (`refresh_main_bins`), reported once, after the summary block
    let mut main_bins_failed: Option<String> = None;
    let mut exit_code: Option<i32> = None;
    let mut build_hold: Option<BuildHold> = None; // held (when taken) until this function returns
    for (i, step) in steps.iter().enumerate().skip(spec.first_step) {
        let mut free = free_src();
        if let Err(why) = guard_decision(pause::is_paused(&cfg.root), free, cfg.reserve_bytes(), cfg.floor_bytes()) {
            // Low free space (not an existing pause) gets the same cleanup `cleanup` runs, once, before the job is refused;
            // if that frees enough, the job goes on. The cleanup pauses by itself only if space is still below the floor.
            let low = !pause::is_paused(&cfg.root);
            let retry = low && {
                println!("broadsword: {why}; running one cleanup pass before refusing");
                let r = low_disk_cleanup(cfg, &mut *free_src, &format!("(when starting `{kind} {name}`; job refused)"));
                free = r.free_after;
                guard_decision(pause::is_paused(&cfg.root), free, cfg.reserve_bytes(), cfg.floor_bytes()).is_ok()
            };
            if !retry {
                eprintln!("broadsword: refusing to start {step}: {why}");
                return end(&mut rec, record::REFUSED, EXIT_REFUSED);
            }
            println!("broadsword: cleanup freed enough space (free {}): the job goes on", pause::gb(free));
        }
        if i == spec.first_step {
            if kind != "run" {
                // Only `run` can change files, so only `run` is ever rolled back. build, test and check are resumed or re-run
                // (recover.rs), never rolled back: a checkpoint for them would be used by nothing.
                rec.set("checkpoint", "none");
                rec.set("checkpoint_reason", format!("{kind} jobs are resumed or re-run, never rolled back: only `run` takes a checkpoint"));
            } else if git::is_repo(&project_dir) {
                match git::checkpoint(&project_dir, &cfg.state_dir, &job_id, &git::Opts::for_project(cfg, &project_dir)) {
                    Ok(c) => {
                        println!("checkpoint: {} ({})", c.ref_name, &c.commit[..c.commit.len().min(10)]);
                        rec.set("checkpoint", c.ref_name);
                        rec.set("checkpoint_commit", c.commit);
                        rec.set("checkpoint_ignored", c.ignored_ref.unwrap_or_default());
                        rec.set("index_tree", c.index_tree.unwrap_or_default());
                        // Names are shown with control characters escaped (a name can hold a line break); the record keeps
                        // them whole through `record::encode_list`, which `recover` reads back with `decode_list`.
                        let shown = |names: Vec<&String>| names.iter().map(|n| n.escape_debug().to_string()).collect::<Vec<_>>().join(", ");
                        if !c.secret_skipped.is_empty() {
                            // names only, never content
                            println!("checkpoint: secret file(s) not saved (by design): {}", shown(c.secret_skipped.iter().collect()));
                            rec.set("secret_skipped", record::encode_list(&c.secret_skipped));
                        }
                        if !c.skipped.is_empty() {
                            eprintln!(
                                "broadsword: warning: {} ignored file(s) were NOT saved in the checkpoint (over the {} MB checkpoint_ignored_max_mb limit): {}",
                                c.skipped.len(),
                                cfg.checkpoint_ignored_max_mb,
                                shown(c.skipped.iter().map(|(p, _)| p).collect())
                            );
                            rec.set("ignored_skipped", record::encode_list(&c.skipped.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>()));
                            rec.set("ignored_skipped_len", record::encode_list(&c.skipped.iter().map(|(_, n)| n.to_string()).collect::<Vec<_>>()));
                        }
                    }
                    Err(e) => {
                        eprintln!("broadsword: refusing to run: cannot take a checkpoint: {e}");
                        return end(&mut rec, record::REFUSED, EXIT_REFUSED);
                    }
                }
            } else {
                rec.set("checkpoint", "none");
                rec.set("checkpoint_reason", "the project is not a Git repository");
            }
        }
        if build_hold.is_none() {
            // The first step this job EXECUTES (whatever step a resumed job starts at: `skip(first_step)` above), so the
            // lock and the prepare below are never skipped. Clean, record and build are one step across processes: take
            // the build lock now and keep it while cargo compiles: until this job ends, except that the test step releases it
            // once it has copied its test executables out of the shared folder (`TestStep`). `run` runs a shell command that may call cargo itself, so it takes the
            // lock too and prepares the same way (a `cargo test` inside it must test THIS folder's code, not the binary
            // another checkout left), then marks the notes of the folder's packages as `run:<folder>` (before the command, so a
            // crash cannot leave a note that vouches for what the command builds).
            let asked = Instant::now();
            let lock = match buildsrc::lock(cfg, priority) {
                Ok(l) => l,
                Err((code, msg)) => {
                    eprintln!("broadsword: {msg}");
                    // the wait a refused (or failed) job really had, so audits see lock waits of refused jobs too
                    rec.set("build_lock_wait_secs", format!("{:.1}", asked.elapsed().as_secs_f64()));
                    return end(&mut rec, if code == buildsrc::EXIT_BUSY { record::REFUSED } else { record::FAILED }, code);
                }
            };
            let waited = asked.elapsed().as_secs_f64();
            println!("build lock: taken after waiting {waited:.1} s");
            rec.set("build_lock_wait_secs", format!("{waited:.1}"));
            build_hold = Some(BuildHold { proof: None, _lock: lock, taken: Instant::now() });
            let outcome = if kind == "run" {
                buildsrc::prepare_for_run(
                    cfg,
                    &project_dir,
                    &source,
                    spec.command.as_deref().unwrap_or_default(),
                )
            } else {
                buildsrc::prepare(cfg, &project_dir, &source).map(|p| p.lines)
            };
            match outcome {
                Ok(lines) => {
                    for l in &lines {
                        println!("{l}");
                        if let Ok(mut f) = log.lock() {
                            let _ = writeln!(f, "{l}");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("broadsword: {e}");
                    return end(&mut rec, record::FAILED, EXIT_FAILED);
                }
            }
        }
        rec.set("step", *step);
        // This save puts a `run`'s checkpoint ref into the record on disk, and it comes BEFORE the command starts: a record
        // still saying `checkpoint = pending` therefore proves the command never ran (recover::settle_never_started relies on it).
        if let Err(e) = save_record(&rec) {
            eprintln!("broadsword: {e}");
            return end(&mut rec, record::FAILED, EXIT_FAILED);
        }
        if let Ok(mut f) = log.lock() {
            let _ = writeln!(f, "=== {step} {name} ({}) ===", pause::now_string());
        }
        // THE one record both refreshes of main.bins use (after this step, or after the test step's compile): cargo's bin exes
        // before the step, so only the ones it produces are kept (buildsrc::StepBins). Taken under the build lock.
        let step_bins = build_hold.is_some().then(|| buildsrc::snapshot_main_bins(cfg, &project_dir, &source));
        let mut cmd = match (&spec.command, *step) {
            (Some(line), "run") => shell_command(&project_dir, line),
            _ => cargo_command(cfg, &project_dir, step),
        };
        // The one place every job's child gets the shared build folder: cargo steps, and any `cargo ...` inside `run`
        // (a worktree's .cargo/config.toml with a relative target-dir would otherwise build inside the worktree).
        // The same place gives the child the scrubbed environment (`scrub.rs`): no secret-named variable, nothing holding a `.env` value.
        scrub::apply(&mut cmd, cfg);
        cmd.env("CARGO_TARGET_DIR", &cfg.build_dir);
        if let (true, Some(hold)) = (kind == "run", build_hold.as_mut()) {
            // this process holds the build lock for the whole command: once it has proved that (buildsrc::run_proof, kept in
            // the hold and let go just before the build lock), a Broadsword call inside it refuses at once (buildsrc::lock)
            // instead of waiting out a lock that frees only when the command ends. Set after the scrub, so it replaces any
            // marker this process inherited. With no proof the command never starts (review q-20261121-222529): every run
            // command that holds the build lock carries a proven marker, so there is no silent fallback to the deadlock.
            let marker = scrub::run_lock_marker(&job_id);
            match buildsrc::run_proof(cfg, &marker) {
                Ok(proof) => {
                    hold.proof = Some(proof);
                    cmd.env(scrub::RUN_LOCK_MARKER, marker);
                }
                Err(e) => {
                    eprintln!("broadsword: cannot prove this run holds the build lock ({e}), so a Broadsword call inside its command could not refuse at once and would wait out the lock this run holds; the command was not started");
                    return end(&mut rec, record::FAILED, EXIT_FAILED);
                }
            }
        }
        let mut spawn_err: Option<String> = None;
        // The children this job has running right now (one, or the test step's side-by-side pool), recorded so recovery can find
        // each orphan: `child_pid`/`child_start` name the first, `child_pids` all of them as `pid:start|pid:start...`.
        let mut on_live = |pids: &[u32]| {
            let starts: Vec<String> = pids.iter().map(|p| sys::start_time(*p).map(|t| t.to_string()).unwrap_or_default()).collect();
            rec.set("child_pid", pids.first().map(|p| p.to_string()).unwrap_or_default());
            rec.set("child_start", starts.first().cloned().unwrap_or_default());
            rec.set("child_pids", pids.iter().zip(&starts).map(|(p, s)| format!("{p}:{s}")).collect::<Vec<_>>().join("|"));
            if let Err(e) = save_record(&rec) {
                spawn_err = Some(e);
            }
            if panic_hook && !pids.is_empty() {
                panic!("test hook: panic in job after its child started");
            }
        };
        let mut py: Option<PyTests> = None;
        let ran = if *step == "test" && kind != "run" {
            let parallel = cfg.test_parallel_binaries as usize;
            let mut t = TestStep {
                cfg,
                project: name,
                priority,
                job_id: &job_id,
                project_dir: &project_dir,
                source: &source,
                log: &log,
                free: &mut *free_src,
                poll,
                on_live: &mut on_live,
                secrets: &secrets,
                parallel,
                main_bins_failed: &mut main_bins_failed,
                step_bins: step_bins.as_ref(),
            };
            // the project's Python tests (SCALING F9) run once cargo's tests pass (side by side: in the same pool, after the
            // copies), always after the build lock is released
            match t.run(cmd, &mut build_hold, python_scripts(cfg, name)) {
                Ok((r, Some(PyTests::Killed(f)))) => Ok(StepRun { outcome: Outcome::DiskDropped(f), ..r }),
                Ok((r, p)) => {
                    py = p;
                    Ok(r)
                }
                Err(e) => Err(e),
            }
        } else {
            run_child(cfg, cmd, &log, &mut *free_src, poll, &mut |pid| on_live(&[pid]), &secrets)
        };
        let run = match ran {
            Ok(r) => r,
            Err(e) => {
                eprintln!("broadsword: {e}");
                return end(&mut rec, record::FAILED, EXIT_FAILED);
            }
        };
        if let Some(e) = spawn_err {
            eprintln!("broadsword: ERROR: {e}");
            return end(&mut rec, record::FAILED, EXIT_FAILED);
        }
        if let Outcome::DiskDropped(f) = run.outcome {
            eprintln!("broadsword: free space {} fell below the {} GB floor during `{kind} {name}`; job killed; running one cleanup pass before pausing", pause::gb(f), cfg.disk_floor_gb);
            let was_paused = pause::is_paused(&cfg.root);
            let r = low_disk_cleanup(cfg, &mut *free_src, &format!("(during `{kind} {name}`; job killed)"));
            if was_paused {
                // the cleanup writes no pause (and sends no alert) over an existing one: do not blame the disk for it
                eprintln!("broadsword: Broadsword was already paused before this job was killed ({}); free space after cleanup is {}", pause::read_pause(&cfg.root).unwrap_or_default().trim(), pause::gb(r.free_after));
            } else if pause::is_paused(&cfg.root) {
                eprintln!("broadsword: still below the floor after cleanup (free {}): Broadsword is paused", pause::gb(r.free_after));
            } else {
                eprintln!("broadsword: cleanup freed enough space (free {} is above the {} GB floor): Broadsword is NOT paused; the killed job can be started again", pause::gb(r.free_after), cfg.disk_floor_gb);
            }
            return end(&mut rec, record::KILLED_LOW_DISK, EXIT_DISK_DROPPED);
        }
        if let Outcome::Exited(c) = run.outcome {
            exit_code = c;
        }
        let (line, ok) = match *step {
            "build" => step_line_build(&run),
            "run" => step_line_run(&run),
            _ => step_line_test(&run),
        };
        let (line, ok) = match &py {
            Some(p) => {
                rec.set("python_tests", python_record(p));
                step_line_python(line, ok, p)
            }
            None => (line, ok),
        };
        summary.push(line);
        if !ok {
            all_ok = false;
            break; // check never tests a failed build
        }
        // a successful step of the main checkout refreshes `build/main.bins`, the stable place of main's binaries (buildsrc.rs),
        // while the build lock still holds (the test step does it itself, before it releases the lock)
        if let (Some(_), Some(before)) = (build_hold.as_ref(), step_bins.as_ref()) {
            if !refresh_main_bins(cfg, &project_dir, &source, &log, before, &mut main_bins_failed) {
                break; // no further step; the failure is reported after the summary below
            }
        }
    }
    info.summary = summary.clone();
    println!("--- summary ---");
    for l in &summary {
        println!("{l}");
    }
    // THE one place a main.bins refresh failure is reported: after the summary block, then the job fails
    if let Some(e) = &main_bins_failed {
        eprintln!("broadsword: {e}");
        return end(&mut rec, record::FAILED, EXIT_FAILED);
    }
    let code = if kind == "run" {
        // the command's own exit code becomes the job's (never 0 for a failure)
        match exit_code {
            Some(0) => 0,
            Some(c) if (c & 0xFF) != 0 => (c & 0xFF) as u8,
            _ => EXIT_FAILED,
        }
    } else if all_ok {
        0
    } else {
        EXIT_FAILED
    };
    end(&mut rec, if all_ok { record::DONE } else { record::FAILED }, code)
    }));
    match result {
        Ok(code) => code,
        Err(payload) => {
            let why = panic_text(&*payload);
            eprintln!("broadsword: PANIC in job {job_id} ({kind} {name}): {why}");
            // A panic in the guard itself must not hide the original panic (already printed above).
            let guarded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                recover_after_panic(&mut rec, kind, &why);
            }));
            if guarded.is_err() {
                eprintln!("broadsword: the panic guard itself panicked while handling the job's panic ({why}); job {job_id}'s record may be left `running`: see `broadsword jobs` and `broadsword recover`");
            }
            EXIT_FAILED
        }
    }
}

/// After a panic in a job: kill its child tree (only if it is verifiably the recorded process, the same check as `recover`),
/// then a build, test or check is marked `failed` with a note; a `run` stays `running` with no live Broadsword pid, so the
/// normal interrupted-run path offers rollback (it may have half-changed files).
fn recover_after_panic(rec: &mut Record, kind: &str, why: &str) {
    crate::recover::kill_orphan(rec);
    if kind == "run" {
        rec.set("pid", ""); // this process lives on only to report; the record must read as interrupted, not as a live job
        rec.set("child_pid", "");
        rec.set("child_start", "");
        rec.set("child_pids", "");
        rec.set("panic", why);
        if let Err(e) = save_record(rec) {
            eprintln!("broadsword: ERROR: {e}");
        }
        eprintln!("broadsword: this run job may have changed files: run `broadsword recover {id}` to roll it back (plain `broadsword recover` only lists interrupted runs), or `broadsword recover --abandon {id}`", id = rec.id());
    } else {
        rec.set("failure_reason", format!("Broadsword panicked while running this job: {why}"));
        end(rec, record::FAILED, EXIT_FAILED);
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
