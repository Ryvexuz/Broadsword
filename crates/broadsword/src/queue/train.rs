use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec};
use crate::lock::{self, Lock};
use crate::record::{self, Record};
use crate::reviewer::{self, CallError};
use crate::task::{self, fail, now_secs};
use crate::worktree::{self, JobDirError};
use crate::perfgate::{self, Gate as Perf};
use crate::donecheck;
use crate::{git, pause, sys, tools, wtgit};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;
use super::entry::{EXT, half_merged, is_open, load_entries, load_entry, process_secrets, queue_dir, resume_first};
use super::{APPROVED, BLOCKED, CANCELLED, CHECK_FAILED, CONFLICT, EXIT_BAD_INPUT, EXIT_BUSY, EXIT_ERROR, EXIT_MERGED_BAD, EXIT_REJECTED, EXIT_RETRY, MERGED, MERGED_CHECK_FAILED, PERF_FAILED, QUEUED, R, REJECTED, Row, Run, RUNNING, STALE, STEP_MERGE, STEPS};

/// One of the queue's two lock files, waiting at most `queue_lock_wait_seconds`; `Err` is the exit code, message printed.
fn take(cfg: &Config, file: &str, what: &str) -> Result<Lock, ExitCode> {
    let wait = cfg.queue_lock_wait_seconds;
    let what = what.to_string();
    lock::acquire(&queue_dir(cfg).join(file), Duration::from_secs(wait), &move |owner| {
        let who = owner.map(|p| format!(" (pid {p})")).unwrap_or_default();
        format!("queue busy: {what}{who} held the lock for the whole {wait} s wait; nothing was changed, try again later (the lock frees itself when its holder ends)")
    })
    .map_err(|e| fail(if e.starts_with("queue busy") { EXIT_BUSY } else { EXIT_ERROR }, e))
}

fn queue_lock(cfg: &Config) -> Result<Lock, ExitCode> {
    take(cfg, "queue.lock", "another `queue run` (or a cancel of a running entry)")
}

/// A run LANE (SCALING F2): the entries one `queue run` may take, and the OS lock that proves no other run takes them. Holding it is
/// what makes a `running` entry of the lane an interrupted one. `key` is a lowercase project name (`lane_key`), or None for every
/// project. The lock is held as long as the Lane lives.
struct Lane {
    _lock: Lock,
    key: Option<String>,
}

impl Lane {
    /// Whether entry `e` is in this lane. Project names are matched without case, like `job::canonical_project` matches them (the
    /// canonical name of a plain name is that name up to case), so this is exact without reading the projects folder.
    fn holds(&self, e: &Record) -> bool {
        self.key.as_ref().is_none_or(|k| e.get("project").to_lowercase() == *k)
    }
}

/// The lane key of an entry's project: its name, lowercased. Err when it is not a plain project name (no lock file could carry it). A
/// plain name that is no longer a project keys as itself, so its entries can still be resumed (to end `blocked`) or cancelled.
fn lane_key(cfg: &Config, name: &str) -> Result<String, String> {
    match job::canonical_project(&cfg.projects_dir, name) {
        Ok(c) => Ok(c.to_lowercase()),
        Err(m) if m.starts_with("unknown project") => Ok(name.to_lowercase()),
        Err(m) => Err(m),
    }
}

/// LANE HOLD (queue_auto_revert): the file `<state_dir>/queue/hold-<lane key>` holds a project's lane after an automatic revert, or
/// the check after it, failed. While it exists `queue run` merges nothing of that project (`hold_gate`, and `lane_hold` before every
/// fast-forward). Only `hold_gate` removes it, and only after a check of that project's main passed.
fn hold_path(cfg: &Config, project: &str) -> PathBuf {
    let key = lane_key(cfg, project).unwrap_or_else(|_| project.to_lowercase());
    queue_dir(cfg).join(format!("hold-{}", record::sanitize(&key)))
}

/// Write the hold of `project`'s lane: UTC time, the entry, the project and the reason, one `key = value` line each.
fn write_hold(cfg: &Config, project: &str, entry: &str, reason: &str) -> Result<PathBuf, String> {
    let path = hold_path(cfg, project);
    let text = format!("time = {}\nentry = {entry}\nproject = {project}\nreason = {}\n", pause::now_string(), reason.replace(['\n', '\r'], " "));
    std::fs::create_dir_all(queue_dir(cfg)).and_then(|_| std::fs::write(&path, text)).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

/// One field of a hold file's text (empty when it is missing).
fn hold_field(text: &str, key: &str) -> String {
    text.lines().find_map(|l| l.strip_prefix(&format!("{key} = "))).unwrap_or("").to_string()
}

/// The reason `project`'s lane is held, or None. A hold file that cannot be read still holds the lane (fail closed).
fn lane_hold(cfg: &Config, project: &str) -> Option<String> {
    let path = hold_path(cfg, project);
    if !path.exists() {
        return None;
    }
    Some(match std::fs::read_to_string(&path) {
        Ok(t) => hold_field(&t, "reason"),
        Err(e) => format!("the hold file {} cannot be read: {e}", path.display()),
    })
}

/// `check <project>` of the project's main checkout `dir`, only while it has its trunk checked out: the job id when it passed, else why
/// not (refused, or the job's exit and summary).
fn main_check_now(cfg: &Config, dir: &Path, project: &str) -> Result<String, String> {
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
/// check: tip, bisect and alone). A project listed in `merge_fast_tests` gets the fast subset when `fast_plan` verifies its names:
/// ONE `run` job of `fast_command` through the same job machinery as `check` (disk guard, build lock, log, scrubbed environment,
/// redaction). Names that cannot be verified give the full `check`; a name cargo does not know refuses the check (exit 1, no job).
/// Any other project gets the full `check`, exactly as before. Returns the exit code, normalised so a subset whose command ran and
/// failed is 1 like a failed check (a refusal or a disk kill keeps its own code: no verdict), and the line that says which check
/// ran. That line ends the job's lines in the entry's `check_summary` and heads its `check_tail`, so neither a record nor the
/// reviewer can read a subset as the full suite. The main check after a merge never comes through here: it is always the full
/// `check` (`s_main_check`).
fn pre_merge_check(cfg: &Config, project: &str, task_id: &str, info: &mut JobInfo) -> (u8, String) {
    let full = |info: &mut JobInfo| job::run_job_info(cfg, JobSpec { kind: "check".into(), name: project.to_string(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: Some(task_id.to_string()) }, info);
    let Some(tests) = cfg.merge_fast_subset(project) else {
        return (full(info), "check: full".to_string());
    };
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
            println!("fast subset check of {project} --task {task_id} (merge_fast_tests): {command}; the full check runs on main after the merge");
            let code = job::run_job_info(cfg, JobSpec { kind: "run".into(), name: project.to_string(), command: Some(command.clone()), first_step: 0, resumed_from: None, resume_count: 0, task: Some(task_id.to_string()) }, info);
            let failed = code != 0 && info.summary.first().is_some_and(|l| l.starts_with("run: FAILED"));
            let label = format!("check: fast subset ({}) = `{command}`, NOT the full suite: the full check runs on main right after the merge, and a failure there is reverted", tests.join(" "));
            (if failed { 1 } else { code }, label)
        }
    }
}

/// Did entry `id` end merged_check_failed with its automatic revert DONE and the check after it PASSED, its lane not held? Then
/// main is green again and `queue run --all` goes on with the next entry; anything else (no revert, a failed one, a held lane, an
/// entry that cannot be read) stops the drain as before (fail closed).
fn green_revert(cfg: &Config, id: &str) -> bool {
    load_entry(cfg, id).is_ok_and(|e| e.status() == MERGED_CHECK_FAILED && e.get("revert_check") == "passed" && lane_hold(cfg, e.get("project")).is_none())
}

/// An exit code that stops `queue run --all`: an internal error, or merged_check_failed unless its revert left main green.
fn stops_drain(cfg: &Config, id: &str, code: u8) -> bool {
    code == EXIT_ERROR || (code == EXIT_MERGED_BAD && !green_revert(cfg, id))
}

/// HOLD GATE, at the start of every `queue run` (one entry or `--all`), under the lane's run lock: for every held project of the lane, a
/// check of its main runs first; when it passes the hold is removed (the ONE place a hold file is ever deleted), otherwise the run merges
/// and runs nothing (exit 3) and says why.
fn hold_gate(cfg: &Config, lane: &Lane) -> Result<(), ExitCode> {
    let Ok(rd) = std::fs::read_dir(queue_dir(cfg)) else { return Ok(()) };
    let mut holds: Vec<PathBuf> = rd.flatten().map(|d| d.path()).filter(|p| p.is_file() && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("hold-"))).collect();
    holds.sort();
    let mut still: Vec<String> = Vec::new();
    for path in holds {
        let key = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix("hold-")).unwrap_or("").to_string();
        if lane.key.as_ref().is_some_and(|k| record::sanitize(k) != key) {
            continue;
        }
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                still.push(format!("lane {key} held: the hold file {} cannot be read ({e}); clear with a passing check of main, then queue run", path.display()));
                continue;
            }
        };
        let project = Some(hold_field(&text, "project")).filter(|p| !p.is_empty()).unwrap_or_else(|| key.clone());
        let reason = hold_field(&text, "reason");
        println!("lane {project} is held ({reason}); checking its main first");
        let checked = job::resolve_project(&cfg.root, &cfg.projects_dir, &project).and_then(|dir| main_check_now(cfg, &dir, &project));
        match checked {
            Ok(job) => match std::fs::remove_file(&path) {
                Ok(()) => println!("lane {project}: hold cleared: the check of main passed (job {job})"),
                Err(e) => still.push(format!("lane {project} held: {reason}; the check of main passed (job {job}) but the hold file could not be removed ({e})")),
            },
            Err(m) => {
                println!("lane {project}: the check of main did not pass ({m})");
                still.push(format!("lane {project} held: {reason}; clear with a passing check of main, then queue run"));
            }
        }
    }
    if still.is_empty() {
        return Ok(());
    }
    for s in &still {
        println!("{s}");
    }
    Err(ExitCode::from(EXIT_RETRY))
}

/// The lane key of a `--project` the caller typed: it must be a project (exit 2 otherwise, nothing run).
fn project_key(cfg: &Config, name: &str) -> Result<String, ExitCode> {
    job::canonical_project(&cfg.projects_dir, name).map(|c| c.to_lowercase()).map_err(|m| fail(EXIT_BAD_INPUT, format!("--project: {m}; nothing was run (see `broadsword projects`)")))
}

/// The run lock file of a lane key: `<project>.queue.lock` next to queue.lock. `sanitize` maps any other character to `_`, so two
/// names can only ever share a lock (stricter), never escape the queue folder.
fn lane_lock_file(key: &str) -> String {
    format!("{}.queue.lock", record::sanitize(key))
}

/// THE ONE PLACE a run lock is taken: every `queue run` (one entry or `--all`) and the cancel of a running entry come through here,
/// before the entries lock (order: run lock, then entries lock). `queue_per_project = "off"`: the one global queue.lock, exactly as
/// before (a `key` then only narrows which entries the run takes). `"on"`: the lane's own `<project>.queue.lock` (same OS lock, same
/// `queue_lock_wait_seconds`, exit 4 naming the project), so runs of different projects go at once; without a key it is refused (exit
/// 2), because a run of no named project cannot tell another project's live `running` entries from interrupted ones. A lock that
/// cannot be taken for any other reason fails closed (exit 1).
fn run_lock(cfg: &Config, key: Option<String>) -> Result<Lane, ExitCode> {
    if !cfg.queue_per_project {
        return Ok(Lane { _lock: queue_lock(cfg)?, key });
    }
    let Some(k) = key else {
        return Err(fail(EXIT_BAD_INPUT, "queue_per_project is on: every project has its own run lock, so name the project with `--project <p>` (or one entry with `--entry <id>`); nothing was run"));
    };
    let lock = take(cfg, &lane_lock_file(&k), &format!("another `queue run` of project `{k}` (or a cancel of one of its running entries)"))?;
    Ok(Lane { _lock: lock, key: Some(k) })
}

/// The lane of `queue run [--entry <id>] [--project <p>]`. `--project` must name a project, and an `--entry` given with it must be
/// of that project. With `--entry` alone and the setting on, the lane is the entry's own project (an entry's project never changes,
/// so reading it before the lock is safe); with the setting off, `--entry` alone is exactly as before (no lane key).
fn lane_for(cfg: &Config, entry: Option<&str>, project: Option<&str>) -> Result<Option<String>, ExitCode> {
    let of_entry = |id: &str| -> Result<String, ExitCode> {
        let e = load_entry(cfg, id).map_err(|m| fail(EXIT_BAD_INPUT, m))?;
        lane_key(cfg, e.get("project")).map_err(|m| fail(EXIT_BAD_INPUT, format!("queue entry {id}: its project `{}` cannot have a run lock: {m}; nothing was run", e.get("project"))))
    };
    match (entry, project) {
        (_, Some(p)) => {
            let k = project_key(cfg, p)?;
            if let Some(id) = entry {
                let ek = of_entry(id)?;
                if ek != k {
                    return Err(fail(EXIT_BAD_INPUT, format!("queue entry {id} is of project `{ek}`, not `{k}`; nothing was run")));
                }
            }
            Ok(Some(k))
        }
        (Some(id), None) if cfg.queue_per_project => Ok(Some(of_entry(id)?)),
        _ => Ok(None),
    }
}

fn entries_lock(cfg: &Config) -> Result<Lock, ExitCode> {
    take(cfg, "entries.lock", "another queue command")
}

fn when(r: &Record, k: &str) -> String {
    r.get_num(k).map(pause::format_utc).unwrap_or_else(|| "-".into())
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// The first few lines of `s`, for a one-line reason.
fn some(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    format!("{} path(s): {}{}", lines.len(), lines.iter().take(5).map(|l| l.trim_end()).collect::<Vec<_>>().join("; "), if lines.len() > 5 { "; ..." } else { "" })
}

/// Which kind of reviewer gates an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateBackend {
    /// The Claude CLI (`reviewer_backend`).
    Claude,
    /// An OpenAI model through clients/openai_review.py (same cap and ledger as the shadow review).
    Gpt,
}

/// The gating reviewer for a risk level.
#[derive(Debug, Clone)]
pub struct Gate {
    pub backend: GateBackend,
    pub model: String,
    /// The note on why (empty unless the Claude fallback was chosen up front).
    pub why: String,
    /// For a GPT gate: the Claude model that gates in the same run when the GPT call is SKIPPED.
    pub fallback: Option<String>,
}

/// The gating reviewer (backend and model) for a risk level. Low risk: `reviewer_model_low`. High risk: a coder of the
/// `reviewer_model_high` family (Opus-coded work) is gated by `reviewer_model_high_gpt` (unless that is `off`), with
/// `reviewer_model_high_fallback` as the Claude fallback for a skipped GPT call; any other coder is gated by `reviewer_model_high`.
/// Err names the problem when no allowed model is left.
pub fn gating_model(cfg: &Config, risk: &str, coder: &str) -> Result<Gate, String> {
    let claude = |m: &str, why: String| Gate { backend: GateBackend::Claude, model: m.to_string(), why, fallback: None };
    if risk != "high" {
        let m = cfg.reviewer_model_low.as_str();
        return if same_model(coder, m) { Err(format!("the reviewer model for low risk is `{m}`, the same as the coder (`{coder}`); a model never reviews its own code. Queue the task again with another --coder, or change reviewer_model_low in broadsword.toml")) } else { Ok(claude(m, String::new())) };
    }
    let (hi, fb) = (cfg.reviewer_model_high.as_str(), cfg.reviewer_model_high_fallback.as_str());
    if !same_model(coder, hi) {
        Ok(claude(hi, String::new()))
    } else if same_model(coder, fb) {
        Err(format!("reviewer_model_high `{hi}` and reviewer_model_high_fallback `{fb}` are both the same as the coder (`{coder}`); a model never reviews its own code. Queue the task again with another --coder, or change reviewer_model_high or reviewer_model_high_fallback in broadsword.toml"))
    } else if cfg.reviewer_model_high_gpt != "off" && reviewer::shadow_family(coder) != reviewer::shadow_family(&cfg.reviewer_model_high_gpt) {
        Ok(Gate { backend: GateBackend::Gpt, model: cfg.reviewer_model_high_gpt.clone(), why: format!("GPT gate; `{fb}` if the call is skipped"), fallback: Some(fb.to_string()) })
    } else {
        Ok(claude(fb, format!("fallback: coder is {}", reviewer::model_family(coder))))
    }
}

/// The `--effort` the gating call gets: `reviewer_effort_high` for `high` risk, none (empty) for `low`.
fn effort_for<'a>(cfg: &'a Config, risk: &str) -> &'a str {
    if risk == "high" {
        &cfg.reviewer_effort_high
    } else {
        ""
    }
}

/// Same model? Compared by family (`sonnet` and `claude-sonnet-4-5` are the same), case-insensitively.
fn same_model(a: &str, b: &str) -> bool {
    reviewer::model_family(a) == reviewer::model_family(b)
}

/// What a shadow review records in the entry (cleared on a fresh attempt).
/// How the GPT client is named in a review file and the entry.
const SHADOW_CLIENT_LABEL: &str = "clients/openai_review.py";

const SHADOW_KEYS: [&str; 9] = ["shadow_model", "shadow_model_used", "shadow_verdict", "shadow_reason", "shadow_blocking", "shadow_cost_usd", "shadow_tokens", "shadow_secs", "shadow_file"];

// ---- add --------------------------------------------------------------------------------

/// A folder of the config (`state_dir`, `build_dir`), resolved the way `canon_root` was: through the canonical root, and fully
/// canonical when it exists (a junction inside it then resolves too).
fn canon_inside(cfg: &Config, canon_root: &Path, dir: &Path) -> PathBuf {
    let joined = canon_root.join(dir.strip_prefix(&cfg.root).unwrap_or(dir));
    std::fs::canonicalize(&joined).unwrap_or(joined)
}

/// A brief file must be a small UTF-8 text file INSIDE the root (after canonicalising, so `..` and links count), not inside
/// `state_dir` (queue records, job logs) or `build_dir`, and not a secret file (`git::is_secret_path`, checked on the name given and
/// on the resolved name).
/// A brief is stored, shown and sent to a model, so it may never be any file the caller can read.
fn read_brief(cfg: &Config, path: &str) -> Result<String, String> {
    if crate::git::is_secret_path(path) {
        return Err(format!("`{path}` is a secret file (`.env`, `.envrc`, `.env.<name>`, `.env-*`, `.env_*` or a short name or stream of one); a brief is sent to a model and secrets must never be"));
    }
    let real = std::fs::canonicalize(path).map_err(|e| format!("cannot read the brief file `{path}`: {e}"))?;
    let canon_root = std::fs::canonicalize(&cfg.root).map_err(|e| format!("cannot resolve the project root: {e}"))?;
    if !real.starts_with(&canon_root) {
        return Err(format!("the brief `{path}` is outside the project root; a brief must be a file inside the root (it is sent to a model)"));
    }
    for (name, dir) in [("state_dir", &cfg.state_dir), ("build_dir", &cfg.build_dir)] {
        if real.starts_with(canon_inside(cfg, &canon_root, dir)) {
            return Err(format!("the brief `{path}` is inside {name}; queue records, job logs and build output are never briefs (they are sent to a model)"));
        }
    }
    if crate::git::is_secret_path(&real.to_string_lossy()) {
        return Err(format!("`{path}` resolves to a secret file ({}); a brief is sent to a model and secrets must never be", real.display()));
    }
    let p = real.as_path();
    let meta = std::fs::metadata(p).map_err(|e| format!("cannot read the brief file `{path}`: {e}"))?;
    let max = cfg.reviewer_brief_max_kb * 1024;
    if !meta.is_file() || meta.len() > max {
        return Err(format!("the brief `{path}` must be a file of at most {} KB (reviewer_brief_max_kb); it is {} bytes", cfg.reviewer_brief_max_kb, meta.len()));
    }
    let text = std::fs::read_to_string(p).map_err(|e| format!("the brief file `{path}` must be UTF-8 text: {e}"))?;
    if text.trim().is_empty() {
        return Err(format!("the brief file `{path}` is empty"));
    }
    Ok(text)
}

pub fn cmd_add(cfg: &Config, task_id: &str, risk: &str, coder: &str, brief_file: Option<&str>) -> ExitCode {
    if risk != "low" && risk != "high" {
        return fail(EXIT_BAD_INPUT, format!("--risk must be `low` or `high`, not `{risk}`"));
    }
    let coder = coder.trim();
    if coder.is_empty() || coder.chars().any(char::is_control) {
        return fail(EXIT_BAD_INPUT, "--coder must be the coder's model name on one line, e.g. sonnet");
    }
    // A coder whose family is the low-risk reviewer's has no allowed reviewer at low risk. Derived from `gating_model`, the one place
    // that picks reviewers, and checked before any lock is taken or any entry is written.
    if risk == "low" && gating_model(cfg, risk, coder).is_err() {
        return fail(EXIT_BAD_INPUT, format!("a {} coder has no allowed reviewer at --risk low; queue it with --risk high", reviewer::model_family(coder)));
    }
    let brief_text = match brief_file.map(|f| read_brief(cfg, f)) {
        None => None,
        Some(Ok(t)) => Some(t),
        Some(Err(e)) => return fail(EXIT_BAD_INPUT, e),
    };
    let t = match task::load_task(cfg, task_id) {
        Ok(t) => t,
        Err(e) => return fail(EXIT_BAD_INPUT, e),
    };
    let refuse = |why: String| fail(EXIT_RETRY, format!("cannot queue task {task_id}: {why}; nothing was queued"));
    let wt = match worktree::job_dir(cfg, task_id, t.get("project")) {
        Ok(p) => p,
        Err(JobDirError::Unknown(m) | JobDirError::Refused(m)) => return refuse(m),
    };
    let dir = match worktree::project_dir(cfg, &t) {
        Ok(d) => d,
        Err(e) => return refuse(e),
    };
    let branch = worktree::branch_of(&t);
    match wtgit::current_branch(&wt) {
        Ok(Some(b)) if b == branch => {}
        Ok(other) => return refuse(format!("its worktree is on {} instead of `{branch}`", other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into()))),
        Err(e) => return refuse(e),
    }
    // Tracked and untracked changes only: a rebase leaves ignored files (an ignored .env, target/) alone. The strict rule, ignored
    // files included, guards worktree REMOVAL at `task done`, which keeps the worktree instead of failing.
    match wtgit::status_porcelain_plain(&wt) {
        Ok(s) if s.is_empty() => {}
        Ok(s) => return refuse(format!("its worktree is not clean (uncommitted or untracked changes; ignored files do not count here): {}", some(&s))),
        Err(e) => return refuse(e),
    }
    let base = t.get("base").to_string();
    if base.is_empty() {
        return refuse("the task record has no base commit".into());
    }
    match wtgit::rev_count(&dir, &format!("{base}..{branch}")) {
        Ok(0) => return refuse(format!("`{branch}` has no commit beyond its base {}; commit the work first", short(&base))),
        Ok(_) => {}
        Err(e) => return refuse(e),
    }
    let brief = process_secrets(cfg).redact(&brief_text.unwrap_or_else(|| {
        format!(
            "Task {}: {}\nProject: {}. The task claims these files and folders: {}.\nNo separate done check was given: judge whether the diff does what the task title says, and flag changes outside the claims.",
            t.id(),
            t.get("title"),
            t.get("project"),
            task::claims_of(&t).join(", ")
        )
    }));
    let _lock = match entries_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let all = match load_entries(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, format!("cannot read the queue, so nothing was queued: {e}")),
    };
    if let Some(x) = all.iter().find(|x| x.get("task") == task_id && is_open(x.status())) {
        return fail(EXIT_RETRY, format!("task {task_id} is already in the queue as {} ({}); a task has one open entry at a time (queued, running, blocked, stale and approved entries are open). Run it with `queue run --entry {}`, or cancel it with `queue cancel {}`, then add again", x.id(), x.status(), x.id(), x.id()));
    }
    // LEFT-TRAIN RULE (see `Run::leave_train`): an earlier entry of this task left a merge train with its branch chained on other
    // entries' commits; while the branch still carries them and main does not, a new entry would review their code as this task's
    for x in all.iter().filter(|x| x.get("task") == task_id) {
        match chained_base(&dir, &branch, x) {
            Ok(None) => {}
            Ok(Some(chained_on)) => {
                let main = trunk_name(&dir);
                return refuse(format!("its earlier queue entry {} {}", x.id(), left_train_text(&main, &branch, &chained_on)));
            }
            Err(e) => return refuse(e),
        }
    }
    let mut e = match record::create_ext(&queue_dir(cfg), &format!("q-{}", job::file_stamp(now_secs())), EXT) {
        Ok(r) => r,
        Err(err) => return fail(EXIT_ERROR, format!("cannot create a queue entry: {err}")),
    };
    e.set("task", task_id);
    e.set("project", t.get("project"));
    e.set("branch", branch);
    e.set("risk", risk);
    e.set("coder", coder);
    e.set("brief", brief);
    e.set("brief_source", if brief_file.is_some() { "file" } else { "task title and claims" });
    e.set("status", QUEUED);
    e.set("attempt", "0");
    e.set("created", now_secs().to_string());
    if let Err(err) = e.save() {
        return fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display()));
    }
    println!("{}", e.id());
    println!("queued task {task_id} ({risk} risk, coder {coder}); `broadsword queue run` will review it with `{}` and merge it if it passes", gating_model(cfg, risk, coder).map_or_else(|_| "no allowed model (the entry will be blocked)".to_string(), |g| if g.why.is_empty() { g.model } else { format!("{} ({})", g.model, g.why) }));
    ExitCode::SUCCESS
}

// ---- list, show -------------------------------------------------------------------------

/// `running` entries whose runner is gone are shown as interrupted.
fn shown_status(e: &Record) -> String {
    if e.status() == RUNNING {
        let alive = e.get_num("pid").is_some_and(|p| sys::is_alive(p as u32, e.get_num("pid_start")));
        return if alive { "running".into() } else { "running (interrupted: `queue run` resumes it)".into() };
    }
    e.status().to_string()
}

pub fn cmd_list(cfg: &Config) -> ExitCode {
    let all = match load_entries(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    if all.is_empty() {
        println!("no queue entries");
    }
    for e in &all {
        let note = if e.get("reason").is_empty() { e.get("step") } else { e.get("reason") };
        println!("{}\t{}\t{}\t{}\t{}\t{}\t{}", e.id(), shown_status(e), e.get("task"), e.get("project"), e.get("risk"), e.get("coder"), note);
    }
    ExitCode::SUCCESS
}

pub fn cmd_show(cfg: &Config, id: &str) -> ExitCode {
    let e = match load_entry(cfg, id) {
        Ok(e) => e,
        Err(m) => return fail(EXIT_BAD_INPUT, m),
    };
    let dash = |k: &str| if e.get(k).is_empty() { "-".to_string() } else { e.get(k).to_string() };
    println!("id: {}", e.id());
    println!("status: {}", shown_status(&e));
    println!("reason: {}", dash("reason"));
    println!("task: {}", e.get("task"));
    println!("project: {}", e.get("project"));
    println!("branch: {}", e.get("branch"));
    println!("risk: {}", e.get("risk"));
    println!("coder: {}", e.get("coder"));
    println!("reviewer: {}", dash("reviewer_model"));
    println!("attempt: {}", dash("attempt"));
    println!("step: {}", dash("step"));
    println!("created: {}", when(&e, "created"));
    println!("started: {}", when(&e, "started"));
    println!("ended: {}", when(&e, "ended"));
    for (label, k) in [
        ("main branch", "main_branch"),
        ("base (main HEAD at rebase)", "base_head"),
        ("pre-rebase tip", "pre_rebase"),
        ("pre-rebase ref", "pre_rebase_ref"),
        ("rebased tip", "rebased_tip"),
        ("checked commit", "checked_tip"),
        ("reviewed commit", "reviewed_tip"),
        ("reviewer CLI", "reviewer_cli"),
        ("test hook", "test_hook"),
        ("review calls", "review_calls"),
        ("gate", "gate"),
        ("gate model used", "gate_model_used"),
        ("check job", "check_job"),
        ("check exit", "check_exit"),
        ("verdict", "verdict"),
        ("review cost (USD)", "cost_usd"),
        ("review tokens in", "tokens_in"),
        ("review tokens out", "tokens_out"),
        ("review file", "review_file"),
        ("shadow verdict", "shadow_verdict"),
        ("shadow reason", "shadow_reason"),
        ("shadow model", "shadow_model"),
        ("shadow model used", "shadow_model_used"),
        ("shadow cost (USD)", "shadow_cost_usd"),
        ("shadow tokens (in+out)", "shadow_tokens"),
        ("shadow seconds", "shadow_secs"),
        ("shadow file", "shadow_file"),
        ("main commit", "main_sha"),
        ("merge train", "train"),
        ("position in the train", "train_pos"),
        ("train check job (shared by the whole train)", "train_job"),
        ("train bisect", "train_bisect"),
        ("check deferred to the train", "check_deferred"),
        ("commit that was reviewed (before the train chained it)", "reviewed_orig_tip"),
        ("own commit before the train chained it", "alone_tip"),
        ("left a train chained on", "left_train_base"),
        ("train chain rebase begun onto (not recorded as finished)", "train_onto"),
        ("worktree detached for its own check at", "alone_detached"),
        ("main check", "main_check"),
        ("last error", "error"),
    ] {
        if !e.get(k).is_empty() {
            println!("{label}: {}", e.get(k));
        }
    }
    if !e.get("check_summary").is_empty() {
        println!("check summary:\n  {}", e.get("check_summary").replace('\n', "\n  "));
    }
    if !e.get("perf").is_empty() {
        println!("perf gate:\n  {}", e.get("perf").replace('\n', "\n  "));
    }
    if !e.get("done_check").is_empty() {
        println!("done check, run by the queue (state/queue/{}):\n  {}", e.get("done_check_file"), e.get("done_check").replace('\n', "\n  "));
    }
    for (label, k) in [("blocking findings", "blocking"),("minor findings", "minor"), ("shadow blocking findings (recorded only, never gating)", "shadow_blocking")] {
        if !e.get(k).is_empty() {
            println!("{label}:\n  {}", e.get(k).replace('\n', "\n  "));
        }
    }
    println!("brief ({}):\n  {}", e.get("brief_source"), e.get("brief").replace('\n', "\n  "));
    if !e.get("log").is_empty() {
        println!("steps:\n  {}", e.get("log").trim_end().replace('\n', "\n  "));
    }
    ExitCode::SUCCESS
}

// ---- cancel -----------------------------------------------------------------------------

/// CANCEL RULE, the ONE place: an entry whose commits may already be on main is never cancelled (it would leave main holding code
/// whose main check and `task done` never ran). Refused: a half-finished train merge (`half_merged`), and any entry whose reviewed
/// commit (or recorded main commit) is an ancestor of the project's trunk (`refs/heads/<trunk>`, never HEAD). When that cannot be decided (git or the project
/// fails), the cancel is refused too: fail closed. `None` = may be cancelled.
fn cancel_refusal(cfg: &Config, e: &Record) -> Option<String> {
    let id = e.id();
    if half_merged(e) {
        return Some(format!("queue entry {id} is a half-finished merge (a merge train took it into its merge at step `{}`): its commits may be on main already. Not cancelled; finish it with `broadsword queue run --all` or `broadsword queue run --entry {id}`", e.get("step")));
    }
    let mut commits: Vec<&str> = [e.get("reviewed_tip"), e.get("main_sha")].into_iter().filter(|c| !c.is_empty()).collect();
    commits.dedup();
    if commits.is_empty() {
        return None;
    }
    let on_main = (|| -> Result<Option<String>, String> {
        let dir = job::resolve_project(&cfg.root, &cfg.projects_dir, e.get("project"))?;
        let (_, head) = wtgit::trunk_commit(&dir)?;
        for c in &commits {
            if wtgit::is_ancestor(&dir, c, &head)? {
                return Ok(Some(c.to_string()));
            }
        }
        Ok(None)
    })();
    match on_main {
        Ok(None) => None,
        Ok(Some(c)) => Some(format!("queue entry {id}: its reviewed commit {} is already on main of project `{}`. Not cancelled (main would hold it with no main check and no `task done`); finish it with `broadsword queue run --entry {id}`", short(&c), e.get("project"))),
        Err(m) => Some(format!("queue entry {id}: cannot tell whether its reviewed commit is already on main ({m}); not cancelled")),
    }
}

pub fn cmd_cancel(cfg: &Config, id: &str) -> ExitCode {
    // A queued entry: only the entry changes, under the short lock. A running one: its project, for its run lock below.
    let project = {
        let _l = match entries_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        let mut e = match load_entry(cfg, id) {
            Ok(e) => e,
            Err(m) => return fail(EXIT_BAD_INPUT, m),
        };
        match e.status() {
            CANCELLED => {
                println!("queue entry {id} was already cancelled");
                return ExitCode::SUCCESS;
            }
            QUEUED | BLOCKED | STALE | APPROVED => {
                if let Some(why) = cancel_refusal(cfg, &e) {
                    return fail(EXIT_RETRY, why);
                }
                let was = e.status().to_string();
                e.set("status", CANCELLED);
                e.set("reason", format!("cancelled while {was}"));
                e.set("ended", now_secs().to_string());
                if let Err(err) = e.save() {
                    return fail(EXIT_ERROR, format!("cannot write the queue entry: {err}"));
                }
                println!("queue entry {id} is cancelled; nothing else was touched (task {} stays as it was)", e.get("task"));
                return ExitCode::SUCCESS;
            }
            RUNNING => e.get("project").to_string(), // below
            other => return fail(EXIT_RETRY, format!("queue entry {id} is `{other}` and cannot be cancelled (it is finished; `queue add` queues the task again if it needs another try)")),
        }
    };
    // A running entry can only be cancelled when nobody is running it: holding its run lock (`run_lock`: the global queue.lock, or
    // with queue_per_project its project's own) proves that. An entry's project never changes, so it was safe to read it first.
    // (with the setting off the key is not needed: the global lock guards every entry, exactly as before)
    let key = match cfg.queue_per_project.then(|| lane_key(cfg, &project)).transpose() {
        Ok(k) => k,
        Err(m) => return fail(EXIT_ERROR, format!("queue entry {id}: its project `{project}` cannot have a run lock ({m}); not cancelled")),
    };
    let _q = match run_lock(cfg, key) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let _l = match entries_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let mut e = match load_entry(cfg, id) {
        Ok(e) => e,
        Err(m) => return fail(EXIT_BAD_INPUT, m),
    };
    if e.status() != RUNNING {
        return fail(EXIT_RETRY, format!("queue entry {id} changed to `{}` while waiting; look at `queue show {id}`", e.status()));
    }
    let step = STEPS.iter().position(|s| *s == e.get("step")).unwrap_or(0);
    if step >= STEP_MERGE {
        return fail(EXIT_RETRY, format!("queue entry {id} was interrupted at step `{}`, at or after the merge: main may already hold the commit. Not cancelled; finish it with `broadsword queue run --entry {id}`", e.get("step")));
    }
    if let Some(why) = cancel_refusal(cfg, &e) {
        return fail(EXIT_RETRY, why);
    }
    let mut note = String::new();
    if let Ok(t) = task::load_task(cfg, e.get("task")) {
        if let (Ok(dir), true) = (worktree::project_dir(cfg, &t), worktree::has_worktree(&t)) {
            if let Ok(wt) = worktree::abs_path(cfg, t.get("worktree")) {
                if wt.exists() && wtgit::is_worktree_of(&dir, &wt) && wtgit::rebase_in_progress(&wt).unwrap_or(false) {
                    match wtgit::rebase_abort(&wt) {
                        Ok(()) => note = "; a rebase the dead run left half done was aborted".into(),
                        Err(m) => return fail(EXIT_ERROR, format!("queue entry {id}: the leftover rebase could not be aborted: {m}; the entry was not cancelled")),
                    }
                    let tip = wtgit::branch_tip(&dir, e.get("branch")).unwrap_or_default();
                    if !e.get("pre_rebase").is_empty() && tip != e.get("pre_rebase") {
                        return fail(EXIT_ERROR, format!("queue entry {id}: after the abort the branch is at {} but was at {} before the rebase; the entry was not cancelled; the old tip is also in {}", short(&tip), short(e.get("pre_rebase")), e.get("pre_rebase_ref")));
                    }
                }
            }
        }
    }
    e.set("status", CANCELLED);
    e.set("reason", format!("cancelled while interrupted at step `{}`", e.get("step")));
    e.set("ended", now_secs().to_string());
    if let Err(err) = e.save() {
        return fail(EXIT_ERROR, format!("cannot write the queue entry: {err}"));
    }
    println!("queue entry {id} is cancelled{note}; nothing else was touched");
    ExitCode::SUCCESS
}

// ---- run --------------------------------------------------------------------------------

/// `queue run [--entry <id>] [--project <p>]`: one entry. `project` limits the choice to that project's entries (see `run_lock` for
/// the lock it takes).
pub fn cmd_run(cfg: &Config, entry: Option<&str>, project: Option<&str>) -> ExitCode {
    let lane = match lane_for(cfg, entry, project).and_then(|k| run_lock(cfg, k)) {
        Ok(l) => l,
        Err(c) => return c,
    };
    if let Err(c) = hold_gate(cfg, &lane) {
        return c;
    }
    // Choose and claim the entry under the short lock. Holding the lane's run lock already means no other run of the lane exists,
    // so every `running` entry of the lane is an interrupted one.
    let chosen = claim(cfg, |all| match entry {
        Some(id) => match all.iter().find(|x| x.id() == id) {
            Some(x) if !lane.holds(x) => Err(fail(EXIT_BAD_INPUT, format!("queue entry {id} is of project `{}`, outside this run's project; nothing was run", x.get("project")))),
            Some(x) if matches!(x.status(), QUEUED | RUNNING | STALE | BLOCKED | APPROVED) => Ok(Some(x.clone())),
            Some(x) => Err(not_runnable(x)),
            None => Err(fail(EXIT_BAD_INPUT, format!("no queue entry with the exact id `{id}` (see `broadsword queue list`)"))),
        },
        None => {
            let mine: Vec<&Record> = all.iter().filter(|x| lane.holds(x)).collect();
            Ok(mine.iter().find(|x| resume_first(x)).or_else(|| mine.iter().find(|x| matches!(x.status(), QUEUED | STALE))).or_else(|| mine.iter().find(|x| x.status() == APPROVED)).map(|x| (*x).clone()))
        }
    });
    let e = match chosen {
        Ok(Some(e)) => e,
        Ok(None) => {
            println!("the queue is empty{}: nothing is queued, interrupted or stale", in_project(&lane));
            return ExitCode::SUCCESS;
        }
        Err(c) => return c,
    };
    match begin(cfg, e) {
        Ok(mut run) => run.go(),
        Err((_, code)) => code,
    }
}

/// ` for project <p>` when the lane is one project, for the "queue is empty" line.
fn in_project(lane: &Lane) -> String {
    lane.key.as_ref().map(|k| format!(" for project `{k}`")).unwrap_or_default()
}

/// `queue run` refuses a finished entry: exit 2 with the way on.
fn not_runnable(x: &Record) -> ExitCode {
    let hint = match x.status() {
        REJECTED | CHECK_FAILED | PERF_FAILED | CONFLICT => "fix the code, commit, and queue the task again with `queue add`",
        _ => "it is finished",
    };
    fail(EXIT_BAD_INPUT, format!("queue entry {} is `{}` and cannot be run; {hint}", x.id(), x.status()))
}

/// Under entries.lock: let `pick` choose an entry from all of them (oldest first) and CLAIM it: a fresh attempt (anything but an
/// interrupted `running` entry or an `approved` one) forgets what the last attempt learned (its log and review files stay) and becomes
/// `running`; the claimer's pid is recorded either way. The caller holds queue.lock.
fn claim(cfg: &Config, pick: impl FnOnce(&[Record]) -> Result<Option<Record>, ExitCode>) -> Result<Option<Record>, ExitCode> {
    let _l = entries_lock(cfg)?;
    let all = load_entries(cfg).map_err(|m| fail(EXIT_ERROR, m))?;
    let Some(mut e) = pick(&all)? else { return Ok(None) };
    // An `approved` entry (review PASSed, waiting for a merge train) that is run on its own is RESUMED at its step `review`, like an
    // interrupted one: its verdict stands, nothing is cleared (a deferred check is made before it can merge: see `merge_phase`).
    let resumed = matches!(e.status(), RUNNING | APPROVED);
    if e.status() == APPROVED {
        e.set("status", RUNNING);
    }
    if !resumed {
        // a fresh attempt: forget what the last attempt learned (its log and review file stay)
        for k in [
            "train", "train_pos", "train_job", "train_up", "train_onto", "train_checked", "train_bisect", "alone_tip", "reviewed_orig_tip", "check_deferred", "main_check", "step", "reason", "error", "pre_rebase", "pre_rebase_ref", "base_head", "rebased_tip", "checked_tip", "reviewed_tip", "reviewer_cli", "test_hook", "check_job", "check_exit", "check_summary", "check_tail", "perf", "done_check", "done_check_file",
            "reviewer_model", "gate", "gate_model_used", "verdict", "blocking", "minor", "cost_usd", "tokens_in", "tokens_out", "review_file", "main_sha", "main_branch", "ended", "pre_merge_sha", "revert_sha", "revert_check",
        ] {
            e.set(k, "");
        }
        for k in SHADOW_KEYS {
            e.set(k, "");
        }
        e.set("attempt", (e.get_num("attempt").unwrap_or(0) + 1).to_string());
        e.set("status", RUNNING);
    }
    let me = std::process::id();
    e.set("pid", me.to_string());
    e.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
    e.set("started", now_secs().to_string());
    e.save().map_err(|err| fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display())))?;
    Ok(Some(e))
}

/// A claimed entry, ready to run: its project resolved (an unknown project ends it `blocked`: `Err((id, exit code))`).
fn begin(cfg: &Config, mut e: Record) -> Result<Run<'_>, (String, ExitCode)> {
    let resumed = !e.get("step").is_empty();
    let dir = match job::resolve_project(&cfg.root, &cfg.projects_dir, e.get("project")) {
        Ok(d) => d,
        Err(m) => {
            let id = e.id().to_string();
            let mut run = Run::new(cfg, e, PathBuf::new());
            return Err((id, run.end(BLOCKED, &format!("the project is unknown: {m}"), EXIT_RETRY)));
        }
    };
    println!("queue entry {}: {} (task {}, project {}, risk {}, coder {}, attempt {})", e.id(), if resumed { "RESUMING after an interrupted run, from step ".to_string() + e.get("step") } else { "starting".into() }, e.get("task"), e.get("project"), e.get("risk"), e.get("coder"), e.get("attempt"));
    e.set("error", "");
    Ok(Run::new(cfg, e, dir))
}

// ---- run --all ----------------------------------------------------------------------------
//
// TP4 SEAM: this is the MINIMAL drain the merge train needs (tp4, t-20261118-095351, is not merged at this base). tp4's own
// `cmd_run_all` adds check-ahead (`queue_check_ahead`) and `--blocked`; when tp4 is ported onto this, it keeps the train hooks marked
// "MERGE TRAIN" below (`approved`, `run_trains`, `settle_approved`, the `train` column) and adds its own parts around them.

/// The exit code a `queue run` returned, as a number (ExitCode hides it).
fn code_num(c: ExitCode) -> u8 {
    (0..=u8::MAX).find(|n| ExitCode::from(*n) == c).unwrap_or(EXIT_ERROR)
}

/// How bad an outcome is, for "the exit code of `--all` is the worst outcome": merged_check_failed (6) is worst, then an internal
/// error (1), rejected / check_failed (5), blocked / conflict / stale (3), bad input (2), busy (4), merged (0).
fn badness(code: u8) -> u8 {
    match code {
        6 => 7,
        1 => 6,
        5 => 5,
        3 => 4,
        2 => 3,
        4 => 2,
        0 => 0,
        _ => 6,
    }
}

/// The next entry `--all` takes: an interrupted one first, then the oldest queued or stale one; never one this drain already took
/// (`taken`), so an entry that ends stale or blocked is not run again in a loop. An `approved` entry (review PASSed, not merged) is
/// picked last, and only when the merge train is off (`train` false): with the train on, the drain itself collects the approved
/// entries into trains (`take_group`).
fn next_for_all(all: &[Record], taken: &[String], train: bool, lane: &Lane) -> Option<Record> {
    let fresh = |x: &&Record| lane.holds(x) && !taken.iter().any(|t| t == x.id());
    all.iter()
        .filter(fresh)
        .find(|x| resume_first(x))
        .or_else(|| all.iter().filter(fresh).find(|x| matches!(x.status(), QUEUED | STALE)))
        .or_else(|| if train { None } else { all.iter().filter(fresh).find(|x| x.status() == APPROVED) })
        .cloned()
}

/// The exit code of `queue run --all`: the worst (see `badness`) of every row's code and of `stop`, the code that stopped the drain (0
/// when it ran to its end). A stop is counted even when no row carries it, so "the check could not give a verdict" (exit 1) is never
/// reported as the exit 3 of the rows it left `approved`.
fn worst_exit(rows: impl Iterator<Item = u8>, stop: u8) -> u8 {
    rows.chain(std::iter::once(stop)).max_by_key(|c| badness(*c)).unwrap_or(0)
}

/// `queue run --all`: every runnable entry, in order, under ONE hold of queue.lock (so no second runner can race it and lose with
/// exit 4): interrupted entries first, then queued and stale ones oldest first. Each entry is taken at most once per drain. An entry
/// that ends blocked, conflict, stale, rejected or check_failed does not stop the rest; an internal error (exit 1: the entry stays
/// `running`) and merged_check_failed (exit 6: main holds a commit whose check failed) STOP the drain, because what comes next would
/// build on a main nobody has looked at. Prints a summary table; the exit code is the worst outcome (see `badness`).
///
/// MERGE TRAIN (merge_train_max > 1): an entry stops at `approved` after its review PASSed (its own check deferred), and approved
/// entries are merged in trains (see `run_train`): whenever a full train is ready, whenever a project's short train is due
/// (`short_train_why`: none of its entries left to run, or its oldest approved entry waited merge_train_wait_minutes), and at the
/// end of the drain for whatever is left.
///
/// `project` (`--project <p>`): only that project's entries are drained (with queue_per_project on it is required, and the run holds
/// only that project's lock: see `run_lock`).
pub fn cmd_run_all(cfg: &Config, project: Option<&str>) -> ExitCode {
    let lane = match lane_for(cfg, None, project).and_then(|k| run_lock(cfg, k)) {
        Ok(l) => l,
        Err(c) => return c,
    };
    if let Err(c) = hold_gate(cfg, &lane) {
        return c;
    }
    let train = cfg.merge_train_max > 1;
    if !train && cfg.review_parallel > 1 {
        // PARALLEL REVIEW needs the merge train: without it each entry merges on the main it was rebased on, so a batch's later
        // entries would end stale after the first merge and be reviewed again (see `parallel_ok`, `s_merge`)
        println!("queue run --all: parallel reviews are off because the merge train is off (review_parallel = {}, merge_train_max = {}): entries are reviewed one at a time", cfg.review_parallel, cfg.merge_train_max);
    }
    let t0 = std::time::Instant::now();
    let mut taken: Vec<String> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut stopped = String::new();
    // the exit code that stopped the drain (1 or 6), which may be on no row (a train check without a verdict leaves its entries
    // `approved` with exit 3 rows): it counts for the drain's exit code (`worst_exit`)
    let mut stop_code = 0u8;
    let mut approved: Vec<(Run, std::time::Instant)> = Vec::new();
    if train {
        // entries an earlier drain approved and then died before merging them: they join the trains first (they are the oldest)
        let left: Vec<Record> = match claim_approved(cfg, &lane) {
            Ok(l) => l,
            Err(c) => return c,
        };
        for e in left {
            let t = std::time::Instant::now();
            taken.push(e.id().to_string());
            match begin(cfg, e) {
                Ok(mut r) => {
                    r.defer_check = true;
                    r.load_claims();
                    approved.push((r, t));
                }
                Err((id, c)) => rows.push(row_for(cfg, &id, code_num(c), t)),
            }
        }
    }
    loop {
        let e = match claim(cfg, |all| Ok(next_for_all(all, &taken, train, &lane))) {
            Ok(Some(e)) => e,
            Ok(None) => {
                // MERGE TRAIN: nothing else to run: every approved entry left (even a single one) goes into a last train
                if train && !approved.is_empty() {
                    let code = run_trains(cfg, &mut approved, true, &taken, &mut rows);
                    if code == EXIT_ERROR || code == EXIT_MERGED_BAD {
                        stopped = train_stop_note(code);
                        stop_code = code;
                    }
                    settle_approved(cfg, &mut approved, &mut rows);
                }
                break;
            }
            Err(c) => {
                stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
                rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
                break;
            }
        };
        taken.push(e.id().to_string());
        // PARALLEL REVIEW (review_parallel > 1, merge train on): more entries are claimed now, each exactly as the next turn of this
        // loop would claim it, as long as each is one a train member's worker may take (`parallel_ok`). With review_parallel = 1, or
        // when only one entry is ready, the batch is this one entry and everything below runs as it always did.
        let mut batch = vec![e];
        let mut claim_err = None;
        if train && cfg.review_parallel > 1 && parallel_ok(&batch[0]) {
            while (batch.len() as u64) < cfg.review_parallel {
                match claim(cfg, |all| Ok(next_for_all(all, &taken, train, &lane).filter(parallel_ok))) {
                    Ok(Some(x)) => {
                        taken.push(x.id().to_string());
                        batch.push(x);
                    }
                    Ok(None) => break,
                    Err(c) => {
                        claim_err = Some(c);
                        break;
                    }
                }
            }
        }
        let mut slots: Vec<Slot> = Vec::new();
        for e in batch {
            let t = std::time::Instant::now();
            match begin(cfg, e) {
                Ok(mut r) => {
                    r.defer_check = train;
                    let start = r.start_step();
                    slots.push(Slot::Run(r, t, start, 0));
                }
                Err((id, c)) => slots.push(Slot::Ended(id, code_num(c), t)),
            }
        }
        drive_all(&mut slots, train);
        // From here on everything is serial and in queue order, exactly as for one entry: the summary rows, the approved list, then
        // the trains (and so every train check, fast-forward and main check).
        let mut codes: Vec<(String, u8, usize)> = Vec::new();
        for slot in slots {
            match slot {
                Slot::Ended(id, code, t) => rows.push(row_for(cfg, &id, code, t)),
                Slot::Run(run, t, start, code) => {
                    let id = run.e.id().to_string();
                    if run.e.status() == APPROVED {
                        // approved entries keep their Run until their train has finished with them
                        approved.push((run, t));
                    } else {
                        rows.push(row_for(cfg, &id, code, t));
                    }
                    codes.push((id, code, start));
                }
            }
        }
        // a merged_check_failed whose automatic revert left main green (`green_revert`) does not stop the drain: the next entry is
        // rebased onto that green main, and the drain's exit code still counts the 6 (`worst_exit`)
        let Some(stop_at) = codes.iter().position(|(id, c, _)| stops_drain(cfg, id, *c)).or(if codes.is_empty() { None } else { Some(codes.len() - 1) }) else {
            if let Some(c) = claim_err {
                stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
                rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
                break;
            }
            continue; // every entry of the batch ended before it ran (unknown project): as before, no train for them
        };
        let (id, code, start) = codes[stop_at].clone();
        let mut train_code = 0u8;
        // no new train while a half-finished merge is being finished (`resume_first` entries come first): a train only starts once
        // main's last merge has had its main check
        let stop = stops_drain(cfg, &id, code);
        if train && start < STEP_MERGE && !approved.is_empty() && !stop {
            train_code = run_trains(cfg, &mut approved, false, &taken, &mut rows);
        }
        let hard = if stop { code } else { train_code };
        if hard == EXIT_ERROR || hard == EXIT_MERGED_BAD {
            stop_code = hard;
            stopped = if hard == train_code && train_code != 0 { train_stop_note(hard) } else { format!("stopped after entry {id} (exit {code}): {}", if code == EXIT_ERROR { "it stays `running` at its step; fix the cause, then `queue run` resumes it" } else { "main holds a commit whose check failed; a human decides before anything else merges" }) };
            settle_approved(cfg, &mut approved, &mut rows);
            break;
        }
        if let Some(c) = claim_err {
            stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
            rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
            break;
        }
    }
    let worst = worst_exit(rows.iter().map(|r| r.code), stop_code);
    if rows.is_empty() {
        println!("the queue is empty{}: nothing is queued, interrupted or stale", in_project(&lane));
    } else {
        println!("--- queue run --all{}: {} entr{} in {:.1} s ---", lane.key.as_ref().map(|k| format!(" --project {k}")).unwrap_or_default(), rows.len(), if rows.len() == 1 { "y" } else { "ies" }, t0.elapsed().as_secs_f64());
        // the train column only exists when a merge train handled an entry, so a drain without trains prints the plain table
        let trains = rows.iter().any(|r| !r.train.is_empty());
        println!("entry\ttask\tstatus\texit\tseconds\tnote{}", if trains { "\ttrain" } else { "" });
        for r in &rows {
            println!("{}\t{}\t{}\t{}\t{:.1}\t{}{}", r.id, r.task, r.status, r.code, r.secs, r.note, if trains { format!("\t{}", if r.train.is_empty() { "-" } else { r.train.as_str() }) } else { String::new() });
        }
        if !stopped.is_empty() {
            eprintln!("broadsword: queue run --all {stopped}");
        }
        println!("worst exit: {worst}");
    }
    ExitCode::from(worst)
}

// ---- PARALLEL REVIEW (review_parallel) ------------------------------------------------------------------------------
//
// `queue run --all` with the merge train on (merge_train_max > 1) and review_parallel > 1 claims up to review_parallel entries at
// once (each with the same `claim` and `next_for_all` the serial loop uses, so the same entries in the same order) and brings them
// to their review verdict together, one worker thread per entry (`drive_all`):
//   - each worker owns its entry's Run (its record and its files: <entry>.entry, .review, .shadow.review, .donecheck); no worker
//     ever reads or writes another entry's record, so a verdict can only be written to its own entry;
//   - the steps before the review (preconditions, rebase, the deferred check, the done check) run one worker at a time, in queue
//     order (`Turn`): git writes to the shared repository and done-check jobs never overlap, exactly as in the serial drain;
//   - the reviews (gate call, then shadow call, per entry, with the same rules, models, timeouts and reviewer_max_calls_per_entry)
//     run side by side, except that every call through clients/openai_review.py (GPT gate, GPT shadow) holds `GPT_CALL`, so the GPT
//     daily cap is checked call by call with every earlier call already in the ledger, as in the serial drain;
//   - a worker only ever ends its entry `approved` or finished (rejected, blocked, stale, ...): `parallel_ok` admits only entries
//     whose run starts before the merge step, and with the merge train on such a run stops at `approved` (`drive`). No worker
//     merges, builds a train, checks a train or fast-forwards: all of that runs afterwards, on the main thread, serially, in queue order.
// The run lock (queue.lock) is held by the drain for all of it; entries.lock is taken for each claim as before. A drain killed
// mid-review leaves each claimed entry `running` at its recorded step, and the next `queue run` resumes each one as it always does.

/// Can this entry be one of a parallel batch? Only an entry whose run starts before the merge step and that is not a half-finished
/// train merge: those (and everything after a review) run on the main thread only.
fn parallel_ok(e: &Record) -> bool {
    !half_merged(e) && STEPS.iter().position(|s| *s == e.get("step")).unwrap_or(0) < STEP_MERGE
}

/// One claimed entry of a batch: `Run(run, started, start step, exit code once driven)`, or `Ended(id, exit code, started)` for one
/// `begin` already ended (its project is unknown).
enum Slot<'a> {
    Run(Run<'a>, std::time::Instant, usize, u8),
    Ended(String, u8, std::time::Instant),
}

/// Every call through clients/openai_review.py (the GPT gate and the GPT shadow) of this process holds this lock, so parallel
/// reviews make their GPT calls one at a time: each call's daily-cap check sees every earlier call's ledger row.
static GPT_CALL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn gpt_call_lock() -> std::sync::MutexGuard<'static, ()> {
    GPT_CALL.lock().unwrap_or_else(|p| p.into_inner())
}

/// Turns in queue order: `wait(i)` returns once turns 0..i are over; the turn ends when the guard drops (also on an early return).
struct Turn {
    next: std::sync::Mutex<usize>,
    cv: std::sync::Condvar,
}

struct TurnGuard<'t>(&'t Turn);

impl Turn {
    fn wait(&self, i: usize) -> TurnGuard<'_> {
        let mut n = self.next.lock().unwrap_or_else(|p| p.into_inner());
        while *n != i {
            n = self.cv.wait(n).unwrap_or_else(|p| p.into_inner());
        }
        TurnGuard(self)
    }
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        *self.0.next.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.0.cv.notify_all();
    }
}

/// One entry through the steps up to its verdict and, without the merge train, its merge: exactly the serial drain's body. With
/// `turn`, the steps before the review wait for their turn (PARALLEL REVIEW).
fn drive(run: &mut Run<'_>, start: usize, train: bool, turn: Option<(&Turn, usize)>) -> u8 {
    let result = (|| -> R<ExitCode> {
        {
            let _t = turn.map(|(t, i)| t.wait(i));
            run.until_review(start)?;
        }
        run.review_phase(start)?;
        if train && start <= 3 {
            run.approve() // MERGE TRAIN: the entry waits at `approved` for its train
        } else {
            run.merge_phase(start)
        }
    })();
    code_num(match result {
        Ok(c) | Err(c) => c,
    })
}

/// Drive every `Slot::Run` of a batch and record its exit code: one alone on this thread (the serial drain), more side by side,
/// one worker thread each (PARALLEL REVIEW: they are all `parallel_ok` with the train on, so each stops at its verdict).
fn drive_all(slots: &mut [Slot<'_>], train: bool) {
    let runs = slots.iter().filter(|s| matches!(s, Slot::Run(..))).count();
    if runs <= 1 {
        for s in slots.iter_mut() {
            if let Slot::Run(run, _, start, code) = s {
                *code = drive(run, *start, train, None);
            }
        }
        return;
    }
    println!("queue run --all: PARALLEL REVIEW of {runs} entries (review_parallel): their steps up to the review run one after another in queue order, their reviews side by side; trains and merges follow, one at a time");
    let turn = Turn { next: std::sync::Mutex::new(0), cv: std::sync::Condvar::new() };
    std::thread::scope(|sc| {
        let mut i = 0;
        for s in slots.iter_mut() {
            if let Slot::Run(run, _, start, code) = s {
                let (turn, at, start) = (&turn, i, *start);
                i += 1;
                sc.spawn(move || {
                    *code = drive(run, start, train, Some((turn, at)));
                });
            }
        }
    });
}

/// A summary row read back from the entry's record (its final status and one-line reason).
fn row_for(cfg: &Config, id: &str, code: u8, t: std::time::Instant) -> Row {
    let e = load_entry(cfg, id).ok();
    let get = |k: &str| e.as_ref().map(|e| e.get(k).to_string()).unwrap_or_default();
    let mut note = get("reason");
    if note.chars().count() > 120 {
        note = note.chars().take(117).collect::<String>() + "...";
    }
    let train = match (get("train"), get("train_pos")) {
        (t, p) if !t.is_empty() => format!("{t}#{p}"),
        _ => String::new(),
    };
    Row { id: id.to_string(), task: get("task"), status: get("status"), code, secs: t.elapsed().as_secs_f64(), note, train }
}

impl<'a> Run<'a> {
    fn go(&mut self) -> ExitCode {
        match self.pipeline() {
            Ok(code) | Err(code) => code,
        }
    }

    /// The step to start from: the recorded one (a resumed run), else the first.
    fn start_step(&self) -> usize {
        STEPS.iter().position(|s| *s == self.e.get("step")).unwrap_or(0)
    }

    fn pipeline(&mut self) -> R<ExitCode> {
        let start = self.start_step();
        self.until_review(start)?;
        self.review_phase(start)?;
        self.merge_phase(start)
    }

    /// Steps 1 to 3: preconditions, rebase, check (each only when `start` is at or before it).
    fn until_review(&mut self, start: usize) -> R<()> {
        if start < STEP_MERGE {
            self.s_preconditions(start)?;
        }
        if start <= 1 {
            self.s_rebase()?;
        }
        if start <= 2 {
            self.check_step()?;
        }
        Ok(())
    }

    /// The check step: the real check, or (merge train mode) the deferral that leaves the check to the train.
    fn check_step(&mut self) -> R<()> {
        if self.defer_check {
            self.s_defer_check()?;
        } else {
            self.s_check()?;
        }
        self.s_done_check()
    }

    /// Step 4, review.
    fn review_phase(&mut self, start: usize) -> R<()> {
        if start <= 3 {
            self.s_review()?;
        }
        Ok(())
    }

    /// Steps 5 to 7: merge, main check, done. An entry whose check was deferred to a merge train that never checked it (a train that
    /// died, or `queue run` of an `approved` entry on its own) is checked alone first: nothing unchecked ever merges.
    fn merge_phase(&mut self, start: usize) -> R<ExitCode> {
        if start <= STEP_MERGE {
            // a commit already on main (a train that died after its fast-forward) is not checked alone: it is merged, and the main
            // check below checks the main that holds it
            if self.e.get("check_deferred") == "yes" && self.e.get("train_checked") != self.e.get("reviewed_tip") && !self.reviewed_on_main()? {
                self.log("merge", "this entry's check was deferred to a merge train that has not checked this commit: it is checked on its own before it can merge")?;
                self.s_check()?;
            }
            self.s_merge()?;
        }
        if start <= 5 {
            self.s_main_check()?;
        }
        self.s_done()
    }

    /// Is the reviewed commit already on the trunk (`refs/heads/<trunk>`: equal to it or an ancestor of it)? False when none is
    /// recorded. A trunk that cannot be told ends the entry `blocked` (`trunk_tip`).
    fn reviewed_on_main(&mut self) -> R<bool> {
        let reviewed = self.e.get("reviewed_tip").to_string();
        if reviewed.is_empty() {
            return Ok(false);
        }
        let dir = self.dir.clone();
        let head = match trunk_tip(&dir, self.e.get("main_branch")) {
            Ok((_, tip)) => tip,
            Err(m) => return Err(self.blocked(&format!("{m}; nothing merged"))),
        };
        self.g(wtgit::is_ancestor(&dir, &reviewed, &head))
    }

    /// `trunk_checkout` for this entry (its recorded trunk): the trunk's tip, or the entry ends `blocked` with the reason.
    fn on_trunk(&mut self) -> R<String> {
        match trunk_checkout(&self.dir, self.e.get("main_branch")) {
            Ok((_, tip)) => Ok(tip),
            Err(m) => Err(self.blocked(&format!("{m}; nothing merged"))),
        }
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

    /// The claims of this entry's task, as a train compares them. A task that cannot be read claims everything (`*`), so it never
    /// shares a train.
    fn load_claims(&mut self) {
        self.claims = match task::load_task(self.cfg, self.e.get("task")) {
            Ok(t) => task::claims_of(&t),
            Err(_) => vec!["*".to_string()],
        };
    }

    /// MERGE TRAIN: the gating review PASSed. The entry stops here as `approved`: nothing is merged, the task stays running, the
    /// worktree is untouched. `run_train` merges it with the other approved entries of its project.
    fn approve(&mut self) -> R<ExitCode> {
        self.load_claims();
        let base = self.e.get("base_head").to_string();
        self.e.set("train_up", base);
        self.e.set("status", APPROVED);
        // when it started to wait: the merge_train_wait_minutes rule (`short_train_why`) measures from here
        self.e.set("approved_at", now_secs().to_string());
        self.e.set("reason", "the review PASSed; it waits for a merge train (nothing is merged yet)");
        println!("queue entry {}: approved (the review PASSed); it waits for a merge train", self.e.id());
        self.log("review", "approved: the entry joins the next merge train of its project (claims that do not overlap, at most merge_train_max entries)")?;
        Ok(ExitCode::SUCCESS)
    }

    /// Before an approved entry joins a train: still exactly the reviewed commit, the worktree on its branch and clean, clean main on
    /// the same branch. A chain rebase that a killed drain left half done is aborted first (the branch is then back at its
    /// `pre_rebase` tip); any OTHER rebase in progress is the coder's own and is never aborted: the entry ends `blocked`.
    fn train_ready(&mut self) -> R<()> {
        self.locate()?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        if self.g(wtgit::rebase_in_progress(&wt))? {
            if self.own_chain_rebase()? {
                self.abort_stale_rebase()?;
            } else {
                return Err(self.leave_train(BLOCKED, "a rebase is in progress in the task's worktree that no merge train started (the coder's own, begun after the review); it was left alone. Finish it or abort it (`git rebase --abort`), then run the entry again; nothing merged", EXIT_RETRY));
            }
        }
        // `train_link` rebases whatever the worktree has checked out: it must be the task's own branch (as in `s_preconditions`)
        match self.g(wtgit::current_branch(&wt))? {
            Some(b) if b == branch => {}
            other => {
                let on = other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into());
                return Err(self.leave_train(BLOCKED, &format!("the task's worktree is on {on}, not `{branch}`; it cannot join a merge train (its chain rebase would move what is checked out there); nothing merged. Check out `{branch}` again, then run the entry"), EXIT_RETRY));
            }
        }
        let (reviewed, checked) = (self.e.get("reviewed_tip").to_string(), self.e.get("checked_tip").to_string());
        if self.e.get("verdict") != "PASS" || reviewed.is_empty() || reviewed != checked {
            return Err(self.leave_train(STALE, &format!("an approved entry without a recorded PASS for its checked commit (reviewed `{}`, checked `{}`) cannot join a merge train. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&checked), self.e.id()), EXIT_RETRY));
        }
        if let Some(why) = self.test_hook_refusal() {
            return Err(self.leave_train(BLOCKED, &why, EXIT_RETRY));
        }
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if tip != reviewed {
            return Err(self.leave_train(STALE, &format!("branch changed after the review: `{branch}` was reviewed at {} but is at {} now; nothing was merged. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let wt_status = self.g(wtgit::status_porcelain_plain(&wt))?;
        if !wt_status.is_empty() {
            return Err(self.leave_train(BLOCKED, &format!("the task's worktree got changes after the review ({}); it cannot join a merge train; nothing merged", some(&wt_status)), EXIT_RETRY));
        }
        if let Err(m) = trunk_checkout(&dir, self.e.get("main_branch")) {
            return Err(self.leave_train(BLOCKED, &format!("{m}; nothing merged"), EXIT_RETRY));
        }
        let dirty = self.g(wtgit::status_porcelain_plain(&dir))?;
        if !dirty.is_empty() {
            return Err(self.leave_train(BLOCKED, &format!("the main checkout has uncommitted or untracked changes ({}); commit or discard them, then run again; nothing merged", some(&dirty)), EXIT_RETRY));
        }
        if self.e.get("alone_tip").is_empty() {
            self.e.set("alone_tip", reviewed);
            self.save()?;
        }
        Ok(())
    }

    /// Put this entry's own commits (everything after `train_up`) on top of `onto`, the previous entry of the chain (or main's HEAD
    /// for the first). `Some(new tip)`; `None` when the entry cannot ride in this train and went back to `queued`: git could not apply
    /// its commits (its branch exactly as it was), or the rebased commits are not the same patches as the reviewed ones. The old tip is
    /// saved under refs/broadsword/queue/ first, as for every rebase.
    ///
    /// THE REVIEW BINDS ITS COMMITS: the review and the merge follow the entry to its new commit only when `git patch-id --stable` of
    /// the rebased commits (`onto..new tip`) equals that of the commits that were reviewed (`base_head..reviewed_orig_tip`), commit for
    /// commit. Anything else (a hunk whose context the entries before it changed) sends the entry back to `queued` ("patch changed by
    /// the train rebase"): it is reviewed again and never merges on the old verdict.
    fn train_link(&mut self, onto: &str, train_id: &str) -> R<Option<String>> {
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        let up = if self.e.get("train_up").is_empty() { self.e.get("base_head").to_string() } else { self.e.get("train_up").to_string() };
        let orig = if self.e.get("reviewed_orig_tip").is_empty() { self.e.get("reviewed_tip").to_string() } else { self.e.get("reviewed_orig_tip").to_string() };
        let base = self.e.get("base_head").to_string();
        let reviewed_ids = self.g(wtgit::patch_ids(&dir, &format!("{base}..{orig}")))?;
        self.e.set("train", train_id);
        let r = self.keep_pre_rebase(&tip)?;
        self.e.set("pre_rebase", tip.clone());
        self.e.set("pre_rebase_ref", r.clone());
        // KILL WINDOW: `train_onto` is SAVED (by the log line below) before the rebase starts and cleared only in the save that
        // records the result. A drain killed after the rebase moved the branch but before that save leaves it set, so the left-train
        // rule (`chained_base`) still sees the branch as chained on `onto`, and `train_ready` knows a rebase in progress is its own.
        self.e.set("train_onto", onto);
        self.log("rebase", &format!("merge train {train_id}: chaining `{branch}` (tip {}) onto {}; its own commits start after {}; the old tip is saved in {r}", short(&tip), short(onto), short(&up)))?;
        match self.g(wtgit::rebase_onto(&wt, onto, &up))? {
            wtgit::Rebase::Done => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if !self.g(wtgit::is_ancestor(&dir, onto, &now))? {
                    return Err(self.internal(&format!("the chain rebase reported success but `{branch}` ({}) does not contain {}; the old tip {} is saved in {r}", short(&now), short(onto), short(&tip))));
                }
                if self.g(wtgit::rev_count(&dir, &format!("{onto}..{now}")))? == 0 {
                    self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                    return Err(self.leave_train(BLOCKED, &format!("after chaining onto the train `{branch}` has no commit left (its changes are already in the train); the old tip {} is saved in {r}", short(&tip)), EXIT_RETRY));
                }
                let now_ids = self.g(wtgit::patch_ids(&dir, &format!("{onto}..{now}")))?;
                if now_ids != reviewed_ids {
                    self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                    self.e.set("rebased_tip", now.clone());
                    self.requeue(&format!("patch changed by the train rebase: merge train {train_id} chained `{branch}` onto the entries before it, and its rebased commits {} are not the same patches (git patch-id --stable) as the reviewed commits {}..{}; the old verdict does not cover them, so the entry goes back to `queued` and is reviewed again (the pre-train tip is saved in {r})", short(&now), short(&base), short(&orig)));
                    return Ok(None);
                }
                if self.e.get("reviewed_orig_tip").is_empty() {
                    self.e.set("reviewed_orig_tip", orig.clone());
                }
                self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                self.e.set("rebased_tip", now.clone());
                self.e.set("checked_tip", now.clone());
                self.e.set("reviewed_tip", now.clone());
                self.e.set("check_deferred", "yes");
                self.e.set("train_checked", "");
                self.save()?;
                self.log("rebase", &format!("chained: tip {} -> {}; the same {} patch(es) as reviewed (git patch-id --stable)", short(&tip), short(&now), now_ids.len()))?;
                Ok(Some(now))
            }
            wtgit::Rebase::Conflict(files) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("the chain rebase conflicted and was aborted, but `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                self.e.set("train_onto", ""); // the branch is exactly as before: it was never chained onto `onto`
                self.requeue(&format!("merge train {train_id}: chaining `{branch}` onto the entries before it conflicts in: {}. The rebase was aborted and the branch is exactly as before; the entry goes back to `queued` and is reviewed again on its own", files.join(", ")));
                Ok(None)
            }
            wtgit::Rebase::Failed(m) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("{m}; and `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                self.e.set("train_onto", "");
                self.requeue(&format!("merge train {train_id}: `{branch}` could not be chained onto the entries before it: {m}. The branch is unchanged; the entry goes back to `queued`"));
                Ok(None)
            }
        }
    }

    /// LEFT-TRAIN RULE for a run outside a train (`s_preconditions`, `s_merge`): an entry whose branch is still chained on other
    /// entries' commits that are not on main is never reviewed or merged on that diff; it ends `blocked` (`leave_train`). Once the
    /// branch no longer carries that commit (the coder rebased, or those entries reached main) the record forgets it and the run goes on.
    fn refuse_chained(&mut self, step: &str) -> R<()> {
        let (dir, branch) = (self.dir.clone(), self.branch());
        if self.g(chained_base(&dir, &branch, &self.e))?.is_some() {
            return Err(self.leave_train(BLOCKED, "it was run while its branch still carries those entries' commits; nothing was reviewed or merged", EXIT_RETRY));
        }
        let base = chain_bases_of(&self.e).into_iter().next().unwrap_or_default();
        let had = !self.e.get("left_train_base").is_empty() || !self.e.get("train_onto").is_empty();
        if had {
            self.e.set("left_train_base", "");
            self.e.set("train_onto", "");
            self.log(step, &format!("the branch no longer carries {}, the commit a merge train chained it on that is not on main: it runs as it is", short(&base)))?;
        }
        Ok(())
    }

    /// An entry a train cannot take goes back to `queued` with the reason; the next `queue run` checks and reviews it afresh. Unless
    /// its branch is chained on other entries (see `leave_train`): then it ends `blocked`.
    /// Both outcomes are exit 3 (the caller records that); a record that cannot be written has printed its own error.
    fn requeue(&mut self, reason: &str) {
        let _ = self.leave_train(QUEUED, reason, EXIT_RETRY);
    }

    /// LEFT-TRAIN RULE, the ONE place it is applied: every way an entry leaves a merge train without merging (back to queued,
    /// check_failed, conflict, stale, blocked) ends here, and so does a solo run of an entry still chained (`s_preconditions`,
    /// `s_merge`). An entry whose branch still carries commits of other entries that are not on main (`chained_onto`) never goes on to
    /// a review or merge of that chained diff: it ends `blocked` ("left a train: ..."), and `left_train_base` keeps the commit it is
    /// chained on across attempts, so a later `queue run` or `queue add` refuses it too until the coder has rebased its own commits
    /// onto main (or those entries reached main). Otherwise the entry gets `status` as asked. A git error counts as chained.
    fn leave_train(&mut self, status: &str, reason: &str, code: u8) -> ExitCode {
        let (dir, branch) = (self.dir.clone(), self.branch());
        let base = match chained_base(&dir, &branch, &self.e) {
            Ok(b) => b,
            // a git error counts as chained, on the first recorded base (an error needs one: no base is never chained)
            Err(_) => Some(chain_bases_of(&self.e).into_iter().next().unwrap_or_default()),
        };
        let Some(base) = base else {
            if status == QUEUED {
                let reason = self.secrets.redact(reason);
                self.e.set("status", QUEUED);
                self.e.set("reason", reason.clone());
                let _ = self.log("train", &format!("back to queued: {reason}"));
                eprintln!("broadsword: queue entry {} goes back to `queued`: {reason}", self.e.id());
                return ExitCode::from(EXIT_RETRY);
            }
            return self.end(status, reason, code);
        };
        let main = if self.e.get("main_branch").is_empty() { trunk_name(&dir) } else { self.e.get("main_branch").to_string() };
        self.e.set("left_train_base", base.clone());
        let text = left_train_text(&main, &branch, &base);
        self.blocked(&format!("{text}. Why it left (it would have been `{status}`): {reason}"))
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
    fn check_commit(&mut self, commit: &str, what: &str) -> TrainCheck {
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
    fn check_alone(&mut self) -> TrainCheck {
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

    // -- step 1: preconditions --------------------------------------------------------------

    /// Find the task's worktree (and prove it is the task's), or end blocked.
    fn locate(&mut self) -> R<()> {
        match worktree::job_dir(self.cfg, self.e.get("task"), self.e.get("project")) {
            Ok(p) => {
                self.wt = p;
                self.restore_alone_checkout()
            }
            Err(JobDirError::Unknown(m) | JobDirError::Refused(m)) => Err(self.blocked(&m)),
        }
    }

    /// A run killed inside `check_alone` left the task's worktree detached on the entry's own commit (`alone_detached` records it).
    /// Every run that uses the worktree passes `locate`, which calls this: the worktree goes back on its branch, and only when it is
    /// exactly where the queue put it (detached at that commit); a worktree the coder moved since is never touched (the entry ends
    /// `blocked`, with the command to run by hand). Nothing recorded: nothing done.
    fn restore_alone_checkout(&mut self) -> R<()> {
        let at = self.e.get("alone_detached").to_string();
        if at.is_empty() {
            return Ok(());
        }
        let (wt, branch) = (self.wt.clone(), self.branch());
        match self.g(wtgit::current_branch(&wt))? {
            Some(b) if b == branch => {}
            Some(other) => return Err(self.blocked(&format!("a killed merge-train check left a note that the worktree was detached at {}, but it is on branch `{other}` now, not `{branch}`; it was not touched. Put it back with `git -C {} checkout {branch}`", short(&at), wt.display()))),
            None => {
                let head = self.g(wtgit::head_commit(&wt))?;
                if head != at {
                    return Err(self.blocked(&format!("the worktree is detached at {}, not at {} where a killed merge-train check put it; it was not touched. Put it back with `git -C {} checkout {branch}`", short(&head), short(&at), wt.display())));
                }
                if let Err(m) = wtgit::checkout_branch(&wt, &branch) {
                    return Err(self.blocked(&format!("a killed merge-train check left the worktree detached at {}, and it could not be put back on `{branch}`: {m}. Do it by hand: git -C {} checkout {branch}", short(&at), wt.display())));
                }
                self.e.set("alone_detached", "");
                return self.log("train", &format!("a killed merge-train check had left the worktree detached on the entry's own commit {}: put it back on `{branch}`", short(&at)));
            }
        }
        self.e.set("alone_detached", "");
        self.save()
    }

    fn s_preconditions(&mut self, start: usize) -> R<()> {
        self.set_step("preconditions")?;
        self.locate()?;
        // A run that died in the middle of ITS rebase (the recorded step is `rebase`) left the worktree half-rebased: put it back
        // before judging anything. A fresh attempt never aborts a rebase: one in progress is the coder's own and blocks as dirty.
        if start == 1 {
            self.abort_stale_rebase()?;
        }
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        if !git::is_repo(&dir) {
            return Err(self.blocked(&format!("project `{}` is not a Git repo", self.e.get("project"))));
        }
        // the entry's `main branch` is the project's TRUNK (never what HEAD happens to be), and it must be what is checked out
        let recorded = if start == 0 { String::new() } else { self.e.get("main_branch").to_string() };
        let (main_branch, trunk_at) = match trunk_checkout(&dir, &recorded) {
            Ok(t) => t,
            Err(m) => return Err(self.blocked(&format!("{m}; nothing merged"))),
        };
        self.e.set("main_branch", main_branch.clone());
        let dirty = self.g(wtgit::status_porcelain_plain(&dir))?;
        if !dirty.is_empty() {
            return Err(self.blocked(&format!("the main checkout has uncommitted or untracked changes ({}); commit or discard them, then run again", some(&dirty))));
        }
        match self.g(wtgit::current_branch(&wt))? {
            Some(b) if b == branch => {}
            other => {
                let on = other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into());
                return Err(self.blocked(&format!("the task's worktree is on {on}, not `{branch}`")));
            }
        }
        // tracked and untracked only: a rebase leaves ignored files alone (see `cmd_add`)
        let wt_status = self.g(wtgit::status_porcelain_plain(&wt))?;
        if !wt_status.is_empty() {
            return Err(self.blocked(&format!("the task's worktree is not clean ({}); commit or remove those changes, then run again (ignored files do not matter here)", some(&wt_status))));
        }
        self.refuse_chained("preconditions")?;
        let coder = self.e.get("coder").to_string();
        let (model, why) = match gating_model(self.cfg, self.e.get("risk"), &coder) {
            Ok(g) => (g.model, if g.why.is_empty() { String::new() } else { format!(" ({})", g.why) }),
            Err(msg) => return Err(self.blocked(&msg)),
        };
        let ahead = self.g(wtgit::rev_count(&dir, &format!("{trunk_at}..{branch}")))?;
        if ahead == 0 {
            return Err(self.blocked(&format!("nothing to merge: `{branch}` has no commit that `{main_branch}` lacks")));
        }
        self.log("preconditions", &format!("ok: main `{main_branch}` clean, worktree clean, task running, {ahead} commit(s) to merge, reviewer `{model}`{why}, effort `{}`, differs from coder `{coder}`", effort_for(self.cfg, self.e.get("risk"))))
    }

    /// Is the rebase in progress in the task's worktree the chain rebase a killed train left (and so the queue's to abort)? Only when
    /// all hold: `train_link` recorded that it began one onto `train_onto` and never finished it, the branch ref is still at the
    /// recorded `pre_rebase` tip (a rebase moves the ref only when it completes), and the worktree's HEAD is on top of `train_onto`
    /// (where that rebase replays the commits). Anything else is the coder's own rebase. A git error answers no (it is not aborted).
    fn own_chain_rebase(&mut self) -> R<bool> {
        let (onto, before) = (self.e.get("train_onto").to_string(), self.e.get("pre_rebase").to_string());
        if onto.is_empty() || before.is_empty() {
            return Ok(false);
        }
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        if self.g(wtgit::branch_tip(&dir, &branch))? != before {
            return Ok(false);
        }
        Ok(wtgit::is_ancestor(&wt, &onto, "HEAD").unwrap_or(false))
    }

    fn abort_stale_rebase(&mut self) -> R<()> {
        let wt = self.wt.clone();
        if self.g(wtgit::rebase_in_progress(&wt))? {
            self.g(wtgit::rebase_abort(&wt))?;
            let (tip, before) = (self.g(wtgit::branch_tip(&self.dir.clone(), &self.branch()))?, self.e.get("pre_rebase").to_string());
            if !before.is_empty() && tip != before {
                return Err(self.internal(&format!("after aborting the rebase an interrupted run left, the branch is at {} but was at {} before it; the old tip is saved in {}", short(&tip), short(&before), self.e.get("pre_rebase_ref"))));
            }
            self.log("rebase", "found a rebase left half done by an interrupted run and aborted it; the branch is back where it was")?;
        }
        Ok(())
    }

    // -- step 2: rebase ---------------------------------------------------------------------

    /// Save `tip` under refs/broadsword/queue/<id>/pre-rebase[-N], create-only (an existing ref at another commit is never
    /// moved: a numbered or suffixed name is used instead). Returns the ref name.
    fn keep_pre_rebase(&mut self, tip: &str) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let leaf = if attempt <= 1 { "pre-rebase".to_string() } else { format!("pre-rebase-{attempt}") };
        let name = format!("refs/broadsword/queue/{}/{leaf}", self.e.id());
        let dir = self.dir.clone();
        if wtgit::keep_ref(&dir, &name, tip).is_ok() {
            return Ok(name);
        }
        let alt = format!("{name}-{}", &tip[..tip.len().min(12)]);
        self.g(wtgit::keep_ref(&dir, &alt, tip).map(|_| alt))
    }

    fn s_rebase(&mut self) -> R<()> {
        self.set_step("rebase")?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let head = self.on_trunk()?;
        self.e.set("base_head", head.clone());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if self.g(wtgit::is_ancestor(&dir, &head, &tip))? {
            self.e.set("rebased_tip", tip.clone());
            return self.log("rebase", &format!("`{branch}` already contains main's HEAD {}: no rebase needed (tip {})", short(&head), short(&tip)));
        }
        self.e.set("pre_rebase", tip.clone());
        let r = self.keep_pre_rebase(&tip)?;
        self.e.set("pre_rebase_ref", r.clone());
        self.log("rebase", &format!("rebasing `{branch}` (tip {}) onto main's HEAD {}; the old tip is saved in {r}", short(&tip), short(&head)))?;
        match self.g(wtgit::rebase(&wt, &head))? {
            wtgit::Rebase::Done => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if !self.g(wtgit::is_ancestor(&dir, &head, &now))? {
                    return Err(self.internal(&format!("the rebase reported success but `{branch}` ({}) does not contain main's HEAD {}; the old tip {} is saved in {r}", short(&now), short(&head), short(&tip))));
                }
                if self.g(wtgit::rev_count(&dir, &format!("{head}..{now}")))? == 0 {
                    return Err(self.blocked(&format!("after the rebase `{branch}` has no commit that main lacks (its changes are already in main); the old tip {} is saved in {r}", short(&tip))));
                }
                self.e.set("rebased_tip", now.clone());
                self.log("rebase", &format!("rebased: tip {} -> {}", short(&tip), short(&now)))
            }
            wtgit::Rebase::Conflict(files) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("the rebase conflicted and was aborted, but `{branch}` is at {} instead of {}: the old tip is saved in {r}; look at the worktree", short(&now), short(&tip))));
                }
                Err(self.end(CONFLICT, &format!("rebasing `{branch}` onto main {} conflicts in: {}. The rebase was aborted and the branch is exactly as before (tip {}). Rebase or fix it yourself, commit, and queue it again with `queue add`", short(&head), files.join(", "), short(&tip)), EXIT_RETRY))
            }
            wtgit::Rebase::Failed(m) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("{m}; and `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                Err(self.blocked(&format!("the rebase could not run: {m}. The branch is unchanged")))
            }
        }
    }

    // -- step 3: check ----------------------------------------------------------------------

    fn s_check(&mut self) -> R<()> {
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

    // -- step 4: review ---------------------------------------------------------------------

    fn s_review(&mut self) -> R<()> {
        self.set_step("review")?;
        if !self.e.get("verdict").is_empty() {
            self.log("review", "the verdict is already recorded (this run resumed): the reviewer is not asked again")?;
            return self.recorded_verdict_ok();
        }
        let (dir, branch) = (self.dir.clone(), self.branch());
        let gate = match gating_model(self.cfg, self.e.get("risk"), self.e.get("coder")) {
            Ok(g) => g,
            Err(msg) => return Err(self.blocked(&msg)),
        };
        // Check, review and merge are bound to ONE commit: the one the check passed on. A branch that moved since (a commit added
        // after the check, live or before a resume) was never checked, so it is never reviewed and never merged.
        let checked = self.e.get("checked_tip").to_string();
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if checked.is_empty() {
            return Err(self.end(STALE, &format!("no checked commit is recorded for this entry, so nothing was reviewed or merged. Run `queue run --entry {}` to check it", self.e.id()), EXIT_RETRY));
        }
        if tip != checked {
            return Err(self.end(STALE, &format!("branch changed after check: `{branch}` was checked at {} but is at {} now; the new commit(s) were never checked, so nothing was reviewed or merged. Run `queue run --entry {}` to rebase and check it again", short(&checked), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let base = self.e.get("base_head").to_string();
        let diff = self.g(wtgit::diff_since(&dir, &base, &checked))?;
        if diff.trim().is_empty() {
            return Err(self.blocked("the diff against main is empty: nothing to review or merge"));
        }
        let max = self.cfg.reviewer_diff_max_kb * 1024;
        if diff.len() as u64 > max {
            return Err(self.blocked(&format!("the diff is {} KB, over reviewer_diff_max_kb = {}: it is never cut, so it is never sent. Split the task into smaller ones", diff.len() / 1024, self.cfg.reviewer_diff_max_kb)));
        }
        let backend = match reviewer::backend(self.cfg) {
            Ok(b) => b,
            Err(m) => return Err(self.blocked(&m)),
        };
        let cli = match backend.prepare(self.cfg) {
            Ok(c) => c,
            Err(m) => return Err(self.blocked(&format!("the reviewer CLI was refused: {m}. No model was called, nothing merged"))),
        };
        let calls = self.e.get_num("review_calls").unwrap_or(0);
        if calls >= self.cfg.reviewer_max_calls_per_entry {
            return Err(self.blocked(&format!("review call cap reached: this entry already made {calls} reviewer call(s) (reviewer_max_calls_per_entry = {}); no model was called, nothing merged. Cancel it and queue the task again if more reviews are wanted", self.cfg.reviewer_max_calls_per_entry)));
        }
        self.e.set("reviewed_tip", checked.clone());
        self.e.set("reviewer_cli", cli.path.clone());
        self.e.set("test_hook", if cli.test_hook { "true" } else { "false" });
        self.save()?;
        if cli.test_hook {
            println!("TEST HOOK ACTIVE: reviewer CLI is {}", cli.path);
        } else {
            println!("queue entry {}: reviewer CLI is {}", self.e.id(), cli.path);
        }
        let brief = with_done_check(self.e.get("brief"), self.e.get("done_check"));
        let prompt = self.secrets.redact(&reviewer::build_prompt(&brief, self.e.get("check_summary"), self.e.get("check_tail"), &diff));
        let short_note = if self.secrets.short_count() > 0 { format!("; {} value(s) of .env are shorter than {} characters: not redacted from text (too common) but kept out of the reviewer's environment", self.secrets.short_count(), self.cfg.redact_min_chars) } else { String::new() };
        // Which gate runs: a GPT gate first when the rule picked one; a SKIPPED GPT call falls back to Claude in this same run.
        let mut model = gate.model.clone();
        let mut gpt_answer = None;
        self.e.set("gate", "claude");
        if gate.backend == GateBackend::Gpt {
            self.e.set("reviewer_model", model.clone());
            self.e.set("reviewer_effort", "");
            match self.gpt_gate(&model, &prompt)? {
                Ok(answer) => {
                    self.e.set("gate", "gpt");
                    gpt_answer = Some(answer);
                }
                Err(reason) => {
                    model = gate.fallback.clone().unwrap_or_default();
                    self.e.set("gate", format!("fallback: GPT gate `{}` SKIPPED ({reason})", gate.model));
                    self.log("review", &format!("the GPT gate `{}` was SKIPPED: {reason}; the Claude reviewer `{model}` gates this entry in the same run", gate.model))?;
                }
            }
        }
        let answered = if let Some((review, script, hook)) = gpt_answer {
            self.e.set("reviewer_cli", format!("{script} ({SHADOW_CLIENT_LABEL})"));
            self.e.set("test_hook", if hook { "true" } else { "false" });
            (review, format!("openai ({SHADOW_CLIENT_LABEL})"), script, hook)
        } else {
            let effort = effort_for(self.cfg, self.e.get("risk")).to_string();
            self.e.set("reviewer_model", model.clone());
            self.e.set("reviewer_effort", effort.clone());
            let n = self.take_review_call()?;
            self.log("review", &format!("calling the {} reviewer `{model}` through {}{} on {} (diff {} bytes, commit {}, call {n} of at most {}); this spends money and uses the network{short_note}", backend.name(), cli.path, if cli.test_hook { " [TEST HOOK]" } else { "" }, if self.e.get("risk") == "high" { "a high-risk change" } else { "a low-risk change" }, diff.len(), short(&checked), self.cfg.reviewer_max_calls_per_entry))?;
            let r = match backend.call(self.cfg, &cli, &model, &effort, crate::reviewer::gate_system_prompt(self.e.get("risk")), &prompt) {
                Ok(r) => r,
                Err(CallError::Timeout(s)) => return Err(self.blocked(&format!("the reviewer `{model}` gave no answer within {s} s (reviewer_timeout_seconds) and was killed: no verdict, nothing merged. Run `queue run --entry {}` to ask again", self.e.id()))),
                Err(CallError::Start(m) | CallError::Failed(m)) => return Err(self.blocked(&format!("the reviewer `{model}` could not be used: {m}. No verdict, nothing merged. Fix it, then `queue run --entry {}`", self.e.id()))),
            };
            (r, backend.name().to_string(), cli.path.clone(), cli.test_hook)
        };
        let (review, backend_name, cli_path, test_hook) = answered;
        let used = if self.e.get("gate") == "gpt" && review.model_reported != Some(true) { "model not reported".to_string() } else { review.models_used.join(", ") };
        self.e.set("gate_model_used", used);
        let verdict = reviewer::judge(&review);
        let file = self.write_review(&model, &checked, &base, prompt.len(), (&backend_name, &cli_path, test_hook), &review, &verdict)?;
        self.e.set("verdict", if verdict.pass { "PASS" } else { "FAIL" });
        self.e.set("blocking", verdict.blocking.join("\n"));
        self.e.set("minor", verdict.minor.join("\n"));
        self.e.set("cost_usd", review.cost_usd.clone().unwrap_or_default());
        self.e.set("tokens_in", review.input_tokens.map(|n| n.to_string()).unwrap_or_default());
        self.e.set("tokens_out", review.output_tokens.map(|n| n.to_string()).unwrap_or_default());
        self.e.set("review_file", file.clone());
        self.save()?;
        self.log("review", &format!("{} by `{model}` (gate: {}); {} blocking, {} minor; cost {} USD; full output in {file}", self.e.get("verdict"), self.e.get("gate"), verdict.blocking.len(), verdict.minor.len(), review.cost_usd.as_deref().unwrap_or("unknown")))?;
        self.s_shadow(&prompt)?;
        if verdict.pass {
            return Ok(());
        }
        let why = if verdict.note.is_empty() { verdict.blocking.join("; ") } else { format!("{} ({})", verdict.note, verdict.blocking.join("; ")) };
        Err(self.end(REJECTED, &format!("the `{model}` reviewer FAILED it: {why}. The task is still running and its worktree is kept: fix, commit, and queue it again with `queue add`. Full review: {file}"), EXIT_REJECTED))
    }

    /// Count one reviewer call (checked against `reviewer_max_calls_per_entry`, saved BEFORE the call, so a run that dies in the middle
    /// of it still paid for one). Returns the number of this call.
    fn take_review_call(&mut self) -> R<u64> {
        let calls = self.e.get_num("review_calls").unwrap_or(0);
        if calls >= self.cfg.reviewer_max_calls_per_entry {
            return Err(self.blocked(&format!("review call cap reached: this entry already made {calls} reviewer call(s) (reviewer_max_calls_per_entry = {}); no model was called, nothing merged. Cancel it and queue the task again if more reviews are wanted", self.cfg.reviewer_max_calls_per_entry)));
        }
        self.e.set("review_calls", (calls + 1).to_string());
        self.save()?;
        Ok(calls + 1)
    }

    /// The GPT gate: the gating prompt to `model` through clients/openai_review.py, the same program (so the same daily cap and ledger,
    /// call type `gate`) the shadow review uses. Ok(Err(reason)) means the call was SKIPPED (no python or script, cap reached, GPT
    /// unavailable, error, timeout, output that is not the wrapper's JSON, or another model answering): the caller falls back to
    /// Claude in the same run. Ok(Ok(..)) is an answer to judge like a Claude one: (review, script, test hook).
    fn gpt_gate(&mut self, model: &str, prompt: &str) -> R<Result<(reviewer::Review, String, bool), String>> {
        let (python, script) = match reviewer::resolve_shadow(self.cfg) {
            Ok(p) => p,
            Err(m) => return Ok(Err(m)),
        };
        let hook = std::env::var(reviewer::SHADOW_SCRIPT_ENV).is_ok_and(|v| !v.trim().is_empty());
        let n = self.take_review_call()?;
        self.log("review", &format!("calling the GPT gate `{model}` through {script}{} (call {n} of at most {}, at most {} s); this spends money (plan, then credits under the daily cap) and uses the network", if hook { " [TEST HOOK]" } else { "" }, self.cfg.reviewer_max_calls_per_entry, self.cfg.reviewer_timeout_seconds))?;
        let one_line = |m: &str| m.lines().next().unwrap_or("").chars().take(300).collect::<String>();
        let gpt = gpt_call_lock(); // PARALLEL REVIEW: one GPT call at a time (see `GPT_CALL`)
        let answer = reviewer::call_gpt(self.cfg, &python, &script, model, reviewer::gate_system_prompt(self.e.get("risk")), prompt, "gate", self.e.get("task"), self.cfg.reviewer_timeout_seconds);
        drop(gpt);
        Ok(match answer {
            Err(CallError::Timeout(s)) => Err(format!("no answer within {s} s (reviewer_timeout_seconds)")),
            Err(CallError::Start(m) | CallError::Failed(m)) => Err(one_line(&m)),
            Ok(r) => match &r.invalid {
                Some(why) => Err(one_line(why)),
                None => Ok((r, script, hook)),
            },
        })
    }

    /// The SHADOW review: the same prompt to a second model (`shadow_reviewer`), run once after the gating verdict is recorded.
    /// It is only RECORDED: it never changes the entry's outcome, so no failure of it (timeout, API error, bad JSON, no python, no
    /// script) ends or blocks the entry; each is recorded as `shadow_verdict = ERROR` with the reason. It runs at most once per
    /// review call, because a resumed run that already has a gating verdict never reaches the review call again.
    /// S4 (no reviewer program inside the root) does not apply to it: it never gates a merge.
    fn s_shadow(&mut self, prompt: &str) -> R<()> {
        if self.cfg.shadow_reviewer == "off" {
            return Ok(());
        }
        let model = self.cfg.shadow_reviewer_model.clone();
        self.e.set("shadow_model", model.clone());
        if self.e.get("gate") == "gpt" {
            return self.shadow_end("SKIPPED", "the gating review itself was GPT (no second GPT call for the same entry)");
        }
        let coder = self.e.get("coder").to_string();
        if reviewer::shadow_family(&coder) == reviewer::shadow_family(&model) {
            return self.shadow_end("SKIPPED", &format!("the coder `{coder}` and the shadow model `{model}` are the same family (a model never reviews its own code)"));
        }
        let (python, script) = match reviewer::resolve_shadow(self.cfg) {
            Ok(p) => p,
            Err(m) => return self.shadow_end("ERROR", &m),
        };
        self.log("shadow", &format!("calling the shadow reviewer `{model}` (recorded only, never gating; at most {} s); this spends money and uses the network", self.cfg.shadow_reviewer_timeout_seconds))?;
        let gpt = gpt_call_lock(); // PARALLEL REVIEW: one GPT call at a time (see `GPT_CALL`)
        let t0 = std::time::Instant::now();
        let result = reviewer::call_shadow(self.cfg, &python, &script, &model, prompt, self.e.get("task"));
        drop(gpt);
        let secs = t0.elapsed().as_secs_f64();
        self.e.set("shadow_secs", format!("{secs:.1}"));
        let review = match result {
            Ok(r) => r,
            Err(CallError::Timeout(s)) => return self.shadow_end("ERROR", &format!("no answer within {s} s (shadow_reviewer_timeout_seconds); it was killed")),
            Err(CallError::Start(m) | CallError::Failed(m)) => return self.shadow_end("ERROR", &m),
        };
        let v = reviewer::judge(&review);
        let file = self.write_shadow_file(&model, &review, &v)?;
        self.e.set("shadow_file", file);
        self.e.set("shadow_model_used", review.models_used.join(", "));
        self.e.set("shadow_cost_usd", review.cost_usd.clone().unwrap_or_default());
        self.e.set("shadow_tokens", match (review.input_tokens, review.output_tokens) {
            (Some(i), Some(o)) => (i + o).to_string(),
            _ => String::new(),
        });
        if review.invalid.is_some() || !v.parsed {
            return self.shadow_end("ERROR", &v.note);
        }
        self.e.set("shadow_blocking", v.blocking.join("
"));
        self.shadow_end(if v.pass { "PASS" } else { "FAIL" }, "")
    }

    fn shadow_end(&mut self, verdict: &str, reason: &str) -> R<()> {
        self.e.set("shadow_verdict", verdict);
        self.e.set("shadow_reason", reason);
        self.save()?;
        println!("queue entry {}: shadow review {verdict}{} (recorded only; the gating review decides)", self.e.id(), if reason.is_empty() { String::new() } else { format!(": {reason}") });
        self.log("shadow", &format!("{verdict}{}", if reason.is_empty() { String::new() } else { format!(": {reason}") }))
    }

    fn write_shadow_file(&mut self, model: &str, r: &reviewer::Review, v: &reviewer::Verdict) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 { format!("{}.shadow.review", self.e.id()) } else { format!("{}.{attempt}.shadow.review", self.e.id()) };
        let path = queue_dir(self.cfg).join(&name);
        let text = format!(
            "entry: {}
attempt: {attempt}
shadow review: recorded only, never gating
model: {model}
models reported: {}
verdict: {} (answer followed the format: {})
{}blocking:
{}
--- raw shadow output (exactly as printed) ---
{}
",
            self.e.id(),
            r.models_used.join(", "),
            if v.pass { "PASS" } else { "FAIL" },
            if v.parsed && r.invalid.is_none() { "yes" } else { "NO" },
            if v.note.is_empty() { String::new() } else { format!("note: {}
", v.note) },
            v.blocking.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("
"),
            r.raw
        );
        let text = self.secrets.redact(&text);
        let tmp = path.with_extension("review.tmp");
        if let Err(e) = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path)) {
            return Err(self.internal(&format!("cannot write the shadow review file {}: {e}", path.display())));
        }
        Ok(path.strip_prefix(&self.cfg.root).unwrap_or(&path).to_string_lossy().replace('\\', "/"))
    }

    /// A verdict is already recorded (a resumed run). Returns Ok only for a PASS on exactly the checked commit; anything else ends
    /// the entry the way a fresh run would, so no resumed entry can be stuck `running` (which would block `queue run` for all):
    /// not PASS -> `rejected` exactly as a fresh FAIL (task running, worktree kept, `queue add` works again);
    /// a PASS without a recorded reviewed or checked commit, or for another commit -> `stale` (`queue run --entry` starts a fresh attempt).
    fn recorded_verdict_ok(&mut self) -> R<()> {
        let id = self.e.id().to_string();
        if self.e.get("verdict") != "PASS" {
            let model = self.e.get("reviewer_model").to_string();
            let file = self.e.get("review_file").to_string();
            let blocking = self.e.get("blocking").replace('\n', "; ");
            let why = if blocking.is_empty() { format!("verdict `{}`", self.e.get("verdict")) } else { blocking };
            return Err(self.end(REJECTED, &format!("the `{model}` reviewer FAILED it: {why}. The task is still running and its worktree is kept: fix, commit, and queue it again with `queue add`. Full review: {file}"), EXIT_REJECTED));
        }
        let (checked, reviewed) = (self.e.get("checked_tip").to_string(), self.e.get("reviewed_tip").to_string());
        if checked.is_empty() || reviewed.is_empty() || checked != reviewed {
            return Err(self.end(STALE, &format!("the recorded PASS does not name the checked commit (checked `{}`, reviewed `{}`), so it cannot be trusted; nothing was merged. Run `queue run --entry {id}` to check and review it again", short(&checked), short(&reviewed)), EXIT_RETRY));
        }
        Ok(())
    }

    /// The review file: header lines, then the reviewer's raw output. Returns its path relative to root. Written atomically.
    #[allow(clippy::too_many_arguments)]
    fn write_review(&mut self, model: &str, tip: &str, base: &str, prompt_bytes: usize, via: (&str, &str, bool), r: &reviewer::Review, v: &reviewer::Verdict) -> R<String> {
        let attempt = self.e.get_num("attempt").unwrap_or(1);
        let name = if attempt <= 1 { format!("{}.review", self.e.id()) } else { format!("{}.{attempt}.review", self.e.id()) };
        let path = queue_dir(self.cfg).join(&name);
        let or_unknown = |o: Option<String>| o.unwrap_or_else(|| "unknown".into());
        let text = format!(
            "entry: {}\nattempt: {attempt}\ntask: {}\nbackend: {}\nreviewer cli: {}\ntest_hook: {}\ngate: {}\nmodel: {model}\nmodels reported by the backend: {}\nrisk: {}\ncoder: {}\nreviewed commit: {tip}\ndiff base: {base}\nprompt bytes: {prompt_bytes}\ncost_usd: {}\ninput tokens: {}\noutput tokens: {}\nverdict: {} (answer followed the format: {})\n{}blocking:\n{}\nminor:\n{}\n--- raw reviewer output (exactly as printed) ---\n{}\n",
            self.e.id(),
            self.e.get("task"),
            via.0,
            via.1,
            via.2,
            self.e.get("gate"),
            if r.models_used.is_empty() { "-".to_string() } else { r.models_used.join(", ") },
            self.e.get("risk"),
            self.e.get("coder"),
            or_unknown(r.cost_usd.clone()),
            or_unknown(r.input_tokens.map(|n| n.to_string())),
            or_unknown(r.output_tokens.map(|n| n.to_string())),
            if v.pass { "PASS" } else { "FAIL" },
            if v.parsed { "yes" } else { "NO" },
            if v.note.is_empty() { String::new() } else { format!("note: {}\n", v.note) },
            v.blocking.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("\n"),
            v.minor.iter().map(|b| format!("  {b}")).collect::<Vec<_>>().join("\n"),
            r.raw
        );
        let text = self.secrets.redact(&text);
        let tmp = path.with_extension("review.tmp");
        let written = std::fs::write(&tmp, text.as_bytes()).and_then(|_| std::fs::rename(&tmp, &path));
        if let Err(e) = written {
            return Err(self.internal(&format!("cannot write the review file {}: {e}", path.display())));
        }
        Ok(path.strip_prefix(&self.cfg.root).unwrap_or(&path).to_string_lossy().replace('\\', "/"))
    }

    // -- step 5: merge ----------------------------------------------------------------------

    fn s_merge(&mut self) -> R<()> {
        self.set_step("merge")?;
        self.locate()?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let reviewed = self.e.get("reviewed_tip").to_string();
        let base = self.e.get("base_head").to_string();
        // A resume at this step skips the review step: the recorded state is judged here, and every bad state ENDS the entry
        // (`rejected` or `stale`), never leaves it `running` where it would block `queue run` for every other entry.
        if self.e.get("verdict") != "PASS" {
            self.recorded_verdict_ok()?; // always an Err here: rejected
        }
        if reviewed.is_empty() || base.is_empty() || self.e.get("checked_tip") != reviewed {
            return Err(self.end(STALE, &format!("the merge step was reached without a recorded PASS for the checked commit (reviewed `{}`, checked `{}`, base `{}`); nothing was merged. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(self.e.get("checked_tip")), short(&base), self.e.id()), EXIT_RETRY));
        }
        let cli_path = self.e.get("reviewer_cli").to_string();
        if let Some(why) = self.test_hook_refusal() {
            return Err(self.blocked(&why));
        }
        self.refuse_chained("merge")?;
        println!("queue entry {}: merging after a {} review through reviewer CLI {}", self.e.id(), if self.e.get("test_hook") == "true" { "TEST HOOK" } else { "real" }, if cli_path.is_empty() { "(none recorded)" } else { &cli_path });
        // the trunk (`refs/heads/<trunk>`), checked out in the main checkout, is what is fast-forwarded: never another branch
        let head = self.on_trunk()?;
        if head == reviewed || self.g(wtgit::is_ancestor(&dir, &reviewed, &head))? {
            // an earlier run already fast-forwarded main to this commit (a merge train: to a later commit that contains it) and died
            // before recording it
            self.e.set("main_sha", reviewed.clone());
            if head == reviewed {
                return self.log("merge", &format!("main's HEAD already is the reviewed commit {}: recorded as merged", short(&head)));
            }
            return self.log("merge", &format!("the reviewed commit {} is already on main (HEAD {} contains it): recorded as merged", short(&reviewed), short(&head)));
        }
        let dirty = self.g(wtgit::status_porcelain_plain(&dir))?;
        if !dirty.is_empty() {
            return Err(self.blocked(&format!("the main checkout got uncommitted or untracked changes while the review ran ({}); nothing merged", some(&dirty))));
        }
        if let Some(why) = lane_hold(self.cfg, self.e.get("project")) {
            return Err(self.blocked(&format!("lane {} held: {why}; nothing merged. Clear with a passing check of main, then queue run", self.e.get("project"))));
        }
        if head != base {
            return Err(self.end(STALE, &format!("main moved from {} to {} after the rebase and the review; nothing was merged. Run `queue run --entry {}` to rebase again", short(&base), short(&head), self.e.id()), EXIT_RETRY));
        }
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if tip != reviewed {
            return Err(self.end(STALE, &format!("branch changed after check and review: `{branch}` moved from the checked and reviewed commit {} to {}; nothing was merged. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let wt_status = self.g(wtgit::status_porcelain_plain(&wt))?;
        if !wt_status.is_empty() {
            return Err(self.blocked(&format!("the task's worktree got changes after the review ({}); nothing merged", some(&wt_status))));
        }
        // the last look before the fast-forward: still the trunk checked out, still at `head`
        if self.on_trunk()? != head {
            return Err(self.end(STALE, &format!("main moved from {} just before the merge; nothing was merged. Run `queue run --entry {}` to rebase again", short(&head), self.e.id()), EXIT_RETRY));
        }
        // AUTO-REVERT: the trunk tip before the fast-forward; `pre_merge_sha..main_sha` is exactly what this entry merged
        self.e.set("pre_merge_sha", head.clone());
        self.save()?;
        if let Err(m) = wtgit::merge_ff_only(&dir, &reviewed) {
            let now = self.g(trunk_tip(&dir, "").map(|(_, tip)| tip))?;
            if now != head {
                return Err(self.end(STALE, &format!("main moved from {} to {} during the merge; nothing was merged by this run. Run `queue run --entry {}` to rebase again", short(&head), short(&now), self.e.id()), EXIT_RETRY));
            }
            return Err(self.blocked(&format!("git refused the fast-forward: {m}. Main is unchanged")));
        }
        let mb = self.e.get("main_branch").to_string();
        let after = self.g(trunk_checkout(&dir, &mb).map(|(_, tip)| tip))?;
        if after != reviewed {
            return Err(self.internal(&format!("after `git merge --ff-only` the trunk `{mb}` is at {} but the reviewed commit is {}; it was {} before; look at the project", short(&after), short(&reviewed), short(&head))));
        }
        self.e.set("main_sha", after.clone());
        let note = format!("fast-forwarded `{}` from {} to {}; reviewed through CLI {} (test_hook: {})", self.e.get("main_branch"), short(&head), short(&after), if cli_path.is_empty() { "(none recorded)" } else { &cli_path }, self.e.get("test_hook"));
        self.log("merge", &note)
    }

    /// A stand-in reviewer (a scratch test hook) may only merge into a project that itself lives in scratch. The ONE place this rule
    /// is decided: the single-entry merge (`s_merge`) and the merge train (`train_ready`, before any chaining) both ask it.
    fn test_hook_refusal(&self) -> Option<String> {
        if self.e.get("test_hook") != "true" {
            return None;
        }
        let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        if canon(&self.dir).starts_with(canon(&self.cfg.scratch_dir)) {
            return None;
        }
        Some(format!("the review was made by a TEST HOOK reviewer ({}), not a real one, and project folder {} is outside scratch_dir; a stand-in review never merges into a real project; nothing merged", self.e.get("reviewer_cli"), self.dir.display()))
    }

    // -- step 6: main check -----------------------------------------------------------------

    fn s_main_check(&mut self) -> R<()> {
        self.set_step("main_check")?;
        let project = self.e.get("project").to_string();
        let sha = self.e.get("main_sha").to_string();
        let mb = self.e.get("main_branch").to_string();
        let loud = |code: String, extra: &str| {
            format!("commit {sha} is ALREADY on `{mb}` of project `{project}`, and `check {project}` there did not pass ({code}){extra}. Main was NOT reset or reverted. A human decides: fix forward, or `git revert` the commit. The task was NOT marked done and its worktree is kept")
        };
        // The main check runs in the main checkout, on its files: only while that checkout is the trunk and holds the merged
        // commit is it a check of the trunk. Otherwise it is refused, loudly (`trunk_checkout`); never run on another branch.
        let on_trunk = trunk_checkout(&self.dir, &mb).and_then(|(trunk, tip)| match wtgit::is_ancestor(&self.dir, &sha, &tip)? {
            true => Ok(()),
            false => Err(format!("the trunk `{trunk}` (at {}) does not hold commit {}", short(&tip), short(&sha))),
        });
        if let Err(m) = on_trunk {
            return Err(self.end(MERGED_CHECK_FAILED, &loud("the check was refused before it started".into(), &format!(": {m}")), EXIT_MERGED_BAD));
        }
        if let Some(code) = tools::guard_refusal(self.cfg, &project, None) {
            return Err(self.end(MERGED_CHECK_FAILED, &loud(format!("the check was refused before it started, exit {code}"), ""), EXIT_MERGED_BAD));
        }
        println!("queue entry {}: check {project} (the main checkout, on its trunk `{mb}`)", self.e.id());
        let mut info = JobInfo::default();
        let code = job::run_job_info(self.cfg, JobSpec { kind: "check".into(), name: project.clone(), command: None, first_step: 0, resumed_from: None, resume_count: 0, task: None }, &mut info);
        if code != 0 {
            let extra = if info.summary.is_empty() { String::new() } else { format!(": {}", info.summary.join("; ")) };
            let job_id = info.id.clone().unwrap_or_else(|| "none".into());
            if !self.cfg.queue_auto_revert {
                return Err(self.end(MERGED_CHECK_FAILED, &loud(format!("exit {code}, job {job_id}"), &extra), EXIT_MERGED_BAD));
            }
            let failed = format!("commit {sha} is on `{mb}` of project `{project}`, and `check {project}` there did not pass (exit {code}, job {job_id}){extra}.");
            let reason = self.auto_revert(&failed, &job_id);
            return Err(self.end(MERGED_CHECK_FAILED, &reason, EXIT_MERGED_BAD));
        }
        self.log("main_check", &format!("check on main passed ({})", info.summary.join("; ")))
    }

    /// AUTO-REVERT (queue_auto_revert = "on"), after a main check that ran and failed: undo exactly the commits this entry (or the
    /// merge train it ended) fast-forwarded, `pre_merge_sha..main_sha`, with ONE new revert commit on the trunk, then check main
    /// again. Never a reset, never a force, never a push. Refused (nothing is touched) when no pre-merge commit is recorded, when the
    /// main checkout is not on the trunk, when the trunk moved since the merge, or when the checkout has changes. Any refusal, a revert
    /// or commit that fails (aborted with `git revert --abort`), or a re-check that fails HOLDS the lane (`write_hold`). Returns the
    /// entry's reason text; `failed` says what failed first.
    fn auto_revert(&mut self, failed: &str, job_id: &str) -> String {
        let project = self.e.get("project").to_string();
        let held = |cfg: &Config, id: &str, why: &str| -> String {
            let note = match write_hold(cfg, &project, id, why) {
                Ok(p) => format!("Lane `{project}` is HELD ({}): `queue run` merges nothing for it until a check of its main passes", p.display()),
                Err(m) => format!("the lane hold could NOT be written ({m})"),
            };
            format!("{failed} The automatic revert did not complete: {why}. {note}. Main was NOT reset. A human decides: fix forward, or `git revert` the commits. The task was NOT marked done and its worktree is kept")
        };
        let id = self.e.id().to_string();
        match self.revert_merged(job_id) {
            Err(why) => held(self.cfg, &id, &why),
            Ok((range, rsha)) => {
                let _ = self.log("main_check", &format!("main check failed; reverted {range} as {rsha}; checking main again"));
                println!("queue entry {id}: main check failed; reverted {range} as {rsha}; check {project} again");
                match main_check_now(self.cfg, &self.dir, &project) {
                    Ok(job) => {
                        // the one mark `green_revert` trusts: main is green again, so `queue run --all` may go on
                        self.e.set("revert_check", "passed");
                        format!("{failed} The merged commits {range} were reverted as {rsha} (one new commit; nothing was reset); main check after the revert passed (job {job}). The task was NOT marked done and its worktree is kept: fix it and queue it again")
                    }
                    Err(m) => held(self.cfg, &id, &format!("the merged commits {range} were reverted as {rsha}, but the main check after the revert did not pass: {m}")),
                }
            }
        }
    }

    /// The revert itself (see `auto_revert`): `Ok((range, revert commit))`, or why nothing (or nothing left) was reverted.
    fn revert_merged(&mut self, job_id: &str) -> Result<(String, String), String> {
        let (dir, mb) = (self.dir.clone(), self.e.get("main_branch").to_string());
        let pre = self.e.get("pre_merge_sha").to_string();
        let last = self.e.get("main_sha").to_string();
        if pre.is_empty() || last.is_empty() {
            // NEVER derived (from the base or anything else): only the queue's own merge step records what it merged
            return Err(format!("no pre-merge commit recorded; not reverting (pre_merge_sha `{pre}`, main_sha `{last}`: the queue's merge step did not record what this entry merged, e.g. main was fast-forwarded by hand)"));
        }
        let (_, tip) = trunk_checkout(&dir, &mb).map_err(|m| format!("{m}; nothing was reverted"))?;
        if tip != last {
            return Err(format!("the trunk `{mb}` moved since the merge: it was {last} after the merge, it is {tip} now (someone else committed); nothing was reverted"));
        }
        if pre == last || !wtgit::is_ancestor(&dir, &pre, &last)? {
            return Err(format!("the recorded pre-merge commit {pre} is not before the merged commit {last}; nothing was reverted"));
        }
        let dirty = wtgit::status_porcelain_plain(&dir)?;
        if !dirty.is_empty() {
            return Err(format!("the main checkout has uncommitted or untracked changes ({}); nothing was reverted", some(&dirty)));
        }
        let range = format!("{pre}..{last}");
        let commits = wtgit::commit_subjects(&dir, &range)?;
        if commits.is_empty() {
            return Err(format!("git lists no commit in {range}; nothing was reverted"));
        }
        let mut msg: String = commits.iter().map(|(_, s)| format!("Revert \"{s}\"\n")).collect();
        msg.push('\n');
        msg.extend(commits.iter().map(|(c, _)| format!("This reverts commit {c}.\n")));
        msg.push_str(&format!("\nReverted-by: broadsword queue {} (main check failed, job {job_id})\n", self.e.id()));
        std::fs::create_dir_all(&self.cfg.scratch_dir).map_err(|e| format!("cannot create {}: {e}; nothing was reverted", self.cfg.scratch_dir.display()))?;
        let msg_file = self.cfg.scratch_dir.join(format!("queue-revert-{}.msg", record::sanitize(self.e.id())));
        std::fs::write(&msg_file, &msg).map_err(|e| format!("cannot write the revert message {}: {e}; nothing was reverted", msg_file.display()))?;
        // the last look before the revert: still the trunk checked out, still at the merged commit
        let (_, now) = trunk_checkout(&dir, &mb).map_err(|m| format!("{m}; nothing was reverted"))?;
        if now != last {
            return Err(format!("the trunk `{mb}` moved just before the revert: it was {last}, it is {now} now; nothing was reverted"));
        }
        let done = wtgit::revert_no_commit(&dir, &range).and_then(|_| wtgit::commit_file(&dir, &msg_file));
        let _ = std::fs::remove_file(&msg_file);
        if let Err(m) = done {
            let aborted = match wtgit::revert_in_progress(&dir) {
                Ok(true) => match wtgit::revert_abort(&dir) {
                    Ok(()) => "the revert was aborted (`git revert --abort`), main is as it was after the merge".to_string(),
                    Err(e) => format!("AND `git revert --abort` failed: {e}; look at the main checkout"),
                },
                Ok(false) => "no revert was left in progress".to_string(),
                Err(e) => format!("whether a revert is left in progress could not be read: {e}; look at the main checkout"),
            };
            return Err(format!("`git revert --no-commit {range}` and its commit failed: {m}; {aborted}"));
        }
        // the revert commit must be exactly ONE new commit on the merged tip, on the trunk that is still checked out; anything else
        // (the trunk moved or was switched during the revert) holds the lane with both shas, and no further git action is taken
        let (_, after) = trunk_checkout(&dir, &mb).map_err(|m| format!("after the revert commit: {m}; expected the trunk at one new commit on {last}; look at the project"))?;
        if after == last || !wtgit::is_ancestor(&dir, &last, &after)? || wtgit::rev_count(&dir, &format!("{last}..{after}"))? != 1 {
            return Err(format!("after the revert the trunk `{mb}` is at {after}, whose parent is not the merged tip {last} checked before the revert; look at the project"));
        }
        self.e.set("revert_sha", after.clone());
        let _ = self.save();
        Ok((range, after))
    }

    // -- step 7: done -----------------------------------------------------------------------

    fn s_done(&mut self) -> R<ExitCode> {
        self.set_step("done")?;
        let id = self.e.get("task").to_string();
        let t = self.g(task::load_task(self.cfg, &id))?;
        if t.status() == task::RUNNING {
            let note = format!("merged by queue entry {} as {}", self.e.id(), short(self.e.get("main_sha")));
            let _ = task::cmd_finish(self.cfg, &id, task::Action::Done, Some(&note)); // prints its own result; judged below from the record
        }
        let t = self.g(task::load_task(self.cfg, &id))?;
        if t.status() != task::DONE {
            return Err(self.internal(&format!("`task done {id}` did not complete (the task is `{}`; it may be that the board was busy); the commit {} IS on main. Resume with `queue run --entry {}`", t.status(), short(self.e.get("main_sha")), self.e.id())));
        }
        // Removal keeps a worktree that holds anything git would lose (ignored files included): the entry is still `merged`.
        let wt_note = if worktree::has_worktree(&t) {
            let why = worktree::abs_path(self.cfg, t.get("worktree")).ok().and_then(|p| wtgit::status_porcelain(&p).ok()).filter(|s| !s.is_empty()).map(|s| format!(": it holds files removal will not delete ({})", some(&s))).unwrap_or_else(|| " (the reason is in the warning `task done` printed)".into());
            format!("its worktree and branch were KEPT{why}; nothing was lost")
        } else {
            "its worktree and branch are removed".to_string()
        };
        Ok(self.end(MERGED, &format!("commit {} is on `{}`, check on main passed, task {id} is done; {wt_note}", short(self.e.get("main_sha")), self.e.get("main_branch")), 0))
    }
}

// ---- merge train ----------------------------------------------------------------------------

/// The result of one check of a commit (see `Run::check_commit`).
enum TrainCheck {
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
struct PassInfo {
    job: String,
    summary: String,
    tail: String,
    /// A failed check: the perf gate's reason when it was the perf gate that failed (empty for a build or test failure).
    perf: String,
}

/// What a `check_alone` said about the culprit of a failed train.
enum Alone {
    /// It fails on its own commit too: the entry itself is bad.
    Bad(PassInfo),
    /// It passes on its own commit: the failure only shows when it is combined with the entries before it.
    Fine,
    Unknown(String),
    /// Its own worktree kept the check from a verdict (see `TrainCheck::EntryBlocked`).
    EntryBlocked(String),
}

/// The entries the next train takes out of `items` (one `(project, claims)` per approved entry, oldest first), as indices into
/// `items`. Entries of one project only; an entry whose claims overlap an entry already taken is left out (it waits for a later
/// train); at most `max` entries. A train is ready when it is full, or when `flush` is set (nothing else is coming) and it holds at
/// least one entry. Empty when no project has a ready train.
#[cfg(test)]
fn pick_train(items: &[(String, Vec<String>)], max: usize, flush: bool) -> Vec<usize> {
    pick_train_when(items, max, &|_| flush)
}

/// `pick_train` with readiness per project: a project's train is ready when it is full, or when `ready(project)` and it holds at
/// least one entry (the flush train of the drain's end is `ready` for every project; `short_train_projects` says which projects get
/// a short train before the end). Projects are tried in queue order of their oldest approved entry.
fn pick_train_when(items: &[(String, Vec<String>)], max: usize, ready: &dyn Fn(&str) -> bool) -> Vec<usize> {
    let mut seen: Vec<&str> = Vec::new();
    for (first, (project, _)) in items.iter().enumerate() {
        if seen.contains(&project.as_str()) {
            continue;
        }
        seen.push(project.as_str());
        let mut group: Vec<usize> = Vec::new();
        for (i, (p, claims)) in items.iter().enumerate().skip(first) {
            if p != project || group.len() >= max {
                continue;
            }
            let clash = group.iter().any(|g| items[*g].1.iter().any(|a| claims.iter().any(|b| task::overlaps(a, b))));
            if !clash {
                group.push(i);
            }
        }
        if group.len() >= max || (!group.is_empty() && ready(project)) {
            return group;
        }
    }
    Vec::new()
}

/// The commits a merge train may have chained this entry's branch onto, in this order: `left_train_base` (recorded when it left a
/// train, kept across attempts), `train_onto` (a chain rebase begun onto it and never recorded as finished: a drain killed after the
/// rebase moved the branch and before `train_link` saved it, see its KILL WINDOW note; `train_up` is then still the old base) and
/// `train_up` (the base of its own commits while it rides a train). Empty for an entry no train ever chained.
fn chain_bases_of(e: &Record) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for k in ["left_train_base", "train_onto", "train_up"] {
        let v = e.get(k);
        if !v.is_empty() && !out.iter().any(|x| x == v) {
            out.push(v.to_string());
        }
    }
    out
}

/// LEFT-TRAIN RULE, for every caller: the first of `chain_bases_of(e)` that `branch` is still chained on (`chained_onto`), or `None`.
fn chained_base(dir: &Path, branch: &str, e: &Record) -> Result<Option<String>, String> {
    for base in chain_bases_of(e) {
        if chained_onto(dir, branch, &base)? {
            return Ok(Some(base));
        }
    }
    Ok(None)
}

/// LEFT-TRAIN RULE, the test: does `branch` still carry `base` (a commit a merge train chained it onto) while main's HEAD does not?
/// Then the branch holds commits of OTHER entries that are not on main, and a review or merge of it would take their code as this
/// entry's. An empty `base` is never chained; a base that main already holds (those entries merged) is not either.
fn chained_onto(dir: &Path, branch: &str, base: &str) -> Result<bool, String> {
    if base.is_empty() {
        return Ok(false);
    }
    let (_, head) = wtgit::trunk_commit(dir)?;
    let tip = wtgit::branch_tip(dir, branch)?;
    Ok(wtgit::is_ancestor(dir, base, &tip)? && !wtgit::is_ancestor(dir, base, &head)?)
}

/// TRUNK RULE of the merge queue, part 1 (the ONE place every step asks; part 2 is `trunk_checkout`): the project's trunk
/// (`wtgit::trunk_branch`: the one of `main`/`master` that exists) and the commit at `refs/heads/<trunk>`. `recorded` is the trunk
/// the entry started on (`main_branch`; empty before it is recorded): a different trunk now is refused. Every "is it on main" test
/// asks this, never HEAD. Any error (both or neither trunk, git failing) is returned and the caller fails closed.
fn trunk_tip(dir: &Path, recorded: &str) -> Result<(String, String), String> {
    let (trunk, tip) = wtgit::trunk_commit(dir)?;
    if !recorded.is_empty() && recorded != trunk {
        return Err(format!("the project's trunk is `{trunk}` now, but this entry started on `{recorded}`"));
    }
    Ok((trunk, tip))
}

/// TRUNK RULE of the merge queue, part 2: `trunk_tip`, and the project's main checkout has exactly that trunk checked out.
/// DECISION (t-20261120-134432): a checkout on any other branch (or a detached HEAD) is REFUSED, never worked around. The queue
/// moves the trunk only with `git merge --ff-only` in that checkout (which fast-forwards whatever is checked out) and runs the main
/// check on that checkout's files, so both are only right while the trunk is what is checked out. It never updates a branch ref
/// behind a checkout's back and never moves, merges into or switches the checked-out branch. Called before the rebase (its base),
/// before every fast-forward, right after it, and before the main check.
fn trunk_checkout(dir: &Path, recorded: &str) -> Result<(String, String), String> {
    let (trunk, tip) = trunk_tip(dir, recorded)?;
    match wtgit::current_branch(dir)? {
        Some(b) if b == trunk => Ok((trunk, tip)),
        other => {
            let on = other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into());
            Err(format!("the project's main checkout ({}) has {on} checked out, not its trunk `{trunk}`. The merge queue merges only into the trunk, by fast-forwarding it in that checkout, and never moves or merges into any other branch; the checkout was not touched. Check out `{trunk}` there (`git switch {trunk}`), then run the entry again", dir.display()))
        }
    }
}

/// The trunk's name for a message (`<the trunk>` when it cannot be told).
fn trunk_name(dir: &Path) -> String {
    wtgit::trunk_branch(dir).unwrap_or_else(|_| "<the trunk>".into())
}

/// The reason an entry with a chained branch gets (see `Run::leave_train`), with the coder's way out.
fn left_train_text(main: &str, branch: &str, base: &str) -> String {
    format!("left a train: rebase your own commits onto main and queue again. `{branch}` is chained on {}, a commit of other entries of a merge train that is not on `{main}`, so a review or merge of it now would take their code as this entry's. In the task's worktree run `git rebase --onto {main} {base} {branch}`, then run the entry again (`queue run --entry <id>`, or `queue cancel` it and `queue add` the task). If those entries reach `{main}` first, the branch needs nothing and runs as it is", short(base))
}

/// Take the next train out of `approved` (see `pick_train_when`), in queue order; the others stay.
fn take_group<'a>(approved: &mut Vec<(Run<'a>, std::time::Instant)>, max: usize, ready: &dyn Fn(&str) -> bool) -> Option<Vec<(Run<'a>, std::time::Instant)>> {
    let items: Vec<(String, Vec<String>)> = approved.iter().map(|(r, _)| (r.e.get("project").to_string(), r.claims.clone())).collect();
    let idx = pick_train_when(&items, max, ready);
    if idx.is_empty() {
        return None;
    }
    let mut group = Vec::new();
    for i in idx.iter().rev() {
        group.push(approved.remove(*i));
    }
    group.reverse();
    Some(group)
}

/// Entries an earlier `queue run --all` approved and never merged (it died, or stopped): claimed by this process (their pid), oldest
/// first. Under entries.lock; their status stays `approved`.
fn claim_approved(cfg: &Config, lane: &Lane) -> Result<Vec<Record>, ExitCode> {
    let _l = entries_lock(cfg)?;
    let all: Vec<Record> = load_entries(cfg).map_err(|m| fail(EXIT_ERROR, m))?.into_iter().filter(|x| lane.holds(x)).collect();
    let me = std::process::id();
    let mut out = Vec::new();
    // a half-finished merge is not a train candidate: the drain resumes it on its own first (`resume_first`)
    for mut e in all.into_iter().filter(|x| x.status() == APPROVED && !half_merged(x)) {
        e.set("pid", me.to_string());
        e.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
        e.save().map_err(|err| fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display())))?;
        out.push(e);
    }
    Ok(out)
}

fn train_stop_note(code: u8) -> String {
    if code == EXIT_MERGED_BAD {
        "stopped by a merge train (exit 6): main holds the train's commits and the main check failed; a human decides before anything else merges".into()
    } else {
        "stopped by a merge train (exit 1): an internal error or a check that could not run to a verdict; the entries it was working on are `approved` (or `running`), and the next `queue run --all` takes them again".into()
    }
}

/// One entry of a train is finished (merged, rejected, back to queued, ...): its summary row, and the train's worst outcome so far.
fn finish_member(cfg: &Config, r: Run<'_>, code: u8, t: std::time::Instant, rows: &mut Vec<Row>, worst: &mut u8) {
    let id = r.e.id().to_string();
    rows.push(row_for(cfg, &id, code, t));
    if badness(code) > badness(*worst) {
        *worst = code;
    }
}

/// The entries stay `approved` (the reason says why), and the next drain takes them again.
fn leave_approved(cfg: &Config, members: Vec<(Run<'_>, std::time::Instant)>, why: &str, rows: &mut Vec<Row>, worst: &mut u8) {
    for (mut r, t) in members {
        r.e.set("reason", format!("approved, not merged: {why}"));
        let _ = r.log("train", why);
        finish_member(cfg, r, EXIT_RETRY, t, rows, worst);
    }
}

/// A train check gave no verdict because of ONE entry's worktree (`TrainCheck::EntryBlocked`): that entry leaves the train and ends
/// `blocked` with the reason (the left-train rule decides the exact text, `leave_train`), as the single-entry path blocks only its own
/// entry. The `others` merged nothing and stay `approved`: the next drain takes them again without it. The drain is NOT stopped.
fn entry_leaves_blocked(cfg: &Config, train_id: &str, member: (Run<'_>, std::time::Instant), msg: &str, others: Vec<(Run<'_>, std::time::Instant)>, rows: &mut Vec<Row>, worst: &mut u8) {
    let (mut r, t) = member;
    let id = r.e.id().to_string();
    println!("merge train {train_id}: entry {id} cannot be checked ({msg}); it leaves the train `blocked`, the other {} entr{} stay approved for the next drain", others.len(), if others.len() == 1 { "y" } else { "ies" });
    let c = r.leave_train(BLOCKED, &format!("merge train {train_id} could not check this entry: {msg}. Put the task's worktree back on its branch at the reviewed commit with nothing else in it, then run the entry again; nothing of it was merged"), EXIT_RETRY);
    finish_member(cfg, r, code_num(c), t, rows, worst);
    leave_approved(cfg, others, &format!("entry {id} left merge train {train_id} (its worktree was not exactly the commit to check); nothing merged, this entry waits for the next drain"), rows, worst);
}

/// An internal error that hits every member (git or the record could not be used): each stays as it is, with the message.
fn fail_members(cfg: &Config, members: Vec<(Run<'_>, std::time::Instant)>, msg: &str, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    for (mut r, t) in members {
        let c = r.internal(msg);
        finish_member(cfg, r, code_num(c), t, rows, worst);
    }
    EXIT_ERROR
}

/// Entries still `approved` when the drain ends early (it stopped) or no train took them: a row each; the next drain takes them.
fn settle_approved(cfg: &Config, approved: &mut Vec<(Run<'_>, std::time::Instant)>, rows: &mut Vec<Row>) {
    for (r, t) in approved.drain(..) {
        let id = r.e.id().to_string();
        rows.push(row_for(cfg, &id, 0, t));
    }
}

/// An entry this drain may still run (so its project's approved entries may wait for it to fill their train): not taken by this
/// drain yet (`taken`; a taken entry is never run again in the same drain, so waiting for it would be waiting for the end), and
/// queued, running or stale, or a half-finished train merge (`resume_first`): what `next_for_all` can still pick.
fn still_to_run(x: &Record, taken: &[String]) -> bool {
    !taken.iter().any(|t| t == x.id()) && (resume_first(x) || matches!(x.status(), QUEUED | RUNNING | STALE))
}

/// STARVATION RULE, for one project with approved entries waiting: why it gets a train now although the train is not full and the
/// drain has not ended, or None (it keeps waiting for more of its entries). `approved_at` holds each of its approved entries'
/// approval time (unix seconds; None when the entry has none that can be read). A train is due when
///   (a) `pending` is false: no entry of the project is left for this drain to run, so nothing more can join its train; or
///   (b) its oldest approved entry has waited `wait_minutes` (merge_train_wait_minutes) by `now`.
/// An entry with no readable approval time (approved before this rule recorded one) counts as having waited long enough: a short
/// train only merges earlier, through the same chain, check, merge and main check as the flush train of the drain's end. A clock
/// that went back counts as no wait.
fn short_train_why(pending: bool, approved_at: &[Option<u64>], now: u64, wait_minutes: u64) -> Option<String> {
    if approved_at.is_empty() {
        return None;
    }
    if !pending {
        return Some("no other entry of its project is left to run".into());
    }
    if approved_at.iter().any(|a| a.is_none()) {
        return Some("an approved entry has no readable approval time (`approved_at`), so it counts as having waited merge_train_wait_minutes".into());
    }
    let oldest = approved_at.iter().flatten().min().copied().unwrap_or(now);
    let waited = now.saturating_sub(oldest);
    (waited >= wait_minutes.saturating_mul(60)).then(|| format!("its oldest approved entry has waited {} s (merge_train_wait_minutes = {wait_minutes})", waited))
}

/// The projects that get a short train now (`short_train_why`), each with the reason, in the order of `approved` (project, approval
/// time). `all` is every queue entry (to see what is still to run), `taken` the entries this drain took.
fn short_train_projects_in(all: &[Record], approved: &[(String, Option<u64>)], taken: &[String], now: u64, wait_minutes: u64) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (project, _) in approved {
        if out.iter().any(|(p, _)| p == project) {
            continue;
        }
        let times: Vec<Option<u64>> = approved.iter().filter(|(p, _)| p == project).map(|(_, a)| *a).collect();
        let pending = all.iter().any(|x| x.get("project") == project && still_to_run(x, taken));
        if let Some(why) = short_train_why(pending, &times, now, wait_minutes) {
            out.push((project.clone(), why));
        }
    }
    out
}

/// `short_train_projects_in` on the queue as it is now (read under entries.lock) and the clock.
fn short_train_projects(cfg: &Config, approved: &[(Run<'_>, std::time::Instant)], taken: &[String]) -> Result<Vec<(String, String)>, String> {
    let all = {
        let _l = entries_lock(cfg).map_err(|c| format!("the queue's entries lock could not be taken (exit {})", code_num(c)))?;
        load_entries(cfg)?
    };
    let items: Vec<(String, Option<u64>)> = approved.iter().map(|(r, _)| (r.e.get("project").to_string(), r.e.get_num("approved_at"))).collect();
    Ok(short_train_projects_in(&all, &items, taken, now_secs(), cfg.merge_train_wait_minutes))
}

/// Run trains while `approved` holds a ready one. `flush` (the drain's end): every project is ready. Otherwise a project's train is
/// ready when it is full, or when the starvation rule says so (`short_train_projects`: none of its entries is left to run, or its
/// oldest approved entry waited merge_train_wait_minutes). A short train is exactly a flush train of that project. `taken`: the
/// entries this drain took. Returns 0, or EXIT_ERROR / EXIT_MERGED_BAD when the drain must stop.
fn run_trains<'a>(cfg: &'a Config, approved: &mut Vec<(Run<'a>, std::time::Instant)>, flush: bool, taken: &[String], rows: &mut Vec<Row>) -> u8 {
    let max = cfg.merge_train_max as usize;
    let mut hard = 0u8;
    while hard == 0 {
        let short = if flush {
            Vec::new()
        } else {
            match short_train_projects(cfg, approved, taken) {
                Ok(s) => s,
                Err(m) => {
                    // fail closed: no train is started on a queue that could not be read; the entries stay approved
                    eprintln!("broadsword: queue run --all: cannot tell which merge trains are due: {m}");
                    return EXIT_ERROR;
                }
            }
        };
        let ready = |p: &str| flush || short.iter().any(|(s, _)| s == p);
        let Some(group) = take_group(approved, max, &ready) else { break };
        if !flush && group.len() < max {
            let project = group[0].0.e.get("project").to_string();
            let why = short.iter().find(|(s, _)| *s == project).map(|(_, w)| w.as_str()).unwrap_or("-");
            println!("merge train for project {project}: {} of at most {max} entries, not waiting for more: {why}", group.len());
        }
        hard = run_train(cfg, group, rows);
    }
    hard
}

/// The commit a chain entry's own check proves (its tip in the chain), for the 1-based position `i`.
fn chain_tip(chain: &[(Run<'_>, std::time::Instant)], i: usize) -> String {
    chain[i - 1].0.e.get("checked_tip").to_string()
}

/// ONE MERGE TRAIN. `group` are approved entries of one project with claims that do not overlap (`pick_train`).
///
/// 1. Each entry is checked to still be exactly its reviewed commit (`train_ready`).
/// 2. CHAIN: each entry's own commits are rebased onto the previous entry's tip, the first onto main's HEAD (`train_link`). The result
///    is one straight line `main <- e1 <- e2 <- ... <- en`, every entry's branch tip is on it, and the tip of entry i contains the
///    entries before it. An entry that cannot be chained (a textual conflict), or whose rebased commits are not the same patches as
///    the reviewed ones, goes back to `queued`; the rest go on.
/// 3. ONE CHECK of the chain tip (`Run::check_commit`).
/// 4. PASS: ONE fast-forward of main to the tip (every entry's commits as they are, in order, no squash), ONE main check (`s_main_check`,
///    run by the last entry), then every entry is `merged` (`task done`), recording train id, position and the shared check job.
/// 5. FAIL: BISECT. The check of entry i's tip is a check of the first i entries, so a binary search over the prefixes finds the first
///    entry k whose prefix fails (about log2(n) checks; the prefix before it passed). The culprit is then checked on its own commit
///    (`check_alone`): if it fails alone it is `check_failed` with that check's log, the proven-good entries before it merge as a
///    smaller train, and the entries after it start a new round on the new main. If it passes alone the failure only shows when it is
///    combined with the entries before it: the culprit AND every entry before it in the chain (the partner is not searched for) are
///    held out with the combined failure's log, and NOTHING is merged for them. COMBINED-ONLY PAIR, DECIDED (round 3, kept as the brief
///    says: "neither merges"; the proven prefix is NOT merged): each held entry asks for `conflict`, and the left-train rule
///    (`leave_train`) decides what it ends as. The first entry of the chain sits on main's HEAD and ends a plain `conflict`; every later
///    one still carries the earlier held entries' commits, which are not on main, so it ends `blocked` ("left a train: ... it would
///    have been `conflict`", with the rebase command). So for a pair e1, e2: e1 `conflict`, e2 `blocked`, main unchanged. Merging the
///    proven prefix instead would turn e2 into a plain `conflict` but put e1 on main while its partner is unknown; that is a later
///    choice for the measurement window, not this phase. Every round removes at least one entry, so it ends.
/// 6. A main check that fails after the fast-forward: every entry of the merge is `merged_check_failed` (exit 6), main is NOT reset, the
///    drain stops. A check that cannot run to a verdict leaves all its entries `approved` and stops the drain, unless the reason is
///    one entry's own worktree (`TrainCheck::EntryBlocked`): then only that entry ends `blocked`, the rest stay `approved` for the
///    next drain, and this drain goes on.
///
/// Returns 0, EXIT_ERROR or EXIT_MERGED_BAD (the drain stops on the last two).
fn run_train<'a>(cfg: &'a Config, group: Vec<(Run<'a>, std::time::Instant)>, rows: &mut Vec<Row>) -> u8 {
    static TRAIN_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let train_id = format!("tr-{}-{}", job::file_stamp(now_secs()), TRAIN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let started = std::time::Instant::now();
    let size = group.len();
    println!("merge train {train_id}: {size} approved entr{} (merge_train_max = {})", if size == 1 { "y" } else { "ies" }, cfg.merge_train_max);
    let mut worst = 0u8;
    let mut hard = 0u8;
    let mut checks = 0u32;
    let mut todo: Vec<(Run<'a>, std::time::Instant)> = Vec::new();
    for (mut r, t) in group {
        match r.train_ready() {
            Ok(()) => todo.push((r, t)),
            Err(c) => {
                let code = code_num(c);
                finish_member(cfg, r, code, t, rows, &mut worst);
            }
        }
    }
    let mut round = 0u32;
    while !todo.is_empty() && hard == 0 {
        round += 1;
        let dir = todo[0].0.dir.clone();
        // the chain's base is the trunk's tip, and the trunk must be what the main checkout has checked out (`trunk_checkout`)
        let head = match trunk_checkout(&dir, todo[0].0.e.get("main_branch")) {
            Ok((_, h)) => h,
            Err(m) => {
                for (mut r, t) in std::mem::take(&mut todo) {
                    let c = r.leave_train(BLOCKED, &format!("{m}; nothing merged"), EXIT_RETRY);
                    finish_member(cfg, r, code_num(c), t, rows, &mut worst);
                }
                break;
            }
        };
        // ---- chain
        let mut chain: Vec<(Run<'a>, std::time::Instant)> = Vec::new();
        let mut prev = head.clone();
        let mut pending: std::collections::VecDeque<(Run<'a>, std::time::Instant)> = std::mem::take(&mut todo).into();
        while let Some((mut r, t)) = pending.pop_front() {
            match r.train_link(&prev, &train_id) {
                Ok(Some(tip)) => {
                    prev = tip;
                    chain.push((r, t));
                }
                Ok(None) => finish_member(cfg, r, EXIT_RETRY, t, rows, &mut worst),
                Err(c) => {
                    let code = code_num(c);
                    finish_member(cfg, r, code, t, rows, &mut worst);
                    if code == EXIT_ERROR {
                        hard = EXIT_ERROR;
                        break;
                    }
                }
            }
        }
        if hard != 0 {
            let rest: Vec<(Run<'a>, std::time::Instant)> = chain.into_iter().chain(pending).collect();
            leave_approved(cfg, rest, "the merge train stopped on an internal error of an earlier entry", rows, &mut worst);
            break;
        }
        if chain.is_empty() {
            continue;
        }
        let m = chain.len();
        for (i, (r, _)) in chain.iter_mut().enumerate() {
            r.e.set("train", train_id.clone());
            r.e.set("train_pos", (i + 1).to_string());
            let _ = r.save();
        }
        // ---- one check of the whole chain, then a bisect if it fails
        let mut lo = 0usize; // the first `lo` entries are proven good by a check that passed (their tip's)
        let mut hi = m; // the check of the first `hi` entries failed
        let mut good = PassInfo::default();
        let mut bad = PassInfo::default();
        let mut blocked: Option<String> = None;
        // a check that gave no verdict because of ONE entry's worktree: that entry's 1-based position in the chain, and why
        let mut entry_blocked: Option<(usize, String)> = None;
        let mut bisect_checks = 0u32;
        let mut bisect_secs = 0f64;
        let tip = chain_tip(&chain, m);
        let first = chain[m - 1].0.check_commit(&tip, &format!("merge train {train_id}: all {m} entries combined"));
        checks += 1;
        match first {
            TrainCheck::Pass { job, summary, tail } => {
                lo = m;
                good = PassInfo { job, summary, tail, perf: String::new() };
            }
            TrainCheck::Fail { job, summary, tail, perf } => bad = PassInfo { job, summary, tail, perf },
            TrainCheck::Blocked(msg) => blocked = Some(msg),
            TrainCheck::EntryBlocked(msg) => entry_blocked = Some((m, msg)),
        }
        while blocked.is_none() && entry_blocked.is_none() && hi - lo > 1 {
            let mid = (lo + hi) / 2;
            let tip_mid = chain_tip(&chain, mid);
            let tb = std::time::Instant::now();
            let res = chain[mid - 1].0.check_commit(&tip_mid, &format!("bisect of merge train {train_id}: the first {mid} entries"));
            bisect_checks += 1;
            checks += 1;
            bisect_secs += tb.elapsed().as_secs_f64();
            match res {
                TrainCheck::Pass { job, summary, tail } => {
                    lo = mid;
                    good = PassInfo { job, summary, tail, perf: String::new() };
                }
                TrainCheck::Fail { job, summary, tail, perf } => {
                    hi = mid;
                    bad = PassInfo { job, summary, tail, perf };
                }
                TrainCheck::Blocked(msg) => blocked = Some(msg),
                TrainCheck::EntryBlocked(msg) => entry_blocked = Some((mid, msg)),
            }
        }
        if bisect_checks > 0 {
            println!("merge train {train_id}: bisect: {bisect_checks} extra check(s) in {bisect_secs:.1} s; the first {lo} entr{} pass, entry {hi} makes it fail", if lo == 1 { "y" } else { "ies" });
        }
        let stats = format!("round {round}: {} check(s) of the chain, {bisect_checks} of them bisect, {bisect_secs:.1} s of bisect", 1 + bisect_checks);
        for (r, _) in chain.iter_mut() {
            r.e.set("train_bisect", stats.clone());
        }
        if let Some(msg) = blocked {
            println!("merge train {train_id}: the check could not give a verdict ({msg}); the {m} entries stay approved");
            leave_approved(cfg, chain, &format!("the merge train's check could not run to a verdict: {msg}"), rows, &mut worst);
            hard = EXIT_ERROR;
            break;
        }
        if let Some((pos, msg)) = entry_blocked {
            // ONE entry's worktree is the reason (see `TrainCheck::EntryBlocked`): only it leaves, the rest wait, the drain goes on
            let member = chain.remove(pos - 1);
            entry_leaves_blocked(cfg, &train_id, member, &msg, chain, rows, &mut worst);
            break;
        }
        if lo == m {
            // ---- the whole chain passed
            let c = merge_members(cfg, &train_id, chain, &tip, &head, &good, rows, &mut worst);
            if c != 0 {
                hard = c;
            }
            continue;
        }
        // ---- entry `hi` (1-based) is the first whose prefix fails
        let k = hi;
        let after: Vec<(Run<'a>, std::time::Instant)> = chain.split_off(k);
        let Some((mut culprit, ct)) = chain.pop() else {
            hard = EXIT_ERROR;
            break;
        };
        let alone = if k == 1 {
            Alone::Bad(bad.clone()) // its tip IS its own commit on main: the check that failed was already the check on its own
        } else {
            checks += 1;
            match culprit.check_alone() {
                TrainCheck::Fail { job, summary, tail, perf } => Alone::Bad(PassInfo { job, summary, tail, perf }),
                TrainCheck::Pass { job, summary, .. } => {
                    let _ = culprit.log("train", &format!("checked on its own commit: passes (job {job}: {summary}); so the failure only shows when it is combined with the entries before it"));
                    Alone::Fine
                }
                TrainCheck::Blocked(msg) => Alone::Unknown(msg),
                TrainCheck::EntryBlocked(msg) => Alone::EntryBlocked(msg),
            }
        };
        let culprit_id = culprit.e.id().to_string();
        match alone {
            Alone::EntryBlocked(msg) => {
                let mut rest = chain;
                rest.extend(after);
                entry_leaves_blocked(cfg, &train_id, (culprit, ct), &msg, rest, rows, &mut worst);
                break;
            }
            Alone::Unknown(msg) => {
                println!("merge train {train_id}: entry {culprit_id} could not be checked on its own ({msg}); every entry stays approved");
                let mut all = chain;
                all.push((culprit, ct));
                all.extend(after);
                leave_approved(cfg, all, &format!("the merge train's check of {culprit_id} on its own could not run to a verdict: {msg}"), rows, &mut worst);
                hard = EXIT_ERROR;
                break;
            }
            Alone::Bad(info) => {
                let failed = if info.perf.is_empty() { CHECK_FAILED } else { PERF_FAILED };
                println!("merge train {train_id}: entry {culprit_id} fails on its own (job {}): {failed}; the {} entr{} before it merge", info.job, lo, if lo == 1 { "y" } else { "ies" });
                culprit.e.set("check_job", info.job.clone());
                culprit.e.set("check_exit", if info.perf.is_empty() { "1" } else { "0" });
                culprit.e.set("check_summary", info.summary.clone());
                culprit.e.set("check_tail", info.tail.clone());
                // the proven-good entries before it merge FIRST: then a culprit chained only on them sits on main and ends a plain
                // `check_failed`; if they could not merge, its branch still carries them and it ends `blocked` (`leave_train`)
                if lo > 0 {
                    let tip_good = chain_tip(&chain, lo);
                    let c = merge_members(cfg, &train_id, chain, &tip_good, &head, &good, rows, &mut worst);
                    if c != 0 {
                        hard = c;
                    }
                }
                let job_shown = if info.job.is_empty() { "?" } else { info.job.as_str() };
                let why = if info.perf.is_empty() {
                    format!("build or tests failed on this entry on its own (job {job_shown}): {}", info.summary.replace('\n', "; "))
                } else {
                    format!("the build and tests passed on this entry on its own (job {job_shown}) but the perf gate failed: {}", info.perf)
                };
                let c = culprit.leave_train(failed, &format!("{why}. Found by the bisect of merge train {train_id}; nothing of it was merged. Fix it, commit, and queue the task again with `queue add`"), EXIT_REJECTED);
                finish_member(cfg, culprit, code_num(c), ct, rows, &mut worst);
                if hard != 0 {
                    leave_approved(cfg, after, "the merge train stopped before this entry's turn", rows, &mut worst);
                    break;
                }
                todo = after;
            }
            Alone::Fine => {
                // COMBINED-ONLY (see 5 above): all held out; the first ends `conflict`, the ones chained on it `blocked` (left-train rule)
                println!("merge train {train_id}: entry {culprit_id} passes on its own, but fails combined with the entries before it: conflict for it and for those {lo} entr{}; nothing of them is merged", if lo == 1 { "y" } else { "ies" });
                let mut held = chain;
                held.push((culprit, ct));
                let ids: Vec<String> = held.iter().map(|(r, _)| r.e.id().to_string()).collect();
                for (mut r, t) in held {
                    r.e.set("check_job", bad.job.clone());
                    r.e.set("check_exit", "1");
                    r.e.set("check_summary", bad.summary.clone());
                    r.e.set("check_tail", bad.tail.clone());
                    let perf_note = if bad.perf.is_empty() { String::new() } else { format!(" (it was the perf gate that failed: {})", bad.perf) };
                    let reason = format!("the combined check of merge train {train_id} failed (job {}){perf_note} although each of these entries passes on its own: {}. They only fail together, so none of them was merged (main is unchanged). The entry's check summary and tail are the combined failure's. Rebase or fix them yourself, commit, and queue them again with `queue add`", if bad.job.is_empty() { "?" } else { bad.job.as_str() }, ids.join(", "));
                    let c = r.leave_train(CONFLICT, &reason, EXIT_RETRY);
                    finish_member(cfg, r, code_num(c), t, rows, &mut worst);
                }
                todo = after;
            }
        }
    }
    println!("merge train {train_id}: finished in {:.1} s; {size} entr{} taken, {checks} check(s) of the train, worst exit {worst}", started.elapsed().as_secs_f64(), if size == 1 { "y" } else { "ies" });
    hard
}

/// Fast-forward main to `tip` (the last of `members`, a chain whose tip passed its check), run ONE main check, mark every member
/// merged. Returns 0, EXIT_ERROR (internal), or EXIT_MERGED_BAD (the main check failed after the fast-forward: loud, main not reset).
#[allow(clippy::too_many_arguments)]
fn merge_members(cfg: &Config, train_id: &str, members: Vec<(Run<'_>, std::time::Instant)>, tip: &str, head: &str, pass: &PassInfo, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    let n = members.len();
    if n == 0 {
        return 0;
    }
    let dir = members[0].0.dir.clone();
    let trunk = members[0].0.e.get("main_branch").to_string();
    // the trunk, checked out in the main checkout, is what is fast-forwarded (`trunk_checkout`); anything else merges nothing
    let now = match trunk_checkout(&dir, &trunk) {
        Ok((_, h)) => h,
        Err(m) => {
            for (mut r, t) in members {
                let c = r.leave_train(BLOCKED, &format!("{m}; merge train {train_id} merged nothing"), EXIT_RETRY);
                finish_member(cfg, r, code_num(c), t, rows, worst);
            }
            return 0;
        }
    };
    if now != head {
        for (mut r, t) in members {
            let id = r.e.id().to_string();
            let c = r.leave_train(STALE, &format!("main moved from {} to {} while merge train {train_id} was being checked; nothing was merged. Run `queue run --entry {id}` to rebase again", short(head), short(&now)), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return 0;
    }
    let dirty = match wtgit::status_porcelain_plain(&dir) {
        Ok(d) => d,
        Err(m) => return fail_members(cfg, members, &m, rows, worst),
    };
    if !dirty.is_empty() {
        for (mut r, t) in members {
            let c = r.leave_train(BLOCKED, &format!("the main checkout got uncommitted or untracked changes while merge train {train_id} was checked ({}); nothing merged", some(&dirty)), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return 0;
    }
    // BEFORE the fast-forward: every member records step `merge`, so a drain killed from here on leaves each one a half-finished
    // merge (`half_merged`) that the next run resumes at its step, never one that is chained into a new train while its commits may
    // already be on main. A member whose record cannot be written stops the train before main moves.
    if let Some(why) = lane_hold(cfg, members[0].0.e.get("project")) {
        for (mut r, t) in members {
            let c = r.leave_train(BLOCKED, &format!("lane {} held: {why}; merge train {train_id} merged nothing. Clear with a passing check of main, then queue run", r.e.get("project")), EXIT_RETRY);
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return 0;
    }
    let mut members = members;
    for (r, _) in members.iter_mut() {
        r.e.set("step", "merge");
        // AUTO-REVERT: the trunk tip before the train's fast-forward; `pre_merge_sha..<train tip>` is exactly what the train merged
        r.e.set("pre_merge_sha", head);
        if r.save().is_err() {
            return fail_members(cfg, members, &format!("could not record the merge step of merge train {train_id} before its fast-forward; main was not moved"), rows, worst);
        }
    }
    if let Err(m) = wtgit::merge_ff_only(&dir, tip) {
        let after = trunk_tip(&dir, "").map(|(_, t)| t).unwrap_or_default();
        for (mut r, t) in members {
            let id = r.e.id().to_string();
            let c = if after != head {
                r.leave_train(STALE, &format!("main moved from {} to {} during the merge of train {train_id}; nothing was merged by this run. Run `queue run --entry {id}` to rebase again", short(head), short(&after)), EXIT_RETRY)
            } else {
                r.leave_train(BLOCKED, &format!("git refused the fast-forward of train {train_id}: {m}. Main is unchanged"), EXIT_RETRY)
            };
            finish_member(cfg, r, code_num(c), t, rows, worst);
        }
        return 0;
    }
    let after = match trunk_checkout(&dir, &trunk) {
        Ok((_, h)) => h,
        Err(m) => return fail_members(cfg, members, &m, rows, worst),
    };
    if after != tip {
        let msg = format!("after `git merge --ff-only` the trunk `{trunk}` is at {} but the train tip is {}; it was {} before; look at the project", short(&after), short(tip), short(head));
        return fail_members(cfg, members, &msg, rows, worst);
    }
    let last = n - 1;
    let last_id = members[last].0.e.id().to_string();
    for (i, (r, _)) in members.iter_mut().enumerate() {
        let own = r.e.get("reviewed_tip").to_string();
        let cli = r.e.get("reviewer_cli").to_string();
        r.e.set("main_sha", own);
        r.e.set("train", train_id);
        r.e.set("train_pos", (i + 1).to_string());
        r.e.set("train_job", pass.job.clone());
        r.e.set("check_job", pass.job.clone());
        r.e.set("check_exit", "0");
        r.e.set("check_summary", pass.summary.clone());
        r.e.set("check_tail", pass.tail.clone());
        r.e.set("check_deferred", "");
        r.e.set("train_checked", tip);
        r.e.set("main_check", if i == last { format!("full check of the train tip {} on main, run by this entry for the whole merge train {train_id}", short(tip)) } else { format!("one main check for the whole merge train {train_id}, run by its last entry {last_id}") });
        r.e.set("step", "merge");
        let _ = r.log("merge", &format!("merge train {train_id}, position {} of {n}: main fast-forwarded {} -> {} (the train tip, checked by job {}); reviewed through CLI {}", i + 1, short(head), short(tip), if pass.job.is_empty() { "?" } else { pass.job.as_str() }, if cli.is_empty() { "(none recorded)" } else { cli.as_str() }));
    }
    println!("merge train {train_id}: main fast-forwarded {} -> {} ({n} entr{}, one check, one fast-forward)", short(head), short(tip), if n == 1 { "y" } else { "ies" });
    // TP2 SEAM: tp2 (main == checked commit -> only a build on main) is not merged at this base, so this ONE main check is the full
    // `check` of main, run by the last entry (its commit is the tip); the others record that it covers them.
    if let Err(c) = members[last].0.s_main_check() {
        let code = code_num(c);
        let reason = members[last].0.e.get("reason").to_string();
        for (i, (mut r, t)) in members.into_iter().enumerate() {
            if i == last {
                finish_member(cfg, r, code, t, rows, worst);
                continue;
            }
            let c2 = if code == EXIT_MERGED_BAD { r.end(MERGED_CHECK_FAILED, &reason, EXIT_MERGED_BAD) } else { r.internal(&format!("the main check of merge train {train_id} did not complete: {reason}")) };
            finish_member(cfg, r, code_num(c2), t, rows, worst);
        }
        // the members' rows keep their exit 6; only a revert that left main green lets the drain go on (`green_revert`)
        return if code == EXIT_MERGED_BAD && green_revert(cfg, &last_id) { 0 } else { code };
    }
    let mut hard = 0u8;
    for (i, (mut r, t)) in members.into_iter().enumerate() {
        if i != last {
            r.e.set("step", "main_check");
            let _ = r.log("main_check", &format!("covered by the one main check of merge train {train_id} (run by entry {last_id}), which passed"));
        }
        let code = code_num(match r.s_done() {
            Ok(c) | Err(c) => c,
        });
        if code == EXIT_ERROR {
            hard = EXIT_ERROR;
        }
        finish_member(cfg, r, code, t, rows, worst);
    }
    hard
}

/// The last `max` bytes of a file as text (empty if it cannot be read). Cut at a line start when it cut anything.
/// The brief as the reviewer gets it: the brief, then the done check the queue ran (`s_done_check`).
fn with_done_check(brief: &str, done_check: &str) -> String {
    let body = if done_check.is_empty() { "not recorded for this entry (its check step ran before the queue ran done checks); nothing was run by the queue" } else { done_check };
    format!("{brief}\n\nDONE CHECK, RUN BY THE QUEUE (the brief's done-check commands that the queue may run were run by it on the reviewed commit, and this is their real output; a line marked NOT RUN was not run by anyone here, so do not take it as passed):\n{body}")
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
    fn same_model_ignores_case_and_space() {
        assert!(same_model("Opus", " opus "));
        assert!(!same_model("haiku", "opus"));
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

    #[test]
    fn steps_are_in_the_documented_order() {
        assert_eq!(STEPS, ["preconditions", "rebase", "check", "review", "merge", "main_check", "done"]);
        assert_eq!(STEPS[STEP_MERGE], "merge");
        assert!(is_open(APPROVED), "an approved entry is open: its task cannot be queued again");
        assert!(is_open(QUEUED) && is_open(RUNNING) && is_open(BLOCKED) && is_open(STALE) && !is_open(MERGED) && !is_open(CANCELLED) && !is_open(REJECTED) && !is_open(CHECK_FAILED) && !is_open(CONFLICT));
    }

    fn item(project: &str, claims: &[&str]) -> (String, Vec<String>) {
        (project.to_string(), claims.iter().map(|c| c.to_string()).collect())
    }

    #[test]
    fn a_train_takes_only_entries_with_claims_that_do_not_overlap_and_only_from_one_project() {
        let items = vec![item("p", &["src/a.rs"]), item("p", &["src"]), item("q", &["src/a.rs"]), item("p", &["docs"]), item("p", &["src/b.rs"])];
        // `src` overlaps `src/a.rs` (and `src/b.rs`), so the second entry is left out; the other project's entry never joins
        assert_eq!(pick_train(&items, 3, false), vec![0, 3, 4], "full train: a, docs, b");
        assert_eq!(pick_train(&items, 4, false), Vec::<usize>::new(), "only 3 compatible entries: not full, so not ready without flush");
        assert_eq!(pick_train(&items, 4, true), vec![0, 3, 4], "flush takes what there is");
        // a task that could not be read claims everything and rides alone
        let items = vec![item("p", &["*"]), item("p", &["src/a.rs"])];
        assert_eq!(pick_train(&items, 2, false), Vec::<usize>::new());
        assert_eq!(pick_train(&items, 2, true), vec![0]);
        // two projects: the first project with a ready train goes first
        let items = vec![item("p", &["a"]), item("q", &["a"]), item("q", &["b"])];
        assert_eq!(pick_train(&items, 2, false), vec![1, 2]);
        assert_eq!(pick_train(&[], 2, true), Vec::<usize>::new());
    }

    #[test]
    fn a_train_uses_key_claim_overlap_without_a_separate_rule() {
        for file in ["broadsword.toml", "crates/broadsword/src/config.rs"] {
            let alpha = format!("{file}#alpha");
            let beta = format!("{file}#beta");
            let items = vec![item("p", &[&alpha]), item("p", &[&beta]), item("p", &[&alpha]), item("p", &[file]), item("p", &["*"])];
            assert_eq!(pick_train(&items, 2, false), vec![0, 1], "different keys share a full train");
            assert_eq!(pick_train(&items, 3, true), vec![0, 1], "same key, whole file and wildcard stay out");
            let whole_first = vec![item("p", &[file]), item("p", &[&alpha]), item("p", &[&beta])];
            assert_eq!(pick_train(&whole_first, 3, true), vec![0], "a whole-file claim rides alone");
        }
    }

    #[test]
    fn a_train_never_exceeds_merge_train_max() {
        let items: Vec<(String, Vec<String>)> = (0..6)
            .map(|i| {
                let f = format!("f{i}");
                item("p", &[f.as_str()])
            })
            .collect();
        assert_eq!(pick_train(&items, 4, false), vec![0, 1, 2, 3]);
        assert_eq!(pick_train(&items, 1, false), vec![0], "1 = every entry on its own");
    }

    fn entry(id: &str, project: &str, status: &str) -> Record {
        Record::parse(Path::new(&format!("{id}.entry")), &format!("id = {id}\nproject = {project}\nstatus = {status}\n"))
    }

    #[test]
    fn a_project_with_no_entry_left_to_run_gets_a_short_train_while_another_project_still_has_queued_entries() {
        // crash-sim has 2 approved entries and nothing left to run; broadsword has 1 approved and more queued (the 2026-11-20 case)
        let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c2", "crash-sim", APPROVED), entry("q-b1", "broadsword", APPROVED), entry("q-b2", "broadsword", QUEUED)];
        let approved = vec![("crash-sim".to_string(), Some(1000)), ("crash-sim".to_string(), Some(1010)), ("broadsword".to_string(), Some(1020))];
        let taken: Vec<String> = ["q-c1", "q-c2", "q-b1"].iter().map(|s| s.to_string()).collect();
        let short = short_train_projects_in(&all, &approved, &taken, 1030, 20);
        assert_eq!(short.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["crash-sim"], "{short:?}");
        assert!(short[0].1.contains("no other entry"), "{short:?}");
        // the train that forms is BOTH crash-sim entries (a train of 2 of at most 4), not one, and broadsword's waits
        let items = vec![item("crash-sim", &["a"]), item("crash-sim", &["b"]), item("broadsword", &["c"])];
        let ready = |p: &str| short.iter().any(|(s, _)| s == p);
        assert_eq!(pick_train_when(&items, 4, &ready), vec![0, 1]);
        // what still counts as left to run: queued, stale, running (interrupted) and a half-finished train merge, never a taken entry
        for (status, left) in [(QUEUED, true), (STALE, true), (RUNNING, true), (BLOCKED, false), (REJECTED, false), (MERGED, false), (APPROVED, false)] {
            let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c3", "crash-sim", status)];
            let got = short_train_projects_in(&all, &approved[..1], &["q-c1".to_string()], 1030, 20);
            assert_eq!(got.is_empty(), left, "{status}: {got:?}");
            let got = short_train_projects_in(&all, &approved[..1], &["q-c1".to_string(), "q-c3".to_string()], 1030, 20);
            assert_eq!(got.len(), 1, "{status} but already taken by this drain: not waited for: {got:?}");
        }
        let mut half = entry("q-c3", "crash-sim", APPROVED);
        half.set("step", "main_check");
        assert!(short_train_projects_in(&[entry("q-c1", "crash-sim", APPROVED), half], &approved[..1], &["q-c1".to_string()], 1030, 20).is_empty(), "a half-finished train merge is still to run");
        // another project's queued entry never holds a project back
        assert_eq!(short_train_projects_in(&[entry("q-x", "other", QUEUED)], &approved[..1], &[], 1030, 20).len(), 1);
        // no approved entry: no train
        assert!(short_train_projects_in(&all, &[], &taken, 1030, 20).is_empty());
    }

    #[test]
    fn the_wait_rule_fires_once_the_oldest_approved_entry_has_waited_merge_train_wait_minutes() {
        // entries of the project are still queued, so only the wait rule can start its train; `now` is the injected clock
        let at = [Some(10_000u64), Some(10_500)];
        assert_eq!(short_train_why(true, &at, 10_000 + 20 * 60 - 1, 20), None, "one second short of 20 minutes");
        let why = short_train_why(true, &at, 10_000 + 20 * 60, 20).expect("20 minutes after the OLDEST approval");
        assert!(why.contains("1200 s") && why.contains("merge_train_wait_minutes = 20"), "{why}");
        assert!(short_train_why(true, &at, 10_000 + 5 * 60, 5).is_some(), "the setting is read, not a constant");
        assert!(short_train_why(true, &at, 10_000 + 5 * 60 - 1, 5).is_none());
        assert_eq!(short_train_why(true, &[Some(10_000)], 9_000, 20), None, "a clock that went back is no wait");
        assert!(short_train_why(true, &[Some(10_000), None], 10_001, 20).is_some(), "no readable approval time: counts as waited");
        assert_eq!(short_train_why(true, &[], u64::MAX, 20), None, "nothing approved");
        assert!(short_train_why(true, &[Some(0)], u64::MAX - 1, u64::MAX).is_none(), "a huge setting saturates instead of overflowing");
        // through the projects function and the real decision: queued work of the same project, the wait elapsed
        let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c2", "crash-sim", QUEUED)];
        let approved = vec![("crash-sim".to_string(), Some(10_000))];
        let taken = vec!["q-c1".to_string()];
        assert!(short_train_projects_in(&all, &approved, &taken, 10_000 + 19 * 60, 20).is_empty());
        let got = short_train_projects_in(&all, &approved, &taken, 10_000 + 20 * 60, 20);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].1.contains("has waited"), "{got:?}");
    }

    #[test]
    fn run_all_badness_orders_exit_codes_worst_first() {
        let mut codes = vec![0u8, 4, 2, 3, 5, 1, 6];
        codes.sort_by_key(|c| std::cmp::Reverse(badness(*c)));
        assert_eq!(codes, vec![6, 1, 5, 3, 2, 4, 0]);
    }

    #[test]
    fn run_all_exit_counts_the_stop_code_even_when_no_row_carries_it() {
        // a train check without a verdict: its entries stay approved (rows exit 3), the drain stopped with 1
        assert_eq!(worst_exit([3u8, 3, 0].into_iter(), EXIT_ERROR), EXIT_ERROR);
        assert_eq!(worst_exit([5u8, 3].into_iter(), EXIT_ERROR), EXIT_ERROR, "1 is worse than rejected / check_failed");
        assert_eq!(worst_exit([6u8, 3].into_iter(), EXIT_ERROR), EXIT_MERGED_BAD, "merged_check_failed stays the worst");
        assert_eq!(worst_exit([3u8, 0].into_iter(), 0), 3, "no stop: the worst row");
        assert_eq!(worst_exit(std::iter::empty(), 0), 0);
    }
}
