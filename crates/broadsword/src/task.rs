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

use crate::config::Config;
use crate::job;
use crate::lock;
use crate::pause;
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

// ---- claims -----------------------------------------------------------------------------

/// Validate and normalise one claim: a file or folder path relative to the project root.
/// `/` and `\` are the same, `.` segments and a trailing slash vanish. Absolute paths, drive letters, `..`,
/// empty claims and patterns (`*`, `?` inside a path) are rejected; `*` alone is the whole project.
pub fn normalize_claim(raw: &str) -> Result<String, String> {
    let bad = |why: &str| Err(format!("invalid claim `{raw}`: {why}"));
    let s = raw.trim();
    if s.is_empty() {
        return bad("empty; name a file or folder, or `*` for the whole project");
    }
    if s.chars().any(char::is_control) {
        return bad("contains a control character");
    }
    if s == WHOLE_PROJECT {
        return Ok(WHOLE_PROJECT.into());
    }
    let s = s.replace('\\', "/");
    if s.starts_with('/') {
        return bad("absolute paths are not allowed; claims are relative to the project root");
    }
    let mut out: Vec<&str> = Vec::new();
    for seg in s.split('/') {
        match seg {
            "" | "." => {}
            ".." => return bad("`..` is not allowed; a claim must stay inside the project"),
            _ if seg.contains(':') => return bad("`:` is not allowed (drive letters and stream names are not project-relative paths)"),
            _ if seg.contains(['*', '?']) => return bad("claims are paths, not patterns: no `*` or `?` (use `*` alone to claim the whole project)"),
            _ if seg.ends_with(' ') || seg.ends_with('.') => return bad("a name may not end with a space or a dot (Windows treats it as another name)"),
            _ => out.push(seg),
        }
    }
    if out.is_empty() {
        return bad("it names no file or folder; use `*` for the whole project");
    }
    Ok(out.join("/"))
}

/// Do two normalised claims overlap? Equal, or one is an ancestor folder of the other by whole path segments
/// (`src` overlaps `src/a.rs`, not `srcx`). `*` overlaps everything. `case_insensitive` is true on Windows.
pub fn overlaps_with(a: &str, b: &str, case_insensitive: bool) -> bool {
    if a == WHOLE_PROJECT || b == WHOLE_PROJECT {
        return true;
    }
    let fold = |s: &str| if case_insensitive { s.to_lowercase() } else { s.to_string() };
    let (a, b) = (fold(a), fold(b));
    let (mut x, mut y) = (a.split('/'), b.split('/'));
    loop {
        match (x.next(), y.next()) {
            (Some(p), Some(q)) if p == q => {}
            (Some(_), Some(_)) => return false,
            _ => return true, // one ran out: it is the ancestor (or equal)
        }
    }
}

pub fn overlaps(a: &str, b: &str) -> bool {
    overlaps_with(a, b, cfg!(windows))
}

/// Normalise a list of claims, dropping repeats. At least one claim is required.
pub fn normalize_claims(raw: &[&str]) -> Result<Vec<String>, String> {
    if raw.is_empty() {
        return Err("at least one claim is required".into());
    }
    let mut out: Vec<String> = Vec::new();
    for r in raw {
        let c = normalize_claim(r)?;
        if !out.iter().any(|o| same(o, &c)) {
            out.push(c);
        }
    }
    Ok(out)
}

fn same(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.to_lowercase() == b.to_lowercase()
    } else {
        a == b
    }
}

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

/// Every task. A record that cannot be read is an error (never silently skipped: `start` decides on this list);
/// an empty file, which only exists for an instant while `add` creates a task, is skipped.
pub fn load_tasks(cfg: &Config) -> Result<Vec<Record>, String> {
    let dir = tasks_dir(cfg);
    let mut v = Vec::new();
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(v) };
    for e in rd.flatten() {
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
            Some(r) if !r.id().is_empty() => v.push(r),
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
    let path = tasks_dir(cfg).join(format!("{id}.{EXT}"));
    match record::load(&path) {
        Ok(r) if r.id() == id => Ok(r),
        _ => Err(nope()),
    }
}

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
    let mut r = match record::create_ext(&tasks_dir(cfg), &format!("t-{}", job::file_stamp(now_secs())), EXT) {
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

/// Test-only hook, like `job::TEST_FREE_HOOK_ENV`: when set to a file path inside this root's scratch folder, `start`
/// waits until that file exists before it touches the board, so a race test can start many processes and release them
/// together. It waits at most 3 x task_lock_wait_seconds (derived from that setting, so no separate limit exists), is
/// ignored for any path outside scratch, and announces itself on stderr. The long name is deliberate: it cannot be
/// set by accident.
pub const TEST_GATE_ENV: &str = "BROADSWORD_INTERNAL_TEST_TASK_START_GATE_FILE";

fn wait_for_test_gate(cfg: &Config) {
    let Ok(gate) = std::env::var(TEST_GATE_ENV) else { return };
    let gate = PathBuf::from(gate);
    let inside = match (gate.parent().and_then(|p| p.canonicalize().ok()), cfg.scratch_dir.canonicalize()) {
        (Some(p), Ok(s)) => p.starts_with(s),
        _ => false,
    };
    if !inside {
        return;
    }
    eprintln!("broadsword: TEST HOOK ACTIVE: task start waits for {} to exist", gate.display());
    let end = std::time::Instant::now() + Duration::from_secs(cfg.task_lock_wait_seconds * 3);
    while !gate.exists() && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(1));
    }
}

pub fn cmd_start(cfg: &Config, id: &str, owner: &str, worktree: bool) -> ExitCode {
    let owner = owner.trim();
    if owner.is_empty() || owner.contains(['\n', '\r']) {
        return fail(EXIT_BAD_INPUT, "the owner must be a non-empty single line, e.g. coder-1");
    }
    wait_for_test_gate(cfg);
    let _lock = match board_lock(cfg) {
        Ok(l) => l,
        Err(c) => return c,
    };
    let mut t = match load_task(cfg, id) {
        Ok(t) => t,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    if let Err(e) = transition(t.status(), Action::Start) {
        return fail(EXIT_ERROR, format!("cannot start {id}: {e}"));
    }
    let all = match load_tasks(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, format!("cannot read the board, so nothing was started: {e}")),
    };
    let conflicts = find_conflicts(t.get("project"), &claims_of(&t), &all, Some(id));
    if !conflicts.is_empty() {
        let lines: Vec<String> = conflicts.iter().map(|c| format!("  {}", describe(c))).collect();
        return fail(EXIT_CONFLICT, format!("cannot start {id}: its claims overlap a running task:\n{}\nstart it after that task is done, failed or abandoned", lines.join("\n")));
    }
    t.set("status", RUNNING);
    t.set("owner", owner);
    t.set("started", now_secs().to_string());
    if let Err(e) = save(&t) {
        return fail(EXIT_ERROR, e);
    }
    println!("task {id} is running (owner {owner}); its claims are held until it is done, failed or abandoned");
    if !worktree {
        return ExitCode::SUCCESS;
    }
    // Still under the board lock: nobody can see the task as running until its worktree exists, or it is open again.
    match worktree::create_for(cfg, &mut t) {
        Ok(c) => match save(&t) {
            Ok(()) => {
                println!("{}", worktree::describe_created(&c));
                ExitCode::SUCCESS
            }
            Err(e) => {
                let left = worktree::undo_create(cfg, &t, &c).map(|l| format!(" The new worktree could not be removed again: {l}")).unwrap_or_default();
                roll_back_start(&mut t, &format!("{e}.{left}"))
            }
        },
        Err(e) => roll_back_start(&mut t, &e),
    }
}

/// The task was marked running but its worktree could not be made: put it back to `open` (owner and start time cleared),
/// so a task is never left running without the worktree that was asked for.
fn roll_back_start(t: &mut Record, why: &str) -> ExitCode {
    let id = t.id().to_string();
    for k in ["worktree", "branch", "base", "worktree_removed"] {
        t.set(k, "");
    }
    t.set("status", OPEN);
    t.set("owner", "");
    t.set("started", "");
    match save(t) {
        Ok(()) => fail(EXIT_ERROR, format!("cannot start {id} with a worktree: {why}\ntask {id} was rolled back to `open` (claims not held, nothing half-made); fix the cause and start it again")),
        Err(e) => fail(EXIT_ERROR, format!("cannot start {id} with a worktree: {why}\nAND the rollback failed: {e}. Task {id} may still say `running` with no worktree: run `broadsword task abandon {id}` or fix its record")),
    }
}

/// done, fail and abandon: change the status under the board lock and release the claims.
pub fn cmd_finish(cfg: &Config, id: &str, action: Action, note: Option<&str>) -> ExitCode {
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
    t.set("status", to);
    t.set("ended", now_secs().to_string());
    if let Some(n) = note {
        t.set("note", n.split_whitespace().collect::<Vec<_>>().join(" "));
    }
    if let Err(e) = save(&t) {
        return fail(EXIT_ERROR, e);
    }
    println!("task {id} is {to}; its claims are released");
    // `done` cleans up the task's worktree, but only when that is safe (see worktree.rs). Either way the task is done:
    // a kept worktree is a warning, not an error, so the exit code stays 0.
    if action == Action::Done && worktree::has_worktree(&t) && worktree::after_done(cfg, &mut t) {
        if let Err(e) = save(&t) {
            return fail(EXIT_ERROR, format!("{e} (the worktree itself was removed)"));
        }
    }
    ExitCode::SUCCESS
}

pub fn cmd_list(cfg: &Config, status: Option<&str>, project: Option<&str>) -> ExitCode {
    if let Some(s) = status {
        if !STATUSES.contains(&s) {
            return fail(EXIT_BAD_INPUT, format!("unknown status `{s}`; use one of: {}", STATUSES.join(", ")));
        }
    }
    let all = match load_tasks(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    // the filter is canonicalised like every project name; an unknown name just matches nothing
    let project = project.map(|p| job::canonical_project(&cfg.projects_dir, p).unwrap_or_else(|_| p.to_string()));
    let shown: Vec<&Record> = all.iter().filter(|t| status.is_none_or(|s| t.status() == s) && project.as_deref().is_none_or(|p| t.get("project") == p)).collect();
    if shown.is_empty() {
        println!("no tasks{}", if status.is_some() || project.is_some() { " match" } else { "" });
    }
    for t in shown {
        let owner = if t.get("owner").is_empty() { "-" } else { t.get("owner") };
        println!("{}\t{}\t{}\t{}\t{}\t{}", t.id(), t.status(), owner, t.get("project"), t.get("title"), claims_of(t).join(", "));
    }
    ExitCode::SUCCESS
}

pub fn cmd_show(cfg: &Config, id: &str) -> ExitCode {
    let t = match load_task(cfg, id) {
        Ok(t) => t,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    let when = |k: &str| t.get_num(k).map(pause::format_utc).unwrap_or_else(|| "-".into());
    let or_dash = |k: &str| if t.get(k).is_empty() { "-".to_string() } else { t.get(k).to_string() };
    println!("id: {}", t.id());
    println!("title: {}", t.get("title"));
    println!("project: {}", t.get("project"));
    println!("status: {}", t.status());
    println!("owner: {}", or_dash("owner"));
    println!("claims: {}", claims_of(&t).join(", "));
    println!("created: {}", when("created"));
    println!("started: {}", when("started"));
    println!("ended: {}", when("ended"));
    println!("note: {}", or_dash("note"));
    if !t.get("worktree").is_empty() {
        println!("worktree: {}{}", t.get("worktree"), if t.get("worktree_removed").is_empty() { "" } else { " (removed)" });
        println!("branch: {}", t.get("branch"));
        println!("base: {}", t.get("base"));
    }
    ExitCode::SUCCESS
}

/// Read-only: would a task with these claims conflict with a running task right now?
pub fn cmd_check(cfg: &Config, project: &str, claims: &[&str]) -> ExitCode {
    let project = &match check_project(cfg, project) {
        Ok(p) => p,
        Err(c) => return c,
    };
    let claims = match normalize_claims(claims) {
        Ok(c) => c,
        Err(e) => return fail(EXIT_BAD_INPUT, e),
    };
    let all = match load_tasks(cfg) {
        Ok(a) => a,
        Err(e) => return fail(EXIT_ERROR, e),
    };
    let conflicts = find_conflicts(project, &claims, &all, None);
    if conflicts.is_empty() {
        println!("free: no running task in `{project}` overlaps {}", claims.join(", "));
        return ExitCode::SUCCESS;
    }
    println!("conflict: these claims overlap a running task in `{project}`:");
    for c in &conflicts {
        println!("  {}", describe(c));
    }
    ExitCode::from(EXIT_CONFLICT)
}

// ---- tests ------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
