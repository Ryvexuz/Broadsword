use crate::config::Config;
use crate::record::Record;
use crate::task::fail;
use crate::{pause, sys};
use std::process::ExitCode;
use super::entry::{load_entries, load_entry};
use super::{EXIT_BAD_INPUT, EXIT_ERROR, RUNNING};

fn when(r: &Record, k: &str) -> String {
    r.get_num(k).map(pause::format_utc).unwrap_or_else(|| "-".into())
}

pub(super) fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

/// The first few lines of `s`, for a one-line reason.
pub(super) fn some(s: &str) -> String {
    let lines: Vec<&str> = s.lines().collect();
    format!("{} path(s): {}{}", lines.len(), lines.iter().take(5).map(|l| l.trim_end()).collect::<Vec<_>>().join("; "), if lines.len() > 5 { "; ..." } else { "" })
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
