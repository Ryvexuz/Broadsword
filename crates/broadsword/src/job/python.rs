use super::*;

// ---- the Python tests of the test step (SCALING F9) -----------------------------------------

/// The Python interpreter the test step runs, resolved on the child's `PATH` (as `cargo` is). Not a setting: like `cargo`, it is
/// a program name, and a machine without it fails the job loudly ("cannot start `python`").
pub const PYTHON: &str = "python";

/// What the Python tests of one test step did.
#[derive(Debug, Clone, PartialEq)]
pub enum PyTests {
    /// The project is not named in `python_tests`: it has no Python tests.
    NoneDeclared,
    /// Every declared script exited 0.
    Ok { count: usize, secs: f64 },
    /// The first script that failed (the rest were not run), and how.
    Failed { script: String, why: String },
    /// Free space fell below the floor while a script ran: it was killed (free bytes at the kill).
    Killed(u64),
}

/// The scripts `python_tests` declares for `project`, matched case-insensitively (project names are); empty when it is not named.
pub fn python_scripts<'a>(cfg: &'a Config, project: &str) -> &'a [String] {
    cfg.python_tests.iter().find(|(p, _)| p.eq_ignore_ascii_case(project)).map(|(_, s)| s.as_slice()).unwrap_or(&[])
}

/// `python -I -B <script>` in `project_dir`, made by the ONE spawn helper, so the child gets the scrubbed environment (`scrub.rs`:
/// no secret-named variable, nothing holding a `.env` value). `-I` (isolated: no `PYTHON*` variables, no user site, no script
/// folder or current folder on `sys.path`) and `-B` (no `.pyc` files written into the checkout) are always passed.
pub(super) fn python_command(cfg: &Config, project_dir: &Path, script: &str) -> Command {
    let mut c = scrub::command(cfg, PYTHON);
    c.args(["-I", "-B", script]).current_dir(project_dir);
    c
}

/// The test step's summary line with its Python tests added: `test: ok — ...; python: N script(s) ok (S s)` when cargo's tests
/// and every script passed; `test: FAILED (python <script>: <why>) — ...` when a script failed (the job fails); unchanged when
/// the project declares none, or when cargo's tests already failed (no script ran).
pub fn step_line_python(line: String, ok: bool, py: &PyTests) -> (String, bool) {
    match py {
        _ if !ok => (line, false),
        PyTests::NoneDeclared => (line, true),
        PyTests::Ok { count, secs } => (format!("{line}; python: {count} script(s) ok ({secs:.1} s)"), true),
        PyTests::Failed { script, why } => {
            let rest = line.strip_prefix("test: ok").unwrap_or("");
            (format!("test: FAILED (python {script}: {why}){rest}"), false)
        }
        PyTests::Killed(_) => ("test: KILLED (disk below floor)".into(), false),
    }
}

/// The job record's `python_tests` field for a finished test step.
pub(super) fn python_record(py: &PyTests) -> String {
    match py {
        PyTests::NoneDeclared => "none declared".into(),
        PyTests::Ok { count, secs } => format!("ok: {count} script(s), {secs:.1} s"),
        PyTests::Failed { script, why } => format!("failed: {script} ({why})"),
        PyTests::Killed(_) => "killed: disk below floor".into(),
    }
}

