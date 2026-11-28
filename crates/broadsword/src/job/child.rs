use super::*;

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

pub(super) type Log = Arc<Mutex<std::fs::File>>;

/// Every line is redacted (every `.env` value becomes `[REDACTED]`) before it reaches the terminal, the job log or the kept
/// output. Per line: a secret split across two lines is a known limit.
fn tee(src: impl Read, to_stderr: bool, echo: bool, log: Log, keep: Option<Arc<Mutex<Vec<u8>>>>, secrets: Arc<Secrets>) {
    let mut r = std::io::BufReader::new(src);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        {
            let text = String::from_utf8_lossy(&buf);
            let red = secrets.redact(&text);
            if red != text {
                buf = red.into_bytes();
            }
        }
        if !echo {
            // machine output (cargo's JSON): kept for the caller only, never echoed or logged
        } else if to_stderr {
            let _ = std::io::stderr().write_all(&buf);
        } else {
            let _ = std::io::stdout().write_all(&buf);
            let _ = std::io::stdout().flush();
        }
        if echo {
            if let Ok(mut f) = log.lock() {
                let _ = f.write_all(&buf);
            }
        }
        if let Some(k) = &keep {
            if let Ok(mut k) = k.lock() {
                k.extend_from_slice(&buf);
            }
        }
    }
}

pub(super) fn kill_tree(child: &mut std::process::Child) {
    // cargo starts rustc and build scripts, a shell starts its commands: take the whole tree down.
    sys::kill_tree(child.id());
    let _ = child.kill();
    let _ = child.wait();
}

/// Run a prepared command, polling `free` every `poll`; below the floor the job is killed.
/// `on_spawn` gets the child's pid as soon as it exists (recorded so recovery can find an orphan).
pub fn run_child(
    cfg: &Config,
    cmd: Command,
    log: &Log,
    free: &mut dyn FnMut() -> u64,
    poll: Duration,
    on_spawn: &mut dyn FnMut(u32),
    secrets: &Arc<Secrets>,
) -> Result<StepRun, String> {
    run_child_opts(cfg, cmd, log, free, poll, on_spawn, secrets, true)
}

/// `run_child`, with `echo_stdout` false for machine output (cargo's JSON): stdout is then only kept (redacted), never printed
/// or logged; stderr is printed and logged as always.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_child_opts(
    cfg: &Config,
    mut cmd: Command,
    log: &Log,
    free: &mut dyn FnMut() -> u64,
    poll: Duration,
    on_spawn: &mut dyn FnMut(u32),
    secrets: &Arc<Secrets>,
    echo_stdout: bool,
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
        let (l, k, sc) = (Arc::clone(log), Arc::clone(&kept), Arc::clone(secrets));
        std::thread::spawn(move || tee(out, false, echo_stdout, l, Some(k), sc))
    };
    let t_err = {
        let (l, sc) = (Arc::clone(log), Arc::clone(secrets));
        std::thread::spawn(move || tee(err, true, true, l, None, sc))
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
        let start_drain = Instant::now();
        let wait = Duration::from_secs(cfg.test_output_wait_seconds);
        let poll = Duration::from_millis(50);
        loop {
            if t_out.is_finished() && t_err.is_finished() {
                let _ = t_out.join();
                let _ = t_err.join();
                break;
            }
            let remaining = wait.saturating_sub(start_drain.elapsed());
            if remaining.is_zero() {
                let note = format!(
                    "broadsword: TRUNCATED: this command exited, but a process it started still held its output open {} s later (test_output_wait_seconds); its output above is as far as it got",
                    wait.as_secs()
                );
                if let Ok(mut f) = log.lock() {
                    // A trailing partial line must not hide the verdict marker.
                    let _ = writeln!(f, "\n{note}");
                }
                eprintln!("\n{note}");
                break;
            }
            std::thread::sleep(poll.min(remaining));
        }
    } // on timeout or kill the readers are left to finish on their own
    let stdout = String::from_utf8_lossy(&kept.lock().map(|k| k.clone()).unwrap_or_default()).into_owned();
    Ok(StepRun { outcome, stdout, secs: start.elapsed().as_secs_f64() })
}

/// `cargo <sub> --offline` in `project_dir` (the caller sets the shared build folder).
pub(super) fn cargo_command(cfg: &Config, project_dir: &Path, sub: &str) -> Command {
    let mut c = buildsrc::cargo(cfg); // the one cargo factory: scrubbed environment
    c.arg(sub).arg("--offline").current_dir(project_dir);
    c
}

/// An arbitrary command line run through the system shell in `project_dir` (the caller sets the shared build folder).
pub(super) fn shell_command(project_dir: &Path, line: &str) -> Command {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn run_shell(tag: &str, line: &str, wait_s: u64) -> (StepRun, String) {
        let root = crate::scrub::planted::config_root(tag);
        let mut cfg = Config::from_text(include_str!("../../../../broadsword.toml"), &root).unwrap();
        cfg.test_output_wait_seconds = wait_s;
        let path = root.join("child.log");
        let log = Arc::new(Mutex::new(std::fs::File::create(&path).unwrap()));
        let run = run_child(
            &cfg,
            shell_command(&root, line),
            &log,
            &mut || u64::MAX,
            Duration::from_secs(cfg.job_disk_poll_seconds),
            &mut |_| {},
            &Arc::new(Secrets::collect(&root, &[], usize::try_from(cfg.redact_min_chars).unwrap())),
        );
        let text = std::fs::read_to_string(&path).unwrap();
        drop(log);
        // A detached reader can still own the log handle on Windows. Files in the
        // scratch fixture can then be removed by the normal scratch cleanup.
        let _ = std::fs::remove_dir_all(&root);
        (run.unwrap(), text)
    }

    #[test]
    fn a_grandchild_holding_the_pipe_cannot_stall_run_child() {
        let line = if cfg!(windows) {
            r#"start /b "" ping -n 31 127.0.0.1 & echo hi"#
        } else {
            "sleep 30 & echo hi"
        };
        let (run, log) = run_shell("child-orphan-pipe", line, 1);
        println!("grandchild pipe: elapsed={} s; log={log:?}", run.secs);
        assert!(matches!(run.outcome, Outcome::Exited(Some(0))));
        assert!(run.stdout.lines().any(|l| l == "hi"), "{:?}", run.stdout);
        assert!(
            log.lines().any(|l| {
                l.starts_with("broadsword: TRUNCATED:")
                    && l.contains("open 1 s later (test_output_wait_seconds)")
            }),
            "{log}"
        );
        // The grandchild holds the pipe ~30 s: a sub-30 s bound rules out run_child waiting for it; 10 s failed under suite load (20261127-202709, 20261128-023251).
        assert!(run.secs < 25.0, "pipe held the job for {} s", run.secs);
    }

    #[test]
    fn a_child_without_grandchildren_is_joined_whole() {
        let (run, log) = run_shell("child-closed-pipe", "echo hi", 10);
        assert!(matches!(run.outcome, Outcome::Exited(Some(0))));
        assert_eq!(run.stdout.trim_end(), "hi");
        assert_eq!(log.trim_end(), "hi");
    }
}
