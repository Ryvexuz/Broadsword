# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 1 milestone 1.

## Where things stand

Stage 1 (Foundation) has started. Milestone 1, the resource monitor, has passed its done check and is merged on `main`. Stage 0 is still untagged; see Unfinished.

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

Stage 1, milestone 2: the starter crash sim. It is a cube of nodes and beams that falls and bounces, in its own repo. Done check: it builds, runs headless and prints the cube's height over time. Decide first where its repo lives. To keep the project portable, a folder inside the project such as `projects/crash-sim/` with its own `git init` is the likely choice, ignored by the Broadsword repo. Start by writing that milestone's task spec and claim, then hand it to the coder.

Disk: 24.1 GB free against the 20 GB floor.
