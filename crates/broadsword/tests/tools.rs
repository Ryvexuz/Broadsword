//! End-to-end tests for `broadsword tools` and the exit codes the registry documents, run through
//! the real binary inside a throwaway root under the repo's `scratch/` folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");

struct Root {
    dir: PathBuf,
}

impl Root {
    fn new(name: &str, floor_gb: u64) -> Root {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("tools-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("projects")).unwrap();
        let toml = format!(
            "root = \".\"\ndisk_floor_gb = {floor_gb}\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\ndisk_check_every_seconds = 60\nlog_max_mb = 50\n\
             ram_warn_percent = 90\ncpu_warn_percent = 90\nalert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
             state_dir = \"state\"\ncheckpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 2\njob_disk_poll_seconds = 1\n"
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN).args(args).current_dir(&self.dir).stdin(Stdio::null()).output().unwrap()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

#[test]
fn tools_lists_every_tool() {
    let r = Root::new("list", 1);
    let o = r.run(&["tools"]);
    assert_eq!(code(&o), 0);
    let text = out(&o);
    for name in ["status", "cleanup", "watch", "resume", "projects", "build", "test", "check", "run", "recover", "abandon", "jobs", "tools"] {
        assert!(text.lines().any(|l| l.starts_with(&format!("{name} "))), "missing {name} in:\n{text}");
    }
    assert!(!text.lines().any(|l| l.starts_with("hook ")), "hook is not a tool");
}

#[test]
fn tools_one_in_full_and_json() {
    let r = Root::new("one", 1);
    let o = r.run(&["tools", "check"]);
    assert_eq!(code(&o), 0);
    let text = out(&o);
    assert!(text.contains("broadsword check <project>") && text.contains("exit codes:") && text.contains("project (string, required)"), "{text}");
    let o = r.run(&["tools", "--json"]);
    assert_eq!(code(&o), 0);
    let j = out(&o);
    assert!(j.trim_start().starts_with('[') && j.contains("\"inputSchema\"") && j.contains("\"name\": \"check\""));
    let o = r.run(&["tools", "check", "--json"]);
    assert_eq!(code(&o), 0);
    assert!(out(&o).trim_start().starts_with('{'));
}

#[test]
fn unknown_names_exit_2() {
    let r = Root::new("unknown", 1);
    let o = r.run(&["tools", "nope"]);
    assert_eq!(code(&o), 2);
    assert!(String::from_utf8_lossy(&o.stderr).contains("unknown tool `nope`"));
    assert_eq!(code(&r.run(&["check", "nope"])), 2);
    assert_eq!(code(&r.run(&["build", "nope"])), 2);
    assert_eq!(code(&r.run(&["run", "nope", "--", "x"])), 2);
}

#[test]
fn usage_lists_every_tool_and_exits_64() {
    let r = Root::new("usage", 1);
    let o = r.run(&["bogus"]);
    assert_eq!(code(&o), 64);
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    for w in ["status", "cleanup", "watch", "resume [--force]", "projects", "build <project>", "test <project>", "check <project>", "run <project> -- <command...>", "recover", "recover --abandon <id>", "jobs", "tools", "hook pre-tool"] {
        assert!(err.contains(w), "usage lacks `{w}`:\n{err}");
    }
}

#[test]
fn documented_exit_codes_through_the_binary() {
    // 1: abandon with an id that has no record; 0: jobs and projects with nothing recorded
    let r = Root::new("codes", 1);
    assert_eq!(code(&r.run(&["recover", "--abandon", "no-such-job"])), 1);
    assert_eq!(code(&r.run(&["jobs"])), 0);
    assert_eq!(code(&r.run(&["recover", "--list"])), 0);
    assert_eq!(code(&r.run(&["projects"])), 0);
    assert_eq!(code(&r.run(&["resume"])), 0); // not paused
    // 1: resume refused while paused with the floor out of reach; 3: a job refused while paused
    let r = Root::new("paused", 100_000_000);
    fs::write(r.dir.join("broadsword.paused"), "test pause\n").unwrap();
    let o = r.run(&["resume"]);
    assert_eq!(code(&o), 1, "{}", String::from_utf8_lossy(&o.stderr));
    let o = r.run(&["build", "broadsword"]);
    assert_eq!(code(&o), 3, "{}", String::from_utf8_lossy(&o.stderr));
}
