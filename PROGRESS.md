# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 3 milestone 1.

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** done, tagged `stage-2-done`.
- **Stage 3 (Autonomous Development):** in progress.

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. Task board | passed, merged at `e82e6bf` | `evidence/stage-3-m1-task-board.md` |
| 2. Worktree per agent | not started | |
| 3. Reviewer step and merge queue | not started | |
| 4. Scale-up test (`max_agents` 3, then 4) | not started | |

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword task`** manages a durable task board in `state/tasks/`, with path claims per project.
  - Commands: `add`, `start --owner`, `done`, `fail`, `abandon`, `list`, `show`, `check`.
  - `start` refuses with exit 3 when a running task's claims overlap.
  - It works over MCP as the `task_*` tools.
  - Under gated races of 2, 8 and 16 processes, exactly one wins.
- **Locks:** the board and `recover` both use OS file locks, which the OS releases when a process dies.
- **Server:** `broadsword mcp` (22 tools) runs from `build/bin/`.
- **OpenAI client:** `clients/openai_mcp.py`.
- **Building:** after changing Broadsword's code, update `crates/broadsword/src/tools.rs`'s timestamp if another checkout built into `build/`, then build, then run `build/debug/broadsword.exe install`.
- **Tests:** 234 Rust tests, 43 client tests and 11 crash-sim tests, all passing.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **This session's environment** still has the old `OPENAI_API_KEY`. The Windows key was updated and now matches `.env`, but this session started before that. New sessions are fine; from this one, run the OpenAI client with the variable cleared.
- **Register Broadsword's MCP server** in this project's `.mcp.json`? Needs Lazlo's OK.
- **Proposed `AGENTS.md` change, not applied:** tool descriptions are part of the done check.
- **Not running yet:** `broadsword watch` doesn't start automatically. Phone alerts and `cargo-sweep` need network access and Lazlo's OK.
- **`IDEAS.md` items:**
  - the coder role should state "no silent defaults";
  - MCP minors;
  - the env-versus-`.env` key warning;
  - a larger description experiment;
  - the shared-build-folder stale-binary hazard;
  - worktree builds of Broadsword's own code;
  - the old worktree base commit;
  - pruning recovery refs;
  - faster ignored-file capture.

## Failed

- Nothing is failing.

## Disk

About 126 GB free against the 20 GB floor.

## Next step

Stage 3, milestone 2: worktree per agent. Done check: worktrees are created and removed automatically, and disappear after merging.

This is where several `IDEAS.md` items land:

- Create each worktree from `main`'s current tip. The subagent tool's worktrees started on the Stage 0 commit seven times.
- Link a worktree to its task on the board.
- Build a worktree's own code into the shared build folder without the stale-binary hazard.
- Remove the worktree, and its branch once merged, with no empty folders left behind.

A likely shape is `broadsword worktree create <task id>` and `broadsword worktree remove <task id>` tools, driven by the task board.
