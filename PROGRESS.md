# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 3 milestone 2.

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** done, tagged `stage-2-done`.
- **Stage 3 (Autonomous Development):** in progress.

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. Task board | passed, merged at `e82e6bf` | `evidence/stage-3-m1-task-board.md` |
| 2. Worktree per agent | passed, merged at `c64d9d1` | `evidence/stage-3-m2-worktree-per-agent.md` |
| 3. Reviewer step and merge queue | not started | |
| 4. Scale-up test (`max_agents` 3, then 4) | not started | |

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword task`** manages a durable task board in `state/tasks/`, with path claims per project.
  - Commands: `add`, `start --owner [--worktree]`, `done`, `fail`, `abandon`, `list`, `show`, `check`.
  - `start` refuses with exit 3 when a running task's claims overlap.
  - Under gated races of 2, 8 and 16 processes, exactly one wins.
- **`task start --worktree`** gives a task its own Git worktree in `worktrees/<project>/<id>`, on branch `task/<id>`, based on the project's current main.
  - `task done` removes the worktree and branch once they are merged and clean.
  - Ignored files such as `.env` block removal.
  - Commits left behind by a rebase or amend are kept under `refs/broadsword/kept/`.
  - Also available: `worktree create`, `list`, `remove [--keep-branch]`, `prune` and `repair`.
  - The crash sim's omega comments were fixed this way (crash-sim `8f9b2c6`).
- **Locks:** the board and `recover` both use OS file locks, which the OS releases when a process dies.
- **Server:** `broadsword mcp` (27 tools) runs from `build/bin/`.
- **OpenAI client:** `clients/openai_mcp.py`.
- **Building:** after changing Broadsword's code, update `crates/broadsword/src/tools.rs`'s timestamp if another checkout built into `build/`, then build, then run `build/debug/broadsword.exe install`.
- **Tests:** 268 Rust tests, 43 client tests and 11 crash-sim tests, all passing.

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
  - agents getting worktrees from the board instead of the subagent tool;
  - pruning recovery refs and kept refs;
  - faster ignored-file capture.

## Failed

- Nothing is failing.

## Disk

About 126 GB free against the 20 GB floor.

## Next step

Stage 3, milestone 3: the reviewer step and merge queue. Done check: a planted bug gets caught before merging.

So far the orchestrator has run reviewers by hand and merged by hand. This milestone makes both part of Broadsword:

- a task's branch enters a merge queue;
- each entry gets an independent review, by a different model from the coder's;
- only a PASS merges: rebase onto current main, `broadsword check`, merge, then `task done`, which removes the worktree.

Questions to settle first:

1. **How does Broadsword run a reviewer?** Through the OpenAI client or a headless Claude Code session. Either costs API money per review, so ask Lazlo.
2. **Where are verdicts recorded?**
3. **What is the planted bug?** It needs a task whose branch contains a deliberate, testable bug that the review must catch.
