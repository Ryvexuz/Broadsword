//! The only copy of the queue flake rule: exact committed allow-list, one serial rerun
//! batch per check, and success only when every named rerun really passes one test.

use crate::{config::Config, wtgit};
use std::collections::HashSet;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Entry {
    pub project: String,
    pub target: String,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct Entries {
    pub project: String,
    pub rev: String,
    pub items: Vec<Entry>,
}

#[derive(Default)]
pub struct Report {
    pub used: bool,
    pub note: Option<String>,
    pub unmatched: Vec<String>,
}

impl Report {
    pub fn record(&self, line: String, rec: &mut crate::record::Record) -> String {
        if !self.unmatched.is_empty() {
            rec.set("flaky_unmatched", self.unmatched.join("; "));
        }
        if let Some(note) = &self.note {
            rec.set("flaky_rerun", note);
            format!("{line}; {note}")
        } else {
            line
        }
    }
}

pub fn parse_list(text: &str) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let w: Vec<_> = line.split_whitespace().take(5).collect();
        let entry = match w.as_slice() {
            [p, "--lib", n] => Some((*p, "--lib".to_string(), *n)),
            [p, flag @ ("--bin" | "--test"), t, n] => Some((*p, format!("{flag} {t}"), *n)),
            _ => None,
        };
        let Some((project, target, name)) = entry else {
            return Err(format!("line {}: expected <project> --lib <test> or <project> --bin/--test <target> <test>", i + 1));
        };
        if line.chars().any(|c| c.is_control() && c != '\t') || name.starts_with('-') {
            return Err(format!("line {}: control character or option-like test name", i + 1));
        }
        let e = Entry { project: project.into(), target, name: name.into() };
        if seen.insert(e.clone()) {
            out.push(e);
        }
    }
    Ok(out)
}

pub fn list_rev(project: &str, main_base: Option<&str>, trunk_tip: &str) -> Option<String> {
    let rev = if project == "broadsword" { main_base.unwrap_or(trunk_tip) } else { trunk_tip };
    (!rev.is_empty()).then(|| rev.to_string())
}

pub fn for_check(cfg: &Config, project: &str, main_base: Option<&str>) -> Option<Entries> {
    let path = &cfg.flaky_tests_file;
    if path.is_empty() {
        return None;
    }
    let read = || -> Result<Entries, String> {
        let tip = if project == "broadsword" && main_base.is_some() {
            String::new()
        } else {
            wtgit::trunk_commit(&cfg.root).map_err(|e| format!("flaky list {path}: cannot choose trunk revision: {e}"))?.1
        };
        let rev = list_rev(project, main_base, &tip).ok_or_else(|| format!("flaky list {path}: no pre_merge_sha for the main check"))?;
        let text = wtgit::file_at(&cfg.root, &rev, path)
            .map_err(|e| format!("flaky list {path} at {rev} cannot be read: {e}"))?
            .ok_or_else(|| format!("flaky list {path} at {rev} is missing"))?;
        let items = parse_list(&text).map_err(|e| format!("flaky list {path} at {rev} is invalid: {e}"))?
            .into_iter().filter(|e| e.project == project).collect::<Vec<_>>();
        if items.is_empty() {
            return Err(format!("flaky list {path} at {rev} has no entries for {project}"));
        }
        Ok(Entries { project: project.into(), rev, items })
    };
    match read() {
        Ok(e) => Some(e),
        Err(e) => {
            eprintln!("{}; no flaky rerun", e.replace(['\r', '\n'], " "));
            None
        }
    }
}

// Require one complete, unambiguous libtest result. Checked arithmetic also protects
// the adjusted first-run counts from overflow before any rerun is allowed.
fn counts(stdout: &str) -> Result<(bool, u64, u64, u64), String> {
    let mut results = stdout.lines().filter_map(|l| l.strip_prefix("test result: "));
    let line = results.next().ok_or("missing test result")?;
    if results.next().is_some() {
        return Err("multiple test results".into());
    }
    let (ok, rest) = if let Some(r) = line.strip_prefix("FAILED. ") {
        (false, r)
    } else if let Some(r) = line.strip_prefix("ok. ") {
        (true, r)
    } else {
        return Err("unknown test result".into());
    };
    let mut parts = rest.split(';');
    let mut number = |label: &str| -> Result<u64, String> {
        let part = parts.next().ok_or("incomplete test result")?.trim();
        let n = part.strip_suffix(label).ok_or("invalid test result label")?;
        if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
            return Err("invalid test result count".into());
        }
        n.parse().map_err(|_| "test result count overflow".into())
    };
    let (p, f, i) = (number(" passed")?, number(" failed")?, number(" ignored")?);
    p.checked_add(f).ok_or("test result total overflow")?;
    Ok((ok, p, f, i))
}

fn test_line(line: &str) -> Option<(&str, &str)> {
    let (name, status) = line.strip_prefix("test ")?.rsplit_once(" ... ")?;
    (!name.is_empty() && !name.chars().any(char::is_control)).then_some((name, status))
}

pub fn failed_tests(stdout: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for line in stdout.lines().take_while(|l| *l != "failures:") {
        if let Some((name, "FAILED")) = test_line(line) {
            if !seen.insert(name) {
                return Err("duplicate failed test name".into());
            }
            names.push(name.to_string());
        }
    }
    let (ok, _, failed, _) = counts(stdout)?;
    if ok || names.is_empty() || u64::try_from(names.len()).ok() != Some(failed) {
        return Err("failure count does not match named tests".into());
    }
    Ok(names)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    Rerun(Vec<String>),
    NoRerun(String),
}

pub fn decide(entries: &Entries, target: &str, failed: &[String], used: bool) -> Decision {
    if used {
        return Decision::NoRerun("one rerun already used in this check".into());
    }
    if failed.is_empty() || failed.iter().collect::<HashSet<_>>().len() != failed.len() {
        return Decision::NoRerun(format!("the failure of {target} could not be tied to named tests"));
    }
    let allowed: HashSet<_> = entries.items.iter().filter(|e| e.project == entries.project && e.target == target).map(|e| e.name.as_str()).collect();
    let listed = |name: &String| allowed.contains(name.as_str());
    if !failed.iter().all(listed) {
        let missing = failed.iter().filter(|n| !listed(n)).map(|n| format!("{target} {n}")).collect::<Vec<_>>();
        return Decision::NoRerun(format!("unlisted failure: {}", missing.join(", ")));
    }
    Decision::Rerun(failed.to_vec())
}

pub fn rerun_passed(exit_code: Option<i32>, stdout: &str) -> bool {
    exit_code == Some(0) && matches!(counts(stdout), Ok((true, 1, 0, _)))
}

pub fn repaired(stdout: &str) -> Result<String, String> {
    let (_, p, f, i) = counts(stdout)?;
    let total = p.checked_add(f).ok_or("test result total overflow")?;
    Ok(stdout.lines().map(|l| {
        if l.starts_with("test result: ") {
            format!("test result: ok. {total} passed; 0 failed; {i} ignored; 0 measured; 0 filtered out\n")
        } else {
            format!("{l}\n")
        }
    }).collect())
}

pub fn unmatched(entries: &Entries, target: &str, stdout: &str) -> Vec<String> {
    let names: HashSet<_> = stdout.lines().take_while(|l| *l != "failures:").filter_map(test_line).map(|(n, _)| n).collect();
    entries.items.iter().filter(|e| e.project == entries.project && e.target == target && !names.contains(e.name.as_str()))
        .map(|e| format!("flaky list: {} {target} {} matches no test of {target} (ignored)", e.project, e.name)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(text: &str) -> Entries {
        Entries { project: "p".into(), rev: "abc".into(), items: parse_list(text).unwrap() }
    }

    const BAD: &str = "test x ... FAILED\nfailures:\n\ntest result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out\n";
    const GOOD: &str = "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out\n";

    #[test]
    fn parse_list_reads_the_allow_list() {
        let e = parse_list("# comment\n\np --lib x\np --bin tool m::x\np --test it x\n").unwrap();
        assert_eq!(e.len(), 3);
        assert_eq!(e[0].target, "--lib");
        assert_eq!(e[2].target, "--test it");
        assert_eq!((&*e[1].project, &*e[1].target, &*e[1].name), ("p", "--bin tool", "m::x"));
        for bad in ["p x", "p --example x y", "p --test x y z", "p --lib --help", "p --lib x\u{0}", "p --lib x # comment"] {
            assert!(parse_list(bad).is_err(), "{bad:?}");
        }
        assert_eq!(parse_list("p --lib x\np --lib x").unwrap().len(), 1);
        let huge = (0..10000).map(|i| format!("p --lib test_{i}\n")).collect::<String>();
        assert_eq!(parse_list(&huge).unwrap().len(), 10000);
        assert!(parse_list(&format!("p --lib x {}", "extra ".repeat(10000))).is_err());
    }

    #[test]
    fn config_refuses_a_bad_list_path() {
        for bad in ["..", "/abs", "C:x", "a\\b", "./x", "a//b", "x\r", "x\n", "a/../b"] {
            assert!(crate::wtgit::check_list_path(bad).is_err(), "{bad:?}");
        }
        for good in ["flaky_tests.txt", "", "lists/flaky.txt"] {
            assert_eq!(crate::wtgit::check_list_path(good).unwrap(), good);
        }
        let text = include_str!("../../../broadsword.toml");
        assert!(Config::from_text(&text.replace("flaky_tests_file =", "# flaky_tests_file ="), std::path::Path::new("p")).is_err());
        assert!(Config::from_text(&text.replace("flaky_tests_file = \"flaky_tests.txt\"", "flaky_tests_file = \"../bad\""), std::path::Path::new("p")).is_err());
    }

    #[test]
    fn failed_tests_names_the_failures() {
        assert_eq!(failed_tests(BAD).unwrap(), ["x"]);
        assert_eq!(failed_tests(&BAD.replace("test x", "test m::x")).unwrap(), ["m::x"]);
        assert_eq!(failed_tests(&format!("test z ... FAILED\n{}", BAD.replace("1 failed", "2 failed"))).unwrap(), ["z", "x"]);
        assert!(repaired(BAD).unwrap().contains("ok. 3 passed; 0 failed; 0 ignored;"));
    }

    #[test]
    fn failed_tests_refuses_what_it_cannot_name() {
        for text in ["test x ... FAILED\n", GOOD, "abort", "failures:\ntest x ... FAILED\n"] {
            assert!(failed_tests(text).is_err(), "{text}");
        }
        for text in [format!("test y ... FAILED\n{BAD}"), BAD.replace("1 failed", "0 failed"),
            BAD.replace("test x ... FAILED\nfailures:", "failures:\ntest x ... FAILED"),
            format!("{BAD}{BAD}"), BAD.replace("2 passed", "18446744073709551615 passed"),
            BAD.replace("1 failed", "-1 failed"), BAD.replace("1 failed", "18446744073709551616 failed")] {
            assert!(failed_tests(&text).is_err(), "{text}");
        }
        assert_eq!(failed_tests(&BAD.replace("failures:", "failures:\ntest fake ... FAILED")).unwrap(), ["x"]);
        assert!(failed_tests(&format!("{}test x ... FAILED\n", "noise\n".repeat(10000))).is_err());
    }

    #[test]
    fn decide_needs_every_failure_listed() {
        let e = entries("p --lib x\np --test it y\nq --lib z");
        assert_eq!(decide(&e, "--lib", &["x".into()], false), Decision::Rerun(vec!["x".into()]));
        for n in ["y", "z", "missing"] {
            assert!(matches!(decide(&e, "--lib", &["x".into(), n.into()], false), Decision::NoRerun(_)));
        }
        assert!(matches!(decide(&e, "--lib", &[], false), Decision::NoRerun(_)));
        assert!(matches!(decide(&e, "--lib", &["x".into(), "x".into()], false), Decision::NoRerun(_)));
    }

    #[test]
    fn decide_allows_one_rerun_per_check() {
        let e = entries("p --lib x");
        assert!(matches!(decide(&e, "--lib", &["x".into()], true), Decision::NoRerun(_)));
        assert!(matches!(decide(&e, "--lib", &["x".into()], false), Decision::Rerun(_)));
    }

    #[test]
    fn a_rerun_that_runs_no_test_is_not_a_pass() {
        assert!(rerun_passed(Some(0), GOOD));
        for text in [GOOD.replace("1 passed", "0 passed"), GOOD.replace("1 passed", "2 passed"), format!("{GOOD}{GOOD}"), String::new()] {
            assert!(!rerun_passed(Some(0), &text));
        }
        assert!(!rerun_passed(Some(101), GOOD));
        assert!(!rerun_passed(None, GOOD));
    }

    #[test]
    fn broadsword_main_check_reads_the_list_before_the_merge() {
        assert_eq!(list_rev("broadsword", Some("abc"), "tip"), Some("abc".into()));
        assert_eq!(list_rev("broadsword", Some(""), "tip"), None);
        assert_eq!(list_rev("crash-sim", Some("abc"), "tip"), Some("tip".into()));
        for project in ["broadsword", "crash-sim"] {
            assert_eq!(list_rev(project, None, "tip"), Some("tip".into()));
        }
    }

    #[test]
    fn unmatched_entries_are_reported() {
        let e = entries("p --lib x\np --lib missing\np --test absent z");
        assert_eq!(unmatched(&e, "--lib", BAD), ["flaky list: p --lib missing matches no test of --lib (ignored)"]);
        assert_eq!(unmatched(&e, "--test absent", ""), ["flaky list: p --test absent z matches no test of --test absent (ignored)"]);
        assert!(unmatched(&e, "--lib", "test x ... ok\ntest missing ... ignored\n").is_empty());
    }
}
