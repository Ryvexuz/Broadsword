//! Shared by the integration tests (`mod support;`). Not a test target itself: cargo only runs `tests/*.rs`.

use std::sync::OnceLock;

/// The path of a bin under test. A Broadsword job runs the tests from COPIED executables and points `CARGO_BIN_EXE_<bin>` at
/// a copied bin at run time, so another job's build into the shared build folder cannot swap the binary mid-run. The
/// compile-time `env!` path (the shared folder itself) is only the fallback, for a plain `cargo test`.
fn runtime_or(var: &str, compiled: &'static str, cell: &'static OnceLock<String>) -> &'static str {
    cell.get_or_init(|| std::env::var(var).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| compiled.to_string()))
}

/// The `broadsword` binary under test.
#[allow(dead_code)]
pub fn bin() -> &'static str {
    static B: OnceLock<String> = OnceLock::new();
    runtime_or("CARGO_BIN_EXE_broadsword", env!("CARGO_BIN_EXE_broadsword"), &B)
}

/// Every key of `broadsword.toml` with the value the tests use, as raw TOML (strings keep their quotes). A new required setting
/// is added HERE, once, and every test root gets it.
const BASE: &[(&str, &str)] = &[
    ("root", "\".\""),
    ("disk_floor_gb", "1"),
    ("cleanup_every_hours", "1"),
    ("scratch_max_age_hours", "24"),
    ("disk_check_every_seconds", "60"),
    ("log_max_mb", "50"),
    ("ram_warn_percent", "100"),
    ("cpu_warn_percent", "90"),
    ("alert_command", "\"\""),
    ("scratch_dir", "\"scratch\""),
    ("build_dir", "\"build\""),
    ("projects_dir", "\"projects\""),
    ("state_dir", "\"state\""),
    ("install_dir", "\"build/bin\""),
    ("mcp_call_timeout_seconds", "1800"),
    ("mcp_output_max_kb", "1024"),
    ("checkpoint_ignored_max_mb", "50"),
    ("max_resumes", "3"),
    ("job_disk_reserve_gb", "0"),
    ("job_disk_poll_seconds", "1"),
    ("task_lock_wait_seconds", "10"),
    ("worktrees_dir", "\"worktrees\""),
    ("build_lock_wait_seconds", "10"),
    ("recover_lock_wait_seconds", "5"),
    ("sweep_older_than_days", "7"),
    ("redact_min_chars", "6"),
    ("reviewer_backend", "\"claude\""),
    ("reviewer_cli", "\"claude\""),
    ("reviewer_model_low", "\"haiku\""),
    ("reviewer_model_high", "\"opus\""),
    ("reviewer_model_high_fallback", "\"sonnet\""),
    // "off" so no test can reach a real GPT call by accident; a GPT-gate test sets it and points BROADSWORD_SHADOW_REVIEWER_SCRIPT at a fake
    ("reviewer_model_high_gpt", "\"off\""),
    ("reviewer_effort_high", "\"high\""),
    ("reviewer_timeout_seconds", "5"),
    ("reviewer_diff_max_kb", "200"),
    ("reviewer_brief_max_kb", "32"),
    ("reviewer_check_tail_kb", "8"),
    ("queue_lock_wait_seconds", "2"),
    ("reviewer_max_calls_per_entry", "3"),
    ("merge_train_max", "1"),
    ("merge_train_wait_minutes", "20"),
    ("perf_gate", "\"on\""),
    ("perf_runs", "2"),
    ("queue_per_project", "\"off\""),
    // no test project runs Python tests unless its test sets this (SCALING F9)
    ("python_tests", "\"\""),
    // one after another unless a test sets it: the serial path keeps its streamed output and first-failure-stops order
    ("test_parallel_binaries", "1"),
    ("test_output_wait_seconds", "2"),
    ("shadow_reviewer", "\"off\""),
    ("shadow_reviewer_model", "\"gpt-5.6-sol\""),
    ("shadow_reviewer_timeout_seconds", "5"),
    ("max_agents", "50"),
    ("max_queued", "50"),
    ("queue_entry_read_wait_ms", "2000"),
];

/// `config` without the key `missing`, for the "a required key is missing" tests. Panics if `BASE` lacks it.
#[allow(dead_code)]
pub fn config_without(missing: &str) -> String {
    assert!(BASE.iter().any(|(b, _)| *b == missing), "unknown key `{missing}`");
    config(&[]).lines().filter(|l| l.split_once(" = ").is_none_or(|(k, _)| k != missing)).map(|l| format!("{l}\n")).collect()
}

/// The text of a test root's `broadsword.toml`: `BASE` with `overrides` (raw TOML values) replacing the keys they name. An
/// override naming a key `BASE` lacks panics, so a typo cannot silently add a setting nothing reads.
#[allow(dead_code)]
pub fn config(overrides: &[(&str, &str)]) -> String {
    for (k, _) in overrides {
        assert!(BASE.iter().any(|(b, _)| b == k), "test config override for unknown key `{k}`");
    }
    let mut s = String::new();
    for (k, v) in BASE {
        let v = overrides.iter().find(|(o, _)| o == k).map(|(_, v)| *v).unwrap_or(v);
        s.push_str(&format!("{k} = {v}\n"));
    }
    s
}

/// The `fake_claude` stand-in reviewer.
#[allow(dead_code)]
pub fn fake_claude() -> &'static str {
    static F: OnceLock<String> = OnceLock::new();
    runtime_or("CARGO_BIN_EXE_fake_claude", env!("CARGO_BIN_EXE_fake_claude"), &F)
}
