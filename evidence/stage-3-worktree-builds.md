# Worktree builds and the stale-binary hazard — done check

**Check:** a task's worktree is built and tested through Broadsword, and building main afterwards gives main's own binaries.

**Result: PASS**, 2026-11-17, on `main` at merge commit `4ebba07`.

This was an extra piece between Stage 3 milestones 2 and 3, not a numbered milestone. It makes the AGENTS.md rule "Build through Broadsword" work for a task's worktree, and closes the shared-build-folder stale-binary hazard found in the Stage 2 close-out. The hazard item was removed from `IDEAS.md` by this work.

Related commits: coder `36bca91` (first pass), fresh coder `9a05924` (fix round), process changes `13e8215`.

## On the real crash sim

Task `t-20261117-234245`, worktree `worktrees/crash-sim/t-20261117-234245`.

### First attempt: marker not seen by any test

Changing `GRAVITY` from 9.81 to 4.905 in the worktree did NOT fail any test. The free-fall test computes its expected value from the same `GRAVITY` constant. The marker was the orchestrator's mistake, not a Broadsword bug. Cleaning and source tracking still worked in that run, but this run does not count as the check.

### Rerun: a marker test that always panics

The marker was a test that always panics, existing only in the worktree.

```text
=== check crash-sim --task t-20261117-234245 ===
source: worktrees/crash-sim/t-20261117-234245
cleaned `crash-sim` ... (it was last built from projects/crash-sim)
build: ok
test: FAILED (exit 101) — 0 passed, 1 failed
"WORKTREE MARKER" in output
exit 1

=== check crash-sim (main) ===
source: projects/crash-sim
cleaned `crash-sim` ... (it was last built from worktrees/crash-sim/t-20261117-234245)
build: ok
test: ok — 11 passed, 0 failed
marker lines: 0
exit 0
```

The `...` marks text cut from the excerpt.

After the run, the worktree edit was reverted, and `task done` removed the worktree and the branch. crash-sim is clean, still at `8f9b2c6`.

## Problems met during the merge

- **The first `broadsword build broadsword` after the merge failed.** Cargo 1.100 refused `cargo clean -p` because `build/` had no `CACHEDIR.TAG`. The folder was created in Stage 0 with a `.gitkeep`, before cargo ever ran, so cargo never wrote the tag. Every test passed, because the tests use fresh folders that cargo creates itself.
  - Broadsword failed safely: exit 1, nothing built, no note written, `install` refused.
  - Fix: the orchestrator added cargo's standard `build/CACHEDIR.TAG` and un-ignored it in `.gitignore`, so fresh clones have it. The bootstrap then ran: build from a temporary copy of the binary (a running exe cannot be cleaned), write the note `.`, then `install`, which succeeded.
  - Lesson: tests in fresh throwaway folders cannot catch problems that come from the real environment's history. The real done check found it.
- **Leftover junctions.** Removing the coder's worktree left two dangling junctions from the junction tests, under `.claude/worktrees/agent-afb…/scratch/bs-buildsrc-junction-*/b/link`. The orchestrator removed them non-recursively, then removed the empty folders. The junction tests do not remove their own links. Recorded in `IDEAS.md`.
- **The coder's worktree started on the Stage 0 commit,** the eighth time.

## What was built

- **`--task <id>`** on `build`, `test`, `check` and `run`. The job runs in that running task's worktree. It reuses `is_worktree_of`, and the job record stores `source` and `task`.
- **Build-source notes**, in `state/build-sources/<package>`. When the folder about to be built differs from the note, or there is no note, the job runs `cargo clean -p <package> --offline` (always with `-p`, and only inside `build_dir`), then writes the note, then builds. The order is crash-safe. Package names come from `cargo metadata --no-deps`, so only workspace members are covered.
- **Build lock**, `state/build.lock`, an OS file lock. It is held for all of a job's cargo steps, tests included. New required setting: `build_lock_wait_seconds = 900`. A job that waits longer than that exits 3 with "build folder busy".
- **`run`** also holds the build lock. It deletes the build-source notes of the packages in its folder before and after the command, so a cargo build inside `run` can never be installed.
- **`install`** installs only main's build. The note must be `.`, and the running exe must be exactly `<main root>/build/debug/broadsword.exe`.
- **`clean`** refuses if `build_dir` or any of its parents is a symlink or junction.
- **`destructiveHint`** stays false for build, test and check. The clean removes only regenerable build output, and the tool descriptions say so.

## Tests

- 291 Rust tests pass on the branch.
- Race test: two checkouts build the same package at the same time, and each must get its own binary.
- Mutation checks: without the lock, job 2 tested job 1's binary. Without cleaning, the stale-binary tests fail.
- The fresh coder added a regression test for the resume-lock point. It fails under a mutation.

## Review

Both passes were by a Haiku reviewer, the first use of the cheaper review tier.

**First pass: FAIL.**
- Blocking 1, "resume skips the lock": FALSE POSITIVE. The loop is `enumerate().skip(first_step)`, so a resume already took the lock. The fresh coder pushed back, and the orchestrator confirmed it by reading the original code.
- Blocking 2: REAL. `run broadsword --task <id> -- cargo build` left a worktree binary that `install` would accept.
- Minor points, confirmed real and dealt with in the fix round:
  - junctions: `clean` now refuses them;
  - `current_exe`: `install` now requires main's `build/debug` exe;
  - path dependencies: documented as not covered.
- Design advice, both taken:
  - keep one lock and hold it through tests, because running tests outside the lock is unsound (`clean -p` could delete a test exe before it runs);
  - set `destructiveHint` to false for build, test and check.

**Second pass: PASS.** It ran the end-to-end check the coder had skipped: `run --task -- cargo build`, then `install` refused, then `build broadsword` from main, then `install` accepted.

## Known limitations and open items (in `IDEAS.md`)

- The junction tests in `tests/buildsrc.rs` leave dangling junctions when their scratch root is deleted.
- `link_in_path` checks every parent up to the drive root. A reparse point above the project, such as a OneDrive-redirected Desktop or an external drive mount, would make every clean refuse.
- `forget_notes` does nothing, silently, if `cargo metadata` fails. For `run`, the forget before the command should fail loudly.

## Cost

- Subagent tokens: coder about 265k (15 min), fresh fix-round coder about 130k, Haiku reviewer about 155k (cumulative over two passes).
- Opus reviews of similar changes earlier today: about 130–180k tokens each.
- Wall time: about 50 minutes.
- Claude plan usage when checked earlier: 5% of the weekly all-models limit. Extra usage was on, with $35.46 spent.
