//! The tool registry: every Broadsword capability is one entry in `TOOLS`, with a name, a description
//! written for an AI, typed inputs, outputs, exit codes, flags, its command-line form and its handler.
//!
//! The command line is dispatched from this table (`resolve`), so the list cannot drift from what runs.
//! Stage 2's MCP server will list tools from `TOOLS` (`registry_json` already has the shape of
//! `tools/list`) and call the same `handler` with the same typed `Inputs`.
//!
//! `broadsword mcp` (the MCP transport, see mcp.rs) is NOT a tool either: it is how tools are reached.
//! `broadsword hook pre-tool` is NOT a tool. It is plumbing for Claude Code's PreToolUse hook: it reads
//! the hook's JSON from stdin and answers only through its exit code. It is not a capability an AI
//! should choose to call, so it lives in `main.rs` as a CLI command outside this table.

use crate::config::Config;
use crate::{cleanup, install, job, json, pause, queue, recover, sys, task, worktree};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

// ---- types ------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ty {
    String,
    #[allow(dead_code)] // part of the schema vocabulary; no tool takes an integer yet
    Integer,
    Boolean,
    StringArray,
}

impl Ty {
    pub fn name(self) -> &'static str {
        match self {
            Ty::String => "string",
            Ty::Integer => "integer",
            Ty::Boolean => "boolean",
            Ty::StringArray => "string-array",
        }
    }
}

pub struct Param {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    pub description: &'static str,
}

pub struct ExitDoc {
    /// A number, or text such as "N" for "the command's own exit code".
    pub code: &'static str,
    pub meaning: &'static str,
}

pub struct Tool {
    pub name: &'static str,
    /// One line, for the plain list.
    pub summary: &'static str,
    /// One paragraph for an AI that has never seen the project.
    pub description: &'static str,
    pub params: &'static [Param],
    pub outputs: &'static str,
    pub exit_codes: &'static [ExitDoc],
    pub read_only: bool,
    pub long_running: bool,
    pub changes_files: bool,
    /// MCP `destructiveHint`: may delete or overwrite existing data.
    pub destructive: bool,
    /// MCP `idempotentHint`: repeating the call with the same inputs has no further effect.
    pub idempotent: bool,
    /// The command-line form, for usage text.
    pub cli: &'static str,
    /// Command lines (without the program name, space separated) that must resolve to this tool,
    /// aliases included. Tests check each one.
    #[allow(dead_code)] // read by the tests that prove every entry is reachable from the command line
    pub cli_examples:&'static [&'static str],
    /// Match the full argument list (command word included) and extract typed inputs.
    pub parse: fn(&[&str]) -> Option<Inputs>,
    /// The inverse of `parse`: command-line arguments (without the program name) that parse back to these inputs.
    /// The MCP server runs a call as `broadsword <to_args>` in a child process.
    pub to_args: fn(&Inputs) -> Vec<String>,
    /// False for a tool that never ends (`watch`): the MCP server refuses it.
    pub mcp_callable: bool,
    /// Do the work. `Err` is printed as `broadsword: <message>` and exits 1.
    pub handler: fn(&Config, &Inputs) -> Result<ExitCode, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Str(String),
    #[allow(dead_code)] // see Ty::Integer
    Int(i64),
    Bool(bool),
    List(Vec<String>),
}

/// Typed inputs of one call: what the command line parses into today and what an MCP call will carry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inputs(Vec<(&'static str, Val)>);

impl Inputs {
    pub fn with(mut self, name: &'static str, v: Val) -> Inputs {
        self.0.push((name, v));
        self
    }
    /// The form `parse` produces: a boolean that is false is the same as absent.
    pub fn canonical(mut self) -> Inputs {
        self.0.retain(|(_, v)| *v != Val::Bool(false));
        self
    }
    fn get(&self, name: &str) -> Option<&Val> {
        self.0.iter().find(|(n, _)| *n == name).map(|(_, v)| v)
    }
    pub fn str(&self, name: &str) -> Option<&str> {
        match self.get(name) {
            Some(Val::Str(s)) => Some(s),
            _ => None,
        }
    }
    pub fn bool(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Val::Bool(true)))
    }
    pub fn list(&self, name: &str) -> Vec<&str> {
        match self.get(name) {
            Some(Val::List(l)) => l.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        }
    }
    fn require(&self, name: &str) -> Result<&str, String> {
        self.str(name).ok_or_else(|| format!("missing required input `{name}`"))
    }
}

/// The free-text string parameters, as (tool, parameter, line breaks allowed). Each one ALWAYS follows its own flag as one
/// argument in `to_args` (`--title <text>`, `--note <text>`), and the command-line parser takes the argument after a flag
/// verbatim, so a leading `-` cannot be read as an option. Line breaks: `title` and `note` have every whitespace run collapsed
/// to one space by the task board, so a multi-line value is stored safely. NOT listed on purpose: `owner` (`task_start`'s parser
/// removes every `--worktree` word before it reads the flags, so `--owner --worktree` would not round-trip) and every other
/// string (a name, an id, a path): a bare argument, or one the parser cannot take verbatim.
const FREE_TEXT: &[(&str, &str, bool)] = &[("task_add", "title", true), ("task_done", "note", true), ("task_fail", "note", true)];

/// The one place that keeps a caller's string value from becoming a command-line option of the child: every
/// string parameter of every tool becomes an argument in `to_args`. A plain value (an id, a name, a path) is a BARE argument,
/// so a value starting with `-` (`--all`, `--worktree`) would be read as a flag, and it is refused; so are NUL and line
/// breaks. The free-text parameters in `FREE_TEXT` are exempt from the `-` rule (they always follow their flag) and, where the
/// table says so, from the line-break rule. NUL is refused everywhere. Array parameters (the `run` command) are meant to be
/// arguments and are exempt. The MCP server calls this before `to_args`. Each parameter's description says which rule applies.
pub fn check_string_inputs(tool: &Tool, inputs: &Inputs) -> Result<(), String> {
    for p in tool.params.iter().filter(|p| p.ty == Ty::String) {
        let Some(v) = inputs.str(p.name) else { continue };
        let free = FREE_TEXT.iter().find(|(t, n, _)| *t == tool.name && *n == p.name);
        if v.starts_with('-') && free.is_none() {
            return Err(format!("argument `{}` must not start with `-`: it would be read as a command-line option, not as a value; got {v:?}", p.name));
        }
        if v.contains('\0') {
            return Err(format!("argument `{}` must not contain a NUL", p.name));
        }
        if v.contains(['\n', '\r']) && !free.is_some_and(|(_, _, multi)| *multi) {
            return Err(format!("argument `{}` must not contain a line break", p.name));
        }
    }
    Ok(())
}

// ---- the table --------------------------------------------------------------------------

/// The sentence every plain string parameter ends with: over MCP such a value becomes one bare argument of the tool's command
/// line, so `check_string_inputs` refuses a leading `-` (it would be read as an option) and a NUL or line break.
macro_rules! NO_DASH {
    () => {
        " Over MCP this value must not start with `-` (it would be read as a command-line option) and must not contain a NUL or a line break; such a call is refused."
    };
}

/// Lazlo's secret-file set as `git::is_secret_in` decides it, in one text so every description says the same.
macro_rules! SECRET_FILES {
    () => {
        "a secret file is one named `.env`, `.envrc` or `.env.<name>`, or whose name starts with `.env-` or `.env_`, at any folder depth, in any letter case, trailing dots and spaces ignored; an NTFS short name or stream of such a file counts too. `.envoy.yaml`, `.environment` and files inside a folder named `.env` are ordinary files"
    };
}

const PROJECT: Param = Param {
    name: "project",
    ty: Ty::String,
    required: true,
    description: concat!("Project name. Must be `broadsword` (this repository) or the name of a folder in the projects folder that contains a Cargo.toml; use the `projects` tool to list them. A plain folder name only: paths, `..` and absolute paths are rejected. Anything else fails with exit code 2.", NO_DASH!()),
};

const TASK: Param = Param {
    name: "task",
    ty: Ty::String,
    required: false,
    description: concat!("The id of a task (see `task_list`) to run in that task's own worktree instead of the project's main checkout. The task must be `running`, belong to `project` and have a worktree (`task_start` with `worktree`) that really is a git worktree of the project. Omit it to use the main checkout. An unknown task id fails with exit code 2; a task that may not be used fails with exit code 3 and nothing runs.", NO_DASH!()),
};

/// The paragraph about `task` and the shared build folder that `build`, `test` and `check` all end with.
macro_rules! TASK_AND_CLEAN {
    () => {
        " WITH `task` (the id of a `running` task that has a worktree, see `task_start`) the job runs in that task's worktree folder instead of the project's main checkout, so an agent working in a worktree builds and tests its own code; `project` must be the task's project. Everything else is the same: disk guard, recovery (resume; there is no checkpoint, see above), log, job record and the one shared build folder; the `source:` output line and the job record name the folder that was built. SHARED BUILD FOLDER, DELETES ITS OWN OLD ARTIFACTS: checkouts of one package share one build folder, and cargo cannot tell their binaries apart (it can print `Finished` and leave another checkout's binary in place), so Broadsword notes in state/build-sources/<package> which folder last built each package. When the folder about to be built differs from the note, or there is no note, it first runs `cargo clean -p <package> --offline` for each such package, which DELETES that package's own artifacts in the shared build folder (its binary included; never its dependencies, never source files, never anything outside the build folder) so they are rebuilt from the right source; a package with no note yet is cleaned and rebuilt once. The clean, the note and all the job's cargo steps (tests included) happen under the build lock (state/build.lock), held for the whole job: seconds to many minutes. A second build, test or check job waits up to the build_lock_wait_seconds setting (900 s as shipped) and is then refused with exit code 3 `build folder busy`, having built, cleaned and recorded nothing; repeat it later. That exit 3 comes after the wait even when the holder is a long but legitimate job (a big test run can hold the lock for many minutes), so a waiter may need to repeat the call. WORKSPACE MEMBERS ONLY: the packages cleaned are those of the folder's own workspace (`cargo metadata --no-deps`); a path dependency outside the workspace is neither noted nor cleaned, so a stale copy of it is not covered. The clean only removes regenerable build artifacts, which is why these tools are not marked destructive. On Windows a running binary cannot be deleted: if the clean fails the job stops with exit code 1 before building, and says so (run the installed copy under install_dir, not the one in the build folder)."
    };
}

const E0: ExitDoc = ExitDoc { code: "0", meaning: "success" };

mod board_tools;
mod job_tools;
mod queue_tools;

pub static TOOLS: &[Tool] = &[
    job_tools::STATUS,
    job_tools::CLEANUP,
    job_tools::WATCH,
    job_tools::RESUME,
    job_tools::PROJECTS,
    job_tools::BUILD,
    job_tools::TEST,
    job_tools::CHECK,
    job_tools::RUN,
    job_tools::RECOVER,
    job_tools::ABANDON,
    job_tools::JOBS,
    job_tools::TOOLS,
    job_tools::INSTALL,
    board_tools::TASK_ADD,
    board_tools::TASK_START,
    board_tools::TASK_DONE,
    board_tools::TASK_FAIL,
    board_tools::TASK_ABANDON,
    board_tools::TASK_LIST,
    board_tools::TASK_SHOW,
    board_tools::TASK_CHECK,
    board_tools::WORKTREE_CREATE,
    board_tools::WORKTREE_REMOVE,
    board_tools::WORKTREE_PRUNE,
    board_tools::WORKTREE_LIST,
    board_tools::WORKTREE_REPAIR,
    queue_tools::QUEUE_ADD,
    queue_tools::QUEUE_LIST,
    queue_tools::QUEUE_SHOW,
    queue_tools::QUEUE_RUN,
    queue_tools::QUEUE_CANCEL,
];


fn finish_args(verb: &str, i: &Inputs) -> Vec<String> {
    let mut v = vec!["task".to_string(), verb.into(), i.str("id").unwrap_or_default().into()];
    if let Some(n) = i.str("note") {
        v.push("--note".into());
        v.push(n.into());
    }
    v
}

/// Parse `task <sub> [<positional>] [--flag value]...` into inputs in the tool's parameter order (`order`).
/// `flags` are (flag, input name, repeatable); a repeatable flag collects a list. Anything unknown, repeated or
/// missing (every name in `order` before the first optional one is required) makes it `None`.
fn parse_task(a: &[&str], sub: &str, positional: Option<&'static str>, flags: &[(&str, &'static str, bool)], order: &[&'static str]) -> Option<Inputs> {
    parse_sub(a, "task", sub, positional, flags, order)
}

/// `parse_task` for any command word: `<word> <sub> [<positional>] [--flag value]...` (`queue add ...`, `task add ...`).
fn parse_sub(a: &[&str], word: &str, sub: &str, positional: Option<&'static str>, flags: &[(&str, &'static str, bool)], order: &[&'static str]) -> Option<Inputs> {
    let rest = match a {
        [w, s, rest @ ..] if *w == word && *s == sub => rest,
        _ => return None,
    };
    let mut pos: Option<&str> = None;
    let mut singles: Vec<(&'static str, &str)> = Vec::new();
    let mut lists: Vec<(&'static str, Vec<String>)> = Vec::new();
    let mut it = rest.iter().copied();
    while let Some(w) = it.next() {
        if let Some((_, name, multi)) = flags.iter().find(|(f, _, _)| *f == w) {
            let v = it.next()?;
            if *multi {
                match lists.iter_mut().find(|(n, _)| n == name) {
                    Some((_, l)) => l.push(v.into()),
                    None => lists.push((name, vec![v.into()])),
                }
            } else if singles.iter().any(|(n, _)| n == name) {
                return None;
            } else {
                singles.push((name, v));
            }
        } else if w.starts_with("--") || positional.is_none() || pos.is_some() {
            return None;
        } else {
            pos = Some(w);
        }
    }
    let mut inputs = Inputs::default();
    for name in order {
        if Some(*name) == positional {
            inputs = inputs.with(name, Val::Str(pos?.into()));
        } else if let Some((_, v)) = singles.iter().find(|(n, _)| n == name) {
            inputs = inputs.with(name, Val::Str((*v).into()));
        } else if let Some((_, l)) = lists.iter().find(|(n, _)| n == name) {
            inputs = inputs.with(name, Val::List(l.clone()));
        } else if required_by_shape(word, sub, name) {
            return None;
        }
    }
    Some(inputs)
}

/// Parse `worktree <sub> [<id>] [flag]`: the id is the one positional (when `positional`), `flag` an optional boolean flag
/// given as (command-line flag, input name). Anything else makes it `None`.
fn parse_worktree(a: &[&str], sub: &str, positional: bool, flag: Option<(&str, &'static str)>) -> Option<Inputs> {
    let rest = match a {
        ["worktree", s, rest @ ..] if *s == sub => rest,
        _ => return None,
    };
    let (mut id, mut on) = (None, false);
    for w in rest {
        if flag.is_some_and(|(f, _)| f == *w) && !on {
            on = true;
        } else if w.starts_with("--") || !positional || id.is_some() {
            return None;
        } else {
            id = Some(*w);
        }
    }
    let mut inputs = Inputs::default();
    if positional {
        inputs = inputs.with("id", Val::Str(id?.into()));
    }
    if on {
        inputs = inputs.with(flag?.1, Val::Bool(true));
    }
    Some(inputs)
}

/// Which inputs of each `task` subcommand must be present.
fn required_by_shape(word: &str, sub: &str, name: &str) -> bool {
    matches!((word, sub, name), ("task", "add", "title" | "claims") | ("task", "start", "owner") | ("task", "check", "claims") | ("queue", "add", "risk" | "coder"))
}

/// The exit codes shared by build, test and check.
const JOB_EXITS: &[ExitDoc] = &[
    ExitDoc { code: "0", meaning: "every step succeeded" },
    ExitDoc { code: "1", meaning: "a step failed (see the log), or the job could not be set up (log or record could not be written; or `cargo metadata` or `cargo clean -p` failed, so the shared build folder could not be made safe and nothing was built)" },
    ExitDoc { code: "2", meaning: "unknown project name (stderr lists the known projects), or with `task` no task with that id" },
    ExitDoc { code: "3", meaning: "refused, nothing built or cleaned: Broadsword is paused, free disk minus the reserve is below the floor, an earlier interrupted run in the same source folder still needs `recover`; with `task`: the task is not `running`, has no finished worktree, belongs to another project, or its folder is not really a git worktree of the project; or the build lock was not free within build_lock_wait_seconds (`build folder busy`: repeat later)" },
    ExitDoc { code: "4", meaning: "killed: free disk fell below the floor while the job ran; Broadsword ran one cleanup pass, and is now paused unless that freed enough space" },
];

fn parse_tools(a: &[&str]) -> Option<Inputs> {
    let rest = a.strip_prefix(&["tools"])?;
    let mut inputs = Inputs::default();
    for w in rest {
        if *w == "--json" {
            inputs = inputs.with("json", Val::Bool(true));
        } else if w.starts_with("--") || inputs.get("name").is_some() {
            return None;
        } else {
            inputs = inputs.with("name", Val::Str((*w).into()));
        }
    }
    Some(inputs)
}

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|t| t.name == name)
}

/// CLI arguments -> registry entry plus typed inputs. `None` = not a tool command line.
pub fn resolve(args: &[&str]) -> Option<(&'static Tool, Inputs)> {
    TOOLS.iter().find_map(|t| (t.parse)(args).map(|i| (t, i)))
}

pub fn usage_text() -> String {
    let mut s = String::from("usage: broadsword <command>\ncommands (see `broadsword tools` for what each one does):\n");
    for t in TOOLS {
        s.push_str(&format!("  {}\n", t.cli.strip_prefix("broadsword ").unwrap_or(t.cli)));
    }
    s.push_str("  hook pre-tool   (internal: Claude Code's PreToolUse hook, not a tool)\n");
    s.push_str("  mcp             (internal: MCP server over stdio, not a tool)\n");
    s
}

// ---- handlers ---------------------------------------------------------------------------

fn free_now(cfg: &Config) -> u64 {
    sys::disk(&cfg.root).map(|d| d.free).unwrap_or_else(|e| {
        eprintln!("disk check failed: {e}");
        0
    })
}

fn flag(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "WARN"
    }
}

fn cmd_status(cfg: &Config) -> Result<(), String> {
    let d = sys::disk(&cfg.root)?;
    let r = sys::ram()?;
    let cpu = sys::cpu_percent(Duration::from_secs(1))?;
    println!(
        "disk: {} free of {} (floor {} GB) [{}]",
        pause::gb(d.free),
        pause::gb(d.total),
        cfg.disk_floor_gb,
        flag(!pause::below_floor(d.free, cfg.floor_bytes()))
    );
    let used = (r.total - r.avail) as f64 * 100.0 / r.total as f64;
    println!(
        "ram:  {} free of {} ({:.0}% used, warn at {}%) [{}]",
        pause::gb(r.avail),
        pause::gb(r.total),
        used,
        cfg.ram_warn_percent,
        flag(used < cfg.ram_warn_percent as f64)
    );
    println!("cpu:  {:.0}% over 1s (warn at {}%) [{}]", cpu, cfg.cpu_warn_percent, flag(cpu < cfg.cpu_warn_percent as f64));
    match pause::read_pause(&cfg.root) {
        Some(body) => println!("paused: YES\n{}", body.trim_end()),
        None => println!("paused: no"),
    }
    Ok(())
}

fn run_cleanup(cfg: &Config) {
    let r = cleanup::cleanup_pass(cfg, SystemTime::now(), &mut || free_now(cfg), true);
    cleanup::print_report(&r);
}

fn cmd_watch(cfg: &Config) -> Result<(), String> {
    println!(
        "watching: disk every {}s, cleanup every {}h, floor {} GB (Ctrl+C to stop)",
        cfg.disk_check_every_seconds, cfg.cleanup_every_hours, cfg.disk_floor_gb
    );
    let every = Duration::from_secs(cfg.cleanup_every_hours * 3600);
    let mut last = Instant::now();
    run_cleanup(cfg);
    loop {
        std::thread::sleep(Duration::from_secs(cfg.disk_check_every_seconds));
        let free = free_now(cfg);
        if pause::below_floor(free, cfg.floor_bytes()) {
            println!("[{}] free space {} below floor, running cleanup", pause::now_string(), pause::gb(free));
            run_cleanup(cfg);
            last = Instant::now();
        } else if last.elapsed() >= every {
            println!("[{}] scheduled cleanup", pause::now_string());
            run_cleanup(cfg);
            last = Instant::now();
        }
    }
}

fn cmd_resume(cfg: &Config, force: bool) -> Result<ExitCode, String> {
    if !pause::is_paused(&cfg.root) {
        println!("not paused");
        return Ok(ExitCode::SUCCESS);
    }
    let free = free_now(cfg);
    match pause::resume_decision(free, cfg.floor_bytes(), force) {
        Ok(()) => {
            std::fs::remove_file(pause::pause_path(&cfg.root)).map_err(|e| e.to_string())?;
            println!("resumed (free space {})", pause::gb(free));
            Ok(ExitCode::SUCCESS)
        }
        Err(need) => {
            eprintln!(
                "refusing to resume: free space {} is not above the {} GB floor; free at least {} more (or use --force)",
                pause::gb(free),
                cfg.disk_floor_gb,
                pause::gb(need)
            );
            Ok(ExitCode::from(1))
        }
    }
}

/// Run a job after the automatic recovery step. A job is refused (exit 3) while an interrupted `run` job
/// in the same source folder still waits for `broadsword recover`, because it may have left files half changed.
fn guarded(cfg: &Config, project: &str, task: Option<&str>, job: impl FnOnce() -> ExitCode) -> ExitCode {
    match guard_refusal(cfg, project, task) {
        Some(code) => ExitCode::from(code),
        None => job(),
    }
}

/// The recovery step every job starts with (also used by the merge queue before its check jobs): `Some(exit code)` when the
/// job must be refused, with the reason already printed. `task` is the job's `--task` (`None` = the project's main checkout):
/// the automatic recovery touches only interrupted jobs of exactly this project and task (others are reported, one line
/// each, and left alone), and an interrupted run blocks only jobs in its own source folder (see `recover::blocked_by_interrupted_run`).
pub fn guard_refusal(cfg: &Config, project: &str, task: Option<&str>) -> Option<u8> {
    let _ = recover::recover(cfg, recover::Mode::Auto { project, task }); // this call's own jobs only: kills their orphans, resumes read-only ones; never rolls back
    if let Some((id, source)) = recover::blocked_by_interrupted_run(cfg, project, task) {
        eprintln!("broadsword: refusing to start: job {id} (run {project}, source folder {source}) was interrupted and may have changed files in that folder; run `broadsword recover {id}` first to roll it back (only if nobody is still working in that folder), or `broadsword recover --abandon {id}` to give up on it");
        return Some(job::EXIT_REFUSED);
    }
    None
}

fn job_tool(cfg: &Config, kind: &str, i: &Inputs) -> Result<ExitCode, String> {
    let project = i.require("project")?;
    Ok(guarded(cfg, project, i.str("task"), || job::cmd_job(cfg, kind, project, i.str("task"))))
}

/// The command line of a build, test or check call: `<kind> <project> [--task <id>]`.
fn job_args(kind: &str, i: &Inputs) -> Vec<String> {
    let mut v = vec![kind.to_string(), i.str("project").unwrap_or_default().into()];
    if let Some(t) = i.str("task") {
        v.push("--task".into());
        v.push(t.into());
    }
    v
}

fn cmd_tools(name: Option<&str>, as_json: bool) -> ExitCode {
    match name {
        Some(n) => match find(n) {
            Some(t) => {
                if as_json {
                    println!("{}", tool_json(t, ""));
                } else {
                    print!("{}", tool_full(t));
                }
                ExitCode::SUCCESS
            }
            None => {
                eprintln!("broadsword: unknown tool `{n}`; known tools: {}", TOOLS.iter().map(|t| t.name).collect::<Vec<_>>().join(", "));
                ExitCode::from(job::EXIT_UNKNOWN_PROJECT) // exit 2 is "unknown name" everywhere
            }
        },
        None => {
            if as_json {
                println!("{}", registry_json());
            } else {
                print!("{}", tool_list());
            }
            ExitCode::SUCCESS
        }
    }
}

// ---- text output ------------------------------------------------------------------------

fn flags_text(t: &Tool) -> String {
    let f: Vec<&str> = [(t.read_only, "read-only"), (t.long_running, "long-running"), (t.changes_files, "changes-files"), (!t.mcp_callable, "not-callable-over-mcp")]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, n)| *n)
        .collect();
    if f.is_empty() {
        "none".into()
    } else {
        f.join(", ")
    }
}

fn inputs_text(t: &Tool) -> String {
    if t.params.is_empty() {
        return "none".into();
    }
    t.params.iter().map(|p| format!("{}: {}{}", p.name, p.ty.name(), if p.required { "" } else { " (optional)" })).collect::<Vec<_>>().join(", ")
}

pub fn tool_list() -> String {
    let w = TOOLS.iter().map(|t| t.name.len()).max().unwrap_or(0);
    let mut s = String::new();
    for t in TOOLS {
        s.push_str(&format!("{:<w$}  {}\n", t.name, t.summary));
        s.push_str(&format!("{:<w$}  inputs: {}; flags: {}\n", "", inputs_text(t), flags_text(t)));
    }
    s.push_str("\n`broadsword tools <name>` shows one tool in full; `broadsword tools --json` prints the registry as JSON.\n");
    s
}

pub fn tool_full(t: &Tool) -> String {
    let mut s = format!("{}\n  {}\n\n{}\n\ncommand line: {}\nflags: {}\n\ninputs:\n", t.name, t.summary, t.description, t.cli, flags_text(t));
    if t.params.is_empty() {
        s.push_str("  none\n");
    }
    for p in t.params {
        s.push_str(&format!("  {} ({}, {}): {}\n", p.name, p.ty.name(), if p.required { "required" } else { "optional" }, p.description));
    }
    s.push_str(&format!("\noutputs:\n  {}\n\nexit codes:\n", t.outputs));
    for (code, meaning) in all_exits(t) {
        s.push_str(&format!("  {code}  {meaning}\n"));
    }
    s.push_str(&format!("\nflag meanings:\n  {READ_ONLY_MEANS}\n  {LONG_RUNNING_MEANS}\n  {CHANGES_FILES_MEANS}\n"));
    s
}

/// Exit codes that every tool can produce, documented once.
pub const COMMON_EXITS: &[ExitDoc] = &[ExitDoc {
    code: "1",
    meaning: "any tool: broadsword.toml could not be loaded, or the tool failed with an error (printed on stderr as `broadsword: ...`)",
}];

/// A tool's own exit codes followed by the common ones.
/// Each code appears once; when a tool and the common list share a code, the meanings are joined.
pub fn all_exits(t: &Tool) -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = t.exit_codes.iter().map(|e| (e.code, e.meaning.to_string())).collect();
    for c in COMMON_EXITS {
        match v.iter_mut().find(|(k, _)| *k == c.code) {
            Some((_, m)) => {
                m.push_str("; ");
                m.push_str(c.meaning);
            }
            None => v.push((c.code, c.meaning.to_string())),
        }
    }
    v
}

// ---- JSON output (MCP tools/list shape) -------------------------------------------------

fn json_type(ty: Ty) -> &'static str {
    match ty {
        Ty::String => "string",
        Ty::Integer => "integer",
        Ty::Boolean => "boolean",
        Ty::StringArray => "array",
    }
}

/// One tool as `{name, description, inputSchema, annotations, _meta}`, an entry of MCP's tools/list `tools` array (the server wraps the array as `{"tools": [...]}`). `ind` is the indent of the opening brace.
pub fn tool_json(t: &Tool, ind: &str) -> String {
    let i1 = format!("{ind}  ");
    let i2 = format!("{ind}    ");
    let i3 = format!("{ind}      ");
    let props: Vec<String> = t
        .params
        .iter()
        .map(|p| {
            let items = match (p.ty, p.required) {
                (Ty::StringArray, true) => ", \"items\": {\"type\": \"string\"}, \"minItems\": 1",
                (Ty::StringArray, false) => ", \"items\": {\"type\": \"string\"}",
                _ => "",
            };
            format!("{i3}{}: {{\"type\": {}{items}, \"description\": {}}}", json::string(p.name), json::string(json_type(p.ty)), json::string(p.description))
        })
        .collect();
    let props = if props.is_empty() { "{}".to_string() } else { format!("{{\n{}\n{i2}}}", props.join(",\n")) };
    let required: Vec<String> = t.params.iter().filter(|p| p.required).map(|p| json::string(p.name)).collect();
    let codes: Vec<String> = all_exits(t).iter().map(|(c, m)| format!("{{\"code\": {}, \"meaning\": {}}}", json::string(c), json::string(m))).collect();
    format!(
        "{{\n\
         {i1}\"name\": {},\n\
         {i1}\"description\": {},\n\
         {i1}\"inputSchema\": {{\n\
         {i2}\"type\": \"object\",\n\
         {i2}\"properties\": {props},\n\
         {i2}\"required\": [{}],\n\
         {i2}\"additionalProperties\": false\n\
         {i1}}},\n\
         {i1}\"outputSchema\": {OUTPUT_SCHEMA},\n\
         {i1}\"annotations\": {{\"readOnlyHint\": {}, \"destructiveHint\": {}, \"idempotentHint\": {}}},\n\
         {i1}\"_meta\": {{\"broadsword\": {{\n\
         {i2}\"mcp_callable\": {},\n\
         {i2}\"cli\": {},\n\
         {i2}\"outputs\": {},\n\
         {i2}\"exit_codes\": [{}],\n\
         {i2}\"read_only\": {},\n\
         {i2}\"long_running\": {},\n\
         {i2}\"changes_files\": {},\n\
         {i2}\"flag_meanings\": {}\n\
         {i1}}}}}\n\
         {ind}}}",
        json::string(t.name),
        json::string(t.description),
        required.join(", "),
        t.read_only,
        t.destructive,
        t.idempotent,
        t.mcp_callable,
        json::string(t.cli),
        json::string(t.outputs),
        codes.join(", "),
        t.read_only,
        t.long_running,
        t.changes_files,
        flag_meanings_json(),
    )
}

/// The `outputSchema` of every tool: what an MCP `tools/call` returns as `structuredContent` (MCP 2025-06-18 and
/// later; the server leaves it out of tools/list for older protocol versions). `exit_code` is -1 when the call
/// was killed on timeout and so has no exit code.
pub const OUTPUT_SCHEMA: &str = "{\"type\": \"object\", \"properties\": {\
\"exit_code\": {\"type\": \"integer\", \"description\": \"The tool's exit code, as documented under exit codes; -1 if the call was killed on timeout and so has none.\"}, \
\"stdout\": {\"type\": \"string\", \"description\": \"The tool's standard output (the last mcp_output_max_kb KB if it was cut).\"}, \
\"stderr\": {\"type\": \"string\", \"description\": \"The tool's standard error (the last mcp_output_max_kb KB if it was cut).\"}, \
\"truncated\": {\"type\": \"boolean\", \"description\": \"True if stdout or stderr was cut to its last mcp_output_max_kb KB.\"}}, \
\"required\": [\"exit_code\", \"stdout\", \"stderr\", \"truncated\"]}";

pub const READ_ONLY_MEANS: &str = "read_only: makes no change anywhere (writes no files, deletes nothing).";
pub const LONG_RUNNING_MEANS: &str = "long_running: can run for minutes or indefinitely. Over MCP a call blocks until it ends (one call at a time) and is killed at the mcp_call_timeout_seconds limit; elsewhere give it a long timeout or run it in the background.";
pub const CHANGES_FILES_MEANS: &str = "changes_files: modifies the project's working tree (its source files). Build output, logs, job records and Git refs do not count.";

fn flag_meanings_json() -> String {
    format!("{{\"read_only\": {}, \"long_running\": {}, \"changes_files\": {}}}", json::string(READ_ONLY_MEANS), json::string(LONG_RUNNING_MEANS), json::string(CHANGES_FILES_MEANS))
}

pub fn registry_json() -> String {
    registry_json_of(TOOLS)
}

fn registry_json_of(tools: &[Tool]) -> String {
    let items: Vec<String> = tools.iter().map(|t| format!("  {}", tool_json(t, "  "))).collect();
    format!("[\n{}\n]", items.join(",\n"))
}

// ---- tests ------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{parse, J};

    fn snake(n: &str) -> bool {
        !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && !n.starts_with(|c: char| c.is_ascii_digit() || c == '_')
    }

    /// A model reads these descriptions literally: they must state Lazlo's secret-file set, never the old `.env*` prefix rule.
    #[test]
    fn descriptions_state_the_secret_file_set() {
        let all = registry_json();
        for must in [".envrc", "`.env.<name>`", "`.env-`", "`.env_`", "`.envoy.yaml`", "`.environment`"] {
            assert!(all.contains(must), "the tool descriptions must mention {must}");
        }
        let lower = all.to_lowercase();
        for old in ["starts with `.env`", "starting with `.env`", "starts with .env", "starting with .env", "`.env*`"] {
            assert!(!lower.contains(old), "the old prefix rule is still described: {old}");
        }
        for name in ["run", "recover", "queue_add"] {
            assert!(tool_full(find(name).unwrap()).contains(".envrc"), "{name} names the secret-file set");
        }
    }

    /// `task_start` must state what max_agents counts (tasks waiting in the merge queue are not counted) and show both cap
    /// refusals as task.rs prints them. The refusal fragments are also required to occur in task.rs, so the two cannot drift.
    #[test]
    fn task_start_description_states_the_cap_rules_and_refusal_texts() {
        let full = tool_full(find("task_start").unwrap());
        let task_rs = include_str!("task.rs");
        for must in [
            "tasks are running with a working coder, the max_agents cap in broadsword.toml is",
            "running tasks whose merge-queue entry is queued, running or approved are not counted",
            "start it after one of them is queued, done, failed or abandoned",
            "merge-queue entries are open, the max_queued cap in broadsword.toml is",
            "start it after the queue has merged or ended some of them",
        ] {
            assert!(task_rs.contains(must), "task.rs no longer prints: {must}");
            assert!(full.contains(must), "the task_start description must show the refusal text: {must}");
        }
        assert!(full.contains("NOT counted"), "task_start must say tasks waiting in the merge queue are not counted");
    }

    /// `queue_run` and `queue_list` must describe the merge train as queue.rs does. Every (queue.rs text, description text) pair
    /// below must hold on BOTH sides: the first part occurs in queue.rs (its messages and comments), the second in the description.
    /// The pairs differ only where queue.rs writes `{id}` and the description writes `<id>`. A description sentence that is not in
    /// this list has no quote behind it, so it must not be added.
    #[test]
    fn queue_run_description_states_the_train_rules_as_queue_rs_does() {
        let full = tool_full(find("queue_run").unwrap());
        let queue_rs = crate::queue::SOURCE;
        let pairs: &[(&str, &str)] = &[
            ("only in `queue run --all`; 1 = off, every entry as before", "TRAINS FORM ONLY UNDER `--all`"),
            ("form a train of up to merge_train_max entries", "`merge_train_max` sets the most entries one train takes"),
            ("(a lone one at the end of the drain too; and a SHORT train, even of 1, as soon as no entry of that project is left for the drain to run, or its oldest approved entry has waited merge_train_wait_minutes, so one project's approved entries never wait behind another project's stream", "(a lone one at the end of the drain too; and a SHORT train, even of 1, as soon as no entry of that project is left for the drain to run, or its oldest approved entry has waited merge_train_wait_minutes, so one project's approved entries never wait behind another project's stream"),
            ("(`short_train_why`: none of its entries left to run, or its oldest approved entry waited merge_train_wait_minutes)", "project with no entry left to run, or whose oldest approved entry has waited merge_train_wait_minutes, gets a train now"),
            ("is a half-finished merge", "is a half-finished merge"),
            ("Not cancelled; finish it with `broadsword queue run --all` or `broadsword queue run --entry {id}`", "Not cancelled; finish it with `broadsword queue run --all` or `broadsword queue run --entry <id>`"),
            ("`queue run` and `queue run --all` take it before anything else", "RESUMED FIRST, by `queue_run` with or without `all`"),
            ("it never joins a new train", "it never joins a new train"),
            ("`--all` starts no train while one is being finished.", "`all` starts no train while one is being finished"),
            ("every entry's commits as they are, in order, no squash", "every entry's commits kept in order, no squash"),
            ("the entries after it start a new round on the new main", "the entries after it start a new round on the new main"),
            ("the culprit AND every entry before it in the chain", "the failing entry and every entry before it in the chain"),
            ("e1 `conflict`, e2 `blocked`, main unchanged", "the first of them is `conflict`, the ones chained on it are `blocked`"),
            ("A check that cannot run to a verdict leaves all its entries `approved` and stops the drain", "A check that cannot run to a verdict leaves the train's entries `approved` (the next call takes them again) and stops the drain"),
            ("unless the reason is one entry's own worktree", "unless the reason is one entry's own worktree"),
            ("left a train: rebase your own commits onto main and queue again.", "left a train: rebase your own commits onto main and queue again."),
            ("git rebase --onto {main} {base} {branch}", "git rebase --onto <main> <base> <branch>"),
            ("every entry of the merge is `merged_check_failed` (exit 6), main is NOT reset, the", "If the main check fails after a train merge the exit is 6, every entry of that merge is `merged_check_failed`, main is NOT reset"),
            ("stops at `approved` when its review PASSes", "stops as `approved` when its review PASSes"),
        ];
        let flat = queue_rs.lines().map(|l| l.trim_start().trim_start_matches("//!").trim_start_matches("///").trim()).collect::<Vec<_>>().join(" ");
        for (q, d) in pairs {
            // queue.rs comments wrap lines: compare against its text with every line's indent and `//!` / `///` marker folded away
            assert!(flat.contains(q), "queue.rs no longer says: {q}");
            assert!(full.contains(d), "the queue_run description must say: {d}");
        }
        // a sentence that states the opposite of queue.rs must stay out
        for never in ["are all `conflict`", "(each passes alone)", "the ones after it form a new train", "or its worktree cannot be checked exactly, it ends"] {
            assert!(!full.contains(never), "the queue_run description must not say: {never}");
        }
        for (field, hint) in [("pub const APPROVED: &str = \"approved\";", "approved"), ("pub const BLOCKED: &str = \"blocked\";", "blocked")] {
            assert!(queue_rs.contains(field), "queue.rs no longer defines the status {hint}");
        }
        let list = tool_full(find("queue_list").unwrap());
        assert!(list.contains("approved ("), "queue_list must list the `approved` status");
        assert!(!list.contains("nothing is merged yet"), "queue_list: an approved entry may be a half-finished merge, so it must not say nothing is merged");
    }

    /// The trunk rule and the recovery scope as the tool descriptions state them. Each (source file, code text, tool, description text)
    /// row must hold on BOTH sides: the code text occurs in the file (comments and messages, line indent and `//!` / `///` folded
    /// away), the description text in the tool's description. A description sentence about these rules has no quote behind it unless it is here.
    #[test]
    fn tools_state_the_trunk_rule_and_the_recovery_scope_as_the_code_does() {
        let flat = |src: &str| src.lines().map(|l| l.trim_start().trim_start_matches("//!").trim_start_matches("///").trim_start_matches("//").trim()).collect::<Vec<_>>().join(" ");
        let (wtgit, worktree, queue, recover) = (flat(include_str!("wtgit.rs")), flat(include_str!("worktree.rs")), flat(crate::queue::SOURCE), flat(include_str!("recover.rs")));
        let rows: &[(&str, &str, &str, &str)] = &[
            (&wtgit, "the ONE that exists (`git show-ref --verify --quiet refs/heads/<name>` for each)", "worktree_create", "the one of the local branches `main` and `master` that exists"),
            (&worktree, "whatever the main checkout has checked out; a trunk that cannot be told is refused", "worktree_create", "whatever the main checkout has checked out"),
            (&wtgit, "has neither `main` nor `master`", "task_start", "the trunk cannot be told (both or neither of `main` and `master`)"),
            (&wtgit, "has both `main` and `master`, so its trunk cannot be told", "worktree_create", "when the trunk cannot be told, because the project has both `main` and `master` or neither"),
            (&worktree, "merged = reachable from the trunk (not from the checked-out branch)", "worktree_remove", "an ancestor of the project's trunk"),
            (&worktree, "merged = reachable from the trunk (not from the checked-out branch)", "worktree_list", "`merged` into the project's trunk"),
            (&queue, "a feature branch or a detached HEAD there ends the entry `blocked` naming both, and that branch and the checkout are never moved, merged into or switched", "queue_run", "a feature branch or a detached HEAD there ends the entry `blocked`, and the checkout is not touched, moved or merged into"),
            (&queue, "`git rebase <trunk tip>` in the task's worktree", "queue_run", "`git rebase <trunk tip>`"),
            (&queue, "`git merge --ff-only <reviewed commit>` in the main checkout while it has the trunk checked out", "queue_run", "in the main checkout, and only while that checkout has the trunk checked out"),
            (&queue, "the check was refused before it started", "queue_run", "it is REFUSED (the entry is `merged_check_failed`"),
            (&queue, "does not hold commit", "queue_run", "the trunk holds the merged commit"),
            (&queue, "The review diff is `<base>...<checked_tip>`", "queue_run", "<trunk tip at the rebase>...the checked commit"),
            (&recover, "The automatic recovery acts only on jobs of the CALLER's own project and task", "build", "recovers the interrupted earlier jobs of this call's own project and task only"),
            (&recover, "is reported on one line and left exactly as it is, for an explicit `broadsword recover` or a call of that same task", "build", "reported on one line and left as it is, for `broadsword recover` or a call for that task"),
            (&recover, "r.get(\"project\") == canon && r.get(\"task\") == task.unwrap_or(\"\")", "recover", "only for the interrupted jobs of its own project and task"),
        ];
        for (code, q, tool, d) in rows {
            assert!(code.contains(q), "the code no longer says: {q}");
            assert!(tool_full(find(tool).unwrap()).contains(d), "the {tool} description must say: {d}");
        }
        let all = registry_json();
        for stale in ["main checkout's HEAD", "recovers any interrupted", "must be on a branch, not detached", "the main checkout is on a detached HEAD", "tip of the project's current branch", "git rev-parse HEAD", "diff main-HEAD"] {
            assert!(!all.contains(stale), "a description still says: {stale}");
        }
    }

    /// The per-project run lock (SCALING F2) as the queue tools describe it, each sentence paired with the queue.rs text it restates.
    #[test]
    fn queue_tools_state_the_per_project_lock_as_queue_rs_does() {
        let run = tool_full(find("queue_run").unwrap());
        let cancel = tool_full(find("queue_cancel").unwrap());
        let queue_rs = crate::queue::SOURCE.lines().map(|l| l.trim_start().trim_start_matches("//!").trim_start_matches("///").trim()).collect::<Vec<_>>().join(" ");
        let pairs: &[(&str, &str, &str)] = &[
            (&run, "format!(\"{}.queue.lock\"", "state/queue/<project>.queue.lock"),
            (&run, "another `queue run` of project `{k}`", "another `queue run` of project `<p>`"),
            (&run, "queue_per_project is on: every project has its own run lock, so name the project with `--project <p>` (or one entry with `--entry <id>`); nothing was run", "`project` (or `entry`, whose own project is then used) is REQUIRED, without either the call exits 2 and runs nothing"),
            (&run, "`queue_per_project = \"off\"`: the one global queue.lock, exactly as before (a `key` then only narrows which entries the run takes)", "the one global state/queue/queue.lock above, exactly as before, and `project` then only narrows which entries a run takes"),
            (&run, "--project: {m}; nothing was run", "It must be a project (see the `projects` tool), else exit code 2 and nothing runs"),
            (&run, "is of project `{ek}`, not `{k}`; nothing was run", "with `entry`, that entry must be of this project (else exit code 2)"),
            (&run, "same `queue_lock_wait_seconds`, exit 4 naming the project", "a second run of the SAME project exits 4"),
            (&cancel, "with queue_per_project its project's own) proves that", "the run lock that guards the entry"),
        ];
        for (desc, q, d) in pairs {
            assert!(queue_rs.contains(q), "queue.rs no longer says: {q}");
            assert!(desc.contains(d), "the description must say: {d}");
        }
        assert!(run.contains("[--project <p>]") && run.contains("project (string, optional)"), "queue_run must show the --project flag and input");
        // the setting the descriptions name exists (its shipped value is a trial choice, "on" or "off")
        assert!(include_str!("../../../broadsword.toml").lines().any(|l| l.starts_with("queue_per_project = \"on\"") || l.starts_with("queue_per_project = \"off\"")));
    }

    /// The perf gate as the queue tools describe it, each sentence paired with the queue.rs / perfgate.rs text it restates.
    #[test]
    fn queue_tools_state_the_perf_gate_as_the_code_does() {
        let run = tool_full(find("queue_run").unwrap());
        let flat = |src: &str| src.lines().map(|l| l.trim_start().trim_start_matches("//!").trim_start_matches("///").trim()).collect::<Vec<_>>().join(" ");
        let (queue_rs, perf_rs) = (flat(crate::queue::SOURCE), flat(include_str!("perfgate.rs")));
        let pairs: &[(&str, &str, &str)] = &[
            (&queue_rs, "a project without one is skipped (printed and logged", "a project without one is skipped and the skip is logged"),
            (&queue_rs, "no other project is ever benched", "no other project is ever benched"),
            (&queue_rs, "Merge train: ONCE on the chain tip", "With a merge train the gate runs ONCE on the chain tip"),
            (&queue_rs, "a perf failure is bisected exactly like a failed check", "a perf failure is bisected exactly like a failed check"),
            (&perf_rs, "cargo run --release --offline --example bench -- --runs", "`cargo run --release --offline --example bench -- --runs <runs_per_case from the baseline>`"),
            (&perf_rs, "case's own `threshold_pct` from the baseline file", "that case's own threshold_pct (from the baseline file)"),
            (&perf_rs, "is a FAILURE (`perf_failed`), never a pass and never a skip", "are `perf_failed` too, never a pass"),
            (&perf_rs, "gives NO verdict: the entry is `blocked`", "gives no verdict: `blocked`"),
            (&perf_rs, "whether the baseline is LOADED", "the output says whether the baseline is LOADED"),
        ];
        for (src, q, d) in pairs {
            assert!(src.contains(q), "the code no longer says: {q}");
            assert!(run.contains(d), "the queue_run description must say: {d}");
        }
        assert!(crate::queue::SOURCE.contains("pub const PERF_FAILED: &str = \"perf_failed\";"));
        for t in ["queue_list", "queue_add", "queue_cancel"] {
            assert!(tool_full(find(t).unwrap()).contains("perf_failed"), "{t} must name the perf_failed status");
        }
        assert!(run.contains("`perf_failed` (the perf gate failed)"), "queue_run exit 5 must name perf_failed");
    }

    #[test]
    fn every_tool_and_param_is_described() {
        for t in TOOLS {
            assert!(!t.summary.trim().is_empty(), "{} summary", t.name);
            assert!(t.description.trim().len() > 80, "{} description too short", t.name);
            assert!(!t.description.contains('\n'), "{} description must be one paragraph", t.name);
            assert!(!t.outputs.trim().is_empty(), "{} outputs", t.name);
            let exits = all_exits(t);
            assert!(!exits.is_empty(), "{} exit codes", t.name);
            let mut seen = std::collections::HashSet::new();
            for (code, meaning) in &exits {
                assert!(!meaning.trim().is_empty(), "{} exit {code}", t.name);
                assert!(seen.insert(*code), "{} lists exit {code} twice", t.name);
            }
            for p in t.params {
                assert!(!p.description.trim().is_empty(), "{}.{} description", t.name, p.name);
                assert!(snake(p.name), "{}.{} not snake_case", t.name, p.name);
            }
        }
    }

    #[test]
    fn names_unique_and_snake_case() {
        let mut seen = std::collections::HashSet::new();
        for t in TOOLS {
            assert!(snake(t.name), "{} is not snake_case", t.name);
            assert!(seen.insert(t.name), "duplicate tool {}", t.name);
        }
        assert!(find("hook").is_none() && find("pre_tool").is_none(), "hook is plumbing, not a tool");
    }

    #[test]
    fn read_only_tools_do_not_change_files() {
        for t in TOOLS {
            assert!(!(t.read_only && (t.changes_files || t.destructive)), "{} cannot be read-only and change anything", t.name);
        }
        // changes_files = modifies the project's working tree: only a free-form command and a rollback do.
        let changing: Vec<&str> = TOOLS.iter().filter(|t| t.changes_files).map(|t| t.name).collect();
        assert_eq!(changing, ["run", "recover", "queue_run"]);
        assert!(find("tools").unwrap().read_only);
    }

    #[test]
    fn json_is_valid_and_has_the_mcp_shape() {
        let v = parse(&registry_json()).expect("registry JSON must parse");
        let J::Arr(items) = v else { panic!("top level must be an array") };
        assert_eq!(items.len(), TOOLS.len());
        for (t, j) in TOOLS.iter().zip(&items) {
            assert_eq!(j.get("name").and_then(J::str), Some(t.name));
            assert_eq!(j.get("description").and_then(J::str), Some(t.description));
            let schema = j.get("inputSchema").expect("inputSchema");
            assert_eq!(schema.get("type").and_then(J::str), Some("object"));
            let Some(J::Obj(props)) = schema.get("properties") else { panic!("properties object") };
            assert_eq!(props.len(), t.params.len());
            let Some(J::Arr(req)) = schema.get("required") else { panic!("required array") };
            let want: Vec<&str> = t.params.iter().filter(|p| p.required).map(|p| p.name).collect();
            let got: Vec<&str> = req.iter().filter_map(J::str).collect();
            assert_eq!(got, want);
            for p in t.params {
                let pj = schema.get("properties").unwrap().get(p.name).expect("property");
                assert_eq!(pj.get("type").and_then(J::str), Some(json_type(p.ty)));
                assert_eq!(pj.get("description").and_then(J::str), Some(p.description));
                if p.ty == Ty::StringArray {
                    assert_eq!(pj.get("items").and_then(|i| i.get("type")).and_then(J::str), Some("string"));
                }
            }
            assert!(j.get("x-broadsword").is_none(), "extras live under _meta");
            let x = j.get("_meta").and_then(|m| m.get("broadsword")).expect("_meta.broadsword");
            for k in ["cli", "outputs", "exit_codes", "read_only", "long_running", "changes_files", "flag_meanings"] {
                assert!(x.get(k).is_some(), "{} missing _meta.broadsword.{k}", t.name);
            }
            assert_eq!(x.get("read_only"), Some(&J::Bool(t.read_only)));
            assert_eq!(x.get("changes_files"), Some(&J::Bool(t.changes_files)));
            let a = j.get("annotations").expect("annotations");
            assert_eq!(a.get("readOnlyHint"), Some(&J::Bool(t.read_only)));
            assert_eq!(a.get("destructiveHint"), Some(&J::Bool(t.destructive)));
            assert_eq!(a.get("idempotentHint"), Some(&J::Bool(t.idempotent)));
            let J::Arr(codes) = x.get("exit_codes").unwrap() else { panic!("exit_codes array") };
            assert_eq!(codes.len(), all_exits(t).len());
        }
    }

    #[test]
    fn every_tool_has_the_output_schema_and_closed_input_schema() {
        let out = parse(OUTPUT_SCHEMA).expect("OUTPUT_SCHEMA must be valid JSON");
        let J::Arr(items) = parse(&registry_json()).unwrap() else { panic!() };
        for j in &items {
            assert_eq!(j.get("outputSchema"), Some(&out));
            assert_eq!(j.get("inputSchema").and_then(|s| s.get("additionalProperties")), Some(&J::Bool(false)));
            let m = j.get("_meta").and_then(|m| m.get("broadsword")).and_then(|b| b.get("mcp_callable"));
            assert_eq!(m, Some(&J::Bool(j.get("name").and_then(J::str) != Some("watch"))));
        }
        let Some(J::Arr(req)) = out.get("required") else { panic!() };
        assert_eq!(req.len(), 4);
        let run = items.iter().find(|j| j.get("name").and_then(J::str) == Some("run")).unwrap();
        let cmd = run.get("inputSchema").unwrap().get("properties").unwrap().get("command").unwrap();
        assert_eq!(cmd.get("minItems"), Some(&J::Num("1".into())));
    }

    #[test]
    fn only_watch_is_not_callable_over_mcp_and_says_so() {
        let no: Vec<&str> = TOOLS.iter().filter(|t| !t.mcp_callable).map(|t| t.name).collect();
        assert_eq!(no, ["watch"]);
        let w = find("watch").unwrap();
        assert!(w.description.contains("NOT AVAILABLE OVER MCP") && w.description.contains("mcp_callable"));
        assert!(flags_text(w).contains("not-callable-over-mcp"));
    }

    #[test]
    fn mcp_string_values_that_look_like_options_are_refused_naming_the_parameter() {
        use crate::mcp::{prepare_call, Reject};
        let refused = |call: &str, param: &str| {
            let params = parse(call).unwrap();
            match prepare_call(Some(&params)) {
                Err(Reject::Tool(m)) => assert!(m.contains(&format!("`{param}`")), "{call}: {m}"),
                other => panic!("{call} must be refused as a tool error, got {other:?}"),
            }
        };
        refused(r#"{"name":"recover","arguments":{"id":"--all"}}"#, "id");
        refused(r#"{"name":"recover","arguments":{"id":"-x"}}"#, "id");
        refused(r#"{"name":"task_start","arguments":{"id":"--worktree","owner":"o"}}"#, "id");
        refused(r#"{"name":"task_start","arguments":{"id":"t-1","owner":"--worktree"}}"#, "owner");
        refused(r#"{"name":"build","arguments":{"project":"--x"}}"#, "project");
        refused(r#"{"name":"build","arguments":{"project":"p","task":"--x"}}"#, "task");
        refused(r#"{"name":"abandon","arguments":{"id":"--all"}}"#, "id");
        refused(r#"{"name":"recover","arguments":{"id":"a\nb"}}"#, "id");
        refused(r#"{"name":"recover","arguments":{"id":"a\u0000b"}}"#, "id");
        // the `run` command array is meant to be arguments: leading dashes stay allowed there
        let ok = prepare_call(Some(&parse(r#"{"name":"run","arguments":{"project":"p","command":["cargo","--version"]}}"#).unwrap())).unwrap();
        assert_eq!(ok.args, ["run", "p", "--", "cargo", "--version"]);
        // a plain id still passes
        assert!(prepare_call(Some(&parse(r#"{"name":"recover","arguments":{"id":"20261117-120000-ab12"}}"#).unwrap())).is_ok());
    }

    #[test]
    fn free_text_parameters_accept_a_leading_dash_and_line_breaks_and_the_cli_parses_them_back() {
        use crate::mcp::prepare_call;
        // legitimate text over MCP: a list-style title, a multi-line note
        let call = r#"{"name":"task_add","arguments":{"project":"p","title":"- fix x\n- and y","claims":["src"]}}"#;
        let ok = prepare_call(Some(&parse(call).unwrap())).unwrap_or_else(|e| panic!("{e:?}"));
        let args: Vec<&str> = ok.args.iter().map(String::as_str).collect();
        assert_eq!(args[..5], ["task", "add", "p", "--title", "- fix x\n- and y"]);
        let (tool, inputs) = resolve(&args).expect("the command line the child gets parses back");
        assert_eq!((tool.name, inputs.str("title")), ("task_add", Some("- fix x\n- and y")));
        for call in [
            r#"{"name":"task_done","arguments":{"id":"t-1","note":"--merged\nline two"}}"#,
            r#"{"name":"task_fail","arguments":{"id":"t-1","note":"-x"}}"#,
        ] {
            let ok = prepare_call(Some(&parse(call).unwrap())).unwrap_or_else(|e| panic!("{call}: {e:?}"));
            let args: Vec<&str> = ok.args.iter().map(String::as_str).collect();
            let (tool, inputs) = resolve(&args).unwrap_or_else(|| panic!("{args:?} must parse back"));
            assert!(tool.name.starts_with("task_") && inputs.str("note").is_some_and(|n| n.starts_with(['-']) || n.contains('\n')), "{args:?}");
        }
        // the refusal stays everywhere else: a NUL anywhere, a leading dash or line break in a plain value (names, ids, owner)
        for (call, param) in [
            (r#"{"name":"task_add","arguments":{"project":"p","title":"a\u0000b","claims":["src"]}}"#, "title"),
            (r#"{"name":"task_done","arguments":{"id":"t-1","note":"a\u0000b"}}"#, "note"),
            (r#"{"name":"task_add","arguments":{"project":"-p","title":"t","claims":["src"]}}"#, "project"),
            (r#"{"name":"task_add","arguments":{"project":"p\nq","title":"t","claims":["src"]}}"#, "project"),
            (r#"{"name":"task_done","arguments":{"id":"-t","note":"n"}}"#, "id"),
            (r#"{"name":"task_start","arguments":{"id":"t-1","owner":"-o"}}"#, "owner"),
            (r#"{"name":"task_start","arguments":{"id":"t-1","owner":"o\np"}}"#, "owner"),
            (r#"{"name":"queue_add","arguments":{"task":"t-1","risk":"low","coder":"-c"}}"#, "coder"),
        ] {
            match prepare_call(Some(&parse(call).unwrap())) {
                Err(crate::mcp::Reject::Tool(m)) => assert!(m.contains(&format!("`{param}`")), "{call}: {m}"),
                other => panic!("{call} must be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn every_string_parameter_description_says_which_refusal_rule_applies() {
        for t in TOOLS {
            for p in t.params.iter().filter(|p| p.ty == Ty::String) {
                let free = FREE_TEXT.iter().any(|(tool, name, _)| *tool == t.name && *name == p.name);
                if free {
                    assert!(p.description.contains("FREE TEXT") && p.description.contains("starts with `-`"), "{}.{}: {}", t.name, p.name, p.description);
                } else {
                    assert!(p.description.contains("must not start with `-`") && p.description.contains("line break"), "{}.{} must document the refusal: {}", t.name, p.name, p.description);
                }
            }
        }
        for (tool, name, _) in FREE_TEXT {
            assert!(find(tool).is_some_and(|t| t.params.iter().any(|p| p.name == *name && p.ty == Ty::String)), "FREE_TEXT names {tool}.{name}, which does not exist");
        }
    }

    #[test]
    fn recover_is_not_marked_idempotent_because_a_second_rollback_is_refused() {
        assert!(!find("recover").unwrap().idempotent);
    }

    #[test]
    fn to_args_round_trips_through_parse_for_every_tool() {
        let s = |x: &str| Val::Str(x.into());
        let samples: Vec<(&str, Inputs)> = vec![
            ("status", Inputs::default()),
            ("cleanup", Inputs::default()),
            ("watch", Inputs::default()),
            ("projects", Inputs::default()),
            ("recover", Inputs::default()),
            ("recover", Inputs::default().with("all", Val::Bool(true))),
            ("recover", Inputs::default().with("id", s("20261117-120000-ab12"))),
            ("jobs", Inputs::default()),
            ("install", Inputs::default()),
            ("resume", Inputs::default()),
            ("resume", Inputs::default().with("force", Val::Bool(true))),
            ("build", Inputs::default().with("project", s("broadsword"))),
            ("test", Inputs::default().with("project", s("crash-sim"))),
            ("check", Inputs::default().with("project", s("p"))),
            ("run", Inputs::default().with("project", s("p")).with("command", Val::List(vec!["cargo".into(), "--version".into()]))),
            ("run", Inputs::default().with("project", s("p")).with("command", Val::List(vec!["echo".into(), "a b".into(), "--".into()]))),
            ("abandon", Inputs::default().with("id", s("20261117-120000-ab12"))),
            ("task_add", Inputs::default().with("project", s("p")).with("title", s("do a thing")).with("claims", Val::List(vec!["src".into(), "docs\\x".into()]))),
            ("task_start", Inputs::default().with("id", s("t-1")).with("owner", s("coder-1"))),
            ("task_done", Inputs::default().with("id", s("t-1"))),
            ("task_done", Inputs::default().with("id", s("t-1")).with("note", s("all merged"))),
            ("task_fail", Inputs::default().with("id", s("t-1")).with("note", s(""))),
            ("task_abandon", Inputs::default().with("id", s("t-1"))),
            ("task_list", Inputs::default()),
            ("task_list", Inputs::default().with("status", s("running"))),
            ("task_list", Inputs::default().with("status", s("open")).with("project", s("p"))),
            ("task_start", Inputs::default().with("id", s("t-1")).with("owner", s("coder-1")).with("worktree", Val::Bool(true))),
            ("worktree_create", Inputs::default().with("id", s("t-1"))),
            ("worktree_remove", Inputs::default().with("id", s("t-1"))),
            ("worktree_remove", Inputs::default().with("id", s("t-1")).with("keep_branch", Val::Bool(true))),
            ("worktree_prune", Inputs::default()),
            ("worktree_list", Inputs::default()),
            ("worktree_repair", Inputs::default()),
            ("task_show", Inputs::default().with("id", s("t-1"))),
            ("task_check", Inputs::default().with("project", s("p")).with("claims", Val::List(vec!["*".into()]))),
            ("queue_add", Inputs::default().with("task", s("t-1")).with("risk", s("low")).with("coder", s("sonnet"))),
            ("queue_add", Inputs::default().with("task", s("t-1")).with("risk", s("high")).with("coder", s("sonnet")).with("brief", s("b.md"))),
            ("queue_list", Inputs::default()),
            ("queue_show", Inputs::default().with("entry", s("q-1"))),
            ("queue_run", Inputs::default()),
            ("queue_run", Inputs::default().with("entry", s("q-1"))),
            ("queue_run", Inputs::default().with("all", Val::Bool(true))),
            ("queue_run", Inputs::default().with("entry", s("q-1")).with("all", Val::Bool(true))),
            ("queue_cancel", Inputs::default().with("entry", s("q-1"))),
            ("tools", Inputs::default()),
            ("tools", Inputs::default().with("name", s("check"))),
            ("tools", Inputs::default().with("json", Val::Bool(true))),
            ("tools", Inputs::default().with("name", s("check")).with("json", Val::Bool(true))),
        ];
        for (name, inputs) in &samples {
            let t = find(name).unwrap();
            let args = (t.to_args)(inputs);
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let (hit, parsed) = resolve(&refs).unwrap_or_else(|| panic!("{name}: {args:?} resolves to no tool"));
            assert_eq!((hit.name, &parsed), (*name, inputs), "{name}: {args:?}");
        }
        for t in TOOLS {
            assert!(samples.iter().any(|(n, _)| *n == t.name), "no round-trip sample for {}", t.name);
        }
    }

    #[test]
    fn single_tool_json_is_valid() {
        let j = parse(&tool_json(find("run").unwrap(), "")).unwrap();
        assert_eq!(j.get("name").and_then(J::str), Some("run"));
    }

    #[test]
    fn escaping_in_descriptions() {
        static ODD: Tool = Tool {
            name: "odd",
            summary: "s",
            description: "say \"hi\" \\ back\nnext line é\ttab",
            params: &[Param { name: "p", ty: Ty::String, required: true, description: "a \"q\" \\ \n é" }],
            outputs: "o",
            exit_codes: &[E0],
            read_only: true,
            long_running: false,
            changes_files: false,
        destructive: false,
        idempotent: true,
            cli: "broadsword odd \"x\"",
            cli_examples: &[],
            parse: |_| None,
            to_args: |_| Vec::new(),
            mcp_callable: true,
            handler: |_, _| Ok(ExitCode::SUCCESS),
        };
        let text = registry_json_of(std::slice::from_ref(&ODD));
        assert!(text.contains(r#"say \"hi\" \\ back\nnext line é\ttab"#), "{text}");
        let J::Arr(items) = parse(&text).unwrap() else { panic!() };
        assert_eq!(items[0].get("description").and_then(J::str), Some(ODD.description));
        let p = items[0].get("inputSchema").unwrap().get("properties").unwrap().get("p").unwrap();
        assert_eq!(p.get("description").and_then(J::str), Some(ODD.params[0].description));
    }

    /// Round-trip through Python's own parser when python is installed (skipped silently otherwise).
    #[test]
    fn json_round_trips_through_python_if_available() {
        use std::io::Write;
        // A root holding this build's own broadsword.toml: from the crate folder of a linked worktree, `command_in` would
        // load the main checkout's file, which lacks any required key this branch adds.
        let root = crate::scrub::planted::config_root("tools-python");
        let Ok(mut child) = crate::scrub::command_in(&root, "python")
            .unwrap()
            .args(["-I", "-c", "import json,sys; d=json.load(sys.stdin); assert isinstance(d,list) and all({'name','description','inputSchema'} <= set(t) for t in d); print(len(d))"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
        else {
            eprintln!("skipped: python not found");
            return;
        };
        child.stdin.take().unwrap().write_all(registry_json().as_bytes()).unwrap();
        let out = child.wait_with_output().unwrap();
        if !out.status.success() && String::from_utf8_lossy(&out.stderr).contains("Microsoft Store") {
            eprintln!("skipped: python not found (only the Windows Store shim)");
            return;
        }
        eprintln!("python validated the registry JSON");
        assert!(out.status.success(), "python rejected the JSON: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), TOOLS.len().to_string());
    }

    #[test]
    fn every_tool_is_reachable_from_its_cli_form() {
        for t in TOOLS {
            assert!(!t.cli_examples.is_empty(), "{} has no CLI example", t.name);
            for ex in t.cli_examples {
                let args: Vec<&str> = ex.split(' ').collect();
                let (hit, _) = resolve(&args).unwrap_or_else(|| panic!("`{ex}` resolves to no tool"));
                assert_eq!(hit.name, t.name, "`{ex}` resolved to the wrong tool");
            }
            assert!(t.cli.starts_with("broadsword "), "{} cli form", t.name);
        }
    }

    #[test]
    fn every_cli_command_maps_to_an_entry() {
        // Every form the CLI accepted before the registry existed, plus the new `tools`; `hook` is the one exception.
        let forms = [
            "status", "cleanup", "watch", "resume", "resume --force", "projects", "build p", "test p", "check p", "run p -- cmd a b", "recover", "recover --all", "recover some-id", "recover --abandon id",
            "recover --list", "jobs", "tools", "tools x", "tools --json", "install",
        ];
        for f in forms {
            let args: Vec<&str> = f.split(' ').collect();
            assert!(resolve(&args).is_some(), "`{f}` has no registry entry");
        }
        assert!(resolve(&["hook", "pre-tool"]).is_none(), "hook is plumbing outside the registry");
        // and nothing else resolves
        for bad in [&[][..], &["nope"], &["status", "x"], &["run", "p"], &["run", "p", "--"], &["build"], &["resume", "-f"], &["recover", "--abandon"], &["tools", "a", "b"], &["tools", "--bogus"]] {
            assert!(resolve(bad).is_none(), "{bad:?} should not resolve");
        }
        // typed inputs come through
        let (t, i) = resolve(&["run", "p", "--", "cargo", "x"]).unwrap();
        assert_eq!((t.name, i.str("project"), i.list("command")), ("run", Some("p"), vec!["cargo", "x"]));
        assert!(resolve(&["resume", "--force"]).unwrap().1.bool("force"));
        assert!(!resolve(&["resume"]).unwrap().1.bool("force"));
        assert_eq!(resolve(&["recover", "--abandon", "j1"]).unwrap().1.str("id"), Some("j1"));
        let (t, i) = resolve(&["tools", "--json", "check"]).unwrap();
        assert_eq!((t.name, i.str("name"), i.bool("json")), ("tools", Some("check"), true));
    }

    #[test]
    fn every_documented_cli_word_is_in_usage() {
        let u = usage_text();
        for t in TOOLS {
            assert!(u.contains(t.cli.strip_prefix("broadsword ").unwrap()), "usage lacks {}", t.name);
        }
        assert!(u.contains("hook pre-tool"));
    }

    #[test]
    fn job_exit_constants_match_the_documented_codes() {
        // The behaviour of each code is proved through the real binary in tests/tools.rs; this pins the numbers.
        assert_eq!((job::EXIT_FAILED, job::EXIT_UNKNOWN_PROJECT, job::EXIT_REFUSED, job::EXIT_DISK_DROPPED), (1, 2, 3, 4));
        let docs: Vec<&str> = JOB_EXITS.iter().map(|e| e.code).collect();
        assert_eq!(docs, ["0", "1", "2", "3", "4"]);
    }
}
