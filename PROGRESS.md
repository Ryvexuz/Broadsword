# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 1 milestone 3.

## Where things stand

Stage 1 (Foundation) is three milestones in, out of five:

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. Resource monitor | passed, merged | `evidence/stage-1-m1-resource-monitor.md` |
| 2. Starter crash sim | passed, own repo `projects/crash-sim/` at `9ff5289` | `evidence/stage-1-m2-starter-crash-sim.md` |
| 3. Job runner and test runner | passed, merged at `5410333` | `evidence/stage-1-m3-job-runner.md` |
| 4. Recovery (Git checkpoints) | not started | |
| 5. Tool registry | not started | |

Stage 0 is still untagged; see Unfinished.

## Working

The `broadsword` binary is in `crates/broadsword/` and uses only the standard library. Build it with `cargo build --offline` at the root, which writes `build/debug/broadsword.exe`. Its commands:

- `status`, `cleanup`, `watch`, `resume [--force]` and `hook pre-tool`: the resource monitor and the pause hook.
- `projects`: lists `broadsword` and every folder under `projects/` that holds a `Cargo.toml`.
- `build`, `test` and `check <project>`: run cargo `--offline` in the shared `build/`. A job is refused (exit 3) if Broadsword is paused or if starting it would leave less than 2 GB above the floor. During a job, free space is checked every 5 s; below the floor the cargo process tree is killed, Broadsword pauses, and the job exits 4. Each job is logged to `scratch/logs/jobs/`. A summary line per step reports summed test counts. Exit 0 only if every step passed.

There are 48 Broadsword tests (23 unit, 15 CLI, 10 jobs) and 11 crash-sim tests, all passing.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **Not running yet:** nothing starts `broadsword watch` automatically. A Windows scheduled task would change the machine's setup and needs Lazlo's OK.
- **Waiting on the network:** phone alerts (`alert_command` is empty) and `cargo-sweep` (not installed). Both need network access and Lazlo's OK.
- **`IDEAS.md` items for the Stage 1 checkpoint:**
  - Plain `cargo` in a worktree still makes its own build folder.
  - `build broadsword` relinks the binary the hook is running.
  - The worktree tool branches from an old commit.

## Failed

- Nothing is failing.

## Disk

128.5 GB free against the 20 GB floor. Space jumped from 23.9 GB during the session, from outside Broadsword.

## Next step

Stage 1, milestone 4: recovery. Before a risky job, Broadsword makes a Git checkpoint. Done check: kill Broadsword mid-job; on restart it resumes or rolls back and nothing is lost. This needs a durable job record that does not live in `scratch/`, since scratch can be wiped. Decide where it lives first, for example `state/jobs/`, ignored by Git, with the path in `broadsword.toml`. Then write the task spec and claim in `crates/broadsword/` and hand it to the coder.
