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
    /// The first script that failed in declared order, and how. In a pool, already-running scripts finish.
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

impl TestStep<'_> {
    /// Python-only pool, called after cargo's tests and doc-tests passed. Validate the whole list before spawning any script.
    pub(super) fn run_python_pool(&mut self, scripts: &[String]) -> Result<PyTests, String> {
        use super::teststep::{PoolEnd, PoolItem};

        if let Some(s) = scripts.iter().find(|s| !self.project_dir.join(s).is_file()) {
            self.note(&format!("error: python test `{PYTHON} -I -B {s}`: {s} is not a file in {}", self.project_dir.display()));
            return Ok(PyTests::Failed { script: s.clone(), why: "script not found".into() });
        }
        let items = scripts
            .iter()
            .map(|s| {
                let shown = format!("{PYTHON} -I -B {s}");
                PoolItem {
                    title: format!("     Running {shown}"),
                    cmd: python_command(self.cfg, self.project_dir, s),
                    fail_line: Box::new(move |why| format!("error: python test failed: `{shown}` ({why})")),
                    counts: false,
                }
            })
            .collect();
        let start = Instant::now();
        let ran = match self.run_pool(items, &mut String::new(), self.cfg.python_parallel, "python_parallel")? {
            PoolEnd::DiskDropped(f) => return Ok(PyTests::Killed(f)),
            PoolEnd::Ran(ran) => ran,
        };
        if let Some((s, f)) = scripts.iter().zip(&ran).find(|(_, f)| f.as_ref().is_some_and(|f| f.code != Some(0))) {
            return Ok(PyTests::Failed { script: s.clone(), why: exit_text(f.as_ref().and_then(|f| f.code)) });
        }
        if ran.len() != scripts.len() || ran.iter().any(Option::is_none) {
            return Err("python_parallel: pool ended without a result for every script".into());
        }
        Ok(PyTests::Ok { count: scripts.len(), secs: start.elapsed().as_secs_f64() })
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pool(tag: &str, second_exists: bool, free_bytes: u64) -> (PyTests, Vec<Vec<u32>>) {
        let dir = crate::scrub::planted::config_root(tag);
        let mut cfg = Config::load_from(&dir).unwrap();
        cfg.python_parallel = 2;
        cfg.disk_floor_gb = 1;
        std::fs::write(dir.join("a.py"), "import time\ntime.sleep(30)\n").unwrap();
        if second_exists {
            std::fs::write(dir.join("b.py"), "import time\ntime.sleep(30)\n").unwrap();
        }
        let scripts = vec!["a.py".into(), "b.py".into()];
        let log = Arc::new(Mutex::new(std::fs::File::create(dir.join("pool.log")).unwrap()));
        let secrets = Arc::new(scrub::secrets(&cfg));
        let mut free = || free_bytes;
        let mut live = Vec::new();
        let mut on_live = |pids: &[u32]| live.push(pids.to_vec());
        let mut main_bins_failed = None;
        let result = TestStep {
            cfg: &cfg,
            project: tag,
            priority: crate::lock::Priority::Agent,
            job_id: tag,
            project_dir: &dir,
            source: "main",
            log: &log,
            free: &mut free,
            poll: Duration::ZERO,
            on_live: &mut on_live,
            secrets: &secrets,
            parallel: 1,
            flaky: &mut None,
            main_bins_failed: &mut main_bins_failed,
            step_bins: None,
        }
        .run_python_pool(&scripts)
        .unwrap();
        drop(log);
        std::fs::remove_dir_all(&dir).unwrap();
        (result, live)
    }

    #[test]
    fn python_parallel_lower_entry_preflights_before_spawning() {
        let (result, live) = pool("python-pool-preflight", false, u64::MAX);
        assert_eq!(result, PyTests::Failed { script: "b.py".into(), why: "script not found".into() });
        assert!(live.is_empty(), "no child may start: {live:?}");
    }

    #[test]
    fn python_parallel_lower_entry_reuses_the_disk_floor_kill() {
        let (result, live) = pool("python-pool-disk", true, 0);
        assert_eq!(result, PyTests::Killed(0));
        let started = live.iter().find(|pids| pids.len() == 2).expect("both pool children started");
        assert!(live.last().unwrap().is_empty(), "live children must be cleared: {live:?}");
        #[cfg(windows)]
        for pid in started {
            assert_eq!(sys::process_state(*pid), sys::ProcState::Gone, "child {pid} outlived the pool");
        }
        #[cfg(not(windows))]
        assert_eq!(started.len(), 2);
    }
}
