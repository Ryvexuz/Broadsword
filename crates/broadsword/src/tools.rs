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
use crate::{cleanup, install, job, json, pause, recover, sys, task, worktree};
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
        mcp_callable: true,
        to_args: |_| vec!["status".into()],
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
        mcp_callable: true,
        to_args: |_| vec!["cleanup".into()],
        handler: |cfg, _| {
            run_cleanup(cfg);
            Ok(ExitCode::SUCCESS)
        },
    },
    Tool {
        name: "watch",
        summary: "Keep watching free disk and clean up on a schedule (runs until stopped).",
        description: "Runs the resource monitor in the foreground and never finishes on its own: it cleans up once at start, then checks free disk every disk_check_every_seconds, cleans up immediately when space falls below the floor, and cleans up every cleanup_every_hours. Each cleanup is the same pass as the `cleanup` tool, so it deletes expired scratch files and may pause Broadsword. Use it only to leave a monitor running in the background for a long session; it must be started as a background process and stopped by killing it, because it never returns. NOT AVAILABLE OVER MCP: a call here is refused at once (isError true, nothing runs), because a tool call must end; this tool's `_meta.broadsword.mcp_callable` is false, so check that flag instead of trying it. Start it yourself from a terminal as a background process.",
        params: &[],
        outputs: "A start line, then one line per check that triggers a cleanup, each followed by the cleanup report. Never ends normally.",
        exit_codes: &[ExitDoc { code: "1", meaning: "the monitor could not run (it has no success exit: it only ends when killed). Over MCP no exit code is ever produced: the call is refused with isError true" }],
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: true,
        idempotent: false,
        cli: "broadsword watch",
        cli_examples: &["watch"],
        parse: |a| (a == ["watch"]).then(Inputs::default),
        mcp_callable: false,
        to_args: |_| vec!["watch".into()],
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
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["resume".to_string()];
            if i.bool("force") {
                v.push("--force".into());
            }
            v
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
        mcp_callable: true,
        to_args: |_| vec!["projects".into()],
        handler: |cfg, _| Ok(job::cmd_projects(cfg)),
    },
    Tool {
        name: "build",
        summary: "Run `cargo build` on a project, with the disk guard and a Git checkpoint.",
        description: "Builds a project with `cargo build --offline` as a recorded job. Before starting it recovers any interrupted earlier jobs, checks the disk guard (it refuses to start if Broadsword is paused or if free space minus job_disk_reserve_gb would be below the floor) and, if the project is a Git repository, takes a Git checkpoint of it (a failed checkpoint only prints a warning and the job runs without one; a project that is not a Git repository is simply run without one). While it runs, free space is polled; if it falls below the floor the job is killed and Broadsword is paused. It does not edit source files. It writes build output to the shared build folder, a log in scratch, a job record in state, the checkpoint refs under refs/broadsword/ in the project's Git data, and cargo may update the project's Cargo.lock. It can run for minutes; over MCP the call blocks until it ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code, and the killed job is then left interrupted for `recover` (or the next job's automatic recovery) to resume. Use it to compile a project; use `check` to build and test in one go.",
        params: &[PROJECT],
        outputs: "A `job:` line with the job id and log path, the checkpoint ref, then `--- summary ---` and `build: ok (N s)` or `build: FAILED (exit N)`. Cargo's full output is printed as it runs (compiler output on stderr) and also saved to the job log, then the summary follows.",
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
        mcp_callable: true,
        to_args: |i| vec!["build".into(), i.str("project").unwrap_or_default().into()],
        handler: |cfg, i| job_tool(cfg, "build", i),
    },
    Tool {
        name: "test",
        summary: "Run `cargo test` on a project, with the disk guard and a Git checkpoint.",
        description: "Runs a project's tests with `cargo test --offline` as a recorded job, with the same recovery, disk guard (refuses when paused or when free space minus the reserve would be below the floor), Git checkpoint (only if the project is a Git repository; otherwise it runs without one) and low-disk kill as `build`. It does not run a separate build first beyond what cargo test itself compiles. It does not edit source files; it writes build output, a log, a job record and checkpoint refs, and cargo may update Cargo.lock. Tests the project runs may of course write their own files. It can run for minutes; over MCP the call blocks until it ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code, and the killed job is then left interrupted for `recover` (or the next job's automatic recovery) to resume. Use it when only the test result matters; use `check` for build then test.",
        params: &[PROJECT],
        outputs: "A `job:` line, the checkpoint ref, then `--- summary ---` and `test: ok — N passed, N failed` or `test: FAILED (exit N) — N passed, N failed`. Cargo's full output is printed as it runs and also saved to the job log, then the summary follows.",
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
        mcp_callable: true,
        to_args: |i| vec!["test".into(), i.str("project").unwrap_or_default().into()],
        handler: |cfg, i| job_tool(cfg, "test", i),
    },
    Tool {
        name: "check",
        summary: "Build, then test, a project in one job (stops at the first failure).",
        description: "The one command that answers whether a project works: runs `cargo build --offline` and then `cargo test --offline` as a single recorded job, and never tests a build that failed. It has the same recovery, disk guard, Git checkpoint (if a Git repository) and low-disk kill as `build`, and the disk guard is re-checked before each step. It does not edit source files; it writes build output, a log, a job record and checkpoint refs (when the project is a Git repository), and cargo may update Cargo.lock. It can run for many minutes; over MCP the call blocks until it ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code, and the killed job is then left interrupted for `recover` (or the next job's automatic recovery) to resume. Use it as the default way to verify a change.",
        params: &[PROJECT],
        outputs: "A `job:` line, the checkpoint ref, then `--- summary ---` and one line per step (`build: ok (N s)`, `test: ok — N passed, N failed`, or the failure). Cargo's full output is printed as it runs and also saved to the job log, then the summary follows.",
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
        mcp_callable: true,
        to_args: |i| vec!["check".into(), i.str("project").unwrap_or_default().into()],
        handler: |cfg, i| job_tool(cfg, "check", i),
    },
    Tool {
        name: "run",
        summary: "Run any shell command inside a project, protected by a Git checkpoint.",
        description: "Runs an arbitrary command line in the project's folder through the shell, as a recorded job. Because the command may change files, the project must be a Git repository: Broadsword first saves a checkpoint of the whole working tree (refs/broadsword/checkpoints/<job id>) and refuses to run without one. If Broadsword is killed mid-run, the next `recover` rolls the files back to the checkpoint, and until then new jobs on that project are refused. It has the same disk guard and low-disk kill as `build`, and can run as long as the command does; over MCP the call blocks until the command ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code. Because the command's own exit code is passed through, exit codes 1-4 are ambiguous on their own: see the exit codes below for how to tell Broadsword's refusals from the command's result. Use it for project-specific commands such as running a simulation; prefer `build`, `test` or `check` for those.",
        params: &[
            PROJECT,
            Param {
                name: "command",
                ty: Ty::StringArray,
                required: true,
                description: "The command and its arguments, as separate strings. They are joined with single spaces and run by the shell in the project folder, so quoting inside one string is not preserved. Must not be empty.",
            },
        ],
        outputs: "A `job:` line, the checkpoint ref, then `--- summary ---` and `run: ok` or `run: FAILED (exit N)`. The command's own output is printed as it runs and also saved to the job log, then the summary follows. Because the command's output is on stdout too, only the LAST `run:` line after the final `--- summary ---` is Broadsword's verdict.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "the command exited 0" },
            ExitDoc { code: "1", meaning: "the command exited 1 or failed without a usable code, or Broadsword could not set the job up (log or record could not be written)" },
            ExitDoc { code: "2", meaning: "Broadsword: unknown project name; or the command itself exited 2" },
            ExitDoc { code: "3", meaning: "Broadsword refused to start (paused or not enough free disk; project is not a Git repository or no checkpoint could be taken; an earlier interrupted run on this project still needs `recover`); or the command itself exited 3" },
            ExitDoc { code: "4", meaning: "Broadsword killed the command because free disk fell below the floor, and is now paused; or the command itself exited 4" },
            ExitDoc { code: "N", meaning: "any other number is the command's own exit code. HOW TO TELL: Broadsword's own verdict is the last `run:` line (`run: ok` or `run: FAILED (exit N)`) after the final `--- summary ---` line on stdout; the command may print a fake one itself, so only the last counts. If there is no such line, Broadsword itself stopped the job (the reason is on stderr, starting `broadsword:`) and the code is one of 1-4 as documented for build; only then is `recover` possibly relevant" },
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
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["run".to_string(), i.str("project").unwrap_or_default().into(), "--".into()];
            v.extend(i.list("command").into_iter().map(String::from));
            v
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
        description: "Finds every job whose Broadsword process died while it was running. build, test and check jobs are resumed from the step they were on, at most max_resumes times in a row. An interrupted `run` job is rolled back: the project's files are restored to the Git checkpoint taken before it started, and the partial work is saved under refs/broadsword/ first, so nothing is lost. Use it after a crash, a reboot or a message that a project is blocked by an interrupted run. It changes files in the affected project and may rebuild, so it can take a while. Jobs still genuinely running are left alone. Every build, test, check and run already does the safe half (resume, never roll back) automatically before it starts.",
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
        mcp_callable: true,
        to_args: |_| vec!["recover".into()],
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
        mcp_callable: true,
        to_args: |i| vec!["recover".into(), "--abandon".into(), i.str("id").unwrap_or_default().into()],
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
        mcp_callable: true,
        to_args: |_| vec!["jobs".into()],
        handler: |cfg, _| Ok(recover::cmd_list(cfg)),
    },
    Tool {
        name: "tools",
        summary: "List these tools, or describe one in full.",
        description: "Returns this registry: every tool Broadsword offers, with its inputs, outputs, exit codes and flags. Give a tool name to get that one tool in full (description, inputs, outputs, exit codes, command-line form). Set json to get JSON instead of text: the same tool objects an MCP client already sees in tools/list. Use it to re-check what Broadsword can do. It only reads and returns immediately.",
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
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["tools".to_string()];
            if let Some(n) = i.str("name") {
                v.push(n.into());
            }
            if i.bool("json") {
                v.push("--json".into());
            }
            v
        },
        handler: |_, i| Ok(cmd_tools(i.str("name"), i.bool("json"))),
    },
    Tool {
        name: "install",
        summary: "Copy this binary to the stable install location that the MCP server and the pause hook run.",
        description: "Copies the broadsword binary that is running this command to <install_dir>/broadsword.exe (install_dir is a setting, build/bin as shipped), so the MCP server and Claude Code's pause hook run a file that rebuilding the project never has to overwrite. The copy is atomic (written under a temporary name, then renamed into place). If the installed copy is currently running, which locks it on Windows, it is renamed to broadsword.old-<stamp>.exe first and deleted by a later install once nothing runs it; leftovers that are no longer in use are deleted each time. Run it after `build broadsword` as `build/debug/broadsword.exe install`; a binary that is already the installed one reports `already installed` and does nothing, so over MCP (where the server normally runs the installed copy) it is only useful when the server was started from another binary. It does not touch any project file, job record or setting.",
        params: &[],
        outputs: "`installed: <path> (N bytes)`, then a line for each old copy renamed aside, removed or still in use; or `already installed: ...` when nothing was to be done.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "the copy or the rename failed (the previous installed binary is left in place), or the install folder could not be created" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: true,
        idempotent: true,
        cli: "broadsword install",
        cli_examples: &["install"],
        parse: |a| (a == ["install"]).then(Inputs::default),
        mcp_callable: true,
        to_args: |_| vec!["install".into()],
        handler: |cfg, _| install::cmd_install(cfg),
    },
    Tool {
        name: "task_add",
        summary: "Add a task to the task board: a title, a project and the paths it will claim.",
        description: "Adds a task to the task board as one durable record (state/tasks/<id>.task) with status `open`, naming the project it works in and the files or folders it will change (its claims). Register planned work with it; claims are what keep parallel agents from editing the same files. Adding is free: it does NOT check for conflicts, so any number of open tasks may overlap, and the check happens later in `task_start`. Claims are PATHS relative to the project's root, never patterns or globs: a file (`src/lib.rs`) or a folder (`src`, which covers everything below it, so it overlaps `src/a.rs` but not `srcx`). `/` and `\\` are the same, `.` segments and a trailing slash are ignored, repeats are dropped, and on Windows letter case is ignored. A claim of `*` alone claims the whole project. Absolute paths, drive letters, `..`, empty claims and `*` or `?` inside a path, `:`, and names ending in a dot or space are rejected (exit code 2). Claims are compared as text, lexically: 8.3 short names, junctions and symlinks are NOT resolved, so claim the real path (`src/engine`, not a link or short name that leads there). The project name is matched case-insensitively and stored under the folder's real name. It prints the new task's id, which every other task tool takes. It writes one new record and changes nothing else.",
        params: &[TASK_PROJECT, TASK_TITLE, TASK_CLAIMS],
        outputs: "The new task's id on one line, for example `t-20261117-120000`. Ids are unique even for tasks added in the same second.",
        exit_codes: &[
            E0,
            ExitDoc { code: "2", meaning: "unknown project name (stderr lists the known ones), an empty title, or an invalid claim (stderr says which and why)" },
            ExitDoc { code: "1", meaning: "the task record could not be created or written" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword task add <project> --title <text> --claim <path> [--claim <path> ...]",
        cli_examples: &["task add p --title t --claim src --claim docs"],
        parse: |a| parse_task(a, "add", Some("project"), &[("--title", "title", false), ("--claim", "claims", true)], &["project", "title", "claims"]),
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["task".to_string(), "add".into(), i.str("project").unwrap_or_default().into(), "--title".into(), i.str("title").unwrap_or_default().into()];
            for c in i.list("claims") {
                v.push("--claim".into());
                v.push(c.into());
            }
            v
        },
        handler: |cfg, i| Ok(task::cmd_add(cfg, i.require("project")?, i.require("title")?, &i.list("claims"))),
    },
    Tool {
        name: "task_start",
        summary: "Start an open task, taking its claims, unless a running task overlaps them.",
        description: "Moves an `open` task to `running`, which takes its claims, but only if no running task in the same project has an overlapping claim. The check and the change are one atomic step under the board lock (with `worktree`, the lock covers only marking the task running with `worktree_creating` set; the slow `git worktree add` runs after the lock is released, with the claims already held, so other starts are not blocked, and the task is recorded as having a worktree in a second short locked step) (state/tasks/board.lock), so when several agents start overlapping tasks at the same moment, from any checkout or worktree, exactly one succeeds and the others are refused. A refusal (exit code 3) names every conflicting task with its id, owner and the pair of overlapping claims; wait until that task is done, failed or abandoned and try again, or start a task that does not overlap. Starting runs nothing: a task is a claim on files, and the agent does the work itself. The claims stay held, however long the work takes, until `task_done`, `task_fail` or `task_abandon`. With `worktree` set it ALSO makes the task's own Git worktree (see `worktree_create` for the layout and its refusals) in one operation: if and only if the start succeeded, it runs `git worktree add -b task/<id> <path> <base>` where <base> is the tip of the project's current branch in its main checkout (resolved fresh; the main checkout must be on a branch, not detached), and records `worktree`, `branch` and `base` in the task. If the worktree cannot be made (the project is not a Git repo, the branch or folder already exists, detached HEAD, a git error) the task is rolled back to `open` (owner and start time cleared, claims not held) and the command exits 1 saying so: a task is never left running with a half-made worktree. Only an `open` task can be started (exit code 1 otherwise). If the lock is held by another task command for longer than the task_lock_wait_seconds setting the call exits 4 `board busy` having changed nothing, and can simply be repeated.",
        params: &[TASK_ID, TASK_OWNER, TASK_WORKTREE],
        outputs: "`task <id> is running (owner <owner>)...` on success; with `worktree` also the lines `worktree: <absolute path>`, `branch: task/<id>` and `base: <commit>`. On a refusal, stderr lists each conflicting running task as `task <id> (owner <owner>): your claim <x> overlaps its claim <y>`.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "no task with that id, or the task is not `open` (it is already running, finished or abandoned), or the record could not be read or written, or (with `worktree`) the worktree could not be made, in which case the task was rolled back to `open`" },
            ExitDoc { code: "2", meaning: "the owner is empty or has more than one line" },
            ExitDoc { code: "3", meaning: "refused: a running task in the same project has an overlapping claim; nothing was changed" },
            ExitDoc { code: "4", meaning: "board busy: the board lock could not be taken in time; nothing was changed, repeat the call" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword task start <id> --owner <name> [--worktree]",
        cli_examples: &["task start t-1 --owner coder-1", "task start t-1 --owner coder-1 --worktree"],
        parse: |a| {
            let n = a.iter().filter(|w| **w == "--worktree").count();
            let rest: Vec<&str> = a.iter().copied().filter(|w| *w != "--worktree").collect();
            let i = parse_task(&rest, "start", Some("id"), &[("--owner", "owner", false)], &["id", "owner"])?;
            match n {
                0 => Some(i),
                1 => Some(i.with("worktree", Val::Bool(true))),
                _ => None,
            }
        },
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["task".to_string(), "start".into(), i.str("id").unwrap_or_default().into(), "--owner".into(), i.str("owner").unwrap_or_default().into()];
            if i.bool("worktree") {
                v.push("--worktree".into());
            }
            v
        },
        handler: |cfg, i| Ok(task::cmd_start(cfg, i.require("id")?, i.require("owner")?, i.bool("worktree"))),
    },
    Tool {
        name: "task_done",
        summary: "Mark a running task done and release its claims.",
        description: "Marks a `running` task as `done` and releases its claims, so overlapping tasks can start. Call it when the work is finished (and reviewed or merged, as the project requires). An optional note is stored with the record. DESTRUCTIVE when the task has a worktree (see `worktree_create`): if the worktree is clean (nothing modified, untracked or ignored) and its branch `task/<id>` is merged, `task_done` DELETES the worktree folder and the `task/<id>` branch (the merged work lives on in the project's branch). It removes them ONLY when that is safe: the worktree holds no uncommitted, untracked or ignored files, every commit in its HEAD log is on some branch, AND the branch `task/<id>` is merged into the project's current branch in its main checkout; then it runs `git worktree remove` and `git branch -d` (never forced) and deletes the now-empty folders. If either condition fails, the task is STILL marked done and the exit code is still 0, but a `WARNING:` on stderr says exactly why the worktree and branch were kept and what to do (commit or discard, merge, then `worktree_remove`); nothing is ever deleted that holds unmerged or uncommitted work. Only a running task can be finished (exit code 1 for an open, finished or abandoned task). The record is kept for good. It takes the board lock for a moment (see `task_start` for exit code 4).",
        params: &[TASK_ID, TASK_NOTE],
        outputs: "`task <id> is done; its claims are released`; then, only for a task with a worktree, either `worktree removed: <path>; branch ... deleted` or `worktree kept: <path>` with a `WARNING:` block on stderr listing the reasons and the fix.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "no task with that id, or it is not `running`, or the record could not be written" }, EXIT_BUSY_DOC],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: true,
        idempotent: false,
        cli: "broadsword task done <id> [--note <text>]",
        cli_examples: &["task done t-1", "task done t-1 --note merged"],
        parse: |a| parse_task(a, "done", Some("id"), &[("--note", "note", false)], &["id", "note"]),
        mcp_callable: true,
        to_args: |i| finish_args("done", i),
        handler: |cfg, i| Ok(task::cmd_finish(cfg, i.require("id")?, task::Action::Done, i.str("note"))),
    },
    Tool {
        name: "task_fail",
        summary: "Mark a running task failed and release its claims.",
        description: "Marks a `running` task as `failed` and releases its claims, so overlapping tasks can start. Use it when the work could not be completed; put the reason in the note. Only a running task can be failed (exit code 1 otherwise). The record is kept for good. It takes the board lock for a moment (see `task_start` for exit code 4).",
        params: &[TASK_ID, TASK_NOTE],
        outputs: "`task <id> is failed; its claims are released`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "no task with that id, or it is not `running`, or the record could not be written" }, EXIT_BUSY_DOC],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword task fail <id> [--note <text>]",
        cli_examples: &["task fail t-1", "task fail t-1 --note tests"],
        parse: |a| parse_task(a, "fail", Some("id"), &[("--note", "note", false)], &["id", "note"]),
        mcp_callable: true,
        to_args: |i| finish_args("fail", i),
        handler: |cfg, i| Ok(task::cmd_finish(cfg, i.require("id")?, task::Action::Fail, i.str("note"))),
    },
    Tool {
        name: "task_abandon",
        summary: "Give up on an open or running task and release its claims.",
        description: "Marks a task as `abandoned` from `open` or `running` and releases its claims, so overlapping tasks can start. Use it for work that is cancelled or whose agent is gone. The record is kept. A task that is already done, failed or abandoned cannot be abandoned (exit code 1). This is about the task board only; to give up on an interrupted job (a build or run that was killed) use the `abandon` tool instead. It takes the board lock for a moment (see `task_start` for exit code 4).",
        params: &[TASK_ID],
        outputs: "`task <id> is abandoned; its claims are released`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "no task with that id, or it is already done, failed or abandoned, or the record could not be written" }, EXIT_BUSY_DOC],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword task abandon <id>",
        cli_examples: &["task abandon t-1"],
        parse: |a| parse_task(a, "abandon", Some("id"), &[], &["id"]),
        mcp_callable: true,
        to_args: |i| vec!["task".into(), "abandon".into(), i.str("id").unwrap_or_default().into()],
        handler: |cfg, i| Ok(task::cmd_finish(cfg, i.require("id")?, task::Action::Abandon, None)),
    },
    Tool {
        name: "task_list",
        summary: "List tasks on the board with their status, owner, project, title and claims.",
        description: "Lists the tasks on the task board, oldest first, optionally only those with one status and/or in one project. Use it to see what is open, who is running what and which claims are held (the claims of `running` tasks are the held ones). It only reads and returns immediately.",
        params: &[
            Param { name: "status", ty: Ty::String, required: false, description: "Show only tasks with this status: open, running, done, failed or abandoned. An unknown status fails with exit code 2." },
            Param { name: "project", ty: Ty::String, required: false, description: "Show only tasks of this project name. Not checked against the known projects: an unknown name just matches nothing." },
        ],
        outputs: "One tab-separated line per task: id, status, owner (`-` if none), project, title, claims separated by `, `; or `no tasks` / `no tasks match`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "the board could not be read (a task record is unreadable)" }, ExitDoc { code: "2", meaning: "unknown status" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword task list [--status <s>] [--project <p>]",
        cli_examples: &["task list", "task list --status running --project p"],
        parse: |a| parse_task(a, "list", None, &[("--status", "status", false), ("--project", "project", false)], &["status", "project"]),
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["task".to_string(), "list".into()];
            for (flag, name) in [("--status", "status"), ("--project", "project")] {
                if let Some(x) = i.str(name) {
                    v.push(flag.into());
                    v.push(x.into());
                }
            }
            v
        },
        handler: |cfg, i| Ok(task::cmd_list(cfg, i.str("status"), i.str("project"))),
    },
    Tool {
        name: "task_show",
        summary: "Show one task in full.",
        description: "Shows one task from the task board in full: title, project, status, owner, claims, the times it was created, started and ended, and the note left when it finished. Use it to see why a task ended or who holds a claim. It only reads and returns immediately.",
        params: &[TASK_ID],
        outputs: "Lines of `key: value`: id, title, project, status, owner, claims, created, started, ended (UTC times, `-` when not yet), note.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "no task with that id" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword task show <id>",
        cli_examples: &["task show t-1"],
        parse: |a| parse_task(a, "show", Some("id"), &[], &["id"]),
        mcp_callable: true,
        to_args: |i| vec!["task".into(), "show".into(), i.str("id").unwrap_or_default().into()],
        handler: |cfg, i| Ok(task::cmd_show(cfg, i.require("id")?)),
    },
    Tool {
        name: "task_check",
        summary: "Would a task with these claims conflict with a running task right now? (read-only)",
        description: "Answers, without changing anything, whether a would-be task in a project could start now: whether any of the given claims overlaps a claim of a task that is currently `running` in that project. Use it when planning parallel work, to pick tasks that can run together. It is only a snapshot: another agent may start a task a moment later, so the real decision is made by `task_start`, which checks again atomically. Claims are paths relative to the project root, never patterns, compared lexically (see `task_add` for the rules, including that symlinks, junctions and 8.3 names are not resolved); `*` is the whole project. It only reads and returns immediately.",
        params: &[TASK_PROJECT, TASK_CLAIMS],
        outputs: "`free: ...` when nothing conflicts, or `conflict:` followed by one line per conflicting running task: its id, owner and the pair of overlapping claims.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "free: no running task conflicts" },
            ExitDoc { code: "1", meaning: "the board could not be read (a task record is unreadable), so nothing can be said" },
            ExitDoc { code: "2", meaning: "unknown project name or an invalid claim" },
            ExitDoc { code: "3", meaning: "conflict: at least one running task overlaps (the output names them)" },
        ],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword task check <project> --claim <path> [--claim <path> ...]",
        cli_examples: &["task check p --claim src"],
        parse: |a| parse_task(a, "check", Some("project"), &[("--claim", "claims", true)], &["project", "claims"]),
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["task".to_string(), "check".into(), i.str("project").unwrap_or_default().into()];
            for c in i.list("claims") {
                v.push("--claim".into());
                v.push(c.into());
            }
            v
        },
        handler: |cfg, i| Ok(task::cmd_check(cfg, i.require("project")?, &i.list("claims"))),
    },
    Tool {
        name: "worktree_create",
        summary: "Make the Git worktree and branch for an already-running task.",
        description: "Gives a `running` task its own Git worktree, for a task that was started without one (`task_start` with `worktree` does the same in the start itself). The worktree is a separate folder checked out on a new branch `task/<task id>` in the PROJECT's own Git repo (the Broadsword repo for project `broadsword`, `projects/<project>` for the others), at <root>/<worktrees_dir>/<canonical project>/<task id>, so the agent edits there and never in the main checkout. The branch starts at the commit at the tip of the project's current branch in its main checkout, resolved fresh (`git rev-parse HEAD`); that commit is recorded as `base` together with `worktree` (path relative to root) and `branch` in the task record. It refuses, changing nothing: when the task is not running or already has a worktree; when the project folder is not a Git repo of its own; when the branch `task/<id>` or the folder already exists; and when the main checkout is on a detached HEAD (check out a branch first). It takes the board lock for its short time (see `task_start` for exit code 4). Remove the worktree again with `task_done` (automatic when safe) or `worktree_remove`.",
        params: &[TASK_ID],
        outputs: "`worktree: <absolute path>`, `branch: task/<id>`, `base: <commit>`, one per line.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "no task with that id, or it is not `running`, or it already has a worktree, or the worktree could not be made (not a Git repo, branch or folder exists, detached HEAD, git error); nothing was left behind" },
            EXIT_BUSY_DOC,
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword worktree create <id>",
        cli_examples: &["worktree create t-1"],
        parse: |a| parse_worktree(a, "create", true, None),
        mcp_callable: true,
        to_args: |i| vec!["worktree".into(), "create".into(), i.str("id").unwrap_or_default().into()],
        handler: |cfg, i| Ok(worktree::cmd_create(cfg, i.require("id")?)),
    },
    Tool {
        name: "worktree_remove",
        summary: "Remove one task's worktree, only if it is clean and merged (destructive, never forced).",
        description: "Removes the Git worktree of a task that is no longer running (done, failed or abandoned), but only when that cannot lose work. It is removed only if ALL of these hold: the worktree has no uncommitted, staged or untracked changes (`git status --porcelain --ignored` is empty: IGNORED files count too, because a `.env` or notes are ignored and Broadsword cannot tell them from build output; there is no override, move or delete them yourself); every commit in the worktree's own HEAD log is reachable from some branch; it is still on its branch `task/<id>`; and that branch is merged, meaning its tip is an ancestor of the main checkout's HEAD. Then it runs `git worktree remove` and `git branch -d` (never `--force`, never `-D`, so Git refuses an unmerged branch too) and deletes the now-empty parent folders under worktrees_dir. With `keep_branch` set the merged rule is dropped but the branch is NEVER deleted: use it for an abandoned or failed task whose work was not merged, to free the folder while every commit stays reachable on `task/<id>`; the worktree must still be clean. If the folder was already deleted by hand it just clears Git's record (and deletes the branch only if merged). A locked worktree is refused with advice to unlock it deliberately. It refuses (exit code 3) with the reasons and what to do, changing nothing, when it is not safe; it refuses (exit 1) for a running task, whose worktree is in use. It takes the board lock for a moment (see `task_start` for exit code 4).",
        params: &[TASK_ID, WT_KEEP_BRANCH],
        outputs: "`worktree removed: <path>; branch ... deleted|kept` on success, or `the worktree of task <id> was already removed`. On refusal (exit 3), stderr lists each reason and a `to fix:` line.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "no task with that id, or the task is running, or it has no worktree, or git/the record could not be read or written" },
            ExitDoc { code: "3", meaning: "refused: the worktree has uncommitted or untracked changes, or its branch is not merged (and keep_branch was not set), or git refused; nothing was changed" },
            EXIT_BUSY_DOC,
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: true,
        idempotent: true,
        cli: "broadsword worktree remove <id> [--keep-branch]",
        cli_examples: &["worktree remove t-1", "worktree remove t-1 --keep-branch"],
        parse: |a| parse_worktree(a, "remove", true, Some(("--keep-branch", "keep_branch"))),
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["worktree".to_string(), "remove".into(), i.str("id").unwrap_or_default().into()];
            if i.bool("keep_branch") {
                v.push("--keep-branch".into());
            }
            v
        },
        handler: |cfg, i| Ok(worktree::cmd_remove(cfg, i.require("id")?, i.bool("keep_branch"))),
    },
    Tool {
        name: "worktree_prune",
        summary: "Sweep finished tasks' worktrees: remove the clean, merged ones, list the rest (destructive, never forced).",
        description: "Looks at every task whose status is done, failed or abandoned and whose worktree still exists, and removes each one that is clean and merged, by exactly the safety rules of `worktree_remove` (no uncommitted or untracked changes, branch `task/<id>` merged into the main checkout's HEAD; `git worktree remove` then `git branch -d`, never forced). The rest are listed with the reason and the fix and left untouched; that is not an error. A `running` (or open) task's worktree is NEVER touched. Afterwards it runs `git worktree prune` in every project repo, which clears Git's records of worktree folders that were deleted by hand. Use it as the periodic tidy-up after merges. It takes the board lock for the whole sweep (see `task_start` for exit code 4).",
        params: &[],
        outputs: "One `removed <id>: ...` line per removed worktree, one `kept <id> (<status>): <path>` block with its reasons per kept one, then a summary line `prune: N removed, N kept, N running or open task(s) left alone; ...`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "a task record or a git call failed (the output says which); worktrees that were already removed stay removed" }, EXIT_BUSY_DOC],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: true,
        idempotent: true,
        cli: "broadsword worktree prune",
        cli_examples: &["worktree prune"],
        parse: |a| (a == ["worktree", "prune"]).then(Inputs::default),
        mcp_callable: true,
        to_args: |_| vec!["worktree".into(), "prune".into()],
        handler: |cfg, _| Ok(worktree::cmd_prune(cfg)),
    },
    Tool {
        name: "worktree_list",
        summary: "List the worktrees of tasks, with branch, base, and whether each is clean and merged. (read-only)",
        description: "Lists every task whose worktree has been made and not yet removed, so you can see what exists and what would be safe to remove. For each: the task id, its status, the worktree path relative to root, its branch, the base commit it started from, whether the worktree is `clean`, `dirty` (uncommitted or untracked changes), `missing` (the folder is gone) or `unknown`, and whether the branch is `merged` into the main checkout's HEAD, `unmerged`, `no-branch` or `unknown`. It only reads (it runs `git status` and `git merge-base --is-ancestor`) and returns immediately.",
        params: &[],
        outputs: "One tab-separated line per worktree: task id, status, path, branch, base commit, clean|dirty|missing|unknown, merged|unmerged|no-branch|unknown; or `no worktrees`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "the board could not be read (a task record is unreadable)" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword worktree list",
        cli_examples: &["worktree list"],
        parse: |a| (a == ["worktree", "list"]).then(Inputs::default),
        mcp_callable: true,
        to_args: |_| vec!["worktree".into(), "list".into()],
        handler: |cfg, _| Ok(worktree::cmd_list(cfg)),
    },
    Tool {
        name: "worktree_repair",
        summary: "After moving the project folder, fix Git's stored worktree paths in every project repo.",
        description: "Run this once after the Broadsword folder was moved or copied to another drive or folder. Git records the full path of every worktree (and each worktree records its repo), so after a move they all point at the old location. For the Broadsword repo and every project repo it runs `git worktree repair`, giving it the worktree folders found under <worktrees_dir>/<project>/, which rewrites those links in both directions. A project that is not a Git repo is skipped. It changes only Git's own link files; it never deletes anything and is safe to repeat.",
        params: &[],
        outputs: "One line per project: `<project>: ok (N worktree folder(s) checked)`, `<project>: repaired ...` with what git fixed, or `not a Git repo, skipped`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "git could not repair at least one project (stderr says which)" }],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword worktree repair",
        cli_examples: &["worktree repair"],
        parse: |a| (a == ["worktree", "repair"]).then(Inputs::default),
        mcp_callable: true,
        to_args: |_| vec!["worktree".into(), "repair".into()],
        handler: |cfg, _| Ok(worktree::cmd_repair(cfg)),
    },
];

const TASK_PROJECT: Param = Param {
    name: "project",
    ty: Ty::String,
    required: true,
    description: "Project the task works in: `broadsword` or the name of a folder in the projects folder that contains a Cargo.toml (use the `projects` tool to list them). Matched case-insensitively and stored under the folder's real name; a trailing dot or space is rejected. Claims are relative to this project's root. Anything else fails with exit code 2.",
};
const TASK_TITLE: Param = Param { name: "title", ty: Ty::String, required: true, description: "A short title saying what the task does. Must not be empty; whitespace runs are collapsed to single spaces." };
const TASK_CLAIMS: Param = Param {
    name: "claims",
    ty: Ty::StringArray,
    required: true,
    description: "The paths the task will change, each a file or folder relative to the project root (not a pattern): `src`, `src/lib.rs`, or `*` for the whole project. A folder covers everything below it. `/` and `\\` are alike, `.` segments and a trailing slash are ignored, case is ignored on Windows. Absolute paths, `..`, empty strings, `*` or `?` inside a path, `:`, and names ending in a dot or space are rejected. Compared lexically as text: 8.3 short names, junctions and symlinks are not resolved, so claim the real path. At least one. On the command line each one is its own `--claim <path>`.",
};
const TASK_ID: Param = Param { name: "id", ty: Ty::String, required: true, description: "The exact task id, as printed by `task_add` and shown in the first column of `task_list`." };
const TASK_OWNER: Param = Param { name: "owner", ty: Ty::String, required: true, description: "Free text naming who runs the task, for example `coder-1`. Shown in conflict messages so others know whom to wait for. Must not be empty." };
const TASK_NOTE: Param = Param { name: "note", ty: Ty::String, required: false, description: "A short note stored with the task, for example what was merged or why it failed. Whitespace runs are collapsed. Default: none." };
const TASK_WORKTREE: Param = Param {
    name: "worktree",
    ty: Ty::Boolean,
    required: false,
    description: "Also make the task's own Git worktree and branch `task/<id>` right after the start succeeds (see `worktree_create`). If that fails the task is rolled back to `open`. Default false.",
};
const WT_KEEP_BRANCH: Param = Param {
    name: "keep_branch",
    ty: Ty::Boolean,
    required: false,
    description: "Remove only the worktree folder, require only that it is clean (not that the branch is merged), and NEVER delete the branch `task/<id>`, so all its commits stay reachable. Use it for abandoned or failed tasks whose work was not merged. Default false.",
};
const EXIT_BUSY_DOC: ExitDoc = ExitDoc { code: "4", meaning: "board busy: the board lock could not be taken in time; nothing was changed, repeat the call" };

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
    let rest = match a {
        ["task", s, rest @ ..] if *s == sub => rest,
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
        } else if required_by_shape(sub, name) {
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
fn required_by_shape(sub: &str, name: &str) -> bool {
    matches!((sub, name), ("add", "title" | "claims") | ("start", "owner") | ("check", "claims"))
}

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
    fn to_args_round_trips_through_parse_for_every_tool() {
        let s = |x: &str| Val::Str(x.into());
        let samples: Vec<(&str, Inputs)> = vec![
            ("status", Inputs::default()),
            ("cleanup", Inputs::default()),
            ("watch", Inputs::default()),
            ("projects", Inputs::default()),
            ("recover", Inputs::default()),
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
