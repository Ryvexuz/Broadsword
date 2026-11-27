use super::*;

#[test]
fn a_conflicting_start_exits_3_and_names_the_other_task() {
    let r = Root::new("conflict");
    let a = r.add("p", "a", &["src"]);
    let b = r.add("p", "b", &["lib", "src/a.rs"]);
    let c = r.add("p", "c", &["srcx"]);
    assert_eq!(r.start(&a, "coder-1").code, 0);
    let o = r.start(&b, "coder-2");
    assert_eq!(o.code, 3, "{}", o.stderr);
    for want in [a.as_str(), "coder-1", "`src/a.rs`", "`src`"] {
        assert!(o.stderr.contains(want), "{want} in {}", o.stderr);
    }
    assert_eq!(r.status_of(&b), "open", "a refused task stays open");
    assert_eq!(r.start(&c, "coder-3").code, 0, "srcx does not overlap src");
    // once the holder is done the refused task can start
    assert_eq!(r.run(&["task", "done", &a]).code, 0);
    assert_eq!(r.start(&b, "coder-2").code, 0);
    // two running tasks are named together
    let w = r.add("p", "whole", &["*"]);
    let o = r.start(&w, "boss");
    assert_eq!(o.code, 3);
    assert!(o.stderr.contains(&b) && o.stderr.contains(&c) && o.stderr.contains("coder-2") && o.stderr.contains("coder-3"), "{}", o.stderr);
}

#[test]
fn key_claims_on_different_keys_run_together() {
    for file in ["broadsword.toml", "crates/broadsword/src/config.rs", "crates/broadsword/tests/support/mod.rs"] {
        let r = Root::new("key-claims");
        let alpha = format!("{file}#alpha");
        let beta = format!("{file}#beta");
        let a = r.add("broadsword", "alpha", &[&alpha]);
        let b = r.add("broadsword", "beta", &[&beta]);
        let c = r.add("broadsword", "alpha again", &[&alpha]);
        let whole = r.add("broadsword", "whole file", &[file]);
        let show = r.run(&["task", "show", &a]);
        assert_eq!(show.code, 0, "{}", show.stderr);
        assert!(show.stdout.contains(&format!("claims: {alpha}")), "{}", show.stdout);
        for id in [&a, &b] {
            let o = r.start(id, "coder");
            assert_eq!(o.code, 0, "{file}: {}", o.stderr);
            assert_eq!(r.status_of(id), "running");
        }
        let o = r.start(&c, "same-key");
        assert_eq!(o.code, 3, "{}", o.stderr);
        for want in [a.as_str(), c.as_str(), alpha.as_str(), "overlaps"] {
            assert!(o.stderr.contains(want), "{want} in {}", o.stderr);
        }
        assert!(o.stderr.contains(&format!("task {a} (owner coder)")), "{}", o.stderr);
        assert!(!o.stderr.contains(&format!("task {b} (owner")), "different key must not be reported: {}", o.stderr);
        assert_eq!(r.status_of(&c), "open");
        let check = r.run(&["task", "check", "broadsword", "--claim", &alpha]);
        assert_eq!(check.code, 3, "{}", check.stdout);
        assert!(check.stdout.contains(&format!("task {a} (owner coder)")), "{}", check.stdout);
        let gamma = format!("{file}#gamma");
        let check = r.run(&["task", "check", "broadsword", "--claim", &gamma]);
        assert_eq!(check.code, 0, "{}", check.stdout);
        assert!(check.stdout.starts_with("free:"), "{}", check.stdout);
        let o = r.start(&whole, "whole-file");
        assert_eq!(o.code, 3, "{}", o.stderr);
        for want in [a.as_str(), b.as_str(), alpha.as_str(), beta.as_str(), file] {
            assert!(o.stderr.contains(want), "{want} in {}", o.stderr);
        }
        assert_eq!(r.status_of(&whole), "open");
        assert_eq!(r.list(&["--status", "running"]).len(), 2);
        for id in [&a, &b] {
            assert_eq!(r.run(&["task", "done", id]).code, 0);
        }
        let o = r.start(&whole, "whole-file");
        assert_eq!(o.code, 0, "{}", o.stderr);
        let o = r.start(&c, "key-after-whole");
        assert_eq!(o.code, 3, "whole-file holder must block a key: {}", o.stderr);
        assert!(o.stderr.contains(&whole) && o.stderr.contains(&alpha), "{}", o.stderr);
    }
}

#[test]
fn key_claim_on_other_file_is_refused() {
    let r = Root::new("key-claim-invalid");
    for claim in ["crates/broadsword/src/task.rs#k", "config.rs#k", "nested/broadsword.toml#k", "crates/broadsword/tests/support#k", "crates/broadsword/tests/support/mod.rs.bak#k", "crates/broadsword/tests/tasks.rs#k"] {
        let o = r.run(&["task", "add", "broadsword", "--title", "bad key file", "--claim", claim]);
        assert_eq!(o.code, 2, "{claim}: {}", o.stderr);
        assert!(o.stderr.contains("key claims are only for broadsword.toml, crates/broadsword/src/config.rs and crates/broadsword/tests/support/mod.rs"), "{}", o.stderr);
    }
    for key in ["", "Alpha", "a-b", "a/b", "a\\b", "a#b", "a b", "a ", "a\r", "a\n", "é"] {
        let claim = format!("broadsword.toml#{key}");
        let o = r.run(&["task", "add", "broadsword", "--title", "bad key", "--claim", &claim]);
        assert_eq!(o.code, 2, "{claim:?}: {}", o.stderr);
        assert!(o.stderr.contains("key must match") || o.stderr.contains("control character"), "{}", o.stderr);
    }
    assert_eq!(r.list(&[]), ["no tasks"], "invalid claims must not create records");
}

#[test]
fn key_claim_starts_in_a_race_preserve_exclusion_and_independence() {
    for file in ["broadsword.toml", "crates/broadsword/src/config.rs", "crates/broadsword/tests/support/mod.rs"] {
        let alpha = format!("{file}#alpha");
        let beta = format!("{file}#beta");
        let mut cases = vec![(alpha.as_str(), alpha.as_str(), false), (file, alpha.as_str(), false), (alpha.as_str(), file, false), (alpha.as_str(), beta.as_str(), true)];
        if let Some((folder, _)) = file.rsplit_once('/') {
            cases.push((folder, alpha.as_str(), false));
        }
        for (left, right, independent) in cases {
            let r = Root::new("key-race");
            let a = r.add("broadsword", "left", &[left]);
            let b = r.add("broadsword", "right", &[right]);
            let res = start_together(&r, &[(&a, "left".into()), (&b, "right".into())], 0);
            let mut codes: Vec<i32> = res.iter().map(|x| x.0).collect();
            codes.sort_unstable();
            if independent {
                assert_eq!(codes, [0, 0], "{left} vs {right}: {res:?}");
                assert_eq!(r.list(&["--status", "running"]).len(), 2);
            } else {
                assert_eq!(codes, [0, 3], "{left} vs {right}: {res:?}");
                assert_eq!(r.list(&["--status", "running"]).len(), 1);
                let loser = res.iter().find(|x| x.0 == 3).unwrap();
                assert!(loser.1.contains(left) && loser.1.contains(right) && loser.1.contains("overlaps"), "{res:?}");
            }
        }
    }
}

#[test]
fn different_projects_never_conflict() {
    let r = Root::new("projects");
    let a = r.add("p", "a", &["src"]);
    let b = r.add("q", "b", &["src"]);
    let c = r.add("broadsword", "c", &["*"]);
    assert_eq!(r.start(&a, "x").code, 0);
    assert_eq!(r.start(&b, "y").code, 0, "same path in another project is fine");
    assert_eq!(r.start(&c, "z").code, 0, "the whole of broadsword is not the whole of p");
    assert_eq!(r.list(&["--status", "running"]).len(), 3);
    assert_eq!(r.list(&["--project", "q"]).len(), 1);
}

#[test]
fn check_is_read_only_and_reports_conflicts() {
    let r = Root::new("check");
    let a = r.add("p", "a", &["src/engine"]);
    let free = r.run(&["task", "check", "p", "--claim", "src/engine"]);
    assert_eq!(free.code, 0, "open tasks hold nothing: {}", free.stdout);
    assert!(free.stdout.starts_with("free:"));
    assert_eq!(r.start(&a, "coder-1").code, 0);
    let c = r.run(&["task", "check", "p", "--claim", "docs", "--claim", "src"]);
    assert_eq!(c.code, 3, "{}", c.stdout);
    for want in [a.as_str(), "coder-1", "`src`", "`src/engine`"] {
        assert!(c.stdout.contains(want), "{want} in {}", c.stdout);
    }
    assert_eq!(r.run(&["task", "check", "p", "--claim", "docs", "--claim", "srcx"]).code, 0);
    assert_eq!(r.run(&["task", "check", "q", "--claim", "src"]).code, 0, "other project");
    assert_eq!(r.run(&["task", "check", "p", "--claim", "*"]).code, 3);
    assert_eq!(r.run(&["task", "check", "nope", "--claim", "src"]).code, 2);
    assert_eq!(r.run(&["task", "check", "p", "--claim", "../x"]).code, 2);
    assert_eq!(r.status_of(&a), "running", "check changed nothing");
    assert_eq!(r.list(&[]).len(), 1, "check added nothing");
}

#[test]
fn done_fail_and_abandon_all_release_the_claims() {
    for how in ["done", "fail", "abandon"] {
        let r = Root::new(how);
        let a = r.add("p", "a", &["src"]);
        let b = r.add("p", "b", &["src/x.rs"]);
        assert_eq!(r.start(&a, "one").code, 0);
        assert_eq!(r.start(&b, "two").code, 3, "{how}: held before release");
        let o = r.run(&["task", how, &a]);
        assert_eq!(o.code, 0, "{how}: {}", o.stderr);
        assert_eq!(r.start(&b, "two").code, 0, "{how}: released");
        assert_eq!(r.status_of(&a), match how { "fail" => "failed", "abandon" => "abandoned", _ => "done" });
    }
    // abandon also works on a task that never started
    let r = Root::new("abandon-open");
    let a = r.add("p", "a", &["src"]);
    assert_eq!(r.run(&["task", "abandon", &a]).code, 0);
}

#[test]
fn project_names_are_canonical_whatever_the_case_or_trailing_dot() {
    let r = Root::new("canon");
    fs::create_dir_all(r.dir.join("projects/Crash")).unwrap();
    fs::write(r.dir.join("projects/Crash/Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let a = r.add("crash", "a", &["src"]);
    let b = r.add("CRASH", "b", &["src/a.rs"]);
    let c = r.add("Crash", "c", &["src"]);
    for id in [&a, &b, &c] {
        assert!(r.run(&["task", "show", id]).stdout.contains("project: Crash"), "stored under the folder's real name");
    }
    assert_eq!(r.start(&a, "one").code, 0);
    assert_eq!(r.start(&b, "two").code, 3, "`CRASH` and `crash` are the same project");
    assert_eq!(r.start(&c, "three").code, 3);
    for bad in ["Crash.", "crash ", "Crash..", "crash. "] {
        assert_eq!(r.run(&["task", "add", bad, "--title", "t", "--claim", "src"]).code, 2, "`{bad}`");
        assert_eq!(r.run(&["task", "check", bad, "--claim", "src"]).code, 2, "`{bad}`");
    }
    assert_eq!(r.run(&["task", "check", "cRaSh", "--claim", "src/x"]).code, 3);
    assert_eq!(r.list(&["--project", "CRASH"]).len(), 3);
}

/// Rewrite the `project = ...` line of a stored task, as an older Broadsword or a hand edit could have left it.
fn set_stored_project(r: &Root, id: &str, name: &str) {
    let path = r.dir.join("state").join("tasks").join(format!("{id}.task"));
    let text = fs::read_to_string(&path).unwrap();
    let new: Vec<String> = text.lines().map(|l| if l.starts_with("project = ") { format!("project = {name}") } else { l.to_string() }).collect();
    assert_ne!(new.join("\n"), text.trim_end_matches('\n'), "the project line must have changed");
    fs::write(&path, new.join("\n") + "\n").unwrap();
}

#[test]
fn a_stored_project_name_in_another_case_still_holds_its_claims() {
    let r = Root::new("stored-case");
    fs::create_dir_all(r.dir.join("projects/Crash-Sim")).unwrap();
    fs::write(r.dir.join("projects/Crash-Sim/Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
    let a = r.add("crash-sim", "a", &["src"]);
    let b = r.add("Crash-Sim", "b", &["src"]);
    assert_eq!(r.start(&a, "one").code, 0);
    // a record written before names were canonical (or edited by hand) says `crash-sim`, `CRASH-SIM` ...
    set_stored_project(&r, &a, "crash-sim");
    set_stored_project(&r, &b, "CRASH-SIM");
    let o = r.start(&b, "two");
    assert_eq!(o.code, 3, "`crash-sim` and `CRASH-SIM` are one project: {}", o.stderr);
    assert!(o.stderr.contains(a.as_str()), "{}", o.stderr);
    assert_eq!(r.status_of(&b), "open");
    // ... and list --project finds both under the folder's name
    assert_eq!(r.list(&["--project", "Crash-Sim"]).len(), 2);
    assert_eq!(r.list(&["--project", "CRASH-SIM"]).len(), 2);
}

#[test]
fn a_stored_name_with_no_project_folder_is_compared_case_folded() {
    let r = Root::new("stored-gone");
    let a = r.add("p", "a", &["src"]);
    let b = r.add("p", "b", &["src"]);
    set_stored_project(&r, &a, "Gone");
    set_stored_project(&r, &b, "gone");
    assert_eq!(r.start(&a, "one").code, 0);
    assert_eq!(r.start(&b, "two").code, 3, "`Gone` and `gone` must not be two projects");
}
