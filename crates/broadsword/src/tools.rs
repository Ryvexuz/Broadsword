//! The tool registry: every Broadsword capability is one entry in `TOOLS`, with a name, a description
//! written for an AI, typed inputs, outputs, exit codes, flags, its command-line form and its handler.
//!
//! The command line is dispatched from this table (`resolve`), so the list cannot drift from what runs.
//! Stage 2's MCP server will list tools from `TOOLS` (`registry_json` already has the shape of
//! `tools/list`) and call the same `handler` with the same typed `Inputs`.
//!
//! `broadsword hook pre-tool` is NOT a tool. It is plumbing for Claude Code's PreToolUse hook: it reads
//! the hook's JSON from stdin and answers only through its exit code. It is not a capability an AI
//! should choose to call, so it lives in `main.rs` as a CLI command outside this table.

use crate::config::Config;
use crate::{cleanup, job, json, pause, recover, sys};
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
    fn with(mut self, name: &'static str, v: Val) -> Inputs {
        self.0.push((name, v));
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

// ---- the table --------------------------------------------------------------------------

const PROJECT: Param = Param {
    name: "project",
    ty: Ty::String,
    required: true,
    description: "Project name. Must be `broadsword` (this repository) or the name of a folder in the projects folder that contains a Cargo.toml; use the `projects` tool to list them. A plain folder name only: paths, `..` and absolute paths are rejected. Anything else fails with exit code 2.",
};

const E0: ExitDoc = ExitDoc { code: "0", meaning: "success" };

pub static TOOLS: &[Tool] = &[
    Tool {
        name: "status",
        summary: "Show free disk, RAM and CPU against the limits, and whether Broadsword is paused.",
        description: "Reports the machine's health: free disk space against the disk floor, free RAM and CPU load (sampled over one second) against their warning limits, and whether Broadsword is currently paused because the disk ran low. Use it first to learn whether it is safe to start work, or to see why jobs are being refused. It only reads; it changes nothing, needs no disk space, and returns within about a second. A WARN marker on a line is information, not a failure.",
        params: &[],
        outputs: "Four text lines: disk, ram and cpu, each ending in [ok] or [WARN], then `paused: no`, or `paused: YES` followed by the reason from the pause file.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "a probe (disk, RAM or CPU) could not be read" },
        ],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword status",
        cli_examples: &["status"],
        parse: |a| (a == ["status"]).then(Inputs::default),
        handler: |cfg, _| cmd_status(cfg).map(|_| ExitCode::SUCCESS),
    },
    Tool {
        name: "cleanup",
        summary: "Free disk space now: old scratch files, oversized logs, and old build artifacts if cargo-sweep is installed.",
        description: "Runs one disk cleanup pass: deletes files in the scratch folder older than scratch_max_age_hours, cuts every *.log file in scratch to its last log_max_mb, and runs `cargo sweep --time 7` on the shared build folder only if the cargo-sweep program is installed (if it is not, the report says `cargo-sweep not installed, skipped` and the other steps still run, so do not count on old build output being freed). If free space is still below the disk floor afterwards, it writes the pause file and sends the configured alert, which makes every job refuse to start until the `resume` tool succeeds. Use it when `status` shows low disk. It deletes only expired scratch files and, when cargo-sweep is installed, build artifacts older than 7 days (never project source, state or job records), so it is safe to repeat. It does not touch any project's working tree.",
        params: &[],
        outputs: "A report: the cargo sweep result (or that it was skipped), each deleted file and capped log, bytes freed, free space before and after, and `PAUSED: still below the floor` if it paused Broadsword.",
        exit_codes: &[E0],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: true,
        idempotent: true,
        cli: "broadsword cleanup",
        cli_examples: &["cleanup"],
        parse: |a| (a == ["cleanup"]).then(Inputs::default),
        handler: |cfg, _| {
            run_cleanup(cfg);
            Ok(ExitCode::SUCCESS)
        },
    },
    Tool {
        name: "watch",
        summary: "Keep watching free disk and clean up on a schedule (runs until stopped).",
        description: "Runs the resource monitor in the foreground and never finishes on its own: it cleans up once at start, then checks free disk every disk_check_every_seconds, cleans up immediately when space falls below the floor, and cleans up every cleanup_every_hours. Each cleanup is the same pass as the `cleanup` tool, so it deletes expired scratch files and may pause Broadsword. Use it only to leave a monitor running in the background for a long session; it must be started as a background process and stopped by killing it, because it never returns. Do not call it expecting a result.",
        params: &[],
        outputs: "A start line, then one line per check that triggers a cleanup, each followed by the cleanup report. Never ends normally.",
        exit_codes: &[ExitDoc { code: "1", meaning: "the monitor could not run (it has no success exit: it only ends when killed)" }],
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: true,
        idempotent: false,
        cli: "broadsword watch",
        cli_examples: &["watch"],
        parse: |a| (a == ["watch"]).then(Inputs::default),
        handler: |cfg, _| cmd_watch(cfg).map(|_| ExitCode::SUCCESS),
    },
    Tool {
        name: "resume",
        summary: "Lift the low-disk pause once there is enough free space.",
        description: "Removes the pause file so jobs can start again, but only if free disk space is above the disk floor; otherwise it refuses and says how much more must be freed. Use it after `cleanup` or after freeing space by hand when `status` says `paused: YES`. With force set it lifts the pause even though space is still low, which is dangerous: use it only when the user explicitly says so. Does nothing, successfully, if Broadsword is not paused. It changes one file (the pause flag) and returns immediately.",
        params: &[Param {
            name: "force",
            ty: Ty::Boolean,
            required: false,
            description: "Lift the pause even if free space is still below the disk floor. Default false.",
        }],
        outputs: "`not paused`, or `resumed (free space ...)`, or on refusal a message on stderr giving the free space and how much more is needed.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "refused: free space is not above the floor and force was not set; or the pause file could not be removed" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword resume [--force]",
        cli_examples: &["resume", "resume --force"],
        parse: |a| match a {
            ["resume"] => Some(Inputs::default()),
            ["resume", "--force"] => Some(Inputs::default().with("force", Val::Bool(true))),
            _ => None,
        },
        handler: |cfg, i| cmd_resume(cfg, i.bool("force")),
    },
    Tool {
        name: "projects",
        summary: "List the projects Broadsword can build, test and run.",
        description: "Lists every known project and its folder: `broadsword` itself plus each folder in the projects folder that contains a Cargo.toml (for example the crash simulator). Call it before `build`, `test`, `check` or `run` to learn the valid values of their project input. It only reads and returns immediately.",
        params: &[],
        outputs: "One line per project: the name, a tab, and the project's folder.",
        exit_codes: &[E0],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword projects",
        cli_examples: &["projects"],
        parse: |a| (a == ["projects"]).then(Inputs::default),
        handler: |cfg, _| Ok(job::cmd_projects(cfg)),
    },
    Tool {
        name: "build",
        summary: "Run `cargo build` on a project, with the disk guard and a Git checkpoint.",
        description: "Builds a project with `cargo build --offline` as a recorded job. Before starting it recovers any interrupted earlier jobs, checks the disk guard (it refuses to start if Broadsword is paused or if free space minus job_disk_reserve_gb would be below the floor) and, if the project is a Git repository, takes a Git checkpoint of it (a failed checkpoint only prints a warning and the job runs without one; a project that is not a Git repository is simply run without one). While it runs, free space is polled; if it falls below the floor the job is killed and Broadsword is paused. It does not edit source files. It writes build output to the shared build folder, a log in scratch, a job record in state, the checkpoint refs under refs/broadsword/ in the project's Git data, and cargo may update the project's Cargo.lock. It can run for minutes, so give it a long timeout or run it in the background. Use it to compile a project; use `check` to build and test in one go.",
        params: &[PROJECT],
        outputs: "A `job:` line with the job id and log path, the checkpoint ref, then `--- summary ---` and `build: ok (N s)` or `build: FAILED (exit N)`. Full compiler output goes to the log file, not to the terminal.",
        exit_codes: JOB_EXITS,
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword build <project>",
        cli_examples: &["build broadsword", "build nope"],
        parse: |a| match a {
            ["build", p] => Some(Inputs::default().with("project", Val::Str((*p).into()))),
            _ => None,
        },
        handler: |cfg, i| job_tool(cfg, "build", i),
    },
    Tool {
        name: "test",
        summary: "Run `cargo test` on a project, with the disk guard and a Git checkpoint.",
        description: "Runs a project's tests with `cargo test --offline` as a recorded job, with the same recovery, disk guard (refuses when paused or when free space minus the reserve would be below the floor), Git checkpoint (only if the project is a Git repository; otherwise it runs without one) and low-disk kill as `build`. It does not run a separate build first beyond what cargo test itself compiles. It does not edit source files; it writes build output, a log, a job record and checkpoint refs, and cargo may update Cargo.lock. Tests the project runs may of course write their own files. It can run for minutes, so give it a long timeout or run it in the background. Use it when only the test result matters; use `check` for build then test.",
        params: &[PROJECT],
        outputs: "A `job:` line, the checkpoint ref, then `--- summary ---` and `test: ok — N passed, N failed` or `test: FAILED (exit N) — N passed, N failed`. Full output goes to the log file.",
        exit_codes: JOB_EXITS,
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword test <project>",
        cli_examples: &["test broadsword", "test nope"],
        parse: |a| match a {
            ["test", p] => Some(Inputs::default().with("project", Val::Str((*p).into()))),
            _ => None,
        },
        handler: |cfg, i| job_tool(cfg, "test", i),
    },
    Tool {
        name: "check",
        summary: "Build, then test, a project in one job (stops at the first failure).",
        description: "The one command that answers whether a project works: runs `cargo build --offline` and then `cargo test --offline` as a single recorded job, and never tests a build that failed. It has the same recovery, disk guard, Git checkpoint (if a Git repository) and low-disk kill as `build`, and the disk guard is re-checked before each step. It does not edit source files; it writes build output, a log, a job record and checkpoint refs (when the project is a Git repository), and cargo may update Cargo.lock. It can run for many minutes, so give it a long timeout or run it in the background. Use it as the default way to verify a change.",
        params: &[PROJECT],
        outputs: "A `job:` line, the checkpoint ref, then `--- summary ---` and one line per step (`build: ok (N s)`, `test: ok — N passed, N failed`, or the failure). Full output goes to the log file.",
        exit_codes: JOB_EXITS,
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword check <project>",
        cli_examples: &["check broadsword", "check nope"],
        parse: |a| match a {
            ["check", p] => Some(Inputs::default().with("project", Val::Str((*p).into()))),
            _ => None,
        },
        handler: |cfg, i| job_tool(cfg, "check", i),
    },
    Tool {
        name: "run",
        summary: "Run any shell command inside a project, protected by a Git checkpoint.",
        description: "Runs an arbitrary command line in the project's folder through the shell, as a recorded job. Because the command may change files, the project must be a Git repository: Broadsword first saves a checkpoint of the whole working tree (refs/broadsword/checkpoints/<job id>) and refuses to run without one. If Broadsword is killed mid-run, the next `recover` rolls the files back to the checkpoint, and until then new jobs on that project are refused. It has the same disk guard and low-disk kill as `build`, and can run as long as the command does. Because the command's own exit code is passed through, exit codes 1-4 are ambiguous on their own: see the exit codes below for how to tell Broadsword's refusals from the command's result. Use it for project-specific commands such as running a simulation; prefer `build`, `test` or `check` for those.",
        params: &[
            PROJECT,
            Param {
                name: "command",
                ty: Ty::StringArray,
                required: true,
                description: "The command and its arguments, as separate strings. They are joined with single spaces and run by the shell in the project folder, so quoting inside one string is not preserved. Must not be empty.",
            },
        ],
        outputs: "A `job:` line, the checkpoint ref, then `--- summary ---` and `run: ok` or `run: FAILED (exit N)`. The command's own output goes to the log file. The `--- summary ---` line is printed only when the command actually ran to completion.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "the command exited 0" },
            ExitDoc { code: "1", meaning: "the command exited 1 or failed without a usable code, or Broadsword could not set the job up (log or record could not be written)" },
            ExitDoc { code: "2", meaning: "Broadsword: unknown project name; or the command itself exited 2" },
            ExitDoc { code: "3", meaning: "Broadsword refused to start (paused or not enough free disk; project is not a Git repository or no checkpoint could be taken; an earlier interrupted run on this project still needs `recover`); or the command itself exited 3" },
            ExitDoc { code: "4", meaning: "Broadsword killed the command because free disk fell below the floor, and is now paused; or the command itself exited 4" },
            ExitDoc { code: "N", meaning: "any other number is the command's own exit code. HOW TO TELL: the command's exit code is reported only after a `--- summary ---` line on stdout followed by `run: ok` or `run: FAILED (exit N)`. If there is no `--- summary ---` line, Broadsword itself stopped the job (the reason is on stderr, starting `broadsword:`) and the code is one of 1-4 as documented for build; only then is `recover` possibly relevant" },
        ],
        read_only: false,
        long_running: true,
        changes_files: true,
        destructive: true,
        idempotent: false,
        cli: "broadsword run <project> -- <command...>",
        cli_examples: &["run broadsword -- cargo --version", "run nope -- x"],
        parse: |a| match a {
            ["run", p, "--", cmd @ ..] if !cmd.is_empty() => {
                Some(Inputs::default().with("project", Val::Str((*p).into())).with("command", Val::List(cmd.iter().map(|s| (*s).into()).collect())))
            }
            _ => None,
        },
        handler: |cfg, i| {
            let project = i.require("project")?;
            let command = i.list("command");
            if command.is_empty() {
                return Err("missing required input `command`".into());
            }
            Ok(guarded(cfg, project, || job::cmd_run(cfg, project, &command)))
        },
    },
    Tool {
        name: "recover",
        summary: "Clean up after a crash: resume interrupted builds/tests, roll back interrupted runs.",
        description: "Finds every job whose Broadsword process died while it was running. Read-only jobs (build, test, check) are resumed from the step they were on, at most max_resumes times in a row. An interrupted `run` job is rolled back: the project's files are restored to the Git checkpoint taken before it started, and the partial work is saved under refs/broadsword/ first, so nothing is lost. Use it after a crash, a reboot or a message that a project is blocked by an interrupted run. It changes files in the affected project and may rebuild, so it can take a while. Jobs still genuinely running are left alone. Every build, test, check and run already does the safe half (resume, never roll back) automatically before it starts.",
        params: &[],
        outputs: "One block per interrupted job saying what was done, or `recover: nothing to do`. Problems are listed on stderr as NEEDS ATTENTION with the job ids.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "nothing to do, or every interrupted job was recovered" },
            ExitDoc { code: "1", meaning: "a job needs attention (could not be rolled back or reached max_resumes), a resumed job failed, or another recovery holds the lock" },
        ],
        read_only: false,
        long_running: true,
        changes_files: true,
        destructive: true,
        idempotent: true,
        cli: "broadsword recover",
        cli_examples: &["recover"],
        parse: |a| (a == ["recover"]).then(Inputs::default),
        handler: |cfg, _| Ok(recover::recover(cfg, recover::Mode::Explicit)),
    },
    Tool {
        name: "abandon",
        summary: "Give up on one interrupted job without touching any project file.",
        description: "Marks a dead, interrupted job as abandoned so that new jobs on its project are no longer refused, without restoring any files. Use it only when rolling back with `recover` is not wanted or not possible, for example when the work done by an interrupted `run` should be kept and inspected by hand. The job's record and every Git ref stay, and the output prints commands to see what was saved. It refuses a job that is still running or already finished. It changes one job record.",
        params: &[Param {
            name: "id",
            ty: Ty::String,
            required: true,
            description: "The exact job id, as shown in the first column of the `jobs` tool's output.",
        }],
        outputs: "A confirmation that the job was abandoned and no file changed, then the checkpoint refs saved for it and a git command to compare files.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "not a valid job id, no record with that exact id, the job is not `running`, it is still alive, or the record could not be saved" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword recover --abandon <id>",
        cli_examples: &["recover --abandon some-id"],
        parse: |a| match a {
            ["recover", "--abandon", id] => Some(Inputs::default().with("id", Val::Str((*id).into()))),
            _ => None,
        },
        handler: |cfg, i| Ok(recover::abandon(cfg, i.require("id")?)),
    },
    Tool {
        name: "jobs",
        summary: "List every job record and its status.",
        description: "Lists all recorded jobs, oldest first, with each one's status (running, done, failed, refused, killed, resumed, rolled back, abandoned), so you can find job ids and see what happened after a crash. A `running` job whose process is gone is interrupted and needs `recover` or `abandon`. It only reads job records and returns immediately; it does not recover anything.",
        params: &[],
        outputs: "One tab-separated line per job: id, status, kind and project, command, plus a note for resumed or rolled-back jobs; or a line saying there are no records.",
        exit_codes: &[E0],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword jobs  (alias: broadsword recover --list)",
        cli_examples: &["jobs", "recover --list"],
        parse: |a| (a == ["jobs"] || a == ["recover", "--list"]).then(Inputs::default),
        handler: |cfg, _| Ok(recover::cmd_list(cfg)),
    },
    Tool {
        name: "tools",
        summary: "List these tools, or describe one in full.",
        description: "Prints this registry: every tool Broadsword offers, with its inputs and flags. Give a tool name to see that tool in full (description, inputs, outputs, exit codes, command-line form). Set json to get the registry as JSON: a bare array of tool objects in the shape of the entries of MCP's tools/list (an MCP server wraps it as {\"tools\": [...]}). Use it to discover what Broadsword can do. It only reads and returns immediately.",
        params: &[
            Param { name: "name", ty: Ty::String, required: false, description: "Show only this tool, in full. Omit to list all tools. An unknown name fails with exit code 2." },
            Param { name: "json", ty: Ty::Boolean, required: false, description: "Print JSON instead of text: an array of {name, description, inputSchema, annotations, _meta}, or one such object when name is given. The array is what an MCP server puts under the `tools` key of its tools/list result. Default false." },
        ],
        outputs: "The tool list, or one tool in full, as text or (with json) as JSON.",
        exit_codes: &[E0, ExitDoc { code: "2", meaning: "no tool with that name" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword tools [<name>] [--json]",
        cli_examples: &["tools", "tools check", "tools --json", "tools check --json"],
        parse: parse_tools,
        handler: |_, i| Ok(cmd_tools(i.str("name"), i.bool("json"))),
    },
];

/// The exit codes shared by build, test and check.
const JOB_EXITS: &[ExitDoc] = &[
    ExitDoc { code: "0", meaning: "every step succeeded" },
    ExitDoc { code: "1", meaning: "a step failed (see the log), or the job could not be set up (log or record could not be written)" },
    ExitDoc { code: "2", meaning: "unknown project name; stderr lists the known projects" },
    ExitDoc { code: "3", meaning: "refused: Broadsword is paused, free disk minus the reserve is below the floor, or an earlier interrupted run on this project still needs `recover`" },
    ExitDoc { code: "4", meaning: "killed: free disk fell below the floor while the job ran, and Broadsword is now paused" },
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
        std::thread::sleep(Duration::from_secs(cfg.disk_check_every_seconds.max(1)));
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
/// on the same project still waits for `broadsword recover`, because it may have left files half changed.
fn guarded(cfg: &Config, project: &str, job: impl FnOnce() -> ExitCode) -> ExitCode {
    let _ = recover::recover(cfg, recover::Mode::Auto); // kills orphans, resumes read-only jobs; never rolls back
    if let Some(id) = recover::blocked_by_interrupted_run(cfg, project) {
        eprintln!("broadsword: refusing to start: job {id} (run {project}) was interrupted and may have changed files; run `broadsword recover` first (or `broadsword recover --abandon {id}` to give up on it)");
        return ExitCode::from(job::EXIT_REFUSED);
    }
    job()
}

fn job_tool(cfg: &Config, kind: &str, i: &Inputs) -> Result<ExitCode, String> {
    let project = i.require("project")?;
    Ok(guarded(cfg, project, || job::cmd_job(cfg, kind, project)))
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
    let f: Vec<&str> = [(t.read_only, "read-only"), (t.long_running, "long-running"), (t.changes_files, "changes-files")]
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
    for e in all_exits(t) {
        s.push_str(&format!("  {}  {}\n", e.code, e.meaning));
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
pub fn all_exits(t: &Tool) -> impl Iterator<Item = &ExitDoc> {
    t.exit_codes.iter().chain(COMMON_EXITS)
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
            let items = if p.ty == Ty::StringArray { ", \"items\": {\"type\": \"string\"}" } else { "" };
            format!("{i3}{}: {{\"type\": {}{items}, \"description\": {}}}", json::string(p.name), json::string(json_type(p.ty)), json::string(p.description))
        })
        .collect();
    let props = if props.is_empty() { "{}".to_string() } else { format!("{{\n{}\n{i2}}}", props.join(",\n")) };
    let required: Vec<String> = t.params.iter().filter(|p| p.required).map(|p| json::string(p.name)).collect();
    let codes: Vec<String> = all_exits(t).map(|e| format!("{{\"code\": {}, \"meaning\": {}}}", json::string(e.code), json::string(e.meaning))).collect();
    format!(
        "{{\n\
         {i1}\"name\": {},\n\
         {i1}\"description\": {},\n\
         {i1}\"inputSchema\": {{\n\
         {i2}\"type\": \"object\",\n\
         {i2}\"properties\": {props},\n\
         {i2}\"required\": [{}]\n\
         {i1}}},\n\
         {i1}\"annotations\": {{\"readOnlyHint\": {}, \"destructiveHint\": {}, \"idempotentHint\": {}}},\n\
         {i1}\"_meta\": {{\"broadsword\": {{\n\
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
        json::string(t.cli),
        json::string(t.outputs),
        codes.join(", "),
        t.read_only,
        t.long_running,
        t.changes_files,
        flag_meanings_json(),
    )
}

pub const READ_ONLY_MEANS: &str = "read_only: makes no change anywhere (writes no files, deletes nothing).";
pub const LONG_RUNNING_MEANS: &str = "long_running: can run for minutes or indefinitely; give it a long timeout or run it in the background.";
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
    use crate::json::check::{parse, J};

    fn snake(n: &str) -> bool {
        !n.is_empty() && n.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') && !n.starts_with(|c: char| c.is_ascii_digit() || c == '_')
    }

    #[test]
    fn every_tool_and_param_is_described() {
        for t in TOOLS {
            assert!(!t.summary.trim().is_empty(), "{} summary", t.name);
            assert!(t.description.trim().len() > 80, "{} description too short", t.name);
            assert!(!t.description.contains('\n'), "{} description must be one paragraph", t.name);
            assert!(!t.outputs.trim().is_empty(), "{} outputs", t.name);
            assert!(all_exits(t).count() >= 2, "{} exit codes", t.name);
            for e in all_exits(t) {
                assert!(!e.meaning.trim().is_empty(), "{} exit {}", t.name, e.code);
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
        assert_eq!(changing, ["run", "recover"]);
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
            assert_eq!(codes.len(), all_exits(t).count());
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
        let Ok(mut child) = std::process::Command::new("python")
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
            "status", "cleanup", "watch", "resume", "resume --force", "projects", "build p", "test p", "check p", "run p -- cmd a b", "recover", "recover --abandon id",
            "recover --list", "jobs", "tools", "tools x", "tools --json",
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
