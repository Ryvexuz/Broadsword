//! Job runner: `broadsword build | test | check | run <project>` and `broadsword projects`.
//!
//! Every cargo step runs with `CARGO_TARGET_DIR` pointing at the main root's shared build
//! folder, is guarded by the disk reserve before it starts, is watched while it runs, and is
//! tee'd to the terminal and to `scratch/logs/jobs/`.

use crate::buildsrc;
use crate::config::Config;
use crate::git;
use crate::pause;
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

// ---- projects ---------------------------------------------------------------------------

/// Known project names: `broadsword` plus every folder in `projects_dir` holding a Cargo.toml.
pub fn list_projects(cfg: &Config) -> Vec<String> {
    let mut names = vec![SELF_PROJECT.to_string()];
    let mut found: Vec<String> = std::fs::read_dir(&cfg.projects_dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join("Cargo.toml").is_file())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n != SELF_PROJECT)
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    names.extend(found);
    names
}

/// The ONE place a project name is accepted: the canonical name (the folder's real name as `list_projects` gives it,
/// or `broadsword`), matched case-insensitively. A name is one plain folder name; `..`, separators, absolute paths
/// and names ending in a dot or space (Windows treats `Crash.` as `Crash`) are rejected, so `Crash`, `crash` and
/// `Crash.` can never be different projects.
pub fn canonical_project(projects_dir: &Path, name: &str) -> Result<String, String> {
    let mut comps = Path::new(name).components();
    let plain = matches!((comps.next(), comps.next()), (Some(Component::Normal(_)), None)) && !name.contains(['/', '\\', ':']) && !name.ends_with(['.', ' ']);
    if !plain {
        return Err(format!("invalid project name `{name}`: use a plain folder name, no path, `..`, absolute path, or trailing dot or space"));
    }
    let want = name.to_lowercase();
    if want == SELF_PROJECT {
        return Ok(SELF_PROJECT.to_string());
    }
    let mut found: Vec<String> = std::fs::read_dir(projects_dir)
        .ok()
        .into_iter()
        .flat_map(|rd| rd.flatten())
        .filter(|e| e.path().join("Cargo.toml").is_file())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.to_lowercase() == want)
        .collect();
    found.sort();
    found.into_iter().next().ok_or_else(|| format!("unknown project `{name}`"))
}

/// Name -> project folder (see `canonical_project` for what a name may be).
pub fn resolve_project(root: &Path, projects_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let canon = canonical_project(projects_dir, name)?;
    Ok(if canon == SELF_PROJECT { root.to_path_buf() } else { projects_dir.join(canon) })
}

pub fn cmd_projects(cfg: &Config) -> ExitCode {
    for n in list_projects(cfg) {
        let dir = resolve_project(&cfg.root, &cfg.projects_dir, &n).unwrap_or_default();
        println!("{n}\t{}", dir.display());
    }
    ExitCode::SUCCESS
}

// ---- disk guard -------------------------------------------------------------------------

/// Pure decision. Err(message) = refuse to start. Free space minus the reserve landing exactly on the floor is allowed.
pub fn guard_decision(paused: bool, free: u64, reserve: u64, floor: u64) -> Result<(), String> {
    if paused {
        return Err("Broadsword is paused (see `broadsword status`); run `broadsword resume` once space is freed".into());
    }
    if free < floor.saturating_add(reserve) {
        return Err(format!(
            "free space {} minus the {} job reserve would be below the {} floor",
            pause::gb(free),
            pause::gb(reserve),
            pause::gb(floor)
        ));
    }
    Ok(())
}

/// Free-space source for a running job. Normally the real disk. TEST ONLY: if the env var
/// `BROADSWORD_INTERNAL_TEST_FREE_BYTES_FILE` names a file inside this root's scratch folder, its
/// whitespace-separated byte counts are used one per call (the last repeats) to simulate the disk
/// filling mid-job. Every reading is `min(hook value, real free space)`, so the hook can only make
/// the disk look fuller, never emptier: it can never switch off the real guard. Files outside
/// scratch are ignored, and use is announced on stderr.
pub fn hooked_free_source(cfg: &Config) -> Box<dyn FnMut() -> u64> {
    let root = cfg.root.clone();
    if let Ok(v) = std::env::var(TEST_FREE_HOOK_ENV) {
        let path = PathBuf::from(&v);
        let inside = match (path.canonicalize(), cfg.scratch_dir.canonicalize()) {
            (Ok(p), Ok(s)) => p.starts_with(s),
            _ => false,
        };
        if inside {
            eprintln!("broadsword: TEST HOOK ACTIVE: free space is read from {v} (never above the real free space)");
            let mut n = 0usize;
            return Box::new(move || {
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                let vals: Vec<u64> = text.split_whitespace().filter_map(|t| t.parse().ok()).collect();
                let hooked = vals.get(n.min(vals.len().saturating_sub(1))).copied().unwrap_or(0);
                n += 1;
                hooked.min(real_free(&root))
            });
        }
        eprintln!("broadsword: ignoring {TEST_FREE_HOOK_ENV}: file is not inside the scratch folder");
    }
    Box::new(move || real_free(&root))
}

/// Real free space on the root's disk.
fn real_free(root: &Path) -> u64 {
    sys::disk(root).map(|d| d.free).unwrap_or_else(|e| {
        eprintln!("disk check failed: {e}");
        0
    })
}

// ---- output parsing ---------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq, Clone, Copy)]
pub struct TestCounts {
    pub passed: u64,
    pub failed: u64,
    pub ignored: u64,
    pub binaries: u64,
}

/// Sum every `test result: ... N passed; N failed; N ignored; ...` line. None if no such line exists.
pub fn parse_test_results(output: &str) -> Option<TestCounts> {
    let mut c = TestCounts::default();
    for line in output.lines() {
        let Some(rest) = line.trim_start().strip_prefix("test result:") else { continue };
        c.binaries += 1;
        for part in rest.split([';', '.']) {
            let mut words = part.split_whitespace();
            if let (Some(n), Some(label)) = (words.next(), words.next()) {
                if let Ok(n) = n.parse::<u64>() {
                    match label {
                        "passed" => c.passed += n,
                        "failed" => c.failed += n,
                        "ignored" => c.ignored += n,
                        _ => {}
                    }
                }
            }
        }
    }
    (c.binaries > 0).then_some(c)
}

// ---- running cargo ----------------------------------------------------------------------

pub enum Outcome {
    Exited(Option<i32>),
    DiskDropped(u64),
}

pub struct StepRun {
    pub outcome: Outcome,
    pub stdout: String,
    pub secs: f64,
}

type Log = Arc<Mutex<std::fs::File>>;

fn tee(src: impl Read, to_stderr: bool, log: Log, keep: Option<Arc<Mutex<Vec<u8>>>>) {
    let mut r = std::io::BufReader::new(src);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if to_stderr {
            let _ = std::io::stderr().write_all(&buf);
        } else {
            let _ = std::io::stdout().write_all(&buf);
            let _ = std::io::stdout().flush();
        }
        if let Ok(mut f) = log.lock() {
            let _ = f.write_all(&buf);
        }
        if let Some(k) = &keep {
            if let Ok(mut k) = k.lock() {
                k.extend_from_slice(&buf);
            }
        }
    }
}

fn kill_tree(child: &mut std::process::Child) {
    // cargo starts rustc and build scripts, a shell starts its commands: take the whole tree down.
    sys::kill_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// Run a prepared command, polling `free` every `poll`; below the floor the job is killed.
/// `on_spawn` gets the child's pid as soon as it exists (recorded so recovery can find an orphan).
pub fn run_child(
    cfg: &Config,
    mut cmd: Command,
    log: &Log,
    free: &mut dyn FnMut() -> u64,
    poll: Duration,
    on_spawn: &mut dyn FnMut(u32),
) -> Result<StepRun, String> {
    let start = Instant::now();
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start `{}`: {e}", cmd.get_program().to_string_lossy()))?;
    on_spawn(child.id());
    let kept = Arc::new(Mutex::new(Vec::new()));
    let (out, err) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
    let t_out = {
        let (l, k) = (Arc::clone(log), Arc::clone(&kept));
        std::thread::spawn(move || tee(out, false, l, Some(k)))
    };
    let t_err = {
        let l = Arc::clone(log);
        std::thread::spawn(move || tee(err, true, l, None))
    };
    let mut next_poll = Instant::now() + poll;
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Outcome::Exited(status.code()),
            Ok(None) => {}
            Err(e) => return Err(format!("waiting for the job failed: {e}")),
        }
        if Instant::now() >= next_poll {
            let f = free();
            if pause::below_floor(f, cfg.floor_bytes()) {
                kill_tree(&mut child);
                break Outcome::DiskDropped(f);
            }
            next_poll = Instant::now() + poll;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    if matches!(outcome, Outcome::Exited(_)) {
        let _ = t_out.join();
        let _ = t_err.join();
    } // after a kill the readers are left to finish on their own: never block on orphaned pipes
    let stdout = String::from_utf8_lossy(&kept.lock().map(|k| k.clone()).unwrap_or_default()).into_owned();
    Ok(StepRun { outcome, stdout, secs: start.elapsed().as_secs_f64() })
}

/// `cargo <sub> --offline` in `project_dir` with the shared build folder.
fn cargo_command(cfg: &Config, project_dir: &Path, sub: &str) -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.arg(sub).arg("--offline").current_dir(project_dir).env("CARGO_TARGET_DIR", &cfg.build_dir);
    c
}

/// An arbitrary command line run through the system shell in `project_dir`.
fn shell_command(project_dir: &Path, line: &str) -> Command {
    #[cfg(windows)]
    let mut c = {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.arg("/C").raw_arg(format!("\"{line}\"")); // cmd strips one outer pair of quotes
        c
    };
    #[cfg(not(windows))]
    let mut c = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(line);
        c
    };
    c.current_dir(project_dir);
    c
}

// ---- commands ---------------------------------------------------------------------------

pub fn file_stamp(secs: u64) -> String {
    // "YYYY-MM-DD HH:MM:SS UTC" -> "YYYYMMDD-HHMMSS"
    pause::format_utc(secs).chars().take(19).filter(|c| c.is_ascii_digit() || *c == ' ').map(|c| if c == ' ' { '-' } else { c }).collect()
}

fn exit_text(c: Option<i32>) -> String {
    c.map(|c| format!("exit {c}")).unwrap_or_else(|| "terminated".into())
}

pub fn step_line_build(run: &StepRun) -> (String, bool) {
    match run.outcome {
        Outcome::Exited(Some(0)) => (format!("build: ok ({:.1} s)", run.secs), true),
        Outcome::Exited(c) => (format!("build: FAILED ({})", exit_text(c)), false),
        Outcome::DiskDropped(_) => ("build: KILLED (disk below floor)".into(), false),
    }
}

pub fn step_line_run(run: &StepRun) -> (String, bool) {
    match run.outcome {
        Outcome::Exited(Some(0)) => (format!("run: ok ({:.1} s)", run.secs), true),
        Outcome::Exited(c) => (format!("run: FAILED ({})", exit_text(c)), false),
        Outcome::DiskDropped(_) => ("run: KILLED (disk below floor)".into(), false),
    }
}

pub fn step_line_test(run: &StepRun) -> (String, bool) {
    let counts = parse_test_results(&run.stdout);
    let code_ok = matches!(run.outcome, Outcome::Exited(Some(0)));
    let line = match (&run.outcome, counts) {
        (Outcome::DiskDropped(_), _) => "test: KILLED (disk below floor)".to_string(),
        (Outcome::Exited(c), Some(n)) if !code_ok || n.failed > 0 => {
            format!("test: FAILED ({}) — {} passed, {} failed", exit_text(*c), n.passed, n.failed)
        }
        (_, Some(n)) => {
            let ign = if n.ignored > 0 { format!(", {} ignored", n.ignored) } else { String::new() };
            format!("test: ok — {} passed, {} failed{ign}", n.passed, n.failed)
        }
        (Outcome::Exited(Some(0)), None) => "test: ok — no `test result:` lines found, so no counts".to_string(),
        (Outcome::Exited(c), None) => format!("test: FAILED ({}) — no `test result:` lines found", exit_text(*c)),
    };
    let ok = code_ok && counts.is_none_or(|n| n.failed == 0);
    (line, ok)
}

/// Create `<base>.log`, or `<base>-2.log`, `-3`... if taken: an earlier log is never overwritten.
pub fn create_log(dir: &Path, base: &str) -> std::io::Result<(PathBuf, std::fs::File)> {
    for n in 1u32.. {
        let file = if n == 1 { format!("{base}.log") } else { format!("{base}-{n}.log") };
        let path = dir.join(file);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

/// The steps of a job kind, in order. `run` is one step: the user's command.
pub fn steps_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "build" => &["build"],
        "test" => &["test"],
        "run" => &["run"],
        _ => &["build", "test"],
    }
}

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
fn end(rec: &mut Record, status: &str, code: ExitCode) -> ExitCode {
    rec.set("status", status);
    rec.set("child_pid", "");
    rec.set("child_start", "");
    rec.set("ended", now_secs().to_string());
    match save_record(rec) {
        Ok(()) => code,
        Err(e) => {
            eprintln!("broadsword: ERROR: {e}; the job's real status is `{status}`");
            ExitCode::from(EXIT_FAILED)
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

/// Run a job: `kind` is build, test, check or run. Writes a durable record, takes a Git checkpoint
/// before the first step (when the project is a Git repo), and runs the steps.
/// The build lock, held until the job function returns. For `run`, dropping it first marks the notes of the folder's
/// packages unknown (the command may have built anything), so the next build, test or check cleans and `install` refuses.
struct BuildHold {
    forget: Option<(Config, PathBuf)>,
    _lock: crate::lock::Lock, // dropped after the Drop body below: the notes change while the lock is still held
}

impl Drop for BuildHold {
    fn drop(&mut self) {
        if let Some((cfg, dir)) = &self.forget {
            buildsrc::forget_notes(cfg, dir);
        }
    }
}

pub fn run_job(cfg: &Config, spec: JobSpec) -> ExitCode {
    let kind = spec.kind.as_str();
    let project_dir = match job_folder(cfg, &spec.name, spec.task.as_deref()) {
        Ok(d) => d,
        Err((code, msg)) => {
            eprintln!("broadsword: {msg}");
            return ExitCode::from(code);
        }
    };
    let source = match buildsrc::rel_source(cfg, &project_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("broadsword: {e}");
            return ExitCode::from(EXIT_FAILED);
        }
    };
    let name = canonical_project(&cfg.projects_dir, &spec.name).unwrap_or_else(|_| spec.name.clone()); // records store the canonical name
    let name = name.as_str();
    let steps = steps_for(kind);
    let mut free_src = hooked_free_source(cfg);
    let poll = Duration::from_secs(cfg.job_disk_poll_seconds.max(1));

    let secs = now_secs();
    let log_dir = cfg.scratch_dir.join("logs").join("jobs");
    let log_opened = std::fs::create_dir_all(&log_dir).and_then(|_| create_log(&log_dir, &format!("{}-{name}-{kind}", file_stamp(secs))));
    let (log_path, log): (PathBuf, Log) = match log_opened {
        Ok((p, f)) => (p, Arc::new(Mutex::new(f))),
        Err(e) => {
            eprintln!("broadsword: cannot create job log in {}: {e}", log_dir.display());
            return ExitCode::from(EXIT_FAILED);
        }
    };

    // The record comes first: from here on a kill at any moment leaves something to recover from.
    let command_text = match &spec.command {
        Some(c) => c.clone(),
        None => steps.iter().map(|s| format!("cargo {s} --offline")).collect::<Vec<_>>().join("; "),
    };
    let mut rec = match record::create(&record::jobs_dir(cfg), &format!("{}-{name}-{kind}", file_stamp(secs))) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("broadsword: cannot create job record in {}: {e}", record::jobs_dir(cfg).display());
            return ExitCode::from(EXIT_FAILED);
        }
    };
    let me = std::process::id();
    rec.set("project", name);
    rec.set("kind", kind);
    rec.set("source", source.as_str());
    rec.set("task", spec.task.as_deref().unwrap_or(""));
    rec.set("command", command_text);
    rec.set("status", record::RUNNING);
    rec.set("pid", me.to_string());
    rec.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
    rec.set("child_pid", "");
    rec.set("child_start", "");
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
        return ExitCode::from(EXIT_FAILED);
    }
    let job_id = rec.id().to_string();
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
            return end(&mut rec, record::FAILED, ExitCode::from(EXIT_FAILED));
        }
    }
    println!("job: {kind} {name} (id {job_id}, log: {})", log_path.display());
    println!("source: {source}{}", spec.task.as_deref().map(|t| format!(" (task {t})")).unwrap_or_default());

    if kind == "run" && !git::is_repo(&project_dir) {
        eprintln!("broadsword: refusing to run: project `{name}` is not a Git repository, so there is no checkpoint to roll back to (run `git init` in {} first)", project_dir.display());
        return end(&mut rec, record::REFUSED, ExitCode::from(EXIT_REFUSED));
    }

    let mut summary: Vec<String> = Vec::new();
    let mut all_ok = true;
    let mut exit_code: Option<i32> = None;
    let mut build_hold: Option<BuildHold> = None; // held (when taken) until this function returns
    for (i, step) in steps.iter().enumerate().skip(spec.first_step) {
        let free = free_src();
        if let Err(why) = guard_decision(pause::is_paused(&cfg.root), free, cfg.reserve_bytes(), cfg.floor_bytes()) {
            eprintln!("broadsword: refusing to start {step}: {why}");
            return end(&mut rec, record::REFUSED, ExitCode::from(EXIT_REFUSED));
        }
        if i == spec.first_step {
            if git::is_repo(&project_dir) {
                match git::checkpoint(&project_dir, &cfg.state_dir, &job_id, &git::Opts::for_project(cfg, &project_dir)) {
                    Ok(c) => {
                        println!("checkpoint: {} ({})", c.ref_name, &c.commit[..c.commit.len().min(10)]);
                        rec.set("checkpoint", c.ref_name);
                        rec.set("checkpoint_commit", c.commit);
                        rec.set("checkpoint_ignored", c.ignored_ref.unwrap_or_default());
                        rec.set("index_tree", c.index_tree.unwrap_or_default());
                        if !c.skipped.is_empty() {
                            eprintln!(
                                "broadsword: warning: {} ignored file(s) were NOT saved in the checkpoint (over the {} MB checkpoint_ignored_max_mb limit): {}",
                                c.skipped.len(),
                                cfg.checkpoint_ignored_max_mb,
                                c.skipped.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join(", ")
                            );
                            rec.set("ignored_skipped", c.skipped.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>().join("|"));
                            rec.set("ignored_skipped_len", c.skipped.iter().map(|(_, n)| n.to_string()).collect::<Vec<_>>().join("|"));
                        }
                    }
                    Err(e) if kind == "run" => {
                        eprintln!("broadsword: refusing to run: cannot take a checkpoint: {e}");
                        return end(&mut rec, record::REFUSED, ExitCode::from(EXIT_REFUSED));
                    }
                    Err(e) => {
                        eprintln!("broadsword: warning: no checkpoint: {e}");
                        rec.set("checkpoint", "none");
                    }
                }
            } else {
                rec.set("checkpoint", "none");
            }
        }
        if build_hold.is_none() {
            // The first step this job EXECUTES (whatever step a resumed job starts at: `skip(first_step)` above), so the
            // lock and the prepare below are never skipped. Clean, record and build are one step across processes: take
            // the build lock now and keep it until this job ends (every cargo step below, tests included, runs the
            // binaries the shared folder holds). `run` runs a shell command that may call cargo itself, so it takes the
            // lock too and, instead of preparing, forgets the notes of the folder's packages (before the command, so a
            // crash cannot leave a stale note, and again after it).
            let lock = match buildsrc::lock(cfg) {
                Ok(l) => l,
                Err((code, msg)) => {
                    eprintln!("broadsword: {msg}");
                    return end(&mut rec, if code == buildsrc::EXIT_BUSY { record::REFUSED } else { record::FAILED }, ExitCode::from(code));
                }
            };
            build_hold = Some(BuildHold { forget: (kind == "run").then(|| (cfg.clone(), project_dir.clone())), _lock: lock });
            let outcome = if kind == "run" {
                Ok(buildsrc::forget_notes(cfg, &project_dir))
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
                    return end(&mut rec, record::FAILED, ExitCode::from(EXIT_FAILED));
                }
            }
        }
        rec.set("step", *step);
        if let Err(e) = save_record(&rec) {
            eprintln!("broadsword: {e}");
            return end(&mut rec, record::FAILED, ExitCode::from(EXIT_FAILED));
        }
        if let Ok(mut f) = log.lock() {
            let _ = writeln!(f, "=== {step} {name} ({}) ===", pause::now_string());
        }
        let cmd = match (&spec.command, *step) {
            (Some(line), "run") => shell_command(&project_dir, line),
            _ => cargo_command(cfg, &project_dir, step),
        };
        let mut spawn_err: Option<String> = None;
        let mut on_spawn = |pid: u32| {
            rec.set("child_pid", pid.to_string());
            rec.set("child_start", sys::start_time(pid).map(|t| t.to_string()).unwrap_or_default());
            if let Err(e) = save_record(&rec) {
                spawn_err = Some(e);
            }
        };
        let run = match run_child(cfg, cmd, &log, &mut *free_src, poll, &mut on_spawn) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("broadsword: {e}");
                return end(&mut rec, record::FAILED, ExitCode::from(EXIT_FAILED));
            }
        };
        if let Some(e) = spawn_err {
            eprintln!("broadsword: ERROR: {e}");
            return end(&mut rec, record::FAILED, ExitCode::from(EXIT_FAILED));
        }
        if let Outcome::DiskDropped(f) = run.outcome {
            let reason = format!("free space {} fell below the {} GB floor during `{kind} {name}`; job killed", pause::gb(f), cfg.disk_floor_gb);
            let _ = pause::write_pause(&cfg.root, f, &reason);
            pause::alert(cfg, &format!("Broadsword PAUSED: {reason}"));
            eprintln!("broadsword: {reason}");
            return end(&mut rec, record::KILLED_LOW_DISK, ExitCode::from(EXIT_DISK_DROPPED));
        }
        if let Outcome::Exited(c) = run.outcome {
            exit_code = c;
        }
        let (line, ok) = match *step {
            "build" => step_line_build(&run),
            "run" => step_line_run(&run),
            _ => step_line_test(&run),
        };
        summary.push(line);
        if !ok {
            all_ok = false;
            break; // check never tests a failed build
        }
    }
    println!("--- summary ---");
    for l in &summary {
        println!("{l}");
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
    end(&mut rec, if all_ok { record::DONE } else { record::FAILED }, ExitCode::from(code))
}

#[cfg(test)]
mod tests {
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
}
