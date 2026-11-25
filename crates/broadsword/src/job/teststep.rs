use super::*;

// ---- the test step: compile under the lock, test from copies ------------------------------

/// The folder (inside scratch) a job's copied test and bin executables live in while its test step runs.
pub fn test_copies_dir(cfg: &Config, job_id: &str) -> PathBuf {
    cfg.scratch_dir.join("test-bins").join(job_id)
}

/// One executable `cargo test --no-run` reported (a `compiler-artifact` message with an `executable`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artifact {
    /// The package's Cargo.toml.
    pub manifest: PathBuf,
    /// The target's first kind: `lib`, `bin`, `test`, `bench`, `example`, ...
    pub kind: String,
    /// The target's name (`CARGO_BIN_EXE_<name>` for a bin).
    pub name: String,
    pub src_path: String,
    pub exe: PathBuf,
}

/// What `cargo test --no-run` built: the test executables in the order `cargo test` runs them, every package's bin
/// executables (for `CARGO_BIN_EXE_*`), and whether a library has doc-tests to run.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Artifacts {
    pub tests: Vec<Artifact>,
    pub bins: Vec<Artifact>,
    pub doctests: bool,
}

/// The order `cargo test` runs a package's test executables in: the library's unit tests, the bins', then integration tests,
/// benches and examples, each by name.
fn kind_rank(kind: &str) -> u8 {
    match kind {
        "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro" => 0,
        "bin" => 1,
        "test" => 2,
        "bench" => 3,
        _ => 4,
    }
}

/// Read cargo's `--message-format=json*` stdout. Lines that are not JSON objects are skipped (cargo prints none there).
pub fn parse_artifacts(stdout: &str) -> Result<Artifacts, String> {
    use crate::json::J;
    let mut a = Artifacts::default();
    for line in stdout.lines().map(str::trim).filter(|l| l.starts_with('{')) {
        let j = crate::json::parse(line).map_err(|e| format!("cannot read cargo's JSON output: {e}: {line}"))?;
        if j.get("reason").and_then(J::str) != Some("compiler-artifact") {
            continue;
        }
        let target = j.get("target").ok_or("a compiler-artifact message has no `target`")?;
        let kinds: Vec<&str> = match target.get("kind") {
            Some(J::Arr(v)) => v.iter().filter_map(J::str).collect(),
            _ => Vec::new(),
        };
        let kind = kinds.first().copied().unwrap_or("").to_string();
        let test = matches!(j.get("profile").and_then(|p| p.get("test")), Some(J::Bool(true)));
        if !test && kinds.iter().any(|k| kind_rank(k) == 0) && matches!(target.get("doctest"), Some(J::Bool(true))) {
            a.doctests = true;
        }
        let Some(exe) = j.get("executable").and_then(J::str) else { continue };
        let art = Artifact {
            manifest: PathBuf::from(j.get("manifest_path").and_then(J::str).ok_or("a compiler-artifact message has no `manifest_path`")?),
            kind: kind.clone(),
            name: target.get("name").and_then(J::str).unwrap_or("").to_string(),
            src_path: target.get("src_path").and_then(J::str).unwrap_or("").to_string(),
            exe: PathBuf::from(exe),
        };
        let list = if test { &mut a.tests } else if kind == "bin" { &mut a.bins } else { continue };
        if !list.contains(&art) {
            list.push(art);
        }
    }
    a.tests.sort_by(|x, y| (&x.manifest, kind_rank(&x.kind), &x.name).cmp(&(&y.manifest, kind_rank(&y.kind), &y.name)));
    Ok(a)
}

/// One package's run-time variables as (name, value) pairs, keyed by the package's Cargo.toml.
pub(super) type PackageEnvs = Vec<(PathBuf, Vec<(String, String)>)>;

/// The run-time variables `cargo test` gives a test of each package (`CARGO_MANIFEST_DIR`, `CARGO_MANIFEST_PATH`, `CARGO_PKG_*`),
/// keyed by the package's Cargo.toml, from `cargo metadata --no-deps` in `dir`.
pub(super) fn package_envs(cfg: &Config, dir: &Path) -> Result<PackageEnvs, String> {
    use crate::json::J;
    let j = crate::json::parse(&buildsrc::metadata_json(cfg, dir)?).map_err(|e| format!("cannot read cargo metadata output: {e}"))?;
    let Some(J::Arr(pkgs)) = j.get("packages") else { return Err("cargo metadata output has no `packages` list".into()) };
    let text = |p: &J, k: &str| p.get(k).and_then(J::str).unwrap_or("").to_string();
    let mut v = Vec::new();
    for p in pkgs {
        let manifest = PathBuf::from(text(p, "manifest_path"));
        let version = text(p, "version");
        let (core, pre) = match version.split_once('+').map(|(c, _)| c).unwrap_or(&version).split_once('-') {
            Some((c, pre)) => (c.to_string(), pre.to_string()),
            None => (version.split_once('+').map(|(c, _)| c).unwrap_or(&version).to_string(), String::new()),
        };
        let mut parts = core.split('.');
        let authors = match p.get("authors") {
            Some(J::Arr(a)) => a.iter().filter_map(J::str).collect::<Vec<_>>().join(":"),
            _ => String::new(),
        };
        let mut env = vec![
            ("CARGO_MANIFEST_DIR".to_string(), manifest.parent().map(|d| d.display().to_string()).unwrap_or_default()),
            ("CARGO_MANIFEST_PATH".to_string(), manifest.display().to_string()),
            ("CARGO_PKG_NAME".to_string(), text(p, "name")),
            ("CARGO_PKG_VERSION".to_string(), version.clone()),
            ("CARGO_PKG_VERSION_MAJOR".to_string(), parts.next().unwrap_or("").to_string()),
            ("CARGO_PKG_VERSION_MINOR".to_string(), parts.next().unwrap_or("").to_string()),
            ("CARGO_PKG_VERSION_PATCH".to_string(), parts.next().unwrap_or("").to_string()),
            ("CARGO_PKG_VERSION_PRE".to_string(), pre),
            ("CARGO_PKG_AUTHORS".to_string(), authors),
        ];
        for (var, key) in [("CARGO_PKG_DESCRIPTION", "description"), ("CARGO_PKG_HOMEPAGE", "homepage"), ("CARGO_PKG_REPOSITORY", "repository"), ("CARGO_PKG_LICENSE", "license"), ("CARGO_PKG_LICENSE_FILE", "license_file"), ("CARGO_PKG_RUST_VERSION", "rust_version"), ("CARGO_PKG_README", "readme")] {
            env.push((var.to_string(), text(p, key)));
        }
        v.push((manifest, env));
    }
    Ok(v)
}

/// Does `a` name the same file as `b`? (cargo's paths and metadata's paths come from the same cargo, but compare loosely.)
pub(super) fn same_path(a: &Path, b: &Path) -> bool {
    let n = |p: &Path| {
        let t = p.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
        if cfg!(windows) { t.to_lowercase() } else { t }
    };
    n(a) == n(b)
}

/// Copy `exe` (and a Windows `.pdb` beside it, for readable backtraces) into `dir`, with the copy's modified time set to now so
/// the scratch cleanup (by age) never removes a copy a running job still needs.
pub(super) fn copy_exe(exe: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let to = dir.join(exe.file_name().ok_or_else(|| format!("cargo named an executable with no file name: {}", exe.display()))?);
    std::fs::copy(exe, &to).map_err(|e| format!("cannot copy the test executable {} to {}: {e}", exe.display(), to.display()))?;
    let _ = std::fs::File::options().write(true).open(&to).and_then(|f| f.set_modified(SystemTime::now()));
    let pdb = exe.with_extension("pdb");
    if cfg!(windows) && pdb.is_file() {
        if let Some(n) = pdb.file_name() {
            let _ = std::fs::copy(&pdb, dir.join(n));
        }
    }
    Ok(to)
}

/// What `cargo test` passes to rerun one executable (`--lib`, `--bin x`, `--test x`, ...), for the failure line.
pub(super) fn rerun_arg(a: &Artifact) -> String {
    match kind_rank(&a.kind) {
        0 => "--lib".into(),
        _ => format!("--{} {}", a.kind, a.name),
    }
}

/// One copied test executable, ready to run.
pub(super) struct Copied {
    pub(super) art: Artifact,
    pub(super) copy: PathBuf,
    pub(super) cwd: PathBuf,
    pub(super) env: Vec<(String, String)>,
}

/// Makes a pool child's `error: ...` failure line from its exit text.
pub(super) type FailLine = Box<dyn Fn(&str) -> String>;

/// One child the shared pool runs (a copied test executable or a Python test script).
pub(super) struct PoolItem {
    /// The `     Running ...` line printed before its output.
    pub(super) title: String,
    pub(super) cmd: Command,
    /// The `error: ...` line printed after its output when it fails, given its exit text.
    pub(super) fail_line: FailLine,
    /// Its stdout counts toward the test results (`test result:` lines): true for cargo's executables, false for Python.
    pub(super) counts: bool,
}

/// How one pool child ended.
pub(super) struct Finished {
    pub(super) code: Option<i32>,
    /// Seconds from the pool's start to this child's start and end.
    pub(super) from: f64,
    pub(super) to: f64,
}

/// How a pool run ended: every child that started, by index (`None` = never started, after a failure), or a disk-floor kill.
pub(super) enum PoolEnd {
    Ran(Vec<Option<Finished>>),
    DiskDropped(u64),
}

/// `buildsrc::refresh_main_bins` for this job, its lines printed and logged. The caller holds the build lock and calls it only
/// after a successful cargo step or `run` command, with `before`: the record of cargo's bin exes that `run_job_info` took right
/// before that step (`buildsrc::snapshot_main_bins`, once per step), so only the exes the step produced are kept. A failure is
/// NOT reported here: it goes into `failed` (the first one is kept), and `run_job_info` reports it in its one place, after the
/// job's summary block, and fails the job (main.bins does not hold what was built). Returns false when it failed.
pub(super) fn refresh_main_bins(cfg: &Config, project_dir: &Path, source: &str, log: &Log, before: &buildsrc::StepBins, failed: &mut Option<String>) -> bool {
    match buildsrc::refresh_main_bins(cfg, project_dir, source, before) {
        Ok(lines) => {
            for l in lines {
                println!("{l}");
                if let Ok(mut f) = log.lock() {
                    let _ = writeln!(f, "{l}");
                }
            }
            true
        }
        Err(e) => {
            failed.get_or_insert(e);
            false
        }
    }
}

/// The test step's context: everything `run_child` needs, borrowed from the job.
pub(super) struct TestStep<'a> {
    pub(super) cfg: &'a Config,
    pub(super) priority: crate::lock::Priority,
    pub(super) job_id: &'a str,
    pub(super) project_dir: &'a Path,
    pub(super) source: &'a str,
    pub(super) log: &'a Log,
    pub(super) free: &'a mut dyn FnMut() -> u64,
    pub(super) poll: Duration,
    /// Called with the pids of the children running right now, each time that set changes (one child, or the pool's), so the
    /// record names every live child.
    pub(super) on_live: &'a mut dyn FnMut(&[u32]),
    pub(super) secrets: &'a Arc<Secrets>,
    /// `test_parallel_binaries`: how many test executables (and Python scripts) run at once. 1 = one after another, streamed.
    pub(super) parallel: usize,
    /// Set when refreshing main.bins after the compile failed (see `refresh_main_bins`); the tests still run, and the job
    /// reports it after its summary and fails.
    pub(super) main_bins_failed: &'a mut Option<String>,
    /// What cargo's folder held for main's bin exes before this step (taken by `run_job_info` while it holds the build lock;
    /// `None` without the lock): the refresh after the compile keeps only the exes the compile produced.
    pub(super) step_bins: Option<&'a buildsrc::StepBins>,
}


impl TestStep<'_> {
    pub(super) fn note(&self, line: &str) {
        eprintln!("{line}");
        if let Ok(mut f) = self.log.lock() {
            let _ = writeln!(f, "{line}");
        }
    }

    /// One child, streamed, through `run_child` (the disk poll, the log, redaction, the recorded pid).
    fn child(&mut self, cmd: Command) -> Result<StepRun, String> {
        let on_live = &mut *self.on_live;
        run_child(self.cfg, cmd, self.log, &mut *self.free, self.poll, &mut |pid| on_live(&[pid]), self.secrets)
    }

    /// The test step (see the module comment). `cmd` is the prepared `cargo test --offline` (scrubbed environment, shared build
    /// folder); `hold` is the job's build lock, released here once the executables are copied. Returns one `StepRun` that reads
    /// like the old single `cargo test`: the kept stdout of every executable, cargo's exit code, the whole step's time; and the
    /// result of the project's Python tests `scripts` (SCALING F9), `None` when cargo's tests did not pass (then none counts).
    pub(super) fn run(&mut self, mut cmd: Command, hold: &mut Option<BuildHold>, scripts: &[String]) -> Result<(StepRun, Option<PyTests>), String> {
        let start = Instant::now();
        let done = |outcome: Outcome, stdout: String| StepRun { outcome, stdout, secs: start.elapsed().as_secs_f64() };
        // 1. compile (under the lock)
        cmd.args(["--no-run", "--message-format=json-render-diagnostics"]);
        let on_live = &mut *self.on_live;
        let compiled = run_child_opts(self.cfg, cmd, self.log, &mut *self.free, self.poll, &mut |pid| on_live(&[pid]), self.secrets, false)?;
        match compiled.outcome {
            Outcome::Exited(Some(0)) => {}
            o => return Ok((done(o, String::new()), None)),
        }
        let arts = parse_artifacts(&compiled.stdout)?;
        let envs = package_envs(self.cfg, self.project_dir)?;
        // the compile succeeded: the main checkout's bins it built go to `build/main.bins` while the lock still holds
        if let (Some(_), Some(before)) = (hold.as_ref(), self.step_bins) {
            refresh_main_bins(self.cfg, self.project_dir, self.source, self.log, before, self.main_bins_failed);
        }
        let mut kept = String::new();
        // 2. copy, then release the lock
        let dir = test_copies_dir(self.cfg, self.job_id);
        let copied = self.copy_all(&arts, &envs, &dir);
        let copied = match copied {
            Ok(c) => c,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        };
        if let Some(h) = hold.take() {
            let held = h.taken.elapsed().as_secs_f64();
            drop(h);
            let line = format!("build lock: released after {held:.1} s (compiling only); {} test executable(s) copied to {}", copied.len(), dir.display());
            println!("{line}");
            if let Ok(mut f) = self.log.lock() {
                let _ = writeln!(f, "{line}");
            }
        }
        // 3. run the copies, in cargo's order; a failure stops the step with cargo's exit code (and, as in cargo, no doc-test
        // runs after a failure). Side by side (parallel > 1), the Python tests run in the same pool, after the copies.
        let (result, side_py) = if self.parallel > 1 {
            match self.run_side_by_side(&copied, scripts, &mut kept) {
                Ok((o, py)) => (Ok(o), Some(py)),
                Err(e) => (Err(e), None),
            }
        } else {
            (self.run_copies(&copied, &mut kept), None)
        };
        let _ = std::fs::remove_dir_all(&dir);
        match result? {
            Outcome::Exited(Some(0)) => {}
            o => return Ok((done(o, kept), None)),
        }
        // 4. doc-tests, last, as cargo runs them. rustdoc builds them from the shared folder's library as it runs them, so they
        // need the build lock again, and the source note re-checked (another job may have rebuilt the folder from its own source).
        if arts.doctests {
            let (waited, lock) = {
                let asked = Instant::now();
                match buildsrc::lock(self.cfg, self.priority) {
                    Ok(l) => (asked.elapsed().as_secs_f64(), l),
                    Err((_, msg)) => return Err(msg),
                }
            };
            self.note(&format!("build lock: taken again for the doc-tests after waiting {waited:.1} s"));
            for l in buildsrc::prepare(self.cfg, self.project_dir, self.source)?.lines {
                self.note(&l);
            }
            let mut doc = cargo_command(self.cfg, self.project_dir, "test");
            doc.arg("--doc");
            doc.env("CARGO_TARGET_DIR", &self.cfg.build_dir);
            let r = self.child(doc);
            drop(lock);
            let r = r?;
            kept.push_str(&r.stdout);
            if !matches!(r.outcome, Outcome::Exited(Some(0))) {
                return Ok((done(r.outcome, kept), None));
            }
        }
        // 5. the Python tests: already run beside the copies, or now, after cargo and doc-tests passed
        let py = match side_py {
            Some(py) => py,
            None if self.cfg.python_parallel > 1 && scripts.len() >= 2 => self.run_python_pool(scripts)?,
            None => self.run_python(scripts)?,
        };
        Ok((done(Outcome::Exited(Some(0)), kept), Some(py)))
    }

    fn copy_all(&self, arts: &Artifacts, envs: &[(PathBuf, Vec<(String, String)>)], dir: &Path) -> Result<Vec<Copied>, String> {
        // each package's bins in their own folder: two packages may both have a bin with one name
        let mut bins: Vec<(PathBuf, String, PathBuf)> = Vec::new();
        for b in &arts.bins {
            let i = envs.iter().position(|(m, _)| same_path(m, &b.manifest)).unwrap_or(envs.len());
            let copy = copy_exe(&b.exe, &dir.join(format!("bins-{i}")))?;
            bins.push((b.manifest.clone(), b.name.clone(), copy));
        }
        let mut out = Vec::new();
        for t in &arts.tests {
            let copy = copy_exe(&t.exe, dir)?;
            let mut env = envs.iter().find(|(m, _)| same_path(m, &t.manifest)).map(|(_, e)| e.clone()).unwrap_or_default();
            if matches!(t.kind.as_str(), "test" | "bench") {
                // cargo sets CARGO_BIN_EXE_* for integration tests and benches only
                for (m, name, copy) in &bins {
                    if same_path(m, &t.manifest) {
                        env.push((format!("CARGO_BIN_EXE_{name}"), copy.display().to_string()));
                    }
                }
            }
            let cwd = t.manifest.parent().map(Path::to_path_buf).unwrap_or_else(|| self.project_dir.to_path_buf());
            out.push(Copied { art: t.clone(), copy, cwd, env });
        }
        Ok(out)
    }

    fn command_for(&self, c: &Copied) -> Command {
        let mut cmd = Command::new(&c.copy);
        cmd.current_dir(&c.cwd);
        scrub::apply(&mut cmd, self.cfg);
        // what cargo would give the test, and nothing an outer cargo left for ANOTHER package
        for (k, _) in std::env::vars_os() {
            let k = k.to_string_lossy();
            if ["CARGO_BIN_EXE_", "CARGO_PKG_", "CARGO_MANIFEST_", "CARGO_CRATE_", "CARGO_PRIMARY_PACKAGE", "CARGO_TARGET_TMPDIR"].iter().any(|p| k.starts_with(p)) {
                cmd.env_remove(k.as_ref());
            }
        }
        cmd.env("CARGO_TARGET_DIR", &self.cfg.build_dir);
        cmd.env("CARGO", std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.envs(c.env.iter().map(|(k, v)| (k, v)));
        cmd
    }

    /// The Python half of the test step (SCALING F9), run after cargo's tests passed and after the build lock is released (Python
    /// tests never need the shared build folder): each declared script, in order, as `python -I -B <script>` in the project
    /// folder, through `run_child` (so the disk poll, the log, redaction and the recorded child pid are those of every job
    /// child) with the scrubbed environment (`python_command`). The first failing script stops the rest.
    fn run_python(&mut self, scripts: &[String]) -> Result<PyTests, String> {
        if scripts.is_empty() {
            self.note("python tests: none (this project is not named in python_tests)");
            return Ok(PyTests::NoneDeclared);
        }
        let start = Instant::now();
        for s in scripts {
            let shown = format!("{PYTHON} -I -B {s}");
            if !self.project_dir.join(s).is_file() {
                self.note(&format!("error: python test `{shown}`: {s} is not a file in {}", self.project_dir.display()));
                return Ok(PyTests::Failed { script: s.clone(), why: "script not found".into() });
            }
            self.note(&format!("     Running {shown}"));
            let cmd = python_command(self.cfg, self.project_dir, s);
            let r = self.child(cmd)?;
            match r.outcome {
                Outcome::Exited(Some(0)) => {}
                Outcome::DiskDropped(f) => return Ok(PyTests::Killed(f)),
                Outcome::Exited(code) => {
                    self.note(&format!("error: python test failed: `{shown}` ({})", exit_text(code)));
                    return Ok(PyTests::Failed { script: s.clone(), why: exit_text(code) });
                }
            }
        }
        Ok(PyTests::Ok { count: scripts.len(), secs: start.elapsed().as_secs_f64() })
    }

    fn run_copies(&mut self, copied: &[Copied], kept: &mut String) -> Result<Outcome, String> {
        for c in copied {
            self.note(&format!("     Running {} ({})", c.art.src_path, c.copy.display()));
            let cmd = self.command_for(c);
            let r = self.child(cmd)?;
            kept.push_str(&r.stdout);
            match r.outcome {
                Outcome::Exited(Some(0)) => {}
                Outcome::DiskDropped(f) => return Ok(Outcome::DiskDropped(f)),
                Outcome::Exited(code) => {
                    self.note(&format!("error: test failed, to rerun pass `{}` ({}, {})", rerun_arg(&c.art), exit_text(code), c.art.src_path));
                    return Ok(Outcome::Exited(Some(101)));
                }
            }
        }
        Ok(Outcome::Exited(Some(0)))
    }

    /// The copies and then the Python tests, up to `parallel` at once, in one pool (`run_pool`). Children start in list order,
    /// so every copy has started before any script: a failing script can stop later scripts from starting, never a copy, and
    /// cargo's counts are always whole. Returns the copies' outcome (101 if any failed, as `run_copies`) and the scripts' result
    /// (only read when the copies passed).
    fn run_side_by_side(&mut self, copied: &[Copied], scripts: &[String], kept: &mut String) -> Result<(Outcome, PyTests), String> {
        let mut items: Vec<PoolItem> = Vec::new();
        for c in copied {
            let (rerun, src) = (rerun_arg(&c.art), c.art.src_path.clone());
            items.push(PoolItem {
                title: format!("     Running {} ({})", c.art.src_path, c.copy.display()),
                cmd: self.command_for(c),
                fail_line: Box::new(move |why| format!("error: test failed, to rerun pass `{rerun}` ({why}, {src})")),
                counts: true,
            });
        }
        // a declared script that is not a file fails the Python tests before any runs (the copies still run)
        let missing = scripts.iter().find(|s| !self.project_dir.join(s).is_file());
        let mut py = if scripts.is_empty() {
            self.note("python tests: none (this project is not named in python_tests)");
            PyTests::NoneDeclared
        } else if let Some(s) = missing {
            self.note(&format!("error: python test `{PYTHON} -I -B {s}`: {s} is not a file in {}", self.project_dir.display()));
            PyTests::Failed { script: s.clone(), why: "script not found".into() }
        } else {
            for s in scripts {
                let shown = format!("{PYTHON} -I -B {s}");
                items.push(PoolItem {
                    title: format!("     Running {shown}"),
                    cmd: python_command(self.cfg, self.project_dir, s),
                    fail_line: Box::new(move |why| format!("error: python test failed: `{shown}` ({why})")),
                    counts: false,
                });
            }
            PyTests::Ok { count: scripts.len(), secs: 0.0 }
        };
        let ran = match self.run_pool(items, kept, self.parallel, "test_parallel_binaries")? {
            PoolEnd::DiskDropped(f) => return Ok((Outcome::DiskDropped(f), PyTests::Killed(f))),
            PoolEnd::Ran(r) => r,
        };
        let (cargo, python) = ran.split_at(copied.len());
        if cargo.iter().any(|f| f.as_ref().is_none_or(|f| f.code != Some(0))) {
            return Ok((Outcome::Exited(Some(101)), py));
        }
        if let PyTests::Ok { secs, .. } = &mut py {
            // the first script that failed, in the declared order, names the failure; otherwise the scripts' wall time
            if let Some((s, f)) = scripts.iter().zip(python).find(|(_, f)| f.as_ref().is_some_and(|f| f.code != Some(0))) {
                let code = f.as_ref().and_then(|f| f.code);
                return Ok((Outcome::Exited(Some(0)), PyTests::Failed { script: s.clone(), why: exit_text(code) }));
            }
            let from = python.iter().flatten().map(|f| f.from).fold(f64::INFINITY, f64::min);
            let to = python.iter().flatten().map(|f| f.to).fold(0.0, f64::max);
            *secs = if from.is_finite() { (to - from).max(0.0) } else { 0.0 };
        }
        Ok((Outcome::Exited(Some(0)), py))
    }

    /// Run `items` up to `width` at once, in list order. `setting` names the key that chose the width in the pool's note.
    /// Each child's stdout and stderr are collected apart from the others,
    /// redacted, and printed whole (and written to the log) in LIST order, each as soon as every item before it has been printed,
    /// so the output reads like a serial run. `counts` items add their stdout to `kept`. After a failure nothing new starts; the
    /// running ones finish and are printed. The disk floor is polled every `poll` and a drop kills every running child's tree;
    /// the record names every running child (`on_live`). A start or wait error kills every running child.
    /// A child that has exited never makes the pool wait on its readers: it moves to `draining`, checked without blocking each
    /// round, and frees its slot at once. When its readers end (its pipes closed) its output is complete; if a process it started
    /// still holds a pipe after `test_output_wait_seconds`, its output is taken as far as it got and marked TRUNCATED (the
    /// reader threads are left to end on their own). The disk poll and the other children go on meanwhile.
    pub(super) fn run_pool(
        &mut self,
        items: Vec<PoolItem>,
        kept: &mut String,
        width: usize,
        setting: &str,
    ) -> Result<PoolEnd, String> {
        /// A child's collected stdout or stderr. `None` once closed (its output was taken at a cut, or the pool gave up on it):
        /// the reader thread then reads and DISCARDS until EOF, so a process that keeps writing never grows memory.
        type Buf = Arc<Mutex<Option<Vec<u8>>>>;
        /// Close `b` and return what it held.
        fn close(b: &Buf) -> Vec<u8> {
            b.lock().map(|mut v| v.take().unwrap_or_default()).unwrap_or_default()
        }
        struct Live {
            idx: usize,
            child: std::process::Child,
            out: Buf,
            err: Buf,
            readers: Vec<std::thread::JoinHandle<()>>,
            from: f64,
        }
        /// An exited child whose output may still be arriving.
        struct Draining {
            idx: usize,
            out: Buf,
            err: Buf,
            readers: Vec<std::thread::JoinHandle<()>>,
            exited: Instant,
        }
        fn reader(mut src: impl Read + Send + 'static, into: Buf) -> std::thread::JoinHandle<()> {
            std::thread::spawn(move || {
                let mut chunk = [0u8; 8192];
                while let Ok(k) = src.read(&mut chunk) {
                    if k == 0 {
                        break;
                    }
                    if let Ok(mut b) = into.lock() {
                        if let Some(b) = b.as_mut() {
                            b.extend_from_slice(&chunk[..k]);
                        } // closed: discarded
                    }
                }
            })
        }
        /// Kill every running child and close every buffer (running and draining), so readers left to end on their own (never
        /// block on orphaned pipes) keep nothing.
        fn kill_all(live: &mut Vec<Live>, draining: &mut Vec<Draining>) {
            for l in live.iter_mut() {
                kill_tree(&mut l.child);
                close(&l.out);
                close(&l.err);
            }
            for d in draining.iter() {
                close(&d.out);
                close(&d.err);
            }
            live.clear();
            draining.clear();
        }
        let n = items.len();
        let width = width.max(1);
        self.note(&format!("broadsword: running {n} test child(ren), up to {width} at once ({setting})"));
        let start = Instant::now();
        let mut items: Vec<Option<PoolItem>> = items.into_iter().map(Some).collect();
        let mut lines: Vec<(String, FailLine, bool)> = Vec::with_capacity(n);
        let mut live: Vec<Live> = Vec::new();
        let mut draining: Vec<Draining> = Vec::new();
        let wait = Duration::from_secs(self.cfg.test_output_wait_seconds);
        let mut finished: Vec<Option<Finished>> = (0..n).map(|_| None).collect();
        let mut outputs: Vec<Option<(String, String)>> = (0..n).map(|_| None).collect();
        let (mut next, mut printed, mut failed) = (0usize, 0usize, false);
        let mut next_poll = Instant::now() + self.poll;
        loop {
            // start what the width allows; after a failure nothing new starts
            let mut changed = false;
            while !failed && next < n && live.len() < width {
                let PoolItem { title, mut cmd, fail_line, counts } = items[next].take().expect("each item starts once");
                let spawned = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
                let mut child = match spawned {
                    Ok(c) => c,
                    Err(e) => {
                        kill_all(&mut live, &mut draining);
                        (self.on_live)(&[]);
                        return Err(format!("cannot start `{}`: {e}", cmd.get_program().to_string_lossy()));
                    }
                };
                let (out, err): (Buf, Buf) = (Arc::new(Mutex::new(Some(Vec::new()))), Arc::new(Mutex::new(Some(Vec::new()))));
                let readers = vec![reader(child.stdout.take().expect("piped"), Arc::clone(&out)), reader(child.stderr.take().expect("piped"), Arc::clone(&err))];
                live.push(Live { idx: next, child, out, err, readers, from: start.elapsed().as_secs_f64() });
                lines.push((title, fail_line, counts));
                next += 1;
                changed = true;
            }
            // reap what has ended
            let mut i = 0;
            while i < live.len() {
                let status = match live[i].child.try_wait() {
                    Ok(s) => s,
                    Err(e) => {
                        kill_all(&mut live, &mut draining);
                        (self.on_live)(&[]);
                        return Err(format!("waiting for a test child failed: {e}"));
                    }
                };
                let Some(status) = status else {
                    i += 1;
                    continue;
                };
                let l = live.swap_remove(i);
                let code = status.code();
                failed |= code != Some(0);
                finished[l.idx] = Some(Finished { code, from: l.from, to: start.elapsed().as_secs_f64() });
                draining.push(Draining { idx: l.idx, out: l.out, err: l.err, readers: l.readers, exited: Instant::now() });
                changed = true;
            }
            // collect the output of exited children, never blocking: complete once its readers ended, else cut at the wait
            let mut i = 0;
            while i < draining.len() {
                let done = draining[i].readers.iter().all(|r| r.is_finished());
                if !done && draining[i].exited.elapsed() < wait {
                    i += 1;
                    continue;
                }
                let d = draining.swap_remove(i);
                if done {
                    for r in d.readers {
                        let _ = r.join(); // already finished: returns at once
                    }
                }
                // closing takes the bytes so far; after a cut the readers keep reading but discard (memory stays bounded)
                let text = |b: &Buf| self.secrets.redact(&String::from_utf8_lossy(&close(b)));
                let (out, mut err) = (text(&d.out), text(&d.err));
                if !done {
                    if !err.is_empty() && !err.ends_with('\n') {
                        err.push('\n');
                    }
                    err.push_str(&format!(
                        "broadsword: TRUNCATED: this test child exited, but a process it started still held its output open {} s later (test_output_wait_seconds); its output above is as far as it got, and the rest is read and discarded\n",
                        wait.as_secs()
                    ));
                }
                outputs[d.idx] = Some((out, err));
            }
            if changed {
                let pids: Vec<u32> = live.iter().map(|l| l.child.id()).collect();
                (self.on_live)(&pids);
            }
            // print, in list order, everything whose predecessors are printed
            while printed < n {
                let Some((out, err)) = outputs[printed].take() else { break };
                self.print_one(&lines[printed], finished[printed].as_ref(), &out, &err, kept);
                printed += 1;
            }
            if live.is_empty() && draining.is_empty() && (failed || next >= n) {
                break;
            }
            if Instant::now() >= next_poll {
                let f = (self.free)();
                if pause::below_floor(f, self.cfg.floor_bytes()) {
                    kill_all(&mut live, &mut draining);
                    (self.on_live)(&[]);
                    return Ok(PoolEnd::DiskDropped(f));
                }
                next_poll = Instant::now() + self.poll;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // after a failure: what finished behind a child that never started is printed now, in order
        for idx in printed..n {
            if let Some((out, err)) = outputs[idx].take() {
                self.print_one(&lines[idx], finished[idx].as_ref(), &out, &err, kept);
            }
        }
        if next < n {
            self.note(&format!("broadsword: {} test child(ren) not started after a failure", n - next));
        }
        Ok(PoolEnd::Ran(finished))
    }

    /// Print one finished pool child: its `Running` line with its time, its stdout and stderr whole, and its failure line.
    fn print_one(&self, line: &(String, FailLine, bool), f: Option<&Finished>, out: &str, err: &str, kept: &mut String) {
        let (title, fail_line, counts) = line;
        let secs = f.map(|f| f.to - f.from).unwrap_or(0.0);
        self.note(&format!("{title} [{secs:.1} s]"));
        print!("{out}");
        let _ = std::io::stdout().flush();
        eprint!("{err}");
        if let Ok(mut l) = self.log.lock() {
            let _ = l.write_all(out.as_bytes());
            let _ = l.write_all(err.as_bytes());
        }
        if *counts {
            kept.push_str(out);
        }
        if let Some(code) = f.map(|f| f.code).filter(|c| *c != Some(0)) {
            self.note(&fail_line(&exit_text(code)));
        }
    }
}
