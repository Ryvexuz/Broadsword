//! The queue-run bash precondition, before any lock, entry claim or job.

use crate::config::Config;
use crate::queue::entry::load_entry;
use crate::queue::EXIT_BAD_INPUT;
use crate::scrub;
use crate::task::fail;
use std::path::PathBuf;
use std::process::{ExitCode, Stdio};

/// Check only the projects this run can touch.
/// A global run without an entry can touch any listed project.
pub(super) fn require_bash(
    cfg: &Config,
    lane_project: Option<&str>,
    entry: Option<&str>,
) -> Result<(), ExitCode> {
    let entry_project = if lane_project.is_none() {
        entry
            .map(|id| load_entry(cfg, id).map(|e| e.get("project").to_string()))
            .transpose()
            .map_err(|m| fail(EXIT_BAD_INPUT, m))?
    } else {
        None
    };
    let project = match lane_project.or(entry_project.as_deref()) {
        Some(p) => cfg
            .bash_projects
            .iter()
            .any(|name| name.eq_ignore_ascii_case(p))
            .then_some(p),
        None => cfg.bash_projects.first().map(String::as_str),
    };
    let Some(project) = project else {
        return Ok(());
    };
    let refuse = |reason: String| {
        fail(
            EXIT_BAD_INPUT,
            format!("queue run: bash not on PATH for project `{project}` (bash_projects): {reason}; put Git for Windows' bin on PATH and start the driver again; nothing was run"),
        )
    };
    let Some(path) = bash_on_path() else {
        return Err(refuse("no bash or bash.exe in any PATH folder".into()));
    };
    // Do not use Command::new("bash"): Windows may find System32's WSL launcher before PATH.
    let failed = |why| {
        refuse(format!(
            "{} was found but `bash -c \"exit 0\"` failed: {why}",
            path.display()
        ))
    };
    let absolute = std::path::absolute(&path).map_err(|e| failed(e.to_string()))?;
    match scrub::command(cfg, absolute)
        .args(["-c", "exit 0"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(failed(status.to_string())),
        Err(e) => Err(failed(e.to_string())),
    }
}

/// Walk PATH folders in order, without the platform's implicit executable search.
fn bash_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["bash.exe", "bash"]
    } else {
        &["bash"]
    };
    for folder in std::env::split_paths(&path) {
        for name in names {
            let candidate = folder.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn unlisted_projects_skip_the_probe_and_entry_load_errors_win() {
        let text = include_str!("../../../../../broadsword.toml");
        let mut cfg = Config::from_text(text, Path::new(".")).unwrap();
        cfg.bash_projects.clear();
        assert!(require_bash(&cfg, None, None).is_ok());
        cfg.bash_projects.push("listed".into());
        // A lane key takes precedence; its caller has already checked any explicit entry.
        assert!(require_bash(&cfg, Some("unlisted"), Some("invalid/id")).is_ok());
        assert_eq!(
            require_bash(&cfg, None, Some("invalid/id")),
            Err(ExitCode::from(EXIT_BAD_INPUT))
        );
    }
}
