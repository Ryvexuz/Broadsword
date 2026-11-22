# Brief: <one-line task title>

<!-- Fill every section; delete none. A coder gets this file and nothing else about the task. Keep the task to one or two files when the code allows it: small exact tasks cost 3-5x fewer tokens (RETRO.md). -->

**Task:** <what to build, in one or two sentences, in the plan's own words>

**Scope limit:** Each task must cover only one milestone or one mechanism. A brief that names two independent mechanisms must be split into separate tasks, even if both mechanisms belong to one milestone. A refactor (moving, splitting or renaming code with no change in behaviour) is always its own task, with before and after numbers (claim waits on the file, review rounds, test time) and reverted if none improves. A feature or fix brief never reshapes code it only passes through: if it needs a refactor first, board that refactor as a separate prerequisite.

**Prerequisites:** <list every task id or commit that must be merged before this task starts; write `none` if there are no prerequisites>

**Claim (the only files you may change):** <path>, <path>
Do not edit any other file. If the task needs one, stop and report it.
Claim only the files you own; a whole-file claim on a hot shared file (see scripts/brief_lint.py HOT) needs a 'Hot file:' line saying why it cannot be split.

**API you may use:** <for example: anything `pub` in src/lib.rs (for example X, Y)>. <!-- A list is either marked "for example" or is the complete rule. -->

**Requirements source:** <plan file path and the task's "Who builds what" row, identified by task id or row label; write `none` only if this task does not come from a plan>
For a task from a plan (`docs/nodes/*.md` or a stage doc), copy the Requirements list verbatim from that plan's "Who builds what" row for this task. Name the plan file and row above.

**Requirements:**
- <behaviour, with exact formats and numbers>
- <the hard cases: what must never happen, edge inputs, failure behaviour> <!-- SPEC_CHECKLIST.md risk table: name each one that applies. -->

**If a check genuinely fails:** do not loosen it. Report the configuration and numbers.

**Build route:** say how long the full suite takes, and that the coder runs it in the foreground and reads the result (never "run it in the background": a subagent cannot wait for its own background job).

**Done check (each line is a command and what it must show):**
1. `broadsword test <project> --task <id>` → `test: ok`, including the new tests <name them if known>
2. `broadsword run <project> --task <id> -- <command>` → <expected output, exit code>
<!-- Write it so it can only be read one way. If a reviewer could read it two ways, say which one is meant. -->
<!-- THE QUEUE RUNS THIS SECTION (crates/broadsword/src/donecheck.rs). On `queue add --brief`, every NUMBERED line here whose FIRST backtick span is exactly `broadsword build <project> --task <id>` or `broadsword run <project> --task <id> -- <command>`, for the entry's own project and task (`<id>`, `<task id>` and `<project>` are filled in), is run by `queue run` in the task's worktree before any review. `broadsword test|check <project> --task <id>` lines are NOT run again: they are shown COVERED by the queue's own check, with its result (pending for a merge-train member, whose train check decides): no shell, done_check_timeout_seconds each, output redacted, the last done_check_tail_lines lines shown to the reviewer and saved in state/queue/<entry>.donecheck. One that fails (exit 1 or a timeout) rejects the entry before a model is asked. Everything else (python, cargo, git, other subcommands or projects, a missing `--task`, shell characters such as `|`, `>`, `&`, `%`, quotes) is shown to the reviewer as NOT RUN (manual): paste its output in the brief yourself. -->
<!-- Keep the done check the LAST bold section, or end it with the next `**Label:**` or `#` heading line: the queue reads up to there. -->

**Measure:** The orchestrator measures first-pass review rate over the next 20 queue entries whose briefs use this template, against the 20 queue entries immediately before this template change. No code is required.
