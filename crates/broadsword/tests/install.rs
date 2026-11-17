//! End-to-end tests for `broadsword install`: the real binary installing itself into throwaway roots under scratch/.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");

struct Root {
    dir: PathBuf,
}

impl Root {
    fn new(name: &str, install_dir: &str) -> Root {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("install-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        let toml = format!(
            "root = \".\"\ndisk_floor_gb = 1\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\ndisk_check_every_seconds = 60\nlog_max_mb = 50\n\
             ram_warn_percent = 90\ncpu_warn_percent = 90\nalert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
             state_dir = \"state\"\ninstall_dir = \"{install_dir}\"\nmcp_call_timeout_seconds = 600\nmcp_output_max_kb = 1024\n\
             checkpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 0\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\n"
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn bin_dir(&self) -> PathBuf {
        self.dir.join("build/bin")
    }

    fn target(&self) -> PathBuf {
        self.bin_dir().join(format!("broadsword{}", std::env::consts::EXE_SUFFIX))
    }

    fn run(&self, exe: &str, args: &[&str]) -> Output {
        Command::new(exe).args(args).current_dir(&self.dir).stdin(Stdio::null()).output().unwrap()
    }

    /// Names of the leftover copies in the install folder.
    fn old_copies(&self) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(self.bin_dir())
            .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| n.starts_with("broadsword.old-") || n.starts_with("broadsword.new-")).collect())
            .unwrap_or_default();
        v.sort();
        v
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn install_copies_the_running_binary_into_install_dir() {
    let r = Root::new("copy", "build/bin");
    assert!(!r.bin_dir().exists());
    let o = r.run(BIN, &["install"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), String::from_utf8_lossy(&o.stderr));
    assert!(stdout(&o).starts_with("installed: "), "{}", stdout(&o));
    assert_eq!(fs::read(r.target()).unwrap(), fs::read(BIN).unwrap(), "the installed file is a byte-for-byte copy");
    assert!(r.old_copies().is_empty(), "no temp file left behind: {:?}", r.old_copies());
    // the installed copy works, and installing from it is a no-op
    let o = r.run(r.target().to_str().unwrap(), &["projects"]);
    assert_eq!(o.status.code(), Some(0));
    let o = r.run(r.target().to_str().unwrap(), &["install"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(stdout(&o).starts_with("already installed"), "{}", stdout(&o));
    // installing again over an idle copy replaces it in place
    let o = r.run(BIN, &["install"]);
    assert_eq!(o.status.code(), Some(0));
    assert!(!stdout(&o).contains("renamed"), "{}", stdout(&o));
    assert!(r.old_copies().is_empty());
}

#[test]
fn a_running_copy_is_renamed_aside_and_cleaned_up_by_a_later_install() {
    let r = Root::new("aside", "build/bin");
    assert_eq!(r.run(BIN, &["install"]).status.code(), Some(0));
    // Run the installed copy as an MCP server: it sits waiting on stdin and holds its own exe open.
    let mut holder: Child = Command::new(r.target()).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    std::thread::sleep(Duration::from_millis(500));
    assert!(holder.try_wait().unwrap().is_none());

    let o = r.run(BIN, &["install"]);
    assert_eq!(o.status.code(), Some(0), "{}{}", stdout(&o), String::from_utf8_lossy(&o.stderr));
    assert_eq!(fs::read(r.target()).unwrap(), fs::read(BIN).unwrap(), "the new copy is in place");
    if cfg!(windows) {
        let old = r.old_copies();
        assert_eq!(old.len(), 1, "{old:?}");
        assert!(old[0].starts_with("broadsword.old-") && old[0].ends_with(".exe"), "{old:?}");
        assert!(stdout(&o).contains("renamed to broadsword.old-"), "{}", stdout(&o));
    }
    assert!(holder.try_wait().unwrap().is_none(), "the running copy was not disturbed");

    // While the old copy still runs, another install must leave it alone and say so.
    if cfg!(windows) {
        let o = r.run(BIN, &["install"]);
        assert_eq!(o.status.code(), Some(0));
        assert!(stdout(&o).contains("still in use"), "{}", stdout(&o));
        assert_eq!(r.old_copies().len(), 1);
    }

    // Stop the old copy; the next install deletes it.
    holder.kill().unwrap();
    holder.wait().unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut last = String::new();
    while Instant::now() < deadline {
        let o = r.run(BIN, &["install"]);
        assert_eq!(o.status.code(), Some(0));
        last = stdout(&o);
        if r.old_copies().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(r.old_copies().is_empty(), "old copy not cleaned up: {:?}\n{last}", r.old_copies());
    assert_eq!(fs::read(r.target()).unwrap(), fs::read(BIN).unwrap());
}

#[test]
fn install_dir_outside_root_is_rejected() {
    let r = Root::new("outside", "../escape");
    let o = r.run(BIN, &["install"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("install_dir"), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!r.dir.join("../escape").exists());
}

#[test]
fn install_through_mcp_runs_as_a_child_and_installs_the_server_binary() {
    use std::io::{BufRead, BufReader, Write};
    let r = Root::new("mcp", "build/bin");
    let mut child = Command::new(BIN).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"install","arguments":{{}}}}}}"#).unwrap();
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    assert!(line.contains(r#""isError":false"#) && line.contains("installed: "), "{line}");
    assert!(r.target().is_file());
    drop(stdin);
    child.wait().unwrap();
}
