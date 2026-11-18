//! A stand-in for the `claude` CLI, for the merge-queue tests only. It never calls a model and never touches the network.
//!
//! It reads the prompt from stdin and prints the JSON `claude -p --output-format json` prints. What it does is steered by the
//! prompt and by these env vars (set by the test on the `broadsword` process, which passes them on):
//! - FAKE_CLAUDE_OUT_DIR: if set, it records what it was given there: `calls.log` (one line per call: its arguments),
//!   `stdin.txt` (the last prompt), `env.txt` (its whole environment, `NAME=value` lines) and `cwd.txt`.
//! - a prompt containing `EMPTY_HEADER_PASS` gets a PASS whose `BLOCKING:` header is empty and followed by a finding;
//!   a prompt containing `PLANTED_BUG` gets a FAIL with one blocking finding; `GARBAGE_REVIEW` gets text that is no verdict;
//!   `EMPTY_PASS` gets a PASS with no BLOCKING line; anything else gets a PASS.
//! - FAKE_CLAUDE_HANG_FILE: while that file exists, it waits (so a test can kill it, time it out, or run a second `queue run`
//!   against it). It leaves as soon as the file is removed, or after HANG_CAP_SECONDS if the test never removes it. It used
//!   to sleep a fixed 60 s: under the load of the full suite the first run woke up early, went on to merge and was killed
//!   there, and the resumed run found main dirty (a flaky test).
//! - FAKE_CLAUDE_COMMIT_IN: a folder (a git repo) in which it commits a file first, simulating the main branch moving while
//!   the review runs.
//! - FAKE_CLAUDE_EXIT: exit with that code and print nothing (a CLI that fails).
//! - FAKE_CLAUDE_MODEL: the model name it reports in `modelUsage` (default: `fake-model`, so the fake never passes the
//!   reviewer family check by itself); the value `auto` reports `claude-<the --model family>-4-5` like the real CLI, `none`
//!   leaves `modelUsage` out.
//! - FAKE_CLAUDE_PLAIN: print this plain text instead of the JSON (a program that is not the Claude CLI).

use std::io::{Read, Write};
use std::process::Command;

/// The longest FAKE_CLAUDE_HANG_FILE keeps the fake waiting if the test never removes the file (a failed test must not leave
/// a process behind for long). Far longer than any test needs.
const HANG_CAP_SECONDS: u64 = 300;

fn main() {
    let mut raw = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut raw);
    let input = String::from_utf8_lossy(&raw).into_owned();
    if let Ok(dir) = std::env::var("FAKE_CLAUDE_OUT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let args: Vec<String> = std::env::args().skip(1).collect();
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("calls.log")) {
            let _ = writeln!(f, "{}", args.iter().map(|a| a.replace('\n', "\\n")).collect::<Vec<_>>().join("\u{1f}"));
        }
        let _ = std::fs::write(dir.join("stdin.txt"), &input);
        let env: String = std::env::vars().map(|(k, v)| format!("{k}={v}\n")).collect();
        let _ = std::fs::write(dir.join("env.txt"), env);
        let _ = std::fs::write(dir.join("cwd.txt"), std::env::current_dir().map(|p| p.display().to_string()).unwrap_or_default());
    }
    if let Ok(dir) = std::env::var("FAKE_CLAUDE_COMMIT_IN") {
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@localhost")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@localhost")
                .output()
                .expect("git")
        };
        let _ = std::fs::write(std::path::Path::new(&dir).join("moved-during-review.txt"), "main moved\n");
        git(&["add", "moved-during-review.txt"]);
        git(&["commit", "-q", "-m", "main moved while the review ran"]);
    }
    if let Ok(f) = std::env::var("FAKE_CLAUDE_HANG_FILE") {
        let end = std::time::Instant::now() + std::time::Duration::from_secs(HANG_CAP_SECONDS);
        while std::path::Path::new(&f).exists() && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
    if let Ok(c) = std::env::var("FAKE_CLAUDE_EXIT") {
        std::process::exit(c.parse().unwrap_or(1));
    }
    if let Ok(t) = std::env::var("FAKE_CLAUDE_PLAIN") {
        println!("{t}");
        return;
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = args.iter().position(|a| a == "--model").and_then(|i| args.get(i + 1)).cloned().unwrap_or_default();
    let reported = match std::env::var("FAKE_CLAUDE_MODEL") {
        Ok(m) if m == "auto" => format!("claude-{asked}-4-5"),
        Ok(m) => m,
        Err(_) => "fake-model".to_string(),
    };
    let usage = if reported == "none" { String::new() } else { format!(",\"modelUsage\":{{\"{reported}\":{{}}}}") };
    let result = if input.contains("GARBAGE_REVIEW") {
        "Looks fine to me, nothing to add."
    } else if input.contains("EMPTY_PASS") {
        "VERDICT: PASS\\nMINOR: none"
    } else if input.contains("EMPTY_HEADER_PASS") {
        "VERDICT: PASS\\nBLOCKING:\\n- src/lib.rs:1 - the build is broken\\nMINOR: none"
    } else if input.contains("PLANTED_BUG") {
        "VERDICT: FAIL\\nBLOCKING: src/lib.rs:1 - planted bug: the function subtracts instead of adding\\nMINOR: none"
    } else {
        "VERDICT: PASS\\nBLOCKING: none\\nMINOR: none"
    };
    println!(
        "{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\"result\":\"{result}\",\"total_cost_usd\":0.0123,\"usage\":{{\"input_tokens\":10,\"output_tokens\":5,\"cache_creation_input_tokens\":0,\"cache_read_input_tokens\":100}}{usage}}}"
    );
}
