use super::*;

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

pub fn file_stamp(secs: u64) -> String {
    // "YYYY-MM-DD HH:MM:SS UTC" -> "YYYYMMDD-HHMMSS"
    pause::format_utc(secs).chars().take(19).filter(|c| c.is_ascii_digit() || *c == ' ').map(|c| if c == ' ' { '-' } else { c }).collect()
}

pub(super) fn exit_text(c: Option<i32>) -> String {
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
