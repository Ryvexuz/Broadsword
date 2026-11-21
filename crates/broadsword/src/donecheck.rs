//! The DONE CHECK the merge queue runs itself (audit S5: "a queue PASS doesn't mean the done check was run"). The reviewer has no
//! tools and one turn, so the queue runs the brief's own done-check commands and the reviewer reads their real output.
//!
//! PARSE (`plan`). The brief's "Done check" section is the lines after the first heading line (starting with `#` or `**`) whose
//! text begins "Done check", up to the next heading line. Each NUMBERED line (`1.` or `1)`) is one item; its FIRST backtick span
//! is its command. Every other line of the section that holds a backtick span is listed too (as not run), so nothing that looks
//! like a command is ever silently dropped. The placeholders `<id>`, `<task id>`, `<task-id>`, `<task>` and `<project>` are
//! replaced by the entry's task and project first.
//!
//! WHAT RUNS (`classify`, the ONE place the rule lives). Only the real binary's own job commands, in exactly the forms its command
//! line takes, for the entry's OWN project and task:
//!   `broadsword build <project> --task <task>`
//!   `broadsword run <project> --task <task> -- <command...>`
//! `broadsword test|check <project> --task <task>` pass the same rule but are NEVER run: they would repeat the queue's own check
//! (the whole suite, per entry, which would undo a merge train's savings). They are shown `COVERED by the queue's check`, with that
//! check's result; for a merge-train member, whose check is the train's and comes after its review, `COVERED (pending)`: the train
//! check decides, and nothing merges unless it passes.
//! `broadsword` is the bare name or a path whose file name is `broadsword` / `broadsword.exe`; whatever path is written, the
//! program started is ALWAYS this running binary (`std::env::current_exe`), never the path in the brief. `<project>` must be the
//! entry's project by its canonical name (`job::canonical_project`), and `--task` the entry's task, so the command checks the
//! branch under review (without `--task` a job builds the main checkout instead). Everything else (python, cargo, git, another
//! subcommand, another project or task, extra flags, an unfilled placeholder, a shell character) is `NOT RUN (manual)` with its
//! reason, and the reviewer is shown it as such.
//!
//! HOW IT RUNS (`run_one`). An argv list built from the parsed parts (never the brief's text, never a shell: the queue starts the
//! binary directly), from `scrub::command`, so the child gets the scrubbed environment; in the task's worktree; stdin closed;
//! stdout and stderr read line by line, each line redacted (`Secrets::redact`) BEFORE it is kept, so cutting the tail can never
//! leave half a secret. Only the last `done_check_tail_lines` lines, and of those at most `reviewer_check_tail_kb` KB, are kept,
//! and memory stays bounded while reading: a line longer than `reviewer_check_tail_kb` keeps only its start, with a note that it
//! was cut (`pump`). A command still running after `done_check_timeout_seconds` is killed with its whole process tree
//! (`sys::kill_tree`, then everything it started, `group`). A command has finished only when NOTHING it started is still running:
//! after it exits, the queue waits (to the same deadline) until its output is closed AND its kill group is empty
//! (`group::Group::running`), whether or not what is left holds its output. Something still running at the deadline did not
//! finish: everything it started is killed and it FAILS (`Ended::LeftRunning`); if the group cannot be asked, everything is
//! killed and it FAILS too (`Ended::CannotTell`). The group is killed before `run_one` returns on every path. Because
//! `broadsword run` hands its command to the system shell, the shell characters `& | < > ^ % ; $ ` " ' ( )` and newlines are
//! refused in every argument, so nothing in a brief can reach a shell as syntax.
//!
//! VERDICT. Commands run in order and stop at the first one that does not pass. Exit 0 passes. A non-zero exit FAILS when the job
//! ran its step and printed its own `<build|run>: FAILED` summary line (a `run` passes its command's own exit code on, so the code
//! alone cannot tell a failed command from a refusal); a timeout FAILS too, and so does a command that did not finish (it exited
//! but something it started was still running at the deadline, or that could not be told), whatever its exit code: the queue ends the entry `rejected` before any reviewer
//! is called. Any other non-zero exit (a refusal: bad input, paused, busy, disk, which print no step summary), no exit code, or a
//! program that could not start gives NO VERDICT: the queue ends the entry `blocked`.
//!
//! NON-WINDOWS. Done-check execution needs Windows job-object containment: off Windows `classify` (through `classify_for`, which
//! takes the platform) marks every otherwise-runnable line (`build`, `run`) `NOT RUN` with the reason "done-check execution needs
//! Windows job-object containment", so nothing is ever spawned there; `run_one` also refuses to spawn off Windows. A process group
//! cannot contain a child that calls `setsid()`, so no weaker containment is offered.
//!
//! Known limits: a secret split across two output lines is not recognised (as in job logs); a command killed at its timeout
//! leaves the job it started interrupted, for `broadsword recover` (as any killed job does). The command cannot start anything
//! outside its kill group: it is created suspended and runs only once it is in the group (`group`).

use crate::config::Config;
use crate::scrub::{self, Secrets};
use crate::{job, sys};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The job subcommands a done-check command may name.
const SUBCOMMANDS: [&str; 4] = ["test", "check", "build", "run"];
/// The ones the queue's own check already runs on the same commit: shown COVERED, never run again.
const COVERED: [&str; 2] = ["test", "check"];

/// Characters refused in every argument: the system shell would read them as syntax once `broadsword run` hands its command on.
const SHELL_CHARS: [char; 15] = ['&', '|', '<', '>', '^', '%', ';', '$', '`', '"', '\'', '(', ')', '\n', '\r'];

/// One item of the brief's done check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Runnable: `args` follow the program (the running binary), exactly as they are passed.
    Run { label: String, text: String, args: Vec<String> },
    /// Shown to the reviewer as not run, with the reason.
    NotRun { label: String, text: String, why: String },
    /// `broadsword test|check` of the entry's own project and task: the queue's own check covers it; never run.
    Covered { label: String, text: String },
}

impl Item {
    pub fn label(&self) -> &str {
        match self {
            Item::Run { label, .. } | Item::NotRun { label, .. } | Item::Covered { label, .. } => label,
        }
    }
}

/// What the brief's done check holds. `found` is false when the brief has no "Done check" section at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub found: bool,
    pub items: Vec<Item>,
}

impl Plan {
    pub fn runnable(&self) -> usize {
        self.items.iter().filter(|i| matches!(i, Item::Run { .. })).count()
    }
}

fn is_heading(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with('#') || t.starts_with("**")
}

fn is_done_check_heading(line: &str) -> bool {
    is_heading(line) && line.trim_start().trim_start_matches(['#', '*', ' ']).to_lowercase().starts_with("done check")
}

/// `1. text` / `1) text` -> ("1", "text").
fn numbered(line: &str) -> Option<(String, &str)> {
    let t = line.trim_start();
    let digits: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let rest = &t[digits.len()..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')).then(|| (digits, rest.trim()))
}

/// The first backtick span of `line`, if it has a closed one.
fn first_span(line: &str) -> Option<&str> {
    let start = line.find('`')? + 1;
    let len = line[start..].find('`')?;
    Some(&line[start..start + len])
}

fn fill(text: &str, project: &str, task: &str) -> String {
    let mut s = text.to_string();
    for p in ["<task id>", "<task-id>", "<id>", "<task>"] {
        s = s.replace(p, task);
    }
    s.replace("<project>", project)
}

/// Is `tok` the Broadsword binary: `broadsword`, or a path whose file name is `broadsword` or `broadsword.exe`?
fn is_broadsword(tok: &str) -> bool {
    let name = tok.rsplit(['/', '\\']).next().unwrap_or(tok).to_lowercase();
    name == "broadsword" || name == "broadsword.exe"
}

/// THE rule: is this done-check command one the queue may run for this entry? `Ok(args)` are the arguments after the program.
pub fn classify(command: &str, projects_dir: &Path, project: &str, task: &str) -> Result<Vec<String>, String> {
    classify_for(command, projects_dir, project, task, cfg!(windows))
}

/// Why nothing is run off Windows (shown as the NOT RUN reason; also the `NotStarted` text of `run_one` there).
const NEEDS_WINDOWS: &str = "done-check execution needs Windows job-object containment";

/// `classify` with the platform as a parameter, so the non-Windows rule is testable on Windows. Off Windows a command that would
/// otherwise run (`build`, `run`) is refused with `NEEDS_WINDOWS`; `test` and `check` are never run on any platform.
fn classify_for(command: &str, projects_dir: &Path, project: &str, task: &str, windows: bool) -> Result<Vec<String>, String> {
    if let Some(c) = command.chars().find(|c| SHELL_CHARS.contains(c)) {
        let what = if c == '<' || c == '>' { "an unfilled placeholder or a redirection" } else { "a shell character" };
        return Err(format!("it contains `{}` ({what}); the queue runs no shell and passes no shell syntax on", c.escape_default()));
    }
    let toks: Vec<&str> = command.split_whitespace().collect();
    let Some(first) = toks.first() else {
        return Err("the backtick span is empty".into());
    };
    if !is_broadsword(first) {
        return Err(format!("only `broadsword build|run <project> --task <task>` commands are run by the queue (`test` and `check` are covered by its own check); `{first}` is not the Broadsword binary"));
    }
    let sub = toks.get(1).copied().unwrap_or("");
    if !SUBCOMMANDS.contains(&sub) {
        return Err(format!("`{}` is not one of the job commands the queue runs (test, check, build, run)", if sub.is_empty() { "(nothing)" } else { sub }));
    }
    let named = toks.get(2).copied().unwrap_or("");
    let canon = job::canonical_project(projects_dir, named).map_err(|e| format!("its project is not usable: {e}"))?;
    if canon != project {
        return Err(format!("it names project `{canon}`, not this entry's project `{project}`"));
    }
    let rest = &toks[3..];
    let (flag, id, tail) = match rest {
        ["--task", id, tail @ ..] => (true, *id, tail),
        _ => (false, "", rest),
    };
    if !flag {
        return Err(format!("it has no `--task {task}` right after the project, so it would build the main checkout, not this branch"));
    }
    if id != task {
        return Err(format!("it names task `{id}`, not this entry's task `{task}`"));
    }
    let mut args: Vec<String> = vec![sub.to_string(), canon, "--task".into(), task.to_string()];
    match (sub, tail) {
        ("run", ["--", cmd @ ..]) if !cmd.is_empty() => {
            args.push("--".into());
            args.extend(cmd.iter().map(|s| s.to_string()));
        }
        ("run", _) => return Err("`broadsword run` needs `-- <command>` right after `--task <task>`".into()),
        (_, []) => {}
        (_, extra) => return Err(format!("`broadsword {sub}` takes nothing after `--task <task>`; it has `{}`", extra.join(" "))),
    }
    if !windows && !COVERED.contains(&sub) {
        return Err(NEEDS_WINDOWS.into());
    }
    Ok(args)
}

/// Parse the brief's done check for the entry's (canonical) `project` and `task`.
pub fn plan(brief: &str, projects_dir: &Path, project: &str, task: &str) -> Plan {
    let mut lines = brief.lines();
    let mut found = false;
    let mut items = Vec::new();
    for l in lines.by_ref() {
        if is_done_check_heading(l) {
            found = true;
            break;
        }
    }
    if !found {
        return Plan { found, items };
    }
    let mut loose = 0usize;
    for l in lines {
        if is_heading(l) {
            break;
        }
        let (label, text, is_numbered) = match numbered(l) {
            Some((n, rest)) => (n, rest, true),
            None => {
                if first_span(l).is_none() {
                    continue;
                }
                loose += 1;
                (format!("line {loose}"), l.trim(), false)
            }
        };
        let text = text.to_string();
        let Some(span) = first_span(&text) else {
            items.push(Item::NotRun { label, text, why: "no command in backticks".into() });
            continue;
        };
        if !is_numbered {
            items.push(Item::NotRun { label, text, why: "not a numbered line of the done check".into() });
            continue;
        }
        let cmd = fill(span, project, task);
        match classify(&cmd, projects_dir, project, task) {
            Ok(args) if COVERED.contains(&args[0].as_str()) => items.push(Item::Covered { label, text: cmd }),
            Ok(args) => items.push(Item::Run { label, text: cmd, args }),
            Err(why) => items.push(Item::NotRun { label, text: cmd, why }),
        }
    }
    Plan { found, items }
}

/// The limits one command runs under (from `done_check_timeout_seconds`, `done_check_tail_lines`, `reviewer_check_tail_kb`).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub timeout: Duration,
    pub tail_lines: usize,
    pub tail_bytes: usize,
}

impl Limits {
    pub fn from_config(cfg: &Config) -> Limits {
        Limits {
            timeout: Duration::from_secs(cfg.done_check_timeout_seconds),
            tail_lines: cfg.done_check_tail_lines as usize,
            tail_bytes: (cfg.reviewer_check_tail_kb as usize).saturating_mul(1024),
        }
    }
}

/// How a command ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Exited(Option<i32>),
    TimedOut,
    /// The command itself exited (with this code), but something it started was still running at the deadline (holding its
    /// output or not): it did not finish. Everything it started was then killed (`killed`: the kill call succeeded).
    LeftRunning { code: Option<i32>, killed: bool },
    /// The command itself exited (with this code), but whether something it started was still running could not be asked
    /// (`why`): fail closed. Everything it started was then killed (`killed`: the kill call succeeded).
    CannotTell { code: Option<i32>, why: String, killed: bool },
    NotStarted(String),
}

/// One command that ran.
#[derive(Debug, Clone)]
pub struct Ran {
    pub ended: Ended,
    pub secs: f64,
    /// The last lines of stdout and stderr (interleaved as they arrived), each already redacted.
    pub tail: Vec<String>,
    /// How many lines were dropped from the front.
    pub dropped: usize,
    /// The job printed its own `<subcommand>: FAILED` summary line: its step ran and failed (seen on every line, kept or not).
    pub step_failed: bool,
}

impl Ran {
    pub fn verdict(&self) -> Verdict {
        match &self.ended {
            Ended::Exited(Some(0)) => Verdict::Pass,
            Ended::Exited(Some(c)) if self.step_failed => Verdict::Fail(format!("exit {c}: the job ran and its step FAILED")),
            Ended::TimedOut => Verdict::Fail("timed out (killed)".into()),
            // never a pass, whatever the exit code: the done check had not finished at its deadline
            Ended::LeftRunning { .. } => Verdict::Fail("did not finish: a process it started was still running at the deadline (killed)".into()),
            // never a pass either: nothing shows that what it started had ended
            Ended::CannotTell { .. } => Verdict::Fail("could not tell whether a process it started was still running (killed)".into()),
            Ended::Exited(Some(c)) => Verdict::Inconclusive(format!("exit {c}: the command could not run to a verdict (bad input, refused, busy or disk)")),
            Ended::Exited(None) => Verdict::Inconclusive("ended without an exit code".into()),
            Ended::NotStarted(m) => Verdict::Inconclusive(format!("could not start: {m}")),
        }
    }

    fn ended_text(&self, limits: &Limits) -> String {
        match &self.ended {
            Ended::Exited(Some(c)) => format!("exit {c}"),
            Ended::Exited(None) => "ended without an exit code".into(),
            Ended::TimedOut => format!("TIMED OUT after {} s (done_check_timeout_seconds): killed with its whole process tree", limits.timeout.as_secs()),
            Ended::LeftRunning { code, killed } => format!(
                "DID NOT FINISH: the command exited ({}) but a process it started was still running after {} s (done_check_timeout_seconds); {}",
                code.map(|c| format!("exit {c}")).unwrap_or_else(|| "no exit code".into()),
                limits.timeout.as_secs(),
                if *killed { "everything it started was killed" } else { "killing what it started FAILED: it may still be running" }
            ),
            Ended::CannotTell { code, why, killed } => format!(
                "NOT KNOWN TO HAVE FINISHED: the command exited ({}) but whether a process it started was still running could not be told ({why}); {}",
                code.map(|c| format!("exit {c}")).unwrap_or_else(|| "no exit code".into()),
                if *killed { "everything it started was killed" } else { "killing what it started FAILED: it may still be running" }
            ),
            Ended::NotStarted(m) => format!("could not start: {m}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Pass,
    Fail(String),
    Inconclusive(String),
}

struct Tail {
    lines: VecDeque<String>,
    /// `done_check_tail_lines`: at most this many lines are kept.
    max: usize,
    /// `reviewer_check_tail_kb` in bytes: the kept lines (plus one newline each) stay within this, except that the newest line is
    /// always kept (and is itself at most `line_cap` plus its cut note, see `pump`).
    max_bytes: usize,
    bytes: usize,
    dropped: usize,
    /// `<subcommand>: FAILED`, the job's own summary line for a failed step.
    marker: String,
    marked: bool,
}

impl Tail {
    fn push(&mut self, line: String) {
        if line.trim_start().starts_with(&self.marker) {
            self.marked = true;
        }
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        while self.lines.len() > self.max || (self.bytes > self.max_bytes && self.lines.len() > 1) {
            if let Some(old) = self.lines.pop_front() {
                self.bytes -= old.len() + 1;
            }
            self.dropped += 1;
        }
    }
}

/// Read `src` line by line into `tail`, each line redacted BEFORE it is kept. Memory is bounded whatever the command prints: of a
/// line, at most `2 * line_cap` bytes are ever held (the rest, up to its newline, is read and thrown away), and a line longer
/// than `line_cap` keeps only its first `line_cap` bytes, followed by a note saying it was cut and how long it was: never cut
/// silently. A line held whole is redacted whole (`Secrets::redact`); a cut line's kept part goes through `Secrets::redact_cut`,
/// the one rule for cut text, which also masks any part of a secret the cut split (and all of it when a secret is longer).
fn pump(src: impl Read, tail: Arc<Mutex<Tail>>, secrets: Arc<Secrets>, line_cap: usize) {
    let mut r = BufReader::new(src);
    let window = line_cap.saturating_mul(2);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        let mut total = 0usize; // bytes of this line, newline included
        let mut eof = false;
        let mut ended = false; // the line ended with a newline
        let mut cr = false; // the last byte seen before that newline was a CR (tracked as read: the held part may be cut)
        loop {
            let avail = match r.fill_buf() {
                Ok(b) => b,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    eof = true; // a pipe broken by a kill ends the output, as an end of file does
                    break;
                }
            };
            if avail.is_empty() {
                eof = true;
                break;
            }
            let (n, newline) = match avail.iter().position(|&b| b == b'\n') {
                Some(i) => {
                    cr = if i > 0 { avail[i - 1] == b'\r' } else { cr };
                    (i + 1, true)
                }
                None => {
                    cr = avail.last() == Some(&b'\r');
                    (avail.len(), false)
                }
            };
            let room = window.saturating_sub(buf.len());
            buf.extend_from_slice(&avail[..n.min(room)]);
            total += n;
            r.consume(n);
            if newline {
                ended = true;
                break;
            }
        }
        if total == 0 {
            break; // end of output, nothing pending
        }
        let text = String::from_utf8_lossy(&buf);
        let raw = text.trim_end_matches(['\n', '\r']);
        // the line's own length, without its line ending
        let len = total - usize::from(ended) - usize::from(ended && cr);
        let line = if len > line_cap {
            let mut keep = line_cap.min(raw.len());
            while !raw.is_char_boundary(keep) {
                keep -= 1;
            }
            // cut RAW, then made safe by the one rule for cut text: no part of a secret the cut split can show
            format!("{} [line cut: {len} bytes, only the first {line_cap} kept (reviewer_check_tail_kb)]", secrets.redact_cut(&raw[..keep]))
        } else {
            secrets.redact(raw) // held whole: every secret in it is whole too
        };
        if let Ok(mut t) = tail.lock() {
            t.push(line);
        }
        if eof {
            break;
        }
    }
}

/// The last lines of `lines` that fit in `max_bytes` (a single longer line keeps its end); the second value counts lines cut.
fn fit(lines: &[String], max_bytes: usize, secrets: &Secrets) -> (Vec<String>, usize) {
    let mut out: Vec<String> = Vec::new();
    let mut used = 0usize;
    for l in lines.iter().rev() {
        if used + l.len() + 1 > max_bytes {
            if out.is_empty() {
                let mut cut = l.len().saturating_sub(max_bytes);
                while !l.is_char_boundary(cut) {
                    cut += 1;
                }
                // the lines are redacted already; the cut still goes through the one rule for cut text
                out.push(format!("...{}", secrets.redact_cut(&l[cut..])));
            }
            break;
        }
        used += l.len() + 1;
        out.push(l.clone());
    }
    let cut = lines.len() - out.len();
    out.reverse();
    (out, cut)
}

/// Run `exe args` in `dir` under `limits`. Every line of output is redacted with `secrets` before it is kept.
pub fn run_one(cfg: &Config, exe: &Path, dir: &Path, args: &[String], limits: &Limits, secrets: &Secrets) -> Ran {
    let start = Instant::now();
    if !cfg!(windows) {
        // belt and braces: `classify` already refuses every runnable line off Windows, so nothing reaches here; if something
        // does, nothing is spawned
        return Ran { ended: Ended::NotStarted(NEEDS_WINDOWS.into()), secs: 0.0, tail: Vec::new(), dropped: 0, step_failed: false };
    }
    let mut cmd = scrub::command(cfg, exe); // the one spawn helper: the scrubbed environment, never a shell
    cmd.args(args).current_dir(dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    group::prepare(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ran { ended: Ended::NotStarted(secrets.redact(&e.to_string())), secs: 0.0, tail: Vec::new(), dropped: 0, step_failed: false },
    };
    // Everything the command starts, so it can all be killed even after the command itself has exited (when `kill_tree` can
    // no longer find the tree from its pid). The child is still suspended here (Windows): it runs only once it is in the group,
    // and if that fails it never runs at all.
    let group = match group::Group::start(&child, cfg) {
        Ok(g) => g,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Ran { ended: Ended::NotStarted(e), secs: start.elapsed().as_secs_f64(), tail: Vec::new(), dropped: 0, step_failed: false };
        }
    };
    let marker = format!("{}: FAILED", args.first().map(String::as_str).unwrap_or(""));
    let tail = Arc::new(Mutex::new(Tail { lines: VecDeque::new(), max: limits.tail_lines, max_bytes: limits.tail_bytes, bytes: 0, dropped: 0, marker, marked: false }));
    let secrets = Arc::new(secrets.clone());
    let line_cap = limits.tail_bytes;
    let readers: Vec<std::thread::JoinHandle<()>> = [child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>), child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)]
        .into_iter()
        .flatten()
        .map(|s| {
            let (t, sc) = (Arc::clone(&tail), Arc::clone(&secrets));
            std::thread::spawn(move || pump(s, t, sc, line_cap))
        })
        .collect();
    let kill_all = |child: &mut std::process::Child| {
        // the child has not been waited on, so its pid is still ours: take the whole tree down (cargo, rustc, shells), then
        // whatever it started that is no longer in its tree
        sys::kill_tree(child.id());
        let _ = child.kill();
        let _ = child.wait();
        group.kill();
    };
    let deadline = start + limits.timeout;
    let tick = Duration::from_millis(50);
    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ended::Exited(status.code()),
            Ok(None) => {}
            Err(e) => {
                kill_all(&mut child);
                break Ended::NotStarted(format!("waiting for it failed: {e}"));
            }
        }
        if Instant::now() >= deadline {
            kill_all(&mut child);
            break Ended::TimedOut;
        }
        std::thread::sleep(tick);
    };
    // A command has finished only when NOTHING it started is still running. The readers end when every holder of the pipes is
    // gone, but a descendant that holds none of them (redirected, closed, detached) is seen only by asking the group. Until
    // both are done it is waited for; at its deadline everything it started is killed, and it FAILS (`LeftRunning`), never
    // passes. A group that cannot be asked fails closed (`CannotTell`), after killing everything in it.
    let ended = match ended {
        Ended::Exited(code) => loop {
            if readers.iter().all(|r| r.is_finished()) {
                match group.running() {
                    Ok(0) => break Ended::Exited(code),
                    Ok(_) => {}
                    Err(why) => {
                        let killed = group.kill();
                        break Ended::CannotTell { code, why, killed };
                    }
                }
            }
            if Instant::now() >= deadline {
                let killed = group.kill();
                break Ended::LeftRunning { code, killed };
            }
            std::thread::sleep(tick);
        },
        other => other,
    };
    // Belt and braces: whatever the ending, nothing the command started outlives this call (an empty group: a no-op).
    group.kill();
    let secs = start.elapsed().as_secs_f64();
    let (lines, dropped, step_failed) = tail.lock().map(|t| (t.lines.iter().cloned().collect::<Vec<_>>(), t.dropped, t.marked)).unwrap_or_default();
    let (lines, cut) = fit(&lines, limits.tail_bytes, &secrets);
    Ran { ended, secs, tail: lines, dropped: dropped + cut, step_failed }
}

/// Everything a done-check command starts, as one unit that can be killed at once, even after the command itself has exited
/// (its descendants then no longer form a tree `sys::kill_tree` can find from its pid: checked on Windows, where `taskkill /T`
/// on an exited parent says "not found" and its grandchild runs on).
/// Windows: the child is created suspended, put in a new Job Object, and only then resumed (`prepare`, `Group::start`), so it
/// runs no instruction outside the job and everything it starts joins the job by itself. The job is set to kill everything in
/// it when its last handle closes (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), so a dropped `Group` never leaves a process alive. If
/// the job cannot be made, limited, joined or the child resumed, the still-suspended child is killed and the command does not
/// run (NO VERDICT). `running` asks the job how many processes in it are still alive (`ActiveProcesses`).
/// Elsewhere: no group exists (a process group cannot hold a child that calls setsid()), and nothing is run there.
mod group {
    use std::process::{Child, Command};

    /// Windows: the child is created SUSPENDED (`CREATE_SUSPENDED`), so it runs no instruction until `Group::start` has put it
    /// in the job and resumed it: nothing it starts can be outside the job.
    #[cfg(windows)]
    pub fn prepare(cmd: &mut Command) {
        use std::os::windows::process::CommandExt;
        const CREATE_SUSPENDED: u32 = 0x0000_0004;
        cmd.creation_flags(CREATE_SUSPENDED);
    }

    /// Elsewhere nothing is ever spawned (`classify_for`, `run_one`): no containment exists, so this does nothing.
    #[cfg(not(windows))]
    pub fn prepare(_cmd: &mut Command) {}

    #[cfg(windows)]
    pub struct Group(isize);

    /// `THREADENTRY32` (tlhelp32.h).
    #[cfg(windows)]
    #[repr(C)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_pid: u32,
        base_pri: i32,
        delta_pri: i32,
        flags: u32,
    }

    /// `JOBOBJECT_BASIC_ACCOUNTING_INFORMATION` (winnt.h), information class 1.
    #[cfg(windows)]
    #[repr(C)]
    #[derive(Default)]
    struct Accounting {
        total_user: i64,
        total_kernel: i64,
        period_user: i64,
        period_kernel: i64,
        page_faults: u32,
        total_processes: u32,
        active_processes: u32,
        terminated_processes: u32,
    }

    /// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` (winnt.h), information class 9: `JOBOBJECT_BASIC_LIMIT_INFORMATION`, then
    /// `IO_COUNTERS` (six u64) and the four memory fields, flattened.
    #[cfg(windows)]
    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimit {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
        io: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    #[cfg(windows)]
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const core::ffi::c_void, name: *const u16) -> isize;
        fn AssignProcessToJobObject(job: isize, process: isize) -> i32;
        fn TerminateJobObject(job: isize, exit_code: u32) -> i32;
        fn SetInformationJobObject(job: isize, class: i32, info: *const core::ffi::c_void, len: u32) -> i32;
        fn QueryInformationJobObject(job: isize, class: i32, info: *mut core::ffi::c_void, len: u32, ret_len: *mut u32) -> i32;
        fn CloseHandle(h: isize) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        fn Thread32First(snapshot: isize, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: isize, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> isize;
        fn ResumeThread(thread: isize) -> u32;
        fn GetLastError() -> u32;
    }

    #[cfg(windows)]
    impl Group {
        /// Put the SUSPENDED `child` (see `prepare`) in a new job, then resume it. Any failure is an error, and the caller kills
        /// the still-suspended child (fail closed: a command is never run outside its kill group).
        pub fn start(child: &Child, _cfg: &crate::config::Config) -> Result<Group, String> {
            use std::os::windows::io::AsRawHandle;
            // SAFETY: plain kernel32 calls on a handle we own (the job) and the child's process handle, which `child` keeps open.
            let group = unsafe {
                let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if job == 0 {
                    return Err(format!("cannot create its kill group (CreateJobObjectW: error {})", GetLastError()));
                }
                let g = Group(job); // closed on every return below
                const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
                const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;
                let limit = ExtendedLimit { limit_flags: JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, ..ExtendedLimit::default() };
                let len = std::mem::size_of::<ExtendedLimit>() as u32;
                if SetInformationJobObject(job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, &limit as *const ExtendedLimit as *const _, len) == 0 {
                    return Err(format!("cannot make its kill group kill on close (SetInformationJobObject: error {})", GetLastError()));
                }
                if AssignProcessToJobObject(job, child.as_raw_handle() as isize) == 0 {
                    return Err(format!("cannot put it in its kill group (AssignProcessToJobObject: error {})", GetLastError()));
                }
                g
            };
            let why = match resume(child.id()) {
                Ok(n) if n >= 1 => return Ok(group),
                Ok(_) => "cannot resume it: no thread of it was found".to_string(),
                Err(e) => format!("cannot resume it: {e}"),
            };
            // It is in the job: kill everything in it before refusing, so nothing it may have started outlives the refusal.
            group.kill();
            Err(why)
        }

        /// Kill every process in the job. True if the call succeeded.
        pub fn kill(&self) -> bool {
            // SAFETY: self.0 is a job handle this value owns.
            unsafe { TerminateJobObject(self.0, 1) != 0 }
        }

        /// How many processes in the job are still running (`ActiveProcesses`); an error if the job cannot be asked.
        pub fn running(&self) -> Result<u32, String> {
            const JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION: i32 = 1;
            let mut info = Accounting::default();
            let len = std::mem::size_of::<Accounting>() as u32;
            // SAFETY: self.0 is a job handle this value owns; `info` is a sized, writable JOBOBJECT_BASIC_ACCOUNTING_INFORMATION.
            let ok = unsafe { QueryInformationJobObject(self.0, JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION, &mut info as *mut Accounting as *mut _, len, std::ptr::null_mut()) };
            if ok == 0 {
                // SAFETY: a plain kernel call with no arguments.
                return Err(format!("QueryInformationJobObject: error {}", unsafe { GetLastError() }));
            }
            Ok(info.active_processes)
        }
    }

    /// Resume every thread of process `pid` (a process created suspended has one, its main thread); how many were resumed.
    #[cfg(windows)]
    fn resume(pid: u32) -> Result<usize, String> {
        const TH32CS_SNAPTHREAD: u32 = 0x0000_0004;
        const THREAD_SUSPEND_RESUME: u32 = 0x0002;
        // SAFETY: the snapshot and thread handles are ours and closed before returning; `entry` is a sized THREADENTRY32.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snap == -1 {
                return Err(format!("CreateToolhelp32Snapshot: error {}", GetLastError()));
            }
            let mut entry = ThreadEntry { size: std::mem::size_of::<ThreadEntry>() as u32, usage: 0, thread_id: 0, owner_pid: 0, base_pri: 0, delta_pri: 0, flags: 0 };
            let mut resumed = 0usize;
            let mut failed: Option<String> = None;
            let mut more = Thread32First(snap, &mut entry) != 0;
            while more {
                if entry.owner_pid == pid {
                    let t = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id);
                    if t == 0 {
                        failed = Some(format!("OpenThread {}: error {}", entry.thread_id, GetLastError()));
                    } else {
                        match ResumeThread(t) {
                            u32::MAX => failed = Some(format!("ResumeThread {}: error {}", entry.thread_id, GetLastError())),
                            // its previous suspend count: 0 means it was already running, so it may have run outside the job
                            0 => failed = Some(format!("thread {} was already running: the command was not created suspended", entry.thread_id)),
                            _ => resumed += 1,
                        }
                        CloseHandle(t);
                    }
                }
                entry.size = std::mem::size_of::<ThreadEntry>() as u32;
                more = Thread32Next(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
            match failed {
                Some(e) => Err(e),
                None => Ok(resumed),
            }
        }
    }

    #[cfg(windows)]
    impl Drop for Group {
        fn drop(&mut self) {
            // SAFETY: closing the job handle this value owns. The job was set to kill on close (`start`), so closing its last
            // handle kills anything still in it: a dropped Group never leaves a process alive.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    /// Elsewhere there is no containment, so no group can be made and nothing runs.
    #[cfg(not(windows))]
    pub struct Group;

    #[cfg(not(windows))]
    impl Group {
        pub fn start(_child: &Child, _cfg: &crate::config::Config) -> Result<Group, String> {
            Err(super::NEEDS_WINDOWS.into())
        }

        pub fn kill(&self) -> bool {
            false
        }

        pub fn running(&self) -> Result<u32, String> {
            Err(super::NEEDS_WINDOWS.into())
        }
    }
}

/// The result of the whole done check.
#[derive(Debug, Clone)]
pub struct Report {
    pub plan: Plan,
    /// One per item of the plan: `Some` for a command that ran.
    pub ran: Vec<Option<Ran>>,
    /// The first item that did not pass, and its verdict.
    pub stopped_at: Option<(usize, Verdict)>,
}

impl Report {
    pub fn verdict(&self) -> Verdict {
        self.stopped_at.as_ref().map(|(_, v)| v.clone()).unwrap_or(Verdict::Pass)
    }

    /// The text the reviewer and the `.donecheck` file get (redact it again before it is stored or sent: belt and braces).
    /// `covered` is what a COVERED item says: the queue check's result, or that the train check is pending.
    pub fn text(&self, exe: &Path, limits: &Limits, covered: &str) -> String {
        let mut s = String::new();
        if !self.plan.found {
            s.push_str("The brief has no \"Done check\" section: nothing was run by the queue.\n");
            return s;
        }
        let run = self.ran.iter().filter(|r| r.is_some()).count();
        let covered_n = self.plan.items.iter().filter(|i| matches!(i, Item::Covered { .. })).count();
        s.push_str(&format!(
            "{} item(s) in the brief's Done check: {} runnable, {} run by the queue, {} COVERED by the queue's check, {} NOT RUN. Program: {} (this queue's own binary; commands were passed as an argument list, no shell, in the task's worktree, timeout {} s each, last {} lines kept).\n",
            self.plan.items.len(),
            self.plan.runnable(),
            run,
            covered_n,
            self.plan.items.len() - run - covered_n,
            exe.display(),
            limits.timeout.as_secs(),
            limits.tail_lines
        ));
        if self.plan.items.is_empty() {
            s.push_str("The Done check section lists no numbered command.\n");
        }
        for (i, item) in self.plan.items.iter().enumerate() {
            s.push('\n');
            match (item, &self.ran[i]) {
                (Item::Run { label, text, args }, Some(r)) => {
                    let v = match r.verdict() {
                        Verdict::Pass => "PASS".to_string(),
                        Verdict::Fail(m) => format!("FAIL ({m})"),
                        Verdict::Inconclusive(m) => format!("NO VERDICT ({m})"),
                    };
                    s.push_str(&format!("[{label}] RUN: {text}\n  argv: {:?}\n  result: {v}; {}; {:.1} s\n", args, r.ended_text(limits), r.secs));
                    if r.dropped > 0 {
                        s.push_str(&format!("  last {} line(s) of its output ({} earlier line(s) cut):\n", r.tail.len(), r.dropped));
                    } else {
                        s.push_str(&format!("  its output ({} line(s)):\n", r.tail.len()));
                    }
                    for l in &r.tail {
                        s.push_str(&format!("    {l}\n"));
                    }
                }
                (Item::Run { label, text, .. }, None) => {
                    s.push_str(&format!("[{label}] NOT RUN (an earlier done-check command did not pass): {text}\n"));
                }
                (Item::Covered { label, text }, _) => {
                    s.push_str(&format!("[{label}] {covered}: {text}\n  not run again: the queue's own check runs the same build and tests on this commit\n"));
                }
                (Item::NotRun { label, text, why }, _) => {
                    s.push_str(&format!("[{label}] NOT RUN (manual): {text}\n  why: {why}\n"));
                }
            }
        }
        s
    }
}

/// Run every runnable item of `plan` in order in `dir`, stopping at the first one that does not pass.
pub fn run_plan(cfg: &Config, exe: &Path, dir: &Path, plan: Plan, limits: &Limits, secrets: &Secrets, mut progress: impl FnMut(&str)) -> Report {
    let mut ran: Vec<Option<Ran>> = vec![None; plan.items.len()];
    let mut stopped_at = None;
    for (i, item) in plan.items.iter().enumerate() {
        let Item::Run { label, text, args } = item else { continue };
        progress(&format!("done check [{label}]: {text}"));
        let r = run_one(cfg, exe, dir, args, limits, secrets);
        let v = r.verdict();
        progress(&format!("done check [{label}]: {}", match &v {
            Verdict::Pass => format!("passed in {:.1} s", r.secs),
            Verdict::Fail(m) => format!("FAILED: {m}"),
            Verdict::Inconclusive(m) => format!("no verdict: {m}"),
        }));
        ran[i] = Some(r);
        if v != Verdict::Pass {
            stopped_at = Some((i, v));
            break;
        }
    }
    Report { plan, ran, stopped_at }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "t-1";

    fn c(cmd: &str) -> Result<Vec<String>, String> {
        // `broadsword` is always a project (SELF_PROJECT), so no folder is needed for these
        classify(cmd, Path::new("no-such-projects-folder"), "broadsword", T)
    }

    #[test]
    fn off_windows_every_runnable_line_is_not_run_and_covered_lines_stay_covered() {
        let p = Path::new("no-such-projects-folder");
        for cmd in ["broadsword build broadsword --task t-1", "broadsword run broadsword --task t-1 -- cargo test"] {
            assert!(classify_for(cmd, p, "broadsword", T, true).is_ok(), "runnable on Windows: {cmd}");
            let why = classify_for(cmd, p, "broadsword", T, false).expect_err("not runnable off Windows");
            assert!(why.contains("needs Windows job-object containment"), "{why}");
        }
        let covered = classify_for("broadsword test broadsword --task t-1", p, "broadsword", T, false).unwrap();
        assert_eq!(covered[0], "test");
        let other = classify_for("cargo test", p, "broadsword", T, false).unwrap_err();
        assert!(other.contains("not the Broadsword binary"), "{other}");
    }

    #[test]
    fn the_job_commands_of_the_entrys_own_project_and_task_run() {
        assert_eq!(c("broadsword test broadsword --task t-1").unwrap(), ["test", "broadsword", "--task", "t-1"]);
        assert_eq!(c("broadsword check BroadSword --task t-1").unwrap(), ["check", "broadsword", "--task", "t-1"], "the canonical name is passed");
        assert_eq!(c(r"C:\x\build\bin\broadsword.exe build broadsword --task t-1").unwrap()[0], "build", "a path to the binary is accepted (the queue's own binary runs)");
        assert_eq!(c("broadsword run broadsword --task t-1 -- cargo clippy -- -D warnings").unwrap(), ["run", "broadsword", "--task", "t-1", "--", "cargo", "clippy", "--", "-D", "warnings"]);
    }

    #[test]
    fn everything_else_is_not_run_with_a_reason() {
        for (cmd, why) in [
            ("python -I scripts/x.py", "not the Broadsword binary"),
            ("cargo test", "not the Broadsword binary"),
            ("broadsword queue run --all", "not one of the job commands"),
            ("broadsword task done t-1", "not one of the job commands"),
            ("broadsword test broadsword", "no `--task t-1`"),
            ("broadsword test broadsword --task t-2", "names task `t-2`"),
            ("broadsword test ../x --task t-1", "not usable"),
            ("broadsword test crash-sim --task t-1", "not usable"),
            ("broadsword test broadsword --task t-1 --release", "takes nothing after"),
            ("broadsword run broadsword --task t-1", "needs `-- <command>`"),
            ("broadsword run broadsword --task t-1 -- echo a & del x", "shell character"),
            ("broadsword run broadsword --task t-1 -- type x | more", "shell character"),
            ("broadsword run broadsword --task t-1 -- echo %PATH%", "shell character"),
            ("broadsword test <project> --task t-1", "unfilled placeholder"),
            ("broadsword run broadsword --task t-1 -- echo \"a b\"", "shell character"),
            ("notbroadsword test broadsword --task t-1", "not the Broadsword binary"),
            ("", "empty"),
        ] {
            let e = c(cmd).expect_err(cmd);
            assert!(e.contains(why), "{cmd}: {e}");
        }
    }

    const BRIEF: &str = "# Brief: x\n\n**Task:** do `broadsword test broadsword --task <id>` things\n\n**Done check (each line is a command and what it must show):**\n1. `broadsword test broadsword --task <id>` -> `test: ok`\n2. `python -I scripts/check.py` -> ok\n3. A scratch-root run's file, pasted.\n   - see `state/queue/x.donecheck`\n4) `broadsword run broadsword --task <task id> -- cargo clippy` -> clean\n\n**Next section:**\n5. `broadsword test broadsword --task t-1`\n";

    #[test]
    fn the_done_check_section_is_parsed_and_nothing_is_dropped() {
        let p = plan(BRIEF, Path::new("none"), "broadsword", T);
        assert!(p.found);
        let labels: Vec<&str> = p.items.iter().map(|i| i.label()).collect();
        assert_eq!(labels, ["1", "2", "3", "line 1", "4"], "{p:?}");
        assert!(matches!(&p.items[0], Item::Covered { text, .. } if text == "broadsword test broadsword --task t-1"), "test/check lines are COVERED, never run");
        assert!(matches!(&p.items[1], Item::NotRun { why, .. } if why.contains("not the Broadsword binary")));
        assert!(matches!(&p.items[2], Item::NotRun { why, .. } if why == "no command in backticks"));
        assert!(matches!(&p.items[3], Item::NotRun { why, .. } if why.contains("not a numbered line")));
        assert!(matches!(&p.items[4], Item::Run { args, .. } if args == &["run", "broadsword", "--task", "t-1", "--", "cargo", "clippy"]), "{:?}", p.items[4]);
        assert_eq!(p.runnable(), 1, "the section ends at the next heading: item 5 is outside it");
    }

    #[test]
    fn a_brief_without_a_done_check_says_so() {
        let p = plan("# Brief\n\n**Task:** x\n", Path::new("none"), "broadsword", T);
        assert!(!p.found && p.items.is_empty());
        let r = Report { plan: p, ran: vec![], stopped_at: None };
        let lim = Limits { timeout: Duration::from_secs(1), tail_lines: 5, tail_bytes: 100 };
        assert!(r.text(Path::new("bs"), &lim, "COVERED").contains("no \"Done check\" section"));
    }

    #[test]
    fn the_tail_keeps_the_end_within_the_byte_cap() {
        let lines: Vec<String> = ["aaaa", "bbbb", "cccc"].iter().map(|s| s.to_string()).collect();
        assert_eq!(fit(&lines, 10, &Secrets::default()), (vec!["bbbb".to_string(), "cccc".to_string()], 1));
        let (one, cut) = fit(&["0123456789".to_string()], 4, &Secrets::default());
        assert_eq!((one, cut), (vec!["...6789".to_string()], 0));
    }

    #[test]
    fn exit_codes_map_to_verdicts() {
        let r = |e| Ran { ended: e, secs: 0.0, tail: vec![], dropped: 0, step_failed: false };
        let failed = |e| Ran { step_failed: true, ..r(e) };
        assert_eq!(r(Ended::Exited(Some(0))).verdict(), Verdict::Pass);
        assert_eq!(failed(Ended::Exited(Some(0))).verdict(), Verdict::Pass, "exit 0 is a pass whatever was printed");
        for c in [1, 2, 3, 128] {
            assert!(matches!(failed(Ended::Exited(Some(c))).verdict(), Verdict::Fail(_)), "exit {c} after the job's own FAILED line");
        }
        assert!(matches!(r(Ended::TimedOut).verdict(), Verdict::Fail(_)));
        for killed in [true, false] {
            assert!(matches!(r(Ended::LeftRunning { code: Some(0), killed }).verdict(), Verdict::Fail(_)), "exit 0 with something still running at the deadline never passes");
            assert!(matches!(r(Ended::CannotTell { code: Some(0), why: "x".into(), killed }).verdict(), Verdict::Fail(_)), "exit 0 with its group not asked never passes");
        }
        for e in [Ended::Exited(Some(1)), Ended::Exited(Some(2)), Ended::Exited(Some(3)), Ended::Exited(Some(4)), Ended::Exited(None), Ended::NotStarted("x".into())] {
            assert!(matches!(r(e.clone()).verdict(), Verdict::Inconclusive(_)), "{e:?}");
        }
    }

    /// `n` bytes of `x` with no newline, then `rest`: generated as read, so the test itself never holds the long line.
    struct LongLine {
        left: usize,
        rest: &'static [u8],
    }

    impl Read for LongLine {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.left > 0 {
                let n = self.left.min(out.len());
                out[..n].fill(b'x');
                self.left -= n;
                return Ok(n);
            }
            let n = self.rest.len().min(out.len());
            out[..n].copy_from_slice(&self.rest[..n]);
            self.rest = &self.rest[n..];
            Ok(n)
        }
    }

    fn tail(max: usize, max_bytes: usize) -> Arc<Mutex<Tail>> {
        Arc::new(Mutex::new(Tail { lines: VecDeque::new(), max, max_bytes, bytes: 0, dropped: 0, marker: "run: FAILED".into(), marked: false }))
    }

    #[test]
    fn a_50_mb_line_is_cut_to_the_cap_and_says_so_and_the_tail_stays_within_its_bytes() {
        let secret = "dc-unit-secret-77f3a1";
        let secrets = Arc::new(Secrets::collect(Path::new("no-such-root"), &[("DC_UNIT_TOKEN".to_string(), secret.to_string())], 6));
        let cap = 16 * 1024;
        // room for the cut line and the two after it (with the byte cap equal to `cap`, as in `run_one`, the newer lines would
        // push the cut line out of the tail: tested below)
        let t = tail(40, 4 * cap);
        // the secret sits at the start of the long line, inside the kept part; then a normal line and the job's FAILED line
        let mut first = secret.as_bytes().to_vec();
        first.extend(std::iter::repeat_n(b'y', 100));
        let src = std::io::Read::chain(std::io::Cursor::new(first), LongLine { left: 50 * 1024 * 1024, rest: b"\r\nshort line\nrun: FAILED (exit 1)\n" });
        pump(src, Arc::clone(&t), secrets, cap);
        let t = t.lock().unwrap();
        let lines: Vec<&String> = t.lines.iter().collect();
        assert_eq!(lines.len(), 3, "{:?}", lines.iter().map(|l| l.len()).collect::<Vec<_>>());
        let total = secret.len() + 100 + 50 * 1024 * 1024;
        assert!(lines[0].len() <= cap + 100, "the long line is cut to the cap: {} bytes kept", lines[0].len());
        assert!(lines[0].ends_with(&format!(" [line cut: {total} bytes, only the first {cap} kept (reviewer_check_tail_kb)]")), "never cut silently: ...{}", &lines[0][lines[0].len().saturating_sub(120)..]);
        assert!(lines[0].starts_with("[REDACTED]yyy") && !lines[0].contains(secret), "redacted before the cut");
        assert_eq!((lines[1].as_str(), t.marked), ("short line", true), "the lines after it are read as usual");
        assert!(t.bytes <= cap + 200 && t.bytes == t.lines.iter().map(|l| l.len() + 1).sum::<usize>(), "{}", t.bytes);
        drop(t);

        // with the byte cap equal to the line cap (as `run_one` sets them) the cut line is pushed out by the lines after it, and
        // the tail says so through its dropped count: still never silent
        let t = tail(40, cap);
        pump(LongLine { left: 50 * 1024 * 1024, rest: b"\nafter\n" }, Arc::clone(&t), Arc::new(Secrets::collect(Path::new("no-such-root"), &[], 6)), cap);
        let t = t.lock().unwrap();
        assert_eq!((t.lines.iter().map(String::as_str).collect::<Vec<_>>(), t.dropped), (vec!["after"], 1));
        drop(t);

        // many ordinary lines: the kept ones stay within the byte cap (the line cap alone would allow 40 x 1000 bytes)
        let t = tail(40, 4000);
        let many: String = (0..200).map(|i| format!("{i:04}{}\n", "z".repeat(996))).collect();
        pump(std::io::Cursor::new(many.into_bytes()), Arc::clone(&t), Arc::new(Secrets::collect(Path::new("no-such-root"), &[], 6)), 4000);
        let t = t.lock().unwrap();
        assert!(t.bytes <= 4000 && t.lines.len() == 3 && t.dropped == 197, "{} bytes, {} lines, {} dropped", t.bytes, t.lines.len(), t.dropped);
        assert!(t.lines.back().unwrap().starts_with("0199"), "the newest lines are the ones kept");
    }

    /// Review q-20261120-220118: a secret 3x the line cap, starting just before the cut, must never show in part.
    #[test]
    fn a_secret_three_times_the_cap_spanning_the_cut_never_shows_in_part() {
        let cap = 64;
        let secret: String = (0..3 * cap).map(|i| char::from(b"0123456789abcdef"[(i * 7 + i / 16) % 16])).collect();
        let secrets = Arc::new(Secrets::collect(Path::new("no-such-root"), &[("DC_LONG_TOKEN".to_string(), secret.clone())], 6));
        let t = tail(40, 100 * cap);
        let line = format!("{}{secret}tail-of-line\nnext\n", "y".repeat(cap - 10));
        pump(std::io::Cursor::new(line.into_bytes()), Arc::clone(&t), secrets, cap);
        let kept: String = t.lock().unwrap().lines.iter().cloned().collect::<Vec<_>>().join("\n");
        let leaks: Vec<&str> = (0..=secret.len() - 4).map(|i| &secret[i..i + 4]).filter(|s| kept.contains(s)).collect();
        assert!(leaks.is_empty(), "part of the secret is kept: {leaks:?} in {kept:?}");
    }

    /// The no-window rule is enforced, not assumed: a child that is already running (not created suspended by `prepare`) is
    /// refused by `Group::start`, and a prepared one starts in the group.
    #[cfg(windows)]
    #[test]
    fn the_kill_group_takes_only_a_child_created_suspended() {
        let cfg = Config::load_from(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
        let spawn = |suspended: bool| {
            let mut c = scrub::command(&cfg, "cmd.exe"); // the one spawn helper, as run_one
            c.args(["/C", "ping -n 3 127.0.0.1 >nul"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            if suspended {
                group::prepare(&mut c);
            }
            c.spawn().unwrap()
        };
        let mut running = spawn(false);
        std::thread::sleep(Duration::from_millis(200));
        let e = group::Group::start(&running, &cfg).err().expect("a running child is refused");
        assert!(e.contains("was already running"), "{e}");
        let _ = running.kill();
        let _ = running.wait();
        let mut held = spawn(true);
        let g = group::Group::start(&held, &cfg).expect("a suspended child is put in the group and resumed");
        assert!(g.kill(), "the group can be killed");
        assert!(held.wait().is_ok());
    }

    /// Review q-20261121-053038: `running` counts what is alive in the group (the child and the ping it starts), and a DROPPED
    /// group kills everything still in it (kill on job close): the ping, which would run ~30 s, is gone at once.
    #[cfg(windows)]
    #[test]
    fn a_dropped_kill_group_kills_what_is_left_in_it_and_running_counts_it() {
        let cfg = Config::load_from(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")).unwrap();
        let mut c = scrub::command(&cfg, "cmd.exe");
        c.args(["/C", "ping -n 30 127.0.0.1 >nul"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        group::prepare(&mut c);
        let mut child = c.spawn().unwrap();
        let g = group::Group::start(&child, &cfg).expect("in the group");
        std::thread::sleep(Duration::from_millis(500));
        let n = g.running().expect("the group can be asked");
        assert!(n >= 1, "cmd (and its ping) are running: {n}");
        assert!(child.try_wait().unwrap().is_none(), "cmd is still running before the drop ({n} in the group)");
        let t = Instant::now();
        drop(g);
        let st = child.wait().unwrap();
        println!("after the drop: {st:?} in {:.2} s", t.elapsed().as_secs_f64());
        // (a kill on job close ends the processes with exit code 0, so only the time tells it from a ping run to its end)
        assert!(t.elapsed() < Duration::from_secs(10), "a dropped group left its process running ({st:?})");
    }

    /// Review q-20261120-230940: a command whose very first act is to start a grandchild, and which then exits at once, must
    /// never let that grandchild escape the kill group (the child must not run before it is in the group). 50 runs, 10 at a
    /// time; the grandchild holds the command's output and would write its marker ~3 s later, after the 1 s deadline.
    #[cfg(windows)]
    #[test]
    fn a_grandchild_started_first_thing_never_escapes_the_kill_group_in_50_runs() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cfg = Arc::new(Config::load_from(&root).unwrap());
        let dir = root.join("scratch").join(format!("dc-unit-escape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lim = Limits { timeout: Duration::from_secs(1), tail_lines: 5, tail_bytes: 1000 };
        let handles: Vec<_> = (0..10)
            .map(|w| {
                let (cfg, dir) = (Arc::clone(&cfg), dir.clone());
                std::thread::spawn(move || {
                    let mut ended = Vec::new();
                    for k in 0..5 {
                        let marker = dir.join(format!("late-{w}-{k}.txt"));
                        let inner = format!("ping -n 4 127.0.0.1 >nul & echo late> {}", marker.display());
                        let args: Vec<String> = ["/C", "start", "/b", "cmd", "/C", &inner].iter().map(|s| s.to_string()).collect();
                        ended.push(run_one(&cfg, Path::new("cmd.exe"), &dir, &args, &lim, &Secrets::default()).ended);
                    }
                    ended
                })
            })
            .collect();
        let ended: Vec<Ended> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        std::thread::sleep(Duration::from_secs(6)); // every grandchild that escaped has written its marker by now
        let escaped: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        // Both endings kill the whole group at the deadline: LeftRunning (cmd exited, the grandchild held the output) and, on a
        // loaded machine where cmd had not exited within the 1 s deadline yet, TimedOut. Anything else did not kill the group.
        let not_killed = ended.iter().filter(|e| !matches!(e, Ended::LeftRunning { killed: true, .. } | Ended::TimedOut)).count();
        let timed_out = ended.iter().filter(|e| matches!(e, Ended::TimedOut)).count();
        println!("50 runs: {} grandchild(ren) escaped {escaped:?}; {timed_out} TimedOut, {not_killed} run(s) that did not kill the group", escaped.len());
        assert_eq!(ended.len(), 50);
        assert!(escaped.is_empty(), "{} of 50 grandchildren escaped the kill group: {escaped:?}", escaped.len());
        assert_eq!(not_killed, 0, "{ended:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A command that exits 0 but leaves a grandchild holding its stdout has not finished: at the deadline it FAILS, never passes,
    /// and the grandchild is killed (it would write a marker file a few seconds later).
    #[cfg(windows)]
    #[test]
    fn a_command_that_exits_but_leaves_a_grandchild_holding_its_output_fails_and_the_grandchild_is_killed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let cfg = Config::load_from(&root).unwrap();
        let dir = root.join("scratch").join(format!("dc-unit-grandchild-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let marker = dir.join("late.txt");
        // `start /b` leaves the inner cmd running with the outer one's stdout; the outer cmd exits 0 at once
        let inner = format!("ping -n 8 127.0.0.1 >nul & echo late> {}", marker.display());
        let args: Vec<String> = ["/C", "start", "/b", "cmd", "/C", &inner].iter().map(|s| s.to_string()).collect();
        let lim = Limits { timeout: Duration::from_secs(3), tail_lines: 5, tail_bytes: 1000 };
        let r = run_one(&cfg, Path::new("cmd.exe"), &dir, &args, &lim, &Secrets::collect(&dir, &[], 6));
        println!("{:?} after {:.1} s: {}", r.ended, r.secs, r.ended_text(&lim));
        assert_eq!(r.ended, Ended::LeftRunning { code: Some(0), killed: true });
        assert!(matches!(r.verdict(), Verdict::Fail(_)), "{:?}", r.verdict());
        assert!(r.ended_text(&lim).starts_with("DID NOT FINISH: the command exited (exit 0)"), "{}", r.ended_text(&lim));
        std::thread::sleep(Duration::from_secs(10));
        assert!(!marker.exists(), "the grandchild was killed at the deadline, so it never wrote its marker");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
