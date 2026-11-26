//! End-to-end tests for the task board (`broadsword task ...`): the real binary, inside throwaway Broadsword roots under
//! the repo's `scratch/` folder (never the real root). Includes the race tests: the done check "two tasks with
//! overlapping claims never run at once", in miniature.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[allow(dead_code)]
#[path = "../src/json.rs"]
mod j;
use j::{parse, J};

mod support;
use support::bin;
const GATE_ENV: &str = "BROADSWORD_INTERNAL_TEST_TASK_START_GATE_FILE";
static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct Root {
    dir: PathBuf,
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Root {
    fn new(name: &str) -> Root {
        Root::with_wait(name, None)
    }

    fn with_wait(name: &str, wait: Option<u64>) -> Root {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("tasks-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("scratch")).unwrap();
        for p in ["p", "q"] {
            fs::create_dir_all(dir.join("projects").join(p)).unwrap();
            fs::write(dir.join("projects").join(p).join("Cargo.toml"), "[package]\nname = \"x\"\n").unwrap();
        }
        let toml = support::config(&[("mcp_call_timeout_seconds", "600"), ("task_lock_wait_seconds", &wait.unwrap_or(10).to_string())]);
        fs::write(dir.join("broadsword.toml"), toml).unwrap();
        Root { dir }
    }

    fn cmd(&self, args: &[&str]) -> Command {
        let mut c = Command::new(bin());
        c.args(args).current_dir(&self.dir).stdin(Stdio::null());
        c
    }

    fn run(&self, args: &[&str]) -> Out {
        let o = self.cmd(args).output().unwrap();
        Out { code: o.status.code().unwrap(), stdout: String::from_utf8_lossy(&o.stdout).into_owned(), stderr: String::from_utf8_lossy(&o.stderr).into_owned() }
    }

    /// `task add`, which must succeed; returns the id.
    fn add(&self, project: &str, title: &str, claims: &[&str]) -> String {
        let mut a = vec!["task", "add", project, "--title", title];
        for c in claims {
            a.push("--claim");
            a.push(c);
        }
        let o = self.run(&a);
        assert_eq!(o.code, 0, "add failed: {}", o.stderr);
        o.stdout.trim().to_string()
    }

    fn start(&self, id: &str, owner: &str) -> Out {
        self.run(&["task", "start", id, "--owner", owner])
    }

    fn status_of(&self, id: &str) -> String {
        let o = self.run(&["task", "show", id]);
        assert_eq!(o.code, 0, "{}", o.stderr);
        o.stdout.lines().find_map(|l| l.strip_prefix("status: ")).unwrap().to_string()
    }

    fn list(&self, extra: &[&str]) -> Vec<String> {
        let mut a = vec!["task", "list"];
        a.extend_from_slice(extra);
        let o = self.run(&a);
        assert_eq!(o.code, 0, "{}", o.stderr);
        o.stdout.lines().map(String::from).collect()
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn archive_record(r: &Root, id: &str, fields: &[(&str, &str)]) -> PathBuf {
    let path = r.dir.join("state/tasks").join(format!("{id}.task"));
    let text = fs::read_to_string(&path).unwrap();
    let mut lines: Vec<String> = text.lines().filter(|line| {
        !fields.iter().any(|(key, _)| line.split_once(" =").is_some_and(|(k, _)| k == *key))
    }).map(String::from).collect();
    for (key, value) in fields {
        lines.push(format!("{key} = {value}"));
    }
    fs::write(&path, format!("{}\n", lines.join("\n"))).unwrap();
    path
}

#[test]
fn archive_moves_old_ended_tasks_and_show_still_finds_them() {
    let r = Root::new("archive-moves");
    let mut records = Vec::new();
    for status in ["done", "failed", "abandoned"] {
        let id = r.add("p", status, &["src"]);
        // 2000-01-01 00:00:00 UTC, independently fixed rather than derived by the archiver.
        let path = archive_record(&r, &id, &[("status", status), ("ended", "946684800")]);
        records.push((id, path.clone(), fs::read(&path).unwrap()));
    }
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("archive: moved 3, kept 0 ended task(s) younger than 30 days"), "{}", o.stdout);
    for (id, path, bytes) in &records {
        let relative = format!("state/tasks/archive/2000-01/{id}.task");
        assert!(o.stdout.contains(&format!("archived {id} -> {relative}")), "{}", o.stdout);
        assert!(!path.exists());
        assert_eq!(fs::read(r.dir.join(relative)).unwrap(), *bytes, "rename preserves all bytes");
        assert_eq!(r.status_of(id), String::from_utf8_lossy(bytes).lines().find_map(|l| l.strip_prefix("status = ")).unwrap());
    }
    let again = r.run(&["task", "archive"]);
    assert_eq!(again.code, 0, "{}", again.stderr);
    assert!(again.stdout.contains("archive: moved 0, kept 0"), "{}", again.stdout);
    // Exactly one archived match is required; never choose an arbitrary duplicate.
    let (id, _, bytes) = &records[0];
    let duplicate = r.dir.join("state/tasks/archive/2000-02");
    fs::create_dir_all(&duplicate).unwrap();
    fs::write(duplicate.join(format!("{id}.task")), bytes).unwrap();
    let show = r.run(&["task", "show", id]);
    assert_eq!(show.code, 1);
    assert!(show.stderr.contains("multiple archived records"), "{}", show.stderr);
}

#[test]
fn archive_stray_file_does_not_break_task_add_or_show() {
    let r = Root::new("archive-stray-file");
    let old = r.add("p", "old", &["src"]);
    archive_record(&r, &old, &[("status", "done"), ("ended", "946684800")]);
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let archive = r.dir.join("state/tasks/archive");
    let archived = archive.join("2000-01").join(format!("{old}.task"));
    let before = fs::read(&archived).unwrap();
    let readme = archive.join("README");
    fs::write(&readme, b"Archive notes, not a month directory.\n").unwrap();

    assert_eq!(r.status_of(&old), "done");
    let current = r.add("p", "new", &["src"]);
    assert_ne!(current, old);
    assert_eq!(r.status_of(&current), "open");
    assert_eq!(r.status_of(&old), "done");
    assert_eq!(fs::read(&archived).unwrap(), before);
    assert_eq!(fs::read(&readme).unwrap(), b"Archive notes, not a month directory.\n");
}

#[test]
fn archive_never_moves_open_running_or_young_tasks() {
    let r = Root::new("archive-keeps");
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs().to_string();
    let cases = [
        vec![("status", "open"), ("ended", "946684800")],
        vec![("status", "running"), ("ended", "946684800")],
        vec![("status", "done"), ("ended", now.as_str())],
        vec![("status", "failed"), ("ended", "")],
        vec![("status", "abandoned"), ("ended", "946684800"), ("worktree", "worktrees/p/old"), ("worktree_removed", "")],
        vec![("status", "done"), ("ended", "18446744073709551615")],
    ];
    let mut paths = Vec::new();
    for fields in &cases {
        let id = r.add("p", "keep", &["src"]);
        let path = archive_record(&r, &id, fields);
        paths.push((path.clone(), fs::read(&path).unwrap()));
    }
    let queued = r.add("p", "queue protects task", &["src"]);
    let path = archive_record(&r, &queued, &[("status", "done"), ("ended", "946684800")]);
    paths.push((path.clone(), fs::read(&path).unwrap()));
    fs::create_dir_all(r.dir.join("state/queue")).unwrap();
    fs::write(r.dir.join("state/queue/q-test.entry"), format!("id = q-test\ntask = {queued}\nstatus = blocked\n")).unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(o.stdout.contains("archive: moved 0, kept 2 ended task(s) younger than 30 days"), "{}", o.stdout);
    for (path, bytes) in paths {
        assert_eq!(fs::read(path).unwrap(), bytes);
    }
}

#[test]
fn archive_never_overwrites_an_existing_archived_record() {
    let r = Root::new("archive-collision");
    let id = r.add("p", "collision", &["src"]);
    let path = archive_record(&r, &id, &[("status", "done"), ("ended", "946684800")]);
    let before = fs::read(&path).unwrap();
    let dir = r.dir.join("state/tasks/archive/2000-01");
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join(format!("{id}.task"));
    fs::write(&target, b"existing archive bytes\n").unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    let diagnostic = o.stderr.replace('\\', "/");
    assert!(
        diagnostic.contains("target already exists")
            && diagnostic.contains(&format!("state/tasks/{id}.task"))
            && diagnostic.contains(&format!("state/tasks/archive/2000-01/{id}.task")),
        "{}",
        o.stderr
    );
    assert_eq!(fs::read(path).unwrap(), before);
    assert_eq!(fs::read(target).unwrap(), b"existing archive bytes\n");
    assert!(o.stdout.contains("archive: moved 0"), "{}", o.stdout);
}

#[test]
fn a_new_task_never_reuses_an_archived_id() {
    let r = Root::new("archive-id");
    let old = r.add("p", "old", &["src"]);
    archive_record(&r, &old, &[("status", "abandoned"), ("ended", "946684800")]);
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    let archived = r.dir.join("state/tasks/archive/2000-01").join(format!("{old}.task"));
    let bytes = fs::read(&archived).unwrap();
    // No sleep: exercise timestamp reuse when both adds occur in the same second.
    // The fixed-base unit test covers collisions deterministically even across a clock tick here.
    let current = r.add("p", "new", &["src"]);
    assert_ne!(current, old, "an archived id must remain reserved");
    assert_eq!(fs::read(&archived).unwrap(), bytes);
    assert_eq!(r.status_of(&old), "abandoned");
    assert_eq!(r.status_of(&current), "open");
    let listed = r.list(&[]);
    assert!(listed.iter().all(|line| line.split_whitespace().next() != Some(old.as_str())), "{listed:?}");
    assert!(listed.iter().any(|line| line.split_whitespace().next() == Some(current.as_str())), "{listed:?}");
}

#[test]
fn start_and_list_ignore_archived_tasks() {
    let r = Root::new("archive-start-list");
    let old = r.add("p", "old", &["src"]);
    archive_record(&r, &old, &[("status", "abandoned"), ("ended", "946684800")]);
    assert_eq!(r.run(&["task", "archive"]).code, 0);
    let listed = r.list(&[]);
    assert!(listed.iter().all(|line| line.split('\t').next() != Some(old.as_str())), "{listed:?}");
    // If start scanned the archive this deliberately unreadable record would fail its board scan.
    let archived = r.dir.join("state/tasks/archive/2000-01").join(format!("{old}.task"));
    fs::write(&archived, [0xff, 0xfe]).unwrap();
    let current = r.add("p", "new", &["src"]);
    let o = r.start(&current, "coder");
    assert_eq!(o.code, 0, "{}", o.stderr);
    let listed = r.list(&[]);
    assert!(listed.iter().all(|line| line.split('\t').next() != Some(old.as_str())), "{listed:?}");
    assert!(listed.iter().any(|line| line.split_whitespace().next() == Some(current.as_str())), "{listed:?}");
    assert_eq!(r.status_of(&current), "running");
    // A readable archived running record must not hold claims either.
    fs::write(&archived, format!("id = {old}\nstatus = running\nproject = p\nclaims = other\n")).unwrap();
    let other = r.add("p", "other", &["other"]);
    assert_eq!(r.run(&["task", "check", "p", "--claim", "other"]).code, 0);
    assert_eq!(r.start(&other, "coder").code, 0);
    assert_eq!(r.run(&["task", "start", &old, "--owner", "coder"]).code, 1);
}

#[test]
fn archive_fails_closed_on_bad_records_and_busy_board() {
    let r = Root::with_wait("archive-fail-closed", Some(1));
    let id = r.add("p", "old", &["src"]);
    let path = archive_record(&r, &id, &[("status", "done"), ("ended", "946684800")]);
    let before = fs::read(&path).unwrap();
    let held = hold(&r);
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 4);
    assert!(o.stderr.contains("board busy"), "{}", o.stderr);
    assert_eq!(fs::read(&path).unwrap(), before);
    drop(held);
    let broken = r.dir.join("state/tasks/z-broken.task");
    for bytes in [
        vec![0xff, 0xfe],
        Vec::new(),
        b"id = z-broken\nstatus = done\nended = -1\n".to_vec(),
        b"id = z-broken\nstatus = running\nstatus = done\nended = 946684800\n".to_vec(),
        b"id = z-broken\nstatus = running\n status = done\nended = 946684800\n".to_vec(),
        b"id = z-broken\nstatus = done\nended = 1\n ended = 946684800\n".to_vec(),
        b"id = z-broken\nstatus = done\nended = 946684800\n   = ignored\n".to_vec(),
        b"id = z-broken\nstatus = done\nended = +1\n".to_vec(),
    ] {
        fs::write(&broken, bytes).unwrap();
        let o = r.run(&["task", "archive"]);
        assert_eq!(o.code, 1, "{}", o.stderr);
        assert_eq!(fs::read(&path).unwrap(), before, "preflight must finish before any move");
    }
    fs::remove_file(broken).unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 0, "{}", o.stderr);
    assert!(!path.exists());
}

#[test]
fn archive_reports_partial_failure_and_keeps_both_collision_records() {
    let r = Root::new("archive-partial");
    let blocked = r.add("p", "collision", &["src"]);
    let movable = r.add("p", "safe move", &["other"]);
    let blocked_path = archive_record(&r, &blocked, &[("status", "done"), ("ended", "946684800")]);
    let movable_path = archive_record(&r, &movable, &[("status", "failed"), ("ended", "946684800")]);
    let blocked_bytes = fs::read(&blocked_path).unwrap();
    let movable_bytes = fs::read(&movable_path).unwrap();
    let dir = r.dir.join("state/tasks/archive/2000-01");
    fs::create_dir_all(&dir).unwrap();
    let target = dir.join(format!("{blocked}.task"));
    fs::write(&target, b"collision bytes").unwrap();
    let o = r.run(&["task", "archive"]);
    assert_eq!(o.code, 1, "{}", o.stderr);
    assert!(o.stdout.contains("archive: moved 1"), "{}", o.stdout);
    assert_eq!(fs::read(blocked_path).unwrap(), blocked_bytes);
    assert_eq!(fs::read(target).unwrap(), b"collision bytes");
    assert!(!movable_path.exists());
    assert_eq!(fs::read(dir.join(format!("{movable}.task"))).unwrap(), movable_bytes);
}

#[test]
fn full_lifecycle() {
    let r = Root::new("life");
    let id = r.add("p", "  build   the thing ", &["src", "docs\\guide.md"]);
    assert!(id.starts_with("t-"), "{id}");
    assert!(r.dir.join("state/tasks").join(format!("{id}.task")).is_file());
    assert_eq!(r.status_of(&id), "open");
    let l = r.list(&[]);
    assert_eq!(l.len(), 1);
    let cols: Vec<&str> = l[0].split('\t').collect();
    assert_eq!(cols, [id.as_str(), "open", "-", "p", "build the thing", "src, docs/guide.md"]);

    let s = r.start(&id, "coder-1");
    assert_eq!(s.code, 0, "{}", s.stderr);
    assert_eq!(r.list(&["--status", "running"])[0].split('\t').collect::<Vec<_>>()[..4], [id.as_str(), "running", "coder-1", "p"]);
    assert_eq!(r.start(&id, "coder-2").code, 1, "a running task cannot be started again");

    let d = r.run(&["task", "done", &id, "--note", "merged as abc"]);
    assert_eq!(d.code, 0, "{}", d.stderr);
    let show = r.run(&["task", "show", &id]);
    for want in ["status: done", "owner: coder-1", "claims: src, docs/guide.md", "note: merged as abc", "project: p"] {
        assert!(show.stdout.contains(want), "{want} in {}", show.stdout);
    }
    assert!(show.stdout.contains("started: 20") && show.stdout.contains("ended: 20") && show.stdout.contains("created: 20"), "{}", show.stdout);
    assert_eq!(r.run(&["task", "done", &id]).code, 1, "done twice");
    assert_eq!(r.run(&["task", "fail", &id]).code, 1, "a done task cannot fail");
    assert_eq!(r.run(&["task", "abandon", &id]).code, 1, "a done task cannot be abandoned");
    assert_eq!(r.run(&["task", "start", &id, "--owner", "x"]).code, 1, "a done task cannot restart");
    assert!(r.dir.join("state/tasks").join(format!("{id}.task")).is_file(), "records are kept");
    assert!(hold_is_free(&r), "the board lock is released (the file itself stays)");
}

#[test]
fn only_a_running_task_can_finish_and_open_can_be_abandoned() {
    let r = Root::new("states");
    let a = r.add("p", "a", &["a"]);
    assert_eq!(r.run(&["task", "done", &a]).code, 1, "open -> done is refused");
    assert_eq!(r.run(&["task", "fail", &a]).code, 1, "open -> failed is refused");
    assert_eq!(r.run(&["task", "abandon", &a]).code, 0);
    assert_eq!(r.status_of(&a), "abandoned");
    assert_eq!(r.start(&a, "x").code, 1, "abandoned tasks never restart");
    assert_eq!(r.run(&["task", "abandon", &a]).code, 1);
}

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
fn bad_input_is_refused_with_the_documented_codes() {
    let r = Root::new("bad");
    let o = r.run(&["task", "add", "nope", "--title", "t", "--claim", "src"]);
    assert_eq!(o.code, 2);
    assert!(o.stderr.contains("unknown project") && o.stderr.contains("broadsword"), "{}", o.stderr);
    for bad in ["", "..", "../x", "a/../../x", "/abs", "C:\\x", "src/*", "*.rs", "."] {
        let o = r.run(&["task", "add", "p", "--title", "t", "--claim", bad]);
        assert_eq!(o.code, 2, "claim `{bad}`: {}", o.stderr);
        assert!(o.stderr.contains("invalid claim"), "{}", o.stderr);
    }
    assert_eq!(r.run(&["task", "add", "p", "--title", "  ", "--claim", "src"]).code, 2);
    assert_eq!(r.run(&["task", "add", "p", "--title", "t"]).code, 64, "no claims: not a command line");
    assert_eq!(r.run(&["task", "add", "p", "--claim", "src"]).code, 64, "no title");
    assert_eq!(r.list(&[]), ["no tasks"], "nothing was created");
    assert_eq!(r.run(&["task", "list", "--status", "weird"]).code, 2);
    assert_eq!(r.run(&["task", "show", "t-nope"]).code, 1);
    assert_eq!(r.run(&["task", "show", "../x"]).code, 1);
    assert_eq!(r.start("t-nope", "x").code, 1);
    let a = r.add("p", "a", &["*"]);
    assert_eq!(r.run(&["task", "start", &a, "--owner", "  "]).code, 2);
    assert_eq!(r.status_of(&a), "open");
    // `*` as a whole-project claim is fine
    assert_eq!(r.start(&a, "x").code, 0);
}

#[test]
fn ids_are_unique_when_tasks_are_added_at_the_same_time() {
    let r = Root::new("ids");
    let mut kids: Vec<Child> = (0..8)
        .map(|i| r.cmd(&["task", "add", "p", "--title", &format!("t{i}"), "--claim", "src"]).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap())
        .collect();
    let mut ids = Vec::new();
    for k in kids.drain(..) {
        let o = k.wait_with_output().unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        ids.push(String::from_utf8(o.stdout).unwrap().trim().to_string());
    }
    let set: std::collections::HashSet<&String> = ids.iter().collect();
    assert_eq!(set.len(), 8, "{ids:?}");
    assert_eq!(r.list(&[]).len(), 8);
}

#[test]
fn worktrees_share_the_main_roots_board() {
    let r = Root::new("wt");
    let wt = r.dir.join("wt").join("a");
    let gitdir = r.dir.join(".git").join("worktrees").join("a");
    fs::create_dir_all(&gitdir).unwrap();
    fs::create_dir_all(&wt).unwrap();
    fs::copy(r.dir.join("broadsword.toml"), wt.join("broadsword.toml")).unwrap();
    fs::write(wt.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
    let from_wt = |args: &[&str]| {
        let o = Command::new(bin()).args(args).current_dir(&wt).output().unwrap();
        (o.status.code().unwrap(), String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let (code, id) = from_wt(&["task", "add", "p", "--title", "from worktree", "--claim", "src"]);
    assert_eq!(code, 0);
    assert!(r.dir.join("state/tasks").join(format!("{id}.task")).is_file(), "the record is in the main root's state");
    assert!(!wt.join("state").exists());
    let b = r.add("p", "from main", &["src/a.rs"]);
    assert_eq!(from_wt(&["task", "start", &id, "--owner", "wt-agent"]).0, 0);
    assert_eq!(r.start(&b, "main-agent").code, 3, "main sees the worktree's running task");
}
fn lock_path(r: &Root) -> PathBuf {
    r.dir.join("state/tasks/board.lock")
}

/// Hold the board lock the way another process would: an OS lock on an open handle.
fn hold(r: &Root) -> fs::File {
    fs::create_dir_all(r.dir.join("state/tasks")).unwrap();
    let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(r)).unwrap();
    f.lock().unwrap();
    f
}

#[test]
fn garbage_in_the_lock_file_is_ignored_and_a_live_holder_means_board_busy() {
    let r = Root::with_wait("lock", Some(1));
    let a = r.add("p", "a", &["src"]);
    // whatever the file says, only a real OS lock counts: no stale detection exists
    for junk in ["4000000000\n\n", "", "garbage", &format!("{}\n\n", std::process::id())] {
        fs::write(lock_path(&r), junk).unwrap();
        assert_eq!(r.run(&["task", "show", &a]).code, 0);
    }
    fs::write(lock_path(&r), "4000000000\n\n").unwrap();
    assert_eq!(r.start(&a, "x").code, 0, "a garbage lock file does not block");
    assert!(lock_path(&r).exists(), "the lock file is never deleted");
    let b = r.add("p", "b", &["lib"]);
    let held = hold(&r);
    let t = Instant::now();
    let o = r.start(&b, "y");
    assert_eq!(o.code, 4, "{}", o.stderr);
    assert!(o.stderr.contains("board busy") && !o.stderr.contains("delete"), "{}", o.stderr);
    assert!(t.elapsed() >= Duration::from_millis(900), "it waited for the lock");
    assert_eq!(r.status_of(&b), "open", "nothing changed");
    drop(held);
    assert_eq!(r.start(&b, "y").code, 0);
}

/// The test binary doubles as the lock holder for the kill test: with the env var set, this "test" takes the board lock,
/// announces it with a marker file and sleeps until killed. Without the env var it does nothing.
#[test]
fn lock_holder_helper() {
    let Ok(dir) = std::env::var("BROADSWORD_TEST_HOLD_LOCK_ROOT") else { return };
    let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(Path::new(&dir).join("state/tasks/board.lock")).unwrap();
    f.lock().unwrap();
    fs::write(Path::new(&dir).join("scratch/held"), "x").unwrap();
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn a_killed_lock_holder_frees_the_board_at_once() {
    // Probe the OS lock directly after the kill: timing a task start also counts process startup under load.
    // Keep the 20 s wait and exit-code checks to prove a live holder blocks and a start after the kill succeeds.
    let r = Root::with_wait("kill", Some(20));
    let a = r.add("p", "a", &["src"]);
    let mut holder = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_holder_helper", "--nocapture"])
        .env("BROADSWORD_TEST_HOLD_LOCK_ROOT", &r.dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while !r.dir.join("scratch/held").exists() {
        assert!(Instant::now() < end, "the holder never took the lock");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(r.start(&a, "x").code, 4, "a live holder blocks");
    let k = Command::new("taskkill").args(["/F", "/PID", &holder.id().to_string()]).output().unwrap();
    assert!(k.status.success(), "taskkill: {}", String::from_utf8_lossy(&k.stderr));
    let _ = holder.wait();
    let f = fs::OpenOptions::new().read(true).write(true).open(lock_path(&r)).unwrap();
    if let Err(err) = f.try_lock() {
        panic!("the killed holder's board lock was still held: {err}");
    }
    drop(f);
    let o = r.start(&a, "x");
    assert_eq!(o.code, 0, "{}", o.stderr);
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

#[test]
fn sixteen_gated_starts_with_a_garbage_lock_and_a_big_board_never_give_two_running() {
    // The reviewer's repro: a garbage lock file, 16 simultaneous starts all claiming `src`, and thousands of records so the
    // critical section is slow. Exactly one runs every round and no start is left blocked (no exit 4).
    // All 16 starts serialize on one lock, each scanning 3000+ records; under load the line exceeded 30 s.
    // A garbage-lock regression still blocks every start and still fails after the 120 s wait.
    const ROUNDS: usize = 8;
    const FILLER: usize = 3000;
    let r = Root::with_wait("race16", Some(120));
    let dir = r.dir.join("state/tasks");
    fs::create_dir_all(&dir).unwrap();
    for i in 0..FILLER {
        fs::write(dir.join(format!("filler-{i}.task")), format!("id = filler-{i}\nproject = p\nstatus = done\nclaims = src\ncreated = {i}\ntitle = filler\n")).unwrap();
    }
    let t0 = Instant::now();
    let mut blocked = 0;
    for round in 0..ROUNDS {
        fs::write(lock_path(&r), "4000000000\n\n").unwrap(); // stale-looking garbage
        let ids: Vec<String> = (0..16).map(|i| r.add("p", &format!("t{i}"), &["src", &format!("own/f{i}")])).collect();
        let who: Vec<(&str, String)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), format!("agent-{i}"))).collect();
        let res = start_together(&r, &who, round);
        let codes: Vec<i32> = res.iter().map(|x| x.0).collect();
        blocked += codes.iter().filter(|c| **c == 4).count();
        assert_eq!(codes.iter().filter(|c| **c == 0).count(), 1, "round {round}: exactly one runs: {codes:?}");
        assert_eq!(codes.iter().filter(|c| **c == 3).count(), 15, "round {round}: fifteen refused with exit 3: {codes:?}");
        assert_eq!(r.list(&["--status", "running"]).len(), 1, "round {round}: running must be exactly 1");
        for id in &ids {
            fs::remove_file(dir.join(format!("{id}.task"))).unwrap();
        }
    }
    assert_eq!(blocked, 0, "no start may be left blocked");
    eprintln!("race16: {ROUNDS} rounds x 16 gated starts, {FILLER} filler records, garbage lock file each round: exactly one running every round, {blocked} blocked starts, {:.1} s", t0.elapsed().as_secs_f64());
}


// ---- MCP ---------------------------------------------------------------------------------

/// One `tools/call` over a fresh `broadsword mcp` server. Returns (isError, text, structured exit_code).
fn mcp_call(r: &Root, name: &str, args: &str) -> (bool, String, i64) {
    let mut child = Command::new(bin()).arg("mcp").current_dir(&r.dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"t","version":"0"}}}}}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#).unwrap();
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"{name}","arguments":{args}}}}}"#).unwrap();
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    let text = String::from_utf8(out.stdout).unwrap();
    let resp = text.lines().filter_map(|l| parse(l).ok()).find(|v| v.get("id") == Some(&J::Num("2".into()))).unwrap_or_else(|| panic!("no response to {name}: {text}"));
    if let Some(e) = resp.get("error") {
        return (true, e.to_json(), -2); // protocol-level error
    }
    let res = resp.get("result").unwrap();
    let Some(J::Arr(c)) = res.get("content") else { panic!("{}", resp.to_json()) };
    let body = c[0].get("text").and_then(J::str).unwrap().to_string();
    let code = match res.get("structuredContent").and_then(|s| s.get("exit_code")) {
        Some(J::Num(n)) => n.parse().unwrap(),
        _ => -1,
    };
    (matches!(res.get("isError"), Some(J::Bool(true))), body, code)
}

#[test]
fn the_task_tools_work_over_mcp() {
    let r = Root::new("mcp");
    let (err, body, code) = mcp_call(&r, "task_add", r#"{"project":"p","title":"first","claims":["src","docs/"]}"#);
    assert!((err, code) == (false, 0), "{body}");
    let id = body.lines().next().unwrap().trim().to_string();
    assert!(id.starts_with("t-"), "{body}");
    let (_, body, _) = mcp_call(&r, "task_add", r#"{"project":"p","title":"second","claims":["src/a.rs"]}"#);
    let id2 = body.lines().next().unwrap().trim().to_string();
    assert_ne!(id, id2);

    let (err, _, code) = mcp_call(&r, "task_check", r#"{"project":"p","claims":["src/a.rs"]}"#);
    assert_eq!((err, code), (false, 0));
    let (err, body, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{id}","owner":"agent-a"}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    let (err, body, code) = mcp_call(&r, "task_check", r#"{"project":"p","claims":["src/a.rs"]}"#);
    assert_eq!((err, code), (true, 3), "{body}");
    assert!(body.contains(&id) && body.contains("agent-a"), "{body}");
    let (err, body, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{id2}","owner":"agent-b"}}"#));
    assert_eq!((err, code), (true, 3), "{body}");
    assert!(body.contains(&id) && body.contains("agent-a") && body.contains("`src/a.rs`") && body.contains("`src`"), "{body}");

    let (_, body, code) = mcp_call(&r, "task_list", r#"{"status":"running"}"#);
    assert_eq!(code, 0);
    assert!(body.contains(&id) && !body.contains(&id2), "{body}");
    let (_, body, _) = mcp_call(&r, "task_show", &format!(r#"{{"id":"{id}"}}"#));
    assert!(body.contains("status: running") && body.contains("owner: agent-a"), "{body}");

    let (err, body, code) = mcp_call(&r, "task_done", &format!(r#"{{"id":"{id}","note":"all good"}}"#));
    assert_eq!((err, code), (false, 0), "{body}");
    let (err, body, code) = mcp_call(&r, "task_start", &format!(r#"{{"id":"{id2}","owner":"agent-b"}}"#));
    assert_eq!((err, code), (false, 0), "released claims: {body}");
    let (err, _, code) = mcp_call(&r, "task_fail", &format!(r#"{{"id":"{id2}","note":"broke"}}"#));
    assert_eq!((err, code), (false, 0));
    let (_, body, _) = mcp_call(&r, "task_list", r#"{"project":"p"}"#);
    assert!(body.contains("done") && body.contains("failed"), "{body}");
    let (_, _, code) = mcp_call(&r, "task_abandon", &format!(r#"{{"id":"{id2}"}}"#));
    assert_eq!(code, 1, "a failed task cannot be abandoned");
    let (err, body, _) = mcp_call(&r, "task_add", r#"{"project":"p","title":"x","claims":[]}"#);
    assert!(err && body.contains("empty"), "{body}");
    let (err, body, code) = mcp_call(&r, "task_add", r#"{"project":"p","title":"x","claims":["../x"]}"#);
    assert_eq!((err, code), (true, 2), "{body}");
    // the CLI sees what MCP did
    assert_eq!(r.status_of(&id), "done");
    assert_eq!(r.status_of(&id2), "failed");
}

// ---- the race: the done check in miniature -----------------------------------------------

/// Spawn one `task start` per (id, owner), all parked on a go-file, then release them together. Returns their exit codes
/// and stderr.
fn start_together(r: &Root, who: &[(&str, String)], round: usize) -> Vec<(i32, String)> {
    let gate = r.dir.join(format!("scratch/go-{round}"));
    let kids: Vec<Child> = who
        .iter()
        .map(|(id, owner)| r.cmd(&["task", "start", id, "--owner", owner]).env(GATE_ENV, &gate).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap())
        .collect();
    std::thread::sleep(Duration::from_millis(150)); // let every process start and park on the gate
    fs::write(&gate, "go").unwrap();
    kids.into_iter()
        .map(|k| {
            let o = k.wait_with_output().unwrap();
            (o.status.code().unwrap(), String::from_utf8_lossy(&o.stderr).into_owned())
        })
        .collect()
}

#[test]
fn two_overlapping_starts_in_a_race_one_wins_every_round() {
    const ROUNDS: usize = 60;
    let r = Root::new("race2");
    let t0 = Instant::now();
    let mut wins_first = 0;
    for round in 0..ROUNDS {
        let _ = fs::remove_dir_all(r.dir.join("state/tasks")); // reset the board
        let (ca, cb) = if round % 2 == 0 { (["src"], ["src/a.rs"]) } else { (["src/a.rs"], ["src"]) };
        let a = r.add("p", "a", &ca);
        let b = r.add("p", "b", &cb);
        let res = start_together(&r, &[(&a, "agent-a".into()), (&b, "agent-b".into())], round);
        let mut codes: Vec<i32> = res.iter().map(|x| x.0).collect();
        assert_eq!(res.iter().filter(|x| x.0 == 0).count(), 1, "round {round}: exactly one must win, got {codes:?}: {res:?}");
        let loser = res.iter().find(|x| x.0 != 0).unwrap();
        assert_eq!(loser.0, 3, "round {round}: the loser must get exit 3: {loser:?}");
        assert!(loser.1.contains("agent-") && loser.1.contains("overlaps"), "round {round}: {}", loser.1);
        let running = r.list(&["--status", "running"]);
        assert_eq!(running.len(), 1, "round {round}: exactly one task is running: {running:?}");
        if res[0].0 == 0 {
            wins_first += 1;
        }
        codes.sort_unstable();
        assert_eq!(codes, [0, 3]);
    }
    eprintln!("race2: {ROUNDS} rounds of 2 simultaneous `task start`, exactly one winner each round, in {:.1} s ({wins_first} won by the first-launched process)", t0.elapsed().as_secs_f64());
}

#[test]
fn eight_mutually_overlapping_starts_in_a_race_exactly_one_runs() {
    const ROUNDS: usize = 10;
    let r = Root::new("race8");
    let t0 = Instant::now();
    for round in 0..ROUNDS {
        let _ = fs::remove_dir_all(r.dir.join("state/tasks"));
        // every task claims `shared` plus its own file, so each pair overlaps through `shared` only
        let ids: Vec<String> = (0..8).map(|i| r.add("p", &format!("t{i}"), &["shared", &format!("own/f{i}.rs")])).collect();
        let who: Vec<(&str, String)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), format!("agent-{i}"))).collect();
        let res = start_together(&r, &who, round);
        let codes: Vec<i32> = res.iter().map(|x| x.0).collect();
        assert_eq!(codes.iter().filter(|c| **c == 0).count(), 1, "round {round}: exactly one runs: {res:?}");
        assert_eq!(codes.iter().filter(|c| **c == 3).count(), 7, "round {round}: the other seven get exit 3: {res:?}");
        assert_eq!(r.list(&["--status", "running"]).len(), 1, "round {round}");
        assert_eq!(r.list(&["--status", "open"]).len(), 7, "round {round}");
    }
    eprintln!("race8: {ROUNDS} rounds of 8 simultaneous `task start` on 8 mutually overlapping tasks, exactly one running each round, in {:.1} s", t0.elapsed().as_secs_f64());
}

#[test]
fn non_overlapping_starts_in_a_race_all_run() {
    let r = Root::new("race-free");
    let ids: Vec<String> = (0..6).map(|i| r.add("p", &format!("t{i}"), &[&format!("mod{i}")])).collect();
    let who: Vec<(&str, String)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), format!("agent-{i}"))).collect();
    let res = start_together(&r, &who, 0);
    assert!(res.iter().all(|x| x.0 == 0), "disjoint claims must all start: {res:?}");
    assert_eq!(r.list(&["--status", "running"]).len(), 6);
}

/// True if nobody holds the board lock right now.
fn hold_is_free(r: &Root) -> bool {
    let f = fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(lock_path(r)).unwrap();
    f.try_lock().is_ok()
}

#[path = "tasks/caps.rs"]
mod caps;
