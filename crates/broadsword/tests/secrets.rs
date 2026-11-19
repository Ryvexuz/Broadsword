//! Secret files through the real binary: which names are secrets (`git::is_secret_path`: `.env`, `.envrc`, `.env.<name>`,
//! `.env-<name>`, `.env_<name>`),
//! and that a secret someone staged never becomes reachable from a `refs/broadsword/*` index ref. Each test makes a
//! throwaway Broadsword root under the repo's `scratch/` folder with a throwaway Git repo as its project; the secret files
//! are fake. Everything is deleted afterwards. Never touches the real repo, its `.env` or the crash sim.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_broadsword");

// The product's own process-identity code, so the test cleanup applies the very same exact-match rule.
#[allow(dead_code)]
#[path = "../src/sys.rs"]
mod sys;

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn git_out(dir: &Path, args: &[&str]) -> Output {
    Command::new("git").args(args).current_dir(dir).env_remove("GIT_DIR").env_remove("GIT_INDEX_FILE").output().expect("git must be installed")
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = git_out(dir, args);
    assert!(out.status.success(), "git {args:?}: {}", text(&out.stderr));
    text(&out.stdout).trim().to_string()
}

fn wait_until(what: &str, secs: u64, mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < end {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    panic!("timed out waiting for {what}");
}

fn kill_hard(pid: u32) {
    // No /T: only this process dies, like a crash. Its child is left running.
    let st = Command::new("taskkill").args(["/PID", &pid.to_string(), "/F"]).output().expect("taskkill");
    assert!(st.status.success(), "taskkill: {}", text(&st.stderr));
}

struct Root {
    dir: PathBuf,
    children: Vec<Child>,
}

impl Root {
    fn new(name: &str) -> Root {
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch");
        let dir = scratch.join(format!("sec-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        fs::create_dir_all(dir.join("projects")).unwrap();
        let toml = "root = \".\"\ndisk_floor_gb = 1\ncleanup_every_hours = 1\nscratch_max_age_hours = 24\n\
                    disk_check_every_seconds = 60\nlog_max_mb = 50\nram_warn_percent = 100\ncpu_warn_percent = 90\n\
                    alert_command = \"\"\nscratch_dir = \"scratch\"\nbuild_dir = \"build\"\nprojects_dir = \"projects\"\n\
                    state_dir = \"state\"\ninstall_dir = \"build/bin\"\nmcp_call_timeout_seconds = 1800\nmcp_output_max_kb = 1024\ncheckpoint_ignored_max_mb = 50\nmax_resumes = 3\njob_disk_reserve_gb = 0\njob_disk_poll_seconds = 1\ntask_lock_wait_seconds = 10\nworktrees_dir = \"worktrees\"\nbuild_lock_wait_seconds = 10\nrecover_lock_wait_seconds = 5
sweep_older_than_days = 7
redact_min_chars = 6
reviewer_backend = \"claude\"\nreviewer_cli = \"claude\"\nreviewer_model_low = \"haiku\"\nreviewer_model_high = \"opus\"\nreviewer_timeout_seconds = 5\nreviewer_diff_max_kb = 200\nreviewer_brief_max_kb = 32\nreviewer_check_tail_kb = 8\nqueue_lock_wait_seconds = 2
reviewer_max_calls_per_entry = 3
shadow_reviewer = \"off\"
shadow_reviewer_model = \"gpt-5.6-sol\"
shadow_reviewer_timeout_seconds = 5
max_agents = 50
";
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir, children: Vec::new() }
    }

    fn project_dir(&self) -> PathBuf {
        self.dir.join("projects/p")
    }

    /// Project `p`: a tiny lib, `tracked.txt` = original, one commit. The secret names are ignored, the look-alikes are not.
    fn project(&self) -> PathBuf {
        let p = self.project_dir();
        fs::create_dir_all(p.join("src")).unwrap();
        fs::write(p.join("Cargo.toml"), "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
        fs::write(p.join("src/lib.rs"), "").unwrap();
        fs::write(p.join("tracked.txt"), "original\n").unwrap();
        fs::write(p.join(".gitignore"), ".env\n.envrc\n.env.*\n.env-*\n.env_*\n*.log\n").unwrap();
        for a in [&["init", "-q"][..], &["config", "user.email", "t@t"], &["config", "user.name", "t"], &["config", "core.autocrlf", "false"], &["add", "-A"], &["commit", "-q", "-m", "init"]] {
            git(&p, a);
        }
        p
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(BIN);
        c.args(args).current_dir(&self.dir).stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Output {
        self.cmd(args).output().expect("failed to start broadsword")
    }

    /// Start broadsword in the background (stdout/stderr discarded).
    fn spawn(&mut self, args: &[&str]) -> u32 {
        let child = self.cmd(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        let pid = child.id();
        self.children.push(child);
        pid
    }

    fn records(&self) -> Vec<String> {
        let mut v: Vec<(String, String)> = fs::read_dir(self.dir.join("state/jobs"))
            .map(|rd| rd.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "rec")).map(|e| (e.file_name().to_string_lossy().into_owned(), fs::read_to_string(e.path()).unwrap())).collect())
            .unwrap_or_default();
        v.sort();
        v.into_iter().map(|(_, r)| r).collect()
    }

    fn field(rec: &str, key: &str) -> String {
        rec.lines().find_map(|l| l.strip_prefix(&format!("{key} = "))).unwrap_or("").to_string()
    }

    fn only_record(&self) -> String {
        let r = self.records();
        assert_eq!(r.len(), 1, "{r:?}");
        r[0].clone()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &c.id().to_string()]).output();
            let _ = c.kill();
            let _ = c.wait();
        }
        // an orphan recorded by a crashed job, killed only if it is verifiably the recorded process
        for rec in self.records() {
            let (pid, start) = (Root::field(&rec, "child_pid"), Root::field(&rec, "child_start"));
            if let Ok(pid) = pid.parse::<u32>() {
                if sys::exact_match(start.parse::<u64>().ok(), sys::process_state(pid)) {
                    let _ = Command::new("taskkill").args(["/T", "/F", "/PID", &pid.to_string()]).output();
                }
            }
        }
        std::thread::sleep(Duration::from_millis(300));
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Paths in a tree-ish, recursively.
fn tree_paths(p: &Path, r: &str) -> Vec<String> {
    git(p, &["ls-tree", "-r", "--name-only", "--full-tree", r]).lines().map(str::to_string).collect()
}

/// No object reachable from any `refs/broadsword/*` ref has this id. (A blob someone `git add`ed exists in the object store
/// by then; what matters is that no Broadsword ref keeps it.)
fn assert_unreachable_from_broadsword_refs(p: &Path, hash: &str) {
    let refs = git(p, &["for-each-ref", "--format=%(refname)", "refs/broadsword/"]);
    let mut args = vec!["rev-list", "--objects"];
    args.extend(refs.lines());
    assert!(args.len() > 2, "there are broadsword refs to check");
    let objs = git(p, &args);
    assert!(!objs.lines().any(|l| l.starts_with(hash)), "blob {hash} of a secret file is reachable from a broadsword ref");
}

/// The done check: a staged `.env` and `.envrc` are absent from `refs/broadsword/index/<id>`, a secret the killed job staged
/// (`.env-prod`) is absent from the saved interrupted index, a secret the job changed (`.envrc`) keeps the job's bytes, and
/// `.envoy.yaml` / `.environment` (ordinary files) are checkpointed and restored.
#[test]
fn staged_env_is_never_in_an_index_ref_and_lookalike_names_roll_back() {
    let mut r = Root::new("idx");
    let p = r.project();
    fs::write(p.join(".env"), "API_KEY=sk-fake-staged-before-the-job\n").unwrap();
    git(&p, &["add", "-f", ".env"]);
    let env_blob = git(&p, &["rev-parse", ":.env"]);
    fs::write(p.join(".envrc"), "export API_KEY=sk-fake-direnv-before-the-job\n").unwrap();
    git(&p, &["add", "-f", ".envrc"]);
    let envrc_blob = git(&p, &["rev-parse", ":.envrc"]);
    fs::write(p.join(".envoy.yaml"), "envoy: original\n").unwrap();
    fs::write(p.join(".environment"), "ENVIRONMENT=original\n").unwrap();
    let line = if cfg!(windows) {
        "echo envoy: job> .envoy.yaml & del .environment & echo DIRENV=job> .envrc & echo API_KEY=sk-fake-staged-by-the-job> .env-prod & git add -f .env-prod & ping -n 30 127.0.0.1 >nul"
    } else {
        "echo 'envoy: job' > .envoy.yaml; rm .environment; echo DIRENV=job > .envrc; echo API_KEY=sk-fake-staged-by-the-job > .env-prod; git add -f .env-prod; sleep 30"
    };
    let bs_pid = r.spawn(&["run", "p", "--", line]);
    wait_until("the job to have changed the look-alikes and .envrc and staged .env-prod", 90, || {
        let recs = r.records();
        recs.len() == 1
            && !Root::field(&recs[0], "child_pid").is_empty()
            && !p.join(".environment").exists()
            && fs::read_to_string(p.join(".envoy.yaml")).is_ok_and(|s| s.contains("job"))
            && fs::read_to_string(p.join(".envrc")).is_ok_and(|s| s.contains("DIRENV=job"))
            && git_out(&p, &["ls-files", "--", ".env-prod"]).stdout.starts_with(b".env-prod")
    });
    let rec = r.only_record();
    let id = Root::field(&rec, "id");
    assert_eq!(Root::field(&rec, "secret_skipped"), ".env|.envrc", "{rec}");
    let index_ref = format!("refs/broadsword/index/{id}");
    let in_index = tree_paths(&p, &index_ref);
    assert!(!in_index.iter().any(|f| f == ".env" || f == ".envrc"), "the staged .env and .envrc are not in {index_ref}: {in_index:?}");
    assert!(in_index.iter().any(|f| f == "tracked.txt"), "{in_index:?}");
    let ckpt = format!("refs/broadsword/checkpoints/{id}");
    assert_eq!(git(&p, &["show", &format!("{ckpt}:.envoy.yaml")]), "envoy: original", ".envoy.yaml is checkpointed");
    assert_eq!(git(&p, &["show", &format!("{ckpt}:.environment")]), "ENVIRONMENT=original", ".environment is checkpointed");
    let prod_blob = git(&p, &["rev-parse", ":.env-prod"]);

    kill_hard(bs_pid);
    let out = r.run(&["recover", &id]);
    let so = text(&out.stdout);
    println!("{so}{}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert_eq!(fs::read(p.join(".envoy.yaml")).unwrap(), b"envoy: original\n", ".envoy.yaml is restored byte for byte");
    assert_eq!(fs::read(p.join(".environment")).unwrap(), b"ENVIRONMENT=original\n", ".environment is restored");
    assert!(fs::read_to_string(p.join(".env-prod")).unwrap().contains("staged-by-the-job"), "secret files are never touched");
    assert!(fs::read_to_string(p.join(".envrc")).unwrap().contains("DIRENV=job"), ".envrc is a secret: never restored");
    let int_index = format!("refs/broadsword/interrupted-index/{id}/1");
    let in_int = tree_paths(&p, &int_index);
    assert!(!in_int.iter().any(|f| f == ".env" || f == ".envrc" || f == ".env-prod"), "no secret in {int_index}: {in_int:?}");
    for h in [&env_blob, &envrc_blob, &prod_blob] {
        assert_unreachable_from_broadsword_refs(&p, h);
    }
    assert_eq!(Root::field(&r.only_record(), "status"), "rolled_back");
    // (the job's own command line is in the record, so only the content of the file staged before the job is checked)
    for s in ["staged-before-the-job", "direnv-before-the-job"] {
        assert!(!so.contains(s) && !rec.contains(s), "no secret content in output or record");
    }
    r.children.clear();
}

/// The 8.3 short name the file system gave the existing file `p`, or None when it has none (8.3 names are disabled on the
/// volume, or the long name already is 8.3).
#[cfg(windows)]
fn short_name(p: &Path) -> Option<String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buf = vec![0u16; 1024];
    // SAFETY: `wide` is NUL-terminated and `buf` holds 1024 u16s; the call writes at most that many.
    let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    assert!(n > 0 && n < buf.len(), "GetShortPathNameW failed for {}", p.display());
    buf.truncate(n);
    let short = OsString::from_wide(&buf).to_string_lossy().into_owned();
    let name = short.rsplit(['/', '\\']).next().unwrap().to_string();
    let long = p.file_name().unwrap().to_string_lossy().into_owned();
    (!name.eq_ignore_ascii_case(&long)).then_some(name)
}

/// Say loudly that a test did not run its checks. Written straight to stderr, which the test harness does not capture, so
/// the line shows in the job log of every run.
fn say_skipped(test: &str, why: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "SKIPPED {test}: {why}");
}

/// Review finding (NTFS 8.3 aliases): `ENV~1` is another name for `.env` and `ENV~1.LOC` for `.env.local`. Staged under
/// those names, the secret bytes must still never reach a `refs/broadsword/*` ref, and rollback must not write through the
/// alias into `.env`. `ENVOY~1.YAM`, the alias of the ordinary `.envoy.yaml`, stays an ordinary file.
#[cfg(windows)]
#[test]
fn an_8dot3_alias_of_a_secret_file_is_a_secret() {
    let mut r = Root::new("alias");
    let p = r.project();
    fs::write(p.join(".env"), "API_KEY=sk-fake-alias-env-original\n").unwrap();
    fs::write(p.join(".env.local"), "API_KEY=sk-fake-alias-local\n").unwrap();
    fs::write(p.join(".envoy.yaml"), "envoy: original\n").unwrap();
    let (Some(s_env), Some(s_local), Some(s_envoy)) = (short_name(&p.join(".env")), short_name(&p.join(".env.local")), short_name(&p.join(".envoy.yaml"))) else {
        say_skipped("an_8dot3_alias_of_a_secret_file_is_a_secret", &format!("the volume holding {} gives files no 8.3 short names (fsutil 8dot3name), so there is no alias to test", p.display()));
        return;
    };
    println!("aliases: .env = {s_env}, .env.local = {s_local}, .envoy.yaml = {s_envoy}");
    git(&p, &["update-index", "--add", &s_env, &s_local, &s_envoy]);
    let blobs: Vec<String> = [".env", ".env.local"].iter().map(|f| git(&p, &["hash-object", "--no-filters", f])).collect();
    let line = "echo API_KEY=sk-fake-alias-env-job> .env & echo changed> tracked.txt & ping -n 30 127.0.0.1 >nul";
    let bs_pid = r.spawn(&["run", "p", "--", line]);
    wait_until("the job to have changed .env", 90, || {
        let recs = r.records();
        recs.len() == 1 && !Root::field(&recs[0], "child_pid").is_empty() && fs::read_to_string(p.join(".env")).is_ok_and(|s| s.contains("env-job"))
    });
    let rec = r.only_record();
    let id = Root::field(&rec, "id");
    for s in [&s_env, &s_local] {
        assert!(Root::field(&rec, "secret_skipped").split('|').any(|f| f == s.as_str()), "{s} is listed as a secret: {rec}");
    }
    let (ckpt, index_ref) = (format!("refs/broadsword/checkpoints/{id}"), format!("refs/broadsword/index/{id}"));
    for rf in [&ckpt, &index_ref] {
        let paths = tree_paths(&p, rf);
        assert!(!paths.iter().any(|f| f == &s_env || f == &s_local), "no alias of a secret in {rf}: {paths:?}");
        assert!(paths.iter().any(|f| f == &s_envoy), "the alias of an ordinary file is kept in {rf}: {paths:?}");
    }
    kill_hard(bs_pid);
    let out = r.run(&["recover", &id]);
    let so = text(&out.stdout);
    println!("{so}{}", text(&out.stderr));
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    assert_eq!(fs::read_to_string(p.join("tracked.txt")).unwrap(), "original\n", "ordinary files roll back");
    let env_now = fs::read_to_string(p.join(".env")).unwrap();
    assert!(env_now.starts_with("API_KEY=sk-fake-alias-env-job"), "rollback never writes into .env through its alias: {env_now:?}");
    assert_eq!(fs::read_to_string(p.join(".env.local")).unwrap(), "API_KEY=sk-fake-alias-local\n");
    for rf in [format!("refs/broadsword/interrupted/{id}/1"), format!("refs/broadsword/interrupted-index/{id}/1")] {
        let paths = tree_paths(&p, &rf);
        assert!(!paths.iter().any(|f| f == &s_env || f == &s_local), "no alias of a secret in {rf}: {paths:?}");
    }
    let job_blob = git(&p, &["hash-object", "--no-filters", ".env"]);
    for h in blobs.iter().chain([&job_blob]) {
        assert_unreachable_from_broadsword_refs(&p, h);
    }
    r.children.clear();
}

/// Review finding (NTFS streams): `.env:s` is a stream of `.env`. Staged under that name (Git allows it with
/// core.protectNTFS off), its bytes must never reach a `refs/broadsword/*` ref.
#[cfg(windows)]
#[test]
fn a_stream_of_a_secret_file_is_a_secret() {
    let r = Root::new("stream");
    let p = r.project();
    git(&p, &["config", "core.protectNTFS", "false"]);
    fs::write(p.join(".env"), "API_KEY=sk-fake-plain\n").unwrap();
    if fs::write(p.join(".env:s"), "API_KEY=sk-fake-in-a-stream\n").is_err() {
        say_skipped("a_stream_of_a_secret_file_is_a_secret", &format!("the volume holding {} has no NTFS streams", p.display()));
        return;
    }
    assert_eq!(fs::read_to_string(p.join(".env:s")).unwrap(), "API_KEY=sk-fake-in-a-stream\n", "a real stream, not a file");
    let blob = git(&p, &["hash-object", "--no-filters", ".env:s"]);
    let staged = git(&p, &["hash-object", "-w", "--no-filters", ".env:s"]);
    assert_eq!(staged, blob);
    git(&p, &["update-index", "--add", "--cacheinfo", &format!("100644,{blob},.env:s")]);
    let out = r.run(&["run", "p", "--", "echo hi"]);
    let so = text(&out.stdout);
    assert_eq!(out.status.code(), Some(0), "{so}{}", text(&out.stderr));
    let rec = r.only_record();
    let id = Root::field(&rec, "id");
    assert!(Root::field(&rec, "secret_skipped").split('|').any(|f| f == ".env:s"), "{rec}");
    for rf in [format!("refs/broadsword/checkpoints/{id}"), format!("refs/broadsword/index/{id}")] {
        let paths = tree_paths(&p, &rf);
        assert!(!paths.iter().any(|f| f.contains(':')), "no stream in {rf}: {paths:?}");
    }
    assert_unreachable_from_broadsword_refs(&p, &blob);
}
