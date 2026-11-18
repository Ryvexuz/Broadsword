// A closed stdout or stderr must never kill Broadsword. std's println! and friends PANIC when the reader has gone
// (`broadsword run ... | head` closes the pipe early), and a panic in the middle of a job leaves its record `running`,
// which blocks every new job on that project until someone runs `recover`. These shadow the std macros for the whole
// crate (they are defined before every `mod`, so every module sees them): output that cannot be written is dropped,
// and the job carries on and records its real result. The job log is unaffected (it is written to a file).
macro_rules! println {
    () => {{ use std::io::Write as _; let _ = writeln!(std::io::stdout()); }};
    ($($a:tt)*) => {{ use std::io::Write as _; let _ = writeln!(std::io::stdout(), $($a)*); }};
}
macro_rules! print {
    ($($a:tt)*) => {{ use std::io::Write as _; let _ = write!(std::io::stdout(), $($a)*); }};
}
macro_rules! eprintln {
    () => {{ use std::io::Write as _; let _ = writeln!(std::io::stderr()); }};
    ($($a:tt)*) => {{ use std::io::Write as _; let _ = writeln!(std::io::stderr(), $($a)*); }};
}
macro_rules! eprint {
    ($($a:tt)*) => {{ use std::io::Write as _; let _ = write!(std::io::stderr(), $($a)*); }};
}

mod buildsrc;
mod cleanup;
mod config;
mod git;
mod install;
mod job;
mod json;
mod lock;
mod mcp;
mod pause;
mod queue;
mod recover;
mod record;
mod reviewer;
mod sys;
mod task;
mod tools;
mod worktree;
mod wtgit;

use config::Config;
use std::io::Read;
use std::process::ExitCode;

/// `broadsword hook pre-tool`: Claude Code's PreToolUse hook. Plumbing, not a tool (see tools.rs), so it
/// stays here, outside the registry.
fn cmd_hook(cfg: &Config) -> ExitCode {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if !pause::is_paused(&cfg.root) || pause::allowed_while_paused(&input) {
        return ExitCode::SUCCESS;
    }
    let reason = pause::read_pause(&cfg.root).unwrap_or_default();
    eprintln!(
        "Broadsword is PAUSED (low disk). Only read-only tools and `broadsword status|resume|cleanup` are allowed.\n{}",
        reason.trim_end()
    );
    ExitCode::from(2)
}

fn usage() -> ExitCode {
    eprint!("{}", tools::usage_text());
    ExitCode::from(64)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `broadsword mcp`: the MCP transport, not a registry tool (see mcp.rs). It must serve even when the config is
    // broken (the config only decides where the log goes), and it must never write to stdout except protocol messages.
    if args.len() == 1 && args[0] == "mcp" {
        return mcp::run(config::load().ok().as_ref());
    }
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            // Fail open on purpose: in hook mode exit 1 is a non-blocking error, so a broken
            // or missing broadsword.toml never locks agents out of every tool call.
            eprintln!("broadsword: {e}");
            return ExitCode::from(1);
        }
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if args == ["hook", "pre-tool"] {
        return cmd_hook(&cfg);
    }
    // Every other command is a registry entry: resolve it to the tool and its typed inputs, then call its handler.
    let Some((tool, inputs)) = tools::resolve(&args) else { return usage() };
    (tool.handler)(&cfg, &inputs).unwrap_or_else(|e| {
        eprintln!("broadsword: {e}");
        ExitCode::from(1)
    })
}
