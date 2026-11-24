//! A conservative Python-only pre-merge check; discovery errors keep the existing cargo check.

use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec};
use crate::wtgit;
use std::fs;
use std::path::Path;

/// Only nonempty lists of canonical, relative Git paths on the no-Rust allow-list qualify.
/// Source text is deliberately searched without parsing: comments and strings count too.
pub(super) fn no_rust_paths(paths: &[String], rust_literals: &str) -> Result<(), String> {
    if paths.is_empty() {
        return Err("empty changed-path list: no evidence for a Python-only check".into());
    }
    let folded_literals = rust_literals.to_ascii_lowercase();
    for path in paths {
        let bad = |rule: &str| Err(format!("{path:?}: {rule}"));
        if !canonical_path(path) {
            return bad("untrusted path: expected a canonical relative path with forward slashes");
        }
        // Case aliases must not bypass the guard on Windows.
        let lower = path.to_ascii_lowercase();
        let allowed_location = [
            "docs/",
            "evidence/",
            "templates/",
            "scripts/",
            "clients/",
            "bench/",
        ]
        .iter()
        .any(|prefix| lower.starts_with(*prefix))
            || (!lower.contains('/')
                && lower.ends_with(".md")
                && !matches!(lower.as_str(), "agents.md" | "claude.md"));
        let allowed_extension = [".md", ".txt", ".py", ".json", ".csv", ".tsv", ".ps1"]
            .iter()
            .any(|extension| lower.ends_with(*extension));
        let allowed_segments = lower.split('/').all(|part| {
            !part.starts_with('.')
                && !matches!(
                    part,
                    "cargo.toml" | "cargo.lock" | "rust-toolchain" | "rust-toolchain.toml"
                )
        });
        if !(allowed_location && allowed_extension && allowed_segments) {
            return bad("allow-list rule: unapproved location, extension or path segment");
        }
        if folded_literals.contains(lower.as_str()) {
            return bad("rule (c): path occurs in Rust source text");
        }
    }
    Ok(())
}

fn canonical_path(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', ':'])
        && !path
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        && path.split('/').all(|part| {
            !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' '])
        })
}

/// Git's NUL-delimited form avoids quoting, trimming or splitting hostile filenames.
fn parse_paths(bytes: &[u8]) -> Result<Vec<String>, String> {
    let text = std::str::from_utf8(bytes).map_err(|e| format!("git paths are not UTF-8: {e}"))?;
    if text.is_empty() {
        return Err("git diff returned an empty changed-path list".into());
    }
    let text = text
        .strip_suffix('\0')
        .ok_or("git diff paths are not NUL-terminated")?;
    let paths: Vec<String> = text.split('\0').map(String::from).collect();
    // Validate before any caller can interpret the paths differently.
    for path in &paths {
        if !canonical_path(path) {
            return Err(format!("git diff returned an untrusted path: {path:?}"));
        }
    }
    Ok(paths)
}

fn changed_paths(dir: &Path) -> Result<Vec<String>, String> {
    let trunk = wtgit::trunk_branch(dir)?;
    // Include both sides of renames: moving a Cargo or .rs file is still a Rust change.
    // The ONE spawn helper (tests/scrub.rs): git gets the scrubbed environment like every other child.
    let output = crate::scrub::command_in(dir, "git")?
        .args([
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            &format!("{trunk}...HEAD"),
            "--",
        ])
        .current_dir(dir)
        .output()
        .map_err(|e| format!("cannot run git diff: {e}"))?;
    if !output.status.success() {
        return Err(format!("git diff failed ({})", output.status));
    }
    parse_paths(&output.stdout)
}

/// Read Rust sources of the commit being checked, not the running binary's checkout.
/// Only queue/no_rust.rs is excluded: its test literals must not disqualify docs and scripts.
/// Linked entries are rejected; an empty or unreadable remaining source tree is not evidence.
fn rust_sources(dir: &Path) -> Result<String, String> {
    let mut source = dir.to_path_buf();
    for part in ["crates", "broadsword", "src"] {
        source.push(part);
        let meta = fs::symlink_metadata(&source)
            .map_err(|e| format!("cannot inspect {}: {e}", source.display()))?;
        if !meta.is_dir() || meta.is_symlink() {
            return Err(format!(
                "untrusted Rust source directory: {}",
                source.display()
            ));
        }
    }
    read_sources(&source)
}

fn read_sources(source: &Path) -> Result<String, String> {
    let own_source = source.join("queue").join("no_rust.rs");
    let mut pending = vec![source.to_path_buf()];
    let mut text = String::new();
    let mut found = false;
    while let Some(dir) = pending.pop() {
        let meta = fs::symlink_metadata(&dir)
            .map_err(|e| format!("cannot inspect {}: {e}", dir.display()))?;
        if !meta.is_dir() || meta.is_symlink() {
            return Err(format!("untrusted Rust source directory: {}", dir.display()));
        }
        for entry in
            fs::read_dir(&dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?
        {
            let entry = entry.map_err(|e| format!("cannot read {} entry: {e}", dir.display()))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
            if kind.is_symlink() || (!kind.is_dir() && !kind.is_file()) {
                return Err(format!("untrusted Rust source entry: {}", path.display()));
            }
            if kind.is_dir() {
                pending.push(path);
            } else if path != own_source && path.extension().is_some_and(|ext| ext == "rs") {
                let content = fs::read_to_string(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                text.push_str(&content);
                text.push('\n');
                found = true;
            }
        }
    }
    if !found {
        return Err("Rust source tree contains no .rs files".into());
    }
    Ok(text)
}

fn python_command(scripts: &[String]) -> Result<String, String> {
    if scripts.is_empty() {
        return Err("project declares no Python tests".into());
    }
    // Config paths normally go directly into argv. This run job uses a shell on both
    // Windows and Unix, so only shell-neutral paths can be interpolated here.
    for script in scripts {
        if !canonical_path(script)
            || !script.ends_with(".py")
            || script.starts_with('-')
            || !script
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
        {
            return Err(format!("Python test path is not shell-neutral: {script:?}"));
        }
    }
    Ok(scripts
        .iter()
        .map(|script| format!("python -I -B {script}"))
        .collect::<Vec<_>>()
        .join(" && "))
}

fn plan(
    scripts: &[String],
    paths: Result<Vec<String>, String>,
    literals: Result<String, String>,
) -> Result<(String, usize), String> {
    let paths = paths?;
    let literals = literals?;
    no_rust_paths(&paths, &literals)?;
    Ok((python_command(scripts)?, paths.len()))
}

fn label(count: usize, command: &str) -> String {
    format!("check: python only (no Rust file changed: {count} file(s)) = `{command}`, NOT the cargo build or tests: the full check runs on main right after the merge, and a failure there is reverted")
}

fn exit_code(code: u8, info: &JobInfo) -> u8 {
    let failed = code != 0
        && info
            .summary
            .first()
            .is_some_and(|line| line.starts_with("run: FAILED"));
    if failed {
        1
    } else {
        code
    }
}

/// None means the caller runs its existing full check or fast subset, without modification.
/// Keep eligibility here too, so another caller cannot bypass the project opt-in.
pub(super) fn pre_merge_check(
    cfg: &Config,
    project: &str,
    task_id: &str,
    info: &mut JobInfo,
) -> Option<(u8, String)> {
    cfg.merge_fast_subset(project)?;
    let scripts = job::python_scripts(cfg, project);
    if scripts.is_empty() {
        return None;
    }
    let planned = job::job_folder(cfg, project, Some(task_id))
        .map_err(|(_, message)| message)
        .and_then(|dir| plan(scripts, changed_paths(&dir), rust_sources(&dir)));
    let (command, count) = match planned {
        Ok(p) => p,
        Err(why) => {
            println!("Python-only check off for {project}: {why}; the existing cargo check runs");
            return None;
        }
    };
    let which = label(count, &command);
    println!("{which}");
    let code = job::run_job_info(
        cfg,
        JobSpec {
            flaky: None,
            kind: "run".into(),
            name: project.to_string(),
            command: Some(command),
            first_step: 0,
            resumed_from: None,
            resume_count: 0,
            task: Some(task_id.to_string()),
        },
        info,
    );
    Some((exit_code(code, info), which))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn rejected(path: &str, rule: &str) {
        let error = no_rust_paths(&paths(&["docs/first.md", path]), "").unwrap_err();
        assert!(error.contains(path) && error.contains(rule), "{error}");
    }

    #[test]
    fn docs_and_scripts_only_pass() {
        assert_eq!(
            no_rust_paths(
                &paths(&["README.md", "docs/queue.md", "scripts/test_queue.py"]),
                "fn main() {}"
            ),
            Ok(())
        );
    }

    #[test]
    fn any_rs_or_crates_path_fails() {
        for path in ["main.rs", "docs/example.rs", "MAIN.RS"] {
            rejected(path, "allow-list rule");
        }
        for path in [
            "crates/readme.md",
            "crates/broadsword/src/main.rs",
            "Crates/docs.md",
            ".github/workflows/check.yml",
            ".claude/agents/coder.md",
            "build/notes.txt",
        ] {
            rejected(path, "allow-list rule");
        }
    }

    #[test]
    fn cargo_and_settings_files_fail() {
        for path in [
            "Cargo.toml",
            "Cargo.lock",
            "broadsword.toml",
            "AGENTS.md",
            "CLAUDE.md",
            "rust-toolchain.toml",
            "CARGO.TOML",
        ] {
            rejected(path, "allow-list rule");
        }
    }

    #[test]
    fn a_script_named_in_rust_source_fails() {
        let error = no_rust_paths(
            &paths(&["clients/openai_review.py"]),
            r#"const SCRIPT: &str = "clients/openai_review.py";"#,
        )
        .expect_err("a Rust-referenced script must keep the cargo check");
        assert!(error.contains("clients/openai_review.py") && error.contains("rule (c)"));
        assert!(no_rust_paths(
            &paths(&["scripts/helper.py"]),
            "// scripts/helper.py is mentioned in a comment"
        )
        .is_err());
        assert!(no_rust_paths(
            &paths(&["clients/OPENAI_REVIEW.py"]),
            r#""clients/openai_review.py""#
        )
        .is_err());
    }

    #[test]
    fn empty_list_fails() {
        assert!(
            no_rust_paths(&[], "").is_err(),
            "an empty diff must keep the cargo check"
        );
    }

    #[test]
    fn projects_path_fails() {
        rejected("projects/crash-sim/README.md", "allow-list rule");
    }

    #[test]
    fn nested_cargo_files_fail() {
        for path in [
            "vendor/foo/Cargo.toml",
            "tools/x/Cargo.lock",
            "docs/Cargo.toml",
            "scripts/Cargo.lock",
            "docs/Cargo.toml/notes.md",
            "clients/CARGO.LOCK/test.py",
        ] {
            rejected(path, "allow-list rule");
        }
    }

    #[test]
    fn dot_folders_fail() {
        for path in [
            ".cargo/config.toml",
            "docs/.cargo/config.json",
            "scripts/.hidden.py",
            "evidence/.hidden/report.md",
        ] {
            rejected(path, "allow-list rule");
        }
    }

    #[test]
    fn rust_toolchain_fails() {
        for path in [
            "rust-toolchain",
            "rust-toolchain.toml",
            "docs/rust-toolchain",
            "templates/rust-toolchain.toml",
            "docs/rust-toolchain/notes.md",
            "bench/RUST-TOOLCHAIN.TOML/results.csv",
        ] {
            rejected(path, "allow-list rule");
        }
    }

    #[test]
    fn unknown_top_folder_fails() {
        for path in [
            "assets/x.md",
            "vendor/notes.md",
            "tools/helper.py",
            "doc/queue.md",
            "docs-extra/queue.md",
            "notes.txt",
            "helper.py",
        ] {
            rejected(path, "allow-list rule");
        }
    }

    #[test]
    fn root_markdown_passes() {
        assert_eq!(
            no_rust_paths(&paths(&["README.md", "RETRO.md", "Notes.MD"]), ""),
            Ok(())
        );
        for path in ["agents.md", "CLAUDE.MD", ".notes.md"] {
            rejected(path, "allow-list rule");
        }
    }

    #[test]
    fn allow_list_requires_both_location_and_extension() {
        for folder in ["docs", "evidence", "templates", "scripts", "clients", "bench"] {
            for extension in ["md", "txt", "py", "json", "csv", "tsv", "ps1"] {
                let path = format!("{folder}/nested/file.{extension}");
                assert_eq!(no_rust_paths(&paths(&[&path]), ""), Ok(()), "{path}");
                assert_eq!(
                    no_rust_paths(&paths(&[&path.to_ascii_uppercase()]), ""),
                    Ok(()),
                    "{path}"
                );
            }
            for name in ["file.rs", "file.toml", "file.lock", "file", "file.md.rs"] {
                rejected(&format!("{folder}/{name}"), "allow-list rule");
            }
        }
    }

    #[test]
    fn own_test_literals_do_not_count() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let path = "docs/no_rust_own_test_literal_probe.md";
        let own = fs::read_to_string(root.join("crates/broadsword/src/queue/no_rust.rs"))
            .expect("this test's source can be read");
        assert!(
            own.contains(path),
            "the probe must actually occur in this module"
        );
        let literals = rust_sources(&root).expect("the remaining Rust sources can be read");
        assert!(!literals.contains(path), "this module must be excluded");
        assert_eq!(no_rust_paths(&paths(&[path]), &literals), Ok(()));
        assert!(
            no_rust_paths(&paths(&[path]), &own).is_err(),
            "including this module would conservatively reject its test literal"
        );
    }

    #[test]
    fn windows_style_path_is_not_trusted() {
        for path in [
            r"crates\broadsword\src\main.rs",
            r"projects\render\README.md",
            r"docs\notes.md",
            "C:/docs/notes.md",
        ] {
            let error = no_rust_paths(&paths(&[path]), "").unwrap_err();
            assert!(error.contains("untrusted path"), "{error}");
        }
    }

    #[test]
    fn ambiguous_paths_and_first_failure_are_rejected() {
        for path in [
            "",
            "/docs/a.md",
            "./Cargo.toml",
            "docs/../Cargo.toml",
            "docs//a.md",
            "crates./a.md",
            "crates /a.md",
            "docs/a\nb.md",
            "docs/a\rb.md",
            "docs/a\u{2028}b.md",
        ] {
            assert!(no_rust_paths(&paths(&[path]), "").is_err(), "{path:?}");
        }
        let error = no_rust_paths(&paths(&["Cargo.lock", "later.rs"]), "").unwrap_err();
        assert!(error.contains("Cargo.lock") && !error.contains("later.rs"));
    }

    #[test]
    fn git_paths_are_not_trimmed_or_lossily_decoded() {
        assert_eq!(
            parse_paths(b"docs/a b.md\0scripts/a.py\0").unwrap(),
            paths(&["docs/a b.md", "scripts/a.py"])
        );
        for bytes in [
            b"".as_slice(),
            b"\0",
            b"docs/a.md",
            b"docs/a.md\0\0",
            b"docs/a\nb.md\0",
            b"docs/a\rb.md\0",
            b"projects\\a.md\0",
            b"\xff\0",
        ] {
            assert!(parse_paths(bytes).is_err(), "{bytes:?}");
        }
    }

    #[test]
    fn python_command_preserves_order_and_refuses_shell_syntax() {
        assert_eq!(
            python_command(&paths(&["scripts/test_a.py", "clients/test_b.py"])).unwrap(),
            "python -I -B scripts/test_a.py && python -I -B clients/test_b.py"
        );
        assert!(python_command(&[]).is_err());
        for script in [
            "scripts/a.py&other.py",
            "scripts/%NAME%.py",
            "scripts/$NAME.py",
            "scripts/a`other`.py",
            "scripts/a\".py",
            "scripts/a b.py",
            "scripts/a\nb.py",
            "../a.py",
            "/a.py",
            "C:/a.py",
            r"scripts\a.py",
            "-a.py",
            "scripts/a.txt",
        ] {
            assert!(python_command(&paths(&[script])).is_err(), "{script:?}");
        }
    }

    #[test]
    fn discovery_errors_and_empty_inputs_cannot_make_a_plan() {
        let scripts = paths(&["scripts/test_a.py"]);
        assert_eq!(
            plan(&scripts, Err("git failed".into()), Ok(String::new())),
            Err("git failed".into())
        );
        assert_eq!(
            plan(&scripts, Ok(paths(&["README.md"])), Err("read failed".into())),
            Err("read failed".into())
        );
        assert!(plan(&scripts, Ok(vec![]), Ok(String::new())).is_err());
        assert!(plan(&[], Ok(paths(&["README.md"])), Ok(String::new())).is_err());
        assert!(plan(&scripts, Ok(paths(&["Cargo.lock"])), Ok(String::new())).is_err());
        assert_eq!(
            plan(&scripts, Ok(paths(&["README.md"])), Ok(String::new())),
            Ok(("python -I -B scripts/test_a.py".into(), 1))
        );
    }

    #[test]
    fn source_reader_collects_nested_sources_and_reports_read_errors() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let text = rust_sources(&root).expect("the checked-out Broadsword source can be read");
        let nested = fs::read_to_string(root.join("crates/broadsword/src/queue/check.rs"))
            .expect("the nested check module can be read");
        assert!(text.contains(&nested));
        let file = root.join("crates/broadsword/src/queue/no_rust.rs");
        assert!(
            read_sources(&file).is_err(),
            "a file is not a source directory"
        );
        assert!(
            rust_sources(&file).is_err(),
            "source discovery errors propagate"
        );
    }

    #[test]
    fn lower_entry_point_keeps_unlisted_and_no_script_projects_unchanged() {
        let mut cfg = Config::from_text(
            include_str!("../../../../broadsword.toml"),
            Path::new("no-rust-unit-test"),
        )
        .expect("shipped settings parse without filesystem access");
        let mut info = JobInfo::default();
        cfg.merge_fast_tests.clear();
        cfg.python_tests = vec![("broadsword".into(), paths(&["scripts/test_a.py"]))];
        assert!(pre_merge_check(&cfg, "broadsword", "not-a-task", &mut info).is_none());
        cfg.merge_fast_tests = vec![("broadsword".into(), paths(&["queue"]))];
        cfg.python_tests.clear();
        assert!(pre_merge_check(&cfg, "broadsword", "not-a-task", &mut info).is_none());
        assert!(info.id.is_none() && info.summary.is_empty());
    }

    #[test]
    fn late_protected_paths_and_literals_are_not_truncated() {
        let mut many = vec!["docs/notes.md".to_string(); 10_000];
        many.push("crates/last.md".into());
        assert!(no_rust_paths(&many, "").is_err());
        let mut literals = "x".repeat(1_000_000);
        literals.push_str("clients/openai_review.py");
        assert!(no_rust_paths(&paths(&["clients/openai_review.py"]), &literals).is_err());
    }

    #[test]
    fn label_and_exit_code_do_not_overclaim() {
        let text = label(2, "python -I -B scripts/test_a.py");
        assert!(text.starts_with("check: python only (no Rust file changed: 2 file(s))"));
        let disclaimer = "NOT the cargo build or tests: the full check runs on main right after the merge, and a failure there is reverted";
        assert!(text.contains(disclaimer));
        let mut info = JobInfo::default();
        assert_eq!(exit_code(0, &info), 0);
        assert_eq!(exit_code(3, &info), 3);
        info.summary = vec!["run: FAILED (exit 17)".into()];
        assert_eq!(exit_code(17, &info), 1);
        info.summary = vec!["run: KILLED (disk below floor)".into()];
        assert_eq!(exit_code(3, &info), 3);
    }
}
