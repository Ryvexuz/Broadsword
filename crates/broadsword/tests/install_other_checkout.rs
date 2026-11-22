//! Install provenance must not survive a run that can overwrite its shared executable from another checkout.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

mod support;

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Root {
    dir: PathBuf,
    timeout: Duration,
}

impl Root {
    fn new() -> Self {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!(
                "install-other-checkout-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        fs::create_dir(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let dir = PathBuf::from(dir.to_string_lossy().trim_start_matches(r"\\?\"));
        fs::create_dir_all(dir.join("scratch")).unwrap();
        let config = support::config(&[]);
        let timeout = config
            .lines()
            .find_map(|line| line.strip_prefix("done_check_timeout_seconds = "))
            .unwrap()
            .parse()
            .unwrap();
        fs::write(dir.join("broadsword.toml"), config).unwrap();
        let root = Self {
            dir,
            timeout: Duration::from_secs(timeout),
        };
        root.package("", "broadsword", "MAIN");
        root.package("projects/driver", "driver", "DRIVER");
        // A real copy of the source, with its own workspace and a changed marker.
        fs::create_dir_all(root.dir.join("scratch/copy/src")).unwrap();
        fs::copy(
            root.dir.join("Cargo.toml"),
            root.dir.join("scratch/copy/Cargo.toml"),
        )
        .unwrap();
        fs::copy(
            root.dir.join("src/main.rs"),
            root.dir.join("scratch/copy/src/main.rs"),
        )
        .unwrap();
        fs::write(
            root.dir.join("scratch/copy/src/main.rs"),
            "fn main() { println!(\"FOREIGN\"); }\n",
        )
        .unwrap();
        for folder in ["", "projects/driver"] {
            for args in [
                &["init", "-q"][..],
                &["config", "user.email", "test@localhost"],
                &["config", "user.name", "test"],
                &["add", "."],
                &["commit", "-q", "-m", "fixture"],
            ] {
                let mut command = Command::new("git");
                command.args(args).current_dir(root.dir.join(folder));
                root.checked(&mut command);
            }
        }
        root.run(&["build", "broadsword"], true);
        assert_eq!(root.note("broadsword"), ".\n");
        assert_eq!(root.note("bin.broadsword"), ".\n");
        root
    }

    fn package(&self, folder: &str, name: &str, marker: &str) {
        let dir = self.dir.join(folder);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("Cargo.toml"),
            format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n[workspace]\n"),
        )
        .unwrap();
        fs::write(
            dir.join("src/main.rs"),
            format!("fn main() {{ println!(\"{marker}\"); }}\n"),
        )
        .unwrap();
        fs::write(dir.join(".gitignore"), "/scratch\n/build\n/state\n/projects\n").unwrap();
    }

    /// Files, not pipes: an inherited stdout held by a descendant cannot hang the test's output collection.
    fn execute(&self, command: &mut Command) -> (bool, String, String) {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let stdout = self.dir.join(format!("scratch/command-{id}.out"));
        let stderr = self.dir.join(format!("scratch/command-{id}.err"));
        command
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap());
        let mut child = command.spawn().unwrap();
        let deadline = Instant::now() + self.timeout;
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("command timed out: {command:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        (
            status.success(),
            fs::read_to_string(stdout).unwrap(),
            fs::read_to_string(stderr).unwrap(),
        )
    }

    fn checked(&self, command: &mut Command) -> String {
        let (ok, out, err) = self.execute(command);
        assert!(ok, "{command:?}\nstdout: {out}\nstderr: {err}");
        out
    }

    fn run(&self, args: &[&str], success: bool) -> (String, String) {
        let mut command = Command::new(support::bin());
        command
            .args(args)
            .current_dir(&self.dir)
            .env_remove("BROADSWORD_RUN_HOLDS_BUILD_LOCK");
        let (ok, out, err) = self.execute(&mut command);
        assert_eq!(ok, success, "{args:?}\nstdout: {out}\nstderr: {err}");
        (out, err)
    }

    fn note(&self, name: &str) -> String {
        fs::read_to_string(self.dir.join("state/build-sources").join(name)).unwrap()
    }

    fn probe(&self) -> String {
        let mut command = Command::new(
            self.dir
                .join("build/debug")
                .join(format!("broadsword{}", std::env::consts::EXE_SUFFIX)),
        );
        self.checked(&mut command)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn a_command_naming_a_scratch_copys_manifest_is_refused_before_main_is_noted() {
    let r = Root::new();
    assert_eq!(r.probe().trim(), "MAIN");
    for command in [
        "cargo build --offline --manifest-path scratch/copy/Cargo.toml",
        "cargo build --offline --manifest-path=scratch/copy/Cargo.toml",
    ] {
        let (_, err) = r.run(&["run", "broadsword", "--", command], false);
        assert!(
            err.contains("refusing run: --manifest-path")
                && err.contains("copy")
                && err.contains("nothing was built"),
            "{err}"
        );
        assert_eq!(r.note("broadsword"), ".\n");
        assert_eq!(r.note("bin.broadsword"), ".\n");
        assert_eq!(r.probe().trim(), "MAIN", "the foreign build never started");
    }
    let (out, _) = r.run(
        &[
            "run",
            "broadsword",
            "--",
            "cargo build --offline --manifest-path Cargo.toml",
        ],
        true,
    );
    assert!(
        out.contains("build manifest:") && out.contains("package: broadsword"),
        "{out}"
    );
    assert_eq!(r.note("bin.broadsword"), "run:.\n");
}

#[test]
fn an_opaque_run_from_another_project_cannot_leave_the_foreign_binary_installable() {
    let r = Root::new();
    // Cargo's fingerprint of the copy equals main's (same package, same workspace-relative path), so a copy whose source is
    // older than the main build looks fresh. Rewrite the copy's source now, as a real edit would.
    let copy_main = r.dir.join("scratch/copy/src/main.rs");
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(&copy_main, fs::read_to_string(&copy_main).unwrap()).unwrap();
    let (out, _) = r.run(
        &[
            "run",
            "driver",
            "--",
            "cd ../../scratch/copy && cargo build --offline",
        ],
        true,
    );
    assert!(out.contains("install provenance invalidated"), "{out}");
    assert_eq!(
        r.probe().trim(),
        "FOREIGN",
        "the copy really overwrote the shared executable"
    );
    assert_eq!(
        r.note("broadsword"),
        ".\n",
        "the old package note alone would still vouch for main"
    );
    assert_eq!(r.note("bin.broadsword"), "run:projects/driver\n");
    let (_, err) = r.run(&["install"], false);
    assert!(
        err.contains("refusing to install")
            && err.contains("binary target")
            && err.contains("run:projects/driver"),
        "{err}"
    );
    assert!(!r.dir.join("build/bin").exists(), "install copied nothing");
    // An unrelated plain build must not clear the shared-exe guard.
    r.run(&["build", "driver"], true);
    assert_eq!(r.note("bin.broadsword"), "run:projects/driver\n");
    r.run(&["build", "broadsword"], true);
    assert_eq!(
        r.probe().trim(),
        "MAIN",
        "main is cleaned even though its old Cargo fingerprint was fresh"
    );
    assert_eq!(r.note("broadsword"), ".\n");
    assert_eq!(r.note("bin.broadsword"), ".\n");
}
