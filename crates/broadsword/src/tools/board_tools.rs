use super::*;

pub(super) const TASK_ADD: Tool = Tool {
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
};

pub(super) const TASK_START: Tool = Tool {
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
};

pub(super) const TASK_DONE: Tool = Tool {
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
};

pub(super) const TASK_FAIL: Tool = Tool {
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
};

pub(super) const TASK_ABANDON: Tool = Tool {
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
};

pub(super) const TASK_LIST: Tool = Tool {
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
};

pub(super) const TASK_SHOW: Tool = Tool {
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
};

pub(super) const TASK_CHECK: Tool = Tool {
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
};

pub(super) const WORKTREE_CREATE: Tool = Tool {
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
};

pub(super) const WORKTREE_REMOVE: Tool = Tool {
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
};

pub(super) const WORKTREE_PRUNE: Tool = Tool {
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
};

pub(super) const WORKTREE_LIST: Tool = Tool {
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
};

pub(super) const WORKTREE_REPAIR: Tool = Tool {
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
