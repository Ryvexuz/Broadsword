//! Pure, fail-closed libtest rerun verdicts. No execution or configuration lives here.

use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Entry {
    pub project: String,
    /// Exactly the cargo arguments printed by `rerun_arg`, including dashes.
    pub target: String,
    pub name: String,
}

#[derive(Debug, Eq, PartialEq)]
pub enum Decision {
    Rerun(Vec<String>),
    NoRerun(String),
}

fn identifier(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

fn test_name(text: &str) -> bool {
    !text.is_empty()
        && !text
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '#' | '/' | '\\'))
}

fn target_arg(text: &str) -> bool {
    text == "--lib"
        || matches!(text.split_once(' '), Some(("--bin" | "--test", name)) if identifier(name))
}

/// Parse `project --lib test` or `project --bin|--test target test`.
/// Blank lines and whole-line comments are ignored; inline comments start with ` # `.
/// Duplicate entries (including entries with different comments) collapse to one.
pub fn parse_list(text: &str) -> Result<Vec<Entry>, String> {
    let mut entries = BTreeSet::new();
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        let error = || format!("allow-list line {}: invalid entry", index + 1);
        if line.chars().any(|c| c.is_control() && c != '\t') {
            return Err(error());
        }
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let body = match line.split_once(" # ") {
            Some((body, comment)) if !comment.trim().is_empty() => body,
            Some(_) => return Err(error()),
            None => line,
        };
        let words: Vec<_> = body.split_whitespace().collect();
        let (project, target, name) = match words.as_slice() {
            [project, "--lib", name] => (*project, "--lib".to_owned(), *name),
            [project, kind @ ("--bin" | "--test"), target, name] if identifier(target) => {
                (*project, format!("{kind} {target}"), *name)
            }
            _ => return Err(error()),
        };
        if !identifier(project) || !test_name(name) {
            return Err(error());
        }
        entries.insert(Entry {
            project: project.into(),
            target,
            name: name.into(),
        });
    }
    Ok(entries.into_iter().collect())
}

#[derive(Debug)]
struct Counts {
    passed: u64,
    failed: u64,
}

fn unsigned(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

// Validate decimal seconds lexically: no floating-point conversion, rounding,
// overflow, NaN or infinity. No arithmetic is performed on this field.
fn seconds(text: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match text.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(text),
    }
}

fn counts(line: &str, status: &str) -> Option<Counts> {
    let rest = line.strip_prefix("test result: ")?.strip_prefix(status)?.strip_prefix(' ')?;
    let (passed, rest) = rest.split_once(" passed; ")?;
    let (failed, rest) = rest.split_once(" failed; ")?;
    let (ignored, rest) = rest.split_once(" ignored; ")?;
    let (measured, rest) = rest.split_once(" measured; ")?;
    let (filtered, duration) = rest.split_once(" filtered out; finished in ")?;
    let passed = unsigned(passed)?;
    let failed = unsigned(failed)?;
    unsigned(ignored)?;
    unsigned(measured)?;
    unsigned(filtered)?;
    if !seconds(duration.strip_suffix('s')?) {
        return None;
    }
    Some(Counts { passed, failed })
}

/// Both verdicts use this parser so neither can accept a truncated, repeated,
/// non-final or malformed summary. CRLF is accepted; embedded CR is not.
fn summary(stdout: &str, status: &str) -> Result<Counts, String> {
    let mut found = None;
    for line in stdout.lines().filter(|line| !line.is_empty()) {
        if line.chars().any(|c| c.is_control() && c != '\t') {
            return Err("control character in test output".into());
        }
        if found.is_some() {
            return Err("text after the summary or multiple summaries".into());
        }
        if line.trim_start().starts_with("test result:") {
            found = Some(counts(line, status).ok_or("incomplete or malformed summary")?);
        }
    }
    found.ok_or_else(|| "missing summary".into())
}

fn failed_tests(stdout: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let mut unique = BTreeSet::new();
    let mut failures = false;
    for line in stdout.lines() {
        if line == "failures:" {
            failures = true;
            break;
        }
        if let Some(name) = line.strip_prefix("test ").and_then(|s| s.strip_suffix(" ... FAILED")) {
            if !test_name(name) || !unique.insert(name) {
                return Err("malformed or duplicate named failure".into());
            }
            names.push(name.to_owned());
        }
    }
    if !failures {
        return Err("missing failures section".into());
    }
    Ok(names)
}

pub fn eligible(
    exit: Option<i32>,
    stdout: &str,
    list: &[Entry],
    project: &str,
    target: &str,
) -> Decision {
    // This guard must precede *all* interpretation of stdout.
    if exit != Some(101) {
        return Decision::NoRerun(format!("crash or kill, exit {exit:?}"));
    }
    if !identifier(project) || !target_arg(target) {
        return Decision::NoRerun("invalid project or target".into());
    }
    let verdict = || -> Result<Vec<String>, String> {
        let counts = summary(stdout, "FAILED.")?;
        let names = failed_tests(stdout)?;
        if names.is_empty() || u64::try_from(names.len()).ok() != Some(counts.failed) {
            return Err("named failures do not match summary".into());
        }
        for name in &names {
            if !list.iter().any(|entry| {
                entry.project == project && entry.target == target && entry.name == *name
            }) {
                return Err(format!("unlisted failure: {name}"));
            }
        }
        Ok(names)
    };
    match verdict() {
        Ok(names) => Decision::Rerun(names),
        Err(reason) => Decision::NoRerun(reason),
    }
}

pub fn rerun_passed(exit: Option<i32>, stdout: &str) -> bool {
    exit == Some(0)
        && summary(stdout, "ok.").is_ok_and(|counts| counts.passed == 1 && counts.failed == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAILED: &str = "running 2 tests\ntest stable ... ok\ntest suite::flaky ... FAILED\n\nfailures:\n\
        \n---- suite::flaky stdout ----\npanic text\n\nfailures:\n    suite::flaky\n\n\
        test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n\n";
    const OK: &str =
        "running 1 test\ntest suite::flaky ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out; finished in 0.00s\n";

    fn list() -> Vec<Entry> {
        parse_list("demo --lib suite::flaky # owner 2026-11-25").unwrap()
    }

    fn decide(exit: Option<i32>, stdout: &str) -> Decision {
        eligible(exit, stdout, &list(), "demo", "--lib")
    }

    fn refused(exit: Option<i32>, stdout: &str) {
        assert!(
            matches!(decide(exit, stdout), Decision::NoRerun(_)),
            "exit={exit:?}, stdout={stdout:?}"
        );
    }

    #[test]
    fn listed_failure_is_eligible_and_single_rerun_passes() {
        assert_eq!(decide(Some(101), FAILED), Decision::Rerun(vec!["suite::flaky".into()]));
        assert!(rerun_passed(Some(0), OK));
        assert!(!rerun_passed(Some(101), FAILED), "a consistent failure remains a failure");
        assert!(!rerun_passed(Some(0), FAILED));
    }

    #[test]
    fn crash_after_a_complete_failure_summary_is_not_rerun() {
        for exit in [None, Some(-1073741819)] {
            assert_eq!(
                decide(exit, FAILED),
                Decision::NoRerun(format!("crash or kill, exit {exit:?}"))
            );
            assert!(!rerun_passed(exit, OK));
        }
    }

    #[test]
    fn exit_other_than_101_is_not_rerun() {
        for code in [0, 1, 2, 3, -1, i32::MIN, i32::MAX] {
            refused(Some(code), FAILED);
        }
    }

    #[test]
    fn summary_cut_after_ignored_is_not_rerun() {
        refused(Some(101), FAILED.split_once("; 0 measured").unwrap().0);
        assert!(!rerun_passed(Some(0), OK.split_once("; 0 measured").unwrap().0));
    }

    #[test]
    fn text_after_the_summary_is_not_rerun() {
        refused(Some(101), &format!("{FAILED}crash after summary\n"));
        assert!(!rerun_passed(Some(0), &format!("{OK}extra\n")));
    }

    #[test]
    fn unlisted_failure_is_not_rerun() {
        for (project, target) in
            [("other", "--lib"), ("demo", "--test demo"), ("demo", "--bin demo")]
        {
            assert!(matches!(
                eligible(Some(101), FAILED, &list(), project, target),
                Decision::NoRerun(_)
            ));
        }
        assert!(matches!(eligible(Some(101), FAILED, &[], "demo", "--lib"), Decision::NoRerun(_)));
        refused(Some(101), &FAILED.replace("suite::flaky", "suite::other"));
    }

    #[test]
    fn a_rerun_that_runs_no_test_is_not_a_pass() {
        assert!(!rerun_passed(Some(0), &OK.replace("1 passed", "0 passed")));
        assert!(!rerun_passed(Some(0), &OK.replace("1 passed", "2 passed")));
        assert!(!rerun_passed(Some(1), OK));
        assert!(!rerun_passed(None, OK));
    }

    #[test]
    fn parse_list_targets_comments_and_duplicates() {
        let entries = parse_list(
            "# header\r\n\n demo --lib suite::flaky # a 2026-11-25\r\n\
             demo --lib suite::flaky # b 2026-11-26\n\
             demo --bin my-tool bin_test\nother --test integration-test suite::test\n",
        )
        .unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.contains(&list()[0]));
        assert!(entries.contains(&Entry {
            project: "demo".into(),
            target: "--bin my-tool".into(),
            name: "bin_test".into(),
        }));
        assert!(entries.contains(&Entry {
            project: "other".into(),
            target: "--test integration-test".into(),
            name: "suite::test".into(),
        }));
        assert!(parse_list("").unwrap().is_empty());
    }

    #[test]
    fn parse_list_errors_name_the_line() {
        for bad in [
            "demo lib test",
            "demo --lib",
            "demo --lib test extra",
            "demo --bin test",
            "demo --example x test",
            "demo --test x test extra",
            "demo --lib test #",
            "demo --lib test#comment",
            "demo --lib test # ",
            "demo --lib te\rst",
            "demo --test C:\\target test",
            "demo --test ../target test",
            "demo --lib test\u{2028}injected",
            "demo --lib test\0",
        ] {
            let error = parse_list(&format!("# header\n{bad}\n")).unwrap_err();
            assert!(error.contains("line 2:"), "input={bad:?}, error={error}");
        }
    }

    #[test]
    fn summaries_require_every_field_and_valid_numbers() {
        for (old, new) in [
            ("1 passed", "-1 passed"),
            ("1 failed", "+1 failed"),
            ("0 ignored", "NaN ignored"),
            ("0 measured", "18446744073709551616 measured"),
            ("0 filtered out", " filtered out"),
            ("0.01s", "NaNs"),
            ("0.01s", "infs"),
            ("0.01s", "-0.01s"),
            ("0.01s", "1e999s"),
            ("0.01s", "0.01"),
            ("0.01s", "0..01s"),
            ("; finished in 0.01s", ""),
            ("0 measured; ", ""),
        ] {
            refused(Some(101), &FAILED.replace(old, new));
        }
        for (old, new) in [
            ("0 measured", "-1 measured"),
            ("0.00s", "NaNs"),
            ("; finished in 0.00s", ""),
        ] {
            assert!(!rerun_passed(Some(0), &OK.replace(old, new)));
        }
    }

    #[test]
    fn integer_field_boundaries_and_decimal_seconds_are_lexical() {
        let maximum = u64::MAX.to_string();
        let line = format!(
            "test result: FAILED. {maximum} passed; {maximum} failed; {maximum} ignored; \
             {maximum} measured; {maximum} filtered out; finished in 0.00000000000000000001s"
        );
        let parsed = counts(&line, "FAILED.").unwrap();
        assert_eq!(parsed.passed, u64::MAX);
        assert_eq!(parsed.failed, u64::MAX);
        assert!(seconds("0"));
        assert!(seconds("0.0"));
        assert!(!seconds(".1"));
        assert!(!seconds("1."));
    }

    #[test]
    fn duplicate_missing_or_mismatched_failure_names_are_refused() {
        refused(
            Some(101),
            &FAILED.replace(
                "test suite::flaky ... FAILED",
                "test suite::flaky ... FAILED\ntest suite::flaky ... FAILED",
            ),
        );
        refused(Some(101), &FAILED.replace("1 failed", "2 failed"));
        refused(Some(101), &FAILED.replace("test suite::flaky ... FAILED\n", ""));
        refused(Some(101), &FAILED.replace("failures:", "missing:"));
        refused(Some(101), &FAILED.replace("1 failed", "0 failed"));
    }

    #[test]
    fn every_failure_must_be_listed_for_the_exact_target() {
        let output = FAILED
            .replace(
                "test suite::flaky ... FAILED",
                "test suite::flaky ... FAILED\ntest suite::second ... FAILED",
            )
            .replace("1 failed", "2 failed");
        refused(Some(101), &output);
        for target in ["--lib", "--bin my-tool", "--test integration-test"] {
            let entries = parse_list(&format!(
                "demo {target} suite::flaky\ndemo {target} suite::second"
            ))
            .unwrap();
            assert_eq!(
                eligible(Some(101), &output, &entries, "demo", target),
                Decision::Rerun(vec!["suite::flaky".into(), "suite::second".into()])
            );
        }
    }

    #[test]
    fn missing_duplicate_and_wrong_status_summaries_are_refused() {
        refused(Some(101), "");
        refused(Some(101), &format!("{FAILED}{FAILED}"));
        refused(Some(101), &FAILED.replace("FAILED.", "ok."));
        assert!(!rerun_passed(Some(0), ""));
        assert!(!rerun_passed(Some(0), &format!("{OK}{OK}")));
    }

    #[test]
    fn crlf_is_accepted_but_embedded_cr_is_not() {
        assert_eq!(decide(Some(101), &FAILED.replace('\n', "\r\n")), decide(Some(101), FAILED));
        assert!(rerun_passed(Some(0), &OK.replace('\n', "\r\n")));
        refused(Some(101), &FAILED.replace("1 failed", "1\r failed"));
    }

    #[test]
    fn direct_entry_point_rejects_invalid_project_and_target() {
        for (project, target) in [
            ("", "--lib"),
            ("C:\\demo", "--lib"),
            ("demo", "--bin C:\\tool"),
            ("demo", "--lib\r"),
            ("demo", "--test"),
        ] {
            let entries = [Entry {
                project: project.into(),
                target: target.into(),
                name: "suite::flaky".into(),
            }];
            assert!(matches!(
                eligible(Some(101), FAILED, &entries, project, target),
                Decision::NoRerun(_)
            ));
        }
    }

    #[test]
    fn large_hostile_fields_fail_closed_without_numeric_overflow() {
        let huge = "9".repeat(100_000);
        refused(Some(101), &FAILED.replace("1 failed", &format!("{huge} failed")));
        refused(Some(101), &format!("{FAILED}{huge}"));
        assert!(!rerun_passed(Some(0), &OK.replace("1 passed", &format!("{huge} passed"))));
        let hostile = format!("demo --lib suite::flaky # {huge}\ndemo --lib suite::other");
        assert_eq!(parse_list(&hostile).unwrap().len(), 2);
    }
}
