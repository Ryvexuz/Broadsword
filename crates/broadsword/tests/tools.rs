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
             state_dir = \"state\"\ninstall_dir = \"build/bin\"\nmcp_call_timeout_seconds = 1800\nmcp_output_max_kb = 1024\ncheckpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 2\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = 10\nreviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
max_agents = 50
"
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
    for w in ["status", "cleanup", "watch", "resume [--force]", "projects", "build <project> [--task <id>]", "test <project> [--task <id>]", "check <project> [--task <id>]", "run <project> [--task <id>] -- <command...>", "recover", "recover --abandon <id>", "jobs", "tools", "hook pre-tool"] {
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

// ---- every documented exit code that can be produced cheaply and safely ------------------
// Not repeated here: exit 4 (free disk drops mid-job: kill, pause) is covered by
// `disk_dropping_mid_job_kills_cargo_pauses_and_exits_4` in tests/jobs.rs, and `watch` never
// ends on its own, so it has no exit code to drive.

fn git(dir: &Path, args: &[&str]) {
    let o = Command::new("git").args(args).current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_INDEX_FILE").output().expect("git must be installed");
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
}

impl Root {
    fn project(&self, name: &str, lib_rs: &str, as_git: bool) {
        let p = self.dir.join("projects").join(name);
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(p.join("Cargo.toml"), format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n")).unwrap();
        fs::write(p.join("src/lib.rs"), lib_rs).unwrap();
        if as_git {
            git(&p, &["init", "-q"]);
            git(&p, &["config", "user.email", "t@t"]);
            git(&p, &["config", "user.name", "t"]);
            git(&p, &["config", "core.autocrlf", "false"]);
            git(&p, &["add", "-A"]);
            git(&p, &["commit", "-q", "-m", "init"]);
        }
    }
}

const ONE_TEST: &str = "#[cfg(test)] mod t { #[test] fn a() {} }\n";

#[test]
fn status_exits_0() {
    let r = Root::new("status", 1);
    let o = r.run(&["status"]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    assert!(out(&o).contains("paused: no"));
}

#[test]
fn resume_exits_0_when_the_pause_is_lifted() {
    let r = Root::new("resume0", 1);
    fs::write(r.dir.join("broadsword.paused"), "test pause\n").unwrap();
    let o = r.run(&["resume"]);
    assert_eq!(code(&o), 0, "{}", String::from_utf8_lossy(&o.stderr));
    assert!(!r.dir.join("broadsword.paused").exists());
}

#[test]
fn build_test_check_exit_0_1_2_and_3() {
    let r = Root::new("jobs", 1);
    r.project("ok", ONE_TEST, false);
    r.project("bad", "fn oops( {\n", false);
    for k in ["build", "test", "check"] {
        let o = r.run(&[k, "ok"]);
        assert_eq!(code(&o), 0, "{k} ok: {}{}", out(&o), String::from_utf8_lossy(&o.stderr));
        assert_eq!(code(&r.run(&[k, "bad"])), 1, "{k} on a project that does not compile");
        assert_eq!(code(&r.run(&[k, "nope"])), 2, "{k} on an unknown project");
    }
    fs::write(r.dir.join("broadsword.paused"), "test pause\n").unwrap();
    for k in ["build", "test", "check"] {
        assert_eq!(code(&r.run(&[k, "ok"])), 3, "{k} while paused");
    }
}

#[test]
fn run_passes_the_commands_exit_code_and_summary_line_tells_it_from_a_refusal() {
    let r = Root::new("run", 1);
    r.project("g", ONE_TEST, true);
    r.project("nogit", ONE_TEST, false);
    let summary = "--- summary ---";
    let o = r.run(&["run", "g", "--", "exit 0"]);
    assert_eq!(code(&o), 0, "{}{}", out(&o), String::from_utf8_lossy(&o.stderr));
    assert!(out(&o).contains(summary) && out(&o).contains("run: ok"));
    // the command's own 3 (and 7): a summary line, `run: FAILED (exit N)`
    for n in ["3", "7"] {
        let o = r.run(&["run", "g", "--", &format!("exit {n}")]);
        assert_eq!(code(&o), n.parse::<i32>().unwrap());
        assert!(out(&o).contains(summary) && out(&o).contains(&format!("run: FAILED (exit {n})")), "{}", out(&o));
    }
    // Broadsword's own 2 and 3: no summary line, a `broadsword:` reason on stderr
    let o = r.run(&["run", "nope", "--", "exit 0"]);
    assert_eq!(code(&o), 2);
    assert!(!out(&o).contains(summary));
    let o = r.run(&["run", "nogit", "--", "exit 0"]);
    assert_eq!(code(&o), 3);
    assert!(!out(&o).contains(summary) && String::from_utf8_lossy(&o.stderr).contains("broadsword:"));
    fs::write(r.dir.join("broadsword.paused"), "test pause\n").unwrap();
    let o = r.run(&["run", "g", "--", "exit 0"]);
    assert_eq!(code(&o), 3);
    assert!(!out(&o).contains(summary));
}

#[test]
fn a_command_cannot_fake_the_verdict() {
    let r = Root::new("fake", 1);
    r.project("g", ONE_TEST, true);
    let line = if cfg!(windows) { "echo --- summary --- & echo run: ok & exit 3" } else { "echo '--- summary ---'; echo 'run: ok'; exit 3" };
    let o = r.run(&["run", "g", "--", line]);
    assert_eq!(code(&o), 3);
    let so = out(&o);
    assert_eq!(so.matches("run: ok").count(), 1, "the command's own output is printed too: {so}");
    let last = so.lines().rev().find(|l| l.starts_with("run:")).unwrap();
    assert_eq!(last.trim(), "run: FAILED (exit 3)", "{so}");
    assert!(so.rfind("--- summary ---").unwrap() < so.rfind("run: FAILED").unwrap());
}

#[test]
fn recover_and_abandon_exit_codes() {
    let r = Root::new("recover", 1);
    r.project("g", ONE_TEST, true);
    assert_eq!(code(&r.run(&["recover"])), 0); // nothing to do
    // a dead process id
    let mut c = Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap();
    let dead = c.id().to_string();
    c.wait().unwrap();
    let jobs = r.dir.join("state/jobs");
    fs::create_dir_all(&jobs).unwrap();
    let rec = |id: &str, kind: &str, ckpt: &str| {
        fs::write(jobs.join(format!("{id}.rec")), format!("id = {id}\nproject = g\nkind = {kind}\nstatus = running\npid = {dead}\ncheckpoint = {ckpt}\n")).unwrap();
    };
    // abandon: 0 for a dead running job
    rec("20260101-000001-g-check", "check", "none");
    assert_eq!(code(&r.run(&["recover", "--abandon", "20260101-000001-g-check"])), 0);
    // recover: 1 when a run job cannot be rolled back
    rec("20260101-000002-g-run", "run", "refs/broadsword/checkpoints/does-not-exist");
    let o = r.run(&["recover"]);
    assert_eq!(code(&o), 1, "{}", String::from_utf8_lossy(&o.stderr));
}
