//! The task board: `broadsword task add|start|done|fail|abandon|list|show|check`.
//!
//! One durable record per task, `<state_dir>/tasks/<id>.task` (the same atomic `key = value` records as jobs, see
//! record.rs; never deleted by cleanup). A task names a project and the file or folder paths it claims. The one
//! rule the board enforces: two tasks whose claims overlap, in the same project, are never `running` at once.
//!
//! Status flow: `open` -> `running` -> `done` | `failed`, and `abandoned` from `open` or `running`.
//! Only `running` tasks hold their claims. `add` is free (no conflict check); the check happens in `start`.
//!
//! `start` is the atomic part: under `<state_dir>/tasks/board.lock` (lock.rs, the same lock `recover` uses) it
//! reads every task, refuses if a running task conflicts, and marks the task running. Every other change of a
//! task's status takes the same lock, so two processes can never both move one task. The lock is held for
//! milliseconds, never while a task "runs": tasks are long-lived agent work, not processes.
//!
//! Claims are paths relative to the project root, never patterns. See `normalize_claim` and `overlaps`.
//!
//! Caps checked by `start`: `max_agents` counts running tasks with a working coder, i.e. running tasks WITHOUT an open
//! merge-queue entry that waits (`queued`, `running` or `approved`, see `WAITING_QUEUE_STATUSES`; a `blocked` or `stale` entry needs
//! its coder, so it counts); `max_queued` counts every open merge-queue entry (see `OPEN_QUEUE_STATUSES`). The queue folder is read
//! without the queue locks, and an unusable entry file fails the start.

use crate::config::Config;
use crate::job;
use crate::lock;
use crate::pause;
use crate::queue;
use crate::record::{self, Record};
use crate::worktree;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

pub const OPEN: &str = "open";
pub const RUNNING: &str = "running";
pub const DONE: &str = "done";
pub const FAILED: &str = "failed";
pub const ABANDONED: &str = "abandoned";
pub const STATUSES: [&str; 5] = [OPEN, RUNNING, DONE, FAILED, ABANDONED];

const EXT: &str = "task";
pub const EXIT_ERROR: u8 = 1;
pub const EXIT_BAD_INPUT: u8 = 2;
pub const EXIT_CONFLICT: u8 = 3;
pub const EXIT_BUSY: u8 = 4;

/// The claim that means the whole project.
pub const WHOLE_PROJECT: &str = "*";

mod archive;
pub use archive::cmd_archive;
mod start;
pub use start::{cmd_start, wait_for_gate};
mod view;
pub use view::{cmd_check, cmd_list, cmd_show};
mod claims;
pub use claims::{normalize_claims, overlaps};
#[cfg(test)]
pub use claims::{KEY_CLAIM_FILES, normalize_claim, overlaps_with};

// ---- conflicts and transitions ----------------------------------------------------------

/// One running task that conflicts with a would-be task, and the first claim pair that overlaps.
#[derive(Debug, PartialEq, Eq)]
pub struct Conflict {
    pub id: String,
    pub owner: String,
    pub mine: String,
    pub theirs: String,
}

pub fn claims_of(r: &Record) -> Vec<String> {
    r.get("claims").split('\n').filter(|c| !c.is_empty()).map(String::from).collect()
}

/// Running tasks in the same project, other than `skip_id`, that overlap any of `claims`.
pub fn find_conflicts(project: &str, claims: &[String], tasks: &[Record], skip_id: Option<&str>) -> Vec<Conflict> {
    let mut out = Vec::new();
    for t in tasks {
        if t.status() != RUNNING || t.get("project") != project || Some(t.id()) == skip_id {
            continue;
        }
        let theirs = claims_of(t);
        let pair = claims.iter().find_map(|m| theirs.iter().find(|o| overlaps(m, o)).map(|o| (m.clone(), o.clone())));
        if let Some((mine, theirs)) = pair {
            out.push(Conflict { id: t.id().to_string(), owner: t.get("owner").to_string(), mine, theirs });
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Done,
    Fail,
    Abandon,
}

/// The state machine: the new status, or why the action is refused.
pub fn transition(from: &str, action: Action) -> Result<&'static str, String> {
    let (ok, to, need): (&[&str], &str, &str) = match action {
        Action::Start => (&[OPEN], RUNNING, "open"),
        Action::Done => (&[RUNNING], DONE, "running"),
        Action::Fail => (&[RUNNING], FAILED, "running"),
        Action::Abandon => (&[OPEN, RUNNING], ABANDONED, "open or running"),
    };
    if ok.contains(&from) {
        Ok(STATUSES.iter().find(|s| **s == to).unwrap())
    } else {
        let verb = match action {
            Action::Start => "started",
            Action::Done => "marked done",
            Action::Fail => "marked failed",
            Action::Abandon => "abandoned",
        };
        Err(format!("the task is `{from}`; only a task that is {need} can be {verb}"))
    }
}

// ---- storage ----------------------------------------------------------------------------

pub fn tasks_dir(cfg: &Config) -> PathBuf {
    cfg.state_dir.join("tasks")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The ONE place a stored project name is normalised: the folder name `broadsword projects` lists (job::canonical_project,
/// case-insensitive), or, when the name matches no project folder, the case-folded name (so `Gone` and `gone` still agree).
/// Every task record passes through here when it is loaded, so claims, conflicts and `list --project` only ever see
/// normalised names, whatever an old or hand-edited record says.
pub fn project_key(cfg: &Config, name: &str) -> String {
    job::canonical_project(&cfg.projects_dir, name).unwrap_or_else(|_| name.to_lowercase())
}

fn normalise_record(cfg: &Config, r: &mut Record) {
    let key = project_key(cfg, r.get("project"));
    if key != r.get("project") {
        r.set("project", key);
    }
}

/// Every task. A record that cannot be read is an error (never silently skipped: `start` decides on this list);
/// an empty file, which only exists for an instant while `add` creates a task, is skipped.
pub fn load_tasks(cfg: &Config) -> Result<Vec<Record>, String> {
    let dir = tasks_dir(cfg);
    let mut v = Vec::new();
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        // a board that was never used has no folder yet: that, and only that, is an empty board
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(v),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    for e in rd {
        let e = e.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let p = e.path();
        if p.extension().is_none_or(|x| x != EXT) {
            continue;
        }
        let mut last = String::new();
        let mut rec = None;
        for _ in 0..5 {
            match record::load(&p) {
                Ok(r) => {
                    rec = Some(r);
                    break;
                }
                Err(e) => {
                    last = e;
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
        }
        match rec {
            Some(mut r) if !r.id().is_empty() => {
                normalise_record(cfg, &mut r);
                v.push(r)
            }
            Some(_) => {}
            None => return Err(last),
        }
    }
    v.sort_by(|a, b| (a.get_num("created"), a.id().len(), a.id()).cmp(&(b.get_num("created"), b.id().len(), b.id())));
    Ok(v)
}

pub fn load_task(cfg: &Config, id: &str) -> Result<Record, String> {
    let nope = || format!("no task with the exact id `{id}` (see `broadsword task list`)");
    if id.is_empty() || record::sanitize(id) != id {
        return Err(nope());
    }
    let mut path = tasks_dir(cfg).join(format!("{id}.{EXT}"));
    match std::fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            path = archive::find(cfg, id)?.ok_or_else(nope)?;
        }
        Err(e) => return Err(format!("cannot inspect {}: {e}", path.display())),
        Ok(_) => {}
    }
    match record::load(&path) {
        Ok(mut r) if r.id() == id => {
            normalise_record(cfg, &mut r);
            Ok(r)
        }
        _ => Err(nope()),
    }
}

pub mod queue_view;
use queue_view::{WAITING_QUEUE_STATUSES, open_queue_entries};
#[cfg(test)]
use queue_view::{ENDED_QUEUE_STATUSES, OPEN_QUEUE_STATUSES};

pub fn save(r: &Record) -> Result<(), String> {
    r.save().map_err(|e| format!("cannot write task record {}: {e}", r.path.display()))
}

/// The board lock. `Err` is already an exit code plus message.
pub fn board_lock(cfg: &Config) -> Result<lock::Lock, ExitCode> {
    let path = tasks_dir(cfg).join("board.lock");
    let wait = cfg.task_lock_wait_seconds;
    lock::acquire(&path, Duration::from_secs(wait), &move |owner| {
        let who = owner.map(|p| format!(" (pid {p})")).unwrap_or_default();
        format!("board busy: another task command{who} held the board for {wait} s; nothing was changed, try again in a moment (the lock frees itself when its holder ends)")
    })
    .map_err(|e| {
        eprintln!("broadsword: {e}");
        ExitCode::from(if e.starts_with("board busy") { EXIT_BUSY } else { EXIT_ERROR })
    })
}

pub fn fail(code: u8, msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("broadsword: {msg}");
    ExitCode::from(code)
}

/// The canonical project name (job::canonical_project, the one place names are accepted); everything stored or
/// compared uses it, so `Crash`, `crash` and `Crash.` can never be different projects.
fn check_project(cfg: &Config, project: &str) -> Result<String, ExitCode> {
    match job::canonical_project(&cfg.projects_dir, project) {
        Ok(c) => Ok(c),
        Err(e) => Err(fail(EXIT_BAD_INPUT, format!("{e}; known projects: {}", job::list_projects(cfg).join(", ")))),
    }
}

fn describe(c: &Conflict) -> String {
    format!("task {} (owner {}): your claim `{}` overlaps its claim `{}`", c.id, if c.owner.is_empty() { "-" } else { &c.owner }, c.mine, c.theirs)
}

// ---- commands ---------------------------------------------------------------------------

pub fn cmd_add(cfg: &Config, project: &str, title: &str, claims: &[&str]) -> ExitCode {
    let project = &match check_project(cfg, project) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let title =title.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        return fail(EXIT_BAD_INPUT, "the title must not be empty");
    }
    let claims = match normalize_claims(claims) {
        Ok(c) => c,
        Err(e) => return fail(EXIT_BAD_INPUT, e),
    };
    let _lock = match board_lock(cfg) {
        Ok(l) => l,
        Err(code) => return code,
    };
    let mut r = match archive::create_task(cfg, &format!("t-{}", job::file_stamp(now_secs()))) {
        Ok(r) => r,
        Err(e) => return fail(EXIT_ERROR, format!("cannot create a task record: {e}")),
    };
    r.set("title", title);
    r.set("project", project);
    r.set("claims", claims.join("\n"));
    r.set("status", OPEN);
    r.set("created", now_secs().to_string());
    if let Err(e) = save(&r) {
        return fail(EXIT_ERROR, e);
    }
    println!("{}", r.id());
    ExitCode::SUCCESS
}

/// done, fail and abandon: change the status under the board lock and release the claims.
pub fn cmd_finish(cfg: &Config, id: &str, action: Action, note: Option<&str>) -> ExitCode {
    let t = {
        let _lock = match board_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        let mut t = match load_task(cfg, id) {
            Ok(t) => t,
            Err(e) => return fail(EXIT_ERROR, e),
        };
        let to = match transition(t.status(), action) {
            Ok(s) => s,
            Err(e) => return fail(EXIT_ERROR, format!("cannot change {id}: {e}")),
        };
        if worktree::is_creating(&t) {
            if action == Action::Done {
                return fail(EXIT_ERROR, format!("cannot mark {id} done: its worktree is still being created (or the creator crashed). Wait, or run `broadsword worktree create {id}` to finish it, or `task fail|abandon {id}`"));
            }
            worktree::settle_creating(cfg, &mut t);
        }
        t.set("status", to);
        t.set("ended", now_secs().to_string());
        if let Some(n) = note {
            t.set("note", n.split_whitespace().collect::<Vec<_>>().join(" "));
        }
        if let Err(e) = save(&t) {
            return fail(EXIT_ERROR, e);
        }
        println!("task {id} is {to}; its claims are released");
        t
    }; // lock released: removing a worktree can take a while and must not block other starts
    // `done` cleans up the task's worktree, but only when that is safe (see worktree.rs). Either way the task is done:
    // a kept worktree is a warning, not an error, so the exit code stays 0.
    if action == Action::Done && worktree::has_worktree(&t) {
        return worktree::after_done(cfg, &t);
    }
    ExitCode::SUCCESS
}

// ---- tests ------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The queue statuses `start` knows must be exactly queue.rs's: every `pub const X: &str = "..."` status constant is in
    /// OPEN_QUEUE_STATUSES or ENDED_QUEUE_STATUSES (once), and queue.rs's `is_open` names exactly the open ones. A status added
    /// to queue.rs, or a change of what it treats as open, fails here until task.rs is updated.
    #[test]
    fn queue_status_lists_match_queue_rs() {
        let src = crate::queue::SOURCE;
        let mut consts: Vec<(String, String)> = Vec::new(); // (NAME, value)
        for l in src.lines() {
            let Some(rest) = l.strip_prefix("pub const ") else { continue };
            let Some((name, val)) = rest.split_once(": &str = \"") else { continue };
            consts.push((name.to_string(), val.trim_end_matches("\";").to_string()));
        }
        let mut known: Vec<&str> = OPEN_QUEUE_STATUSES.iter().chain(ENDED_QUEUE_STATUSES.iter()).copied().collect();
        let mut theirs: Vec<&str> = consts.iter().map(|(_, v)| v.as_str()).collect();
        known.sort_unstable();
        theirs.sort_unstable();
        assert_eq!(known.len(), OPEN_QUEUE_STATUSES.len() + ENDED_QUEUE_STATUSES.len());
        known.dedup();
        assert_eq!(known.len(), OPEN_QUEUE_STATUSES.len() + ENDED_QUEUE_STATUSES.len(), "a status is both open and ended");
        assert_eq!(known, theirs, "queue.rs's status constants changed: update OPEN_QUEUE_STATUSES / ENDED_QUEUE_STATUSES");
        let body = src.split("fn is_open(status: &str) -> bool {").nth(1).expect("queue.rs has fn is_open").split('}').next().unwrap();
        let named: Vec<&str> = body.split(|c: char| !(c.is_ascii_uppercase() || c == '_')).filter(|w| !w.is_empty()).collect();
        let mut open: Vec<&str> = named.iter().map(|n| consts.iter().find(|(c, _)| c == n).map(|(_, v)| v.as_str()).unwrap_or_else(|| panic!("is_open names `{n}`"))).collect();
        let mut mine = OPEN_QUEUE_STATUSES.to_vec();
        open.sort_unstable();
        mine.sort_unstable();
        assert_eq!(open, mine, "queue.rs's is_open changed: update OPEN_QUEUE_STATUSES");
        assert!(WAITING_QUEUE_STATUSES.iter().all(|w| OPEN_QUEUE_STATUSES.contains(w)), "every waiting status is open");
        assert!(!WAITING_QUEUE_STATUSES.contains(&queue::BLOCKED) && !WAITING_QUEUE_STATUSES.contains(&queue::STALE), "blocked and stale need their coder");
        let mut waiting = WAITING_QUEUE_STATUSES.to_vec();
        waiting.sort_unstable();
        let mut expected = vec![queue::QUEUED, queue::RUNNING, queue::APPROVED];
        expected.sort_unstable();
        assert_eq!(waiting, expected, "the waiting set changed: an approved entry waits for a merge train, not for its coder");
    }

    /// `start` decides the overlap check on this list: a tasks folder that cannot be listed is an error, never an empty board.
    /// Only a folder that does not exist yet (a board never used) is an empty board.
    #[test]
    fn load_tasks_fails_loudly_when_the_tasks_folder_cannot_be_listed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-load-tasks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cfg = Config::from_text(include_str!("../../../broadsword.toml"), &root).unwrap();
        // no state/tasks folder at all: an empty board
        assert!(load_tasks(&cfg).unwrap().is_empty());
        // a FILE where the folder should be: read_dir fails with something other than NotFound
        std::fs::create_dir_all(&cfg.state_dir).unwrap();
        std::fs::write(tasks_dir(&cfg), "not a folder").unwrap();
        let e = load_tasks(&cfg).expect_err("an unlistable tasks folder must be an error");
        assert!(e.starts_with("cannot read ") && e.contains("tasks"), "{e}");
        let _ = std::fs::remove_dir_all(&root);
    }

    fn n(s: &str) -> String {
        normalize_claim(s).unwrap()
    }

    #[test]
    fn normalisation() {
        assert_eq!(n("src"), "src");
        assert_eq!(n("src/"), "src");
        assert_eq!(n("src\\a.rs"), "src/a.rs");
        assert_eq!(n("./src/./a.rs"), "src/a.rs");
        assert_eq!(n(".\\src\\\\a"), "src/a");
        assert_eq!(n("  src/x  "), "src/x");
        assert_eq!(n("*"), "*");
        assert_eq!(n("a//b///"), "a/b");
        assert_eq!(n("My Dir/file name.rs"), "My Dir/file name.rs");
    }

    #[test]
    fn invalid_claims_are_rejected() {
        for bad in [
            "", "   ", ".", "./", "/", "\\", "/abs", "\\abs", "\\\\server\\share", "C:/x", "C:\\x", "c:x", "..", "../x", "a/../b", "a/..", "src\\..\\..\\x", "*/x", "src/*", "*.rs", "a?b", "a\nb",
            "a./b", "src.", "a /b",
        ] {
            let r = normalize_claim(bad);
            assert!(r.is_err(), "`{bad:?}` must be rejected, got {r:?}");
        }
        assert!(normalize_claim("../x").unwrap_err().contains("`..`"));
        assert!(normalize_claim("/x").unwrap_err().contains("absolute"));
        assert!(normalize_claim("src/*").unwrap_err().contains("not patterns"));
        assert!(normalize_claim("").unwrap_err().contains("empty"));
    }

    #[test]
    fn claim_lists_dedupe_and_need_one() {
        assert_eq!(normalize_claims(&["src", "src/", ".\\src", "docs"]).unwrap(), ["src", "docs"]);
        assert!(normalize_claims(&[]).is_err());
        assert!(normalize_claims(&["src", "../x"]).is_err());
    }

    #[test]
    fn key_claim_normalisation_and_validation() {
        assert_eq!(n("./broadsword.toml#alpha_09"), "broadsword.toml#alpha_09");
        assert_eq!(n(" .\\crates\\\\broadsword/src/./config.rs/#beta"), "crates/broadsword/src/config.rs#beta");
        assert_eq!(normalize_claims(&["broadsword.toml#alpha", "./broadsword.toml#alpha", "broadsword.toml#beta"]).unwrap(), ["broadsword.toml#alpha", "broadsword.toml#beta"]);
        for key in ["", "Alpha", "a-b", "a.b", "a/b", "a\\b", "a#b", "a b", "a ", "a\t", "a\r", "a\n", "é", "a\u{2028}b"] {
            let claim = format!("broadsword.toml#{key}");
            let e = normalize_claim(&claim).unwrap_err();
            assert!(e.contains("key must match") || e.contains("control character"), "{claim:?}: {e}");
        }
        assert_eq!(n(".\\crates/broadsword//tests/./support/mod.rs/#gamma"), "crates/broadsword/tests/support/mod.rs#gamma");
        for path in ["src/lib.rs", "config.rs", "other/broadsword.toml", "crates/broadsword/src/config.rs.bak", "mod.rs", "crates/broadsword/tests/support", "crates/broadsword/tests/support/mod.rs.bak", "crates/broadsword/tests/tasks.rs"] {
            let e = normalize_claim(&format!("{path}#k")).unwrap_err();
            assert!(e.contains("key claims are only for broadsword.toml, crates/broadsword/src/config.rs and crates/broadsword/tests/support/mod.rs"), "{e}");
        }
        for path in ["../broadsword.toml", "/broadsword.toml", "C:\\broadsword.toml", "crates/../broadsword.toml", "broadsword.toml.", "broadsword.toml?"] {
            assert!(normalize_claim(&format!("{path}#k")).is_err(), "{path}");
        }
        // Large keys are data, not patterns or recursive parser input; validate the whole suffix.
        let large = "a".repeat(100_000);
        assert_eq!(n(&format!("broadsword.toml#{large}")), format!("broadsword.toml#{large}"));
        assert!(normalize_claim(&format!("broadsword.toml#{large}!")).is_err());
        assert_eq!(normalize_claim("BROADsword.toml#alpha").is_ok(), cfg!(windows));
    }

    #[test]
    fn key_claim_overlap_matrix() {
        for file in KEY_CLAIM_FILES {
            let alpha = format!("{file}#alpha");
            let beta = format!("{file}#beta");
            let mut pairs = vec![(alpha.as_str(), alpha.as_str(), true), (alpha.as_str(), beta.as_str(), false), (file, alpha.as_str(), true), ("*", alpha.as_str(), true)];
            let sibling = file.rsplit_once('/').map(|(dir, _)| format!("{dir}x"));
            if let (Some((dir, _)), Some(sibling)) = (file.rsplit_once('/'), sibling.as_deref()) {
                pairs.extend([("crates", alpha.as_str(), true), (dir, alpha.as_str(), true), (sibling, alpha.as_str(), false)]);
            }
            for (a, b, expected) in pairs {
                for ci in [false, true] {
                    assert_eq!(overlaps_with(a, b, ci), expected, "{a} vs {b}, ci={ci}");
                    assert_eq!(overlaps_with(b, a, ci), expected, "{b} vs {a}, ci={ci}");
                }
            }
            let upper = alpha.to_uppercase();
            assert!(!overlaps_with(&upper, &alpha, false));
            assert!(overlaps_with(&upper, &alpha, true));
            assert!(overlaps_with(&file.to_uppercase(), &alpha, true));
        }
        assert!(!overlaps_with("broadsword.toml#alpha", "crates/broadsword/src/config.rs#alpha", false));
        assert!(!overlaps_with("broadsword.toml#alpha", "broadsword.toml.bak", false));
        assert!(!overlaps_with("broadsword.toml#alpha", "broadsword.toml/child", false));
    }

    #[test]
    fn overlap_matrix() {
        let cs = false; // case-sensitive
        let yes = |a: &str, b: &str, ci: bool| {
            let (a, b) = (n(a), n(b));
            assert!(overlaps_with(&a, &b, ci), "{a} should overlap {b}");
            assert!(overlaps_with(&b, &a, ci), "{b} should overlap {a} (symmetric)");
        };
        let no = |a: &str, b: &str, ci: bool| {
            let (a, b) = (n(a), n(b));
            assert!(!overlaps_with(&a, &b, ci), "{a} must not overlap {b}");
            assert!(!overlaps_with(&b, &a, ci), "{b} must not overlap {a} (symmetric)");
        };
        yes("src", "src", cs);
        yes("src", "src/a.rs", cs);
        yes("src", "src/a/b/c.rs", cs);
        yes("src/a", "src/a/b", cs);
        yes("src\\a", "src/a/b.rs", cs); // separators alike
        yes("./src/", "src/a.rs", cs);
        no("src", "srcx", cs);
        no("src", "srcx/a.rs", cs);
        no("src/a", "src/ab", cs);
        no("src/a.rs", "src/b.rs", cs);
        no("a/src", "src", cs); // an ancestor is by path from the project root, not any suffix
        yes("*", "src", cs);
        yes("src/a.rs", "*", cs);
        yes("*", "*", cs);
        // case
        no("Src", "src", false);
        yes("Src", "src", true);
        yes("SRC/A.rs", "src/a.RS", true);
        yes("Src", "src/a.rs", true);
        no("Src", "srcx", true);
    }

    fn rec(id: &str, project: &str, status: &str, owner: &str, claims: &[&str]) -> Record {
        Record::parse(Path::new("x.task"), &format!("id = {id}\nproject = {project}\nstatus = {status}\nowner = {owner}\nclaims = {}\n", claims.join("\\n")))
    }

    fn mine(c: &[&str]) -> Vec<String> {
        c.iter().map(|s| n(s)).collect()
    }

    #[test]
    fn conflicts_need_same_project_running_and_overlap() {
        let tasks = vec![
            rec("a", "p", RUNNING, "coder-1", &["src/a.rs", "docs"]),
            rec("b", "p", OPEN, "", &["src"]),
            rec("c", "p", DONE, "x", &["src"]),
            rec("d", "q", RUNNING, "y", &["src"]),
            rec("e", "p", RUNNING, "coder-2", &["tests"]),
        ];
        let got = find_conflicts("p", &mine(&["src"]), &tasks, None);
        assert_eq!(got, [Conflict { id: "a".into(), owner: "coder-1".into(), mine: "src".into(), theirs: "src/a.rs".into() }]);
        assert!(find_conflicts("p", &mine(&["srcx", "lib"]), &tasks, None).is_empty());
        assert_eq!(find_conflicts("p", &mine(&["*"]), &tasks, None).len(), 2); // a and e, never d (other project)
        assert_eq!(find_conflicts("q", &mine(&["src/z"]), &tasks, None).len(), 1);
        assert!(find_conflicts("p", &mine(&["src"]), &tasks, Some("a")).is_empty(), "a task never conflicts with itself");
        let two = find_conflicts("p", &mine(&["tests", "docs/x"]), &tasks, None);
        assert_eq!(two.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["a", "e"]);
        assert_eq!((two[0].mine.as_str(), two[0].theirs.as_str()), ("docs/x", "docs"));
    }

    #[test]
    fn state_transitions() {
        use Action::*;
        let ok = |from, a| transition(from, a).unwrap();
        assert_eq!(ok(OPEN, Start), RUNNING);
        assert_eq!(ok(RUNNING, Done), DONE);
        assert_eq!(ok(RUNNING, Fail), FAILED);
        assert_eq!(ok(OPEN, Abandon), ABANDONED);
        assert_eq!(ok(RUNNING, Abandon), ABANDONED);
        for from in [RUNNING, DONE, FAILED, ABANDONED] {
            assert!(transition(from, Start).is_err(), "{from} -> start");
        }
        for from in [OPEN, DONE, FAILED, ABANDONED] {
            assert!(transition(from, Done).is_err(), "{from} -> done");
            assert!(transition(from, Fail).is_err(), "{from} -> fail");
        }
        for from in [DONE, FAILED, ABANDONED] {
            assert!(transition(from, Abandon).is_err(), "{from} -> abandon");
        }
        assert!(transition("bogus", Start).is_err());
        assert!(transition(DONE, Start).unwrap_err().contains("`done`"));
    }

    #[test]
    fn claims_round_trip_through_a_record_and_ids_are_unique() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-task-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let mut ids = std::collections::HashSet::new();
        for _ in 0..25 {
            let mut r = record::create_ext(&d, "t-20261117-120000", EXT).unwrap();
            r.set("claims", mine(&["src/a.rs", "docs"]).join("\n"));
            r.set("status", OPEN);
            r.save().unwrap();
            assert!(ids.insert(r.id().to_string()), "duplicate id {}", r.id());
            assert_eq!(claims_of(&record::load(&r.path).unwrap()), ["src/a.rs", "docs"]);
        }
        assert!(ids.contains("t-20261117-120000") && ids.contains("t-20261117-120000-2") && ids.contains("t-20261117-120000-25"));
        assert!(std::fs::read_dir(&d).unwrap().flatten().all(|e| e.path().extension().is_some_and(|x| x == EXT)), "no temp files left");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn ids_are_unique_across_threads() {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-task-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let d = d.clone();
                std::thread::spawn(move || (0..10).map(|_| record::create_ext(&d, "t-1", EXT).unwrap().id().to_string()).collect::<Vec<_>>())
            })
            .collect();
        let all: Vec<String> = hs.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let set: std::collections::HashSet<_> = all.iter().collect();
        assert_eq!((all.len(), set.len()), (80, 80));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
