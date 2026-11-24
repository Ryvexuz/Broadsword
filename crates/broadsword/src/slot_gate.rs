//! CPU admission for new coders; queue checks take priority on a busy machine.

use crate::config::Config;
use crate::{record, sys};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

/// Refuse a busy CPU only while a merge check runs. An unreadable CPU always fails closed.
pub fn cpu_gate(
    cpu_percent: f64,
    cpu_warn_percent: u64,
    merge_check_running: bool,
) -> Result<(), String> {
    if !cpu_percent.is_finite() {
        return Err("CPU could not be read".into());
    }
    if merge_check_running && cpu_percent >= cpu_warn_percent as f64 {
        return Err(format!(
            "CPU is {cpu_percent:.1}% used, at or above cpu_warn_percent {cpu_warn_percent}%; a merge-queue check is running; merge checks come first"
        ));
    }
    Ok(())
}

/// Sample before scanning so checks that start during the sample are visible.
pub fn check_start(cfg: &Config) -> Result<(), String> {
    let cpu = sys::cpu_percent(Duration::from_secs(1))
        .map_err(|e| format!("CPU could not be read: {e}"))?;
    cpu_gate(
        cpu,
        cfg.cpu_warn_percent,
        merge_check_running(&record::jobs_dir(cfg))?,
    )
}

/// Do not use `record::load_all`: it silently drops unreadable records.
fn merge_check_running(dir: &Path) -> Result<bool, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(format!(
                "merge-check jobs could not be read in {}: {e}",
                dir.display()
            ));
        }
    };
    let mut running = false;
    for entry in entries {
        let path = entry
            .map_err(|e| {
                format!(
                    "merge-check jobs could not be listed in {}: {e}",
                    dir.display()
                )
            })?
            .path();
        if path.extension().is_none_or(|ext| ext != "rec") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("merge-check job {} could not be read: {e}", path.display()))?;
        running |= job_is_merge_check(&path, &text)?;
    }
    Ok(running)
}

fn job_is_merge_check(path: &Path, text: &str) -> Result<bool, String> {
    let invalid = |why: &str| format!("merge-check job {} is unusable: {why}", path.display());
    // Record::parse is intentionally permissive. Reject malformed lines and duplicate keys here
    // before parsing, rather than letting a later status/provenance field hide a running check.
    let mut keys = BTreeSet::new();
    for line in text.lines() {
        let (key, _) = line
            .split_once(" = ")
            .or_else(|| line.strip_suffix(" =").map(|key| (key, "")))
            .ok_or_else(|| invalid("malformed record line"))?;
        if key.trim().is_empty() || line.contains('\r') || !keys.insert(key.trim()) {
            return Err(invalid("empty or duplicate key, or unescaped carriage return"));
        }
    }
    let job = record::Record::parse(path, text);
    if job.id().is_empty() {
        return Err(invalid("missing id"));
    }
    match job.status() {
        record::RUNNING => {}
        record::DONE
        | record::FAILED
        | record::REFUSED
        | record::KILLED_LOW_DISK
        | record::RESUMED
        | record::ROLLED_BACK
        | record::ABANDONED => return Ok(false),
        _ => return Err(invalid("missing or unknown status")),
    }
    match job.get("kind") {
        "check" | "run" => {}
        "build" | "test" => return Ok(false),
        _ => return Err(invalid("missing or unknown kind")),
    }
    // job::run_job_recorded records queue_invocation(), not the task id or command text.
    // This also covers queue checks on main (no task), without counting a coder's own job.
    match job.get("started_by_queue") {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(invalid("missing or invalid started_by_queue provenance")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_cpu_during_a_merge_check_refuses() {
        let reason = cpu_gate(95.0, 90, true).unwrap_err();
        assert!(reason.contains("95.0%"), "{reason}");
        assert!(reason.contains("cpu_warn_percent 90%"), "{reason}");
        assert!(
            reason.contains("a merge-queue check is running; merge checks come first"),
            "{reason}"
        );
    }

    #[test]
    fn busy_cpu_without_a_merge_check_allows() {
        assert_eq!(cpu_gate(95.0, 90, false), Ok(()));
    }

    #[test]
    fn quiet_cpu_during_a_merge_check_allows() {
        assert_eq!(cpu_gate(25.0, 90, true), Ok(()));
    }

    #[test]
    fn at_the_limit_refuses() {
        assert!(
            cpu_gate(90.0, 90, true).is_err(),
            "equal CPU and limit must refuse a new coder"
        );
    }

    #[test]
    fn nan_cpu_refuses() {
        for merge_check in [false, true] {
            assert_eq!(
                cpu_gate(f64::NAN, 90, merge_check),
                Err("CPU could not be read".into())
            );
        }
    }

    #[test]
    fn infinite_cpu_refuses() {
        for cpu in [f64::NEG_INFINITY, f64::INFINITY] {
            for merge_check in [false, true] {
                assert_eq!(
                    cpu_gate(cpu, 90, merge_check),
                    Err("CPU could not be read".into())
                );
            }
        }
    }

    #[test]
    fn configured_limit_is_used() {
        assert!(cpu_gate(50.0, 40, true).is_err());
        assert_eq!(cpu_gate(50.0, 60, true), Ok(()));
        assert!(cpu_gate(0.0, 0, true).is_err());
        assert_eq!(cpu_gate(100.0, u64::MAX, true), Ok(()));
    }

    fn job(status: &str, kind: &str, queue: &str, task: &str) -> record::Record {
        let mut job = record::Record::parse(Path::new("unused.rec"), "");
        job.set("id", "job-1");
        job.set("status", status);
        job.set("kind", kind);
        job.set("started_by_queue", queue);
        job.set("task", task);
        job
    }

    fn classify(job: &record::Record) -> Result<bool, String> {
        job_is_merge_check(&job.path, &job.render())
    }

    #[test]
    fn queue_full_fast_and_main_checks_count() {
        for kind in ["check", "run"] {
            for task in ["t-entry", ""] {
                assert_eq!(
                    classify(&job("running", kind, "true", task)),
                    Ok(true),
                    "kind={kind}, task={task}"
                );
            }
        }
    }

    #[test]
    fn coders_own_jobs_do_not_count_even_with_a_task_or_queue_in_command() {
        for kind in ["build", "test", "check", "run"] {
            for task in ["t-entry", ""] {
                let mut job = job("running", kind, "false", task);
                job.set("command", "broadsword queue run --entry q-1");
                job.set("internal_job", "true");
                assert_eq!(classify(&job), Ok(false), "kind={kind}, task={task}");
            }
        }
    }

    #[test]
    fn ended_checks_and_other_queue_job_kinds_do_not_count() {
        for status in [
            record::DONE,
            record::FAILED,
            record::REFUSED,
            record::KILLED_LOW_DISK,
            record::RESUMED,
            record::ROLLED_BACK,
            record::ABANDONED,
        ] {
            assert_eq!(
                classify(&job(status, "check", "true", "t-entry")),
                Ok(false),
                "status={status}"
            );
        }
        for kind in ["build", "test"] {
            assert_eq!(
                classify(&job("running", kind, "true", "t-entry")),
                Ok(false),
                "kind={kind}"
            );
        }
    }

    #[test]
    fn ambiguous_running_checks_fail_closed() {
        for field in ["id", "status", "kind", "started_by_queue"] {
            let mut job = job("running", "check", "true", "t-entry");
            job.set(field, "");
            assert!(classify(&job).is_err(), "missing {field}");
        }
        for value in ["TRUE", "true\n", "true\r", " true", "unknown"] {
            assert!(
                classify(&job("running", "check", value, "t-entry")).is_err(),
                "provenance={value:?}"
            );
        }
        assert!(classify(&job("unknown", "check", "true", "")).is_err());
        assert!(classify(&job("running", "unknown", "true", "")).is_err());
    }

    #[test]
    fn hostile_record_lines_fail_closed() {
        let valid = job("running", "check", "true", "t-entry").render();
        for extra in [
            "status = done\n",
            "started_by_queue = false\n",
            " started_by_queue = false\n",
            "# status = done\nnot a field\n",
            "kind = test\n",
            " = empty key\n",
            "command = raw\rcarriage return\n",
        ] {
            assert!(
                job_is_merge_check(Path::new("unused.rec"), &format!("{valid}{extra}")).is_err(),
                "extra={extra:?}"
            );
        }
        assert!(job_is_merge_check(Path::new("unused.rec"), "").is_err());
    }

    #[test]
    fn large_escaped_command_does_not_hide_provenance() {
        let mut job = job("running", "run", "true", "t-entry");
        job.set(
            "command",
            "status = done\nstarted_by_queue = false\r\nC:\\queue\\run\\ ".repeat(16_384),
        );
        assert_eq!(classify(&job), Ok(true));
        job.set("started_by_queue", "false");
        assert_eq!(classify(&job), Ok(false));
    }

    #[test]
    fn jobs_directory_read_errors_are_reported() {
        let reason = merge_check_running(Path::new("invalid\0jobs")).unwrap_err();
        assert!(reason.contains("merge-check jobs could not be read"), "{reason}");
    }
}
