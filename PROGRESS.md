# Progress

Rewritten at the end of each session. Last updated 2026-11-18, after Stage 3 milestone 4 (scale-up test).

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** done, tagged `stage-2-done`.
- **Stage 3 (Autonomous Development):** done, tagged `stage-3-done`.

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. Task board | passed, merged at `e82e6bf` | `evidence/stage-3-m1-task-board.md` |
| 2. Worktree per agent | passed, merged at `c64d9d1` | `evidence/stage-3-m2-worktree-per-agent.md` |
| 3. Reviewer step and merge queue | passed, merged at `9ec7442` | `evidence/stage-3-m3-merge-queue.md` |
| 4. Scale-up test (`max_agents` 3, then 4) | passed, commits `16c9279` (cap enforced), `4349051` (cap 4), `b578103` (closed-stdout fix) | `evidence/stage-3-m4-scale-up.md` |

Between milestones 2 and 3, an extra piece (not numbered): worktree builds and the stale-binary hazard, merged at `4ebba07`. Evidence: `evidence/stage-3-worktree-builds.md`.

**Stage 3 done check (assessment).** `DEVELOPMENT_PLAN.md` sets a done check for each Stage 3 milestone, and all four are now met by the evidence above. `AGENTS.md` sets a stage-level check: two agents build two separate crash-sim features in parallel, each reviewed by a different model, both merge cleanly, and the reviewer catches a planted bug. The evidence supports most of it:
- the milestone 4 rounds ran 3, then 4 parallel agents, and every one merged cleanly;
- every change was reviewed by a model different from its coder (Sonnet coders, Haiku reviewers);
- the milestone 3 done check caught the planted bug.

The stricter reading, two different reviewer models, was then run too: a damping-stability feature reviewed by Haiku and a `--summary` feature reviewed by Opus, built in parallel, both merged (`0e6805c`, `9c016cc`). See the addendum in `evidence/stage-3-m4-scale-up.md`.

Stage 3 is done, tagged `stage-3-done`.

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword task`** manages a durable task board in `state/tasks/`, with path claims per project.
  - Commands: `add`, `start --owner [--worktree]`, `done`, `fail`, `abandon`, `list`, `show`, `check`.
  - `start` refuses with exit 3 when a running task's claims overlap.
  - Under gated races of 2, 8 and 16 processes, exactly one wins.
- **Agent cap (`max_agents`).** `task start` refuses with exit 3 when `max_agents` tasks are already `running`, across all projects. A refused start changes nothing. The check runs under the board's OS file lock, in the same step as the claim check. The cap is set to 4 in `broadsword.toml`. Lowering it leaves running tasks alone.
  - Tested at caps of 3 and 4 on the real board, with an 8-process race. Exactly 3 win each round, and the losers exit 3.
- **An interrupted `run` blocks only its own folder** (`ba03a84`): another agent's worktree, or main, keeps working. Records without a `source` still block the whole project. A panic inside a job kills its verified child; a build, test or check is then marked `failed`, while a `run` stays interrupted, so `recover` offers the rollback.
- **Secrets stay out of Git** (`f660c76`, audit finding B1): checkpoints never store `.env*` files, and only `run` jobs take a checkpoint. The 23 refs that held `.env` were deleted and the objects pruned (Lazlo approved this on 2026-11-18); no `refs/broadsword/*` object holds a secret now. The key never left this machine. Lazlo will rotate it when convenient.
- **Full audit, 2026-11-18:** `evidence/full-review-2026-11-18.md`. B1 is fixed. The 17 serious and 14 minor findings are still open, and they are fixed through the queue before Stage 4 code.
- **`run` uses the shared build folder** (`d23b969`): every job's child gets `CARGO_TARGET_DIR` at one spawn site.
- **Closed stdout no longer kills Broadsword.** Crate-wide, `println!`, `print!`, `eprintln!` and `eprint!` are shadowed with versions that drop write errors instead of panicking (`b578103`). Before the fix, `| head` made a job panic mid-run and left its record `running`, which blocked new jobs on that project until `recover`.
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
- **Crash sim:** has `--gravity <m/s^2>` (default 9.81, must be finite and above 0). It has 40 tests, a `--summary` option, a damping-stability check in `validate`, all passing, and a `docs/PHYSICS.md` physics model document. crash-sim main is `9c016cc`, status clean, with no worktrees or task branches left.
- **Server:** `broadsword mcp` runs from `build/bin/`.
- **OpenAI client:** `clients/openai_mcp.py`.
- **Building:** after changing Broadsword's code, run `broadsword build broadsword` from `build/bin/` or a copy (a running `build/debug` exe cannot be cleaned, so it is not built from there). Then run `build/debug/broadsword.exe install`. No plain cargo, and no timestamp touching.
- **Tests:** 362 Broadsword tests pass, run through `broadsword test broadsword`. The queue has 40 of them, using a fake reviewer.

## Unfinished

- **Planned process experiments** (run alongside Stage 4; each gets a row in the improvement ledger in `RETRO.md`):
  - **Reviewer canary:** every few merges, queue a known planted bug and record whether the reviewer catches it, to track catch rate per model over time. Once Lazlo's OpenAI models are linked, run the same canaries across Claude and OpenAI models and effort levels, reusing `bench/reviewer/`.
  - **Handoff test:** a fresh agent resumes from `PROGRESS.md` alone; record what it got wrong or had to ask.
- **Next:** the OpenAI half of the reviewer benchmark, when Lazlo sends the models list. Stage 4 starts only after that.
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
  - pruning recovery refs and kept refs (crash-sim now holds 142 `refs/broadsword/*` refs);
  - faster ignored-file capture;
  - queue squash of a task's commits into one reviewed commit;
  - `fake_claude` as test-only, not a product `[[bin]]`;
  - queue check from a clean export of the checked commit, not the live worktree;
  - Stage 3 items from milestone 4: scoping an interrupted job's block to its own worktree, the `lib.rs` comment fix, and the drop guard around job bodies.

## Failed

- **The queue's first review failed.** Opus's first pass found three blockers and six minor points: an unchecked commit could merge (B1), `.env` values and env variables reached the reviewer (B2), and the reviewer CLI override could point at the fake reviewer (B3). All were fixed in the fix round, and the second pass passed. Details in `evidence/stage-3-m3-merge-queue.md`.
- **The planted commit reached crash-sim's history.** The done check's planted commit `7ab23fc` was rejected on review and fixed in `9cd162f`. The fast-forward merge kept both, so the history holds a rejected commit. The final tree is correct and tested.
- **`max_agents` was never enforced.** The setting existed in `broadsword.toml`, but no code read it. Found in milestone 4 before the scale-up test; fixed in `16c9279`.
- **One agent's crashed job blocked every agent on the project.** A coder piped Broadsword's output into `head`, which closed the pipe. The job panicked and stayed `running`, so Broadsword refused all new jobs on that project until `recover`. Fixed at the root in `b578103`. The scale-up test's evidence is in `evidence/stage-3-m4-scale-up.md`.
- **A false rejection from an ambiguous brief.** The queue's reviewer rejected a correct change because the brief's parenthesised API list read as exhaustive. Fixed by rewording the brief, not the code.
- **Earlier, from the worktree-build work, both fixed and recorded** in `evidence/stage-3-worktree-builds.md`: the first build after the merge failed, because `build/` had no `CACHEDIR.TAG`; and the first done-check run used a marker no test could see.

## Disk

127.0 GB free against the 20 GB floor, measured 2026-11-18 after the scale-up test. The shared build folder was 415 MB at that point.

## Next step

The OpenAI half of the reviewer benchmark, when Lazlo sends the models list. Stage 4 (graphics and rendering) starts only after that.
