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
        description: "Runs one disk cleanup pass: deletes files in the scratch folder older than scratch_max_age_hours, cuts every *.log file in scratch to its last log_max_mb, and runs `cargo sweep --time N` (N = sweep_older_than_days) on the shared build folder only if the cargo-sweep program is installed (if it is not, the report starts with `WARNING: cargo-sweep is NOT installed, so the build folder was NOT swept` and the other steps still run, so do not count on old build output being freed; Broadsword never installs it). If free space is still below the disk floor afterwards, it writes the pause file and sends the configured alert, which makes every job refuse to start until the `resume` tool succeeds. Use it when `status` shows low disk. A job that is killed or refused because free space fell below the floor runs this same pass by itself, once, before it pauses, and pauses only if space is still below the floor. It deletes only expired scratch files and, when cargo-sweep is installed, build artifacts unused for sweep_older_than_days days (never project source, state or job records), so it is safe to repeat. It does not touch any project's working tree.",
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
        summary: "Run `cargo build` on a project, with the disk guard (no Git checkpoint).",
        description: concat!("Builds a project with `cargo build --offline` as a recorded job. Before starting it recovers the interrupted earlier jobs of this call's own project and task only (an interrupted job of another project or task is reported on one line and left as it is, for `broadsword recover` or a call for that task), checks the disk guard (it refuses to start if Broadsword is paused or if free space minus job_disk_reserve_gb would be below the floor) and starts. It takes NO Git checkpoint and creates no refs/broadsword/ ref (only `run` does): the job record says `checkpoint = none` with the reason, and an interrupted build is resumed from its step, never rolled back. While it runs, free space is polled; if it falls below the floor the job is killed and Broadsword is paused. It does not edit source files. It writes build output to the shared build folder, a log in scratch, a job record in state, and cargo may update the project's Cargo.lock. It can run for minutes; over MCP the call blocks until it ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code, and the killed job is then left interrupted for `recover` (or the automatic recovery of the next job of the same project and task) to resume. Use it to compile a project; use `check` to build and test in one go. DISK GUARD CLEANUP: a start refused only because free space minus the job reserve is below the floor (free space itself above the floor) may still run the same cleanup pass as `cleanup` (scratch files past scratch_max_age_hours, log caps, cargo-sweep if installed) once, because expired scratch files are disposable by definition; if that frees enough the job goes on, and Broadsword pauses only if free space is below the floor itself.", TASK_AND_CLEAN!()),
        params: &[PROJECT, TASK],
        outputs: "A `job:` line with the job id and log path, then `--- summary ---` and `build: ok (N s)` or `build: FAILED (exit N)`. Cargo's full output is printed as it runs (compiler output on stderr) and also saved to the job log, then the summary follows. A `source:` line follows the `job:` line (the folder built, relative to root, and the task if any) and a `build source:` line lists the packages; a `cleaned ...` line says which packages had their artifacts deleted from the shared build folder first.",
        exit_codes: JOB_EXITS,
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword build <project> [--task <id>]",
        cli_examples: &["build broadsword", "build nope", "build broadsword --task t-1"],
        parse: |a| match a {
            ["build", p] => Some(Inputs::default().with("project", Val::Str((*p).into()))),
            ["build", p, "--task", t] => Some(Inputs::default().with("project", Val::Str((*p).into())).with("task", Val::Str((*t).into()))),
            _ => None,
        },
        mcp_callable: true,
        to_args: |i| job_args("build", i),
        handler: |cfg, i| job_tool(cfg, "build", i),
    },
    Tool {
        name: "test",
        summary: "Run `cargo test` on a project, with the disk guard (no Git checkpoint).",
        description: concat!("Runs a project's tests with `cargo test --offline` as a recorded job, with the same recovery, disk guard (refuses when paused or when free space minus the reserve would be below the floor), low-disk kill and absence of a Git checkpoint (none is taken, as for `build`). It does not run a separate build first beyond what cargo test itself compiles. It does not edit source files; it writes build output, a log and a job record, and cargo may update Cargo.lock. Tests the project runs may of course write their own files. It can run for minutes; over MCP the call blocks until it ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code, and the killed job is then left interrupted for `recover` (or the automatic recovery of the next job of the same project and task) to resume. Use it when only the test result matters; use `check` for build then test. DISK GUARD CLEANUP: a start refused only because free space minus the job reserve is below the floor (free space itself above the floor) may still run the same cleanup pass as `cleanup` (scratch files past scratch_max_age_hours, log caps, cargo-sweep if installed) once, because expired scratch files are disposable by definition; if that frees enough the job goes on, and Broadsword pauses only if free space is below the floor itself.", TASK_AND_CLEAN!()),
        params: &[PROJECT, TASK],
        outputs: "A `job:` line, then `--- summary ---` and `test: ok — N passed, N failed` or `test: FAILED (exit N) — N passed, N failed`. Cargo's full output is printed as it runs and also saved to the job log, then the summary follows. A `source:` line follows the `job:` line (the folder built, relative to root, and the task if any) and a `build source:` line lists the packages; a `cleaned ...` line says which packages had their artifacts deleted from the shared build folder first.",
        exit_codes: JOB_EXITS,
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword test <project> [--task <id>]",
        cli_examples: &["test broadsword", "test nope", "test broadsword --task t-1"],
        parse: |a| match a {
            ["test", p] => Some(Inputs::default().with("project", Val::Str((*p).into()))),
            ["test", p, "--task", t] => Some(Inputs::default().with("project", Val::Str((*p).into())).with("task", Val::Str((*t).into()))),
            _ => None,
        },
        mcp_callable: true,
        to_args: |i| job_args("test", i),
        handler: |cfg, i| job_tool(cfg, "test", i),
    },
    Tool {
        name: "check",
        summary: "Build, then test, a project in one job (stops at the first failure).",
        description: concat!("The one command that answers whether a project works: runs `cargo build --offline` and then `cargo test --offline` as a single recorded job, and never tests a build that failed. It has the same recovery, disk guard and low-disk kill as `build` (and, like it, no Git checkpoint), and the disk guard is re-checked before each step. It does not edit source files; it writes build output, a log and a job record, and cargo may update Cargo.lock. It can run for many minutes; over MCP the call blocks until it ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code, and the killed job is then left interrupted for `recover` (or the automatic recovery of the next job of the same project and task) to resume. Use it as the default way to verify a change. DISK GUARD CLEANUP: a start refused only because free space minus the job reserve is below the floor (free space itself above the floor) may still run the same cleanup pass as `cleanup` (scratch files past scratch_max_age_hours, log caps, cargo-sweep if installed) once, because expired scratch files are disposable by definition; if that frees enough the job goes on, and Broadsword pauses only if free space is below the floor itself.", TASK_AND_CLEAN!()),
        params: &[PROJECT, TASK],
        outputs: "A `job:` line, then `--- summary ---` and one line per step (`build: ok (N s)`, `test: ok — N passed, N failed`, or the failure). Cargo's full output is printed as it runs and also saved to the job log, then the summary follows. A `source:` line follows the `job:` line (the folder built, relative to root, and the task if any) and a `build source:` line lists the packages; a `cleaned ...` line says which packages had their artifacts deleted from the shared build folder first.",
        exit_codes: JOB_EXITS,
        read_only: false,
        long_running: true,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword check <project> [--task <id>]",
        cli_examples: &["check broadsword", "check nope", "check broadsword --task t-1"],
        parse: |a| match a {
            ["check", p] => Some(Inputs::default().with("project", Val::Str((*p).into()))),
            ["check", p, "--task", t] => Some(Inputs::default().with("project", Val::Str((*p).into())).with("task", Val::Str((*t).into()))),
            _ => None,
        },
        mcp_callable: true,
        to_args: |i| job_args("check", i),
        handler: |cfg, i| job_tool(cfg, "check", i),
    },
    Tool {
        name: "run",
        summary: "Run any shell command inside a project, protected by a Git checkpoint.",
        description: concat!("Runs an arbitrary command line in the project's folder through the shell, as a recorded job. Because the command may change files, the project must be a Git repository: Broadsword first saves a checkpoint of the working tree (refs/broadsword/checkpoints/<job id>) and refuses to run without one. SECRET FILES ARE NEVER SAVED (", SECRET_FILES!(), "): such a file is not stored in the checkpoint, and rollback never restores, overwrites or deletes it, so after a rollback it keeps whatever bytes it has then; the job record lists such files by name only (`secret_skipped`). Other git-ignored files are saved (up to checkpoint_ignored_max_mb) and restored. If Broadsword is killed mid-run, `recover <job id>` rolls that one run's files back to the checkpoint (plain `recover` only lists it, and no other job's files are touched), and until then new jobs in the same source folder (the main checkout, or that task's worktree) are refused; jobs in other worktrees of the project are not. A panic inside a `run` job (a Broadsword bug) leaves it interrupted the same way, after killing its command. It has the same disk guard and low-disk kill as `build`, and can run as long as the command does; over MCP the call blocks until the command ends (one call at a time) and is killed after the mcp_call_timeout_seconds setting (1800 s as shipped), returning isError with no exit code. Because the command's own exit code is passed through, exit codes 1-4 are ambiguous on their own: see the exit codes below for how to tell Broadsword's refusals from the command's result. Use it for project-specific commands such as running a simulation; prefer `build`, `test` or `check` for those. WITH `task` (see `build`) the command runs in that task's worktree folder instead of the main checkout, and the checkpoint and any rollback are of that worktree; `project` must be the task's project. SHARED BUILD FOLDER: an arbitrary shell command cannot prove which packages it builds. Every accepted `run` therefore invalidates the shared `broadsword` binary-target note, even when run from another project; install stays refused until a plain main build, test or check cleans and rebuilds it. A command containing `--manifest-path` is accepted only as a literal single cargo command naming the job folder's own Cargo.toml (no expansion, shell operators, wrappers, duplicate flags or other manifests); otherwise setup fails with exit 1 before notes change or the command starts. Successful Cargo metadata supplies package/manifest pairs in the job log. It therefore takes the build lock (state/build.lock, waiting up to build_lock_wait_seconds, then exit 3 `build folder busy`) for the whole command, so no build, test or check job interleaves with it, and BEFORE the command starts it makes the same preparation as `build`, `test` and `check`: every package of the folder whose build-source note names another checkout (or has none) gets `cargo clean -p <package>`, so a `cargo test` inside the command tests THIS folder's code and not the binary another worktree left in the shared build folder (if the folder's packages cannot be listed, for example because its Cargo.toml is broken, the job still runs and says the protection was skipped; a failing `cargo clean` refuses the job with exit 1). Then it records the note `run:<folder>` for each package of the folder it ran in (before the command, so also if Broadsword is killed): another `run` in that same folder needs no clean (so `run -- cargo build --release` followed by `run -- <the built program>` keeps the program), but a job in a different folder cleans it first, and a `run` note is never installable, and the first plain build, test or check in that folder afterwards runs `cargo clean -p` once for each of its packages (the run may have replaced a binary while cargo still thinks it fresh) before it builds, so an installable binary is always one Broadsword built after the run. A `run` whose packages cannot be listed (broken Cargo.toml) still notes every package name it can read (the root's and the workspace members'), and also leaves a marker file for that folder when the list may be incomplete or is empty. `install` refuses while ANY folder marker exists, whichever folder it names; only a plain build, test or check of that same folder removes its marker (after cleaning every package there), so building another project does not lift the refusal. Apart from that `cargo clean -p`, `run` deletes nothing in the build folder; and `install` refuses until a proper `broadsword build` of the main checkout. DISK GUARD CLEANUP: a start refused only because free space minus the job reserve is below the floor (free space itself above the floor) may still run the same cleanup pass as `cleanup` (scratch files past scratch_max_age_hours, log caps, cargo-sweep if installed) once, because expired scratch files are disposable by definition; if that frees enough the job goes on, and Broadsword pauses only if free space is below the floor itself."),
        params: &[
            PROJECT,
            TASK,
            Param {
                name: "command",
                ty: Ty::StringArray,
                required: true,
                description: "The command and its arguments, as separate strings. They are joined with single spaces and run by the shell in the project folder, so quoting inside one string is not preserved. Must not be empty.",
            },
        ],
        outputs: "A `job:` line, the checkpoint ref (and a line naming any secret files left out of it), then `--- summary ---` and `run: ok` or `run: FAILED (exit N)`. The command's own output is printed as it runs and also saved to the job log, then the summary follows. Because the command's output is on stdout too, only the LAST `run:` line after the final `--- summary ---` is Broadsword's verdict.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "the command exited 0" },
            ExitDoc { code: "1", meaning: "the command exited 1 or failed without a usable code, or Broadsword could not set the job up (log or record could not be written)" },
            ExitDoc { code: "2", meaning: "Broadsword: unknown project name, or with `task` no task with that id; or the command itself exited 2" },
            ExitDoc { code: "3", meaning: "Broadsword refused to start (paused or not enough free disk; project is not a Git repository or no checkpoint could be taken (a `run` only); an earlier interrupted run in the same source folder still needs `recover`; with `task`: the task is not `running`, has no finished worktree, belongs to another project, or its folder is not really a git worktree of the project); or the command itself exited 3" },
            ExitDoc { code: "4", meaning: "Broadsword killed the command because free disk fell below the floor, ran one cleanup pass, and is now paused unless that freed enough space; or the command itself exited 4" },
            ExitDoc { code: "N", meaning: "any other number is the command's own exit code. HOW TO TELL: Broadsword's own verdict is the last `run:` line (`run: ok` or `run: FAILED (exit N)`) after the final `--- summary ---` line on stdout; the command may print a fake one itself, so only the last counts. If there is no such line, Broadsword itself stopped the job (the reason is on stderr, starting `broadsword:`) and the code is one of 1-4 as documented for build; only then is `recover` possibly relevant" },
        ],
        read_only: false,
        long_running: true,
        changes_files: true,
        destructive: true,
        idempotent: false,
        cli: "broadsword run <project> [--task <id>] -- <command...>",
        cli_examples: &["run broadsword -- cargo --version", "run nope -- x", "run broadsword --task t-1 -- cargo --version"],
        parse: |a| match a {
            ["run", p, "--", cmd @ ..] if !cmd.is_empty() => {
                Some(Inputs::default().with("project", Val::Str((*p).into())).with("command", Val::List(cmd.iter().map(|s| (*s).into()).collect())))
            }
            ["run", p, "--task", t, "--", cmd @ ..] if !cmd.is_empty() => Some(
                Inputs::default().with("project", Val::Str((*p).into())).with("task", Val::Str((*t).into())).with("command", Val::List(cmd.iter().map(|s| (*s).into()).collect())),
            ),
            _ => None,
        },
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["run".to_string(), i.str("project").unwrap_or_default().into()];
            if let Some(t) = i.str("task") {
                v.push("--task".into());
                v.push(t.into());
            }
            v.push("--".into());
            v.extend(i.list("command").into_iter().map(String::from));
            v
        },
        handler: |cfg, i| {
            let project = i.require("project")?;
            let command = i.list("command");
            if command.is_empty() {
                return Err("missing required input `command`".into());
            }
            Ok(guarded(cfg, project, i.str("task"), || job::cmd_run(cfg, project, i.str("task"), &command)))
        },
    },
    Tool {
        name: "recover",
        summary: "Clean up after a crash: resume interrupted builds/tests; list interrupted runs, or roll back ONE named run.",
        description: concat!("Finds every job whose Broadsword process died while it was running. build, test and check jobs are resumed from the step they were on, at most max_resumes times in a row, and a verified orphaned child process is killed. An interrupted `run` job may have changed files, and an agent may still be working in that folder, so a rollback happens ONLY for the job you name. Called with no `id` and no `all`, it does the automatic part above and then only LISTS each interrupted `run` job (id, project, source folder, start time and the exact command to roll it back); it changes no project file and exits 1 while any is listed. With `id` it rolls back exactly that one interrupted `run` job: the files of its source folder are restored to the Git checkpoint taken before it started, and the partial work is saved under refs/broadsword/ first, so nothing is lost; secret files (", SECRET_FILES!(), ") are not part of any checkpoint and are left exactly as they are, and each one left alone is named in the output. Other interrupted runs are left alone and still listed. It refuses (exit 2, nothing changed) an id that is not an interrupted `run` job, or whose Broadsword process is still alive. With `all` it rolls back EVERY interrupted `run` job in every folder: use it only when the user says so, never while any agent may still be editing a worktree. Every build, test, check and run already does the safe half (resume, never roll back, never list) automatically before it starts, but only for the interrupted jobs of its own project and task (an interrupted job of another project or task is reported on one line and left as it is, for this tool or a call for that task). Rolling back changes files in the affected source folder and may take a while. Jobs still genuinely running are left alone."),
        params: &[
            Param {
                name: "id",
                ty: Ty::String,
                required: false,
                description: concat!("The exact id of one interrupted `run` job to roll back (see the list this tool prints, or the `jobs` tool). Without it nothing is rolled back. Not together with `all`.", NO_DASH!()),
            },
            Param {
                name: "all",
                ty: Ty::Boolean,
                required: false,
                description: "Roll back EVERY interrupted `run` job in every source folder, including worktrees. Default false. Dangerous: use only when the user explicitly asks. Not together with `id`.",
            },
        ],
        outputs: "One block per interrupted job saying what was done (for an interrupted `run` without `id`: its id, source folder, start time and the command to roll it back), or `recover: nothing to do`. Problems and unrolled runs are listed on stderr as NEEDS ATTENTION with the job ids.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "nothing to do, or everything asked for was recovered (the named run was rolled back; with `all`, every interrupted run was)" },
            ExitDoc { code: "1", meaning: "without `id` or `all`: at least one interrupted `run` job is listed and NOT rolled back; or a job needs attention (could not be rolled back or reached max_resumes), a resumed job failed, or another recovery holds the lock" },
            ExitDoc { code: "2", meaning: "refused, nothing changed: `id` is not a job record, not a `run` job, not `running`, or its Broadsword is still alive; or both `id` and `all` were given" },
        ],
        read_only: false,
        long_running: true,
        changes_files: true,
        destructive: true,
        idempotent: false, // `recover <id>` refuses a second call (the run is then rolled back)
        cli: "broadsword recover [--all | <job id>]",
        cli_examples: &["recover", "recover --all", "recover some-id"],
        parse: |a| match a {
            ["recover"] => Some(Inputs::default()),
            ["recover", "--all"] => Some(Inputs::default().with("all", Val::Bool(true))),
            ["recover", "--all", id] if !id.starts_with('-') => Some(Inputs::default().with("id", Val::Str((*id).into())).with("all", Val::Bool(true))), // refused by the handler (exit 2), never run
            ["recover", id] if !id.starts_with('-') => Some(Inputs::default().with("id", Val::Str((*id).into()))),
            _ => None,
        },
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["recover".to_string()];
            if i.bool("all") {
                v.push("--all".into());
            }
            if let Some(id) = i.str("id") {
                v.push(id.into());
            }
            v
        },
        handler: |cfg, i| match (i.str("id"), i.bool("all")) {
            (Some(_), true) => {
                eprintln!("broadsword: give either `id` (roll back one run) or `all`, not both");
                Ok(ExitCode::from(2))
            }
            (Some(id), false) if id.starts_with('-') || crate::record::sanitize(id) != id => {
                eprintln!("broadsword: `{id}` is not a valid job id; nothing was rolled back");
                Ok(ExitCode::from(2))
            }
            (Some(id), false) => Ok(recover::recover_one(cfg, id)),
            (None, true) => Ok(recover::recover(cfg, recover::Mode::All)),
            (None, false) => Ok(recover::recover(cfg, recover::Mode::List)),
        },
    },
    Tool {
        name: "abandon",
        summary: "Give up on one interrupted job without touching any project file.",
        description: "Marks a dead, interrupted job as abandoned so that new jobs in its source folder are no longer refused, without restoring any files. Use it only when rolling back with `recover` is not wanted or not possible, for example when the work done by an interrupted `run` should be kept and inspected by hand. The job's record and every Git ref stay, and the output prints commands to see what was saved. It refuses a job that is still running or already finished. It changes one job record.",
        params: &[Param {
            name: "id",
            ty: Ty::String,
            required: true,
            description: concat!("The exact job id, as shown in the first column of the `jobs` tool's output.", NO_DASH!()),
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
            Param { name: "name", ty: Ty::String, required: false, description: concat!("Show only this tool, in full. Omit to list all tools. An unknown name fails with exit code 2.", NO_DASH!()) },
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
        description: "Copies the broadsword binary that is running this command to <install_dir>/broadsword.exe (install_dir is a setting, build/bin as shipped), so the MCP server and Claude Code's pause hook run a file that rebuilding the project never has to overwrite. The copy is atomic (written under a temporary name, then renamed into place). If the installed copy is currently running, which locks it on Windows, it is renamed to broadsword.old-<stamp>.exe first and deleted by a later install once nothing runs it; leftovers that are no longer in use are deleted each time. Run it after `build broadsword` as `build/debug/broadsword.exe install`; a binary that is already the installed one reports `already installed` and does nothing, so over MCP (where the server normally runs the installed copy) it is only useful when the server was started from another binary. It REFUSES (exit code 1, nothing copied) unless the broadsword binary in the build folder was last built from the main checkout of Broadsword: it reads state/build-sources/broadsword, the note every build, test or check keeps, and refuses when the note names a task's worktree (or any folder but the main checkout) or is missing, because the installed copy runs the pause hook and the MCP server for everyone and must never be a worktree's experiment; the message names the folder and says to run `build broadsword` without `task` first. A binary that already is the installed one is exempt. It does not touch any project file, job record or setting.",
        params: &[],
        outputs: "`installed: <path> (N bytes)`, then a line for each old copy renamed aside, removed or still in use; or `already installed: ...` when nothing was to be done.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "refused because the binary in the build folder was not built from the main checkout (or there is no record of what built it): nothing was copied, build the main checkout first; or the copy or the rename failed (the previous installed binary is left in place), or the install folder could not be created" },
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
        description: "A settings-key claim is `broadsword.toml#<key>` or `crates/broadsword/src/config.rs#<key>`, where key matches `[a-z0-9_]+`; other files or invalid keys are refused with exit code 2. Adds a task to the task board as one durable record (state/tasks/<id>.task) with status `open`, naming the project it works in and the files or folders it will change (its claims). Register planned work with it; claims are what keep parallel agents from taking overlapping work. Adding is free: it does NOT check for conflicts, so any number of open tasks may overlap, and the check happens later in `task_start`. Claims are PATHS relative to the project's root, never patterns or globs: a file (`src/lib.rs`) or a folder (`src`, which covers everything below it, so it overlaps `src/a.rs` but not `srcx`). `/` and `\\` are the same, `.` segments and a trailing slash are ignored, repeats are dropped, and on Windows letter case is ignored. A claim of `*` alone claims the whole project. Absolute paths, drive letters, `..`, empty claims and `*` or `?` inside a path, `:`, and names ending in a dot or space are rejected (exit code 2). Claims are compared as text, lexically: 8.3 short names, junctions and symlinks are NOT resolved, so claim the real path (`src/engine`, not a link or short name that leads there). The project name is matched case-insensitively and stored under the folder's real name. It prints the new task's id, which every other task tool takes. It writes one new record and changes nothing else.",
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
        summary: "Start an open task, taking its claims, unless a running task overlaps them, the max_agents cap (tasks with a working coder; tasks waiting in the merge queue are not counted) is reached or the max_queued cap (open merge-queue entries) is reached.",
        description: "Claims `broadsword.toml#<key>` and `crates/broadsword/src/config.rs#<key>` with keys matching `[a-z0-9_]+` allow different keys of one file to run together, while the same key, a whole-file claim, a containing folder or `*` still conflicts. Moves an `open` task to `running`, which takes its claims, but only if fewer than max_agents tasks (the broadsword.toml setting) are running with a working coder across all projects (a running task whose merge-queue entry is open in the status queued, running or approved is waiting for the queue, not working, so it is NOT counted; a blocked or stale entry needs its coder, so that task IS counted), fewer than max_queued merge-queue entries (the broadsword.toml setting) are open, and no running task in the same project has an overlapping claim. The check and the change are one atomic step under the board lock (with `worktree`, the lock covers only marking the task running with `worktree_creating` set; the slow `git worktree add` runs after the lock is released, with the claims already held, so other starts are not blocked, and the task is recorded as having a worktree in a second short locked step) (state/tasks/board.lock), so when several agents start overlapping tasks at the same moment, from any checkout or worktree, exactly one succeeds and the others are refused, and concurrent starts never push the working count past max_agents. Four refusals, all exit code 3 (nothing is changed): ON MEMORY, when RAM use is at or above the ram_warn_percent setting (the same reading `status` reports), stderr reads `cannot start <id>: RAM is <p>% used (<free> free), at or above ram_warn_percent <w>; start it when memory frees up`, and the task stays `open`; if RAM cannot be read the start is refused the same way, saying so (fail closed); it is checked first, and starting works again once memory frees up; AT THE max_agents CAP, stderr reads `cannot start <id>: N tasks are running with a working coder, the max_agents cap in broadsword.toml is M (running tasks whose merge-queue entry is queued, running or approved are not counted; Q are): <id> (<project>), ...` and then the line `start it after one of them is queued, done, failed or abandoned (`task abandon` frees the slot of a task whose agent died)`, and any working task being queued, finishing or being abandoned frees a slot (a task whose agent died still holds its slot and its claims until `task_abandon`); AT THE max_queued CAP, stderr reads `cannot start <id>: N merge-queue entries are open, the max_queued cap in broadsword.toml is M: <entry> (task <id>, <status>), ...` and then the line `start it after the queue has merged or ended some of them`, followed by one line per orphan entry (open, but its task is not running or not on the board) naming the `broadsword queue cancel <entry>` that clears it; ON AN OVERLAP, stderr names every conflicting task with its id, owner and the pair of overlapping claims, and only that task finishing frees the claim (or start a task that does not overlap). The order is memory, then max_agents, then max_queued, then overlap, so a start refused earlier may still meet a later refusal. Starting runs nothing: a task is a claim on files, and the agent does the work itself. The claims stay held, however long the work takes, until `task_done`, `task_fail` or `task_abandon`. With `worktree` set it ALSO makes the task's own Git worktree (see `worktree_create` for the layout and its refusals) in one operation: if and only if the start succeeded, it runs `git worktree add -b task/<id> <path> <base>` where <base> is the tip of the project's trunk (the one of the local branches `main` and `master` that exists, resolved fresh; whatever the main checkout has checked out does not matter, and if both or neither exist the start is refused rather than guessed), and records `worktree`, `branch` and `base` in the task. If the worktree cannot be made (the project is not a Git repo, the branch or folder already exists, the trunk cannot be told (both or neither of `main` and `master`), a git error) the task is rolled back to `open` (owner and start time cleared, claims not held) and the command exits 1 saying so: a task is never left running with a half-made worktree. Only an `open` task can be started (exit code 1 otherwise). If the lock is held by another task command for longer than the task_lock_wait_seconds setting the call exits 4 `board busy` having changed nothing, and can simply be repeated.",
        params: &[TASK_ID, TASK_OWNER, TASK_WORKTREE],
        outputs: "`task <id> is running (owner <owner>)...` on success; with `worktree` also the lines `worktree: <absolute path>`, `branch: task/<id>` and `base: <commit>`. On an overlap refusal, stderr lists each conflicting running task as `task <id> (owner <owner>): your claim <x> overlaps its claim <y>`; at the max_agents cap it reads `cannot start <id>: N tasks are running with a working coder, the max_agents cap in broadsword.toml is M (...): <id> (<project>), ...` and `start it after one of them is queued, done, failed or abandoned (...)`; at the max_queued cap it reads `cannot start <id>: N merge-queue entries are open, the max_queued cap in broadsword.toml is M: <entry> (task <id>, <status>), ...` and `start it after the queue has merged or ended some of them`.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "no task with that id, or the task is not `open` (it is already running, finished or abandoned), or the record could not be read or written, or (with `worktree`) the worktree could not be made, in which case the task was rolled back to `open`" },
            ExitDoc { code: "2", meaning: "the owner is empty or has more than one line" },
            ExitDoc { code: "3", meaning: "refused: max_agents tasks are already running with a working coder (any project), or max_queued merge-queue entries are open, or a running task in the same project has an overlapping claim; nothing was changed" },
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
        description: "Marks a `running` task as `done` and releases its claims, so overlapping tasks can start. Call it when the work is finished (and reviewed or merged, as the project requires). An optional note is stored with the record. DESTRUCTIVE when the task has a worktree (see `worktree_create`): if the worktree is clean (nothing modified, untracked or ignored) and its branch `task/<id>` is merged, `task_done` DELETES the worktree folder and the `task/<id>` branch (the merged work lives on in the project's branch). It removes them ONLY when that is safe: the worktree holds no uncommitted, untracked or ignored files, every commit in its HEAD log is on some branch, AND the branch `task/<id>` is merged into the project's trunk (the one of `main` and `master` that exists, not whatever the main checkout has checked out); then it runs `git worktree remove` and `git branch -d` (never forced) and deletes the now-empty folders. If either condition fails, the task is STILL marked done and the exit code is still 0, but a `WARNING:` on stderr says exactly why the worktree and branch were kept and what to do (commit or discard, merge, then `worktree_remove`); nothing is ever deleted that holds unmerged or uncommitted work. Only a running task can be finished (exit code 1 for an open, finished or abandoned task). The record is kept for good. It takes the board lock for a moment (see `task_start` for exit code 4).",
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
            Param { name: "status", ty: Ty::String, required: false, description: concat!("Show only tasks with this status: open, running, done, failed or abandoned. An unknown status fails with exit code 2.", NO_DASH!()) },
            Param { name: "project", ty: Ty::String, required: false, description: concat!("Show only tasks of this project name. Not checked against the known projects: an unknown name just matches nothing.", NO_DASH!()) },
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
        description: "Gives a `running` task its own Git worktree, for a task that was started without one (`task_start` with `worktree` does the same in the start itself). The worktree is a separate folder checked out on a new branch `task/<task id>` in the PROJECT's own Git repo (the Broadsword repo for project `broadsword`, `projects/<project>` for the others), at <root>/<worktrees_dir>/<canonical project>/<task id>, so the agent edits there and never in the main checkout. The branch starts at the commit at the tip of the project's trunk, resolved fresh: the one of the local branches `main` and `master` that exists, whatever the main checkout has checked out; that commit is recorded as `base` together with `worktree` (path relative to root) and `branch` in the task record. It refuses, changing nothing: when the task is not running or already has a worktree; when the project folder is not a Git repo of its own; when the branch `task/<id>` or the folder already exists; and when the trunk cannot be told, because the project has both `main` and `master` or neither (merge or rename so exactly one exists). It takes the board lock for its short time (see `task_start` for exit code 4). Remove the worktree again with `task_done` (automatic when safe) or `worktree_remove`.",
        params: &[TASK_ID],
        outputs: "`worktree: <absolute path>`, `branch: task/<id>`, `base: <commit>`, one per line.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "no task with that id, or it is not `running`, or it already has a worktree, or the worktree could not be made (not a Git repo, branch or folder exists, the trunk cannot be told (both or neither of `main` and `master`), git error); nothing was left behind" },
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
        description: "Removes the Git worktree of a task that is no longer running (done, failed or abandoned), but only when that cannot lose work. It is removed only if ALL of these hold: the worktree has no uncommitted, staged or untracked changes (`git status --porcelain --ignored` is empty: IGNORED files count too, because a `.env` or notes are ignored and Broadsword cannot tell them from build output; there is no override, move or delete them yourself); every commit in the worktree's own HEAD log is reachable from some branch; it is still on its branch `task/<id>`; and that branch is merged, meaning its tip is an ancestor of the project's trunk (the one of `main` and `master` that exists, not whatever the main checkout has checked out; a trunk that cannot be told refuses the removal). Then it runs `git worktree remove` and `git branch -d` (never `--force`, never `-D`, so Git refuses an unmerged branch too) and deletes the now-empty parent folders under worktrees_dir. With `keep_branch` set the merged rule is dropped but the branch is NEVER deleted: use it for an abandoned or failed task whose work was not merged, to free the folder while every commit stays reachable on `task/<id>`; the worktree must still be clean. If the folder was already deleted by hand it just clears Git's record (and deletes the branch only if merged). A locked worktree is refused with advice to unlock it deliberately. It refuses (exit code 3) with the reasons and what to do, changing nothing, when it is not safe; it refuses (exit 1) for a running task, whose worktree is in use. It takes the board lock for a moment (see `task_start` for exit code 4).",
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
        description: "Looks at every task whose status is done, failed or abandoned and whose worktree still exists, and removes each one that is clean and merged, by exactly the safety rules of `worktree_remove` (no uncommitted or untracked changes, branch `task/<id>` merged into the project's trunk, the one of `main` and `master` that exists; `git worktree remove` then `git branch -d`, never forced). The rest are listed with the reason and the fix and left untouched; that is not an error. A `running` (or open) task's worktree is NEVER touched. Afterwards it runs `git worktree prune` in every project repo, which clears Git's records of worktree folders that were deleted by hand. Use it as the periodic tidy-up after merges. It takes the board lock for the whole sweep (see `task_start` for exit code 4).",
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
        description: "Lists every task whose worktree has been made and not yet removed, so you can see what exists and what would be safe to remove. For each: the task id, its status, the worktree path relative to root, its branch, the base commit it started from, whether the worktree is `clean`, `dirty` (uncommitted or untracked changes), `missing` (the folder is gone) or `unknown`, and whether the branch is `merged` into the project's trunk (the one of `main` and `master` that exists), `unmerged`, `no-branch` or `unknown`. It only reads (it runs `git status` and `git merge-base --is-ancestor`) and returns immediately.",
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
    Tool {
        name: "queue_add",
        summary: "Put a finished task branch in the merge queue, to be rebased, checked, reviewed by another model and merged.",
        description: concat!("Queues the work of a `running` task for `queue_run`: records one queue entry (state/queue/<entry id>.entry) with the task, project, branch `task/<id>`, the risk (`low` or `high`, which picks the reviewer model: the reviewer_model_low or reviewer_model_high setting, `haiku` and `opus` as shipped), the coder's model name (the reviewer must be a different model) and a brief for the reviewer: the contents of the file given as `brief` (state the task and its done check there; at most reviewer_brief_max_kb KB; the file must be inside the project root after resolving `..` and links, and is refused (exit code 2) if it is outside the root, inside state_dir or build_dir, or is a secret file (", SECRET_FILES!(), "), because the brief goes to a model), or, without one, a brief made from the task's title and claims. It only RECORDS: nothing is rebased, built, reviewed or merged until `queue_run`, and no model is called here. It refuses (exit code 3, nothing queued) unless the task is `running` and has a finished worktree that really is the task's, that worktree is on its branch and CLEAN with ignored files counted (the same rule that guards worktree removal: commit or remove everything first), the branch has at least one commit beyond the task's recorded base, and the task is not already in the queue as a `queued` or `running` entry (a task is queued once at a time; an entry that ended as rejected, check_failed, perf_failed, conflict or merged does not count, so a coder whose work was rejected fixes it, commits, and calls `queue_add` again). It takes a short lock on the queue's entry table (state/queue/entries.lock, waiting at most queue_lock_wait_seconds, then exit code 4); it never waits for a running `queue_run`."),
        params: &[QUEUE_TASK, QUEUE_RISK, QUEUE_CODER, QUEUE_BRIEF],
        outputs: "The new entry's id on the first line, for example `q-20261117-120000`, then one line saying what will happen.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "the queue could not be read or the entry could not be written" },
            ExitDoc { code: "2", meaning: concat!("bad input: risk is not `low` or `high`, the coder is empty, the brief file cannot be read, is not UTF-8 text, is empty, is over reviewer_brief_max_kb KB or is a secret file (", SECRET_FILES!(), "), or there is no task with that id") },
            ExitDoc { code: "3", meaning: "refused, nothing queued: the task is not running or has no usable worktree, the worktree is not on its branch or not clean (ignored files count), the branch has no commit beyond its base, or the task is already queued (the output says which and names the entry)" },
            ExitDoc { code: "4", meaning: "queue busy: the entry-table lock could not be taken in time; repeat the call" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword queue add <task> --risk low|high --coder <model name> [--brief <file>]",
        cli_examples: &["queue add t-1 --risk low --coder sonnet", "queue add t-1 --risk high --coder sonnet --brief brief.md"],
        parse: |a| parse_sub(a, "queue", "add", Some("task"), &[("--risk", "risk", false), ("--coder", "coder", false), ("--brief", "brief", false)], &["task", "risk", "coder", "brief"]),
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["queue".to_string(), "add".into(), i.str("task").unwrap_or_default().into(), "--risk".into(), i.str("risk").unwrap_or_default().into(), "--coder".into(), i.str("coder").unwrap_or_default().into()];
            if let Some(b) = i.str("brief") {
                v.push("--brief".into());
                v.push(b.into());
            }
            v
        },
        handler: |cfg, i| Ok(queue::cmd_add(cfg, i.require("task")?, i.require("risk")?, i.require("coder")?, i.str("brief"))),
    },
    Tool {
        name: "queue_list",
        summary: "List the merge queue: every entry with its status. (read-only)",
        description: "Lists every queue entry, oldest first, with its status: queued (waiting), running (a `queue_run` is on it, or it says `interrupted` when that run died and a later `queue_run` will resume it), approved (merge train only: the review PASSed; it waits for a train, or is a half-finished train merge that `queue_run` resumes first), merged, blocked, conflict, stale, check_failed, perf_failed (the build and tests passed but the perf gate found a bench case slower than its baseline allows, or the baseline or bench could not be used), rejected, merged_check_failed or cancelled; `queue_show` explains one. It only reads entry records and returns immediately; it calls no model.",
        params: &[],
        outputs: "One tab-separated line per entry: id, status, task id, project, risk, coder model, and the reason (or the current step); or `no queue entries`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "an entry record could not be read" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword queue list",
        cli_examples: &["queue list"],
        parse: |a| (a == ["queue", "list"]).then(Inputs::default),
        mcp_callable: true,
        to_args: |_| vec!["queue".into(), "list".into()],
        handler: |cfg, _| Ok(queue::cmd_list(cfg)),
    },
    Tool {
        name: "queue_show",
        summary: "Show one queue entry in full: status, reason, every step, the check summary and the review findings. (read-only)",
        description: "Shows one queue entry: its status and the reason for it, the task, branch, risk, coder and reviewer model, the attempt number, the commits involved (the trunk's tip at the rebase, the pre-rebase tip and the ref that keeps it, the rebased tip, the reviewed commit, the merge commit), the check summary, the reviewer's verdict with its blocking and minor findings, cost and tokens, the path of the review file that holds the reviewer's full raw output, the brief and the time-stamped log of every step. Use it to see why an entry did not merge and what to fix. It only reads and returns immediately.",
        params: &[QUEUE_ENTRY],
        outputs: "Lines of `key: value` (id, status, reason, task, project, branch, risk, coder, reviewer, attempt, step, times, commits, verdict, review file ...), then the check summary, the findings, the brief and the step log as indented blocks.",
        exit_codes: &[E0, ExitDoc { code: "2", meaning: "no queue entry with that exact id" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword queue show <entry>",
        cli_examples: &["queue show q-1"],
        parse: |a| parse_sub(a, "queue", "show", Some("entry"), &[], &["entry"]),
        mcp_callable: true,
        to_args: |i| vec!["queue".into(), "show".into(), i.str("entry").unwrap_or_default().into()],
        handler: |cfg, i| Ok(queue::cmd_show(cfg, i.require("entry")?)),
    },
    Tool {
        name: "queue_run",
        summary: "Process one queue entry (or, with `all`, drain the queue in order, merging approved entries in merge trains): rebase onto the trunk (`main` or `master`, which the project's checkout must have checked out), check, review by another model, fast-forward merge into the trunk, check the trunk, finish the task. (destructive, spends money)",
        description: "ALL: with `all`, every runnable entry is run, in order, one after another under ONE hold of the run lock (state/queue/queue.lock; with queue_per_project \"on\", the project's own state/queue/<project>.queue.lock: see PER-PROJECT LOCK) (so no second runner races it and loses with exit 4): interrupted entries first, then the queued and stale ones oldest first (blocked ones are skipped: they wait on a fix). Each entry is taken at most once per call. An entry that ends blocked, conflict, stale, rejected, check_failed or perf_failed does not stop the rest; an internal error (exit 1) or merged_check_failed (exit 6) STOPS the drain. A summary table follows (entry, task, status, exit, seconds, note), and the exit code is the WORST outcome, worst first: 6, 1, 5, 3, 2, 4, 0. MERGE TRAIN, with `all` and when the merge_train_max setting is above 1 (4 as shipped; 1 = off, every entry is then checked, merged and main-checked on its own exactly as described below): every entry is still reviewed on its own diff, brief and gate reviewer (its shadow too), but it does NOT run its own check (the review prompt says so) and stops as `approved` when its review PASSes, with nothing merged. PARALLEL REVIEW (setting review_parallel; 1 = one entry at a time, as before; ignored with the train off): up to review_parallel entries are taken at once and brought to their verdict together: their steps before the review (preconditions, rebase, done check) run one entry after another in queue order, their reviews (gate, then shadow, per entry, same rules, models, timeouts and call cap) side by side, every GPT call (gate or shadow) one at a time so the daily cap is checked per call as before; each entry's verdict is written only to its own entry; the trains, train checks, fast-forwards and main checks after them stay one at a time, in queue order. A call killed mid-review leaves each of those entries `running` at its step, and the next call resumes each. Approved entries of one project whose task claims do not overlap form a TRAIN of up to merge_train_max entries (a lone one at the end of the drain too; and a SHORT train, even of 1, as soon as no entry of that project is left for the drain to run, or its oldest approved entry has waited merge_train_wait_minutes, so one project's approved entries never wait behind another project's stream): their branches are chained one on top of the other with a rebase (a textual conflict sends that entry back to `queued`; so does a rebase that changed the entry's patches, compared by `git patch-id --stable` with the reviewed commits: it is reviewed again, never merged on the old verdict), ONE check runs on the chain tip, main is fast-forwarded ONCE to it (every entry's commits kept in order, no squash) and checked ONCE, and every entry is `merged`, recording its train id, position and the shared check job. If the combined check fails it is BISECTED (the check of the first i entries, by halves): an entry that fails on its own is `check_failed` with the log of that check (`perf_failed` when it was the perf gate that failed), the entries proven good before it merge as a smaller train and the entries after it start a new round on the new main; if the failing entry passes on its own, the failure only shows combined: the failing entry and every entry before it in the chain are held out and none of them merges (main unchanged), the first of them is `conflict`, the ones chained on it are `blocked` with the left-train reason below. If the main check fails after a train merge the exit is 6, every entry of that merge is `merged_check_failed`, main is NOT reset and the drain stops. A check that cannot run to a verdict leaves the train's entries `approved` (the next call takes them again) and stops the drain, unless the reason is one entry's own worktree: then only that entry ends `blocked`, the rest stay `approved` and the drain goes on. TRAINS FORM ONLY UNDER `--all`; `merge_train_max` sets the most entries one train takes. SHORT-TRAIN RULE: a project with no entry left to run, or whose oldest approved entry has waited merge_train_wait_minutes (20 as shipped), gets a train now, even of 1, so its approved entries never wait behind another project's stream. An approved entry that is run on its own (without `all`) is checked first and then merged as a single entry, with one exception: a HALF-FINISHED TRAIN MERGE, an `approved` entry that a train already took into its merge (its step is merge, main_check or done, or it has a main_sha), whose commit a train may already have fast-forwarded into main. It is RESUMED FIRST, by `queue_run` with or without `all`, before any other entry and at its recorded step: the fast-forward is recorded as done (not repeated), only the check of the merged main and `task done` remain, and it is finished as `merged`; it never joins a new train and is not checked alone again. `all` starts no train while one is being finished, and `queue_cancel` refuses it (\"is a half-finished merge ... Not cancelled; finish it with `broadsword queue run --all` or `broadsword queue run --entry <id>`\": main may already hold its commits). AN ENTRY THAT LEAVES A TRAIN WHILE ITS BRANCH IS STILL CHAINED on commits of other entries of a train that are not on main ends `blocked` with a reason that starts `left a train: rebase your own commits onto main and queue again.` and names `git rebase --onto <main> <base> <branch>`. The summary table gets a `train` column when a train ran. WITHOUT `all`: takes the oldest entry that is interrupted (`running` with no process behind it), else the oldest `queued` or `stale` one (or the one named by `entry`: queued, running-but-interrupted, stale or blocked) and runs it through the pipeline, one entry at a time (an OS file lock, state/queue/queue.lock; a second call waits queue_lock_wait_seconds, then exits 4). PER-PROJECT LOCK (setting queue_per_project, \"off\" as shipped; \"off\" = the one global state/queue/queue.lock above, exactly as before, and `project` then only narrows which entries a run takes): with \"on\", a run holds only its project's own run lock, state/queue/<project>.queue.lock (the same OS file lock, freed by the operating system when its holder ends however it ends, and the same queue_lock_wait_seconds), so runs of DIFFERENT projects (`queue run --all --project <p>`) go at the same time, while a second run of the SAME project exits 4 with a `queue busy:` message that names the project (\"another `queue run` of project `<p>`\") and the holder's pid when it can be read; `project` (or `entry`, whose own project is then used) is REQUIRED, without either the call exits 2 and runs nothing, because a run of no named project cannot tell another project's live entries from interrupted ones. The entries lock (state/queue/entries.lock, milliseconds) stays ONE for all projects, and merge trains stay within one project. Do not switch the setting while a `queue run` is going: a run started under the other value does not see its lock. STEPS, each recorded in the entry before it is relied on: (1) preconditions: the task is still running, the project's trunk (the one of the local branches `main` and `master` that exists; both or neither is an error) can be told and is what the main checkout of the project has checked out (a feature branch or a detached HEAD there ends the entry `blocked`, and the checkout is not touched, moved or merged into), that checkout is clean (`git status --porcelain`), the task's worktree is clean (uncommitted and untracked changes; ignored files such as an ignored .env do not count, they only make `task done` keep the worktree), the reviewer model for the risk differs from the entry's coder model, and the branch has a commit main lacks; if one fails the entry is `blocked` and nothing was changed. (2) rebase: `git rebase <trunk tip>` (the tip of `refs/heads/<trunk>`, never HEAD) inside the task's worktree, skipped if the branch already contains that tip; the pre-rebase tip is first saved in the entry and, create-only, under refs/broadsword/queue/<entry>/pre-rebase; on a conflict the rebase is ABORTED, the branch tip is verified to be exactly what it was, and the entry is `conflict` naming the files. (3) check: `check <project> --task <id>` (the same job as the `check` tool: disk guard, build lock, notes, log); if it fails the entry is `check_failed` and NO model is asked. PERF GATE (setting perf_gate, \"on\" as shipped; part of every check, the single entry's and every train check, bisect and own check alike): once the build and tests pass, the entry's own project is benched on the same commit when it has a baseline state/perf/<project>-baseline.json (a project without one is skipped and the skip is logged; no other project is ever benched): `cargo run --release --offline --example bench -- --runs <runs_per_case from the baseline>` runs perf_runs times (5 as shipped) as `run` jobs in the task's worktree, each holding the build lock; a case whose median of the runs' medians is slower than the baseline by more than that case's own threshold_pct (from the baseline file) makes the entry `perf_failed` (exit 5, NO model is asked), with each case's baseline, now and change in % in the reason, and the output says whether the baseline is LOADED. FAIL CLOSED: a baseline file that exists but cannot be read or lacks a needed field, a bench that exits non-zero, and output with a missing or invalid case are `perf_failed` too, never a pass; a bench job that never ran (refused) gives no verdict: `blocked`. With a merge train the gate runs ONCE on the chain tip, with its check, and a perf failure is bisected exactly like a failed check. (4) review: the reviewer model (reviewer_model_low or reviewer_model_high by risk, through the reviewer_backend setting, `claude` as shipped = the Claude Code CLI run headless, one turn, no tools, in a folder under scratch with no CLAUDE.md, with every environment variable named *_API_KEY, *_KEY, *_TOKEN, *_SECRET, *_PAT, *_PASSWORD, *_PASS, *_CREDENTIAL* or *_AUTH* removed, and every one whose value equals or contains ANY value of the root's .env, whatever its name) gets a fixed reviewer prompt, the brief, the check summary and the end of its output, and the whole diff <trunk tip at the rebase>...the checked commit (never cut: a diff over reviewer_diff_max_kb makes the entry `blocked`); EVERY value of the root's .env (redact_min_chars characters or longer, 6 as shipped; shorter ones are only kept out of the environment) and every secret-named variable's value are replaced by [REDACTED] first, in the prompt and in everything stored in the entry and review file. Limits: a secret split across two diff lines is not redacted, and a .env value that is also ordinary code (REGION=return) is blanked everywhere, hiding that code from the reviewer, so keep .env values distinctive. The check, review and merge are bound to ONE commit: the one the check passed on; a branch that moved after the check makes the entry `stale` (\"branch changed after check\") and nothing is reviewed or merged. THIS CALLS A MODEL: it costs money (about $0.006 for haiku, $0.18 for opus per the benchmark) and uses the network; the CLI program is the reviewer_cli setting, or the env var BROADSWORD_REVIEWER_CLI (a machine-specific path); the RESOLVED PATH is printed and recorded in the entry and the review file every time, the override must point OUTSIDE the project root (like the real Claude install) or inside the root's scratch_dir (a TEST HOOK: it prints `TEST HOOK ACTIVE: reviewer CLI is <path>` and the review is marked test_hook: true), and any other path inside the root, build/ included, is refused; the answer must be the CLI's JSON and its modelUsage must name the requested model family (haiku, sonnet, opus), else it is a FAIL; at most reviewer_max_calls_per_entry calls are made per entry (then `blocked`, \"review call cap reached\"); a call longer than reviewer_timeout_seconds is killed and the entry is `blocked` (no verdict). The answer must follow `VERDICT: PASS|FAIL`, `BLOCKING:` lines and `MINOR:` lines; an answer that does not is a FAIL, and a PASS needs an explicit `BLOCKING: none` and no blocking finding (an empty `BLOCKING:` header followed by findings is a FAIL). The full raw output, cost and tokens are saved in state/queue/<entry>.review. A FAIL makes the entry `rejected`: the task stays running and its worktree is kept so the coder can fix, commit and `queue_add` again. (5) merge, on PASS only (a review made by a TEST HOOK reviewer never merges into a project whose folder is outside scratch_dir: the entry is `blocked`; the reviewer CLI's resolved path is printed and logged at the merge; LIMIT: an override that points outside the root is trusted as a real reviewer, because nothing can prove a program there is not a stand-in; a resumed entry whose recorded verdict is not PASS ends `rejected`, and one whose PASS does not name the checked commit ends `stale`): `git merge --ff-only` of the exact reviewed commit in the main checkout, and only while that checkout has the trunk checked out (never a merge commit, never into another branch); if the trunk or the branch moved since the rebase the entry is `stale` and nothing is merged (call again to rebase again). (6) `check <project>` in the main checkout; it is REFUSED (the entry is `merged_check_failed`, naming why) unless that checkout still has the trunk checked out and the trunk holds the merged commit, and if it fails, which should not happen, the trunk is NOT reset: the entry is `merged_check_failed` with a loud message naming the commit, and a human decides. (7) `task done`, which removes the worktree and branch under its usual safety rules; the entry is `merged` with main's commit. NEVER uses --force, -D, reset --hard or push. NOT IDEMPOTENT, DESTRUCTIVE (it rewrites the task branch, moves main's branch forward and removes the worktree), and LONG-RUNNING (a check plus a review take minutes; over MCP the call blocks and is killed after the mcp_call_timeout_seconds setting, 1800 s as shipped, leaving the entry `running` for the next call to resume). CRASH AND RESUME: every step is recorded, so if the process dies the entry stays `running`; the next `queue_run` resumes it from the recorded step (a half-done rebase is aborted first, a recorded verdict is not asked for again, a fast-forward that already happened is recorded, not repeated). The task-board lock is never held during any of this; `task done` takes it for a moment. Every outcome except `merged` exits non-zero with a one-line reason and the entry id on stderr. SHADOW REVIEW (when `shadow_reviewer` is `openai`; `off` disables it): after the gating review is recorded, the SAME prompt goes once to a second model (`shadow_reviewer_model`, run as `python -I -B clients/openai_review.py`, key from the root .env, `shadow_reviewer_timeout_seconds`). Its answer is only RECORDED in the entry (`shadow_verdict` PASS, FAIL, ERROR or SKIPPED, `shadow_blocking`, `shadow_model_used`, `shadow_cost_usd`, `shadow_tokens`, `shadow_secs`) and in state/queue/<id>.shadow.review; it NEVER changes the outcome: the gating review alone decides the merge, and a shadow timeout or error is recorded as ERROR and the merge goes on. SKIPPED when the coder is a GPT model. The rule that no reviewer program may live inside the project root does not apply to the shadow, because it never gates a merge.",
        params: &[QUEUE_RUN_ENTRY, QUEUE_RUN_ALL, QUEUE_RUN_PROJECT],
        outputs: "Progress lines `queue entry <id>: step <name>`, the check job's own output, then one final line: `queue entry <id>: merged: ...` on stdout, or `broadsword: queue entry <id> is <status>: <reason>` on stderr (`broadsword: LOUD: ...` for merged_check_failed). The reviewer's full answer is in state/queue/<id>.review (see `queue_show`). `the queue is empty` and exit 0 when there is nothing to run. With `all`: each entry's own lines, `merge train <id>: ...` lines for every train (its size, the chain, the check, a bisect with its extra checks and seconds), then `--- queue run --all: N entries in S s ---`, a tab-separated table `entry task status exit seconds note` (plus `train`, as `<train id>#<position>`, when a train ran), and `worst exit: N`; a stopped drain says why on stderr.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "merged (the entry is `merged`), or the queue is empty" },
            ExitDoc { code: "1", meaning: "internal error (git or a record could not be read or written, a verification failed): the entry stays `running` at its step and the next call resumes it; or the entry record could not be written" },
            ExitDoc { code: "2", meaning: "bad input: no entry with that id, or the entry is finished (merged, rejected, check_failed, perf_failed, conflict, merged_check_failed, cancelled) and cannot be run; or `entry` was given with `all`; or `project` is not a project, or `entry` is not of that project; or queue_per_project is \"on\" and neither `project` nor `entry` was given; nothing was run" },
            ExitDoc { code: "3", meaning: "not merged, retryable: the entry is `blocked` (a precondition failed, the reviewer model equals the coder, the diff is too big, the check could not run, the reviewer could not be used or timed out), `conflict` (the rebase conflicted and was aborted) or `stale` (main or the branch moved); the reason names the fix" },
            ExitDoc { code: "4", meaning: "queue busy: another `queue_run` holds the run lock (state/queue/queue.lock; with queue_per_project \"on\", that project's state/queue/<project>.queue.lock, and the message names the project) for longer than queue_lock_wait_seconds; nothing was changed" },
            ExitDoc { code: "5", meaning: "not merged, the code needs changes: the entry is `rejected` (the reviewer said FAIL, or gave no usable verdict) or `check_failed` (build or tests failed on the rebased branch) or `perf_failed` (the perf gate failed)" },
            ExitDoc { code: "6", meaning: "LOUD: the commit IS on main but `check` failed there (`merged_check_failed`); main was not reset, a human decides" },
        ],
        read_only: false,
        long_running: true,
        changes_files: true,
        destructive: true,
        idempotent: false,
        cli: "broadsword queue run [--entry <id> | --all] [--project <p>]",
        cli_examples: &["queue run", "queue run --entry q-1", "queue run --all", "queue run --all --project crash-sim", "queue run --project crash-sim"],
        parse: |a| {
            // `--entry <id>` once, `--all` once, `--project <p>` once; inputs in parameter order (entry, all, project). The handler
            // refuses entry with all, so every combination still round-trips.
            let rest = match a {
                ["queue", "run", rest @ ..] => rest,
                _ => return None,
            };
            let (mut entry, mut all, mut project): (Option<&str>, bool, Option<&str>) = (None, false, None);
            let mut it = rest.iter().copied();
            while let Some(w) = it.next() {
                match w {
                    "--entry" if entry.is_none() => entry = Some(it.next()?),
                    "--all" if !all => all = true,
                    "--project" if project.is_none() => project = Some(it.next()?),
                    _ => return None,
                }
            }
            let mut i = Inputs::default();
            if let Some(e) = entry {
                i = i.with("entry", Val::Str(e.into()));
            }
            if all {
                i = i.with("all", Val::Bool(true));
            }
            if let Some(p) = project {
                i = i.with("project", Val::Str(p.into()));
            }
            Some(i)
        },
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["queue".to_string(), "run".into()];
            if let Some(e) = i.str("entry") {
                v.push("--entry".into());
                v.push(e.into());
            }
            if i.bool("all") {
                v.push("--all".into());
            }
            if let Some(p) = i.str("project") {
                v.push("--project".into());
                v.push(p.into());
            }
            v
        },
        handler: |cfg, i| match (i.str("entry"), i.bool("all")) {
            (Some(_), true) => {
                eprintln!("broadsword: give `entry` (run that one entry) or `all` (drain the queue), not both; nothing was run");
                Ok(ExitCode::from(queue::EXIT_BAD_INPUT))
            }
            (_, true) => Ok(queue::cmd_run_all(cfg, i.str("project"))),
            (e, false) => Ok(queue::cmd_run(cfg, e, i.str("project"))),
        },
    },
    Tool {
        name: "queue_cancel",
        summary: "Drop an entry from the merge queue; nothing else is touched.",
        description: "Marks a `queued`, `blocked`, `stale` or interrupted `running` entry as `cancelled`, so `queue_run` no longer picks it. The task, its worktree and branch, and main are not touched, with one exception: for an interrupted run that died in the middle of its rebase, the half-done rebase left in the worktree is aborted (and the branch verified to be back at its pre-rebase tip), because a worktree stuck in a rebase is unusable. An interrupted entry that already reached the merge step is refused: main may already hold the commit, so resume it with `queue_run` instead. An entry that is running right now in another process cannot be cancelled (the call waits for the run lock that guards the entry, state/queue/queue.lock, or with the queue_per_project setting \"on\" the entry's project's state/queue/<project>.queue.lock, then exits 4). An entry that is already cancelled succeeds without change; one that is finished (merged, rejected, check_failed, perf_failed, conflict, merged_check_failed) is refused. The record is kept for good.",
        params: &[QUEUE_ENTRY],
        outputs: "`queue entry <id> is cancelled; nothing else was touched` (plus a note when a leftover rebase was aborted), or `queue entry <id> was already cancelled`.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "the entry record could not be written, or a leftover rebase could not be aborted or did not restore the tip, or a running entry's project is not a plain name so no run lock can guard it (the entry was not cancelled)" },
            ExitDoc { code: "2", meaning: "no queue entry with that exact id" },
            ExitDoc { code: "3", meaning: "not cancelled: the entry is finished, or was interrupted at or after the merge step" },
            ExitDoc { code: "4", meaning: "queue busy: a lock could not be taken in time (for a running entry: its run lock, which names the project when queue_per_project is \"on\"); repeat the call" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword queue cancel <entry>",
        cli_examples: &["queue cancel q-1"],
        parse: |a| parse_sub(a, "queue", "cancel", Some("entry"), &[], &["entry"]),
        mcp_callable: true,
        to_args: |i| vec!["queue".into(), "cancel".into(), i.str("entry").unwrap_or_default().into()],
        handler: |cfg, i| Ok(queue::cmd_cancel(cfg, i.require("entry")?)),
    },
];

const QUEUE_TASK: Param = Param { name: "task", ty: Ty::String, required: true, description: concat!("The id of the task whose branch `task/<id>` is queued (see `task_list`). It must be `running` with a clean worktree and at least one commit beyond its base.", NO_DASH!()) };
const QUEUE_RISK: Param = Param {
    name: "risk",
    ty: Ty::String,
    required: true,
    description: concat!("`low` or `high`: picks the reviewer model (the reviewer_model_low or reviewer_model_high setting). Use `high` for deleting or rewriting files, locks, secrets, or anything that runs before or instead of a safety check; `low` for the rest. Anything else fails with exit code 2.", NO_DASH!()),
};
const QUEUE_CODER: Param = Param { name: "coder", ty: Ty::String, required: true, description: concat!("The model name that wrote the code, for example `sonnet`. The reviewer model must differ from it (compared case-insensitively), or `queue_run` blocks the entry.", NO_DASH!()) };
const QUEUE_BRIEF: Param = Param {
    name: "brief",
    ty: Ty::String,
    required: false,
    description: concat!("Path of a UTF-8 text file with the brief the reviewer gets: what the task is and its done check. At most reviewer_brief_max_kb KB; a secret file (", SECRET_FILES!(), ") is refused. Default: a brief made from the task's title and claims.", NO_DASH!()),
};
const QUEUE_ENTRY: Param = Param { name: "entry", ty: Ty::String, required: true, description: concat!("The exact queue entry id, as printed by `queue_add` and shown in the first column of `queue_list`.", NO_DASH!()) };
const QUEUE_RUN_ALL: Param = Param {
    name: "all",
    ty: Ty::Boolean,
    required: false,
    description: "Drain the queue: run every runnable entry in order (interrupted first, then queued and stale ones oldest first) under one hold of the run lock (with queue_per_project \"on\": the project's own, and `project` is required), each at most once, then print a summary table; the exit code is the worst outcome. With merge_train_max above 1 (4 as shipped), approved entries with non-overlapping claims are merged together by ONE check of their chained tip, one fast-forward and one main check (bisected when the check fails; see the main description). A project with no entry left to run, or whose oldest approved entry has waited merge_train_wait_minutes, gets a train now, even of 1. Not together with `entry`. Default false.",
};

const QUEUE_RUN_PROJECT: Param = Param {
    name: "project",
    ty: Ty::String,
    required: false,
    description: concat!("Take only this project's entries (with `all`: drain only this project). It must be a project (see the `projects` tool), else exit code 2 and nothing runs; with `entry`, that entry must be of this project (else exit code 2). When the queue_per_project setting is \"on\" (\"off\" as shipped), every project has its own run lock, state/queue/<project>.queue.lock, so runs of different projects go at the same time, and `project` (or `entry`) is REQUIRED: without either, exit code 2 and nothing runs. When it is \"off\", every run holds the one state/queue/queue.lock as before and `project` only narrows which entries the run takes.", NO_DASH!()),
};

const QUEUE_RUN_ENTRY: Param = Param {
    name: "entry",
    ty: Ty::String,
    required: false,
    description: concat!("Run this entry (it must be queued, interrupted, stale or blocked) instead of the oldest one. Omit it to take the oldest interrupted entry, else the oldest queued or stale one. An unknown id or a finished entry fails with exit code 2.", NO_DASH!()),
};

const TASK_PROJECT: Param = Param {
    name: "project",
    ty: Ty::String,
    required: true,
    description: concat!("Project the task works in: `broadsword` or the name of a folder in the projects folder that contains a Cargo.toml (use the `projects` tool to list them). Matched case-insensitively and stored under the folder's real name; a trailing dot or space is rejected. Claims are relative to this project's root. Anything else fails with exit code 2.", NO_DASH!()),
};
const TASK_TITLE: Param = Param { name: "title", ty: Ty::String, required: true, description: concat!("A short title saying what the task does. Must not be empty; whitespace runs are collapsed to single spaces.", " FREE TEXT: over MCP a value that starts with `-` and a value with line breaks are both accepted (the command line gets it as the single argument after `--title`; whitespace runs, line breaks included, are collapsed). A NUL is refused.") };
const TASK_CLAIMS: Param = Param {
    name: "claims",
    ty: Ty::StringArray,
    required: true,
    description: "The paths the task will change, each a file or folder relative to the project root (not a pattern): `src`, `src/lib.rs`, or `*` for the whole project. A folder covers everything below it. `/` and `\\` are alike, `.` segments and a trailing slash are ignored, case is ignored on Windows. Absolute paths, `..`, empty strings, `*` or `?` inside a path, `:`, and names ending in a dot or space are rejected. Compared lexically as text: 8.3 short names, junctions and symlinks are not resolved, so claim the real path. At least one. On the command line each one is its own `--claim <path>`.",
};
const TASK_ID: Param = Param { name: "id", ty: Ty::String, required: true, description: concat!("The exact task id, as printed by `task_add` and shown in the first column of `task_list`.", NO_DASH!()) };
const TASK_OWNER: Param = Param { name: "owner", ty: Ty::String, required: true, description: concat!("Free text naming who runs the task, for example `coder-1`. Shown in conflict messages so others know whom to wait for. Must not be empty.", NO_DASH!()) };
const TASK_NOTE: Param = Param { name: "note", ty: Ty::String, required: false, description: concat!("A short note stored with the task, for example what was merged or why it failed. Whitespace runs are collapsed. Default: none.", " FREE TEXT: over MCP a value that starts with `-` and a value with line breaks are both accepted (the command line gets it as the single argument after `--note`; whitespace runs, line breaks included, are collapsed). A NUL is refused.") };
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
