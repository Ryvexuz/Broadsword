# Progress

Rewritten at the end of each session. Last updated 2026-11-18, after Stage 3 milestone 3 (reviewer step and merge queue).

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** done, tagged `stage-2-done`.
- **Stage 3 (Autonomous Development):** in progress.

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. Task board | passed, merged at `e82e6bf` | `evidence/stage-3-m1-task-board.md` |
| 2. Worktree per agent | passed, merged at `c64d9d1` | `evidence/stage-3-m2-worktree-per-agent.md` |
| 3. Reviewer step and merge queue | passed, merged at `9ec7442` | `evidence/stage-3-m3-merge-queue.md` |
| 4. Scale-up test (`max_agents` 3, then 4) | not started | |

Between milestones 2 and 3, an extra piece (not numbered): worktree builds and the stale-binary hazard, merged at `4ebba07`. Evidence: `evidence/stage-3-worktree-builds.md`.

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword task`** manages a durable task board in `state/tasks/`, with path claims per project.
  - Commands: `add`, `start --owner [--worktree]`, `done`, `fail`, `abandon`, `list`, `show`, `check`.
  - `start` refuses with exit 3 when a running task's claims overlap.
  - Under gated races of 2, 8 and 16 processes, exactly one wins.
- **`task start --worktree`** gives a task its own Git worktree in `worktrees/<project>/<id>`, on branch `task/<id>`, based on the project's current main.
  - `task done` removes the worktree and branch once they are merged and clean. Ignored files such as `.env` block removal.
  - Commits left behind by a rebase or amend are kept under `refs/broadsword/kept/`.
  - Also available: `worktree create`, `list`, `remove [--keep-branch]`, `prune` and `repair`.
- **Merge queue:** `broadsword queue` with `add`, `list`, `show`, `run [--entry]` and `cancel`. Entries are in `state/queue/`.
  - A `run` checks the preconditions, rebases onto main, runs `check`, then gets an independent review. The reviewer is Haiku for low-risk entries and Opus for high-risk ones, and always a different model family from the coder.
  - Only a PASS merges, with `git merge --ff-only` to the exact reviewed commit. Then it checks main, and runs `task done`.
  - A failed check, a rejected review, a stale branch or a conflict stops the entry before any merge. Exit codes: 0 merged, 3 blocked/conflict/stale, 5 rejected/check failed, 6 merged but main's check failed.
  - Real reviews need `BROADSWORD_REVIEWER_CLI` set to the Claude CLI's install path on this machine, because `claude` is not on PATH. The path is not written into any file.
- **Builds from a worktree:** `--task <id>` on `build`, `test`, `check` and `run` runs the job in that running task's worktree. Each job records its source and task.
  - Each package has a build-source note in `state/build-sources/<package>`. If the folder about to be built differs from the note, or there is no note, Broadsword runs `cargo clean -p <package> --offline` in the build folder, writes the note, then builds.
  - `state/build.lock` is an OS file lock, held for all of a job's cargo steps, tests included. A job waits `build_lock_wait_seconds` (900) for it, then exits 3 "build folder busy".
  - `run` holds the same lock. It deletes the build-source notes of the packages in its folder before and after the command, so a cargo build inside `run` cannot be installed.
  - `install` installs only main's build: the note must be `.`, and the running exe must be exactly `<main root>/build/debug/broadsword.exe`.
  - `clean` refuses if `build_dir` or any of its parents is a symlink or junction.
  - Build, test and check stay `destructiveHint` false.
- **Locks:** the board, `recover`, the build lock and the queue (`queue.lock`, and `entries.lock` briefly) all use OS file locks, which the OS releases when a process dies.
- **Crash sim:** has `--gravity <m/s^2>` (default 9.81, must be finite and above 0). crash-sim main is `9cd162f`, status clean.
- **Server:** `broadsword mcp` runs from `build/bin/`.
- **OpenAI client:** `clients/openai_mcp.py`.
- **Building:** after changing Broadsword's code, run `broadsword build broadsword` from `build/bin/` or a copy (a running `build/debug` exe cannot be cleaned, so it is not built from there). Then run `build/debug/broadsword.exe install`. No plain cargo, and no timestamp touching.
- **Tests:** 343 passed, 0 failed, run through `broadsword test broadsword`. The queue has 40 of them, using a fake reviewer.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **Reviewer benchmark, OpenAI half.** Needs a small chat-completions runner, and Lazlo's models list. "Terra 6.1" is unverified.
- **An `openai` reviewer backend for the queue.** Not built. The model should be chosen from the benchmark results.
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
  - faster ignored-file capture;
  - queue squash of a task's commits into one reviewed commit;
  - `fake_claude` as test-only, not a product `[[bin]]`;
  - queue check from a clean export of the checked commit, not the live worktree.

## Failed

- **The queue's first review failed.** Opus's first pass found three blockers and six minor points: an unchecked commit could merge (B1), `.env` values and env variables reached the reviewer (B2), and the reviewer CLI override could point at the fake reviewer (B3). All were fixed in the fix round, and the second pass passed. Details in `evidence/stage-3-m3-merge-queue.md`.
- **The planted commit reached crash-sim's history.** The done check's planted commit `7ab23fc` was rejected on review and fixed in `9cd162f`. The fast-forward merge kept both, so the history holds a rejected commit. The final tree is correct and tested.
- **Earlier, from the worktree-build work, both fixed and recorded** in `evidence/stage-3-worktree-builds.md`: the first build after the merge failed, because `build/` had no `CACHEDIR.TAG`; and the first done-check run used a marker no test could see.

## Disk

128.1 GB free against the 20 GB floor, measured 2026-11-18 after the merge-queue done check.

## Next step

Stage 3, milestone 4: the scale-up test. Raise `max_agents` to 3, then 4. Done check: a full session at the higher cap, with clean merges and the disk under control.

The lesson focus is to measure collisions, disk use and wasted work at each cap. Everything needed is now in place: task board claims, worktrees, builds through Broadsword and the merge queue.
