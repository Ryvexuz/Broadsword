//! Pause file, alerts, resume decision and the PreToolUse hook decision.
//!
//! Known limit of the pause allow-list (`is_allowed_broadsword_command`): the program is matched by FILE NAME only, so any
//! path whose last part is `broadsword` or `broadsword.exe` passes while paused (for example `scratch/x/broadsword.exe cleanup`).
//! A binary planted under that name before the pause could therefore run. The rule is kept as is: the installed copy's
//! path differs per machine and root, and the subcommand and the absence of chaining are still enforced.

use crate::config::{Config, GB};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const PAUSE_FILE: &str = "broadsword.paused";

pub fn pause_path(root: &Path) -> PathBuf {
    root.join(PAUSE_FILE)
}

pub fn is_paused(root: &Path) -> bool {
    pause_path(root).is_file()
}

pub fn read_pause(root: &Path) -> Option<String> {
    std::fs::read_to_string(pause_path(root)).ok()
}

/// Format unix seconds as `YYYY-MM-DD HH:MM:SS UTC`.
pub fn format_utc(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rem = secs % 86400;
    // Civil-from-days (Howard Hinnant).
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", rem / 3600, rem % 3600 / 60, rem % 60)
}

pub fn now_string() -> String {
    format_utc(SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0))
}

pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GB as f64)
}

pub fn write_pause(root: &Path, free: u64, reason: &str) -> std::io::Result<()> {
    let body = format!("time: {}\nfree: {}\nreason: {}\n", now_string(), gb(free), reason);
    std::fs::write(pause_path(root), body)
}

/// Pure decision: below the floor means pause.
pub fn below_floor(free: u64, floor: u64) -> bool {
    free < floor
}

/// Resume decision. Ok = remove the pause file; Err(bytes_needed) = refuse.
pub fn resume_decision(free: u64, floor: u64, force: bool) -> Result<(), u64> {
    if force || free > floor {
        Ok(())
    } else {
        Err(floor + 1 - free)
    }
}

/// Run `cmd /C "<command> "<message>""` with the command line passed verbatim, so Rust's
/// argument quoting cannot mangle it. cmd strips the outer quotes.
#[cfg(windows)]
fn run_cmd_verbatim(command: &str, msg: &str) -> std::io::Result<std::process::ExitStatus> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("cmd")
        .arg("/C")
        .raw_arg(format!("\"{command} \"{msg}\"\""))
        .status()
}

#[cfg(not(windows))]
fn run_cmd_verbatim(_: &str, _: &str) -> std::io::Result<std::process::ExitStatus> {
    unreachable!("only used on Windows")
}

/// Alert: stderr, scratch/logs/monitor.log, and `alert_command` if set.
pub fn alert(cfg: &Config, message: &str) {
    eprintln!("ALERT: {message}");
    let logs = cfg.scratch_dir.join("logs");
    if std::fs::create_dir_all(&logs).is_ok() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(logs.join("monitor.log")) {
            let _ = writeln!(f, "[{}] {}", now_string(), message);
        }
    }
    if !cfg.alert_command.trim().is_empty() {
        let msg: String = message.chars().filter(|c| !matches!(c, '"' | '%')).collect();
        let result = if cfg!(windows) {
            run_cmd_verbatim(&cfg.alert_command, &msg)
        } else {
            std::process::Command::new("sh").arg("-c").arg(format!("{} \"$0\"", cfg.alert_command)).arg(&msg).status()
        };
        if let Err(e) = result {
            eprintln!("alert_command failed to start: {e}");
        }
    }
}

/// Extract the value of `"tool_name"` from hook JSON (no full JSON parser).
pub fn tool_name(json: &str) -> Option<String> {
    let after = &json[json.find("\"tool_name\"")? + "\"tool_name\"".len()..];
    let after = after.trim_start().strip_prefix(':')?.trim_start().strip_prefix('"')?;
    Some(after[..after.find('"')?].to_string())
}

/// Prefix Claude Code gives the Broadsword MCP server's tools in `tool_name`.
pub const MCP_PREFIX: &str = "mcp__broadsword__";

/// An MCP tool of this server that may run while paused: every read-only registry tool (from the registry's own
/// `read_only` flag, not a list kept here), plus `resume` and `cleanup`, which are how the pause is ended. Whatever
/// is allowed here is still subject to the job guard inside the child process.
pub fn mcp_tool_allowed_while_paused(tool_name: &str) -> bool {
    let Some(name) = tool_name.strip_prefix(MCP_PREFIX) else { return false };
    matches!(name, "resume" | "cleanup") || crate::tools::find(name).is_some_and(|t| t.read_only)
}

/// The broadsword commands a shell call may run while paused: how the pause is inspected and ended.
const PAUSED_SHELL_COMMANDS: [&str; 3] = ["status", "resume", "cleanup"];

/// True only if the whole shell command line IS one broadsword command from `allowed`: the program is broadsword (a bare
/// name or a path ending in `broadsword` or `broadsword.exe`), the rest parses as exactly one registry command (the CLI's
/// own parser decides, so an unknown flag or word is refused) whose tool is in `allowed`, and nothing else is on the
/// line: no `;`, `&`, `|`, newline, redirection, substitution (`$`, backtick, parentheses) and no quote after the program.
/// A command that merely CONTAINS `broadsword status` (`rm x; broadsword status`) never passes.
pub fn is_allowed_broadsword_command(line: &str, allowed: &[&str]) -> bool {
    let line = line.trim();
    if line.is_empty() || line.contains([';', '&', '|', '\n', '\r', '<', '>', '`', '$', '(', ')', '%', '^']) {
        return false;
    }
    // The program may be quoted (a path with spaces); no other token may hold a quote.
    let (program, rest) = match line.chars().next() {
        Some(q @ ('"' | '\'')) => match line[1..].split_once(q) {
            Some((prog, rest)) if !prog.contains(['"', '\'']) && (rest.is_empty() || rest.starts_with(char::is_whitespace)) => (prog, rest),
            _ => return false,
        },
        _ => line.split_once(char::is_whitespace).unwrap_or((line, "")),
    };
    if rest.contains(['"', '\'']) {
        return false;
    }
    let file = program.rsplit(['/', '\\']).next().unwrap_or("").to_ascii_lowercase();
    if file != "broadsword" && file != "broadsword.exe" {
        return false;
    }
    let args: Vec<&str> = rest.split_whitespace().collect();
    crate::tools::resolve(&args).is_some_and(|(tool, _)| allowed.contains(&tool.name))
}

/// The command line of a Bash or PowerShell hook call (`tool_input.command`), or None if the JSON has no such string.
fn shell_command(stdin: &str) -> Option<String> {
    let j = crate::json::parse(stdin).ok()?;
    Some(j.get("tool_input")?.get("command")?.str()?.to_string())
}

/// True if the hook call may proceed while paused.
pub fn allowed_while_paused(stdin: &str) -> bool {
    match tool_name(stdin).as_deref() {
        Some("Read" | "Glob" | "Grep") => true,
        Some(n) if n.starts_with(MCP_PREFIX) => mcp_tool_allowed_while_paused(n),
        Some("Bash" | "PowerShell") => shell_command(stdin).is_some_and(|c| is_allowed_broadsword_command(&c, &PAUSED_SHELL_COMMANDS)),
        _ => false,
    }
}

/// True if the hook call is read-only: it may proceed when the config cannot be loaded (the hook then fails closed for
/// everything else). Read, Glob, Grep, a read-only tool of the Broadsword MCP server, or the shell command `broadsword status`.
pub fn read_only_call(stdin: &str) -> bool {
    match tool_name(stdin).as_deref() {
        Some("Read" | "Glob" | "Grep") => true,
        Some(n) if n.starts_with(MCP_PREFIX) => crate::tools::find(&n[MCP_PREFIX.len()..]).is_some_and(|t| t.read_only),
        Some("Bash" | "PowerShell") => shell_command(stdin).is_some_and(|c| is_allowed_broadsword_command(&c, &["status"])),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pause_below_floor_only() {
        assert!(below_floor(19, 20));
        assert!(!below_floor(20, 20));
    }

    #[test]
    fn resume_refuses_at_or_below_floor() {
        assert_eq!(resume_decision(100, 100, false), Err(1));
        assert_eq!(resume_decision(90, 100, false), Err(11));
        assert_eq!(resume_decision(101, 100, false), Ok(()));
        assert_eq!(resume_decision(0, 100, true), Ok(()));
    }

    #[test]
    fn utc_format() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13:20 UTC");
    }

    #[test]
    fn hook_decisions() {
        let w = r#"{"tool_name":"Write","tool_input":{"file_path":"a"}}"#;
        let r = r#"{ "tool_name" : "Read" }"#;
        let ok = r#"{"tool_name":"Bash","tool_input":{"command":"build/debug/broadsword.exe status"}}"#;
        let ok2 = r#"{"tool_name":"Bash","tool_input":{"command":"broadsword resume --force"}}"#;
        let bad = r#"{"tool_name":"Bash","tool_input":{"command":"cargo build"}}"#;
        assert_eq!(tool_name(w).as_deref(), Some("Write"));
        assert!(!allowed_while_paused(w));
        assert!(allowed_while_paused(r));
        assert!(allowed_while_paused(ok), "a real path to broadsword.exe passes");
        assert!(allowed_while_paused(ok2));
        assert!(!allowed_while_paused(bad));
        assert!(allowed_while_paused(r#"{"tool_name":"PowerShell","tool_input":{"command":"broadsword cleanup"}}"#));
        assert!(!allowed_while_paused(r#"{"tool_name":"PowerShell","tool_input":{"command":"dir"}}"#));
        assert!(!allowed_while_paused("garbage"));
    }

    fn shell(tool: &str, cmd: &str) -> String {
        format!(r#"{{"tool_name":"{tool}","tool_input":{{"command":{}}}}}"#, crate::json::string(cmd))
    }

    #[test]
    fn paused_shell_calls_that_are_exactly_an_allowed_broadsword_command_pass() {
        for ok in [
            "broadsword status",
            "broadsword.exe status",
            "  broadsword status  ",
            "build/bin/broadsword.exe status",
            "C:\\Users\\x\\build\\bin\\broadsword.exe status",
            "./Broadsword.EXE cleanup",
            "broadsword resume",
            "broadsword resume --force",
            "\"C:/My Dir/broadsword.exe\" status",
        ] {
            assert!(allowed_while_paused(&shell("Bash", ok)), "{ok}");
            assert!(allowed_while_paused(&shell("PowerShell", ok)), "{ok}");
        }
    }

    #[test]
    fn paused_shell_calls_that_merely_contain_an_allowed_command_are_denied() {
        for bad in [
            "rm x; broadsword status",
            "broadsword status && rm x",
            "broadsword status; rm x",
            "broadsword status | cat",
            "broadsword status & del x",
            "broadsword status\nrm x",
            "rm x\nbroadsword status",
            "broadsword status\r\nrm x",
            "echo broadsword status",
            "echo broadsword status > x",
            "rm -rf x # broadsword status",
            "broadsword status $(rm x)",
            "broadsword status `rm x`",
            "broadsword status > out.txt",
            "evil broadsword status",
            "notbroadsword status",
            "broadswordx.exe status",
            "broadsword.exe.bat status",
            "broadsword",
            "broadsword build broadsword",
            "broadsword status extra",
            "broadsword status \"x\"",
            "broadsword task add",
            "\"broadsword status",
            "",
        ] {
            assert!(!allowed_while_paused(&shell("Bash", bad)), "{bad:?}");
            assert!(!allowed_while_paused(&shell("PowerShell", bad)), "{bad:?}");
        }
        // a command string that is not where the hook JSON puts it never passes by substring
        assert!(!allowed_while_paused(r#"{"tool_name":"Bash","tool_input":{"command":"rm x","note":"broadsword status"}}"#));
        assert!(!allowed_while_paused(r#"{"tool_name":"Bash","tool_input":{"description":"broadsword status"}}"#));
        assert!(!allowed_while_paused(r#"{"tool_name":"Bash","tool_input":{"command":"rm x"}} broadsword status"#));
    }

    #[test]
    fn read_only_calls_are_the_only_ones_allowed_without_a_config() {
        assert!(read_only_call(&hook_for("Read")) && read_only_call(&hook_for("Glob")) && read_only_call(&hook_for("Grep")));
        assert!(read_only_call(&hook_for("mcp__broadsword__status")) && read_only_call(&hook_for("mcp__broadsword__task_list")));
        assert!(read_only_call(&shell("Bash", "broadsword status")));
        for bad in ["Write", "Edit", "WebFetch", "Agent", "mcp__broadsword__build", "mcp__broadsword__cleanup", "mcp__broadsword__resume", "mcp__other__status", ""] {
            assert!(!read_only_call(&hook_for(bad)), "{bad}");
        }
        for bad in ["broadsword cleanup", "broadsword resume", "rm x; broadsword status", "cargo build"] {
            assert!(!read_only_call(&shell("Bash", bad)), "{bad}");
        }
        assert!(!read_only_call("garbage") && !read_only_call(""));
    }

    fn hook_for(tool: &str) -> String {
        format!(r#"{{"tool_name":"{tool}","tool_input":{{}}}}"#)
    }

    #[test]
    fn mcp_tools_while_paused() {
        for ok in ["status", "tools", "projects", "jobs", "resume", "cleanup", "task_list", "task_show", "task_check", "worktree_list"] {
            assert!(allowed_while_paused(&hook_for(&format!("mcp__broadsword__{ok}"))), "{ok}");
        }
        for blocked in ["build", "test", "check", "run", "recover", "abandon", "watch", "install", "task_add", "task_start", "task_done", "task_fail", "task_abandon", "worktree_create", "worktree_remove", "worktree_prune", "worktree_repair", "nope", ""] {
            assert!(!allowed_while_paused(&hook_for(&format!("mcp__broadsword__{blocked}"))), "{blocked}");
        }
        // only this server's prefix counts
        for other in ["mcp__other__status", "mcp__broadswordx__status", "broadsword__status", "mcp__broadsword_status"] {
            assert!(!allowed_while_paused(&hook_for(other)), "{other}");
        }
    }

    #[test]
    fn mcp_allow_list_is_derived_from_the_registry() {
        // Exactly the read-only tools plus resume and cleanup: a new read-only tool is allowed without touching this file,
        // and a tool that is not read-only is never allowed by accident.
        let mut allowed: Vec<&str> = crate::tools::TOOLS.iter().filter(|t| mcp_tool_allowed_while_paused(&format!("mcp__broadsword__{}", t.name))).map(|t| t.name).collect();
        allowed.sort_unstable();
        let mut want: Vec<&str> = crate::tools::TOOLS.iter().filter(|t| t.read_only || t.name == "resume" || t.name == "cleanup").map(|t| t.name).collect();
        want.sort_unstable();
        assert_eq!(allowed, want);
        assert_eq!(allowed, ["cleanup", "jobs", "projects", "queue_list", "queue_show", "resume", "status", "task_check", "task_list", "task_show", "tools", "worktree_list"]);
    }
}
