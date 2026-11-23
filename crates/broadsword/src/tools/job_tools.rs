use super::*;

pub(super) const STATUS: Tool = Tool {
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
};

pub(super) const CLEANUP: Tool = Tool {
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
};

pub(super) const WATCH: Tool = Tool {
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
};

pub(super) const RESUME: Tool = Tool {
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
};

pub(super) const PROJECTS: Tool = Tool {
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
};

pub(super) const BUILD: Tool = Tool {
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
};

pub(super) const TEST: Tool = Tool {
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
};

pub(super) const CHECK: Tool = Tool {
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
};

pub(super) const RUN: Tool = Tool {
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
};

pub(super) const RECOVER: Tool = Tool {
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
};

pub(super) const ABANDON: Tool = Tool {
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
};

pub(super) const JOBS: Tool = Tool {
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
};

pub(super) const TOOLS_TOOL: Tool = Tool {
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
};

pub(super) const INSTALL: Tool = Tool {
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
};
