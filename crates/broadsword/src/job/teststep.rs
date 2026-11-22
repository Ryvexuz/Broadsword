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

/// One child the side-by-side pool runs (a copied test executable or a Python test script).
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

