//! Evidence for a post-merge revert; this is independent of the pre-merge train check.

use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec};
use crate::{tools, wtgit};

use super::check::main_check_info;
use super::merge::trunk_checkout;
use super::postcheck::main_record_path;
use super::{Run, R};

const RERUN_LINE: &str = "error: test failed, to rerun pass ";

fn sha(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

fn rerun_line(line: &str) -> Result<(&str, &str), String> {
    let (arg, detail) = line
        .strip_prefix(RERUN_LINE)
        .and_then(|s| s.strip_prefix('`'))
        .and_then(|s| s.split_once("` ("))
        .and_then(|(arg, detail)| Some((arg, detail.strip_suffix(')')?)))
        .ok_or("malformed test re-run line")?;
    let valid = arg == "--lib"
        || matches!(arg.split_once(' '), Some(("--bin" | "--test", name))
            if !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')));
    if !valid || detail.chars().any(char::is_control) {
        return Err("malformed test re-run argument or detail".into());
    }
    Ok((arg, detail))
}

fn complete_log(log: &str) -> Result<(), String> {
    if log.lines().any(|l| l.starts_with("broadsword: TRUNCATED:")) {
        return Err("TRUNCATED job log".into());
    }
    if log.trim().is_empty() {
        return Err("empty job log".into());
    }
    Ok(())
}

fn compile_error(line: &str) -> bool {
    line.starts_with("error: could not compile") || line.starts_with("error[E")
}

fn failing_args(log: &str) -> Result<Vec<&str>, String> {
    complete_log(log)?;
    let mut args = Vec::new();
    let mut seen = BTreeSet::new();
    for line in log.lines() {
        if line.starts_with("error: python test") {
            return Err("Python test failure cannot be re-run".into());
        }
        if compile_error(line) {
            return Err("compile error in the main check".into());
        }
        if line.starts_with(RERUN_LINE.trim_end()) {
            let (arg, detail) = rerun_line(line)?;
            if !detail.starts_with("exit 101, ") {
                return Err("test executable did not report a normal libtest failure".into());
            }
            if !seen.insert(arg) {
                return Err("ambiguous duplicate test re-run selector".into());
            }
            args.push(arg);
        }
    }
    if args.is_empty() {
        return Err("no failing Rust test selector in the job log".into());
    }
    Ok(args)
}

fn result_line(line: &str) -> Result<Option<u64>, String> {
    let Some(line) = line.strip_prefix("test result:") else {
        return Ok(None);
    };
    let parse = || {
        let (ok, counts) = if let Some(counts) = line.strip_prefix(" ok. ") {
            (true, counts)
        } else {
            (false, line.strip_prefix(" FAILED. ")?)
        };
        let (passed, counts) = counts.split_once(" passed; ")?;
        let (failed, _) = counts.split_once(" failed;")?;
        let unsigned = |s: &str| {
            if s.bytes().all(|b| b.is_ascii_digit()) {
                s.parse::<u64>().ok()
            } else {
                None
            }
        };
        unsigned(passed)?;
        let failed = unsigned(failed)?;
        (ok == (failed == 0) && !line.chars().any(char::is_control)).then_some(failed)
    };
    parse().map(Some).ok_or("malformed libtest result".into())
}

#[derive(Debug, PartialEq, Eq)]
enum Rerun {
    Pass,
    Fail,
    NoVerdict(String),
}

fn classify_rerun(code: u8, summary: &[String], log: &str, n_args: usize) -> Rerun {
    let classify = || -> Result<Rerun, String> {
        complete_log(log)?;
        let first = summary.first().ok_or("re-run job has no summary")?;
        let mut count = 0;
        let mut failed = false;
        for line in log.lines() {
            if compile_error(line) {
                return Err("compile error in the re-run".into());
            }
            if let Some(n) = result_line(line)? {
                count += 1;
                failed |= n != 0;
            }
        }
        if n_args == 0 || count != n_args {
            return Err(format!("re-run reported {count} test results for {n_args} selectors"));
        }
        match code {
            0 if first.starts_with("run: ok") && !failed => Ok(Rerun::Pass),
            101 if first.starts_with("run: FAILED (exit 101)") && failed => Ok(Rerun::Fail),
            _ => Err(format!("re-run has no test verdict (exit {code}, {first})")),
        }
    };
    classify().unwrap_or_else(Rerun::NoVerdict)
}

fn open_existing(path: &Path, opts: &mut OpenOptions) -> std::io::Result<File> {
    let expected = fs::symlink_metadata(path)?;
    if !expected.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        opts.custom_flags(0x0020_0000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    let file = opts.open(path)?;
    let opened = file.metadata()?;
    #[cfg(windows)]
    let same = {
        use std::os::windows::fs::MetadataExt;
        opened.file_attributes() & 0x400 == 0 // FILE_ATTRIBUTE_REPARSE_POINT
    };
    #[cfg(unix)]
    let same = {
        use std::os::unix::fs::MetadataExt;
        (expected.dev(), expected.ino()) == (opened.dev(), opened.ino()) && opened.nlink() == 1
    };
    if !opened.is_file() || !same {
        return Err(std::io::Error::other("file changed before open"));
    }
    Ok(file)
}

fn read_text(path: &Path, limit: u64) -> Result<String, String> {
    let read = || -> std::io::Result<String> {
        let file = open_existing(path, OpenOptions::new().read(true))?;
        if file.metadata()?.len() > limit {
            return Err(std::io::Error::other("over the size limit"));
        }
        let mut text = String::new();
        file.take(limit.saturating_add(1)).read_to_string(&mut text)?;
        if text.len() as u64 > limit {
            return Err(std::io::Error::other("over the size limit"));
        }
        Ok(text)
    };
    read().map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn job_log(cfg: &Config, info: &JobInfo) -> Result<String, String> {
    if info.id.as_deref().is_none_or(str::is_empty) {
        return Err("job did not start (no job id)".into());
    }
    let path = info.log.as_deref().ok_or("job has no log")?;
    // Cleanup can replace a job log by its last log_max_mb bytes without a prefix.
    let limit = cfg.log_max_mb.saturating_mul(1024 * 1024);
    read_text(path, limit.saturating_sub(1))
}

fn failure_evidence<'a>(info: &JobInfo, log: &'a str) -> Result<Vec<&'a str>, String> {
    complete_log(log)?;
    if !matches!(info.summary.as_slice(), [build, test]
        if build.starts_with("build: ok") && test.starts_with("test: FAILED (exit 101)"))
    {
        return Err("main check did not report a Rust test step failure".into());
    }
    let args = failing_args(log)?;
    let failed = log.lines().try_fold(0, |count, line| {
        result_line(line).map(|n| count + if n.is_some_and(|n| n > 0) { 1 } else { 0 })
    })?;
    if failed != args.len() {
        return Err("main check did not report a failed test result for every selector".into());
    }
    Ok(args)
}

fn excused_by_rerun<'a>(log: &'a str, args: &[&str]) -> Result<BTreeSet<&'a str>, String> {
    const RUNNING: &str = "     Running ";
    complete_log(log)?;
    let selectors: BTreeSet<_> = args.iter().copied().collect();
    let mut lines = log.lines().peekable();
    let mut excused = BTreeSet::new();
    let boundary = |l: &&str| l.starts_with(RUNNING) || l.starts_with(RERUN_LINE);
    let bad_char = |c: char| c.is_whitespace() || c.is_control() || "#/\\".contains(c);
    while let Some(title) = lines.find_map(|line| line.strip_prefix(RUNNING)) {
        let mut names = BTreeSet::new();
        let mut valid = true;
        while let Some(line) = lines.next_if(|l| !boundary(l)) {
            let test = line.strip_prefix("test ");
            if let Some(name) = test.and_then(|s| s.strip_suffix(" ... FAILED")) {
                valid &= !name.is_empty() && !name.contains(bad_char) && names.insert(name);
            }
        }
        let Some(line) = lines.next_if(|l| l.starts_with(RERUN_LINE)) else {
            continue;
        };
        let (arg, detail) = rerun_line(line)?;
        let src = detail.strip_prefix("exit 101, ").ok_or("invalid rerun source")?;
        valid &= !src.is_empty() && title.starts_with(&format!("{src} ("));
        valid &= selectors.contains(arg) && !names.is_empty();
        // Consume the rerun block without letting its output count as first-run failures.
        while let Some(line) = lines.next_if(|l| !boundary(l)) {
            if let Some(pass) = line.strip_prefix("flaky rerun: passed") {
                let (target, name) = pass
                    .strip_suffix(" (first run failed)")
                    .and_then(|s| s.strip_prefix(' ')?.rsplit_once(' '))
                    .ok_or("malformed flaky rerun pass line")?;
                valid &= target == arg && names.remove(name);
            } else if line.starts_with("flaky rerun:") {
                valid = false;
            }
        }
        if valid && names.is_empty() {
            excused.insert(arg);
        }
    }
    Ok(excused)
}

fn bisect_selectors<'a>(
    log: &'a str,
    mut args: Vec<&'a str>,
    note: impl FnOnce(BTreeSet<&'a str>) -> Result<(), String>,
) -> Result<Vec<&'a str>, String> {
    match excused_by_rerun(log, &args) {
        Ok(excused) if !excused.is_empty() => {
            args.retain(|arg| !excused.contains(arg));
            note(excused)?;
        }
        Err(why) => eprintln!("merge train bisect: keeping all selectors: {why}"),
        _ => {}
    }
    let why = "every failing test binary passed its flaky rerun; no selector to bisect";
    let remaining = (!args.is_empty()).then_some(args);
    remaining.ok_or_else(|| why.into())
}

pub(super) struct Boundary {
    entry: String,
    task: String,
    up: String,
    tip: String,
}

impl Boundary {
    pub(super) fn of(run: &Run<'_>) -> Self {
        let up = run.e.get("train_up");
        Self {
            entry: run.e.id().into(),
            task: run.e.get("task").into(),
            // A standalone entry was never chained; its reviewed base is its boundary.
            up: if up.is_empty() && run.e.get("train").is_empty() {
                run.e.get("base_head")
            } else {
                up
            }
            .into(),
            tip: run.e.get("main_sha").into(),
        }
    }
}

#[derive(Default)]
pub(super) struct Bisect {
    members: Vec<Boundary>,
    kept: Option<(usize, String)>,
}

impl Bisect {
    pub(super) fn new(members: impl IntoIterator<Item = Boundary>) -> Self {
        Self {
            members: members.into_iter().collect(),
            kept: None,
        }
    }

    fn validate(&self, run: &Run<'_>, checked_sha: &str) -> Result<(), String> {
        let mut up = run.e.get("pre_merge_sha");
        let mut entries = BTreeSet::new();
        let mut tasks = BTreeSet::new();
        for m in &self.members {
            if m.entry.is_empty()
                || m.task.is_empty()
                || !entries.insert(&m.entry)
                || !tasks.insert(&m.task)
                || !sha(up)
                || !sha(&m.tip)
                || m.up != up
                || m.tip == up
                || !wtgit::is_ancestor(&run.dir, up, &m.tip)?
            {
                return Err(format!("invalid or non-contiguous boundary for entry {}", m.entry));
            }
            up = &m.tip;
        }
        if self.members.last().is_none_or(|m| {
            m.entry != run.e.id() || m.tip != run.e.get("main_sha") || m.tip != checked_sha
        }) {
            return Err("no boundary list ending at the checked merge tip".into());
        }
        main_at(run, checked_sha)
    }

    /// `None`: flake, `Some(start)`: confirmed failure, `Err`: use the whole range.
    pub(super) fn choose(
        &mut self,
        run: &mut Run<'_>,
        checked_sha: &str,
        info: &JobInfo,
    ) -> Result<Option<String>, String> {
        self.kept = None;
        run.e.set("revert_from", "");
        self.validate(run, checked_sha)?;
        let log = job_log(run.cfg, info)?;
        let args = failure_evidence(info, &log)?;
        if args.iter().any(|arg| run.secrets.redact(arg) != *arg) {
            return Err("a test selector contains a redacted value".into());
        }
        let args = if run.cfg.flaky_tests_file.is_empty() {
            args
        } else {
            bisect_selectors(&log, args, |excused| {
                let n = excused.len();
                let kind = if n == 1 { "binary" } else { "binaries" };
                let job = info.id.as_deref().unwrap_or("none");
                let excused = excused.into_iter().collect::<Vec<_>>().join(", ");
                let note = run.secrets.redact(&format!(
                    "merge train bisect: {n} test {kind} excused by a passing flaky rerun in job {job}: {excused}"
                ));
                println!("{note}");
                run.log("main_check", &note)
                    .map_err(|_| "could not record the flaky rerun excusal in the entry log".into())
            })?
        };
        let n = self.members.len();
        let tip = &self.members[n - 1];
        let start = Instant::now();
        let (res, job) = rerun(run, tip, &args);
        match res {
            Rerun::Pass => {
                let full = full_check_at(run, &tip.tip, &mut JobInfo::default()).map_err(|e| {
                    format!("tip re-run job {job} passed, but the full check did not: {e}")
                })?;
                if let Err(e) = record_green(run.cfg, run.e.get("project"), &tip.tip) {
                    eprintln!("broadsword: {e} (passing full main check job {full})");
                }
                let args = args.join(", ");
                run.e.set(
                    "flake_rerun",
                    format!("pass: {args} re-run job {job}, full main check job {full}"),
                );
                println!(
                    "queue entry {}: main check FAILED then passed on re-run (flake): {args}; nothing reverted",
                    run.e.id()
                );
                return Ok(None);
            }
            Rerun::NoVerdict(why) => return Err(why),
            Rerun::Fail => {}
        }
        let mut jobs = 1;
        let mut passing_job = String::new();
        let first = walk_back(n - 1, |i| {
            let (res, job) = rerun(run, &self.members[i], &args);
            jobs += 1;
            if res == Rerun::Pass {
                passing_job = job;
            }
            res
        })?;
        if first > 0 {
            self.kept = Some((first, passing_job));
        }
        let m = &self.members[first];
        run.e.set("revert_from", &m.entry);
        run.e.set(
            "main_bisect",
            format!(
                "first failing member {} (position {} of {n}); members 1..{first} stay merged; {jobs} re-run job(s) in {} s",
                m.entry,
                first + 1,
                start.elapsed().as_secs()
            ),
        );
        Ok(Some(m.up.clone()))
    }

    pub(super) fn record_revert(
        &mut self,
        run: &mut Run<'_>,
        plan: &Result<Option<String>, String>,
        reverted: bool,
    ) {
        if !reverted {
            self.kept = None;
        }
        if let Err(why) = plan {
            let result = if reverted {
                "the whole merge was reverted"
            } else {
                "the whole-merge revert did not complete"
            };
            let detail = run.secrets.redact(&format!("no verdict: {why}; {result}"));
            run.e.set("main_bisect", detail);
        } else if !reverted {
            run.e.set(
                "main_bisect",
                format!(
                    "revert did not complete; proposed verdict: {}",
                    run.e.get("main_bisect")
                ),
            );
        }
    }

    pub(super) fn kept_log(&self, index: usize, train: &str) -> Option<String> {
        let (count, job) = self.kept.as_ref()?;
        (index < *count).then(|| {
            let where_passed = if index + 1 == *count {
                "at this entry's commit"
            } else {
                "at a later boundary that contains this entry's commit"
            };
            format!("kept on main: the bisect of the main check of merge train {train} passed {where_passed} (job {job})")
        })
    }
}

fn walk_back(earlier: usize, mut probe: impl FnMut(usize) -> Rerun) -> Result<usize, String> {
    for i in (0..earlier).rev() {
        match probe(i) {
            Rerun::Pass => return Ok(i + 1),
            Rerun::Fail => {}
            Rerun::NoVerdict(why) => return Err(why),
        }
    }
    Ok(0)
}

fn clean_at(dir: &Path, commit: &str) -> Result<(), String> {
    let head = wtgit::head_commit(dir)?;
    if head != commit || !wtgit::status_porcelain_plain(dir)?.is_empty() {
        return Err(format!(
            "checkout {} is not clean at {commit} (HEAD {head})",
            dir.display()
        ));
    }
    Ok(())
}

fn main_at(run: &Run<'_>, commit: &str) -> Result<(), String> {
    trunk_checkout(&run.dir, run.e.get("main_branch"))?;
    clean_at(&run.dir, commit)
}

fn rerun(run: &Run<'_>, member: &Boundary, args: &[&str]) -> (Rerun, String) {
    let mut info = JobInfo::default();
    let mut run_job = || -> Result<Rerun, String> {
        let (cfg, project, task) = (run.cfg, run.e.get("project"), member.task.as_str());
        if let Some(code) = tools::guard_refusal(cfg, project, Some(task)) {
            return Err(format!("re-run was refused before it started (exit {code})"));
        }
        let folder = || job::job_folder(cfg, project, Some(task)).map_err(|(_, why)| why);
        let dir = folder()?;
        let identity = fs::canonicalize(&dir).map_err(|e| format!("resolve task worktree: {e}"))?;
        let main = fs::canonicalize(&run.dir).map_err(|e| format!("resolve main checkout: {e}"))?;
        if identity == main {
            return Err("task worktree resolves to the main checkout".into());
        }
        clean_at(&dir, &member.tip)?;
        let spec = JobSpec {
            kind: "run".into(),
            name: project.into(),
            command: Some(format!(
                "cargo build --offline && cargo test --offline --no-fail-fast {}",
                args.join(" ")
            )),
            first_step: 0,
            resumed_from: None,
            resume_count: 0,
            task: Some(member.task.clone()),
        };
        let code = job::run_job_info(cfg, spec, &mut info);
        let after = folder()?;
        if fs::canonicalize(&after).ok().as_ref() != Some(&identity) {
            return Err("task worktree changed location during the re-run".into());
        }
        clean_at(&after, &member.tip)?;
        let log = job_log(cfg, &info)?;
        Ok(classify_rerun(code, &info.summary, &log, args.len()))
    };
    let result = run_job().unwrap_or_else(Rerun::NoVerdict);
    let job = info.id.unwrap_or_else(|| "none".into());
    let result = match result {
        Rerun::NoVerdict(why) => {
            Rerun::NoVerdict(format!("entry {} re-run job {job}: {why}", member.entry))
        }
        verdict => verdict,
    };
    (result, job)
}

fn verified_full_pass(run: &Run<'_>, commit: &str, info: &JobInfo) -> Result<(), String> {
    if !sha(commit) {
        return Err("no exact clean trunk commit identified when the full check started".into());
    }
    if !matches!(info.summary.as_slice(), [build, test]
        if build.starts_with("build: ok") && test.starts_with("test: ok"))
    {
        return Err("full main check job has no passing build and test summary".into());
    }
    let log = job_log(run.cfg, info)?;
    complete_log(&log)?;
    if log.lines().any(compile_error) {
        return Err("compile error in the full check job log".into());
    }
    main_at(run, commit)
}

fn full_check_at(run: &Run<'_>, commit: &str, info: &mut JobInfo) -> Result<String, String> {
    main_at(run, commit)?;
    let result = main_check_info(run.cfg, &run.dir, run.e.get("project"), info);
    let job = result?;
    verified_full_pass(run, commit, info).map_err(|e| format!("job {job}: {e}"))?;
    Ok(job)
}

pub(super) fn record_planned_green(run: &Run<'_>, commit: &str, info: &JobInfo) -> Result<(), String> {
    let project = run.e.get("project");
    match verified_full_pass(run, commit, info) {
        Ok(()) => record_green(run.cfg, project, commit),
        Err(error) => {
            let message = format!("no green record for {project}: {error}");
            eprintln!("broadsword: {}", run.secrets.redact(&message));
            Ok(())
        }
    }
}

fn record_green(cfg: &Config, project: &str, commit: &str) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    if !sha(commit) {
        return Err("invalid commit SHA for green main check record".into());
    }
    let path = main_record_path(cfg, project, "main-green")?;
    let temp = path.with_file_name(format!(
        "main-green-{project}.tmp-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let write = || -> std::io::Result<()> {
        fs::create_dir_all(super::entry::queue_dir(cfg))?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let result = (|| {
            writeln!(file, "{commit}")?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, &path)
        })();
        if let Err(e) = result {
            return match fs::remove_file(&temp) {
                Ok(()) => Err(e),
                Err(cleanup) => Err(std::io::Error::other(format!(
                    "{e}; remove temp record: {cleanup}"
                ))),
            };
        }
        Ok(())
    };
    write().map_err(|e| format!("write green main check record {}: {e}", path.display()))
}

type Failures<'a> = BTreeSet<(&'a str, &'a str)>;

fn failing_names<'a>(info: &JobInfo, log: &'a str) -> Result<Failures<'a>, String> {
    failure_evidence(info, log)?;
    let mut all = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut failed = false;
    for line in log.lines() {
        if line.starts_with("     Running ") && (failed || !names.is_empty()) {
            return Err("unpaired named test failures".into());
        } else if let Some(name) = line
            .strip_prefix("test ")
            .and_then(|s| s.strip_suffix(" ... FAILED"))
        {
            if failed
                || name.is_empty()
                || name
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '#' | '/' | '\\'))
                || !names.insert(name)
            {
                return Err("malformed or duplicate failed test name".into());
            }
        } else if let Some(n) = result_line(line)? {
            if failed || n != names.len() as u64 {
                return Err("named failures do not match the test result".into());
            }
            failed = n > 0;
        } else if line.starts_with(RERUN_LINE) {
            let (arg, _) = rerun_line(line)?;
            if !std::mem::take(&mut failed) {
                return Err("test selector has no matching failed result".into());
            }
            all.extend(std::mem::take(&mut names).into_iter().map(|name| (arg, name)));
        }
    }
    if failed || !names.is_empty() || all.is_empty() {
        return Err("incomplete evidence of named test failures".into());
    }
    Ok(all)
}

/// One line in the allow-list format of `flake::parse_list` (that module is test-only in the binary, so the
/// shape is checked here and the unit test below proves `parse_list` accepts it).
fn candidate_line(project: &str, arg: &str, name: &str, id: &str, utc: &str) -> Result<String, String> {
    let line = format!("{project} {arg} {name} # candidate from {id} {utc}\n");
    let words = if arg == "--lib" { 3 } else { 4 };
    let body = line.split_once(" # ").map(|(body, _)| body).unwrap_or("");
    if body.split_whitespace().count() != words || line.trim_end().chars().any(char::is_control) {
        return Err("malformed flake candidate line".into());
    }
    Ok(line)
}

fn append_candidates(run: &Run<'_>, failures: &Failures<'_>) -> Result<(), String> {
    let project = run.e.get("project");
    let path = main_record_path(run.cfg, project, "flake-candidates")?;
    let id = run.e.id();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return Err("invalid queue entry id for flake candidate".into());
    }
    let utc = crate::pause::format_utc(crate::task::now_secs());
    let mut text = String::new();
    for (arg, name) in failures {
        let line = candidate_line(project, arg, name, id, &utc)?;
        if run.secrets.redact(&line) != line {
            return Err("a flake candidate contains a redacted value".into());
        }
        text.push_str(&line);
    }
    let append = || -> std::io::Result<()> {
        let mut opts = OpenOptions::new();
        opts.append(true);
        let mut file = match opts.clone().create_new(true).open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                open_existing(&path, &mut opts)?
            }
            Err(e) => return Err(e),
        };
        file.write_all(text.as_bytes())?;
        file.sync_all()
    };
    append().map_err(|e| format!("append flake candidates {}: {e}", path.display()))
}

fn green_release(
    run: &Run<'_>,
    commit: &str,
    triggering: &JobInfo,
    post_revert: &JobInfo,
) -> Result<(String, String), String> {
    let path = main_record_path(run.cfg, run.e.get("project"), "main-green")?;
    // A full SHA-256 plus its line ending; no extra lines or whitespace.
    let text = read_text(&path, 64 + 2)?;
    let green = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(&text);
    if !sha(green) || !wtgit::is_ancestor(&run.dir, green, green)? {
        return Err("malformed green main check record".into());
    }
    main_at(run, commit)?;
    if wtgit::tree_of(&run.dir, green)? != wtgit::tree_of(&run.dir, commit)? {
        return Err("the revert tree differs from the last green tree".into());
    }
    let original = job_log(run.cfg, triggering)?;
    let after = job_log(run.cfg, post_revert)?;
    let original = failing_names(triggering, &original)?;
    let after = failing_names(post_revert, &after)?;
    if after.is_empty() || !after.is_subset(&original) {
        return Err("the revert check has a new or unknown failing test".into());
    }
    append_candidates(run, &after)?;
    let names = after.iter().map(|(_, name)| *name).collect::<BTreeSet<_>>();
    Ok((
        green.into(),
        names.into_iter().collect::<Vec<_>>().join(", "),
    ))
}

/// A failed check keeps its first failure visible even if retry or green-tree evidence is unusable.
pub(super) fn check_revert(
    run: &mut Run<'_>,
    commit: &str,
    triggering: Option<&JobInfo>,
) -> Result<String, String> {
    let mut first = JobInfo::default();
    let failure = match full_check_at(run, commit, &mut first) {
        Ok(job) => return Ok(mark_revert_pass(run, commit, &job)),
        Err(why) => why,
    };
    let no_release = match triggering
        .ok_or_else(|| "the triggering check was not of the merged tip".to_string())
        .and_then(|info| green_release(run, commit, info, &first))
    {
        Ok((green, names)) => {
            run.e.set(
                "revert_check",
                format!("passed (green tree {})", super::view::short(&green)),
            );
            let text = format!("failed on a tree that passed a full check ({green}); flake {names}; lane NOT held");
            run.e.set("flake", names);
            return Ok(text);
        }
        Err(why) => why,
    };
    let first_job = first.id.as_deref().unwrap_or("none");
    match full_check_at(run, commit, &mut JobInfo::default()) {
        Ok(job) => {
            run.e.set(
                "revert_flake",
                format!("pass: first job {first_job}, re-run job {job}"),
            );
            let pass = mark_revert_pass(run, commit, &job);
            Ok(format!(
                "{pass} on re-run (first check: {failure}; no green-tree release: {no_release})"
            ))
        }
        Err(why) => Err(format!(
            "{failure}; no green-tree release: {no_release}; re-run did not pass: {why}"
        )),
    }
}

fn mark_revert_pass(run: &mut Run<'_>, commit: &str, job: &str) -> String {
    run.e.set("revert_check", "passed");
    if let Err(e) = record_green(run.cfg, run.e.get("project"), commit) {
        eprintln!("broadsword: {e} (passing post-revert check job {job})");
    }
    format!("passed (job {job})")
}

pub(super) fn recorded_green_release(check: &str, names: &str) -> bool {
    !names.is_empty()
        && check
            .strip_prefix("passed (green tree ")
            .and_then(|s| s.strip_suffix(')'))
            .is_some_and(|s| s.len() == 10 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub(super) fn kept_done(run: &mut Run<'_>, note: &str) -> R<std::process::ExitCode> {
    run.log("main_check", note)?;
    run.merge_phase(super::STEPS.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The libtest output and teststep's Running and re-run lines, in log order.
    const OK: &str =
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
    const FAILED: &str =
        "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n";
    const SELECTOR: &str = "error: test failed, to rerun pass `--test full` (exit 101, tests/full.rs)\n";
    const TRUNCATED: &str = "broadsword: TRUNCATED: this command exited, but a process it started still held its output open 2 s later (test_output_wait_seconds); its output above is as far as it got\n";

    #[test]
    fn failing_args_reads_only_rerun_lines_and_refuses_python_or_malformed_ones() {
        let log = format!(
            "     Running tests/full.rs (build/full)\n\
             test module::fails ... FAILED\n{FAILED}{SELECTOR}"
        );
        assert_eq!(
            failing_args(&format!(
                "test a ... FAILED\n{SELECTOR}error: test failed, to rerun pass `--lib` (exit 101, src/lib.rs)\r\n"
            ))
            .unwrap(),
            ["--test full", "--lib"]
        );
        let info = JobInfo {
            summary: vec![
                "build: ok (0.1 s)".into(),
                "test: FAILED (exit 101) — 0 passed, 1 failed".into(),
            ],
            ..JobInfo::default()
        };
        assert_eq!(
            failing_names(&info, &log).unwrap(),
            BTreeSet::from([("--test full", "module::fails")])
        );
        assert!(failing_names(&info, &log.replace("1 failed", "2 failed")).is_err());
        assert!(failing_names(&info, &log.replace("module::fails", "bad name")).is_err());
        for log in [
            format!("{SELECTOR}error: python test failed: `script.py` (exit 1)\n"),
            format!("{SELECTOR}{TRUNCATED}"),
            format!("{SELECTOR}{SELECTOR}"),
            format!("# {SELECTOR}"),
            format!("quoted \"{SELECTOR}\""),
            "test module::fails ... FAILED\n".into(),
        ] {
            assert!(failing_args(&log).is_err(), "{log:?}");
        }
        for arg in [
            "",
            "--lib twice",
            "--bins",
            "--test",
            "--test a b",
            "--test a&b",
            "--test a\r",
            "--test C:x",
            r"--bin x\y",
        ] {
            let line = format!("error: test failed, to rerun pass `{arg}` (exit 101, src/lib.rs)\n");
            assert!(failing_args(&line).is_err(), "{arg:?}");
        }
        for (arg, words) in [("--test full", 4), ("--lib", 3), ("--bin b", 4)] {
            let line = candidate_line("p", arg, "module::fails", "q-1", "2026-11-27 05:00:00 UTC").unwrap();
            let parsed = crate::flake::parse_list(&line).unwrap();
            assert_eq!(parsed.len(), 1, "{line}");
            assert_eq!((parsed[0].target.as_str(), parsed[0].name.as_str()), (arg, "module::fails"));
            assert_eq!(line.split(" # ").next().unwrap().split_whitespace().count(), words);
        }
        assert!(candidate_line("p", "--test full", "a b", "q-1", "t").is_err());
        let root = crate::scrub::planted::config_root("bisect-evidence-size");
        let path = root.join("large.log");
        fs::write(&path, SELECTOR.repeat(100_000)).unwrap();
        let oversize = read_text(&path, SELECTOR.len() as u64).unwrap_err();
        assert!(oversize.contains("over the size limit"), "{oversize}");
        fs::remove_dir_all(root).unwrap();
    }

    const A_PASS: &str = "flaky rerun: passed --test a t1 (first run failed)\n";
    fn a_log(rerun: &str) -> String {
        format!(
            "     Running tests/a.rs (build/a)\ntest t1 ... FAILED\n{FAILED}\
             error: test failed, to rerun pass `--test a` (exit 101, tests/a.rs)\n{rerun}"
        )
    }
    fn filtered(log: &str) -> Result<Vec<&str>, String> {
        bisect_selectors(log, failing_args(log)?, |_| Ok(()))
    }

    #[test]
    fn a_binary_that_passed_its_flaky_rerun_is_not_a_bisect_selector() {
        let log = format!(
            "{}     Running tests/full.rs (build/full)\ntest t2 ... FAILED\n{FAILED}{SELECTOR}\
             flaky rerun: none (one rerun already used in this job)\n",
            a_log(A_PASS)
        );
        let excused = excused_by_rerun(&log, &["--test a", "--test full"]).unwrap();
        assert_eq!(excused, BTreeSet::from(["--test a"]));
        assert_eq!(filtered(&log).unwrap(), ["--test full"]);
    }
    #[test]
    fn a_binary_whose_rerun_failed_is_still_blamed() {
        let log = a_log(&format!("{A_PASS}flaky rerun: failed (exit 101) — --test a t1\n"));
        assert_eq!(filtered(&log).unwrap(), ["--test a"]);
    }
    #[test]
    fn a_partly_rerun_binary_is_not_excused() {
        let log = a_log(A_PASS)
            .replace("test result:", "test t2 ... FAILED\ntest result:")
            .replace("1 failed;", "2 failed;");
        assert_eq!(filtered(&log).unwrap(), ["--test a"]);
    }
    #[test]
    fn a_passed_line_for_another_binary_or_name_excuses_nothing() {
        for rerun in [A_PASS.replace(" a ", " b "), A_PASS.replace("t1", "t9"), A_PASS.repeat(2)] {
            assert_eq!(filtered(&a_log(&rerun)).unwrap(), ["--test a"], "{rerun:?}");
        }
    }
    #[test]
    fn a_refused_rerun_excuses_nothing() {
        let log = a_log("flaky rerun: none (unlisted failure: t1)\n");
        assert_eq!(filtered(&log).unwrap(), ["--test a"]);
    }
    #[test]
    fn every_selector_excused_is_no_verdict() {
        assert_eq!(
            filtered(&a_log(A_PASS)).unwrap_err(),
            "every failing test binary passed its flaky rerun; no selector to bisect"
        );
    }
    #[test]
    fn without_rerun_lines_the_selectors_are_unchanged() {
        let lib = "error: test failed, to rerun pass `--lib` (exit 101, src/lib.rs)\r\n";
        let log = format!("test a ... FAILED\n{SELECTOR}{lib}");
        assert_eq!(filtered(&log).unwrap(), failing_args(&log).unwrap());
    }

    #[test]
    fn a_build_lock_refusal_or_a_killed_rerun_is_no_verdict() {
        let success = vec!["run: ok (0.2 s)".into()];
        let failure = vec!["run: FAILED (exit 101)".into()];
        let mixed = format!("{OK}{FAILED}");
        let passing = format!("{OK}{OK}");
        let killed = vec!["run: KILLED (disk below floor)".into()];
        let running = "     Running tests/full.rs (build/full)\n";
        let cases: &[(u8, &[String], &str, usize)] = &[
            (crate::buildsrc::EXIT_BUSY, &[][..], mixed.as_str(), 2),
            (crate::job::EXIT_DISK_DROPPED, killed.as_slice(), &mixed, 2),
            (58, &["run: FAILED (exit 58)".into()], running, 2),
            (101, &[], &mixed, 2),
            (0, &[], &passing, 2),
            (0, &success, &mixed, 2),
            (101, &failure, &passing, 2),
            (101, &failure, FAILED, 2),
            (101, &failure, &mixed, 1),
            (0, &success, "no test results\n", 0),
            (101, &failure, &format!("{mixed}error: could not compile `p`\n"), 2),
            (101, &failure, &format!("error[E0425]: missing name\n{mixed}"), 2),
            (101, &failure, &format!("{mixed}{TRUNCATED}"), 2),
            (0, &success, &format!("{passing}{TRUNCATED}"), 2),
            (0, &success, &passing.replace("1 passed", "+1 passed"), 2),
            (0, &success, &passing.replace("1 passed", "18446744073709551616 passed"), 2),
        ];
        for &(code, summary, log, count) in cases {
            assert!(
                matches!(classify_rerun(code, summary, log, count), Rerun::NoVerdict(_)),
                "exit={code} summary={summary:?} selectors={count} log={log:?}"
            );
        }
        assert_eq!(classify_rerun(101, &failure, &mixed, 2), Rerun::Fail);
        assert_eq!(classify_rerun(0, &success, &passing, 2), Rerun::Pass);
    }

    #[test]
    fn the_bisect_walks_back_and_blames_the_member_after_the_last_passing_boundary() {
        // Member 4 already failed. Try 3 (fails), then 2 (passes), and never touch 1.
        let mut seen = Vec::new();
        assert_eq!(
            walk_back(3, |i| {
                seen.push(i);
                match i {
                    2 => Rerun::Fail,
                    1 => Rerun::Pass,
                    _ => panic!("ran after the passing boundary"),
                }
            }),
            Ok(2)
        );
        assert_eq!(seen, [2, 1]);
        assert_eq!(walk_back(3, |_| Rerun::Fail), Ok(0));
        assert_eq!(walk_back(0, |_| panic!("no earlier members")), Ok(0));
        assert_eq!(
            walk_back(3, |i| match i {
                2 => Rerun::Fail,
                1 => Rerun::NoVerdict("refused".into()),
                _ => panic!("ran after no verdict"),
            }),
            Err("refused".into())
        );
    }
}
