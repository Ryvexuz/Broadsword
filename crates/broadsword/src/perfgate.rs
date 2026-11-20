//! The merge queue's PERF GATE: after the check of a commit passes, the project's timing bench runs on that same commit and a case
//! whose median got slower than its baseline allows fails the entry (`perf_failed`).
//!
//! WHICH PROJECTS. A queue entry touches exactly one project: the one it was queued for (its branch lives in that project's repo).
//! The gate looks only at that project's baseline, `<state_dir>/perf/<project>-baseline.json`. A project with no such file is
//! SKIPPED, and the skip is printed and logged in the entry; no bench runs for it, and no other project's bench ever runs.
//!
//! FAIL CLOSED. A baseline file that exists but cannot be read, is not valid JSON, or lacks a field the gate needs is a FAILURE
//! (`perf_failed`), never a pass and never a skip. So is a bench that exits non-zero, prints no case line, prints a line that is not
//! a valid case, or leaves out a case the baseline has. A bench job that never ran (refused by the disk guard, the build lock or a
//! pause) gives NO verdict: the entry is `blocked` (or, in a train, every entry stays `approved`), so it is never merged either.
//!
//! THE BENCH. `cargo run --release --offline --example bench -- --runs <runs_per_case>` (the documented command of the crash sim's
//! docs/BENCH.md), run `perf_runs` times as `broadsword run <project> --task <task>`: in the task's worktree, on the commit the check
//! just passed, holding the build lock for the whole run, with the job's disk guard, scrubbed environment and log. `runs_per_case`
//! comes from the baseline, so the timings are taken the way the baseline's were. The bench prints one JSON line per case
//! (`{"name":"box","runs":10,"median_ms":1.9,...}`); the gate reads those lines from the job's log.
//!
//! THE VERDICT. For every case of the baseline: `now` is the median of the `perf_runs` medians; the change is
//! `(now - baseline) / baseline * 100`; the case FAILS when the change is above the case's own `threshold_pct` from the baseline file
//! (no global threshold: each case's noise differs). The reason lists every case: baseline, now and the change in %. A case the bench
//! prints but the baseline lacks is listed as not judged. The first line says whether the baseline is LOADED (`loaded: true`: it was
//! recorded while the machine was busy, so its thresholds are wide) or quiet.
//!
//! The settings are `perf_gate` (`"on"` or `"off"`) and `perf_runs` (how many times the bench runs).

use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec};
use crate::json::{self, J};
use std::path::{Path, PathBuf};

/// The bench command, without its `--runs` value (appended from the baseline's `runs_per_case`).
pub const BENCH_COMMAND: &str = "cargo run --release --offline --example bench -- --runs";

/// The most timed runs per case a baseline may ask for (a typo such as 1000000 would hold the build lock for hours).
const MAX_RUNS_PER_CASE: u64 = 1000;

/// One case of a baseline.
#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub name: String,
    pub median_ms: f64,
    pub threshold_pct: f64,
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
/// (finite, above 0) and `threshold_pct` (finite, 0 or more). Other fields are ignored.
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
        cases.push(Case {
            name: name.clone(),
            median_ms,
            threshold_pct,
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

/// The header line of every report: which baseline, and whether it is LOADED.
pub fn header(cfg: &Config, project: &str, base: &Baseline) -> String {
    let path = shown(cfg, &baseline_path(cfg, project));
    if base.loaded {
        format!("perf gate {project}: baseline {path} is LOADED (recorded while the machine was busy, so its thresholds are wide); {} case(s), thresholds from the baseline", base.cases.len())
    } else {
        format!("perf gate {project}: baseline {path} is quiet (not loaded); {} case(s), thresholds from the baseline", base.cases.len())
    }
}

/// Judge `runs` (each run's case lines, from `parse_bench`) against `base`. Returns (passed, one line per case). A baseline case
/// missing from any run fails.
pub fn judge(base: &Baseline, runs: &[Vec<(String, f64)>]) -> (bool, Vec<String>) {
    let mut ok = !runs.is_empty();
    let mut lines = Vec::new();
    if runs.is_empty() {
        lines.push("no bench run to judge".to_string());
    }
    for c in &base.cases {
        let mut got: Vec<f64> = Vec::new();
        let mut missing: Vec<usize> = Vec::new();
        for (i, r) in runs.iter().enumerate() {
            match r.iter().find(|(n, _)| *n == c.name) {
                Some((_, m)) => got.push(*m),
                None => missing.push(i + 1),
            }
        }
        if !missing.is_empty() || got.is_empty() {
            ok = false;
            lines.push(format!("{}: baseline {:.3} ms, now MISSING (the bench did not print this case in run(s) {:?}): FAIL", c.name, c.median_ms, missing));
            continue;
        }
        let now = median(&mut got);
        let change = (now - c.median_ms) / c.median_ms * 100.0;
        // compared as a limit in ms, not as the % change: (1.1 - 1.0) / 1.0 * 100 is 10.000000000000009, so a case exactly at its
        // threshold would fail on rounding
        let limit = c.median_ms * (1.0 + c.threshold_pct / 100.0);
        let slow = !change.is_finite() || !limit.is_finite() || now > limit;
        if slow {
            ok = false;
        }
        lines.push(format!(
            "{}: baseline {:.3} ms, now {now:.3} ms, change {change:+.1}% (threshold {}%): {}",
            c.name,
            c.median_ms,
            c.threshold_pct,
            if slow {
                "FAIL, slower than allowed"
            } else {
                "ok"
            }
        ));
    }
    for r in runs.iter().take(1) {
        for (n, m) in r {
            if !base.cases.iter().any(|c| c.name == *n) {
                lines.push(format!("{n}: now {m:.3} ms, no baseline: not judged"));
            }
        }
    }
    (ok, lines)
}

/// THE GATE, the one place every queue path calls (the single-entry check step, a train's check of its tip, every bisect check and
/// a culprit's own check): judge `project` at the commit its task's worktree holds now. `what` names the check in the output.
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
    let head = header(cfg, project, &base);
    println!("{head}");
    let mut lines = vec![head];
    let command = format!("{BENCH_COMMAND} {}", base.runs_per_case);
    let mut runs: Vec<Vec<(String, f64)>> = Vec::new();
    let mut jobs: Vec<String> = Vec::new();
    for i in 1..=cfg.perf_runs {
        println!(
            "perf gate {project}: bench run {i} of {} ({what}): {command}",
            cfg.perf_runs
        );
        let mut info = JobInfo::default();
        let code = job::run_job_info(
            cfg,
            JobSpec {
                kind: "run".into(),
                name: project.to_string(),
                command: Some(command.clone()),
                first_step: 0,
                resumed_from: None,
                resume_count: 0,
                task: Some(task.to_string()),
            },
            &mut info,
        );
        let id = info.id.clone().unwrap_or_else(|| "none".into());
        jobs.push(id.clone());
        if info.summary.is_empty() {
            return Gate::NoVerdict(format!("bench run {i} of {} could not run (exit {code}, job {id}; the reason is printed above, e.g. Broadsword paused, the disk guard or a busy build folder)", cfg.perf_runs));
        }
        if code != 0 {
            let reason = format!("the bench failed: run {i} of {} exited {code} (job {id}: {}); a bench that cannot run on this commit fails the gate", cfg.perf_runs, info.summary.join("; "));
            lines.push(format!("FAILED: {reason}"));
            return Gate::Fail(reason, lines);
        }
        let text = match info.log.as_deref().map(std::fs::read_to_string) {
            Some(Ok(t)) => t,
            Some(Err(e)) => {
                return Gate::NoVerdict(format!(
                    "the log of bench run {i} (job {id}) could not be read: {e}"
                ))
            }
            None => return Gate::NoVerdict(format!("bench run {i} (job {id}) has no log")),
        };
        match parse_bench(&text) {
            Ok(r) => runs.push(r),
            Err(m) => {
                let reason = format!(
                    "the output of bench run {i} of {} (job {id}) cannot be used: {m}",
                    cfg.perf_runs
                );
                lines.push(format!("FAILED: {reason}"));
                return Gate::Fail(reason, lines);
            }
        }
    }
    let (ok, case_lines) = judge(&base, &runs);
    for l in &case_lines {
        println!("perf gate {project}: {l}");
    }
    lines.extend(case_lines.iter().cloned());
    lines.push(format!("bench jobs: {}", jobs.join(", ")));
    if ok {
        lines.push(format!(
            "perf gate {project}: PASS ({} bench run(s))",
            cfg.perf_runs
        ));
        println!("perf gate {project}: PASS");
        Gate::Pass(lines)
    } else {
        let reason = format!(
            "{} ({}): {}",
            if base.loaded {
                "slower than the LOADED baseline allows"
            } else {
                "slower than the baseline allows"
            },
            shown(cfg, &baseline_path(cfg, project)),
            case_lines.join("; ")
        );
        lines.push(format!("perf gate {project}: FAIL"));
        println!("perf gate {project}: FAIL");
        Gate::Fail(reason, lines)
    }
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
                })
                .collect(),
        }
    }

    fn run(cases: &[(&str, f64)]) -> Vec<(String, f64)> {
        cases.iter().map(|(n, m)| (n.to_string(), *m)).collect()
    }

    #[test]
    fn each_case_is_judged_on_the_median_of_the_runs_against_its_own_threshold() {
        let b = base(&[("box", 1.0, 10.0), ("cube", 1.0, 20.0)]);
        // box: median of 1.05, 1.30, 1.08 = 1.08 (+8%, under 10%); cube: +15%, under 20%
        let (ok, lines) = judge(
            &b,
            &[
                run(&[("box", 1.05), ("cube", 1.15)]),
                run(&[("box", 1.30), ("cube", 1.15)]),
                run(&[("box", 1.08), ("cube", 1.15)]),
            ],
        );
        assert!(ok, "{lines:?}");
        assert!(
            lines[0].starts_with(
                "box: baseline 1.000 ms, now 1.080 ms, change +8.0% (threshold 10%): ok"
            ),
            "{lines:?}"
        );
        // cube +15% is over box's 10% but under its own 20%; box +12% is over its 10%
        let (ok, lines) = judge(&b, &[run(&[("box", 1.12), ("cube", 1.15)])]);
        assert!(!ok);
        assert!(
            lines[0].contains("+12.0%") && lines[0].contains("FAIL"),
            "{lines:?}"
        );
        assert!(lines[1].ends_with(": ok"), "{lines:?}");
        // exactly at the threshold passes; faster passes
        assert!(judge(&b, &[run(&[("box", 1.1), ("cube", 0.5)])]).0);
    }

    #[test]
    fn a_case_missing_from_a_run_fails_and_an_extra_case_is_not_judged() {
        let b = base(&[("box", 1.0, 10.0)]);
        let (ok, lines) = judge(&b, &[run(&[("box", 1.0)]), run(&[("other", 1.0)])]);
        assert!(!ok && lines[0].contains("MISSING"), "{lines:?}");
        let (ok, lines) = judge(&b, &[run(&[("box", 1.0), ("new", 9.0)])]);
        assert!(ok && lines[1].contains("not judged"), "{lines:?}");
        assert!(!judge(&b, &[]).0, "no run is no pass");
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
