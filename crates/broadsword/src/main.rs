mod cleanup;
mod config;
mod git;
mod job;
mod json;
mod mcp;
mod pause;
mod recover;
mod record;
mod sys;
mod tools;

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
