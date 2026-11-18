//! End-to-end tests for `--task` on build/test/check/run and for the shared-build-folder rules (state/build-sources
//! notes, `cargo clean -p`, state/build.lock). Every test makes a throwaway Broadsword root under the repo's `scratch/`
//! with a real Git repo and a tiny cargo project under `projects/`, and runs the real binary there. Never the real repo,
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

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");
const SYNC_GATE: &str = "BROADSWORD_INTERNAL_TEST_BUILD_SYNC_GATE_FILE";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

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
            "#[test]\nfn the_binary_is_this_checkouts_binary() {\n    let o = std::process::Command::new(env!(\"CARGO_BIN_EXE_markprobe\")).output().unwrap();\n    let printed = String::from_utf8_lossy(&o.stdout).into_owned();\n    let source = std::fs::read_to_string(\"src/lib.rs\").unwrap();\n    assert!(source.contains(&format!(\"\\\"{printed}\\\"\")), \"binary prints {printed}, this checkout says {source}\");\n}\n",
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
        let toml = format!(
            "root = \".\"\ndisk_floor_gb = 1\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\ndisk_check_every_seconds = 60\nlog_max_mb = 50\n\
             ram_warn_percent = 100\ncpu_warn_percent = 90\nalert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
             state_dir = \"state\"\ninstall_dir = \"build/bin\"\nmcp_call_timeout_seconds = 1800\nmcp_output_max_kb = 1024\n\
             checkpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 0\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = {build_lock_wait}\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
light_check_paths = \"docs/,*.md\"
light_check_timeout_seconds = 300
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50\n"
        );
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args).current_dir(&self.dir).env_remove(SYNC_GATE).stdin(Stdio::null());
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
    assert_eq!(names(&dir), [wt_marker.clone()]);
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
    assert_eq!(names(&dir), [wt_marker.clone()], "another folder's build must not remove it");
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
    let mut child = Command::new(BIN).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
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
/// lock, job 2 cannot even read the note until job 1 is finished; then it sees t1's note, cleans and builds its own code.
/// Without it, job 2 decides "no clean" off the stale note, job 1 cleans and builds t1 into the folder, and job 2's
/// cargo reports `Finished` on its own still-fresh build and leaves t1's binary in place: its tests (which run the
/// binary) fail. Each job's tests assert that the binary is its own checkout's.
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
    fs::write(&g1, "go").unwrap();
    assert_eq!(finish(j1, 300), 0, "job 1 (t1) failed:\n{}", fs::read_to_string(&e1).unwrap_or_default());
    assert_eq!(r.probe(), "MARK-t1", "job 1's binary is its own");
    fs::write(&g2, "go").unwrap();
    assert_eq!(finish(j2, 300), 0, "job 2 (t2) got a binary that is not its own:\n{}", fs::read_to_string(&e2).unwrap_or_default());
    // the last job to run was t2's, so the folder holds t2's binary (a stale one would still print MARK-t1) and the note agrees
    assert_eq!(r.probe(), "MARK-t2", "job 2's binary is its own");
    assert!(!reached, "with the lock job 2 must not get past reading the note while job 1 holds it");
    assert_eq!(r.note().as_deref(), Some(format!("worktrees/proj/{t2}").as_str()));
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
