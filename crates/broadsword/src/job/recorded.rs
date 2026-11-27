use super::*;

use super::child::shell_command;
use super::projects::low_disk_cleanup;
use super::python::python_record;
use super::teststep::refresh_main_bins;
use crate::git;
use std::io::IsTerminal;

/// The text of a panic payload.
fn panic_text(p: &(dyn std::any::Any + Send)) -> String {
    p.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| p.downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into())
}

pub(super) fn run_job_recorded(cfg: &Config, spec: JobSpec, info: &mut JobInfo, direct: bool) -> u8 {
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
        let mut flaky = None;
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
                flaky: &mut flaky,
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
        if let Some(u) = &flaky {
            rec.set("flaky_rerun", teststep::flakerun::record(u));
        }
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
        let (line, ok) =
            teststep::flakerun::step_line((line, ok), flaky.as_ref(), &run, py.as_ref());
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
