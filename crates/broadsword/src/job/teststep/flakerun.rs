//! One serial batch of allow-listed libtest reruns per job.

use super::{rerun_arg, Copied, TestStep};
use crate::job::projects::low_disk_cleanup;
use crate::job::steps::{exit_text, parse_test_results};
use crate::job::{step_line_python, Outcome, PyTests, StepRun};
use crate::pause;
use std::{fs, io};

// main.rs declares this module only for unit tests. Use the same source for the real binary,
// without declaring it twice in a test build.
#[cfg(test)]
use crate::flake;
#[cfg(not(test))]
#[path = "../../flake.rs"]
mod flake;
use flake::{Decision, Entry};

pub(in crate::job) enum Used {
    Refused(String),
    Failed(String),
    Passed(Vec<String>),
}

pub(in crate::job) fn record(used: &Used) -> String {
    match used {
        Used::Refused(reason) => format!("none ({reason})"),
        Used::Failed(results) => results.clone(),
        Used::Passed(tests) => format!("{} failed then passed", tests.join(", ")),
    }
}

pub(in crate::job) fn step_line(
    base: (String, bool),
    used: Option<&Used>,
    run: &StepRun,
    py: Option<&PyTests>,
) -> (String, bool) {
    let tests = match used {
        None => return base,
        Some(Used::Passed(tests)) if matches!(run.outcome, Outcome::Exited(Some(0))) => tests,
        _ => return (base.0, false),
    };
    let Some(counts) = parse_test_results(&run.stdout) else {
        return (base.0, false);
    };
    // A passed batch only excuses the failures in that batch, not other summaries.
    if u64::try_from(tests.len()).ok() != Some(counts.failed) {
        return (base.0, false);
    }
    let line = format!(
        "test: ok after flaky rerun — {} passed, {} failed on the first run; rerun passed: {}",
        counts.passed,
        counts.failed,
        tests.join(", ")
    );
    // The ordinary test verdict was false because it saw the original counts.
    // Apply Python's verdict to the recovered verdict, not to those counts.
    match py {
        Some(p) => step_line_python(line, true, p),
        None => (line, true),
    }
}

impl TestStep<'_> {
    fn read_flaky_list(&self) -> Result<Vec<Entry>, String> {
        // The configured path is validated by Config and is relative to the main checkout.
        let path = self.cfg.root.join(&self.cfg.flaky_tests_file);
        match fs::read_to_string(&path) {
            Ok(text) => {
                flake::parse_list(&text).map_err(|e| format!("list {}: {e}", path.display()))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Err(format!("list {} missing", path.display()))
            }
            Err(e) => Err(format!("cannot read list {}: {e}", path.display())),
        }
    }

    fn refuse_flaky(&mut self, reason: &str) {
        let reason = self.secrets.redact(reason);
        self.note(&format!("flaky rerun: none ({reason})"));
        // A later failure must not overwrite the verdict of the one batch already used.
        if self.flaky.is_none() {
            *self.flaky = Some(Used::Refused(reason));
        }
    }

    pub(super) fn note_parallel_flaky_failure(&self, result: &Result<Outcome, String>) {
        if self.parallel > 1
            && !self.cfg.flaky_tests_file.is_empty()
            && !matches!(result, Ok(Outcome::Exited(Some(0))))
        {
            self.note("flaky rerun: none (test_parallel_binaries is above 1)");
        }
    }

    pub(super) fn flaky_child_error(&mut self, error: String) -> Result<Outcome, String> {
        if self.cfg.flaky_tests_file.is_empty() {
            return Err(error);
        }
        self.refuse_flaky(&error);
        Ok(Outcome::Exited(Some(101)))
    }

    pub(super) fn flaky_disk_drop(&mut self, free: u64) -> Outcome {
        if self.cfg.flaky_tests_file.is_empty() {
            return Outcome::DiskDropped(free);
        }
        let reason = disk_reason(free);
        self.refuse_flaky(&reason);
        self.clean_flaky_disk_drop(&reason);
        Outcome::Exited(Some(101))
    }

    fn clean_flaky_disk_drop(&mut self, reason: &str) {
        // Normalizing the flake-enabled step to 101 must not bypass cleanup and pausing.
        low_disk_cleanup(self.cfg, &mut *self.free, reason);
    }

    pub(super) fn try_flaky_reruns(&mut self, c: &Copied, exit: Option<i32>, stdout: &str) -> bool {
        if self.cfg.flaky_tests_file.is_empty() {
            return false;
        }
        if self.flaky.is_some() {
            self.refuse_flaky("one rerun already used in this job");
            return false;
        }
        let target = rerun_arg(&c.art);
        // A crash is a refusal regardless of what the list or the child's output says.
        let list = if exit == Some(101) {
            match self.read_flaky_list() {
                Ok(list) => list,
                Err(reason) => {
                    self.refuse_flaky(&reason);
                    return false;
                }
            }
        } else {
            Vec::new()
        };
        let names = match flake::eligible(exit, stdout, &list, self.project, &target) {
            Decision::Rerun(names) => names,
            Decision::NoRerun(reason) => {
                self.refuse_flaky(&reason);
                return false;
            }
        };
        if names.iter().any(|name| name.starts_with('-')) {
            self.refuse_flaky("a test name would be interpreted as a libtest option");
            return false;
        }

        let mut passed = Vec::new();
        let mut failures = Vec::new();
        for name in names {
            let label = format!("{target} {name}");
            let mut cmd = self.command_for(c);
            cmd.args([name.as_str(), "--exact"]);
            // Stream through the same runner, but do not add rerun stdout to the first-run counts.
            let (failure, stop) = match self.child(cmd) {
                Ok(run) => match run.outcome {
                    Outcome::Exited(code) if flake::rerun_passed(code, &run.stdout) => {
                        self.note(&format!("flaky rerun: passed {label} (first run failed)"));
                        passed.push(label);
                        continue;
                    }
                    Outcome::Exited(Some(0)) => {
                        ("incomplete or invalid rerun summary".into(), false)
                    }
                    Outcome::Exited(code) => (exit_text(code), false),
                    Outcome::DiskDropped(free) => {
                        let reason = disk_reason(free);
                        self.note(&format!("flaky rerun: failed ({reason}) — {label}"));
                        self.clean_flaky_disk_drop(&reason);
                        failures.push(format!("{label} failed ({reason})"));
                        break;
                    }
                },
                Err(error) => (self.secrets.redact(&error), true),
            };
            self.note(&format!("flaky rerun: failed ({failure}) — {label}"));
            failures.push(format!("{label} failed ({failure})"));
            if stop {
                break;
            }
        }
        if failures.is_empty() {
            *self.flaky = Some(Used::Passed(passed));
            true
        } else {
            failures.extend(passed.iter().map(|label| format!("{label} failed then passed")));
            *self.flaky = Some(Used::Failed(failures.join(", ")));
            false
        }
    }
}

fn disk_reason(free: u64) -> String {
    format!("disk below floor (free {}); job killed", pause::gb(free))
}
