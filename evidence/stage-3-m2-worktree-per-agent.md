# Stage 3, milestone 2: worktree per agent — done check

**Check:** worktrees are created and removed automatically, and disappear after merging.

**Result: PASS**, 2026-11-17, on `main` at merge commit `c64d9d1`.

## On the real crash sim, with real work

The task was to correct the crash sim's "about 450 rad/s" comments. The Stage 1 reviewer had computed the stiffest mode as exactly 400 rad/s. The run used `scratch/m3-2-done-check.sh`, then continued by hand (see the note below).

```text
=== before ===
crash-sim main: 9ff5289  branches: main
task: t-20261117-224405   (claim: src/lib.rs)

=== start --worktree ===
task t-20261117-224405 is running (owner orchestrator); its claims are held until it is done, failed or abandoned
worktree: ...\worktrees\crash-sim\t-20261117-224405
branch: task/t-20261117-224405
base: 9ff528907fe3b10c4b3511447abd6f03d3016d44
worktree base == crash-sim main HEAD: yes

=== work in the worktree ===
 src/lib.rs | 4 ++--
committed on task/t-20261117-224405: 8f9b2c6 Correct omega_max comments: the stiffest cube mode is 400 rad/s

=== removal while the task is still running ===
broadsword: task t-20261117-224405 is running; its worktree is in use. Finish the task first (...)    [exit 3]

=== merge into crash-sim main ===
crash-sim main now: 8f9b2c6 Correct omega_max comments ...
check crash-sim exit=0  (test: ok — 11 passed, 0 failed)

=== task done ===
task t-20261117-224405 is done; its claims are released
worktree removed: worktrees/crash-sim/t-20261117-224405; branch task/t-20261117-224405 deleted (it was merged)
task done exit=0

=== after ===
worktree on disk: gone
worktrees/crash-sim folder: gone
branch task/t-20261117-224405: gone
git worktree list:  .../projects/crash-sim 8f9b2c6 [main]   (only the main checkout)
crash-sim status: 0 changes
task show: status done; worktree: worktrees/crash-sim/t-20261117-224405 (removed); base 9ff5289...
```

**Note on the run.** The script used `set -e`, so it stopped when `worktree remove` correctly refused a running task with exit 3. The remaining steps were run by hand with the same commands, and the output above comes from both parts.

**The old-commit problem is fixed.** Worktrees made by the subagent tool started on the Stage 0 commit seven times. Broadsword's worktree was based on the project's current `main` tip.

## What was built

- **Settings.** A required setting, `worktrees_dir = "worktrees"`, which is Git-ignored. Each task's worktree goes in `worktrees/<project>/<task id>` on branch `task/<task id>`, in the project's own repo.
- **Creation.** `task start --worktree` works in three steps:
  1. Under the board lock, it checks for conflicts, marks the task running and records that creation has begun.
  2. Outside the lock, it runs `git worktree add` from a freshly resolved HEAD.
  3. Under the lock again, it records the result, or rolls back to `open`.

  A crashed creator is recovered by `worktree create`, `task fail` or `task abandon`. `worktree create <id>` is also available on its own for a task that's already running.
- **Removal.** One function serves `task done`, `worktree remove [--keep-branch]` and `worktree prune`. It removes a worktree only when all of these hold:
  - the folder really is that project's worktree (a stray folder is left in place);
  - it is clean, with no ignored files either (`.env` and notes block it);
  - it is on `task/<id>`;
  - it is merged into the main checkout's HEAD.

  Commits reachable only from the worktree's HEAD log, such as those left after a rebase or amend, are saved first under `refs/broadsword/kept/<id>/<sha>`. It never uses `--force` or `-D`, and empty parent folders are removed only if they are real directories.
- **Other commands.** `worktree list` (read-only), `worktree repair` (after moving the project) and `worktree prune` (which never touches running tasks).

## Git commands added, all in `crates/broadsword/src/wtgit.rs`

`symbolic-ref -q --short HEAD`, `rev-parse HEAD`, `show-ref --verify --quiet`, `worktree add -b`, `status --porcelain --ignored`, `merge-base --is-ancestor`, `log -g --format=%H HEAD`, `rev-list --stdin --not --all`, `update-ref <ref> <sha> ""`, `rev-parse --show-toplevel` / `--git-common-dir`, `worktree remove`, `branch -d`, `worktree prune`, `worktree repair`.

## Tests

The full suite on `main` passes 268 Rust tests, including 33 worktree tests and the 27 task race tests. Clippy runs with `-D warnings` and is clean.

## Review

**First pass: FAIL.**
- `task done` silently deleted ignored files: the reviewer's `.env` and notes folder vanished with exit 0.
- `task_done` was not flagged destructive.
- The board lock was held through `git worktree add`. On a 20k-file repo that took 10.8 s, and a concurrent start got exit 4.

**Second pass: PASS.**
- No attack lost work: squash-merge, amend, stash, submodules, a locked worktree, a rebase in progress, `..` paths, junctions, crashes at each creation step, and two concurrent retries.
- The lock is now held only 80–170 ms while a 12 s creation runs.
- Two minor points were fixed before the merge: stray folders, and orphan commits saved instead of blocking.

## Known limitations

- There is a small window in which an ignored file written between the cleanliness check and `git worktree remove` would be lost. It is documented in the code.
- Claims are still compared lexically.
- `refs/broadsword/kept/*` refs accumulate.
