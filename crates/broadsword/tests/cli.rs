//! End-to-end tests: run the built `broadsword` binary inside a throwaway project
//! folder under the repo's `scratch/` area. Nothing here touches the real project
//! files; each test deletes its own folder when it ends.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");

/// A throwaway project folder with its own broadsword.toml. Removed on drop.
struct Project {
    dir: PathBuf,
}

impl Project {
    fn new(name: &str, disk_floor_gb: u64) -> Project {
        let scratch_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        let dir = scratch_root.join(format!("cli-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        let toml = format!(
            "root = \".\"\n\
             disk_floor_gb = {disk_floor_gb}\n\
             cleanup_every_hours = 1\n\
             scratch_max_age_hours = 24\n\
             disk_check_every_seconds = 60\n\
             log_max_mb = 50\n\
             ram_warn_percent = 100\n\
             cpu_warn_percent = 90\n\
             alert_command = \"\"\n\
             scratch_dir = \"scratch\"\n\
             build_dir = \"build\"\n\
             projects_dir = \"projects\"\n\
             state_dir = \"state\"\n\
             install_dir = \"build/bin\"\n\
             mcp_call_timeout_seconds = 1800\n\
             mcp_output_max_kb = 1024\n\
             checkpoint_ignored_max_mb = 50\n\
             max_resumes = 3\n\
             job_disk_reserve_gb = 2\n\
             job_disk_poll_seconds = 5\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = 10\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
light_check_paths = \"docs/,*.md\"
light_check_timeout_seconds = 300
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
"
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Project { dir }
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }

    fn pause_file(&self) -> PathBuf {
        self.path("broadsword.paused")
    }

    fn monitor_log(&self) -> PathBuf {
        self.path("scratch/logs/monitor.log")
    }

    /// Run the binary with the project as the working directory, feeding `stdin`.
    fn run(&self, args: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(BIN)
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to start broadsword");
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    }

    /// Run `broadsword cleanup` and check that it paused the project.
    fn pause_via_cleanup(&self) {
        let out = self.run(&["cleanup"], "");
        assert!(out.status.success(), "cleanup failed: {}", text(&out.stderr));
        assert!(self.pause_file().is_file(), "cleanup below the floor should create broadsword.paused");
    }

    /// Set a file's modified time to `hours` ago.
    fn age(&self, rel: &str, hours: u64) {
        let p = self.path(rel);
        let f = File::options().write(true).open(&p).unwrap();
        f.set_modified(SystemTime::now() - Duration::from_secs(hours * 3600)).unwrap();
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn hook_json(tool: &str) -> String {
    format!(r#"{{"tool_name":"{tool}","tool_input":{{}}}}"#)
}

fn bash_json(command: &str) -> String {
    format!(r#"{{"tool_name":"Bash","tool_input":{{"command":"{command}"}}}}"#)
}

fn code(out: &Output) -> Option<i32> {
    out.status.code()
}

// ---- cleanup and the pause file -------------------------------------------

#[test]
fn cleanup_below_floor_creates_pause_file_and_appends_log() {
    let p = Project::new("below-floor", 99999);
    // Pre-existing log line: the alert must append, not overwrite.
    fs::create_dir_all(p.path("scratch/logs")).unwrap();
    fs::write(p.monitor_log(), "earlier line\n").unwrap();

    let out = p.run(&["cleanup"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    assert!(p.pause_file().is_file(), "broadsword.paused should exist");
    let pause_body = fs::read_to_string(p.pause_file()).unwrap();
    assert!(pause_body.contains("reason:"), "pause file: {pause_body}");

    let stdout = text(&out.stdout);
    assert!(stdout.contains("PAUSED"), "stdout: {stdout}");
    assert!(text(&out.stderr).contains("ALERT:"), "alert should go to stderr");

    let log = fs::read_to_string(p.monitor_log()).unwrap();
    assert!(log.starts_with("earlier line\n"), "log was overwritten: {log}");
    assert!(log.contains("PAUSED"), "log: {log}");
}

#[test]
fn cleanup_above_floor_does_not_pause() {
    let p = Project::new("above-floor", 1);
    let out = p.run(&["cleanup"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    assert!(!p.pause_file().exists(), "no pause file expected above the floor");
    assert!(!p.monitor_log().exists() || !fs::read_to_string(p.monitor_log()).unwrap().contains("PAUSED"));
}

#[test]
fn cleanup_deletes_only_old_scratch_files() {
    let p = Project::new("expiry", 1);
    fs::create_dir_all(p.path("scratch/sub")).unwrap();
    fs::write(p.path("scratch/old.txt"), "old").unwrap();
    fs::write(p.path("scratch/sub/old-nested.txt"), "old").unwrap();
    fs::write(p.path("scratch/fresh.txt"), "fresh").unwrap();
    fs::write(p.path("scratch/.gitkeep"), "").unwrap();
    p.age("scratch/old.txt", 25);
    p.age("scratch/sub/old-nested.txt", 48);
    p.age("scratch/.gitkeep", 500);
    // fresh.txt keeps its just-written mtime.

    let out = p.run(&["cleanup"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    assert!(!p.path("scratch/old.txt").exists(), "old scratch file should be deleted");
    assert!(!p.path("scratch/sub/old-nested.txt").exists(), "old nested scratch file should be deleted");
    assert!(p.path("scratch/fresh.txt").exists(), "fresh scratch file must be kept");
    assert!(p.path("scratch/.gitkeep").exists(), ".gitkeep must be kept");
}

#[test]
fn cleanup_never_deletes_files_outside_scratch() {
    let p = Project::new("outside", 1);
    fs::create_dir_all(p.path("notes")).unwrap();
    fs::write(p.path("keep.txt"), "keep").unwrap();
    fs::write(p.path("notes/keep-too.txt"), "keep").unwrap();
    fs::write(p.path("scratch/old.txt"), "gone").unwrap();
    p.age("keep.txt", 1000);
    p.age("notes/keep-too.txt", 1000);
    p.age("scratch/old.txt", 1000);

    let out = p.run(&["cleanup"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    assert!(p.path("keep.txt").exists(), "file outside scratch was deleted");
    assert!(p.path("notes/keep-too.txt").exists(), "file in sibling folder was deleted");
    assert!(!p.path("scratch/old.txt").exists(), "control: old scratch file should be gone");
}

// ---- hook exit codes -------------------------------------------------------

#[test]
fn hook_write_is_blocked_while_paused() {
    let p = Project::new("hook-write-paused", 99999);
    p.pause_via_cleanup();
    let out = p.run(&["hook", "pre-tool"], &hook_json("Write"));
    assert_eq!(code(&out), Some(2), "stderr: {}", text(&out.stderr));
    assert!(text(&out.stderr).contains("PAUSED"));
}

#[test]
fn hook_read_glob_grep_allowed_while_paused() {
    let p = Project::new("hook-readonly-paused", 99999);
    p.pause_via_cleanup();
    for tool in ["Read", "Glob", "Grep"] {
        let out = p.run(&["hook", "pre-tool"], &hook_json(tool));
        assert_eq!(code(&out), Some(0), "{tool} should pass while paused: {}", text(&out.stderr));
    }
}

#[test]
fn hook_bash_broadsword_status_allowed_while_paused() {
    let p = Project::new("hook-bash-paused", 99999);
    p.pause_via_cleanup();
    let out = p.run(&["hook", "pre-tool"], &bash_json("broadsword status"));
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
}

#[test]
fn hook_bash_other_commands_blocked_while_paused() {
    let p = Project::new("hook-bash-blocked", 99999);
    p.pause_via_cleanup();
    let out = p.run(&["hook", "pre-tool"], &bash_json("cargo build"));
    assert_eq!(code(&out), Some(2), "stderr: {}", text(&out.stderr));
}

#[test]
fn hook_write_allowed_when_not_paused() {
    let p = Project::new("hook-write-free", 1);
    assert!(!p.pause_file().exists());
    let out = p.run(&["hook", "pre-tool"], &hook_json("Write"));
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
}

#[test]
fn hook_bypass_attempts_are_blocked_while_paused_and_a_real_exe_path_passes() {
    let p = Project::new("hook-bypass", 99999);
    p.pause_via_cleanup();
    for bad in [
        "rm x; broadsword status",
        "broadsword status && rm x",
        "broadsword status; rm x",
        "broadsword status | cat",
        "broadsword status & del x",
        "rm x\\nbroadsword status",
        "broadsword status\\nrm x",
        "echo broadsword status",
        "broadsword status $(rm x)",
    ] {
        let out = p.run(&["hook", "pre-tool"], &bash_json(bad));
        assert_eq!(code(&out), Some(2), "`{bad}` must be denied while paused; stderr: {}", text(&out.stderr));
        assert!(text(&out.stderr).contains("PAUSED"), "{}", text(&out.stderr));
    }
    for ok in ["C:/some/dir/build/bin/broadsword.exe status", "./build/bin/broadsword.exe cleanup", "broadsword resume --force"] {
        let out = p.run(&["hook", "pre-tool"], &bash_json(ok));
        assert_eq!(code(&out), Some(0), "`{ok}` must pass while paused; stderr: {}", text(&out.stderr));
    }
}

#[test]
fn hook_with_a_broken_config_fails_closed_and_says_why() {
    let p = Project::new("hook-broken-config", 1);
    // not paused, config present: everything passes (control)
    assert_eq!(code(&p.run(&["hook", "pre-tool"], &hook_json("Write"))), Some(0));
    for (what, toml) in [("an unreadable value", "root = \".\"\ndisk_floor_gb = banana\n"), ("missing keys", "root = \".\"\n")] {
        fs::write(p.path("broadsword.toml"), toml).unwrap();
        for tool in ["Write", "Edit", "WebFetch", "Agent", "mcp__broadsword__build", "mcp__broadsword__cleanup"] {
            let out = p.run(&["hook", "pre-tool"], &hook_json(tool));
            let se = text(&out.stderr);
            assert_eq!(code(&out), Some(2), "{tool} must be denied with {what}: {se}");
            assert!(se.contains("DENIED") && se.contains("broadsword.toml could not be loaded") && se.contains("Why:"), "{se}");
        }
        for cmd in ["rm x", "broadsword cleanup", "rm x; broadsword status"] {
            let out = p.run(&["hook", "pre-tool"], &bash_json(cmd));
            assert_eq!(code(&out), Some(2), "`{cmd}` must be denied with {what}: {}", text(&out.stderr));
        }
        for tool in ["Read", "Glob", "Grep", "mcp__broadsword__status"] {
            let out = p.run(&["hook", "pre-tool"], &hook_json(tool));
            assert_eq!(code(&out), Some(0), "read-only {tool} passes with {what}: {}", text(&out.stderr));
        }
        let out = p.run(&["hook", "pre-tool"], &bash_json("broadsword status"));
        assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
        let out = p.run(&["hook", "pre-tool"], "not json at all");
        assert_eq!(code(&out), Some(2), "unparseable hook input is denied: {}", text(&out.stderr));
    }
    // the same broken config is still an ordinary error (exit 1) for every other command
    assert_eq!(code(&p.run(&["status"], "")), Some(1));
}

#[test]
fn cleanup_says_clearly_when_cargo_sweep_is_missing() {
    let p = Project::new("sweep-note", 1);
    let installed = Command::new("cargo").args(["sweep", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    let out = p.run(&["cleanup"], "");
    let so = text(&out.stdout);
    assert_eq!(code(&out), Some(0), "{so}\n{}", text(&out.stderr));
    if installed {
        assert!(so.contains("cargo sweep ran") || so.contains("cargo sweep failed"), "{so}");
    } else {
        assert!(so.contains("WARNING: cargo-sweep is NOT installed") && so.contains("build folder was NOT swept"), "{so}");
    }
}

// ---- resume ----------------------------------------------------------------

#[test]
fn resume_refused_while_below_floor() {
    let p = Project::new("resume-refused", 99999);
    p.pause_via_cleanup();
    let out = p.run(&["resume"], "");
    assert_eq!(code(&out), Some(1), "stdout: {}", text(&out.stdout));
    assert!(text(&out.stderr).contains("refusing to resume"));
    assert!(p.pause_file().is_file(), "refused resume must keep the pause file");
    let hook = p.run(&["hook", "pre-tool"], &hook_json("Write"));
    assert_eq!(code(&hook), Some(2), "still paused after refused resume");
}

#[test]
fn resume_force_removes_pause_file() {
    let p = Project::new("resume-force", 99999);
    p.pause_via_cleanup();
    let out = p.run(&["resume", "--force"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    assert!(!p.pause_file().exists(), "--force should remove broadsword.paused");
    let hook = p.run(&["hook", "pre-tool"], &hook_json("Write"));
    assert_eq!(code(&hook), Some(0), "Write should pass after forced resume");
}

#[test]
fn resume_removes_pause_file_when_above_floor() {
    let p = Project::new("resume-above", 1);
    fs::write(p.pause_file(), "reason: test\n").unwrap();
    let out = p.run(&["resume"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    assert!(!p.pause_file().exists());
}

#[test]
fn resume_when_not_paused_is_a_no_op() {
    let p = Project::new("resume-noop", 1);
    let out = p.run(&["resume"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    assert!(text(&out.stdout).contains("not paused"));
}

// ---- worktrees and alert_command ---------------------------------------------

#[test]
fn hook_in_linked_worktree_honours_main_root_pause() {
    let p = Project::new("worktree", 99999);
    // Fake layout: main root has a .git directory; the worktree has a .git file pointing into it.
    let gitdir = p.path(".git/worktrees/wt");
    fs::create_dir_all(&gitdir).unwrap();
    let wt = p.path("wt/nested");
    fs::create_dir_all(&wt).unwrap();
    fs::copy(p.path("broadsword.toml"), wt.join("broadsword.toml")).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();

    p.pause_via_cleanup();
    assert!(!wt.join("broadsword.paused").exists());

    let run_in_wt = |stdin: &str| {
        let mut child = Command::new(BIN)
            .args(["hook", "pre-tool"])
            .current_dir(&wt)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
        child.wait_with_output().unwrap()
    };
    let out = run_in_wt(&hook_json("Write"));
    assert_eq!(code(&out), Some(2), "worktree hook must see the main root's pause: {}", text(&out.stderr));
    assert_eq!(code(&run_in_wt(&hook_json("Read"))), Some(0));
}

#[test]
fn alert_command_receives_full_message() {
    let p = Project::new("alertcmd", 99999);
    let toml = fs::read_to_string(p.path("broadsword.toml")).unwrap();
    fs::write(p.path("broadsword.toml"), toml.replace("alert_command = \"\"", "alert_command = \"echo ALERTCMD\"")).unwrap();
    let out = p.run(&["cleanup"], "");
    assert_eq!(code(&out), Some(0), "stderr: {}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("ALERTCMD") && stdout.contains("Broadsword PAUSED: free space") && stdout.contains("GB floor after cleanup"),
        "alert command output missing the full message: {stdout}"
    );
}
