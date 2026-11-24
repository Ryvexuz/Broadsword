//! Queue-only reruns use exactly the ordinary copied-executable child path.

use super::*;
use crate::flake::{self, Decision};

impl TestStep<'_> {
    pub(super) fn flaky_absent(&mut self, copied: &[Copied]) {
        let Some(entries) = self.flaky else { return };
        self.note(&format!("flaky list: using committed revision {}", entries.rev));
        let mut absent = Vec::new();
        for e in &entries.items {
            if !copied.iter().any(|c| rerun_arg(&c.art) == e.target) && !absent.contains(&e.target) {
                absent.push(e.target.clone());
            }
        }
        for target in absent {
            self.flaky_output(&target, "");
        }
    }

    pub(super) fn flaky_output(&mut self, target: &str, stdout: &str) {
        let Some(entries) = self.flaky else { return };
        for line in flake::unmatched(entries, target, stdout) {
            if !self.flake_report.unmatched.contains(&line) {
                self.note(&line);
                self.flake_report.unmatched.push(line);
            }
        }
    }

    pub(super) fn flaky_none(&mut self, reason: &str) {
        let note = format!("flaky rerun: none ({reason})");
        self.note(&note);
        match &mut self.flake_report.note {
            Some(previous) => {
                previous.push_str("; ");
                previous.push_str(&note);
            }
            slot => *slot = Some(note),
        }
    }

    /// None means no rerun; Some is the batch's effective outcome. Rerun stdout is
    /// logged by `child`, never added to the suite's counts a second time.
    pub(super) fn flaky_rerun(&mut self, c: &Copied, stdout: &str) -> Result<Option<(Outcome, String)>, String> {
        let Some(entries) = self.flaky else { return Ok(None) };
        let target = rerun_arg(&c.art);
        let failed = match flake::failed_tests(stdout) {
            Ok(names) => names,
            Err(_) => {
                self.flaky_none(&format!("the failure of {target} could not be tied to named tests"));
                return Ok(None);
            }
        };
        let names = match flake::decide(entries, &target, &failed, self.flake_report.used) {
            Decision::Rerun(names) => names,
            Decision::NoRerun(reason) => {
                self.flaky_none(&reason);
                return Ok(None);
            }
        };
        let fixed = flake::repaired(stdout)?;
        self.flake_report.used = true;
        let mut results = Vec::new();
        let mut passed = true;
        let mut first_error = None;
        for name in names {
            self.note(&format!("     Flaky rerun of {target}: {name} --exact --test-threads=1"));
            let mut cmd = self.command_for(c);
            cmd.args([name.as_str(), "--exact", "--test-threads=1"]);
            let r = match self.child(cmd) {
                Ok(r) => r,
                Err(e) => {
                    self.note(&format!("flaky rerun of {target}: cannot complete {name}: {e}"));
                    results.push(format!("{name} FAILED then FAILED again"));
                    first_error.get_or_insert(e);
                    passed = false;
                    continue;
                }
            };
            let ok = match r.outcome {
                Outcome::DiskDropped(f) => {
                    results.push(format!("{name} FAILED then FAILED again"));
                    self.flake_report.note = Some(format!("flaky rerun of {target}: {}", results.join(", ")));
                    return Ok(Some((Outcome::DiskDropped(f), stdout.into())));
                }
                Outcome::Exited(code) => flake::rerun_passed(code, &r.stdout),
            };
            passed &= ok;
            results.push(format!("{name} FAILED then {}", if ok { "ok" } else { "FAILED again" }));
        }
        let note = format!("flaky rerun of {target}: {}", results.join(", "));
        self.note(&note);
        self.flake_report.note = Some(note);
        if let Some(e) = first_error {
            return Err(e);
        }
        Ok(Some(if passed {
            (Outcome::Exited(Some(0)), fixed)
        } else {
            (Outcome::Exited(Some(101)), stdout.into())
        }))
    }
}
