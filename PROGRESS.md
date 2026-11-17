# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 3 milestone 2 and the worktree-build work.

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

Between milestones 2 and 3, an extra piece (not numbered): worktree builds and the stale-binary hazard, merged at `4ebba07`. Evidence: `evidence/stage-3-worktree-builds.md`.

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
- **Builds from a worktree:** `--task <id>` on `build`, `test`, `check` and `run` runs the job in that running task's worktree. Each job records its source and task.
  - Each package has a build-source note in `state/build-sources/<package>`. If the folder about to be built differs from the note, or there is no note, Broadsword runs `cargo clean -p <package> --offline` in the build folder, writes the note, then builds.
  - `state/build.lock` is an OS file lock, held for all of a job's cargo steps, tests included. A job waits `build_lock_wait_seconds` (900) for it, then exits 3 "build folder busy".
  - `run` holds the same lock. It deletes the build-source notes of the packages in its folder before and after the command, so a cargo build inside `run` cannot be installed.
  - `install` installs only main's build: the note must be `.`, and the running exe must be exactly `<main root>/build/debug/broadsword.exe`.
  - `clean` refuses if `build_dir` or any of its parents is a symlink or junction.
  - Build, test and check stay `destructiveHint` false.
- **Locks:** the board, `recover` and the build lock all use OS file locks, which the OS releases when a process dies.
- **Server:** `broadsword mcp` runs from `build/bin/`.
- **OpenAI client:** `clients/openai_mcp.py`.
- **Building:** after changing Broadsword's code, run `broadsword build broadsword` from `build/bin/` or a copy (a running `build/debug` exe cannot be cleaned, so it is not built from there). Then run `build/debug/broadsword.exe install`. No plain cargo, and no timestamp touching.
- **Tests:** 291 Rust tests, 43 client tests and 11 crash-sim tests, all passing.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **This session's environment** still has the old `OPENAI_API_KEY`. The Windows key was updated and now matches `.env`, but this session started before that. New sessions are fine; from this one, run the OpenAI client with the variable cleared.
- **Register Broadsword's MCP server** in this project's `.mcp.json`? Needs Lazlo's OK.
- **Proposed `AGENTS.md` change, not applied:** tool descriptions are part of the done check.
- **Not running yet:** `broadsword watch` doesn't start automatically. Phone alerts and `cargo-sweep` need network access and Lazlo's OK.
- **`IDEAS.md` items:**
  - MCP minors;
  - the env-versus-`.env` key warning;
  - a larger description experiment;
  - agents getting worktrees from the board instead of the subagent tool;
  - pruning recovery refs and kept refs;
  - faster ignored-file capture.

## Failed

- Nothing is failing now. Two failures from the worktree-build work are fixed and recorded in `evidence/stage-3-worktree-builds.md`:
  - the first build after the merge failed, because `build/` had no `CACHEDIR.TAG`;
  - the first done-check run used a marker that no test could see, so it was rerun with a test that always panics.

## Disk

125.1 GB free against the 20 GB floor, measured 2026-11-17 after the worktree-build merge.

## Next step

Stage 3, milestone 3: the reviewer step and merge queue. Done check: a planted bug gets caught before merging.

So far the orchestrator has run reviewers by hand and merged by hand. This milestone makes both part of Broadsword:

- a task's branch enters a merge queue;
- each entry gets an independent review, by a different model from the coder's;
- only a PASS merges: rebase onto current main, `broadsword check`, merge, then `task done`, which removes the worktree.

Review tiers, as in `SPEC_CHECKLIST.md`: a cheaper reviewer for lower-risk changes and Opus for high-risk ones. The first Haiku review (worktree builds) found one real bug and one false positive, so its findings must be checked against the code before acting.

Questions to settle first:

1. **How does Broadsword run a reviewer?** Through the OpenAI client or a headless Claude Code session. Either costs API money per review, so ask Lazlo.
2. **Where are verdicts recorded?**
3. **What is the planted bug?** It needs a task whose branch contains a deliberate, testable bug that the review must catch.
4. **Which cheap reviewer model?** The reviewer model is to be chosen once Lazlo shares his OpenAI models list. He mentioned a cheap model he called "Terra 6.1"; not yet verified.

**Proposed before milestone 3: a reviewer benchmark.** Replay the five first-pass commits whose real blocking bug is known:
- `a19ee1a`: silent NaN;
- `fd98728`: key leak;
- `3ad7320`: lock with two holders;
- `cadbb6d`: `.env` deleted;
- `36bca91`: `run` bypass, plus one known false positive.

Run each as a fixed diff-only review prompt against every model and effort level, and score bug caught, false positives, tokens and time. The Claude half covers Haiku, Sonnet and Opus at default and high effort, 30 runs, estimated at $5–10 equivalent. The OpenAI half follows once the models list is in. Waiting on Lazlo's go-ahead.
