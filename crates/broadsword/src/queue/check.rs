use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec};
use crate::perfgate::{self, Gate as Perf};
use crate::donecheck;
use crate::{tools, wtgit};
use std::path::Path;
use super::entry::queue_dir;
use super::merge::trunk_checkout;
use super::{CHECK_FAILED, EXIT_REJECTED, EXIT_RETRY, PERF_FAILED, R, REJECTED, Run, STALE};
use super::view::{short, some};

/// `check <project>` of the project's main checkout `dir`, only while it has its trunk checked out: the job id when it passed, else why
/// not (refused, or the job's exit and summary).
pub(super) fn main_check_now(cfg: &Config, dir: &Path, project: &str) -> Result<String, String> {
    trunk_checkout(dir, "")?;
    if let Some(code) = tools::guard_refusal(cfg, project, None) {
        return Err(format!("the check of main was refused before it started, exit {code}"));
    }
    let mut info = JobInfo::default();
    let code = job::run_job_info(cfg, JobSpec { kind: "check".into(), name: project.to_string(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: None }, &mut info);
    let job_id = info.id.clone().unwrap_or_else(|| "none".into());
    if code != 0 {
        return Err(format!("exit {code}, job {job_id}: {}", info.summary.join("; ")));
    }
    Ok(job_id)
}

/// The command line of a fast subset: one build, then the unit tests (`units`: the `--lib` / `--bins` flags `targets_of` found)
/// and each listed test binary.
fn fast_command(units: &[&str], tests: &[String]) -> String {
    let mut c = "cargo build --offline && cargo test --offline".to_string();
    for u in units {
        c.push_str(&format!(" {u}"));
    }
    for t in tests {
        c.push_str(&format!(" --test {t}"));
    }
    c
}

/// What cargo will test in a folder, from its `cargo metadata`: the unit-test flags and the test target names of the SELECTED
/// packages, the workspace's default members (`workspace_default_members`: the root package when the root is one, else
/// `default-members` or every member), never a member cargo does not test.
#[derive(Debug, PartialEq, Eq)]
struct Targets {
    /// `--lib` when a selected package has a library target, `--bins` when one has a binary target (cargo refuses `--lib` when no
    /// selected package has a library: the Broadsword crate has only binaries).
    units: Vec<&'static str>,
    /// The names of the selected packages' targets of kind `test` (`tests/*.rs` found by cargo, and `[[test]]` targets by their
    /// `name`): THE ONE SOURCE of valid `merge_fast_tests` names. A file cargo does not build (`autotests = false`, or a path a
    /// renamed `[[test]]` uses) is never one.
    tests: Vec<String>,
}

/// `Targets` of a `cargo metadata` output. Anything missing or malformed is an error (the caller then runs the full check).
fn targets_of(metadata: &str) -> Result<Targets, String> {
    use crate::json::J;
    let j = crate::json::parse(metadata).map_err(|e| format!("cannot read cargo metadata output: {e}"))?;
    let Some(J::Arr(pkgs)) = j.get("packages") else { return Err("cargo metadata output: `packages` is not a list".into()) };
    let Some(J::Arr(defaults)) = j.get("workspace_default_members") else { return Err("cargo metadata output: no `workspace_default_members` list (cargo too old?)".into()) };
    let selected: Vec<&str> = defaults.iter().filter_map(J::str).collect();
    if selected.is_empty() {
        return Err("cargo metadata output: `workspace_default_members` names no package".into());
    }
    let (mut lib, mut bin, mut tests) = (false, false, Vec::new());
    for p in pkgs.iter().filter(|p| p.get("id").and_then(J::str).is_some_and(|id| selected.contains(&id))) {
        let Some(J::Arr(targets)) = p.get("targets") else { return Err("cargo metadata output: a package has no `targets` list".into()) };
        for t in targets {
            let Some(J::Arr(kinds)) = t.get("kind") else { return Err("cargo metadata output: a target has no `kind` list".into()) };
            for k in kinds.iter().filter_map(J::str) {
                lib |= matches!(k, "lib" | "rlib" | "dylib" | "cdylib" | "staticlib" | "proc-macro");
                bin |= k == "bin";
                if k == "test" {
                    let Some(name) = t.get("name").and_then(J::str) else { return Err("cargo metadata output: a test target has no `name`".into()) };
                    tests.push(name.to_string());
                }
            }
        }
    }
    Ok(Targets { units: [(lib, "--lib"), (bin, "--bins")].into_iter().filter(|(on, _)| *on).map(|(_, f)| f).collect(), tests })
}

/// What the pre-merge check of a listed project runs (`fast_plan`).
#[derive(Debug, PartialEq, Eq)]
enum FastPlan {
    /// Every listed name is a test target cargo metadata lists: run this subset command.
    Subset(String),
    /// The names could not be verified (cargo metadata could not run or be read): the FULL check runs, for this reason.
    Unverifiable(String),
    /// A listed name is not a test target cargo metadata lists: the check is refused (fails), for this reason.
    Refused(String),
}

/// THE ONE CHECK of a project's `merge_fast_tests` names, against `metadata` (the `cargo metadata` output of the commit being
/// checked, or the error that kept it from running).
fn fast_plan(project: &str, tests: &[String], metadata: Result<String, String>) -> FastPlan {
    let t = match metadata.and_then(|m| targets_of(&m)) {
        Ok(t) => t,
        Err(m) => return FastPlan::Unverifiable(m),
    };
    if let Some(bad) = tests.iter().find(|n| !t.tests.contains(n)) {
        return FastPlan::Refused(format!("merge_fast_tests: `{bad}` (project {project}) is not a test target cargo metadata lists for the packages cargo tests; its test targets are [{}]", t.tests.join(" ")));
    }
    FastPlan::Subset(fast_command(&t.units, tests))
}

/// THE ONE PRE-MERGE CHECK of an entry's commit in task `task_id`'s worktree (the single entry's own check, and every merge-train
/// check: tip, bisect and alone). A listed project may run only Python tests for a proven no-Rust diff (`no_rust`); otherwise it
/// gets the fast subset when `fast_plan` verifies its names:
/// ONE `run` job of `fast_command`, then any configured Python tests touched by the diff, through the same job machinery as
/// `check` (disk guard, build lock, log, scrubbed environment, redaction). Unverifiable names or failed Python selection give the
/// full `check`; a name cargo does not know refuses the check (exit 1, no job).
/// Any other project gets the full `check`, exactly as before. Returns the exit code, normalised so a Python or subset command that
/// failed is 1 like a failed check (a refusal or a disk kill keeps its own code: no verdict), and the line that says which check
/// ran. That line ends the job's lines in the entry's `check_summary` and heads its `check_tail`, so neither a record nor the
/// reviewer can read a subset as the full suite. The main check after a merge never comes through here: it is always the full
/// `check` (`s_main_check`).
fn pre_merge_check(cfg: &Config, project: &str, task_id: &str, info: &mut JobInfo) -> (u8, String) {
    let full = |info: &mut JobInfo| job::run_job_info(cfg, JobSpec { kind: "check".into(), name: project.to_string(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: Some(task_id.to_string()) }, info);
    let Some(tests) = cfg.merge_fast_subset(project) else {
        return (full(info), "check: full".to_string());
    };
    if let Some(result) = super::no_rust::pre_merge_check(cfg, project, task_id, info) {
        return result;
    }
    let metadata = job::job_folder(cfg, project, Some(task_id)).map_err(|(_, m)| m).and_then(|d| crate::buildsrc::metadata_json(cfg, &d));
    match fast_plan(project, tests, metadata) {
        FastPlan::Unverifiable(why) => {
            println!("fast subset off for {project}: test names could not be verified ({why}); the FULL check runs instead");
            (full(info), format!("check: full (fast subset off: test names could not be verified: {why})"))
        }
        FastPlan::Refused(why) => {
            println!("fast subset check of {project} --task {task_id} REFUSED: {why}");
            // the reason is the check's summary, so the entry ends check_failed saying why (an empty summary would read as no verdict)
            info.summary = vec![format!("check: REFUSED before any job ran: {why}")];
            (1, format!("check: REFUSED ({why}); nothing ran. Fix merge_fast_tests in broadsword.toml"))
        }
        FastPlan::Subset(command) => {
            let (command, python) = match super::no_rust::subset_python(cfg, project, task_id) {
                Ok(None) => (command, String::new()),
                Ok(Some(py)) => (
                    format!("{command} && {}", py.command),
                    format!(
                        " + python ({} of {} script(s), {})",
                        py.selected, py.of, py.why
                    ),
                ),
                Err(why) => {
                    println!("fast subset off for {project}: python selection failed ({why}); the FULL check runs instead");
                    return (
                        full(info),
                        format!("check: full (python selection failed: {why})"),
                    );
                }
            };
            println!("fast subset check of {project} --task {task_id} (merge_fast_tests): {command}; the full check runs on main after the merge");
            let code = job::run_job_info(cfg, JobSpec { kind: "run".into(), name: project.to_string(), command: Some(command.clone()), first_step: 0, resumed_from: None, resume_count: 0, task: Some(task_id.to_string()) }, info);
            let failed = code != 0 && info.summary.first().is_some_and(|l| l.starts_with("run: FAILED"));
            let label = format!("check: fast subset ({}){python} = `{command}`, NOT the full suite: the full check runs on main right after the merge, and a failure there is reverted", tests.join(" "));
            (if failed { 1 } else { code }, label)
        }
    }
}

impl<'a> Run<'a> {
    /// The check step: the real check, or (merge train mode) the deferral that leaves the check to the train.
    pub(super) fn check_step(&mut self) -> R<()> {
        if self.defer_check {
            self.s_defer_check()?;
        } else {
            self.s_check()?;
        }
        self.s_done_check()
    }

    // -- merge train (see `run_train`) ------------------------------------------------------

    /// MERGE TRAIN, check step: the entry's own check is NOT run. The commit that is reviewed is recorded as `checked_tip` (review and
    /// merge are bound to it as always); `check_deferred = yes` says that no check has run on it, and the review prompt says so too.
    /// The build and tests run once, on the whole train; a deferred commit never merges before a check of it has passed (`merge_phase`).
    fn s_defer_check(&mut self) -> R<()> {
        self.set_step("check")?;
        let (dir, branch) = (self.dir.clone(), self.branch());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        self.e.set("check_job", "");
        self.e.set("check_exit", "");
        self.e.set("check_summary", "NOT RUN for this change alone: with a merge train the build and tests run once on the combined branches of every approved change, after the reviews, and nothing merges before that check passes. Judge the diff on its own merits.");
        self.e.set("check_tail", "");
        self.e.set("checked_tip", tip.clone());
        self.e.set("check_deferred", "yes");
        self.e.set("train_checked", "");
        self.save()?;
        self.log("check", &format!("deferred to the merge train: commit {} is reviewed without a check of its own", short(&tip)))
    }

    /// PERF GATE of the commit the task's worktree holds now (`perfgate::run_gate`, the one gate every check path calls): the report
    /// is recorded in the entry (`perf`) and its log; a skip is printed. The caller decides what the outcome means for the entry.
    fn perf_gate(&mut self, what: &str) -> Perf {
        let (project, task_id) = (self.e.get("project").to_string(), self.e.get("task").to_string());
        let g = perfgate::run_gate(self.cfg, &project, &task_id, what);
        let text = match &g {
            Perf::Skipped(line) => {
                println!("{line}");
                line.clone()
            }
            Perf::Pass(lines) | Perf::Fail(_, lines) => lines.join("\n"),
            Perf::NoVerdict(m) => format!("perf gate {project}: no verdict: {m}"),
        };
        self.e.set("perf", text.clone());
        let last = text.lines().last().unwrap_or("").to_string();
        let _ = self.log("perf", &format!("{what}: {last}"));
        g
    }

    /// NB-18 PLUG-IN POINT 1 of 2. Check `commit` of this entry's project: the build and tests, as `check <project>` runs them.
    /// Interim implementation (nb-18 is not merged): the check runs in this entry's task worktree (`check <project> --task <id>`), and
    /// only for a commit that worktree has checked out right now. nb-18 (check from a clean checkout of the exact commit) replaces the
    /// body with "check `commit` in a clean checkout", and then `check_alone` no longer needs to move the worktree.
    pub(super) fn check_commit(&mut self, commit: &str, what: &str) -> TrainCheck {
        let (project, task_id) = (self.e.get("project").to_string(), self.e.get("task").to_string());
        if let Some(code) = tools::guard_refusal(self.cfg, &project, Some(&task_id)) {
            return TrainCheck::Blocked(format!("the check was refused before it started (exit {code}; the reason is printed above, e.g. Broadsword paused or an interrupted run to recover)"));
        }
        let head = match wtgit::head_commit(&self.wt) {
            Ok(h) => h,
            Err(m) => return TrainCheck::Blocked(m),
        };
        if head != commit {
            return TrainCheck::EntryBlocked(format!("the worktree of task {task_id} is at {} instead of {}: the interim check can only check a commit its worktree has checked out", short(&head), short(commit)));
        }
        match wtgit::status_porcelain_plain(&self.wt) {
            Ok(s) if s.is_empty() => {}
            Ok(s) => return TrainCheck::EntryBlocked(format!("the worktree of task {task_id} has uncommitted or untracked changes ({}); the check would not be of commit {} alone, so it did not run", some(&s), short(commit))),
            Err(m) => return TrainCheck::Blocked(m),
        }
        println!("queue entry {}: check {project} --task {task_id} ({what}; commit {})", self.e.id(), short(commit));
        let mut info = JobInfo::default();
        let (code, which) = pre_merge_check(self.cfg, &project, &task_id, &mut info);
        let mut summary = format!("{}\n{which}", info.summary.join("\n"));
        let tail = format!("{which}\n{}", info.log.as_deref().map(|p| read_tail(p, self.cfg.reviewer_check_tail_kb * 1024)).unwrap_or_default());
        let job_id = info.id.clone().unwrap_or_default();
        // PERF GATE, part of the check: only once the build and tests passed, on the same commit (verified below, after it too)
        let mut perf_fail = String::new();
        if code == 0 {
            match self.perf_gate(what) {
                Perf::Skipped(_) | Perf::Pass(_) => {}
                Perf::Fail(reason, _) => perf_fail = reason,
                Perf::NoVerdict(m) => return TrainCheck::Blocked(format!("the perf gate could not run to a verdict (check job {}): {m}", if job_id.is_empty() { "none" } else { job_id.as_str() })),
            }
            summary = format!("{summary}\n{}", self.e.get("perf"));
        }
        // The check is bound to `commit` only if the worktree still holds exactly that commit, with nothing else, when it ends (the
        // single-entry path's "branch changed during the check" guard). Whatever the verdict, a changed worktree gives none.
        let head_after = match wtgit::head_commit(&self.wt) {
            Ok(h) => h,
            Err(m) => return TrainCheck::Blocked(format!("the worktree of task {task_id} could not be read after the check (job {}): {m}", if job_id.is_empty() { "none" } else { job_id.as_str() })),
        };
        if head_after != commit {
            return TrainCheck::EntryBlocked(format!("the worktree of task {task_id} changed during the check (job {}): it was at {} when the check started and is at {} now; the check is not of that commit, nothing was merged", if job_id.is_empty() { "none" } else { job_id.as_str() }, short(commit), short(&head_after)));
        }
        match wtgit::status_porcelain_plain(&self.wt) {
            Ok(s) if s.is_empty() => {}
            Ok(s) => return TrainCheck::EntryBlocked(format!("the worktree of task {task_id} changed during the check (job {}): it got uncommitted or untracked changes ({}); the check is not of commit {} alone, nothing was merged", if job_id.is_empty() { "none" } else { job_id.as_str() }, some(&s), short(commit))),
            Err(m) => return TrainCheck::Blocked(format!("the worktree of task {task_id} could not be read after the check (job {}): {m}", if job_id.is_empty() { "none" } else { job_id.as_str() })),
        }
        match code {
            0 if perf_fail.is_empty() => TrainCheck::Pass { job: job_id, summary, tail },
            0 => TrainCheck::Fail { job: job_id, summary, tail, perf: perf_fail },
            1 if !info.summary.is_empty() => TrainCheck::Fail { job: job_id, summary, tail, perf: String::new() },
            c => TrainCheck::Blocked(format!("the check could not run to a verdict (exit {c}; job {})", if job_id.is_empty() { "none" } else { job_id.as_str() })),
        }
    }

    /// NB-18 PLUG-IN POINT 2 of 2. Check this entry's OWN commit (`alone_tip`: the reviewed commit on main as it was, before a train
    /// chained it) without the entries before it. Interim: the worktree is put on that commit (detached, no branch moves), checked, and
    /// put back on its branch. With nb-18 this is simply `check_commit(alone_tip)` in a clean checkout.
    pub(super) fn check_alone(&mut self) -> TrainCheck {
        let alone = self.e.get("alone_tip").to_string();
        if alone.is_empty() {
            return TrainCheck::Blocked("no alone commit is recorded for this entry".into());
        }
        let (wt, branch) = (self.wt.clone(), self.branch());
        // recorded BEFORE the worktree is detached: a run killed while it is detached leaves this key, and the next run's `locate`
        // puts the worktree back on its branch (`restore_alone_checkout`)
        self.e.set("alone_detached", alone.clone());
        if self.save().is_err() {
            return TrainCheck::Blocked("could not record that the worktree is about to be detached; it was not moved".into());
        }
        if let Err(m) = wtgit::checkout_detach(&wt, &alone) {
            return TrainCheck::Blocked(format!("cannot put the worktree on the entry's own commit {}: {m}", short(&alone)));
        }
        let res = self.check_commit(&alone, "alone: the entry's own commit without the entries before it");
        if let Err(m) = wtgit::checkout_branch(&wt, &branch) {
            return TrainCheck::Blocked(format!("the worktree could NOT be put back on `{branch}` ({m}); do it by hand: git -C {} checkout {branch}", wt.display()));
        }
        self.e.set("alone_detached", "");
        if self.save().is_err() {
            return TrainCheck::Blocked("the worktree is back on its branch, but the record could not be written".into());
        }
        res
    }

    // -- step 3: check ----------------------------------------------------------------------

    pub(super) fn s_check(&mut self) -> R<()> {
        self.set_step("check")?;
        let (project, task_id) = (self.e.get("project").to_string(), self.e.get("task").to_string());
        if let Some(code) = tools::guard_refusal(self.cfg, &project, Some(&task_id)) {
            return Err(self.blocked(&format!("the check was refused before it started (exit {code}; the reason is printed above, e.g. Broadsword paused or an interrupted run to recover); nothing changed")));
        }
        println!("queue entry {}: check {project} --task {task_id}", self.e.id());
        let (dir, branch) = (self.dir.clone(), self.branch());
        let tip_before = self.g(wtgit::branch_tip(&dir, &branch))?;
        let mut info = JobInfo::default();
        let (code, which) = pre_merge_check(self.cfg, &project, &task_id, &mut info);
        let summary = format!("{}\n{which}", info.summary.join("\n"));
        let tail = format!("{which}\n{}", info.log.as_deref().map(|p| read_tail(p, self.cfg.reviewer_check_tail_kb * 1024)).unwrap_or_default());
        self.e.set("check_job", info.id.clone().unwrap_or_default());
        self.e.set("check_exit", code.to_string());
        self.e.set("check_summary", summary.clone());
        self.e.set("check_tail", tail);
        self.save()?;
        match code {
            0 => {
                // The one commit the check ran on. Review and merge are bound to exactly this commit.
                let tip_after = self.g(wtgit::branch_tip(&dir, &branch))?;
                if tip_after != tip_before {
                    return Err(self.end(STALE, &format!("branch changed during the check: `{branch}` was at {} when the check started and is at {} now; nothing was reviewed or merged. Run `queue run --entry {}` to check it again", short(&tip_before), short(&tip_after), self.e.id()), EXIT_RETRY));
                }
                // PERF GATE on the commit the check just passed, before any model is asked
                let gate = self.perf_gate("the entry's own check");
                let tip_perf = self.g(wtgit::branch_tip(&dir, &branch))?;
                if tip_perf != tip_before {
                    return Err(self.end(STALE, &format!("branch changed during the perf gate: `{branch}` was at {} when the check started and is at {} now; nothing was reviewed or merged. Run `queue run --entry {}` to check it again", short(&tip_before), short(&tip_perf), self.e.id()), EXIT_RETRY));
                }
                match gate {
                    Perf::Skipped(_) | Perf::Pass(_) => {
                        let perf = self.e.get("perf").to_string();
                        self.e.set("check_summary", format!("{summary}\n{perf}"));
                    }
                    Perf::Fail(reason, _) => {
                        return Err(self.end(PERF_FAILED, &format!("the build and tests passed (job {}) but the perf gate failed on commit {}: {reason}. No model was asked. Make it fast again, commit, and queue the task again with `queue add` (if the slowdown is intended, the baseline must be re-recorded first)", info.id.as_deref().unwrap_or("?"), short(&tip_before)), EXIT_REJECTED));
                    }
                    Perf::NoVerdict(m) => {
                        return Err(self.blocked(&format!("the build and tests passed but the perf gate could not run to a verdict: {m}; nothing was reviewed or merged; fix the cause, then `queue run --entry {}`", self.e.id())));
                    }
                }
                self.e.set("checked_tip", tip_before.clone());
                self.e.set("check_deferred", "");
                self.save()?;
                self.log("check", &format!("passed on commit {} ({})", short(&tip_before), info.summary.join("; ")))
            }
            1 if !info.summary.is_empty() => Err(self.end(CHECK_FAILED, &format!("build or tests failed on the rebased branch (job {}): {}. No model was asked. Fix it, commit, and queue the task again with `queue add`", info.id.as_deref().unwrap_or("?"), info.summary.join("; ")), EXIT_REJECTED)),
            c => Err(self.blocked(&format!("the check could not run to a verdict (exit {c}; job {}); nothing was merged; see its output above, fix the cause, then `queue run --entry {}`", info.id.as_deref().unwrap_or("none"), self.e.id()))),
        }
    }

    /// DONE CHECK (donecheck.rs), the end of the check step, before any model is asked. The brief's runnable done-check commands
    /// (`broadsword build|run <project> --task <task>`, see `donecheck::classify`) run in the task's worktree on the commit that is
    /// about to be reviewed; `broadsword test|check` lines are never run, but shown COVERED with the queue check's result (pending
    /// for a train member: its train's check decides); the others are listed NOT RUN. WHERE: a single entry runs them right after its own check (and
    /// perf gate) passed; a MERGE TRAIN member, whose own check is deferred, runs them on its own rebased commit in its worktree
    /// before its review, because the review is per entry and a failing done check must reject before any review is spent. They
    /// run once per attempt: the deferred check a train-less merge makes later (`merge_phase`) does not repeat them. The result,
    /// redacted, is stored in `done_check` (the review prompt's "DONE CHECK, RUN BY THE QUEUE" section) and in
    /// `<entry>.donecheck` (`<entry>.<attempt>.donecheck` for a later attempt). A command that FAILS (exit 1, or killed at
    /// done_check_timeout_seconds) ends the entry `rejected` with no model asked; one that gives no verdict ends it `blocked`; a
    /// branch that moved while they ran ends it `stale`.
    fn s_done_check(&mut self) -> R<()> {
        let (dir, branch, wt) = (self.dir.clone(), self.branch(), self.wt.clone());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        let stored = self.e.get("project").to_string();
        let project = job::canonical_project(&self.cfg.projects_dir, &stored).unwrap_or(stored);
        let plan = donecheck::plan(self.e.get("brief"), &self.cfg.projects_dir, &project, self.e.get("task"));
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => return Err(self.blocked(&format!("the done check cannot run: the path of this broadsword binary cannot be read ({e}); nothing was reviewed or merged"))),
        };
        let limits = donecheck::Limits::from_config(self.cfg);
        self.log("check", &format!("done check: {} item(s) in the brief's Done check, {} runnable; running them on commit {} in the task's worktree", plan.items.len(), plan.runnable(), short(&tip)))?;
        let id = self.e.id().to_string();
        let report = donecheck::run_plan(self.cfg, &exe, &wt, plan, &limits, &self.secrets, |m| println!("queue entry {id}: {m}"));
        // `broadsword test|check` lines are never run: the queue's own check covers them (a train member's check is its train's)
        let covered = if self.e.get("check_deferred") == "yes" {
            "COVERED (pending: the merge train's check of the combined branches decides; nothing merges unless it passes)".to_string()
        } else {
            format!("COVERED by the queue's check (job {}: {})", if self.e.get("check_job").is_empty() { "none" } else { self.e.get("check_job") }, self.e.get("check_summary").lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("; "))
        };
        let text = self.secrets.redact(&format!("commit: {tip}\n{}", report.text(&exe, &limits, &covered)));
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 { format!("{id}.donecheck") } else { format!("{id}.{attempt}.donecheck") };
        let path = queue_dir(self.cfg).join(&name);
        let file = format!("entry: {id}\nattempt: {attempt}\ntask: {}\nproject: {project}\n{text}", self.e.get("task"));
        let tmp = path.with_extension("donecheck.tmp");
        if let Err(e) = std::fs::write(&tmp, self.secrets.redact(&file).as_bytes()).and_then(|_| std::fs::rename(&tmp, &path)) {
            return Err(self.internal(&format!("cannot write the done-check file {}: {e}", path.display())));
        }
        self.e.set("done_check", text);
        self.e.set("done_check_file", name.clone());
        self.save()?;
        let now = self.g(wtgit::branch_tip(&dir, &branch))?;
        if now != tip {
            return Err(self.end(STALE, &format!("branch changed during the done check: `{branch}` was at {} and is at {} now; nothing was reviewed or merged. Run `queue run --entry {id}` to check it again", short(&tip), short(&now)), EXIT_RETRY));
        }
        let which = report.stopped_at.as_ref().map(|(i, _)| report.plan.items[*i].label().to_string()).unwrap_or_default();
        match report.verdict() {
            donecheck::Verdict::Pass => self.log("check", &format!("done check passed on commit {} (see state/queue/{name})", short(&tip))),
            donecheck::Verdict::Fail(m) => Err(self.end(REJECTED, &format!("done-check command [{which}] of the brief FAILED ({m}) on commit {}; see state/queue/{name}. No model was asked. Fix it, commit, and queue the task again with `queue add`", short(&tip)), EXIT_REJECTED)),
            donecheck::Verdict::Inconclusive(m) => Err(self.blocked(&format!("done-check command [{which}] of the brief could not run to a verdict ({m}); see state/queue/{name}; nothing was reviewed or merged; fix the cause, then `queue run --entry {id}`"))),
        }
    }
}

// ---- merge train ----------------------------------------------------------------------------

/// The result of one check of a commit (see `Run::check_commit`).
pub(super) enum TrainCheck {
    Pass { job: String, summary: String, tail: String },
    /// The build or tests failed (`perf` empty), or they passed and the PERF GATE failed (`perf` is its reason).
    Fail { job: String, summary: String, tail: String, perf: String },
    /// The check could not run to a verdict for a reason that is not this entry's own (refused, paused, git could not be read, the
    /// job could not start or end): no conclusion about the code, and the drain stops (the next one tries again).
    Blocked(String),
    /// The check could not run to a verdict because of THIS entry's worktree: it was not exactly the commit to check, before or after
    /// the job (another commit or branch checked out, uncommitted or untracked files). As on the single-entry path, only this entry
    /// ends `blocked` (`leave_train`); the other members stay `approved` and the drain goes on, so one entry never stops every drain.
    EntryBlocked(String),
}

/// What a check recorded: the job id, its summary lines and the end of its output.
#[derive(Default, Clone)]
pub(super) struct PassInfo {
    pub(super) job: String,
    pub(super) summary: String,
    pub(super) tail: String,
    /// A failed check: the perf gate's reason when it was the perf gate that failed (empty for a build or test failure).
    pub(super) perf: String,
}

fn read_tail(path: &Path, max: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let cut = len > max;
    if cut && f.seek(SeekFrom::Start(len - max)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let s = String::from_utf8_lossy(&buf).into_owned();
    if cut {
        let rest = s.split_once('\n').map(|(_, r)| r.to_string()).unwrap_or(s);
        format!("[... earlier output cut: only the last {} KB are shown ...]\n{rest}", max / 1024)
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_keeps_the_end_and_says_it_cut() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-queue-tail-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let f = d.join("x.log");
        std::fs::write(&f, "line one\nline two\nline three\n").unwrap();
        assert_eq!(read_tail(&f, 1024), "line one\nline two\nline three\n");
        let t = read_tail(&f, 20);
        assert!(t.starts_with("[... earlier output cut") && t.ends_with("line three\n") && !t.contains("line one"), "{t}");
        assert_eq!(read_tail(&d.join("missing"), 10), "");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_fast_subset_builds_its_command() {
        assert_eq!(fast_command(&["--bins"], &["queue".into(), "wtgit".into()]), "cargo build --offline && cargo test --offline --bins --test queue --test wtgit");
        assert_eq!(fast_command(&["--lib", "--bins"], &["t".into()]), "cargo build --offline && cargo test --offline --lib --bins --test t");
    }

    #[test]
    fn targets_count_only_the_default_members() {
        let pkg = |id: &str, kind: &str, name: &str| format!("{{\"id\":\"{id}\",\"targets\":[{{\"kind\":[\"{kind}\"],\"name\":\"{name}\"}}]}}");
        let meta = |defaults: &str| format!("{{\"packages\":[{},{},{}],\"workspace_default_members\":{defaults}}}", pkg("root", "bin", "root"), pkg("inner", "lib", "inner"), pkg("t", "test", "foo"));
        let units = |d: &str| targets_of(&meta(d)).unwrap().units;
        assert_eq!(units("[\"root\"]"), ["--bins"]);
        assert_eq!(units("[\"root\",\"inner\"]"), ["--lib", "--bins"]);
        assert_eq!(targets_of(&meta("[\"root\"]")).unwrap().tests, Vec::<String>::new());
        assert_eq!(targets_of(&meta("[\"root\",\"t\"]")).unwrap().tests, ["foo"]);
        assert!(targets_of(&meta("[]")).is_err());
        assert!(targets_of(&format!("{{\"packages\":[{}]}}", pkg("root", "bin", "root"))).is_err());
        assert!(targets_of("{\"packages\":[{\"id\":\"a\",\"targets\":[{\"kind\":[\"test\"]}]}],\"workspace_default_members\":[\"a\"]}").is_err(), "a test target with no name");
    }

    #[test]
    fn fast_plan_checks_the_names_against_the_metadata_only() {
        let meta = "{\"packages\":[{\"id\":\"p\",\"targets\":[{\"kind\":[\"lib\"],\"name\":\"p\"},{\"kind\":[\"test\"],\"name\":\"foo\"}]}],\"workspace_default_members\":[\"p\"]}";
        assert_eq!(fast_plan("p", &["foo".into()], Ok(meta.into())), FastPlan::Subset("cargo build --offline && cargo test --offline --lib --test foo".into()));
        assert!(matches!(fast_plan("p", &["foo".into(), "bar".into()], Ok(meta.into())), FastPlan::Refused(m) if m.contains("`bar` (project p) is not a test target") && m.contains("[foo]")));
        assert!(matches!(fast_plan("p", &["foo".into()], Err("cargo metadata failed".into())), FastPlan::Unverifiable(m) if m == "cargo metadata failed"));
        assert!(matches!(fast_plan("p", &["foo".into()], Ok("not json".into())), FastPlan::Unverifiable(_)));
    }
}
