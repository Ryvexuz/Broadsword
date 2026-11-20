//! Every child process Broadsword starts gets the scrubbed environment (`src/scrub.rs`), from ONE spawn helper
//! (`scrub::command` / `scrub::command_in`). This test reads every `.rs` file under `src/` and fails on any `Command::new` that is
//! not on the allow-list below, and on an allow-list entry that no longer matches (so the list cannot silently go stale).
//! Comment lines are skipped. The behavioural tests (a planted secret the child must not see) sit next to each spawn site:
//! `scrub.rs`, `git.rs`, `wtgit.rs`, `pause.rs`.

use std::fs;
use std::path::{Path, PathBuf};

/// (file under src/, text the line contains, how many such lines, why it may start a child without the helper).
const ALLOWED: &[(&str, &str, usize, &str)] = &[
    ("scrub.rs", "Command::new(program)", 1, "the spawn helper itself: scrub::command applies the scrubbed environment"),
    ("buildsrc.rs", "Command::new(std::env::var(\"CARGO\")", 1, "already scrubbed via scrub::apply (buildsrc::cargo, the cargo factory)"),
    ("buildsrc.rs", "Command::new(\"cmd\").arg(\"/C\").raw_arg(format!(\"mklink /J", 1, "test-only (#[cfg(test)]): makes a junction, no secret reaches it"),
    ("job.rs", "Command::new(\"cmd\")", 1, "already scrubbed via scrub::apply (shell_command's caller, the job step)"),
    ("job.rs", "Command::new(\"sh\")", 1, "already scrubbed via scrub::apply (shell_command's caller, the job step)"),
    ("job.rs", "Command::new(&c.copy)", 1, "already scrubbed via scrub::apply (command_for, the copied test executables)"),
    ("mcp.rs", "Command::new(exe)", 1, "already scrubbed via scrub::apply (run_child, the MCP tool child)"),
    ("reviewer.rs", "Command::new(program)", 1, "run_cli: scrubbed by its callers' remove list (Secrets::scrub_names, the same rule)"),
    ("sys.rs", "Command::new(\"taskkill\")", 1, "system binary given only a PID (kill_tree); it reads nothing secret from its environment"),
    ("sys.rs", "Command::new(\"kill\")", 1, "system binary given only a PID (kill_tree); it reads nothing secret from its environment"),
    ("sys.rs", "Command::new(\"cmd\").args([\"/C\", \"exit 0\"])", 1, "test-only (#[cfg(test)]): a child that exits at once"),
];

/// Every problem with `sources` ((path relative to src/ with `/`, text) pairs) against `allowed`; empty means all spawns are covered.
fn problems(sources: &[(String, String)], allowed: &[(&str, &str, usize, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    let mut used = vec![0usize; allowed.len()];
    for (file, text) in sources {
        for (n, line) in text.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") || !line.contains("Command::new") {
                continue;
            }
            match allowed.iter().position(|(f, snippet, _, _)| f == file && line.contains(snippet)) {
                Some(i) => used[i] += 1,
                None => out.push(format!("src/{file}:{}: `Command::new` outside the spawn helper; use scrub::command or scrub::command_in (or allow-list it here with a reason): {}", n + 1, t)),
            }
        }
    }
    for (i, (f, snippet, want, _)) in allowed.iter().enumerate() {
        if used[i] != *want {
            out.push(format!("allow-list entry src/{f} `{snippet}`: expected {want} line(s), found {}", used[i]));
        }
    }
    out
}

fn rs_files(dir: &Path, base: &Path, out: &mut Vec<(String, String)>) {
    for e in fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display())) {
        let p: PathBuf = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, base, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            out.push((rel, fs::read_to_string(&p).unwrap()));
        }
    }
}

#[test]
fn every_command_new_under_src_is_the_helper_or_allow_listed() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    rs_files(&src, &src, &mut sources);
    let spawns: usize = sources.iter().map(|(_, t)| t.lines().filter(|l| !l.trim_start().starts_with("//") && l.contains("Command::new")).count()).sum();
    eprintln!("scanned {} files under src/, {} Command::new lines", sources.len(), spawns);
    assert!(sources.iter().any(|(f, _)| f == "scrub.rs") && sources.len() > 10, "the scan found too few files to mean anything: {}", sources.len());
    assert!(spawns >= ALLOWED.len(), "the scan found only {spawns} Command::new lines");
    let p = problems(&sources, ALLOWED);
    assert!(p.is_empty(), "{}", p.join("\n"));
}

#[test]
fn the_check_fails_on_a_new_spawn_site_and_on_a_stale_entry() {
    let ok = vec![("a.rs".to_string(), "let c = Command::new(program);\n// Command::new(\"in a comment\")\n".to_string())];
    let allowed: &[(&str, &str, usize, &str)] = &[("a.rs", "Command::new(program)", 1, "helper")];
    assert!(problems(&ok, allowed).is_empty(), "{:?}", problems(&ok, allowed));

    let added = vec![("a.rs".to_string(), "let c = Command::new(program);\nlet d = std::process::Command::new(\"git\");\n".to_string())];
    let p = problems(&added, allowed);
    assert_eq!(p.len(), 1, "{p:?}");
    assert!(p[0].starts_with("src/a.rs:2: `Command::new` outside the spawn helper"), "{p:?}");

    let gone = vec![("a.rs".to_string(), "nothing here\n".to_string())];
    let p = problems(&gone, allowed);
    assert_eq!(p, ["allow-list entry src/a.rs `Command::new(program)`: expected 1 line(s), found 0"], "{p:?}");
}
