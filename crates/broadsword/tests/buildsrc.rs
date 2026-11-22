//! End-to-end tests for `--task` on build/test/check/run and for the shared-build-folder rules (state/build-sources
//! notes, `cargo clean -p`, state/build.lock). Job tests make a throwaway Broadsword root under the repo's `scratch/`
//! with a real Git repo and a tiny cargo project under `projects/`, and run the real binary there. Never the real repo,
//! its worktrees or `projects/crash-sim`.
//!
//! The project `proj` holds the package `markprobe`: a binary that prints a marker string from its library, and an
//! integration test that runs that binary and asserts it prints the marker of the checkout being tested. A stale binary
//! (another checkout's) therefore makes `test` fail, which is exactly the hazard.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

mod support;
use support::bin;

#[path = "../src/lock.rs"]
mod priority_lock;

// The product's own process-identity code, to plant an owner record naming a live process exactly (start time included).
#[allow(dead_code)]
#[path = "../src/sys.rs"]
mod sys;
const SYNC_GATE: &str = "BROADSWORD_INTERNAL_TEST_BUILD_SYNC_GATE_FILE";
/// scrub::RUN_LOCK_MARKER: a `run` sets it on its command; `Root::cmd` removes any inherited one so every test starts clean.
const RUN_MARKER: &str = "BROADSWORD_RUN_HOLDS_BUILD_LOCK";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

// These races exercise the production lock entry point directly, in separate processes. Readiness is registration
// (a marker exists under arbitration), not a sleep that guesses when a waiter started.
struct PriorityRoot {
    dir: PathBuf,
}

impl PriorityRoot {
    fn new(tag: &str) -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!(
                "bs-priority-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::SeqCst)
            ));
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        fs::create_dir(&dir).unwrap();
        Self { dir }
    }

    fn path(&self) -> PathBuf {
        self.dir.join("build.lock")
    }

    fn registered(&self, count: usize) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let entries = match fs::read_dir(self.dir.join("build.lock.waiters")) {
                Ok(rd) => rd.collect::<Result<Vec<_>, _>>().unwrap().len(),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                Err(e) => panic!("cannot read waiter directory: {e}"),
            };
            if entries == count {
                return;
            }
            assert!(
                Instant::now() < end,
                "registration: expected {count} markers, got {entries}"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn spawn(&self, name: &str, human: bool, wait_ms: u64) -> PriorityChild {
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "priority_waiter_child", "--nocapture"])
            .env("BROADSWORD_TEST_PRIORITY_DIR", &self.dir)
            .env("BROADSWORD_TEST_PRIORITY_NAME", name)
            .env(
                "BROADSWORD_TEST_PRIORITY_CLASS",
                if human { "human" } else { "agent" },
            )
            .env("BROADSWORD_TEST_PRIORITY_WAIT_MS", wait_ms.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(self.dir.join(format!("{name}.err"))).unwrap())
            .spawn()
            .unwrap();
        PriorityChild {
            child,
            dir: self.dir.clone(),
            name: name.into(),
        }
    }
}

impl Drop for PriorityRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct PriorityChild {
    child: Child,
    dir: PathBuf,
    name: String,
}

impl PriorityChild {
    fn taken(&self) {
        assert!(
            wait_for(&self.dir.join(format!("{}.taken", self.name)), "taken", 10),
            "{} never acquired: {}",
            self.name,
            fs::read_to_string(self.dir.join(format!("{}.err", self.name))).unwrap()
        );
    }

    fn release(&mut self) {
        fs::write(self.dir.join(format!("{}.release", self.name)), "release").unwrap();
        self.finish();
    }

    fn finish(&mut self) {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(
                    status.success(),
                    "{} failed: {}",
                    self.name,
                    fs::read_to_string(self.dir.join(format!("{}.err", self.name))).unwrap()
                );
                return;
            }
            assert!(Instant::now() < end, "{} did not finish", self.name);
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for PriorityChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Subprocess helper: no real job or environment-based job classification is involved.
#[test]
fn priority_waiter_child() {
    let Some(dir) = std::env::var_os("BROADSWORD_TEST_PRIORITY_DIR") else {
        return;
    };
    let dir = PathBuf::from(dir);
    let name = std::env::var("BROADSWORD_TEST_PRIORITY_NAME").unwrap();
    let priority = match std::env::var("BROADSWORD_TEST_PRIORITY_CLASS").unwrap().as_str() {
        "human" => priority_lock::Priority::Human,
        "agent" => priority_lock::Priority::Agent,
        other => panic!("invalid test priority {other}"),
    };
    let wait = Duration::from_millis(
        std::env::var("BROADSWORD_TEST_PRIORITY_WAIT_MS")
            .unwrap()
            .parse()
            .unwrap(),
    );
    let started = Instant::now();
    let lock =
        priority_lock::acquire_priority(&dir.join("build.lock"), wait, priority, &|_| "busy".into());
    match lock {
        Ok(_lock) => {
            fs::write(dir.join(format!("{name}.taken")), "taken").unwrap();
            assert!(
                wait_for(&dir.join(format!("{name}.release")), "release", 10),
                "helper release gate timed out"
            );
        }
        Err(e) => {
            assert_eq!(e, "busy", "lock errors must not masquerade as timeouts");
            assert!(
                started.elapsed() >= wait,
                "refused earlier than its own wait clock"
            );
            fs::write(dir.join(format!("{name}.timeout")), "timeout").unwrap();
        }
    }
}

fn priority_order(tag: &str, first_human: bool, second_human: bool, second_wins: bool) {
    let r = PriorityRoot::new(tag);
    let holder = priority_lock::acquire(&r.path(), Duration::ZERO, &|_| "busy".into()).unwrap();
    let mut first = r.spawn("first", first_human, 20_000);
    r.registered(1);
    let mut second = r.spawn("second", second_human, 20_000);
    r.registered(2);
    assert!(
        !r.dir.join("first.taken").exists() && !r.dir.join("second.taken").exists(),
        "the holder was pre-empted"
    );
    drop(holder);
    let (winner, loser) = if second_wins {
        (&mut second, &mut first)
    } else {
        (&mut first, &mut second)
    };
    winner.taken();
    assert!(
        !r.dir.join(format!("{}.taken", loser.name)).exists(),
        "two holders or wrong selection order"
    );
    winner.release();
    loser.taken();
    loser.release();
}

#[test]
fn human_waiter_overtakes_an_earlier_agent_without_preempting_holder() {
    priority_order("human-first", false, true, true);
}

#[test]
fn two_agent_waiters_keep_arrival_order() {
    priority_order("agents-fifo", false, false, false);
}

#[test]
fn two_human_waiters_keep_arrival_order() {
    priority_order("humans-fifo", true, true, false);
}

#[test]
fn humans_cut_in_without_reordering_the_agents_they_pass() {
    let r = PriorityRoot::new("mixed-fifo");
    let holder = priority_lock::acquire(&r.path(), Duration::ZERO, &|_| "busy".into()).unwrap();
    let mut a1 = r.spawn("a1", false, 20_000);
    r.registered(1);
    let mut u1 = r.spawn("u1", true, 20_000);
    r.registered(2);
    let mut a2 = r.spawn("a2", false, 20_000);
    r.registered(3);
    let mut u2 = r.spawn("u2", true, 20_000);
    r.registered(4);
    drop(holder);
    u1.taken();
    for name in ["a1", "a2", "u2"] {
        assert!(!r.dir.join(format!("{name}.taken")).exists(), "{name} jumped u1");
    }
    u1.release();
    u2.taken();
    for name in ["a1", "a2"] {
        assert!(!r.dir.join(format!("{name}.taken")).exists(), "{name} jumped u2");
    }
    u2.release();
    a1.taken();
    assert!(!r.dir.join("a2.taken").exists(), "a2 jumped a1 after humans cut in");
    a1.release();
    a2.taken();
    a2.release();
}

#[test]
fn stale_human_marker_from_a_dead_process_does_not_delay_an_agent() {
    let r = PriorityRoot::new("dead-human");
    let holder = priority_lock::acquire(&r.path(), Duration::ZERO, &|_| "busy".into()).unwrap();
    let mut human = r.spawn("human", true, 20_000);
    r.registered(1);
    human.child.kill().unwrap();
    human.child.wait().unwrap();
    r.registered(1); // The dead process left its marker, but no OS lock on it.
    drop(holder);
    drop(
        priority_lock::acquire_priority(
            &r.path(),
            Duration::ZERO,
            priority_lock::Priority::Agent,
            &|_| "busy".into(),
        )
        .unwrap(),
    );
}

#[test]
fn human_cut_in_does_not_refuse_an_agent_before_its_own_wait_expires() {
    let r = PriorityRoot::new("agent-clock");
    let holder = priority_lock::acquire(&r.path(), Duration::ZERO, &|_| "busy".into()).unwrap();
    let mut agent = r.spawn("agent", false, 2_000);
    r.registered(1);
    let mut human = r.spawn("human", true, 20_000);
    r.registered(2);
    drop(holder);
    human.taken();
    // The helper checks elapsed >= its configured wait on the timeout path. It must finish even though the human
    // still holds the lock, rather than treating priority as an immediate refusal or waiting for that human to finish.
    agent.finish();
    assert!(wait_for(&r.dir.join("agent.timeout"), "timeout", 1));
    assert!(!r.dir.join("agent.taken").exists());
    human.release();
}

#[test]
fn zero_wait_probe_cannot_bypass_a_registered_human() {
    let r = PriorityRoot::new("probe");
    let holder = priority_lock::acquire(&r.path(), Duration::ZERO, &|_| "busy".into()).unwrap();
    let mut human = r.spawn("human", true, 20_000);
    r.registered(1);
    // Freeze selection, then free the resource. A raw try_lock would now incorrectly jump the registered human.
    let gate = priority_lock::acquire(
        &r.dir.join("build.lock.queue"),
        Duration::from_secs(10),
        &|_| "busy".into(),
    )
    .unwrap();
    drop(holder);
    assert_eq!(
        priority_lock::acquire_priority(
            &r.path(),
            Duration::ZERO,
            priority_lock::Priority::Agent,
            &|_| "busy".into(),
        )
        .err()
        .as_deref(),
        Some("busy")
    );
    drop(gate);
    human.taken();
    human.release();
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@localhost")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@localhost")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?} in {}: {}", dir.display(), String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

struct Root {
    dir: PathBuf,
}

impl Root {
    fn new(name: &str, build_lock_wait: u64) -> Root {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bsrc-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        let dir = fs::canonicalize(&dir).unwrap();
        let dir = PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"));
        // proj: a Git repo with the markprobe package. other: a second project, only for wrong-project refusals.
        let p = dir.join("projects/proj");
        fs::create_dir_all(p.join("src")).unwrap();
        fs::create_dir_all(p.join("tests")).unwrap();
        fs::write(p.join("Cargo.toml"), "[package]\nname = \"markprobe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        fs::write(p.join(".gitignore"), "target\n").unwrap();
        fs::write(p.join("src/main.rs"), "fn main() { print!(\"{}\", markprobe::MARKER); }\n").unwrap();
        fs::write(
            p.join("tests/marker.rs"),
            // cargo runs a test with the package root of the checkout being tested as its working directory, so reading
            // src/lib.rs at run time compares the binary with THIS checkout's source even if the test executable itself
            // is a stale one built from another checkout
            "#[test]\nfn the_binary_is_this_checkouts_binary() {\n    let o = std::process::Command::new(std::env::var(\"CARGO_BIN_EXE_markprobe\").unwrap_or_else(|_| env!(\"CARGO_BIN_EXE_markprobe\").to_string())).output().unwrap();\n    let printed = String::from_utf8_lossy(&o.stdout).into_owned();\n    let source = std::fs::read_to_string(\"src/lib.rs\").unwrap();\n    assert!(source.contains(&format!(\"\\\"{printed}\\\"\")), \"binary prints {printed}, this checkout says {source}\");\n}\n",
        )
        .unwrap();
        set_marker(&p, "MARK-main");
        git(&p, &["init", "-q", "-b", "main"]);
        git(&p, &["config", "core.autocrlf", "false"]);
        git(&p, &["add", "."]);
        git(&p, &["commit", "-q", "-m", "first"]);
        let o = dir.join("projects/other");
        fs::create_dir_all(o.join("src")).unwrap();
        fs::write(o.join("Cargo.toml"), "[package]\nname = \"other\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        fs::write(o.join("src/lib.rs"), "").unwrap();
        let toml = support::config(&[("build_lock_wait_seconds", &build_lock_wait.to_string())]);
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(bin());
        c.args(args).current_dir(&self.dir).env_remove(SYNC_GATE).env_remove(RUN_MARKER).stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Out {
        let o = self.cmd(args).output().unwrap();
        Out { code: o.status.code().unwrap(), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
    }

    fn ok(&self, args: &[&str]) -> Out {
        let o = self.run(args);
        assert_eq!(o.code, 0, "{args:?}\nstdout:\n{}\nstderr:\n{}", o.stdout, o.stderr);
        o
    }

    /// A task in `proj` with its own worktree whose marker is `marker`. Returns the id.
    fn task(&self, claim: &str, marker: &str) -> String {
        let id = self.ok(&["task", "add", "proj", "--title", "t", "--claim", claim]).stdout.trim().to_string();
        self.ok(&["task", "start", &id, "--owner", "coder", "--worktree"]);
        set_marker(&self.worktree(&id), marker);
        id
    }

    fn worktree(&self, id: &str) -> PathBuf {
        self.dir.join("worktrees/proj").join(id)
    }

    /// What the binary in the shared build folder prints.
    fn probe(&self) -> String {
        let exe = self.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
        let o = Command::new(&exe).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", exe.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    }

    fn note(&self) -> Option<String> {
        fs::read_to_string(self.dir.join("state/build-sources/markprobe")).ok().map(|s| s.trim().to_string())
    }

    /// The note of the binary target `markprobe` (state/build-sources/bin.markprobe).
    fn bin_note(&self) -> Option<String> {
        fs::read_to_string(self.dir.join("state/build-sources/bin.markprobe")).ok().map(|s| s.trim().to_string())
    }

    fn job_records(&self) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = fs::read_dir(self.dir.join("state/jobs")).map(|rd| rd.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        v.sort();
        v
    }

    fn scratch(&self, name: &str) -> PathBuf {
        self.dir.join("scratch").join(name)
    }

    /// Start a job in the background with its stderr in a scratch file and the sync gate set to `gate` (a scratch path).
    fn spawn_gated(&self, args: &[&str], gate: &Path, err: &Path) -> Child {
        self.cmd(args).env(SYNC_GATE, gate).stdout(Stdio::null()).stderr(fs::File::create(err).unwrap()).spawn().unwrap()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn set_marker(project: &Path, marker: &str) {
    fs::write(project.join("src/lib.rs"), format!("pub const MARKER: &str = \"{marker}\";\n")).unwrap();
}

fn wait_for(file: &Path, text: &str, secs: u64) -> bool {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if fs::read_to_string(file).is_ok_and(|s| s.contains(text)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn finish(mut c: Child, secs: u64) -> i32 {
    let end = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(s) = c.try_wait().unwrap() {
            return s.code().unwrap();
        }
        assert!(Instant::now() < end, "the job did not finish within {secs} s");
        std::thread::sleep(Duration::from_millis(50));
    }
}

// ---- --task builds the worktree's code; main afterwards gives main's ------------------------

#[test]
fn task_builds_the_worktrees_code_and_main_afterwards_is_not_stale() {
    let r = Root::new("marker", 120);
    let t1 = r.task("a", "MARK-t1");

    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("source: projects/proj\n"), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-main");
    assert_eq!(r.note().as_deref(), Some("projects/proj"));

    // the worktree's code, shown by its marker; the clean is reported and the record names the folder built
    let o = r.ok(&["build", "proj", "--task", &t1]);
    let source = format!("worktrees/proj/{t1}");
    assert!(o.stdout.contains(&format!("source: {source} (task {t1})")), "{}", o.stdout);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains("last built from projects/proj"), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-t1", "the shared folder holds the worktree's binary");
    assert_eq!(r.note().as_deref(), Some(source.as_str()));
    let rec = fs::read_to_string(r.job_records().last().unwrap()).unwrap();
    assert!(rec.contains(&format!("source = {source}")) && rec.contains(&format!("task = {t1}")), "{rec}");

    // the stale-binary case: main again, cargo would say `Finished` for main's still-fresh build and leave t1's binary
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains(&format!("last built from {source}")), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-main", "building main after a worktree must give main's binary, not the worktree's");
    assert_eq!(r.note().as_deref(), Some("projects/proj"));

    // and the tests, which run that binary, agree in both directions
    let o = r.ok(&["check", "proj", "--task", &t1]);
    assert!(o.stdout.contains("test: ok"), "{}", o.stdout);
    let o = r.ok(&["check", "proj"]);
    assert!(o.stdout.contains("test: ok"), "{}", o.stdout);
    // building the same source twice in a row cleans nothing
    let o = r.ok(&["build", "proj"]);
    assert!(!o.stdout.contains("cleaned"), "{}", o.stdout);
}

/// Finding 39: a worktree that renames the package but keeps the bin name builds over the same exe. The package note of
/// `markprobe` still says main built it, so only a note per bin target can tell main's next build (and `install`, which reads
/// the same notes: unit test `install_refuses_when_another_package_built_the_same_bin_name`) that the exe in the shared
/// folder is not main's. On the code before the fix (46d6bbb) main's next build cleaned nothing (this test failed at the
/// `cleaned` assertion; cargo itself re-linked main's exe there, so the probe was already right) and no bin note existed.
#[test]
fn a_renamed_package_with_the_same_bin_name_does_not_leave_its_exe_vouched_for_main() {
    let r = Root::new("binname", 120);
    let t1 = r.task("a", "MARK-t1");
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("source: projects/proj\n"), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-main");

    // the worktree: package `markprobe-wt`, but the same library and binary names
    let wt = r.worktree(&t1);
    let manifest = "[package]\nname = \"markprobe-wt\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[lib]\nname = \"markprobe\"\n\n[[bin]]\nname = \"markprobe\"\npath = \"src/main.rs\"\n\n[workspace]\n";
    fs::write(wt.join("Cargo.toml"), manifest).unwrap();
    let source = format!("worktrees/proj/{t1}");
    let o = r.ok(&["build", "proj", "--task", &t1]);
    assert!(o.stdout.contains(&format!("source: {source} (task {t1})")), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-t1", "the worktree's package overwrote the exe of the same name");
    assert_eq!(r.note().as_deref(), Some("projects/proj"), "the package note is still main's: this is the hole");

    // main again: its package note is fresh, but the exe is not its own, so it is cleaned and rebuilt
    let o = r.ok(&["build", "proj"]);
    assert_eq!(r.probe(), "MARK-main", "main's build must give main's binary, not the renamed package's\n{}", o.stdout);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains(&format!("last built from {source}")), "{}", o.stdout);
    assert_eq!(r.bin_note().as_deref(), Some("projects/proj"), "the bin note names main's folder again");
    // and a second build of the same source cleans nothing
    let o = r.ok(&["build", "proj"]);
    assert!(!o.stdout.contains("cleaned"), "{}", o.stdout);

    // the worktree once more: its build leaves a bin note that names the worktree, which is what `install` reads
    r.ok(&["build", "proj", "--task", &t1]);
    assert_eq!(r.bin_note().as_deref(), Some(source.as_str()), "the bin note names the folder that built the exe");
}

/// t-20261120-191700 (Lazlo's retest, 2026-11-20 19:15Z): he built `crash-sim` from projects/crash-sim, and one second later the
/// queue's perf gate for a worktree entry cleaned `crash-sim` (its note named the main checkout) and built only its bench
/// example, so build/release/crash-sim.exe was gone. Here: main builds the bin (debug through `build`, release through `run`, as
/// Lazlo did), a worktree builds ONLY an example (with the clean). Main's exe is still available, still main's build: kept in
/// `main.bins/proj/<profile>` with a note naming main (on the code before the task the kept copy did not exist). Cargo's own
/// folder holds only what the worktree job built: no `markprobe` exe at all (round 8, review q-20261121-011535: copying main's
/// exe back there mislabelled it as the worktree's build). Then the worktree builds the bin itself: cargo's place holds the
/// worktree's exe (its bin note says so), and the kept copy is still main's.
#[test]
fn a_worktree_that_builds_only_an_example_leaves_mains_binary_kept_and_none_in_cargos_folder() {
    let r = Root::new("keepmain", 120);
    let proj = r.dir.join("projects/proj");
    fs::create_dir_all(proj.join("examples")).unwrap();
    fs::write(proj.join("examples/only.rs"), "fn main() { print!(\"example {}\", markprobe::MARKER); }\n").unwrap();
    git(&proj, &["add", "."]);
    git(&proj, &["commit", "-q", "-m", "example"]);
    let t1 = r.task("a", "MARK-t1");
    let t2 = r.task("b", "MARK-t2");
    let exe = |profile: &str| r.dir.join("build").join(profile).join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let kept = |profile: &str| r.dir.join("build/main.bins/proj").join(profile).join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let prints = |p: &Path| -> String {
        let o = Command::new(p).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", p.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let kept_note = |profile: &str| fs::read_to_string(format!("{}.source", kept(profile).display())).ok().map(|s| s.trim().to_string());

    // A = the main checkout builds the bin, in both profiles
    r.ok(&["build", "proj"]);
    r.ok(&["run", "proj", "--", "cargo", "build", "--release", "--offline"]);
    assert_eq!(prints(&exe("debug")), "MARK-main");
    assert_eq!(prints(&exe("release")), "MARK-main");

    // B = a worktree builds only an example of the same package (release, like the perf gate's bench); its job cleans first
    let o = r.ok(&["run", "proj", "--task", &t1, "--", "cargo", "build", "--release", "--offline", "--example", "only"]);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains("last built from projects/proj"), "the race needs the clean:\n{}", o.stdout);
    for profile in ["debug", "release"] {
        assert!(!exe(profile).exists(), "cargo's {profile} folder holds only what this job built, never main's exe under the worktree's note:\n{}", o.stdout);
        assert_eq!(prints(&kept(profile)), "MARK-main", "main's {profile} binary is still available, still main's, in main.bins:\n{}", o.stdout);
        // each kept copy names the step that built it: debug by `build`, release by the `run` (round 9: the release `run` did
        // not write the debug exe, so it left that copy and its note alone)
        let built_by = if profile == "debug" { "projects/proj" } else { "run:projects/proj" };
        assert_eq!(kept_note(profile).as_deref(), Some(built_by), "the kept {profile} copy names the main step that built it");
    }
    assert!(!o.stdout.contains("restored"), "nothing is put back into cargo's folder:\n{}", o.stdout);
    let example = r.dir.join("build/release/examples").join(format!("only{}", std::env::consts::EXE_SUFFIX));
    assert_eq!(prints(&example), "example MARK-t1", "the job's own cargo step built the worktree's example");
    assert_eq!(r.bin_note().as_deref(), Some(format!("run:worktrees/proj/{t1}").as_str()));

    // a worktree whose cargo step needs the bin rebuilds it from its own source (the clean removed cargo's fingerprint)
    let o = r.ok(&["check", "proj", "--task", &t2]);
    assert!(o.stdout.contains("test: ok"), "{}", o.stdout);
    assert_eq!(prints(&exe("debug")), "MARK-t2", "the worktree's own build gives its own binary");
    assert_eq!(r.bin_note().as_deref(), Some(format!("worktrees/proj/{t2}").as_str()), "and its bin note says so");
    assert_eq!(prints(&kept("debug")), "MARK-main", "the kept copy is never replaced by a worktree's exe");
    assert_eq!(kept_note("debug").as_deref(), Some("projects/proj"));

    // the next job of another folder that cleans leaves no exe in cargo's folder, and main's kept copy as it was
    let o = r.ok(&["run", "proj", "--task", &t1, "--", "cargo", "build", "--offline", "--example", "only"]);
    assert!(o.stdout.contains("cleaned `markprobe`"), "{}", o.stdout);
    assert!(!exe("debug").exists(), "{}", o.stdout);
    assert_eq!(prints(&kept("debug")), "MARK-main");

    // and main's own build afterwards gives main's binary in both places
    r.ok(&["build", "proj"]);
    assert_eq!(prints(&exe("debug")), "MARK-main");
    assert_eq!(prints(&kept("debug")), "MARK-main");
    assert_eq!(r.bin_note().as_deref(), Some("projects/proj"));
}

/// Round 8 (review q-20261121-011535, the blocking finding): a worktree job that cleans the package and then runs nothing that
/// builds it must leave NO binary for that bin in cargo's folder (a test or script there that runs the exe by path fails loudly,
/// as before this task), and main's copy in main.bins untouched. On round 7's code (ce4ca63) `restore_main_bins` copied main's
/// exe back into cargo's folder under the worktree's bin note, so the first assertion failed.
#[test]
fn a_worktree_job_that_cleans_and_builds_nothing_leaves_no_binary_in_cargos_folder() {
    let r = Root::new("cleannothing", 120);
    let t1 = r.task("a", "MARK-t1");
    let exe = r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let kept = r.dir.join("build/main.bins/proj/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    r.ok(&["build", "proj"]);
    assert_eq!(r.probe(), "MARK-main");
    let before = fs::read(&kept).unwrap_or_else(|e| panic!("main's build kept no copy at {}: {e}", kept.display()));

    let o = r.ok(&["run", "proj", "--task", &t1, "--", "echo", "nothing"]);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains("last built from projects/proj"), "the job cleans:\n{}", o.stdout);
    assert!(!exe.exists(), "cargo's folder has no binary for `markprobe` after the clean:\n{}", o.stdout);
    assert_eq!(fs::read(&kept).unwrap(), before, "main.bins still has main's copy, byte for byte");
    assert_eq!(fs::read_to_string(format!("{}.source", kept.display())).unwrap().trim(), "projects/proj");
}

/// Round 8 (review q-20261121-011535, minor at keep_main_bins): a main build that FAILS can leave an exe it did build in cargo's
/// folder (bin A compiled, bin B did not), with the bin note already set to main by prepare. The next job of another folder
/// that cleans must NOT copy it into main.bins: main.bins keeps the last good copy, byte for byte and note for note. On round
/// 7's code (ce4ca63) the foreign clean's keep copied the failed build's `markprobe` (MARK-new) into main.bins.
#[test]
fn a_failed_main_build_never_reaches_main_bins_even_through_a_foreign_clean() {
    let r = Root::new("failedkeep", 120);
    let t1 = r.task("a", "MARK-t1");
    let proj = r.dir.join("projects/proj");
    let exe = r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let kept = r.dir.join("build/main.bins/proj/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let note = PathBuf::from(format!("{}.source", kept.display()));
    let prints = |p: &Path| -> String {
        let o = Command::new(p).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", p.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    r.ok(&["build", "proj"]);
    assert_eq!(prints(&kept), "MARK-main");
    let (before, before_note) = (fs::read(&kept).unwrap(), fs::read(&note).unwrap());

    // new code for bin A (`markprobe`) and a second bin B that does not compile; --keep-going builds A although B fails
    set_marker(&proj, "MARK-new");
    fs::create_dir_all(proj.join("src/bin")).unwrap();
    fs::write(proj.join("src/bin/broken.rs"), "fn main() { this does not compile }\n").unwrap();
    let o = r.run(&["run", "proj", "--", "cargo", "build", "--offline", "--keep-going"]);
    assert_ne!(o.code, 0, "the main build fails:\n{}\n{}", o.stdout, o.stderr);
    assert_eq!(prints(&exe), "MARK-new", "bin A compiled into cargo's folder (the partial build):\n{}\n{}", o.stdout, o.stderr);
    assert_eq!(fs::read(&kept).unwrap(), before, "the failed build refreshed nothing");

    // a worktree job cleans the package: its keep must not take the failed build's exe
    let o = r.ok(&["run", "proj", "--task", &t1, "--", "echo", "nothing"]);
    assert!(o.stdout.contains("cleaned `markprobe`"), "{}", o.stdout);
    assert!(o.stdout.contains("main.bins: `markprobe` (debug) not kept from"), "{}", o.stdout);
    assert_eq!(fs::read(&kept).unwrap(), before, "main.bins is unchanged, byte for byte:\n{}", o.stdout);
    assert_eq!(fs::read(&note).unwrap(), before_note, "and its note too");
    assert_eq!(prints(&kept), "MARK-main");
}

/// Round 9 (review q-20261121-023602, the reviewer's exact sequence): a main `run -- cargo build --keep-going` FAILS but links a
/// new bin A (`markprobe`, MARK-new) while bin B does not compile; then a SUCCESSFUL main `run -- cargo build --example only`
/// builds only the example. That success must not promote the failed build's A into main.bins: a step refreshes only the exes
/// it produced, so main.bins keeps main's last good A, bytes and note. On 35517c05 (round 8) the successful run refreshed every
/// cached bin and main.bins got MARK-new. The positive case follows: a successful build that does write A refreshes it.
#[test]
fn a_successful_step_refreshes_only_the_bins_it_built_never_a_failed_steps_leftover() {
    let r = Root::new("onlyproduced", 120);
    let proj = r.dir.join("projects/proj");
    fs::create_dir_all(proj.join("examples")).unwrap();
    fs::write(proj.join("examples/only.rs"), "fn main() { print!(\"example {}\", markprobe::MARKER); }\n").unwrap();
    let exe = r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let kept = r.dir.join("build/main.bins/proj/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let note = PathBuf::from(format!("{}.source", kept.display()));
    let prints = |p: &Path| -> String {
        let o = Command::new(p).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", p.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    r.ok(&["build", "proj"]);
    assert_eq!(prints(&kept), "MARK-main");
    let (before, before_note) = (fs::read(&kept).unwrap(), fs::read(&note).unwrap());

    // the failed main build: A links (MARK-new), B does not compile
    set_marker(&proj, "MARK-new");
    fs::create_dir_all(proj.join("src/bin")).unwrap();
    fs::write(proj.join("src/bin/broken.rs"), "fn main() { this does not compile }\n").unwrap();
    let o = r.run(&["run", "proj", "--", "cargo", "build", "--offline", "--keep-going"]);
    assert_ne!(o.code, 0, "the main build fails:\n{}\n{}", o.stdout, o.stderr);
    assert_eq!(prints(&exe), "MARK-new", "bin A from the failed build is in cargo's folder:\n{}\n{}", o.stdout, o.stderr);

    // a successful main step that builds only the example: A is not its output
    let o = r.ok(&["run", "proj", "--", "cargo", "build", "--offline", "--example", "only"]);
    assert_eq!(prints(&exe), "MARK-new", "the example build left cargo's A as it was");
    assert!(!o.stdout.contains("kept main's `markprobe`"), "the example build kept nothing it did not build:\n{}", o.stdout);
    assert_eq!(fs::read(&kept).unwrap(), before, "main.bins keeps main's last good A, byte for byte:\n{}", o.stdout);
    assert_eq!(fs::read(&note).unwrap(), before_note, "and its note");

    // the positive case: a successful build that writes A refreshes it
    fs::remove_file(proj.join("src/bin/broken.rs")).unwrap();
    set_marker(&proj, "MARK-good");
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("kept main's `markprobe` (debug"), "{}", o.stdout);
    assert_eq!(prints(&kept), "MARK-good", "a successful build of A refreshes A:\n{}", o.stdout);
    assert_eq!(fs::read_to_string(&note).unwrap().trim(), "projects/proj");
}

/// Round 2 (review q-20261120-202642): `build/main.bins` is THE stable place of main's binaries, so every SUCCESSFUL job of the
/// main checkout refreshes it (a build step, the test step's compile, a `run` command), and nothing a worktree builds ever
/// replaces it. Checked by content: each main build prints a new marker. On round 1's code (9dd3171) the first assertion
/// failed: a main build alone left no kept copy (only another folder's clean made one).
#[test]
fn every_successful_main_build_refreshes_main_bins_and_no_worktree_build_replaces_them() {
    let r = Root::new("refresh", 120);
    let t1 = r.task("a", "MARK-t1");
    let proj = r.dir.join("projects/proj");
    let suffix = std::env::consts::EXE_SUFFIX;
    let kept = |profile: &str| r.dir.join("build/main.bins/proj").join(profile).join(format!("markprobe{suffix}"));
    let prints = |p: &Path| -> String {
        let o = Command::new(p).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", p.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let kept_note = |profile: &str| fs::read_to_string(format!("{}.source", kept(profile).display())).ok().map(|s| s.trim().to_string());

    // a main build refreshes main.bins
    let o = r.ok(&["build", "proj"]);
    assert_eq!(prints(&kept("debug")), "MARK-main", "a main build keeps its binary:\n{}", o.stdout);
    assert_eq!(kept_note("debug").as_deref(), Some("projects/proj"));
    assert!(o.stdout.contains("kept main's `markprobe` (debug"), "{}", o.stdout);
    // a second main build of new code refreshes it again, with no other folder's job in between
    set_marker(&proj, "MARK-main2");
    r.ok(&["build", "proj"]);
    assert_eq!(prints(&kept("debug")), "MARK-main2", "main.bins holds main's NEWEST build");
    // the same build again copies nothing (unchanged)
    let o = r.ok(&["build", "proj"]);
    assert!(!o.stdout.contains("kept main's"), "{}", o.stdout);

    // a worktree builds the bin itself: cargo's cache holds its exe, main.bins is still main's
    r.ok(&["build", "proj", "--task", &t1]);
    assert_eq!(r.probe(), "MARK-t1", "cargo's own folder is a cache a worktree may overwrite");
    assert_eq!(prints(&kept("debug")), "MARK-main2", "a worktree build never replaces main's kept binary");
    r.ok(&["check", "proj", "--task", &t1]);
    assert_eq!(prints(&kept("debug")), "MARK-main2", "nor does a worktree check");

    // the test step's compile refreshes it too (a `test` job has no build step)
    set_marker(&proj, "MARK-main3");
    let o = r.ok(&["test", "proj"]);
    assert!(o.stdout.contains("test: ok"), "{}", o.stdout);
    assert_eq!(prints(&kept("debug")), "MARK-main3", "a main test job keeps what it compiled:\n{}", o.stdout);

    // a main `run` that builds release refreshes the release copy; its note says it came from a `run`
    let o = r.ok(&["run", "proj", "--", "cargo", "build", "--release", "--offline"]);
    assert_eq!(prints(&kept("release")), "MARK-main3", "{}", o.stdout);
    assert_eq!(kept_note("release").as_deref(), Some("run:projects/proj"));

    // a FAILED main build refreshes nothing: main.bins keeps the last good build
    fs::write(proj.join("src/lib.rs"), "pub const MARKER: &str = \"MARK-broken\" this does not compile;\n").unwrap();
    let o = r.run(&["build", "proj"]);
    assert_ne!(o.code, 0, "{}", o.stdout);
    assert_eq!(prints(&kept("debug")), "MARK-main3", "a failed build never touches main.bins");

    // a worktree release build after all that leaves the release copy main's
    r.ok(&["run", "proj", "--task", &t1, "--", "cargo", "build", "--release", "--offline"]);
    assert_eq!(prints(&r.dir.join("build/release").join(format!("markprobe{suffix}"))), "MARK-t1");
    assert_eq!(prints(&kept("release")), "MARK-main3", "a worktree release build never replaces main's kept release binary");
}

/// A kept copy that is RUNNING (Lazlo drives main's crash-sim from main.bins) cannot be replaced on Windows. A successful main
/// build must still pass: the old copy stays, a `.stale` marker names the new build, the job says so loudly, and the next main
/// build that finds the copy free refreshes it and removes the marker. On 8574bf2 the main build failed ("cannot keep main's").
/// Windows only: elsewhere a rename replaces a running exe, so there is no in-use case.
#[cfg(windows)]
#[test]
fn a_kept_copy_in_use_is_left_marked_stale_and_refreshed_by_the_next_main_build() {
    let r = Root::new("inuse", 120);
    let proj = r.dir.join("projects/proj");
    // main's binary waits while it is given `wait`, so the kept copy can be kept running
    fs::write(proj.join("src/main.rs"), "fn main() {\n    if std::env::args().nth(1).as_deref() == Some(\"wait\") {\n        std::thread::sleep(std::time::Duration::from_secs(120));\n    }\n    print!(\"{}\", markprobe::MARKER);\n}\n").unwrap();
    let kept = r.dir.join("build/main.bins/proj/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let stale = PathBuf::from(format!("{}.stale", kept.display()));
    let prints = |p: &Path| -> String {
        let o = Command::new(p).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", p.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    r.ok(&["build", "proj"]);
    assert_eq!(prints(&kept), "MARK-main");

    // run the kept copy, then build new main code
    let mut running = Command::new(&kept).arg("wait").stdout(Stdio::null()).spawn().unwrap();
    set_marker(&proj, "MARK-main2");
    let o = r.run(&["build", "proj"]);
    let _ = running.kill();
    let _ = running.wait();
    assert_eq!(o.code, 0, "a kept copy in use must not fail a successful main build:\n{}\n{}", o.stdout, o.stderr);
    assert!(o.stdout.contains("main.bins: `markprobe` (debug) NOT refreshed (in use") && o.stdout.contains("the copy is from projects/proj"), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-main2", "the new build is in cargo's folder");
    assert_eq!(prints(&kept), "MARK-main", "the old copy stays as it was");
    let marker = fs::read_to_string(&stale).unwrap_or_else(|e| panic!("no stale marker at {}: {e}\n{}", stale.display(), o.stdout));
    assert!(marker.starts_with("stale: projects/proj, in use at "), "{marker}");
    assert_eq!(fs::read_to_string(format!("{}.source", kept.display())).unwrap().trim(), "projects/proj", "the copy is still named by its own note");

    assert!(marker.lines().nth(1).is_some_and(|l| l.starts_with("built: ") && l.contains(" bytes, modified ")), "the marker names the new exe: {marker}");

    // nothing runs it now: the next main build (no code change) refreshes it and removes the marker
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("kept main's `markprobe` (debug"), "{}", o.stdout);
    assert_eq!(prints(&kept), "MARK-main2");
    assert!(!stale.exists(), "the marker is gone once the copy is fresh");

    // a stale copy before ANOTHER folder's clean: the exe its marker vouches for (main's successful build) is kept, and while
    // the copy is still in use that job is refused (its clean would delete main's newest build with no copy of it)
    let t1 = r.task("a", "MARK-t1");
    let mut running = Command::new(&kept).arg("wait").stdout(Stdio::null()).spawn().unwrap();
    set_marker(&proj, "MARK-main3");
    r.ok(&["build", "proj"]);
    assert!(stale.exists());
    let o = r.run(&["run", "proj", "--task", &t1, "--", "echo", "nothing"]);
    let still = r.probe();
    let _ = running.kill();
    let _ = running.wait();
    assert_ne!(o.code, 0, "refused while the vouched copy is in use:\n{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("before cleaning `markprobe`"), "{}", o.stderr);
    assert_eq!(still, "MARK-main3", "nothing was cleaned");
    let o = r.ok(&["run", "proj", "--task", &t1, "--", "echo", "nothing"]);
    assert!(o.stdout.contains("kept main's `markprobe` (debug") && o.stdout.contains("cleaned `markprobe`"), "{}", o.stdout);
    assert_eq!(prints(&kept), "MARK-main3", "main's newest successful build reached main.bins before the clean");
    assert!(!stale.exists());
}

/// Round 10 (review q-20261121-032813), the shared part of the reviewer's sequence: project A (`proj`, package `markprobe`) and
/// project B (`projb`, package `b_package`) both build a bin `markprobe`, so both write `build/<profile>/markprobe.exe`. A's
/// main build is kept; then, while A's kept copy runs, A's main checkout is rebuilt (MARK-main2), so the copy is marked stale
/// and A's newer exe is only in cargo's folder. Returns B's task id, the running copy, A's kept exe and its stale marker.
/// Round 11 (review q-20261121-041528) uses it with another package name for B: only the bin (exe path) is shared.
#[cfg(windows)]
fn shared_exe_path_with_a_stale_copy_in_use(r: &Root, b_package: &str) -> (String, Child, PathBuf, PathBuf) {
    let proj = r.dir.join("projects/proj");
    fs::write(proj.join("src/main.rs"), "fn main() {\n    if std::env::args().nth(1).as_deref() == Some(\"wait\") {\n        std::thread::sleep(std::time::Duration::from_secs(120));\n    }\n    print!(\"{}\", markprobe::MARKER);\n}\n").unwrap();
    // project B: another repo whose package `b_package` has a bin named `markprobe`
    let b = r.dir.join("projects/projb");
    fs::create_dir_all(b.join("src")).unwrap();
    fs::write(b.join("Cargo.toml"), format!("[package]\nname = \"{b_package}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"markprobe\"\npath = \"src/main.rs\"\n\n[workspace]\n")).unwrap();
    fs::write(b.join(".gitignore"), "target\n").unwrap();
    fs::write(b.join("src/main.rs"), "fn main() { print!(\"MARK-b\"); }\n").unwrap();
    git(&b, &["init", "-q", "-b", "main"]);
    git(&b, &["config", "core.autocrlf", "false"]);
    git(&b, &["add", "."]);
    git(&b, &["commit", "-q", "-m", "first"]);
    let tb = r.ok(&["task", "add", "projb", "--title", "t", "--claim", "b"]).stdout.trim().to_string();
    r.ok(&["task", "start", &tb, "--owner", "coder", "--worktree"]);

    let kept = r.dir.join("build/main.bins/proj/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let stale = PathBuf::from(format!("{}.stale", kept.display()));
    r.ok(&["build", "proj"]);
    let running = Command::new(&kept).arg("wait").stdout(Stdio::null()).spawn().unwrap();
    set_marker(&proj, "MARK-main2");
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("NOT refreshed (in use"), "{}", o.stdout);
    assert!(stale.is_file());
    assert_eq!(r.probe(), "MARK-main2", "A's newer exe is only in cargo's folder");
    (tb, running, kept, stale)
}

/// Round 10 (review q-20261121-032813): while A's stale kept copy is still in use, a worktree job of project B (same package
/// name) must refuse its clean, which would delete A's newer exe with no copy of it. On a8588620 B's job looked only at B's
/// main checkout, cleaned, and deleted A's MARK-main2 (this test failed at the refusal assertion). Windows only (in use).
#[cfg(windows)]
#[test]
fn another_projects_worktree_with_the_same_package_name_refuses_while_the_stale_copy_is_in_use() {
    let r = Root::new("sharedrefuse", 120);
    let (tb, mut running, kept, stale) = shared_exe_path_with_a_stale_copy_in_use(&r, "markprobe");
    let o = r.run(&["run", "projb", "--task", &tb, "--", "echo", "nothing"]);
    let left = r.probe();
    let _ = running.kill();
    let _ = running.wait();
    assert_ne!(o.code, 0, "B's clean must be refused while A's vouched copy is in use:\n{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("before cleaning `markprobe`"), "{}", o.stderr);
    assert_eq!(left, "MARK-main2", "A's newer exe was not deleted");
    assert!(stale.is_file() && kept.is_file());
}

/// Round 10 (review q-20261121-032813): once A's stale kept copy is free, B's worktree clean (same package name) first keeps A's
/// newer exe in main.bins/proj, then cleans. On a8588620 the clean deleted it and main.bins kept the older MARK-main (this test
/// failed at the `kept main's` assertion). Windows only (the stale marker needs a copy in use).
#[cfg(windows)]
#[test]
fn another_projects_worktree_with_the_same_package_name_keeps_the_vouched_exe_before_its_clean() {
    let r = Root::new("sharedkeep", 120);
    let (tb, mut running, kept, stale) = shared_exe_path_with_a_stale_copy_in_use(&r, "markprobe");
    let _ = running.kill();
    let _ = running.wait();
    let o = r.ok(&["run", "projb", "--task", &tb, "--", "echo", "nothing"]);
    assert!(o.stdout.contains("kept main's `markprobe` (debug") && o.stdout.contains("cleaned `markprobe`"), "{}", o.stdout);
    let printed = Command::new(&kept).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&printed.stdout), "MARK-main2", "A's newest successful build reached main.bins/proj before B's clean");
    assert!(!stale.exists(), "the copy is fresh again");
    assert!(!r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX)).exists(), "and B's clean ran");
}

/// Round 11 (review q-20261121-041528): B's package has ANOTHER name (`otherpkg`) but the same bin name `markprobe`, so both
/// projects write `build/debug/markprobe.exe`. While A's stale kept copy is in use, B's worktree job must refuse before it
/// cleans or builds that path. On dd272911 B's job matched other projects by package name only, found none, and went on (this
/// test failed at the refusal assertion). Windows only (in use).
#[cfg(windows)]
#[test]
fn another_projects_worktree_with_only_the_same_bin_name_refuses_while_the_stale_copy_is_in_use() {
    let r = Root::new("binrefuse", 120);
    let (tb, mut running, kept, stale) = shared_exe_path_with_a_stale_copy_in_use(&r, "otherpkg");
    let o = r.run(&["run", "projb", "--task", &tb, "--", "cargo", "build", "--offline"]);
    let exe = r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let left = fs::read(&exe).ok();
    let _ = running.kill();
    let _ = running.wait();
    assert_ne!(o.code, 0, "B's job must be refused while A's vouched copy is in use:\n{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("before cleaning `otherpkg`"), "{}", o.stderr);
    let printed = Command::new(&exe).output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    assert!(left.is_some() && printed == "MARK-main2", "A's newer exe is still in cargo's folder, not removed or overwritten (it prints {printed:?})");
    assert!(stale.is_file() && kept.is_file());
}

/// Round 11 (review q-20261121-041528): once A's stale kept copy is free, B's worktree job (package `otherpkg`, bin `markprobe`)
/// first keeps A's newer exe in main.bins/proj, then cleans and builds its own exe at the same path. On dd272911 the job lost
/// A's MARK-main2 (main.bins kept the older MARK-main; this test failed at the `kept main's` assertion). Windows only.
#[cfg(windows)]
#[test]
fn another_projects_worktree_with_only_the_same_bin_name_keeps_the_vouched_exe_before_writing_its_path() {
    let r = Root::new("binkeep", 120);
    let (tb, mut running, kept, stale) = shared_exe_path_with_a_stale_copy_in_use(&r, "otherpkg");
    let _ = running.kill();
    let _ = running.wait();
    let o = r.ok(&["run", "projb", "--task", &tb, "--", "cargo", "build", "--offline"]);
    assert!(o.stdout.contains("kept main's `markprobe` (debug") && o.stdout.contains("cleaned `otherpkg`"), "{}", o.stdout);
    let prints = |p: &Path| String::from_utf8_lossy(&Command::new(p).output().unwrap().stdout).into_owned();
    assert_eq!(prints(&kept), "MARK-main2", "A's newest successful build reached main.bins/proj before B wrote the path");
    assert!(!stale.exists(), "the copy is fresh again");
    assert_eq!(prints(&r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX))), "MARK-b", "B's own build then wrote the shared path");
}

fn main_build_with_empty_other_workspace(name: &str) -> (Root, String) {
    let r = Root::new(name, 120);
    let t1 = r.task("a", "MARK-t1");
    r.ok(&["build", "proj"]);
    fs::write(r.dir.join("projects/other/Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    (r, t1)
}

/// An empty workspace in another main checkout has no bins to preserve and must not block a worktree's clean.
#[test]
fn a_clean_succeeds_when_another_main_is_an_empty_workspace() {
    let (r, t1) = main_build_with_empty_other_workspace("emptymain");
    let source = format!("worktrees/proj/{t1}");
    let o = r.ok(&["build", "proj", "--task", &t1]);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains("last built from projects/proj"), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-t1", "the worktree built its own binary after cleaning");
    assert_eq!(r.note().as_deref(), Some(source.as_str()));
    assert_eq!(r.bin_note().as_deref(), Some(source.as_str()));
}

/// The empty-workspace exception applies only to successful metadata, not to a broken manifest.
#[test]
fn a_clean_is_refused_when_an_empty_other_workspace_manifest_is_invalid() {
    let (r, t1) = main_build_with_empty_other_workspace("emptyinvalid");
    fs::write(r.dir.join("projects/other/Cargo.toml"), "[workspace\nmembers = []\n").unwrap();
    let o = r.run(&["build", "proj", "--task", &t1]);
    assert_ne!(o.code, 0, "{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("refusing to clean: the packages of project `other` cannot be listed"), "{}", o.stderr);
    assert!(o.stderr.contains("cargo metadata failed"), "{}", o.stderr);
    assert_eq!(r.probe(), "MARK-main", "nothing was cleaned or built");
    assert_eq!(r.note().as_deref(), Some("projects/proj"));
    assert_eq!(r.bin_note().as_deref(), Some("projects/proj"));
}

/// The folder about to be built still needs at least one package, even though other empty mains are skipped.
#[test]
fn building_an_empty_workspace_itself_is_refused() {
    let (r, _) = main_build_with_empty_other_workspace("emptybuild");
    let o = r.run(&["build", "other"]);
    assert_ne!(o.code, 0, "{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("cargo metadata lists no packages in this folder"), "{}", o.stderr);
    assert_eq!(r.probe(), "MARK-main", "the unrelated main binary was not touched");
    assert_eq!(r.note().as_deref(), Some("projects/proj"));
    assert!(!r.dir.join("state/build-sources/other").exists(), "the empty workspace was not recorded as built");
}

/// Round 10: if any project's packages cannot be listed, a clean is refused (fail closed): nothing then tells whether it would
/// delete that project's main exe. Here project `other` gets a broken Cargo.toml and a worktree of `proj` would clean.
#[test]
fn a_clean_is_refused_when_some_projects_packages_cannot_be_listed() {
    let r = Root::new("listfail", 120);
    let t1 = r.task("a", "MARK-t1");
    r.ok(&["build", "proj"]);
    fs::write(r.dir.join("projects/other/Cargo.toml"), "this is not toml\n").unwrap();
    let o = r.run(&["build", "proj", "--task", &t1]);
    assert_ne!(o.code, 0, "{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("refusing to clean: the packages of project `other` cannot be listed"), "{}", o.stderr);
    assert_eq!(r.probe(), "MARK-main", "nothing was cleaned or built");
}

/// Review q-20261120-231552: a junction at build/main.bins/proj/debug pointing OUTSIDE the build folder must never make the refresh
/// delete a `.source` note or replace an exe there. The job refuses and the outside folder is untouched, byte for byte. On
/// 1e9bf60 the refresh followed the junction (the outside exe and its note were replaced). Windows only (a junction).
#[cfg(windows)]
#[test]
fn a_junction_in_main_bins_makes_the_refresh_refuse_and_writes_nothing_outside() {
    use std::os::windows::process::CommandExt;
    let r = Root::new("keepjunction", 120);
    let proj = r.dir.join("projects/proj");
    r.ok(&["build", "proj"]);
    let debug = r.dir.join("build/main.bins/proj/debug");
    assert!(debug.is_dir());
    // the profile folder becomes a junction to a folder outside the build folder that holds files of the same names
    let outside = r.dir.join("outside");
    fs::create_dir_all(&outside).unwrap();
    let exe_name = format!("markprobe{}", std::env::consts::EXE_SUFFIX);
    fs::write(outside.join(&exe_name), "not ours").unwrap();
    fs::write(outside.join(format!("{exe_name}.source")), "projects/proj\n").unwrap();
    fs::remove_dir_all(&debug).unwrap();
    let ok = Command::new("cmd").arg("/C").raw_arg(format!("mklink /J \"{}\" \"{}\"", debug.display(), outside.display())).output().unwrap().status.success();
    assert!(ok, "mklink /J failed");
    let before: Vec<(String, Vec<u8>)> = {
        let mut v: Vec<_> = fs::read_dir(&outside).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap())).collect();
        v.sort();
        v
    };

    // new main code: the build succeeds, the refresh must refuse; and so must the test step's refresh after its compile
    set_marker(&proj, "MARK-main2");
    let o = r.run(&["build", "proj"]);
    let ot = r.run(&["test", "proj"]);
    let after: Vec<(String, Vec<u8>)> = {
        let mut v: Vec<_> = fs::read_dir(&outside).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read(e.path()).unwrap())).collect();
        v.sort();
        v
    };
    let _ = fs::remove_dir(&debug); // the junction only, not its target
    assert_eq!(after, before, "nothing outside the build folder was written, removed or replaced:\n{}\n{}", o.stdout, o.stderr);
    assert_eq!(o.code, 1, "the job refuses:\n{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("refusing to keep main's binaries") && o.stderr.contains("junction"), "{}", o.stderr);
    // round 8 (review q-20261121-011535, minor at job.rs:1611): a job whose build steps succeeded still prints its summary
    // block before it reports the refresh failure and fails (on ce4ca63 it ended without the summary)
    assert!(o.stdout.contains("--- summary ---\nbuild: ok"), "the summary is printed:\n{}", o.stdout);
    assert_eq!(ot.code, 1, "the test job fails too:\n{}\n{}", ot.stdout, ot.stderr);
    assert!(ot.stdout.contains("--- summary ---\ntest: ok"), "its tests ran and its summary is printed:\n{}\n{}", ot.stdout, ot.stderr);
    assert!(ot.stderr.contains("refusing to keep main's binaries") && ot.stderr.contains("junction"), "{}", ot.stderr);
}

/// Review q-20261120-233123: the temp names beside a kept exe (`<exe>.source.tmp`, `.stale.tmp`, `.bskeep.tmp`) are written too,
/// so a link planted there must never let a main build write outside the build folder. (a) a junction at `<exe>.source.tmp`
/// pointing outside: the job refuses (`guard_kept_files` checks every temp name) and the outside folder is untouched. (b) a hard
/// link at `<exe>.source.tmp` to a file outside (not a reparse point, so no path check can see it): the name is removed and a
/// NEW temp file is created, so the outside file keeps its bytes and the build passes. On 4a3bdd7 (a) failed with another
/// error than the refusal and (b) overwrote the outside file. Windows only (a junction; NTFS hard links).
#[cfg(windows)]
#[test]
fn a_link_planted_at_a_temp_name_beside_a_kept_exe_never_writes_outside() {
    use std::os::windows::process::CommandExt;
    let r = Root::new("keeptmp", 120);
    let proj = r.dir.join("projects/proj");
    r.ok(&["build", "proj"]);
    let kept = r.dir.join("build/main.bins/proj/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let tmp = PathBuf::from(format!("{}.source.tmp", kept.display()));
    let prints = |p: &Path| -> String {
        let o = Command::new(p).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", p.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };

    // (a) a junction to an outside folder
    let outside = r.dir.join("outside-dir");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("precious.txt"), "precious").unwrap();
    let ok = Command::new("cmd").arg("/C").raw_arg(format!("mklink /J \"{}\" \"{}\"", tmp.display(), outside.display())).output().unwrap().status.success();
    assert!(ok, "mklink /J failed");
    set_marker(&proj, "MARK-main2");
    let o = r.run(&["build", "proj"]);
    let listing: Vec<String> = fs::read_dir(&outside).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    let _ = fs::remove_dir(&tmp); // the junction only
    assert_eq!(listing, ["precious.txt"], "nothing was created in the outside folder");
    assert_eq!(fs::read_to_string(outside.join("precious.txt")).unwrap(), "precious");
    assert_eq!(o.code, 1, "the job refuses:\n{}\n{}", o.stdout, o.stderr);
    assert!(o.stderr.contains("refusing to keep main's binaries") && o.stderr.contains("junction") && o.stderr.contains(".source.tmp"), "{}", o.stderr);
    assert_eq!(prints(&kept), "MARK-main", "nothing was kept through it");

    // (b) a hard link to an outside file
    let precious = r.dir.join("outside-file.txt");
    fs::write(&precious, "precious file").unwrap();
    fs::hard_link(&precious, &tmp).unwrap();
    // new code, so this step writes the exe (round 9: a step refreshes only the exes it produced; (a)'s exe was built by a step
    // whose refresh refused, and a build that finds it fresh does not vouch for it)
    set_marker(&proj, "MARK-main3");
    let o = r.ok(&["build", "proj"]);
    assert_eq!(fs::read_to_string(&precious).unwrap(), "precious file", "the outside file keeps its bytes:\n{}", o.stdout);
    assert_eq!(prints(&kept), "MARK-main3", "and the kept copy is refreshed through a new temp file:\n{}", o.stdout);
    assert_eq!(fs::read_to_string(format!("{}.source", kept.display())).unwrap().trim(), "projects/proj");
    assert!(!tmp.exists(), "no temp name is left");
}

#[test]
fn run_with_task_runs_in_the_worktree_and_checkpoints_it() {
    let r = Root::new("run", 120);
    let t1 = r.task("a", "MARK-t1");
    fs::write(r.worktree(&t1).join("only-here.txt"), "x").unwrap();
    let list = if cfg!(windows) { "dir /b" } else { "ls" };
    let o = r.ok(&["run", "proj", "--task", &t1, "--", list]);
    assert!(o.stdout.contains("only-here.txt"), "the command ran in the worktree: {}", o.stdout);
    let rec = fs::read_to_string(r.job_records().last().unwrap()).unwrap();
    assert!(rec.contains(&format!("task = {t1}")) && rec.contains("checkpoint = refs/broadsword/checkpoints/"), "{rec}");
    let o = r.ok(&["run", "proj", "--", list]);
    assert!(!o.stdout.contains("only-here.txt"), "without --task it ran in the main checkout: {}", o.stdout);
    assert_eq!(r.note().as_deref(), Some("run:projects/proj"), "`run` leaves a `run:` note: it names the folder but vouches for nothing");
}

/// Blocker 2: `run <project> -- cargo build` in a worktree puts the worktree's binary in the shared folder. `run` does not
/// parse the command; it marks the notes of the folder's packages `run:<folder>`, so `install` (which needs exactly `.`/main)
/// refuses and the next build in another folder cleans and records its own. Without this the note kept saying main.
#[test]
fn run_that_builds_in_a_worktree_marks_the_note_as_a_run_so_the_next_build_elsewhere_cleans() {
    let r = Root::new("runbuild", 120);
    let t1 = r.task("a", "MARK-t1");
    r.ok(&["build", "proj"]);
    assert_eq!(r.note().as_deref(), Some("projects/proj"));
    set_marker(&r.worktree(&t1), "MARK-t1"); // newer than main's build, so cargo rebuilds for the worktree
    let target = r.dir.join("build").to_string_lossy().replace('\\', "/");
    let o = r.ok(&["run", "proj", "--task", &t1, "--", "cargo", "build", "--offline", "--target-dir", &target]);
    assert_eq!(r.probe(), "MARK-t1", "the run command really did put the worktree's binary in the shared folder:
{}{}", o.stdout, o.stderr);
    assert_eq!(r.note().as_deref(), Some(format!("run:worktrees/proj/{t1}").as_str()), "the note no longer vouches for main, and is never installable");
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains(&format!("last built from worktrees/proj/{t1}")), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-main");
    assert_eq!(r.note().as_deref(), Some("projects/proj"));
}

/// A `cargo test` inside `run` right after ANOTHER worktree built must test THIS worktree's code. Cargo keeps a fingerprint per
/// checkout: build B, then A, and B looks fresh, so cargo says `Finished` and runs the binary A left in the shared folder. `run`
/// now does the same preparation as build, test and check (`cargo clean -p` for a package whose note names another checkout).
#[test]
fn run_after_another_worktrees_build_tests_its_own_code_not_the_other_binary() {
    let r = Root::new("runstale", 120);
    let t1 = r.task("a", "MARK-t1");
    let t2 = r.task("b", "MARK-t2");
    r.ok(&["check", "proj", "--task", &t2]); // B built: cargo now thinks B is fresh
    assert_eq!(r.probe(), "MARK-t2");
    r.ok(&["build", "proj", "--task", &t1]); // A built after it: the shared folder holds A's binary
    assert_eq!(r.probe(), "MARK-t1");
    let target = r.dir.join("build").to_string_lossy().replace('\\', "/");
    let o = r.ok(&["run", "proj", "--task", &t2, "--", "cargo", "test", "--offline", "--target-dir", &target]);
    assert!(o.stdout.contains("cleaned `markprobe`"), "run prepared the build folder before its command: {}", o.stdout);
    assert!(o.stdout.contains("test result: ok") && o.stdout.contains("the_binary_is_this_checkouts_binary"), "B's test ran and passed against B's own binary: {}\n{}", o.stdout, o.stderr);
    assert_eq!(r.probe(), "MARK-t2", "the binary in the shared folder is B's: the run tested B's code");
    assert_eq!(r.note().as_deref(), Some(format!("run:worktrees/proj/{t2}").as_str()), "a `run` build is still never installable: the note is a `run:` note");
}

/// The stale `--release` case (found by the merge queue's perf gate): checkout B builds release, checkout A builds release,
/// then B again. `cargo clean -p` alone cleans only the dev profile, so B's second release build said `Finished` without
/// compiling and left A's exe in build/release. Every clean now covers the release profile too, so each build gives its own
/// checkout's binary.
#[test]
fn a_release_build_after_another_checkouts_release_build_rebuilds_its_own_binary() {
    let r = Root::new("relstale", 120);
    let t1 = r.task("a", "MARK-t1");
    let t2 = r.task("b", "MARK-t2");
    let probe_release = || {
        let exe = r.dir.join("build/release").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
        let o = Command::new(&exe).output().unwrap_or_else(|e| panic!("no binary at {}: {e}", exe.display()));
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    for (id, want) in [(&t2, "MARK-t2"), (&t1, "MARK-t1"), (&t2, "MARK-t2"), (&t1, "MARK-t1")] {
        let o = r.ok(&["run", "proj", "--task", id, "--", "cargo", "build", "--release", "--offline"]);
        assert_eq!(probe_release(), want, "the release exe after a release build in {id} must be its own:\n{}", o.stdout);
    }
}

/// The blocker of the first review: every `run` after another `run` in the same folder used to clean the package, so
/// `run -- cargo build` followed by `run -- <the built program>` deleted the program before the second command started.
/// Same folder now means no clean; the note stays a `run:` note throughout, so `install` still refuses it.
#[test]
fn a_second_run_in_the_same_folder_cleans_nothing_and_the_built_file_survives() {
    let r = Root::new("runrun", 120);
    let target = r.dir.join("build").to_string_lossy().replace('\\', "/");
    let o = r.ok(&["run", "proj", "--", "cargo", "build", "--offline", "--target-dir", &target]);
    assert!(o.stdout.contains("cleaned `markprobe`"), "the first run had no note, so it prepared the folder: {}", o.stdout);
    assert_eq!(r.probe(), "MARK-main");
    let exe = r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    // the second command only checks that the built file is still there
    let check = if cfg!(windows) { format!("if exist \"{}\" (exit 0) else (exit 7)", exe.display()) } else { format!("test -f '{}'", exe.display()) };
    let o = r.ok(&["run", "proj", "--", &check]);
    assert!(!o.stdout.contains("cleaned"), "no clean in the same folder: {}", o.stdout);
    assert!(exe.is_file(), "the program built by the first run is still there");
    assert_eq!(r.note().as_deref(), Some("run:projects/proj"));
    // the first plain build in the same folder afterwards cleans once (a run may have replaced the exe), and its note is plain again
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("cleaned `markprobe`") && o.stdout.contains("a `run` in this folder"), "{}", o.stdout);
    assert_eq!(r.note().as_deref(), Some("projects/proj"));
    assert_eq!(r.probe(), "MARK-main");
    // a second plain build needs no clean
    let o = r.ok(&["build", "proj"]);
    assert!(!o.stdout.contains("cleaned"), "{}", o.stdout);
    // and a run after that goes back to a `run:` note without a clean
    let o = r.ok(&["run", "proj", "--", &check]);
    assert!(!o.stdout.contains("cleaned"), "{}", o.stdout);
    assert_eq!(r.note().as_deref(), Some("run:projects/proj"));
}

/// A `run` may replace the built exe while cargo's fingerprint stays fresh. The first plain build afterwards must rebuild it.
#[test]
fn the_first_plain_build_after_a_run_rebuilds_an_exe_the_run_replaced() {
    let r = Root::new("runreplace", 120);
    let t1 = r.task("a", "MARK-t1");
    r.ok(&["build", "proj", "--task", &t1]);
    assert_eq!(r.probe(), "MARK-t1");
    let exe = r.dir.join("build/debug").join(format!("markprobe{}", std::env::consts::EXE_SUFFIX));
    let saved = r.scratch("t1-exe");
    fs::copy(&exe, &saved).unwrap(); // a different, working markprobe
    r.ok(&["build", "proj"]);
    assert_eq!(r.probe(), "MARK-main");
    let copy = if cfg!(windows) { format!("copy /Y \"{}\" \"{}\"", saved.display(), exe.display()) } else { format!("cp '{}' '{}'", saved.display(), exe.display()) };
    r.ok(&["run", "proj", "--", &copy]);
    assert_eq!(r.probe(), "MARK-t1", "the run really replaced the exe");
    assert_eq!(r.note().as_deref(), Some("run:projects/proj"));
    let o = r.ok(&["build", "proj"]);
    assert!(o.stdout.contains("cleaned `markprobe`"), "{}", o.stdout);
    assert_eq!(r.probe(), "MARK-main", "the exe was rebuilt, not the one the run left");
    assert_eq!(r.note().as_deref(), Some("projects/proj"));
}

/// If `cargo metadata` fails before a `run` (a broken Cargo.toml) the run still marks the folder, and what it builds is
/// never installable: a package name it can read gets a `run:` note; otherwise a marker file for that folder that the next plain build of that folder clears.
#[test]
fn a_run_in_a_folder_whose_packages_cannot_be_listed_still_marks_it() {
    for (broken, expect_note) in [("[package]
name = \"markprobe\"
version = 
", true), ("this is not toml at all
", false)] {
        let r = Root::new("runbroken", 120);
        r.ok(&["build", "proj"]);
        assert_eq!(r.note().as_deref(), Some("projects/proj"));
        let proj = r.dir.join("projects/proj");
        let good = r.scratch("Cargo.good");
        fs::copy(proj.join("Cargo.toml"), &good).unwrap();
        fs::write(proj.join("Cargo.toml"), broken).unwrap();
        let fix = if cfg!(windows) { format!("copy /Y \"{}\" Cargo.toml && cargo build --offline", good.display()) } else { format!("cp '{}' Cargo.toml && cargo build --offline", good.display()) };
        let o = r.ok(&["run", "proj", "--", &fix]);
        assert!(o.stdout.contains("not checked"), "{}", o.stdout);
        let marker = r.dir.join("state/build-sources/unlisted-run.d/projects%2Fproj");
        if expect_note {
            assert_eq!(r.note().as_deref(), Some("run:projects/proj"), "{}", o.stdout);
            assert!(!marker.exists());
        } else {
            assert!(marker.is_file(), "no name could be found, so the folder is marked: {}", o.stdout);
        }
        // a plain build clears whatever the run left and vouches again
        let o = r.ok(&["build", "proj"]);
        assert!(o.stdout.contains("cleaned `markprobe`"), "{}", o.stdout);
        assert!(!marker.exists());
        assert_eq!(r.note().as_deref(), Some("projects/proj"));
        assert_eq!(r.probe(), "MARK-main");
    }
}

/// The reviewer's scenario: a `run` in a task worktree whose Cargo.toml is broken repairs it and builds. Its marker is for THAT
/// folder, so a plain build of another folder (the main checkout, another project) must not lift it and `install` stays
/// refused; only a plain build of the worktree itself removes it.
#[test]
fn a_folder_marker_survives_plain_builds_of_other_folders() {
    let r = Root::new("markerscope", 120);
    r.ok(&["build", "proj"]);
    let t1 = r.task("src", "MARK-t1");
    let wt = r.worktree(&t1);
    let good = r.scratch("Cargo.good");
    fs::copy(wt.join("Cargo.toml"), &good).unwrap();
    fs::write(wt.join("Cargo.toml"), "this is not toml at all\n").unwrap();
    let fix = if cfg!(windows) { format!("copy /Y \"{}\" Cargo.toml && cargo build --offline", good.display()) } else { format!("cp '{}' Cargo.toml && cargo build --offline", good.display()) };
    let o = r.ok(&["run", "proj", "--task", &t1, "--", &fix]);
    assert!(o.stdout.contains("folder is marked"), "{}", o.stdout);
    let dir = r.dir.join("state/build-sources/unlisted-run.d");
    let names = |d: &Path| -> Vec<String> { fs::read_dir(d).map(|rd| rd.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default() };
    let wt_marker = format!("worktrees%2Fproj%2F{t1}");
    assert_eq!(names(&dir), std::slice::from_ref(&wt_marker));
    assert_eq!(fs::read_to_string(dir.join(&wt_marker)).unwrap().trim(), format!("worktrees/proj/{t1}"));
    let refused = |r: &Root| {
        let o = r.run(&["install"]);
        assert_eq!(o.code, 1, "{}{}", o.stdout, o.stderr);
        assert!(o.stderr.contains("folder marker") && o.stderr.contains(&t1), "{}", o.stderr);
    };
    refused(&r);
    // plain builds of the main checkout and of the other project, in between: the marker stays, install stays refused
    r.ok(&["build", "proj"]);
    r.ok(&["build", "other"]);
    assert_eq!(names(&dir), std::slice::from_ref(&wt_marker), "another folder's build must not remove it");
    refused(&r);
    // the worktree's own plain build cleans every package there, then removes only its marker
    let o = r.ok(&["build", "proj", "--task", &t1]);
    assert!(o.stdout.contains("cleaned `markprobe`"), "{}", o.stdout);
    assert!(names(&dir).is_empty(), "{:?}", names(&dir));
}

/// A broken root manifest whose `[workspace] members` can be read: the members' packages get `run:` notes too; a member whose
/// manifest cannot be read marks the folder.
#[test]
fn a_run_notes_workspace_member_packages_and_marks_the_folder_if_one_is_unreadable() {
    for (member_ok, want_marker) in [(true, false), (false, true)] {
        let r = Root::new("members", 120);
        let proj = r.dir.join("projects/proj");
        fs::create_dir_all(proj.join("crates/m1/src")).unwrap();
        fs::write(proj.join("crates/m1/Cargo.toml"), "[package]\nname = \"memberone\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[package.metadata.x]\nname = \"decoy\"\n").unwrap();
        fs::write(proj.join("crates/m1/src/lib.rs"), "").unwrap();
        let good = "[package]\nname = \"markprobe\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\nmembers = [\n  \"crates/m1\",\n]\n";
        git(&proj, &["add", "."]);
        git(&proj, &["commit", "-q", "-m", "member"]);
        fs::write(proj.join("Cargo.toml"), good).unwrap();
        if !member_ok {
            fs::write(proj.join("Cargo.toml"), good.replace("crates/m1", "crates/gone")).unwrap();
        }
        // the root is broken by a stray line after the members, so cargo metadata fails but the names are readable
        let broken = format!("{}\nversion = = =\n", fs::read_to_string(proj.join("Cargo.toml")).unwrap());
        fs::write(proj.join("Cargo.toml"), broken).unwrap();
        let o = r.ok(&["run", "proj", "--", "echo hi"]);
        assert!(o.stdout.contains("not checked"), "{}", o.stdout);
        let notes = r.dir.join("state/build-sources");
        assert_eq!(fs::read_to_string(notes.join("markprobe")).unwrap().trim(), "run:projects/proj", "{}", o.stdout);
        if member_ok {
            assert_eq!(fs::read_to_string(notes.join("memberone")).unwrap().trim(), "run:projects/proj", "{}", o.stdout);
            assert!(!notes.join("decoy").exists(), "[package.metadata.x] is not [package]");
        }
        assert_eq!(notes.join("unlisted-run.d/projects%2Fproj").exists(), want_marker, "{}", o.stdout);
    }
}

/// `run` holds the build lock for its whole command.
#[test]
fn run_holds_the_build_lock_while_it_runs() {
    let r = Root::new("runlock", 1);
    let flag = r.scratch("run-started.txt");
    let flag_s = flag.to_string_lossy().replace('\\', "/");
    let cmd: Vec<&str> = if cfg!(windows) { vec!["echo", "x>", &flag_s, "&&", "ping", "-n", "8", "127.0.0.1", ">nul"] } else { vec!["echo x >", &flag_s, "; sleep 8"] };
    let mut args = vec!["run", "proj", "--"];
    args.extend(cmd);
    let j = r.cmd(&args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    assert!(wait_for(&flag, "x", 60), "the run command never started");
    let o = r.run(&["build", "proj"]);
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert!(o.stderr.contains("build folder busy"), "{}", o.stderr);
    finish(j, 60);
    r.ok(&["build", "proj"]);
}

/// The `build_lock_wait_secs` of every job record of `r` whose status is `status` (None where the field is missing).
fn lock_waits(r: &Root, status: &str) -> Vec<Option<f64>> {
    r.job_records()
        .iter()
        .filter_map(|p| fs::read_to_string(p).ok())
        .filter(|t| t.lines().any(|l| l.trim() == format!("status = {status}")))
        .map(|t| t.lines().find_map(|l| l.strip_prefix("build_lock_wait_secs = ")).map(|v| v.trim().parse::<f64>().unwrap_or_else(|e| panic!("bad build_lock_wait_secs {v:?}: {e}"))))
        .collect()
}

/// Audit 2026-11-21 22Z fix 3: a `broadsword build` inside a `run` of the same root used to wait out the whole
/// `build_lock_wait_seconds` (here 120 s) for the lock its own parent holds. It must be refused at once with exit 3.
#[test]
fn nested_build_inside_run_is_refused_at_once() {
    let r = Root::new("nested", 120);
    let started = Instant::now();
    let o = r.run(&["run", "proj", "--", bin(), "build", "proj"]);
    let took = started.elapsed();
    let all = format!("{}\n{}", o.stdout, o.stderr);
    assert!(took < Duration::from_secs(60), "the nested build waited {took:?} (the lock wait is 120 s)\n{all}");
    assert_eq!(o.code, 3, "the run ends with the inner call's exit 3\n{all}");
    assert!(all.contains("runs inside") && all.contains("`broadsword run` job"), "{all}");
    let refused = lock_waits(&r, "refused");
    assert_eq!(refused.len(), 1, "one refused record, the inner build: {refused:?}");
    assert!(refused[0].is_some_and(|w| w < 60.0), "{refused:?}");
    // the run itself let go of the lock when its command ended: a plain build takes it
    r.ok(&["build", "proj"]);
}

/// A marker that does not name the lock's live holder (a stale one from a finished run, or another process) refuses nothing
/// sooner: with the lock free the build takes it; with the lock busy it waits the whole configured time, as before.
#[test]
fn stale_run_marker_does_not_refuse() {
    let r = Root::new("stalemark", 1);
    let o = r.cmd(&["build", "proj"]).env(RUN_MARKER, format!("20200101-000000-proj-run:{}", dead_pid())).output().unwrap();
    assert_eq!(o.status.code(), Some(0), "free lock, stale marker: the build runs\n{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(r.probe(), "MARK-main");
    let t1 = r.task("a", "MARK-t1");
    let (g1, e1) = (r.scratch("gate1"), r.scratch("err1.txt"));
    let j1 = r.spawn_gated(&["build", "proj", "--task", &t1], &g1, &e1);
    assert!(wait_for(&e1, "TEST HOOK ACTIVE", 60), "job 1 never reached the sync gate");
    // a live pid that is not the holder (this test process), and a malformed marker
    for marker in [format!("x-run:{}", std::process::id()), "garbage".to_string()] {
        let started = Instant::now();
        let o = r.cmd(&["build", "proj"]).env(RUN_MARKER, &marker).output().unwrap();
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        assert_eq!(o.status.code(), Some(3), "{marker}: {err}");
        assert!(err.contains("build_lock_wait_seconds") && !err.contains("runs inside"), "{marker}: the normal refusal after the full wait: {err}");
        assert!(started.elapsed() >= Duration::from_secs(1), "{marker}: it waited the configured time: {:?}", started.elapsed());
    }
    fs::write(&g1, "go").unwrap();
    assert_eq!(finish(j1, 300), 0);
}

/// Review q-20261121-220925: the lock changes hands to a job that has not (yet) written any owner record, while the previous
/// owner named by the marker is still alive and every owner record left behind still names it. That must refuse nothing
/// sooner. The test plants every leftover record a previous run owner could leave: `build.lock.owner` (`<pid> <start>`, the
/// record commit c8df5695 trusted) and `build.lock.run.id` (the marker, the record the fix reads), both naming THIS live
/// test process, while a gated build that writes neither holds the lock.
#[test]
fn stale_owner_record_of_a_live_previous_owner_does_not_refuse() {
    let r = Root::new("staleowner", 1); // build_lock_wait_seconds = 1
    let t1 = r.task("a", "MARK-t1");
    let (g1, e1) = (r.scratch("gate1"), r.scratch("err1.txt"));
    let j1 = r.spawn_gated(&["build", "proj", "--task", &t1], &g1, &e1);
    assert!(wait_for(&e1, "TEST HOOK ACTIVE", 60), "job 1 never reached the sync gate");
    let me = std::process::id();
    let start = sys::start_time(me).expect("this test process's start time");
    let marker = format!("20261121-000000-proj-run:{me}");
    fs::write(r.dir.join("state/build.lock.owner"), format!("{me} {start}\n")).unwrap();
    fs::write(r.dir.join("state/build.lock.run.id"), &marker).unwrap();
    let started = Instant::now();
    let o = r.cmd(&["build", "proj"]).env(RUN_MARKER, &marker).output().unwrap();
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    let took = started.elapsed();
    fs::write(&g1, "go").unwrap();
    assert_eq!(finish(j1, 300), 0);
    assert_eq!(o.status.code(), Some(3), "{err}");
    assert!(!err.contains("runs inside") && err.contains("build_lock_wait_seconds"), "a stale owner record refused at once: {err}");
    assert!(took >= Duration::from_secs(1), "it waited the configured time: {took:?}");
}

/// A job refused after the full lock wait records how long it waited.
#[test]
fn refused_job_records_its_lock_wait() {
    let r = Root::new("refusedwait", 1); // build_lock_wait_seconds = 1
    let t1 = r.task("a", "MARK-t1");
    let (g1, e1) = (r.scratch("gate1"), r.scratch("err1.txt"));
    let j1 = r.spawn_gated(&["build", "proj", "--task", &t1], &g1, &e1);
    assert!(wait_for(&e1, "TEST HOOK ACTIVE", 60), "job 1 never reached the sync gate");
    assert_eq!(r.run(&["build", "proj"]).code, 3);
    let refused = lock_waits(&r, "refused");
    fs::write(&g1, "go").unwrap();
    assert_eq!(finish(j1, 300), 0);
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused[0].is_some_and(|w| w >= 1.0), "build_lock_wait_secs present and at least the 1 s wait: {refused:?}");
}

/// Review q-20261121-222529: when a `run` cannot prove it holds the build lock (here the proof id cannot be written: a folder
/// stands at its path), it used to start its command anyway without the marker, so a nested build waited the whole lock wait.
/// Now the run fails before its command starts and says why: no run command ever runs without the fail-fast marker.
#[test]
fn run_that_cannot_prove_its_lock_never_starts_its_command() {
    let r = Root::new("noproof", 30);
    fs::create_dir_all(r.dir.join("state/build.lock.run.id")).unwrap();
    let flag = r.scratch("noproof-started.txt");
    let flag_s = flag.to_string_lossy().replace('\\', "/");
    let cmd: Vec<&str> = if cfg!(windows) { vec!["echo", "x>", &flag_s, "&&", bin(), "build", "proj"] } else { vec!["echo x >", &flag_s, ";", bin(), "build", "proj"] };
    let mut args = vec!["run", "proj", "--"];
    args.extend(cmd);
    let started = Instant::now();
    let o = r.run(&args);
    let took = started.elapsed();
    let all = format!("{}\n{}", o.stdout, o.stderr);
    assert!(took < Duration::from_secs(20), "the run took {took:?} (the lock wait is 30 s)\n{all}");
    assert_eq!(o.code, 1, "the run fails (EXIT_FAILED)\n{all}");
    assert!(all.contains("cannot prove this run holds the build lock"), "{all}");
    assert!(!flag.exists(), "the run's command never started\n{all}");
    assert_eq!(lock_waits(&r, "failed").len(), 1, "one failed record, the run itself");
    // the run let go of the lock: a plain build takes it
    r.ok(&["build", "proj"]);
}

fn dead_pid() -> u32 {
    let mut c = if cfg!(windows) { Command::new("cmd").args(["/C", "exit 0"]).spawn().unwrap() } else { Command::new("true").spawn().unwrap() };
    let pid = c.id();
    c.wait().unwrap();
    pid
}

/// Blocker 1: a `check` killed during its test step resumes at `test`. The resumed job must still take the build lock and
/// prepare (clean + note), or another checkout's build in between leaves the wrong binary under cargo's fresh fingerprint.
#[test]
fn a_check_resumed_at_the_test_step_prepares_the_build_folder_first() {
    let r = Root::new("resume", 120);
    let t1 = r.task("a", "MARK-t1");
    r.ok(&["check", "proj", "--task", &t1]); // t1's fingerprints are fresh
    r.ok(&["build", "proj"]); // another checkout builds in between: main's binary, note main
    assert_eq!(r.probe(), "MARK-main");
    let dead = dead_pid().to_string();
    let dir = r.dir.join("state/jobs");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("20260101-000001-proj-check.rec"),
        format!("id = 20260101-000001-proj-check
project = proj
kind = check
status = running
pid = {dead}
step = test
task = {t1}
"),
    )
    .unwrap();
    let o = r.ok(&["recover"]);
    let out = format!("{}{}", o.stdout, o.stderr);
    assert!(out.contains("resuming `check proj` from step `test`"), "{out}");
    assert!(o.stdout.contains("cleaned `markprobe`"), "the resumed job prepared the folder: {out}");
    assert!(o.stdout.contains("test: ok"), "the resumed test ran against its own source: {out}");
    assert_eq!(r.probe(), "MARK-t1");
    assert_eq!(r.note().as_deref(), Some(format!("worktrees/proj/{t1}").as_str()));
}

// ---- refusals ---------------------------------------------------------------------------

#[test]
fn a_task_that_may_not_be_used_is_refused_and_nothing_runs() {
    let r = Root::new("refuse", 120);
    let good = r.task("a", "MARK-t1");
    let before = r.job_records().len();
    let refused = |args: &[&str], code: i32, needle: &str| {
        let o = r.run(args);
        assert_eq!(o.code, code, "{args:?}: {}", o.stderr);
        assert!(o.stderr.contains(needle), "{args:?}: wanted `{needle}` in:\n{}", o.stderr);
        assert!(!o.stdout.contains("job:"), "{args:?}: no job started");
    };
    // wrong project
    refused(&["build", "other", "--task", &good], 3, "belongs to project `proj`, not `other`");
    refused(&["check", "OTHER", "--task", &good], 3, "belongs to project `proj`");
    // no such task: exit 2 like an unknown project
    refused(&["build", "proj", "--task", "t-nope"], 2, "no task with the exact id");
    refused(&["test", "proj", "--task", "../x"], 2, "no task with the exact id");
    // open (never started)
    let open = r.ok(&["task", "add", "proj", "--title", "o", "--claim", "b"]).stdout.trim().to_string();
    refused(&["build", "proj", "--task", &open], 3, "it is `open`");
    // running but without a worktree
    let plain = r.ok(&["task", "add", "proj", "--title", "p", "--claim", "c"]).stdout.trim().to_string();
    r.ok(&["task", "start", &plain, "--owner", "coder"]);
    refused(&["build", "proj", "--task", &plain], 3, "no worktree");
    // not running any more: failed keeps its worktree folder, but the task no longer owns it
    let failed = r.task("d", "MARK-t2");
    r.ok(&["task", "fail", &failed]);
    assert!(r.worktree(&failed).is_dir());
    refused(&["build", "proj", "--task", &failed], 3, "it is `failed`");
    // a stray folder in place of the worktree: not a git worktree of the project (the folder is inside the enclosing
    // repository that holds the test scratch area, which is exactly the case git would otherwise answer for)
    let stray = r.task("e", "MARK-t3");
    fs::remove_dir_all(r.worktree(&stray)).unwrap();
    fs::create_dir_all(r.worktree(&stray).join("src")).unwrap();
    fs::copy(r.dir.join("projects/proj/Cargo.toml"), r.worktree(&stray).join("Cargo.toml")).unwrap();
    set_marker(&r.worktree(&stray), "MARK-stray");
    refused(&["build", "proj", "--task", &stray], 3, "not a git worktree of project `proj`");
    refused(&["run", "proj", "--task", &stray, "--", "echo", "hi"], 3, "not a git worktree of project `proj`");
    // the worktree folder is gone altogether
    let gone = r.task("f", "MARK-t4");
    fs::remove_dir_all(r.worktree(&gone)).unwrap();
    refused(&["build", "proj", "--task", &gone], 3, "is gone");
    // nothing above created a job record, a build folder, a note or a clean
    assert_eq!(r.job_records().len(), before);
    assert!(!r.dir.join("build").exists(), "nothing was built");
    assert_eq!(r.note(), None);
    // while the good one still works
    r.ok(&["build", "proj", "--task", &good]);
    assert_eq!(r.probe(), "MARK-t1");
}

#[test]
fn a_flag_without_its_value_is_a_usage_error() {
    let r = Root::new("usage", 120);
    for args in [&["build", "proj", "--task"][..], &["run", "proj", "--task", "--", "x"], &["check", "--task", "t", "proj"]] {
        let o = r.run(args);
        assert_eq!(o.code, 64, "{args:?}: {}", o.stderr);
    }
}

// ---- the MCP path -----------------------------------------------------------------------

#[test]
fn mcp_check_with_task_builds_and_tests_the_worktree() {
    use std::io::{BufRead, BufReader, Write};
    let r = Root::new("mcp", 120);
    let t1 = r.task("a", "MARK-t1");
    r.ok(&["build", "proj"]); // main first, so the call below has to clean
    let mut child = Command::new(bin()).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut out = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"check","arguments":{{"project":"proj","task":"{t1}"}}}}}}"#).unwrap();
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    assert!(line.contains(r#""isError":false"#), "{line}");
    assert!(line.contains(&format!("source: worktrees/proj/{t1} (task {t1})")) && line.contains("test: ok"), "{line}");
    assert_eq!(r.probe(), "MARK-t1");
    // a refused task is an MCP error that carries the reason and the exit code
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"build","arguments":{{"project":"other","task":"{t1}"}}}}}}"#).unwrap();
    line.clear();
    out.read_line(&mut line).unwrap();
    assert!(line.contains(r#""isError":true"#) && line.contains("belongs to project") && line.contains("exit code: 3"), "{line}");
    // the tool list documents the input
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":3,"method":"tools/list"}}"#).unwrap();
    line.clear();
    out.read_line(&mut line).unwrap();
    assert!(line.contains(r#""task":{"type":"string""#) || line.contains(r#""task":{"#), "{line}");
    drop(stdin);
    child.wait().unwrap();
}

// ---- the lock ---------------------------------------------------------------------------

/// Job 1 (task t1, whose package note names t2, so it must clean) pauses at the sync gate: it has DECIDED to clean and
/// has not yet cleaned or built. Job 2 (task t2, whose note matches, so it needs no clean) starts meanwhile. With the
/// lock, job 2 cannot even read the note until job 1 has finished COMPILING (a test job releases the lock once its test
/// executables are copied out, and runs them from the copies); then it sees t1's note, cleans and builds its own code.
/// Without it, job 2 decides "no clean" off the stale note, job 1 cleans and builds t1 into the folder, and job 2's
/// cargo reports `Finished` on its own still-fresh build and leaves t1's binary in place: its tests (which run the
/// binary) fail. Each job's tests assert that the binary is its own checkout's (they read the run-time
/// `CARGO_BIN_EXE_markprobe`, which is the job's COPY). The shared folder's binary after the jobs is no longer asserted: once
/// job 1 has copied its executables job 2 may clean and rebuild, and job 1's doc-tests take the lock again.
#[test]
fn two_checkouts_building_at_once_each_get_their_own_binary() {
    let r = Root::new("race", 120);
    let t1 = r.task("a", "MARK-t1");
    let t2 = r.task("b", "MARK-t2");
    r.ok(&["build", "proj", "--task", &t2]); // the folder now holds t2's binary and the note says t2
    assert_eq!(r.probe(), "MARK-t2");
    let (g1, g2, e1, e2) = (r.scratch("gate1"), r.scratch("gate2"), r.scratch("err1.txt"), r.scratch("err2.txt"));
    let j1 = r.spawn_gated(&["test", "proj", "--task", &t1], &g1, &e1);
    assert!(wait_for(&e1, "TEST HOOK ACTIVE", 60), "job 1 never reached the sync gate:\n{}", fs::read_to_string(&e1).unwrap_or_default());
    let j2 = r.spawn_gated(&["test", "proj", "--task", &t2], &g2, &e2);
    // Unlocked, job 2 reaches its own gate within moments (it never waits for anything). Locked, it never does until job 1
    // has ended. Either way the test goes on after this wait; only the outcome below tells the two apart.
    let reached = wait_for(&e2, "TEST HOOK ACTIVE", 6);
    eprintln!("job 2 reached the sync gate while job 1 held it: {reached}");
    // open both gates: job 1's doc-tests need the lock again after its copies run, and job 2 holds it at its own gate
    fs::write(&g1, "go").unwrap();
    fs::write(&g2, "go").unwrap();
    assert_eq!(finish(j1, 300), 0, "job 1 (t1) failed:\n{}", fs::read_to_string(&e1).unwrap_or_default());
    assert_eq!(finish(j2, 300), 0, "job 2 (t2) got a binary that is not its own:\n{}", fs::read_to_string(&e2).unwrap_or_default());
    assert!(!reached, "with the lock job 2 must not get past reading the note while job 1 holds it");
}

#[test]
fn a_second_job_is_refused_with_exit_3_when_the_build_lock_stays_busy() {
    let r = Root::new("busy", 1); // build_lock_wait_seconds = 1
    let t1 = r.task("a", "MARK-t1");
    let (g1, e1) = (r.scratch("gate1"), r.scratch("err1.txt"));
    let j1 = r.spawn_gated(&["build", "proj", "--task", &t1], &g1, &e1);
    assert!(wait_for(&e1, "TEST HOOK ACTIVE", 60), "job 1 never reached the sync gate");
    let started = Instant::now();
    let o = r.run(&["build", "proj"]);
    assert_eq!(o.code, 3, "{}", o.stderr);
    assert!(o.stderr.contains("build folder busy") && o.stderr.contains("build_lock_wait_seconds") && o.stderr.contains("nothing was built, cleaned or recorded"), "{}", o.stderr);
    assert!(started.elapsed() >= Duration::from_secs(1) && started.elapsed() < Duration::from_secs(20), "it waited the configured time, not forever: {:?}", started.elapsed());
    // ids sort by name and two jobs of one second share a stem, so look for the refused record instead of taking the last one
    let recs: Vec<String> = r.job_records().iter().map(|p| fs::read_to_string(p).unwrap()).collect();
    assert!(recs.iter().any(|rec| rec.contains("status = refused")), "{recs:?}");
    assert!(!r.dir.join("build").exists() || r.note().is_none(), "the refused job changed nothing");
    // the same through check and test
    for k in ["test", "check"] {
        assert_eq!(r.run(&[k, "proj"]).code, 3);
    }
    fs::write(&g1, "go").unwrap();
    assert_eq!(finish(j1, 300), 0);
    // lock released: the next job runs
    r.ok(&["build", "proj"]);
    assert_eq!(r.probe(), "MARK-main");
}

#[test]
fn a_clean_that_fails_stops_the_job_before_building_and_keeps_the_old_note() {
    let r = Root::new("cleanfail", 120);
    let t1 = r.task("a", "MARK-t1");
    r.ok(&["build", "proj"]);
    // a cargo that fails to clean: the job must stop (exit 1), say so, build nothing, and leave the note alone
    let fake = r.dir.join(if cfg!(windows) { "fakecargo.cmd" } else { "fakecargo.sh" });
    fs::write(&fake, if cfg!(windows) { "@echo off\r\nif \"%1\"==\"clean\" (echo access denied 1>&2 & exit /b 1)\r\ncargo %*\r\n" } else { "#!/bin/sh\nif [ \"$1\" = clean ]; then echo denied >&2; exit 1; fi\nexec cargo \"$@\"\n" }).unwrap();
    let o = r.cmd(&["build", "proj", "--task", &t1]).env("CARGO", &fake).output().unwrap();
    assert_eq!(o.status.code(), Some(1), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("cargo clean -p markprobe") && err.contains("nothing was built"), "{err}");
    assert_eq!(r.note().as_deref(), Some("projects/proj"), "the note still names the folder whose binary is there");
    assert_eq!(r.probe(), "MARK-main");
}
