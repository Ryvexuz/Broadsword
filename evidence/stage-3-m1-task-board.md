# Stage 3, milestone 1: task board — done check

**Check:** two tasks with overlapping claims never run at once.

**Result: PASS**, 2026-11-17, on `main` at merge commit `e82e6bf`.

## On the real board and project

The run used `scratch/m3-1-done-check.sh` against the real board (`state/tasks/`) and the real `crash-sim`. It ran 30 rounds. Each round added task A claiming `src` and task B claiming `src/lib.rs`, started both as background processes at the same moment, counted the running tasks, then abandoned both.

```text
round 1 exits: A=0 B=3 (0 = started, 3 = refused for a conflict)
rounds=30 bad=0  A won 30, B won 0
running crash-sim tasks left: 0
```

A refused start, verbatim:

```text
$ broadsword task start t-20261117-215656-4 --owner check-b
broadsword: cannot start t-20261117-215656-4: its claims overlap a running task:
  task t-20261117-215656-3 (owner check-a): your claim `src/lib.rs` overlaps its claim `src`
start it after that task is done, failed or abandoned
exit=3

$ broadsword task check crash-sim --claim src/vec3.rs     -> conflict ... exit=3
$ broadsword task check crash-sim --claim README.md       -> free: no running task in `crash-sim` overlaps README.md, exit=0
```

**Caveat.** A won every round. It was launched a moment earlier, so this run shows conflicts being refused, but it doesn't contest the race hard. The race evidence is the gated tests below, where all starts are released at the same instant. The crash sim's files were untouched: `git status` was empty, because tasks don't edit files.

## Race evidence (gated: every process waits on a go-file, then all start at once)

| Run | By | Rounds × starts | Result |
| --- | --- | --- | --- |
| `race2` | coder test | 60 × 2 | exactly one winner every round, 14.8 s |
| `race8` | coder test | 10 × 8, all mutually overlapping | exactly 1 running every round |
| `race16` | coder test | 8 × 16, 3,000 filler records, garbage lock each round | exactly 1 running, 0 blocked |
| Stale-lock repro | reviewer | 32 × 16, planted `4000000000` lock, 3,000 filler records | 0 bad rounds, 0 exit-4 |
| Mixed stress | reviewer | 20 × 16, half CLI and half MCP, main root plus a git worktree, claims spelled `*` / `SRC/a.rs` / `src\a.rs`, projects `p`/`P` | 20 winners out of 320, one per round |
| Kill the holder | coder test and reviewer | `taskkill /F` or `Stop-Process` on the lock holder | the next start got the lock at once, under 3 s |

A mutation check confirmed the race test is real. Giving each process its own lock file made round 0 fail, with both starts getting exit 0.

## What `broadsword task` does

- **Records.** One durable record per task in `state/tasks/<id>.task`: title, canonical project, claims, owner, status (`open` → `running` → `done`/`failed`, or `abandoned`), times and a note.
- **Claims.** Paths relative to the project, normalised:
  - `\`/`/`, `.` segments and trailing slashes;
  - case-insensitive on Windows;
  - `*` means the whole project.
  - Two claims overlap if they are equal or one is an ancestor by whole path segments. `src` overlaps `src/lib.rs`; `src` does not overlap `srcx`.
  - Claims are compared lexically: 8.3 short names and junctions are not resolved.
- **Project names** are resolved to the folder's real name in one place (`job::canonical_project`). `crash`, `CRASH` and `Crash` are one project; `Crash.` and `crash ` are rejected.
- **Commands:**
  - `start`, `done`, `fail` and `abandon` run under the board lock;
  - `start` refuses with exit 3 if a running task's claims overlap;
  - `check` is read-only.
  - All are registry tools (`task_add`, `task_start`, …), so they work over MCP.
- **The lock** is an OS file lock (`File::try_lock`, LockFileEx on Windows) held on an open handle for the critical section only. The OS releases it when a process dies, so there is no stale-lock detection or takeover. `recover.lock` now uses the same mechanism.

## Tests

234 Rust tests pass on `main`, 27 of them task tests. Clippy runs with `-D warnings` and is clean.

## Review

- **First pass: FAIL.** The reviewer reproduced two overlapping tasks running at once.
  - The stale-lock takeover inherited from `recover` (rename aside, re-read, put back) let a waiter move a fresh lock aside while a third process created its own, so there were two holders. This was Stage 1's known limitation 7.
  - The same code left ownerless locks that blocked starts for 60 s.
  - Project names were compared as raw strings, so `Crash` and `crash` didn't conflict.
  - A new setting had a hard-coded default.
- **The fix replaced the design** rather than patching it: OS file locks for both the board and recover.
- **Second pass: PASS.**

## Not covered

- Claims are lexical only: no symlink, junction or 8.3 resolution. Agents must claim real paths.
- Tasks are not yet linked to worktrees or branches. That is milestone 2.
- 62 task records from the tests and the done check stay in `state/tasks/`, all `abandoned`. Records are never deleted, by design.
