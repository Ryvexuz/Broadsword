//! The merge queue's PERF GATE: after the check of a commit passes, the project's timing bench runs on that commit AND on the
//! project's trunk tip in the same run (A/B), and a case where the entry is slower than main by more than the case allows fails the
//! entry (`perf_failed`).
//!
//! WHICH PROJECTS. A queue entry touches exactly one project: the one it was queued for (its branch lives in that project's repo).
//! The gate looks only at that project's baseline, `<state_dir>/perf/<project>-baseline.json`. A project with no such file is
//! SKIPPED, and the skip is printed and logged in the entry; no bench runs for it, and no other project's bench ever runs.
//!
//! WHY A/B. A stored baseline goes stale and moves with machine load (crash-sim main's box case went 1.566 -> 2.220 ms, +42%, on
//! load alone), so the gate times main and the entry in the same window instead: load slows both alike, and only their RATIO is
//! judged. The baseline file now supplies only the cases, each case's `threshold_pct` and `runs_per_case`; its `median_ms` values
//! are still checked (a broken file fails closed) and kept as a record, but nothing is judged against them.
//!
//! NOT JUDGED. When the entry's commit differs from the trunk tip only in documentation (`git diff --name-only --no-renames` of the
//! two commits: every changed path is under `docs/` or ends in `.md`, and none is under `src/`, `examples/` or `benches/`), the
//! gate is SKIPPED as "not judged: no benched source changed" and nothing is built or benched. This is an ALLOW-LIST of what cannot
//! change the bench, not a list of what can: the crash sim's bench also reads `scenarios/` at run time, so every path that is not
//! documentation (scenarios, Cargo.toml, Cargo.lock, build.rs, `.cargo/`, anything new) is judged. Limit: a Rust file that
//! `include_str!`s a `.md` outside `src/`, `examples/` and `benches/` is not seen.
//!
//! FAIL CLOSED. A baseline file that exists but cannot be read, is not valid JSON, or lacks a field the gate needs is a FAILURE
//! (`perf_failed`), never a pass and never a skip. So is a bench that cannot be built or exits non-zero (on main or on the entry),
//! prints no case line, prints a line that is not a valid case, or leaves out a case the baseline has, and a case whose main median
//! is 0 ms. A bench job that never ran (refused by the disk guard, the build lock or a pause), and a state the gate cannot trust
//! (the main checkout not on the trunk tip or not clean, a checkout that moved during the gate, the build folder rebuilt by another
//! job before the bench was copied out, git failing) gives NO verdict: the entry is `blocked` (or, in a train, every entry stays
//! `approved`), so it is never merged either.
//!
//! THE BENCH, A and B. A is the project's MAIN CHECKOUT, which must have the trunk tip checked out and be clean (the queue's own
//! precondition for merging); B is the task's worktree, at the commit the check just passed. Each side is BUILT ONCE by a `run` job
//! (`broadsword run <project> [--task <task>]`: disk guard, scrubbed environment, log, the buildsrc stale-package clean) with
//! `cargo build --release --offline --locked --example bench --message-format=json-render-diagnostics` (`--locked`: the build never
//! rewrites the main checkout's Cargo.lock), the same binary `cargo run --release --offline --example bench -- --runs` runs (the
//! documented command of the crash sim's docs/BENCH.md). Right after its build job, holding the build lock, the gate checks that the
//! package's build-source note still names that side's folder (buildsrc.rs: anyone who built the package from another folder since
//! would have changed it first) and copies the bench executable to `<scratch_dir>/perf-bins/<job id>/<side>/`. So two builds of
//! one crate from different commits in the shared build folder never hand each other a stale binary, and the A copy is safe from
//! B's build, which cleans A's artifacts. The copies are removed when the gate ends.
//!
//! THE RUNS. Holding the build lock from B's copy to the last run (so no compile starts between an A run and its B run, and both
//! see the same machine), after the job's own disk and pause guard, the gate runs `<copy> --runs <runs_per_case>` alternating A B A B
//! ... for `perf_runs` rounds, each in its own checkout folder (the bench reads files relative to it, as under `cargo run`), with
//! the scrubbed environment, the job's disk poll and redaction (`job::run_child`), all output in one log
//! `<scratch_dir>/logs/perf/<time>-<project>-perf.log`. The bench prints one JSON line per case
//! (`{"name":"box","runs":10,"median_ms":1.9,...}`).
//!
//! THE VERDICT. For every case of the baseline: `main` and `entry` are the medians of each side's `perf_runs` medians; the case
//! FAILS when `entry` is above `main * (1 + threshold_pct / 100)`, with the case's own `threshold_pct` from the baseline file (no
//! global threshold: each case's noise differs). The reason lists every case: both medians, the ratio entry/main and the change in
//! %. A case the entry's bench prints but the baseline lacks is listed as not judged. The first line says whether the baseline is
//! LOADED (`loaded: true`: it was recorded while the machine was busy, so its thresholds are wide) or quiet.
//!
//! BUDGETS. A case of the baseline file may also have `budget_ms`, an absolute ceiling for the entry's median (the A/B rule only
//! sees a slowdown against main, so a creep across many small merges, each under its threshold, would never be caught). The shape,
//! the one place a budget is set (no `broadsword.toml` key):
//!
//! ```json
//! "cases": { "box": { "median_ms": 1.571, "threshold_pct": 50, "budget_ms": 2.36 } }
//! ```
//!
//! `budget_ms` must be a finite number above 0; 0, a negative, a string, null, NaN and infinity make the whole file BAD (fail
//! closed, like any other bad field). When a case has one, the entry FAILS if its median (the median of its `perf_runs` medians)
//! is above `budget_ms`, even when it is within its A/B threshold; the reason line says "over budget" with both numbers. A case
//! with no `budget_ms` is judged exactly as before. If MAIN is itself over the budget in the same run, the entry is not blamed for
//! it: the budget rule passes the case (the A/B rule still applies) and a loud `main over budget` line is printed and kept in the
//! report, so the orchestrator boards a fix. Budgets hold in quiet and loaded runs alike; the gate does not record the CPU load of
//! a run, so a budget reason says "load not recorded". Budgets only change the verdict of an entry that is benched: a docs-only
//! entry is skipped (NOT JUDGED) before any bench is built, so a budget never runs a bench for it.
//!
//! The settings are `perf_gate` (`"on"` or `"off"`) and `perf_runs` (how many A/B rounds).

use crate::buildsrc;
use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec, Outcome};
use crate::json::{self, J};
use crate::lock::Lock;
use crate::{pause, scrub, wtgit};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The build command of each side. Its `bench` example is the binary `cargo run --release --offline --example bench` runs.
pub const BUILD_COMMAND: &str =
    "cargo build --release --offline --locked --example bench --message-format=json-render-diagnostics";

/// The most timed runs per case a baseline may ask for (a typo such as 1000000 would hold the build lock for hours).
const MAX_RUNS_PER_CASE: u64 = 1000;

/// One case of a baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub name: String,
    pub median_ms: f64,
    pub threshold_pct: f64,
    /// Optional absolute ceiling for the entry's median, in ms (finite, above 0); see "BUDGETS" in the module doc.
    pub budget_ms: Option<f64>,
}

/// A project's perf baseline: what the gate compares against.
#[derive(Debug, Clone, PartialEq)]
pub struct Baseline {
    /// Recorded while the machine was busy (its thresholds are wide); printed as LOADED.
    pub loaded: bool,
    /// `--runs` of each bench run.
    pub runs_per_case: u64,
    pub cases: Vec<Case>,
}

/// What a project's baseline file says.
#[derive(Debug)]
pub enum Found {
    /// No file: the project has no baseline and is skipped.
    None,
    Some(Baseline),
    /// A file that exists but cannot be used: the gate FAILS (fail closed).
    Bad(String),
}

/// The outcome of the gate for one commit.
#[derive(Debug)]
pub enum Gate {
    /// The gate did not apply (`perf_gate` off, or no baseline): one line saying why. Not a verdict on the code.
    Skipped(String),
    /// Every case is within its threshold: the report lines.
    Pass(Vec<String>),
    /// A case is too slow, or the gate could not get trustworthy numbers from a baseline or bench that exists: the reason (one line)
    /// and the report lines.
    Fail(String, Vec<String>),
    /// The bench job never ran (refused, paused, disk): no verdict either way.
    NoVerdict(String),
}

/// `<state_dir>/perf/<project>-baseline.json`.
pub fn baseline_path(cfg: &Config, project: &str) -> PathBuf {
    cfg.state_dir
        .join("perf")
        .join(format!("{project}-baseline.json"))
}

/// A path as it is shown: relative to the root when it is inside it.
fn shown(cfg: &Config, p: &Path) -> String {
    p.strip_prefix(&cfg.root)
        .unwrap_or(p)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn num(v: Option<&J>, what: &str) -> Result<f64, String> {
    match v {
        Some(J::Num(t)) => {
            let x: f64 = t
                .parse()
                .map_err(|_| format!("{what} is not a number: {t}"))?;
            if x.is_finite() {
                Ok(x)
            } else {
                Err(format!("{what} is not finite: {t}"))
            }
        }
        Some(_) => Err(format!("{what} is not a number")),
        None => Err(format!("{what} is missing")),
    }
}

/// A case name goes into reasons and logs: letters, digits, `_` and `-` only, 1 to 64 characters.
fn valid_case_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 64
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse a baseline file's text. Every field the gate uses is required and checked: `loaded` (true or false), `runs_per_case` (a
/// whole number from 1 to MAX_RUNS_PER_CASE), `cases` (an object with at least one case, no name twice), and per case `median_ms`
/// (finite, above 0), `threshold_pct` (finite, 0 or more) and, only if present, `budget_ms` (finite, above 0). Other fields are ignored.
pub fn parse_baseline(text: &str) -> Result<Baseline, String> {
    let v = json::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("not valid JSON: {e}"))?;
    if !matches!(v, J::Obj(_)) {
        return Err("not a JSON object".into());
    }
    let loaded = match v.get("loaded") {
        Some(J::Bool(b)) => *b,
        Some(_) => return Err("`loaded` is not true or false".into()),
        None => return Err("`loaded` is missing".into()),
    };
    let rpc = num(v.get("runs_per_case"), "`runs_per_case`")?;
    if rpc.fract() != 0.0 || rpc < 1.0 || rpc > MAX_RUNS_PER_CASE as f64 {
        return Err(format!(
            "`runs_per_case` must be a whole number from 1 to {MAX_RUNS_PER_CASE}, not {rpc}"
        ));
    }
    let members = match v.get("cases") {
        Some(J::Obj(m)) => m,
        Some(_) => return Err("`cases` is not an object".into()),
        None => return Err("`cases` is missing".into()),
    };
    if members.is_empty() {
        return Err("`cases` is empty".into());
    }
    let mut cases: Vec<Case> = Vec::new();
    for (name, c) in members {
        if !valid_case_name(name) {
            return Err(format!(
                "case name {name:?} is not 1-64 letters, digits, `_` or `-`"
            ));
        }
        if cases.iter().any(|k| k.name == *name) {
            return Err(format!("case `{name}` appears twice"));
        }
        if !matches!(c, J::Obj(_)) {
            return Err(format!("case `{name}` is not an object"));
        }
        let median_ms = num(c.get("median_ms"), &format!("`cases.{name}.median_ms`"))?;
        if median_ms <= 0.0 {
            return Err(format!(
                "`cases.{name}.median_ms` must be above 0, not {median_ms}"
            ));
        }
        let threshold_pct = num(
            c.get("threshold_pct"),
            &format!("`cases.{name}.threshold_pct`"),
        )?;
        if threshold_pct < 0.0 {
            return Err(format!(
                "`cases.{name}.threshold_pct` must be 0 or more, not {threshold_pct}"
            ));
        }
        // optional, but when the key is there it must be a finite number above 0 (null, a string, 0, a negative are refused)
        let budget_ms = match c.get("budget_ms") {
            None => None,
            Some(_) => {
                let b = num(c.get("budget_ms"), &format!("`cases.{name}.budget_ms`"))?;
                if b <= 0.0 {
                    return Err(format!("`cases.{name}.budget_ms` must be above 0, not {b}"));
                }
                Some(b)
            }
        };
        cases.push(Case {
            name: name.clone(),
            median_ms,
            threshold_pct,
            budget_ms,
        });
    }
    Ok(Baseline {
        loaded,
        runs_per_case: rpc as u64,
        cases,
    })
}

/// Read a project's baseline file. Only a file that does not exist means "no baseline"; every other problem (a folder in its place,
/// no permission, not UTF-8, bad content) is `Bad`.
pub fn find_baseline(path: &Path) -> Found {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Found::None,
        Err(e) => return Found::Bad(format!("cannot look at it: {e}")),
        Ok(_) => {}
    }
    match std::fs::read_to_string(path) {
        Ok(t) => match parse_baseline(&t) {
            Ok(b) => Found::Some(b),
            Err(m) => Found::Bad(m),
        },
        Err(e) => Found::Bad(format!("cannot read it: {e}")),
    }
}

/// The case lines of one bench run's output: every line that starts with `{"name"`. Each must be a JSON object with a valid `name`
/// and a finite `median_ms` of 0 or more, and no case may appear twice. No case line at all is an error.
pub fn parse_bench(text: &str) -> Result<Vec<(String, f64)>, String> {
    let mut out: Vec<(String, f64)> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if !l.starts_with("{\"name\"") {
            continue;
        }
        let v = json::parse(l).map_err(|e| format!("a case line is not valid JSON ({e}): {l}"))?;
        let name = v
            .get("name")
            .and_then(|n| n.str())
            .ok_or_else(|| format!("a case line has no string `name`: {l}"))?;
        if !valid_case_name(name) {
            return Err(format!(
                "case name {name:?} is not 1-64 letters, digits, `_` or `-`"
            ));
        }
        let m = num(v.get("median_ms"), &format!("`median_ms` of case `{name}`"))?;
        if m < 0.0 {
            return Err(format!("`median_ms` of case `{name}` is negative: {m}"));
        }
        if out.iter().any(|(n, _)| n == name) {
            return Err(format!("case `{name}` was printed twice"));
        }
        out.push((name.to_string(), m));
    }
    if out.is_empty() {
        return Err("the bench printed no case line (`{\"name\":...,\"median_ms\":...}`)".into());
    }
    Ok(out)
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

/// What a budget reason says about the machine's CPU load: the gate does not measure it (the baseline file records the load of
/// its own recording only), so a budget failure on a loaded machine is told apart by the reader, not by the gate.
const LOAD_NOTE: &str = "load not recorded";

/// The header line of every report: which baseline, and whether it is LOADED.
pub fn header(cfg: &Config, project: &str, base: &Baseline) -> String {
    let path = shown(cfg, &baseline_path(cfg, project));
    if base.loaded {
        format!("perf gate {project}: baseline {path} is LOADED (recorded while the machine was busy, so its thresholds are wide); {} case(s), thresholds from the baseline", base.cases.len())
    } else {
        format!("perf gate {project}: baseline {path} is quiet (not loaded); {} case(s), thresholds from the baseline", base.cases.len())
    }
}

/// One bench run's case lines (`parse_bench`): (case, median_ms).
type Run = Vec<(String, f64)>;

/// Every run's value of case `name`, or the (1-based) runs that lack it.
fn case_values(runs: &[Run], name: &str) -> Result<Vec<f64>, Vec<usize>> {
    let mut got = Vec::new();
    let mut missing = Vec::new();
    for (i, r) in runs.iter().enumerate() {
        match r.iter().find(|(n, _)| n == name) {
            Some((_, m)) => got.push(*m),
            None => missing.push(i + 1),
        }
    }
    if missing.is_empty() && !got.is_empty() {
        Ok(got)
    } else {
        Err(missing)
    }
}

/// Judge the entry's runs `b` against main's runs `a` (each run's case lines, from `parse_bench`), case by case with the baseline's
/// thresholds. Returns (passed, one line per case). A baseline case missing from any run of either side fails, so does a case
/// whose main median is not above 0 (no ratio can be taken), and so does an empty side.
pub fn judge(
    base: &Baseline,
    a: &[Vec<(String, f64)>],
    b: &[Vec<(String, f64)>],
) -> (bool, Vec<String>) {
    let mut ok = !a.is_empty() && !b.is_empty();
    let mut lines = Vec::new();
    if !ok {
        lines.push(format!(
            "no bench run to judge (main {} run(s), entry {} run(s))",
            a.len(),
            b.len()
        ));
    }
    for c in &base.cases {
        let (va, vb) = (case_values(a, &c.name), case_values(b, &c.name));
        let (mut va, mut vb) = match (va, vb) {
            (Ok(x), Ok(y)) => (x, y),
            (ra, rb) => {
                ok = false;
                let side = |r: &Result<Vec<f64>, Vec<usize>>| match r {
                    Ok(_) => "printed".to_string(),
                    Err(m) => format!("MISSING in run(s) {m:?}"),
                };
                lines.push(format!(
                    "{}: main {}, entry {}: FAIL (the bench did not print this case)",
                    c.name,
                    side(&ra),
                    side(&rb)
                ));
                continue;
            }
        };
        let (ma, mb) = (median(&mut va), median(&mut vb));
        let limit = ma * (1.0 + c.threshold_pct / 100.0);
        // `!(mb <= limit)` is also true for NaN; a main median of 0 has no ratio
        let slow = !(ma > 0.0 && ma.is_finite() && limit.is_finite() && mb <= limit);
        if slow {
            ok = false;
        }
        let ratio = if ma > 0.0 { mb / ma } else { f64::INFINITY };
        let mut status = if ma <= 0.0 {
            "FAIL, main's median is 0 ms so no ratio can be judged".to_string()
        } else if slow {
            "FAIL, slower than main by more than allowed".to_string()
        } else {
            "ok".to_string()
        };
        // the absolute budget: only a case that has one is judged on it (every median here is finite: parse_bench refuses the rest)
        let mut main_over: Option<String> = None;
        if let Some(bg) = c.budget_ms {
            if ma > bg {
                // main is over its own budget: the entry is not blamed for that (the A/B rule above still applies)
                main_over = Some(format!(
                    "{}: main over budget: main {ma:.3} ms is above the budget {bg} ms; the entry is not blamed for it, board a fix",
                    c.name
                ));
                status.push_str(&format!(" (budget {bg} ms: main over budget, not judged)"));
            } else if mb > bg {
                ok = false;
                let why = format!(
                    "over budget: entry {mb:.3} ms is above the budget {bg} ms (main {ma:.3} ms; {LOAD_NOTE})"
                );
                status = if status == "ok" {
                    format!("FAIL, {why}")
                } else {
                    format!("{status}; {why}")
                };
            } else {
                status.push_str(&format!(" (budget {bg} ms: entry under budget)"));
            }
        }
        lines.push(format!(
            "{}: main {ma:.3} ms, entry {mb:.3} ms, ratio {ratio:.3} ({:+.1}%, threshold {}%): {status}",
            c.name,
            (ratio - 1.0) * 100.0,
            c.threshold_pct,
        ));
        if let Some(l) = main_over {
            lines.push(l);
        }
    }
    for r in b.iter().take(1) {
        for (n, m) in r {
            if !base.cases.iter().any(|c| c.name == *n) {
                lines.push(format!(
                    "{n}: entry {m:.3} ms, no baseline case: not judged"
                ));
            }
        }
    }
    (ok, lines)
}

/// Can a change of this repo-relative path (`/`-separated, as git prints it) change what the bench measures? Everything is
/// benched except documentation: a path under `docs/` or ending in `.md`, unless it is under `src/`, `examples/` or `benches/`
/// (the folders the build reads). An allow-list of what cannot matter (see "NOT JUDGED" in the module doc). Compared without case,
/// as Windows resolves folder names.
pub fn is_benched_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    if ["src/", "examples/", "benches/"]
        .iter()
        .any(|d| p.starts_with(d))
    {
        return true;
    }
    !(p.starts_with("docs/") || p.ends_with(".md"))
}

/// `git diff --name-only --no-renames -z <from> <to>` in `dir` (two commits, so no working-tree file and no filter is involved;
/// `--no-ext-diff` so no configured diff program runs): every path that differs, a rename as both of its paths.
fn changed_paths(dir: &Path, from: &str, to: &str) -> Result<Vec<String>, String> {
    let args = [
        "diff",
        "--name-only",
        "--no-renames",
        "--no-ext-diff",
        "-z",
        from,
        to,
    ];
    let o = scrub::command_in(dir, "git")?
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("cannot start git: {e}"))?;
    if !o.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    // a path that is not UTF-8 is kept lossily: it cannot end in `.md` by accident, so it stays benched
    Ok(o.stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// The one `bench` example executable a `--message-format=json*` build printed (`compiler-artifact`, target kind `example`, name
/// `bench`), with the Cargo.toml of its package. None, or two different ones, is an error.
pub fn bench_artifact(log: &str) -> Result<(PathBuf, PathBuf), String> {
    let mut found: Vec<(PathBuf, PathBuf)> = Vec::new();
    for l in log
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("{\"reason\":\"compiler-artifact\""))
    {
        let j =
            json::parse(l).map_err(|e| format!("cannot read cargo's JSON output ({e}): {l}"))?;
        let Some(t) = j.get("target") else { continue };
        let example = matches!(t.get("kind"), Some(J::Arr(k)) if k.iter().any(|k| k.str() == Some("example")));
        if !example || t.get("name").and_then(J::str) != Some("bench") {
            continue;
        }
        let (Some(exe), Some(manifest)) = (
            j.get("executable").and_then(J::str),
            j.get("manifest_path").and_then(J::str),
        ) else {
            continue;
        };
        let art = (PathBuf::from(manifest), PathBuf::from(exe));
        if !found.contains(&art) {
            found.push(art);
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err(
            "the build printed no `bench` example executable (is there an examples/bench.rs?)"
                .into(),
        ),
        n => Err(format!(
            "the build printed {n} different `bench` example executables"
        )),
    }
}

/// The copied bench executables of one gate, in `<scratch_dir>/perf-bins/<id>`; removed when the gate ends.
struct Copies(PathBuf);

impl Drop for Copies {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One side of the A/B: main (the project's main checkout at the trunk tip) or the entry (the task's worktree).
struct Side {
    /// `main` or `entry`, as the report names it.
    label: &'static str,
    /// The checkout the bench is built and run in.
    dir: PathBuf,
    /// `--task` of its jobs: None for the main checkout.
    task: Option<String>,
    /// The commit `dir` must have checked out for the whole gate.
    commit: String,
}

/// Why a step of the gate stopped it.
enum Stop {
    Fail(String),
    NoVerdict(String),
}

impl Side {
    /// The commit check: the checkout still holds `commit`, and (main only, where the queue merges) nothing uncommitted.
    fn still_there(&self) -> Result<(), Stop> {
        let head = wtgit::head_commit(&self.dir).map_err(Stop::NoVerdict)?;
        if head != self.commit {
            return Err(Stop::NoVerdict(format!(
                "the {} checkout {} moved from {} to {} during the gate",
                self.label,
                self.dir.display(),
                short(&self.commit),
                short(&head)
            )));
        }
        match wtgit::status_porcelain_plain(&self.dir) {
            Ok(s) if s.is_empty() => Ok(()),
            Ok(s) => Err(Stop::NoVerdict(format!("the {} checkout {} has uncommitted or untracked changes ({}), so its bench would not be of commit {} alone", self.label, self.dir.display(), s.lines().take(5).collect::<Vec<_>>().join(", "), short(&self.commit)))),
            Err(m) => Err(Stop::NoVerdict(m)),
        }
    }

    /// Build this side's bench with a `run` job, then (holding the build lock, returned) check the build folder still holds THIS
    /// folder's build and copy the executable to `<copies>/<label>/`. Returns (job id, the copy, the lock).
    fn build(
        &self,
        cfg: &Config,
        project: &str,
        copies: &Path,
    ) -> Result<(String, PathBuf, Lock), Stop> {
        self.still_there()?;
        println!(
            "perf gate {project}: building the bench of {} ({}): {BUILD_COMMAND}",
            self.label,
            short(&self.commit)
        );
        let mut info = JobInfo::default();
        let code = job::run_job_info(
            cfg,
            JobSpec {
                kind: "run".into(),
                name: project.to_string(),
                command: Some(BUILD_COMMAND.to_string()),
                first_step: 0,
                resumed_from: None,
                resume_count: 0,
                task: self.task.clone(),
            },
            &mut info,
        );
        let id = info.id.clone().unwrap_or_else(|| "none".into());
        if info.summary.is_empty() {
            return Err(Stop::NoVerdict(format!("the bench build of {} could not run (exit {code}, job {id}; the reason is printed above, e.g. Broadsword paused, the disk guard or a busy build folder)", self.label)));
        }
        if code != 0 {
            return Err(Stop::Fail(format!("the bench could not be built on {} ({}): job {id} exited {code} ({}); a bench that cannot be built fails the gate", self.label, short(&self.commit), info.summary.join("; "))));
        }
        let text = match info.log.as_deref().map(std::fs::read_to_string) {
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                return Err(Stop::NoVerdict(format!(
                    "the log of bench build job {id} could not be read: {e}"
                )))
            }
            None => return Err(Stop::NoVerdict(format!("bench build job {id} has no log"))),
        };
        let (manifest, exe) = bench_artifact(&text).map_err(|m| {
            Stop::Fail(format!(
                "the bench build of {} (job {id}) cannot be used: {m}",
                self.label
            ))
        })?;
        let same = |a: &Path, b: &Path| matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y);
        if !manifest.parent().is_some_and(|p| same(p, &self.dir)) {
            return Err(Stop::Fail(format!("the `bench` example built for {} (job {id}) belongs to {}, not to the package at the root of {}", self.label, manifest.display(), self.dir.display())));
        }
        let package = std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|t| buildsrc::package_name_in(&t));
        let Some(package) = package else {
            return Err(Stop::Fail(format!(
                "cannot read the package name in {}",
                manifest.display()
            )));
        };
        let lock = buildsrc::lock(cfg).map_err(|(_, m)| Stop::NoVerdict(m))?;
        // under the lock: whoever builds this package from another folder rewrites its note FIRST (buildsrc.rs), so a note that
        // still names this folder means the executable in the build folder is this folder's
        let source = buildsrc::rel_source(cfg, &self.dir).map_err(Stop::NoVerdict)?;
        let want = format!("{}{source}", buildsrc::RUN_NOTE_PREFIX);
        let note = buildsrc::read_note(cfg, &package);
        if note.as_deref() != Some(want.as_str()) {
            return Err(Stop::NoVerdict(format!("the shared build folder changed hands before the bench of {} could be copied out (package `{package}` is noted as {:?}, not {want:?}); run the entry again", self.label, note.unwrap_or_default())));
        }
        self.still_there()?;
        let dest_dir = copies.join(self.label);
        let dest = dest_dir.join(exe.file_name().unwrap_or_default());
        std::fs::create_dir_all(&dest_dir)
            .and_then(|_| std::fs::copy(&exe, &dest))
            .map_err(|e| {
                Stop::NoVerdict(format!(
                    "cannot copy the bench {} to {}: {e}",
                    exe.display(),
                    dest.display()
                ))
            })?;
        Ok((id, dest, lock))
    }
}

/// Run one bench copy in its checkout folder: the case lines it printed.
fn bench_once(
    cfg: &Config,
    side: &Side,
    exe: &Path,
    runs_per_case: u64,
    log: &Arc<Mutex<std::fs::File>>,
    what: &str,
) -> Result<Vec<(String, f64)>, Stop> {
    if let Ok(mut f) = log.lock() {
        let _ = std::io::Write::write_all(
            &mut *f,
            format!(
                "=== {what}: {} at {} in {} ===\n",
                side.label,
                short(&side.commit),
                side.dir.display()
            )
            .as_bytes(),
        );
    }
    let mut cmd = scrub::command(cfg, exe);
    cmd.arg("--runs")
        .arg(runs_per_case.to_string())
        .current_dir(&side.dir);
    let mut free = job::hooked_free_source(cfg);
    let secrets = Arc::new(scrub::secrets(cfg));
    let ran = job::run_child(
        cfg,
        cmd,
        log,
        &mut *free,
        Duration::from_secs(cfg.job_disk_poll_seconds),
        &mut |_| {},
        &secrets,
    )
    .map_err(Stop::NoVerdict)?;
    match ran.outcome {
        Outcome::DiskDropped(f) => Err(Stop::NoVerdict(format!(
            "{what} ({}) was stopped: free space fell to {} (below the floor)",
            side.label,
            pause::gb(f)
        ))),
        Outcome::Exited(Some(0)) => parse_bench(&ran.stdout).map_err(|m| {
            Stop::Fail(format!(
                "the output of {what} ({}) cannot be used: {m}",
                side.label
            ))
        }),
        Outcome::Exited(c) => Err(Stop::Fail(format!(
            "the bench failed: {what} ({}) exited {}; a bench that cannot run fails the gate",
            side.label,
            c.map(|c| c.to_string())
                .unwrap_or_else(|| "without a code".into())
        ))),
    }
}

/// THE GATE, the one place every queue path calls (the single-entry check step, a train's check of its tip, every bisect check and
/// a culprit's own check): judge `project` at the commit its task's worktree holds now against the project's trunk tip. `what`
/// names the check in the output.
pub fn run_gate(cfg: &Config, project: &str, task: &str, what: &str) -> Gate {
    if !cfg.perf_gate {
        return Gate::Skipped(format!(
            "perf gate {project}: off (perf_gate = \"off\"); no bench ran"
        ));
    }
    let path = baseline_path(cfg, project);
    let base = match find_baseline(&path) {
        Found::None => return Gate::Skipped(format!("perf gate {project}: skipped: the project has no baseline ({} does not exist); no bench ran", shown(cfg, &path))),
        Found::Bad(m) => {
            let reason = format!("the perf baseline {} exists but cannot be used ({m}); the gate fails closed. Fix or remove the baseline file", shown(cfg, &path));
            return Gate::Fail(reason.clone(), vec![format!("perf gate {project}: FAILED: {reason}")]);
        }
        Found::Some(b) => b,
    };
    let no_verdict = |m: String| Gate::NoVerdict(m);
    // the two sides
    let main_dir = match job::job_folder(cfg, project, None) {
        Ok(d) => d,
        Err((_, m)) => return no_verdict(m),
    };
    let wt = match job::job_folder(cfg, project, Some(task)) {
        Ok(d) => d,
        Err((_, m)) => return no_verdict(m),
    };
    let (trunk, tip) = match wtgit::trunk_commit(&main_dir) {
        Ok(t) => t,
        Err(m) => return no_verdict(m),
    };
    let entry = match wtgit::head_commit(&wt) {
        Ok(h) => h,
        Err(m) => return no_verdict(m),
    };
    // nothing benched changed: not judged, nothing built
    let changed = match changed_paths(&wt, &tip, &entry) {
        Ok(c) => c,
        Err(m) => {
            return no_verdict(format!(
                "cannot list what the entry changes against main: {m}"
            ))
        }
    };
    if !changed.iter().any(|p| is_benched_path(p)) {
        let list = if changed.is_empty() {
            "no file differs".to_string()
        } else {
            format!(
                "only documentation differs: {}",
                changed
                    .iter()
                    .take(10)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        return Gate::Skipped(format!("perf gate {project}: not judged: no benched source changed against main {trunk} ({}; {list}); no bench was built or ran", short(&tip)));
    }
    let a = Side {
        label: "main",
        dir: main_dir,
        task: None,
        commit: tip.clone(),
    };
    let b = Side {
        label: "entry",
        dir: wt,
        task: Some(task.to_string()),
        commit: entry.clone(),
    };
    let head = header(cfg, project, &base);
    println!("{head}");
    let ab = format!("perf gate {project}: A/B ({what}): main {trunk} at {} (the project's main checkout) against the entry at {} (task {task}'s worktree); {} round(s) of main then entry, under one hold of the build lock", short(&tip), short(&entry), cfg.perf_runs);
    println!("{ab}");
    let mut lines = vec![head, ab];
    let fail = |reason: String, mut lines: Vec<String>| {
        lines.push(format!("FAILED: {reason}"));
        println!("perf gate {project}: FAILED: {reason}");
        Gate::Fail(reason, lines)
    };
    let stopped = |s: Stop, lines: Vec<String>| match s {
        Stop::Fail(r) => fail(r, lines),
        Stop::NoVerdict(m) => Gate::NoVerdict(m),
    };
    // build A, copy it out (its lock is dropped before B's build job needs the lock); build B, copy it out and KEEP the lock
    let copies = Copies(cfg.scratch_dir.join("perf-bins").join(format!(
        "{project}-{}-{}",
        job::file_stamp(now_secs()),
        std::process::id()
    )));
    let mut built: Vec<(String, PathBuf)> = Vec::new();
    let mut hold: Option<Lock> = None;
    for side in [&a, &b] {
        drop(hold.take());
        match side.build(cfg, project, &copies.0) {
            Ok((id, exe, lock)) => {
                built.push((id, exe));
                hold = Some(lock);
            }
            Err(s) => return stopped(s, lines),
        }
    }
    // the runs, under the lock taken for B's copy
    let mut free = job::hooked_free_source(cfg);
    if let Err(why) = job::guard_decision(
        pause::is_paused(&cfg.root),
        free(),
        cfg.reserve_bytes(),
        cfg.floor_bytes(),
    ) {
        return no_verdict(format!("the bench runs were refused: {why}"));
    }
    if let Err(s) = a.still_there().and_then(|_| b.still_there()) {
        return stopped(s, lines);
    }
    let log_dir = cfg.scratch_dir.join("logs").join("perf");
    let (log_path, log) = match std::fs::create_dir_all(&log_dir).and_then(|_| {
        job::create_log(
            &log_dir,
            &format!("{}-{project}-perf", job::file_stamp(now_secs())),
        )
    }) {
        Ok((p, f)) => (p, Arc::new(Mutex::new(f))),
        Err(e) => {
            return no_verdict(format!(
                "cannot create the bench log in {}: {e}",
                log_dir.display()
            ))
        }
    };
    let (mut ra, mut rb): (Vec<Run>, Vec<Run>) = (Vec::new(), Vec::new());
    for i in 1..=cfg.perf_runs {
        for (side, (_, exe), out) in [(&a, &built[0], &mut ra), (&b, &built[1], &mut rb)] {
            let run = format!("bench round {i} of {}", cfg.perf_runs);
            println!(
                "perf gate {project}: {run}: {} ({}): {} --runs {}",
                side.label,
                short(&side.commit),
                exe.display(),
                base.runs_per_case
            );
            match bench_once(cfg, side, exe, base.runs_per_case, &log, &run) {
                Ok(r) => out.push(r),
                Err(s) => {
                    lines.push(format!("bench log: {}", shown(cfg, &log_path)));
                    return stopped(s, lines);
                }
            }
        }
    }
    drop(hold);
    drop(copies);
    let (ok, case_lines) = judge(&base, &ra, &rb);
    for l in &case_lines {
        println!("perf gate {project}: {l}");
    }
    lines.extend(case_lines.iter().cloned());
    lines.push(format!(
        "bench build jobs: main {}, entry {}; bench log: {}",
        built[0].0,
        built[1].0,
        shown(cfg, &log_path)
    ));
    if ok {
        lines.push(format!(
            "perf gate {project}: PASS ({} A/B round(s))",
            cfg.perf_runs
        ));
        println!("perf gate {project}: PASS");
        Gate::Pass(lines)
    } else {
        let reason = format!(
            "slower than main {trunk} ({}) by more than the {} baseline's thresholds allow ({}): {}",
            short(&tip),
            if base.loaded { "LOADED" } else { "quiet" },
            shown(cfg, &baseline_path(cfg, project)),
            case_lines.join("; ")
        );
        lines.push(format!("perf gate {project}: FAIL"));
        println!("perf gate {project}: FAIL");
        Gate::Fail(reason, lines)
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REAL: &str = r#"{"crash_sim_sha":"57f2","runs":3,"runs_per_case":10,"loaded":true,
      "cases":{"box":{"median_ms":1.888,"threshold_pct":10},"cube":{"median_ms":1.173,"threshold_pct":18.93},"settle":{"median_ms":5.046,"threshold_pct":14.98}}}"#;

    #[test]
    fn the_real_baseline_shape_parses_with_per_case_thresholds() {
        let b = parse_baseline(REAL).unwrap();
        assert!(b.loaded);
        assert_eq!(b.runs_per_case, 10);
        assert_eq!(
            b.cases
                .iter()
                .map(|c| (c.name.as_str(), c.threshold_pct))
                .collect::<Vec<_>>(),
            vec![("box", 10.0), ("cube", 18.93), ("settle", 14.98)]
        );
    }

    #[test]
    fn a_baseline_missing_or_bad_in_any_needed_field_is_refused() {
        for (bad, why) in [
            ("", "empty"),
            ("[]", "array"),
            (
                r#"{"runs_per_case":10,"cases":{"box":{"median_ms":1,"threshold_pct":1}}}"#,
                "no loaded",
            ),
            (
                r#"{"loaded":"yes","runs_per_case":10,"cases":{"box":{"median_ms":1,"threshold_pct":1}}}"#,
                "loaded not bool",
            ),
            (
                r#"{"loaded":true,"cases":{"box":{"median_ms":1,"threshold_pct":1}}}"#,
                "no runs_per_case",
            ),
            (
                r#"{"loaded":true,"runs_per_case":0,"cases":{"box":{"median_ms":1,"threshold_pct":1}}}"#,
                "runs 0",
            ),
            (
                r#"{"loaded":true,"runs_per_case":2.5,"cases":{"box":{"median_ms":1,"threshold_pct":1}}}"#,
                "runs fraction",
            ),
            (
                r#"{"loaded":true,"runs_per_case":100000,"cases":{"box":{"median_ms":1,"threshold_pct":1}}}"#,
                "runs huge",
            ),
            (r#"{"loaded":true,"runs_per_case":10}"#, "no cases"),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{}}"#,
                "empty cases",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"threshold_pct":1}}}"#,
                "no median",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"median_ms":0,"threshold_pct":1}}}"#,
                "median 0",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"median_ms":-1,"threshold_pct":1}}}"#,
                "median negative",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"median_ms":1}}}"#,
                "no threshold",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"median_ms":1,"threshold_pct":-1}}}"#,
                "threshold negative",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"median_ms":1,"threshold_pct":"10"}}}"#,
                "threshold string",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"median_ms":1e999,"threshold_pct":1}}}"#,
                "median infinite",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"box":{"median_ms":1,"threshold_pct":1},"box":{"median_ms":1,"threshold_pct":1}}}"#,
                "duplicate",
            ),
            (
                r#"{"loaded":true,"runs_per_case":10,"cases":{"a b":{"median_ms":1,"threshold_pct":1}}}"#,
                "bad name",
            ),
        ] {
            assert!(parse_baseline(bad).is_err(), "{why} must be refused");
        }
    }

    #[test]
    fn bench_output_is_read_from_its_case_lines_and_bad_lines_fail() {
        let log = "job: run p\n   Compiling p\n{\"name\":\"box\",\"runs\":3,\"median_ms\":2.000,\"min_ms\":1}\nnoise\n{\"name\":\"cube\",\"runs\":3,\"median_ms\":1.5}\n";
        assert_eq!(
            parse_bench(log).unwrap(),
            vec![("box".to_string(), 2.0), ("cube".to_string(), 1.5)]
        );
        assert!(parse_bench("nothing here\n").is_err(), "no case line");
        assert!(
            parse_bench("{\"name\":\"box\",\"median_ms\":oops}\n").is_err(),
            "not JSON"
        );
        assert!(parse_bench("{\"name\":\"box\"}\n").is_err(), "no median");
        assert!(
            parse_bench("{\"name\":\"box\",\"median_ms\":-1}\n").is_err(),
            "negative"
        );
        assert!(
            parse_bench("{\"name\":\"box\",\"median_ms\":1}\n{\"name\":\"box\",\"median_ms\":1}\n")
                .is_err(),
            "twice"
        );
    }

    fn base(cases: &[(&str, f64, f64)]) -> Baseline {
        Baseline {
            loaded: true,
            runs_per_case: 3,
            cases: cases
                .iter()
                .map(|(n, m, t)| Case {
                    name: n.to_string(),
                    median_ms: *m,
                    threshold_pct: *t,
                    budget_ms: None,
                })
                .collect(),
        }
    }

    fn run(cases: &[(&str, f64)]) -> Vec<(String, f64)> {
        cases.iter().map(|(n, m)| (n.to_string(), *m)).collect()
    }

    #[test]
    fn each_case_is_judged_on_the_ratio_of_the_two_sides_medians_against_its_own_threshold() {
        // the stored medians (9.0) are a record only: the verdict is entry against main
        let b = base(&[("box", 9.0, 10.0), ("cube", 9.0, 20.0)]);
        let main = [
            run(&[("box", 2.0), ("cube", 1.0)]),
            run(&[("box", 2.2), ("cube", 1.0)]),
            run(&[("box", 1.8), ("cube", 1.0)]),
        ];
        // box: entry median of 2.1, 2.6, 2.16 = 2.16 against main 2.0: ratio 1.08 (+8%, under 10%); cube +15%, under 20%
        let (ok, lines) = judge(
            &b,
            &main,
            &[
                run(&[("box", 2.1), ("cube", 1.15)]),
                run(&[("box", 2.6), ("cube", 1.15)]),
                run(&[("box", 2.16), ("cube", 1.15)]),
            ],
        );
        assert!(ok, "{lines:?}");
        assert_eq!(
            lines[0], "box: main 2.000 ms, entry 2.160 ms, ratio 1.080 (+8.0%, threshold 10%): ok",
            "{lines:?}"
        );
        // cube +15% is over box's 10% but under its own 20%; box +12% is over its 10%
        let (ok, lines) = judge(&b, &main, &[run(&[("box", 2.24), ("cube", 1.15)])]);
        assert!(!ok);
        assert!(
            lines[0].contains("ratio 1.120 (+12.0%") && lines[0].contains("FAIL"),
            "{lines:?}"
        );
        assert!(lines[1].ends_with(": ok"), "{lines:?}");
        // exactly at the threshold passes; faster passes
        assert!(
            judge(
                &b,
                &[run(&[("box", 1.0), ("cube", 1.0)])],
                &[run(&[("box", 1.1), ("cube", 0.5)])]
            )
            .0
        );
    }

    #[test]
    fn a_uniform_load_on_both_sides_does_not_fail_and_a_main_median_of_zero_fails_closed() {
        let b = base(&[("box", 1.0, 10.0)]);
        // the whole machine 3x slower: both sides alike, ratio 1
        assert!(judge(&b, &[run(&[("box", 3.0)])], &[run(&[("box", 3.0)])]).0);
        let (ok, lines) = judge(&b, &[run(&[("box", 0.0)])], &[run(&[("box", 0.0)])]);
        assert!(
            !ok && lines[0].contains("main's median is 0 ms"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_case_missing_from_a_run_of_either_side_fails_and_an_extra_case_is_not_judged() {
        let b = base(&[("box", 1.0, 10.0)]);
        let one = [run(&[("box", 1.0)])];
        let (ok, lines) = judge(&b, &one, &[run(&[("box", 1.0)]), run(&[("other", 1.0)])]);
        assert!(
            !ok && lines[0].contains("entry MISSING in run(s) [2]"),
            "{lines:?}"
        );
        let (ok, lines) = judge(&b, &[run(&[("other", 1.0)])], &one);
        assert!(
            !ok && lines[0].contains("main MISSING in run(s) [1]"),
            "{lines:?}"
        );
        let (ok, lines) = judge(&b, &one, &[run(&[("box", 1.0), ("new", 9.0)])]);
        assert!(ok && lines[1].contains("not judged"), "{lines:?}");
        assert!(!judge(&b, &[], &one).0, "no main run is no pass");
        assert!(!judge(&b, &one, &[]).0, "no entry run is no pass");
    }

    fn budgeted(budget: Option<f64>) -> Baseline {
        let mut b = base(&[("box", 1.0, 50.0)]);
        b.cases[0].budget_ms = budget;
        b
    }

    #[test]
    fn a_budget_fails_an_entry_within_its_threshold_and_a_case_without_one_is_judged_as_before() {
        let main = [run(&[("box", 1.0)])];
        let entry = [run(&[("box", 1.3)])];
        // +30% is within 50%: no budget passes, exactly as before
        let (ok, plain) = judge(&budgeted(None), &main, &entry);
        assert!(ok);
        assert_eq!(
            plain[0],
            "box: main 1.000 ms, entry 1.300 ms, ratio 1.300 (+30.0%, threshold 50%): ok"
        );
        // budget 1.2: the same numbers fail, and the line names the budget, both numbers and the load note
        let (ok, lines) = judge(&budgeted(Some(1.2)), &main, &entry);
        assert!(!ok, "{lines:?}");
        assert!(
            lines[0].contains("over budget")
                && lines[0].contains("entry 1.300 ms is above the budget 1.2 ms")
                && lines[0].contains("load not recorded"),
            "{lines:?}"
        );
        // budget 1.4: under it passes
        let (ok, lines) = judge(&budgeted(Some(1.4)), &main, &entry);
        assert!(ok && lines[0].contains("entry under budget"), "{lines:?}");
        // exactly at the budget passes
        assert!(judge(&budgeted(Some(1.3)), &main, &entry).0);
    }

    #[test]
    fn main_over_budget_is_not_blamed_on_the_entry_but_the_ab_rule_still_applies() {
        let b = budgeted(Some(1.5));
        let (ok, lines) = judge(&b, &[run(&[("box", 2.0)])], &[run(&[("box", 2.1)])]);
        assert!(ok, "{lines:?}");
        assert!(
            lines
                .iter()
                .any(|l| l.contains("box: main over budget: main 2.000 ms")),
            "{lines:?}"
        );
        let (ok, _) = judge(&b, &[run(&[("box", 2.0)])], &[run(&[("box", 4.0)])]);
        assert!(!ok, "+100% is over the 50% threshold");
    }

    #[test]
    fn a_bad_budget_is_refused_and_a_good_one_is_read() {
        let with = |v: &str| {
            format!(
                r#"{{"loaded":true,"runs_per_case":10,"cases":{{"box":{{"median_ms":1,"threshold_pct":1,"budget_ms":{v}}}}}}}"#
            )
        };
        for v in ["0", "-1", "-0.0", "null", "\"2\"", "true", "1e999", "[]"] {
            let e = parse_baseline(&with(v)).unwrap_err();
            assert!(e.contains("budget_ms"), "{v}: {e}");
        }
        assert_eq!(
            parse_baseline(&with("2.5")).unwrap().cases[0].budget_ms,
            Some(2.5)
        );
        assert_eq!(parse_baseline(REAL).unwrap().cases[0].budget_ms, None);
    }

    #[test]
    fn only_documentation_is_not_benched() {
        for p in [
            "docs/STAGE6.md",
            "README.md",
            "notes/x.MD",
            "docs/img/a.png",
            "DOCS/x.txt",
        ] {
            assert!(!is_benched_path(p), "{p}");
        }
        for p in [
            "src/lib.rs",
            "src/README.md",
            "examples/bench.rs",
            "examples/notes.md",
            "benches/a.md",
            "SRC/x.md",
            "Cargo.toml",
            "Cargo.lock",
            "scenarios/box.bsn",
            "build.rs",
            ".cargo/config.toml",
            "scripts/x.py",
            ".gitattributes",
            "docsx/a.rs",
        ] {
            assert!(is_benched_path(p), "{p}");
        }
    }

    #[test]
    fn the_bench_executable_is_read_from_cargos_json_and_must_be_exactly_one() {
        let art = |kind: &str, name: &str, exe: &str| {
            format!("{{\"reason\":\"compiler-artifact\",\"manifest_path\":\"C:\\\\p\\\\Cargo.toml\",\"target\":{{\"kind\":[\"{kind}\"],\"name\":\"{name}\"}},\"executable\":{exe}}}")
        };
        let log = format!(
            "job: run p\n{}\n{}\n{}\n{{\"reason\":\"build-finished\",\"success\":true}}\n",
            art("lib", "p", "null"),
            art(
                "example",
                "bench",
                r#""C:\\b\\release\\examples\\bench.exe""#
            ),
            art("example", "energy", r#""C:\\b\\e.exe""#)
        );
        assert_eq!(
            bench_artifact(&log).unwrap(),
            (
                PathBuf::from(r"C:\p\Cargo.toml"),
                PathBuf::from(r"C:\b\release\examples\bench.exe")
            )
        );
        assert!(bench_artifact("job: run p\n").is_err(), "none");
        let two = format!(
            "{}\n{}\n",
            art("example", "bench", "\"a.exe\""),
            art("example", "bench", "\"b.exe\"")
        );
        assert!(bench_artifact(&two).is_err(), "two");
        assert!(
            bench_artifact("{\"reason\":\"compiler-artifact\",oops}\n").is_err(),
            "not JSON"
        );
    }

    #[test]
    fn a_missing_file_is_no_baseline_but_a_folder_or_bad_file_is_bad() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!("bs-perf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("dir-baseline.json")).unwrap();
        assert!(matches!(
            find_baseline(&d.join("none-baseline.json")),
            Found::None
        ));
        assert!(
            matches!(find_baseline(&d.join("dir-baseline.json")), Found::Bad(_)),
            "a folder in its place is not 'no baseline'"
        );
        std::fs::write(d.join("bad-baseline.json"), "{not json").unwrap();
        assert!(matches!(
            find_baseline(&d.join("bad-baseline.json")),
            Found::Bad(_)
        ));
        std::fs::write(d.join("ok-baseline.json"), REAL).unwrap();
        assert!(matches!(
            find_baseline(&d.join("ok-baseline.json")),
            Found::Some(_)
        ));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
