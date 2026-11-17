# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 1 milestone 2.

## Where things stand

Stage 1 (Foundation) is under way.

- Milestone 1, the resource monitor: passed, merged on `main`.
- Milestone 2, the starter crash sim: passed, in its own repo at `projects/crash-sim/`.
- Stage 0 is still untagged; see Unfinished.

## Starter crash sim

- `projects/crash-sim/` is its own Git repo, on `main` at `9ff5289`. Broadsword ignores it.
- A cube of 8 nodes and 28 spring-damper beams drops 2 m, bounces once to center_y 1.004 m and settles on a face.
- `cargo run --offline` prints CSV `t,min_y,center_y,max_beam_strain` for 5 s.
- Unstable time steps are refused with exit 1.
- 11 tests pass. Evidence: `evidence/stage-1-m2-starter-crash-sim.md`.
- Small leftover: a doc comment in `src/lib.rs` says ω is about 450 rad/s; the reviewer computed exactly 400.

## Working

- **`broadsword` binary**, in `crates/broadsword/`, using only Rust's standard library. Build it with `cargo build --offline` from the project root; it lands in `build/debug/broadsword.exe`. Its commands:
  - `status`: disk, RAM and CPU against their limits, and whether Broadsword is paused.
  - `cleanup`: deletes scratch files past their age, caps logs, runs `cargo sweep` if installed, and pauses if free space is still below the floor.
  - `watch`: the hourly loop, plus a disk check every 60 s.
  - `resume [--force]`: lifts the pause.
  - `hook pre-tool`: the Claude Code hook.
- **Pause.** `broadsword.paused` at the main root. A PreToolUse hook in `.claude/settings.json` blocks every tool except Read, Glob, Grep and `broadsword status|resume|cleanup` commands. It works from worktrees too.
- **Tests.** 17 unit tests and 15 CLI tests, all passing. Clippy is clean.
- **Evidence.** `evidence/stage-1-m1-resource-monitor.md`: a real 5 GB fill paused the main session.
- The four role files and the Stage 0 setup are unchanged.

## Unfinished

- **Stage 0 close-out.** task-router's files are gone from the account, but a headless session started from the running app still listed it. Confirm in a new session started from the app, then tag `stage-0-done`. Nothing named TokenWise was found.
- **Not running yet:** nothing runs `broadsword watch` automatically. Starting it at login, for example with a Windows scheduled task, changes the machine's setup, so it needs Lazlo's OK.
- **Phone alerts:** `alert_command` is empty. Sending to a phone needs a network service and Lazlo's OK.
- **`cargo-sweep`:** not installed, so cleanup skips it. Installing it needs the network.
- **Shared build folder in worktrees:** works only when `CARGO_TARGET_DIR` is set; see `IDEAS.md`.

## Failed

- Nothing is failing.

## Next step

Stage 1, milestone 3: the job runner and test runner. Done check: one command builds and tests any project, starting with `projects/crash-sim`. It should also:

- refuse to start a job when free space is below the floor;
- run builds with `CARGO_TARGET_DIR` set to the main root's `build/`, which closes the worktree gap in `IDEAS.md`.

Start by writing its task spec and claim: a `broadsword build <project>` / `broadsword test <project>` command in `crates/broadsword/`.

Disk: 24.1 GB free against the 20 GB floor.
