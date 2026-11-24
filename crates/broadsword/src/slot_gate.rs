//! CPU admission for new coders; queue checks take priority on a busy machine.

use crate::config::Config;
use crate::{record, sys};
use std::collections::BTreeSet;
use std::io::{self, BufRead, BufReader, Seek};
use std::path::Path;
use std::time::{Duration, Instant};

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
    let cpu = sys::cpu_percent(Duration::from_secs(1));
    sampled_cpu_gate(
        cpu,
        cfg.cpu_warn_percent,
        merge_check_running(
            &record::jobs_dir(cfg),
            Duration::from_millis(cfg.queue_entry_read_wait_ms),
        ),
    )
}

fn sampled_cpu_gate(
    cpu: Result<f64, String>,
    limit: u64,
    merge_check_running: bool,
) -> Result<(), String> {
    // CPU admission protects live queue checks, not historical or unreadable job records.
    if !merge_check_running {
        return Ok(());
    }
    cpu_gate(
        cpu.map_err(|e| format!("CPU could not be read: {e}"))?,
        limit,
        true,
    )
}

/// Only a valid, live running queue check counts. Unreadable or odd records are skipped.
fn merge_check_running(dir: &Path, read_wait: Duration) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    // Share the existing queue-read budget across this scan, rather than waiting per old record.
    let scan_started = Instant::now();
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "rec") {
            return false;
        }
        let text = read_with_retry(scan_started, read_wait, || {
            let file = std::fs::File::open(&path)?;
            read_running_record(BufReader::new(file))
        });
        text.is_some_and(|text| job_is_merge_check(&path, &text))
    })
}

fn read_with_retry(
    scan_started: Instant,
    read_wait: Duration,
    mut read: impl FnMut() -> io::Result<Option<String>>,
) -> Option<String> {
    // Same short backoff as task::queue_view::open_queue_entries, using its configured budget.
    let mut pause = Duration::from_millis(10);
    loop {
        match read() {
            Ok(text) => return text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
            Err(_) => {
                let Some(remaining) = read_wait.checked_sub(scan_started.elapsed()) else {
                    return None;
                };
                if remaining.is_zero() {
                    return None;
                }
                std::thread::sleep(pause.min(remaining));
                pause = (pause * 2).min(Duration::from_millis(250));
            }
        }
    }
}

/// Stop at the status line; only running records are rewound and read in full.
/// Reuse the same open file so an atomic replacement cannot change the record between reads.
fn read_running_record(mut reader: impl BufRead + Seek) -> io::Result<Option<String>> {
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.strip_suffix('\n').unwrap_or(&line);
        let Some((key, value)) = record_line(line) else {
            return Ok(None);
        };
        if key == "status" {
            if value != record::RUNNING {
                return Ok(None);
            }
            reader.rewind()?;
            let mut text = String::new();
            reader.read_to_string(&mut text)?;
            return Ok(Some(text));
        }
    }
}

fn record_line(line: &str) -> Option<(&str, &str)> {
    if line.contains('\r') {
        return None;
    }
    let (key, value) = line
        .split_once(" = ")
        .or_else(|| line.strip_suffix(" =").map(|key| (key, "")))?;
    let key = key.trim();
    (!key.is_empty()).then_some((key, value))
}

fn job_is_merge_check(path: &Path, text: &str) -> bool {
    // Record::parse is permissive. Validate with the same line grammar before parsing so duplicate
    // or malformed fields cannot manufacture a live check. Invalid records never block starts.
    let mut keys = BTreeSet::new();
    for line in text.split_terminator('\n') {
        let Some((key, _)) = record_line(line) else {
            return false;
        };
        if !keys.insert(key) {
            return false;
        }
    }
    let job = record::Record::parse(path, text);
    // job::run_job_recorded records queue_invocation(), not the task id or command text.
    // This also covers queue checks on main (no task), without counting a coder's own job.
    if job.id().is_empty()
        || job.status() != record::RUNNING
        || !matches!(job.get("kind"), "check" | "run")
        || job.get("started_by_queue") != "true"
    {
        return false;
    }
    let (pid, start) = (job.get("pid"), job.get("pid_start"));
    if ![pid, start]
        .iter()
        .all(|value| !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    let (Ok(pid), Ok(start)) = (pid.parse::<u32>(), start.parse::<u64>()) else {
        return false;
    };
    if pid == 0 {
        return false;
    }
    sys::is_alive(pid, Some(start))
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

    #[test]
    fn cpu_read_errors_refuse_only_for_a_live_merge_check() {
        let error: Result<f64, String> = Err("sample failed".into());
        let reason = sampled_cpu_gate(error.clone(), 90, true).unwrap_err();
        assert_eq!(reason, "CPU could not be read: sample failed");
        assert_eq!(sampled_cpu_gate(error, 90, false), Ok(()));
        assert_eq!(sampled_cpu_gate(Ok(f64::NAN), 90, false), Ok(()));
        assert!(sampled_cpu_gate(Ok(f64::NAN), 90, true).is_err());
    }

    #[test]
    fn a_vanished_record_does_not_block_starts() {
        let mut attempts = 0;
        let text = read_with_retry(Instant::now(), Duration::MAX, || {
            attempts += 1;
            Err(io::Error::from(io::ErrorKind::NotFound))
        });
        assert_eq!(
            attempts, 1,
            "a vanished record must not consume the retry budget"
        );
        assert_eq!(text, None);
        let running = text.is_some_and(|text| job_is_merge_check(Path::new("vanished.rec"), &text));
        assert_eq!(sampled_cpu_gate(Ok(95.0), 90, running), Ok(()));
    }

    #[test]
    fn transient_read_errors_are_retried_and_persistent_errors_are_skipped() {
        let mut attempts = 0;
        let text = read_with_retry(Instant::now(), Duration::MAX, || {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                Ok(Some("status = running\n".into()))
            }
        });
        assert_eq!(attempts, 2);
        assert_eq!(text.as_deref(), Some("status = running\n"));

        for kind in [io::ErrorKind::PermissionDenied, io::ErrorKind::InvalidData] {
            let text =
                read_with_retry(Instant::now(), Duration::ZERO, || Err(io::Error::from(kind)));
            assert_eq!(text, None, "read error={kind:?}");
        }
    }

    #[test]
    fn a_running_record_with_an_unreadable_body_is_skipped() {
        let mut bytes = b"status = running\ncommand = ".to_vec();
        bytes.push(0xff);
        let text = read_with_retry(Instant::now(), Duration::ZERO, || {
            read_running_record(io::Cursor::new(&bytes))
        });
        assert_eq!(text, None);
        let running = text.is_some_and(|text| job_is_merge_check(Path::new("partial.rec"), &text));
        assert_eq!(sampled_cpu_gate(Ok(95.0), 90, running), Ok(()));
    }

    #[test]
    fn old_done_records_are_ignored() {
        for status in [
            record::DONE,
            record::FAILED,
            record::REFUSED,
            record::KILLED_LOW_DISK,
            record::RESUMED,
            record::ROLLED_BACK,
            record::ABANDONED,
            "unknown",
            "",
        ] {
            let prefix = format!("id = old-job\nstatus = {status}\n");
            let mut bytes = prefix.as_bytes().to_vec();
            // An unreadable, large tail proves the scanner does not read the historical body.
            bytes.extend(std::iter::repeat_n(0xff, 1_048_576));
            let mut reader = io::Cursor::new(bytes);
            assert_eq!(read_running_record(&mut reader).unwrap(), None, "{status:?}");
            assert_eq!(reader.position(), prefix.len() as u64, "{status:?}");
        }
    }

    #[test]
    fn only_exact_running_status_reads_the_full_record() {
        let text = "id = job-1\nstatus = running\nkind = check\n";
        let mut reader = io::Cursor::new(text);
        assert_eq!(
            read_running_record(&mut reader).unwrap().as_deref(),
            Some(text)
        );
        assert_eq!(reader.position(), text.len() as u64);
        for text in [
            "status = running \n",
            "status = running\\n\n",
            "status = running\r\n",
            "status = unknown\nstatus = running\n",
            "malformed\nstatus = running\n",
            "id = no-status\n",
        ] {
            assert_eq!(
                read_running_record(io::Cursor::new(text)).unwrap(),
                None,
                "{text:?}"
            );
        }
    }

    #[test]
    fn jobs_directory_read_errors_do_not_block_starts() {
        assert!(!merge_check_running(
            Path::new("invalid\0jobs"),
            Duration::ZERO
        ));
    }

    // The OS liveness implementation is Windows-only (sys::process_state elsewhere is a stub).
    #[cfg(windows)]
    mod jobs {
        use super::*;

        fn job(status: &str, kind: &str, queue: &str, task: &str) -> record::Record {
            let mut job = record::Record::parse(Path::new("unused.rec"), "");
            job.set("id", "job-1");
            job.set("status", status);
            job.set("kind", kind);
            job.set("started_by_queue", queue);
            job.set("task", task);
            let pid = std::process::id();
            job.set("pid", pid.to_string());
            job.set(
                "pid_start",
                sys::start_time(pid).expect("own start time").to_string(),
            );
            job
        }

        fn classify(job: &record::Record) -> bool {
            job_is_merge_check(&job.path, &job.render())
        }

        #[test]
        fn queue_full_fast_and_main_checks_count() {
            for kind in ["check", "run"] {
                for task in ["t-entry", ""] {
                    assert!(
                        classify(&job("running", kind, "true", task)),
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
                    assert!(!classify(&job), "kind={kind}, task={task}");
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
                "unknown",
            ] {
                assert!(!classify(&job(status, "check", "true", "t-entry")), "{status}");
            }
            for kind in ["build", "test", "unknown", ""] {
                assert!(!classify(&job("running", kind, "true", "t-entry")), "{kind}");
            }
        }

        #[test]
        fn incomplete_or_ambiguous_running_checks_are_skipped() {
            for field in ["id", "status", "kind", "started_by_queue", "pid", "pid_start"] {
                let mut job = job("running", "check", "true", "t-entry");
                job.set(field, "");
                assert!(!classify(&job), "missing {field}");
            }
            for value in ["TRUE", "true\n", "true\r", " true", "unknown"] {
                assert!(
                    !classify(&job("running", "check", value, "t-entry")),
                    "provenance={value:?}"
                );
            }
            for field in ["pid", "pid_start"] {
                for value in ["-1", "+1", " 1", "1\n", "1\r", "18446744073709551616"] {
                    let mut job = job("running", "check", "true", "t-entry");
                    job.set(field, value);
                    assert!(!classify(&job), "{field}={value:?}");
                }
            }
            let mut job = job("running", "check", "true", "t-entry");
            job.set("pid", "0");
            assert!(!classify(&job));
            job.set("pid", (u64::from(u32::MAX) + 1).to_string());
            assert!(!classify(&job));
        }

        #[test]
        fn a_malformed_record_does_not_block_starts() {
            let valid = job("running", "check", "true", "t-entry").render();
            assert!(job_is_merge_check(Path::new("unused.rec"), &valid));
            for extra in [
                "status = done\n",
                "started_by_queue = false\n",
                " started_by_queue = false\n",
                "# status = done\nnot a field\n",
                "kind = test\n",
                " = empty key\n",
                "command = raw\rcarriage return\n",
                "tail = raw carriage return\r\n",
            ] {
                let running =
                    job_is_merge_check(Path::new("unused.rec"), &format!("{valid}{extra}"));
                assert!(!running, "extra={extra:?}");
                assert_eq!(
                    sampled_cpu_gate(Ok(95.0), 90, running),
                    Ok(()),
                    "extra={extra:?}"
                );
            }
            assert!(!job_is_merge_check(Path::new("unused.rec"), ""));
        }

        #[test]
        fn a_running_record_whose_process_is_gone_is_not_a_merge_check() {
            let mut job = job("running", "check", "true", "t-entry");
            assert!(
                classify(&job),
                "the live fixture must count before changing its pid"
            );
            let mut child = crate::scrub::command_in(std::path::Path::new("."), "cmd")
                .expect("the spawn helper builds the command")
                .args(["/C", "exit 0"])
                .spawn()
                .expect("start a short-lived process");
            let pid = child.id();
            assert!(child.wait().expect("reap the process").success());
            assert_eq!(sys::process_state(pid), sys::ProcState::Gone);
            job.set("pid", pid.to_string());
            assert!(!classify(&job), "a gone process must not hold a coder slot");
            assert_eq!(sampled_cpu_gate(Ok(95.0), 90, classify(&job)), Ok(()));
        }

        #[test]
        fn a_reused_pid_does_not_count_as_the_recorded_process() {
            let mut job = job("running", "check", "true", "t-entry");
            let start = job.get_num("pid_start").unwrap();
            job.set("pid_start", start.checked_add(1).unwrap().to_string());
            assert!(!classify(&job));
        }

        #[test]
        fn large_escaped_command_does_not_hide_provenance() {
            let mut job = job("running", "run", "true", "t-entry");
            job.set(
                "command",
                "status = done\nstarted_by_queue = false\r\nC:\\queue\\run\\ ".repeat(16_384),
            );
            assert!(classify(&job));
            job.set("started_by_queue", "false");
            assert!(!classify(&job));
        }
    }
}
