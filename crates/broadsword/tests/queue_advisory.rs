//! Real-binary queue tests with a scratch-only native reviewer. The tiny fixture is built through
//! Broadsword, copied into scratch/reviewer, and never calls a model or the network.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

mod support;

const REVIEWER: &str = r#"
use std::io::{Read, Write};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model = args.windows(2).find(|a| a[0] == "--model").unwrap()[1].clone();
    let advisory = model == "sonnet";
    let out = std::path::PathBuf::from(std::env::var("ADVISORY_FIXTURE_OUT").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).unwrap();
    std::fs::write(out.join(format!("{model}.prompt")), prompt).unwrap();
    std::fs::write(out.join(format!("{model}.args")), args.join("\n")).unwrap();
    let mut calls = std::fs::OpenOptions::new().create(true).append(true).open(out.join("calls")).unwrap();
    writeln!(calls, "{model}").unwrap();
    if advisory {
        let entry = std::fs::read_to_string(std::env::var("ADVISORY_FIXTURE_ENTRY").unwrap()).unwrap();
        assert!(entry.lines().any(|l| l == "verdict = PASS" || l == "verdict = FAIL"),
            "the gating verdict must already be durable");
        assert!(entry.contains("advisory_file = "), "the call must be reserved");
    }
    let kind = if model == "opus" { "pass".into() } else { std::env::var(if advisory { "ADVISORY_FIXTURE_KIND" } else { "GATE_FIXTURE_KIND" }).unwrap() };
    if kind == "timeout" {
        // Longer than the test root's configured reviewer timeout.
        let timeout: u64 = std::env::var("ADVISORY_FIXTURE_TIMEOUT").unwrap().parse().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(timeout.saturating_add(timeout)));
    }
    if kind == "error" {
        eprintln!("advisory fixture CLI failure");
        std::process::exit(7);
    }
    let text = match kind.as_str() {
        "fail" => "VERDICT: FAIL\\nBLOCKING: src/note.txt:1 - advisory fixture finding\\nMINOR: none",
        "malformed" => "not a verdict",
        _ => "VERDICT: PASS\\nBLOCKING: none\\nMINOR: none",
    };
    println!("{{\"type\":\"result\",\"is_error\":false,\"result\":\"{text}\",\"total_cost_usd\":0.0123,\"modelUsage\":{{\"claude-{model}-4-5\":{{}}}}}}");
}
"#;

const OBSERVER: &str = r#"
#[test]
fn observe_advisory_while_queue_is_alive() {
    let entry = std::path::PathBuf::from(std::env::var("ADVISORY_FIXTURE_ENTRY").unwrap());
    let path = entry.with_extension("advisory");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if text.lines().any(|l| matches!(l, "verdict = PASS" | "verdict = FAIL" | "verdict = ERROR")) {
                break;
            }
        }
        assert!(std::time::Instant::now() < deadline, "advisory did not complete: {}", path.display());
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}
"#;

fn record_field(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{key} = ")))
        .unwrap_or("")
        .to_string()
}

struct Root {
    dir: PathBuf,
    reviewer: PathBuf,
}

fn success(output: Output) -> String {
    assert!(
        output.status.success(),
        "exit {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

fn git(dir: &Path, args: &[&str]) -> String {
    success(
        Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@localhost")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@localhost")
            .stdin(Stdio::null())
            .output()
            .unwrap(),
    )
    .trim()
    .to_string()
}

impl Root {
    fn new(model: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!(
                "queue-advisory-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&dir).unwrap();
        let dir = fs::canonicalize(dir).unwrap();
        let dir = PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"));
        let toml = support::config(&[
            ("projects_dir", "\"scratch/projects\""),
            ("reviewer_model_low_advisory", &format!("\"{model}\"")),
            ("reviewer_max_calls_per_entry", "1"),
            ("reviewer_timeout_seconds", "5"),
            ("mcp_call_timeout_seconds", "600"),
            ("build_lock_wait_seconds", "60"),
        ]);
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        let project = dir.join("scratch/projects/p");
        fs::create_dir_all(project.join("src/bin")).unwrap();
        fs::write(
            project.join("Cargo.toml"),
            "[package]\nname = \"p\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n",
        )
        .unwrap();
        fs::write(
            project.join("Cargo.lock"),
            "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 3\n\n[[package]]\nname = \"p\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        fs::write(
            project.join("src/lib.rs"),
            "pub fn value() -> u8 { 1 }\n",
        )
        .unwrap();
        fs::write(project.join("src/bin/advisory_fixture.rs"), REVIEWER).unwrap();
        git(&project, &["init", "-q", "-b", "main"]);
        git(&project, &["config", "core.autocrlf", "false"]);
        git(&project, &["config", "commit.gpgsign", "false"]);
        git(&project, &["add", "."]);
        git(&project, &["commit", "-q", "-m", "fixture"]);
        let name = format!("advisory_fixture{}", std::env::consts::EXE_SUFFIX);
        let root = Self {
            reviewer: dir.join("scratch/reviewer").join(&name),
            dir,
        };
        root.ok(&["build", "p"]);
        fs::create_dir_all(root.reviewer.parent().unwrap()).unwrap();
        fs::copy(root.dir.join("build/debug").join(name), &root.reviewer).unwrap();
        root
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(support::bin());
        command
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::null());
        command
    }

    fn ok(&self, args: &[&str]) -> String {
        success(self.command(args).output().unwrap())
    }

    fn project(&self) -> PathBuf {
        self.dir.join("scratch/projects/p")
    }

    fn queue(&self) -> (String, String) {
        let task = self
            .ok(&[
                "task",
                "add",
                "p",
                "--title",
                "a note",
                "--claim",
                "src/note.txt",
            ])
            .trim()
            .to_string();
        self.ok(&["task", "start", &task, "--owner", "coder", "--worktree"]);
        let wt = self.dir.join("worktrees/p").join(&task);
        fs::write(wt.join("src/note.txt"), "a note\n").unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "add note"]);
        let output = self.ok(&[
            "queue", "add", &task, "--risk", "low", "--coder", "opus",
        ]);
        let entry = output.lines().next().unwrap().trim().to_string();
        (task, entry)
    }

    fn run(&self, entry: &str, gate: &str, advisory: &str) -> Output {
        let drain = self.configured_advisory();
        if drain {
            self.keepalive();
        }
        let mut command = if drain {
            self.command(&["queue", "run", "--all"])
        } else {
            self.command(&["queue", "run", "--entry", entry])
        };
        command
            .env("BROADSWORD_REVIEWER_CLI", &self.reviewer)
            .env("ADVISORY_FIXTURE_OUT", self.dir.join("fixture-out"))
            .env("ADVISORY_FIXTURE_ENTRY", self.entry_path(entry))
            .env("GATE_FIXTURE_KIND", gate)
            .env("ADVISORY_FIXTURE_KIND", advisory)
            .env("ADVISORY_FIXTURE_TIMEOUT", "5")
            .output()
            .unwrap()
    }

    fn configured_advisory(&self) -> bool {
        fs::read_to_string(self.dir.join("broadsword.toml"))
            .unwrap()
            .contains("reviewer_model_low_advisory = \"sonnet\"")
    }

    fn keepalive(&self) {
        // A second, high-risk entry keeps the real queue process alive while its check waits
        // for the first entry's detached result. This is test harness lifetime management, not
        // a production wait.
        let task = self
            .ok(&[
                "task",
                "add",
                "p",
                "--title",
                "advisory observer",
                "--claim",
                "tests/advisory_observer.rs",
            ])
            .trim()
            .to_string();
        self.ok(&["task", "start", &task, "--owner", "observer", "--worktree"]);
        let wt = self.dir.join("worktrees/p").join(&task);
        fs::create_dir_all(wt.join("tests")).unwrap();
        fs::write(wt.join("tests/advisory_observer.rs"), OBSERVER).unwrap();
        git(&wt, &["add", "."]);
        git(&wt, &["commit", "-q", "-m", "observe detached advisory"]);
        self.ok(&["queue", "add", &task, "--risk", "high", "--coder", "sonnet"]);
    }

    fn advisory(&self, entry: &str) -> String {
        let path = self.field(entry, "advisory_file");
        assert!(!path.is_empty(), "missing advisory reservation");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Ok(text) = fs::read_to_string(self.dir.join(&path)) {
                if matches!(
                    record_field(&text, "verdict").as_str(),
                    "PASS" | "FAIL" | "ERROR"
                ) {
                    return text;
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "advisory did not finish: {path}"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    fn entry_path(&self, entry: &str) -> PathBuf {
        self.dir
            .join("state/queue")
            .join(format!("{entry}.entry"))
    }

    fn field(&self, entry: &str, key: &str) -> String {
        let text = fs::read_to_string(self.entry_path(entry)).unwrap();
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{key} = ")))
            .unwrap_or("")
            .to_string()
    }

    fn calls(&self) -> String {
        fs::read_to_string(self.dir.join("fixture-out/calls")).unwrap()
    }

    fn assert_same_prompt(&self) {
        let read =
            |name| fs::read_to_string(self.dir.join("fixture-out").join(name)).unwrap();
        assert!(!read("haiku.prompt").is_empty());
        assert_eq!(read("haiku.prompt"), read("sonnet.prompt"));
        let gate = read("haiku.args").replace("haiku", "sonnet");
        assert_eq!(
            gate,
            read("sonnet.args"),
            "the system prompt and CLI options match too"
        );
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let scratch =
            fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch")).unwrap();
        let target = fs::canonicalize(&self.dir).unwrap();
        assert!(target.starts_with(&scratch) && target != scratch);
        fs::remove_dir_all(target).unwrap();
    }
}

#[test]
fn advisory_fail_never_blocks_a_merge() {
    let root = Root::new("sonnet");
    let (task, entry) = root.queue();
    let output = root.run(&entry, "pass", "fail");
    let stdout = success(output);
    assert!(stdout.contains("calling the advisory reviewer sonnet (recorded only, never gating)"));
    assert!(stdout.contains("advisory review FAIL (recorded only; the gating review decides)"));
    assert_eq!(root.field(&entry, "status"), "merged");
    assert_eq!(root.field(&entry, "verdict"), "PASS");
    let record = root.advisory(&entry);
    assert_eq!(record_field(&record, "verdict"), "FAIL");
    assert_eq!(root.field(&entry, "advisory_model"), "sonnet");
    assert!(record_field(&record, "blocking").contains("fixture finding"));
    assert_eq!(record_field(&record, "cost_usd"), "0.0123");
    assert!(!record_field(&record, "secs").is_empty());
    let file = root.field(&entry, "advisory_file");
    assert!(!file.is_empty());
    assert!(fs::read_to_string(root.dir.join(file))
        .unwrap()
        .contains("VERDICT: FAIL"));
    assert_eq!(
        root.field(&entry, "review_calls"),
        "1",
        "advisory has its own allowance"
    );
    assert_eq!(root.calls(), "haiku\nsonnet\nopus\n");
    root.assert_same_prompt();
    assert!(root.ok(&["task", "show", &task]).contains("status: done"));
    assert!(root.project().join("src/note.txt").is_file());
    let rerun = root
        .command(&["queue", "run", "--entry", &entry])
        .output()
        .unwrap();
    assert_eq!(rerun.status.code(), Some(2), "{rerun:?}");
    assert_eq!(
        root.calls(),
        "haiku\nsonnet\nopus\n",
        "a completed entry never calls again"
    );
}

#[test]
fn advisory_pass_never_rescues_a_rejected_entry() {
    let root = Root::new("sonnet");
    let (task, entry) = root.queue();
    let before = git(&root.project(), &["rev-parse", "HEAD"]);
    let output = root.run(&entry, "fail", "pass");
    assert_eq!(output.status.code(), Some(5), "{output:?}");
    assert_eq!(root.field(&entry, "status"), "rejected");
    assert_eq!(root.field(&entry, "verdict"), "FAIL");
    assert_eq!(record_field(&root.advisory(&entry), "verdict"), "PASS");
    assert_eq!(root.calls(), "haiku\nsonnet\nopus\n");
    assert_ne!(
        git(&root.project(), &["rev-parse", "HEAD"]),
        before,
        "only the observer merged"
    );
    assert!(!root.project().join("src/note.txt").exists());
    assert!(root.ok(&["task", "show", &task]).contains("status: running"));
    root.assert_same_prompt();
}

#[test]
fn advisory_off_makes_no_call() {
    let root = Root::new("off");
    let (_, entry) = root.queue();
    // A fresh attempt must erase all advisory fields left by an earlier attempt.
    let path = root.entry_path(&entry);
    let mut text = fs::read_to_string(&path).unwrap();
    for key in [
        "advisory_model",
        "advisory_verdict",
        "advisory_reason",
        "advisory_blocking",
        "advisory_cost_usd",
        "advisory_secs",
        "advisory_file",
    ] {
        text.push_str(&format!("{key} = old attempt\n"));
    }
    fs::write(path, text).unwrap();
    success(root.run(&entry, "pass", "fail"));
    assert_eq!(root.field(&entry, "status"), "merged");
    assert_eq!(root.calls(), "haiku\n");
    assert!(root.field(&entry, "advisory_model").is_empty());
    assert!(root.field(&entry, "advisory_verdict").is_empty());
    for key in [
        "advisory_reason",
        "advisory_blocking",
        "advisory_cost_usd",
        "advisory_secs",
        "advisory_file",
    ] {
        assert!(root.field(&entry, key).is_empty(), "{key}");
    }
}

#[test]
fn advisory_error_is_recorded_and_the_entry_goes_on() {
    for kind in ["error", "timeout", "malformed"] {
        let root = Root::new("sonnet");
        let (_, entry) = root.queue();
        success(root.run(&entry, "pass", kind));
        assert_eq!(root.field(&entry, "status"), "merged", "{kind}");
        assert_eq!(root.field(&entry, "verdict"), "PASS");
        let record = root.advisory(&entry);
        assert_eq!(record_field(&record, "verdict"), "ERROR", "{kind}");
        assert!(!record_field(&record, "reason").is_empty(), "{kind}");
        assert_eq!(root.calls(), "haiku\nsonnet\nopus\n");
    }
}
