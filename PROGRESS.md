# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 1 milestone 4.

## Where things stand

Stage 1 (Foundation) is four milestones in, out of five:

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. Resource monitor | passed, merged | `evidence/stage-1-m1-resource-monitor.md` |
| 2. Starter crash sim | passed, own repo `projects/crash-sim/` at `9ff5289` | `evidence/stage-1-m2-starter-crash-sim.md` |
| 3. Job runner and test runner | passed, merged | `evidence/stage-1-m3-job-runner.md` |
| 4. Recovery (Git checkpoints) | passed, merged at `fde0798` | `evidence/stage-1-m4-recovery.md` |
| 5. Tool registry | not started | |

Stage 0 is still untagged; see Unfinished.

## Working

The `broadsword` binary is in `crates/broadsword/` and uses only the standard library. Build it with `cargo build --offline`; it lands in `build/debug/broadsword.exe`.

- **Monitor:** `status`, `cleanup`, `watch`, `resume [--force]`, plus `hook pre-tool`, which blocks agent tools while paused.
- **Jobs:** `projects`, and `build`, `test` and `check <project>`. Each job has a disk guard, a disk watch during the job, and a log in `scratch/logs/jobs/`. Builds always go to the shared `build/`.
- **Risky jobs:** `run <project> -- <command>` runs a command that may change files.
- **Recovery:**
  - Every job writes a durable record to `state/jobs/` (git-ignored, never cleaned).
  - Before every job, Broadsword takes a byte-exact Git checkpoint in `refs/broadsword/checkpoints/<id>`. It covers tracked, untracked and ignored files up to 50 MB, and does not touch the working tree, HEAD or index.
  - `recover` resumes interrupted read-only jobs, up to `max_resumes` times. It rolls back interrupted `run` jobs after saving their partial work, index included.
  - `recover --abandon <id>` gives up on a job without changing anything.
  - `jobs` lists all records.

There are 84 Broadsword tests (46 unit, 15 CLI, 10 jobs, 13 recovery) and 11 crash-sim tests, all passing.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **Not running yet:** nothing starts `broadsword watch` automatically. That needs Lazlo's OK.
- **Waiting on the network:** phone alerts and `cargo-sweep`. Both need network access and Lazlo's OK.
- **`IDEAS.md` items:**
  - plain `cargo` in a worktree;
  - the hook binary being relinked by `build broadsword`;
  - the worktree tool branching from old commits;
  - pruning recovery refs;
  - faster ignored-file capture.
- **Recovery known limitations:** listed in its evidence file.

## Failed

- Nothing is failing.

## Disk

128 GB free against the 20 GB floor.

## Next step

Stage 1, milestone 5: the tool registry. Every capability becomes a named tool with inputs and outputs. Done check: list all tools from the command line. This is what Stage 2's MCP server will expose.

Start by writing its task spec and claim in `crates/broadsword/`. It should be a `broadsword tools` command, listing each tool's name, description and input/output schema, generated from the same table the commands dispatch from. That way the list can't drift from what actually runs.

After milestone 5, Stage 1 is done. Do the end-of-stage checkpoint from `DEVELOPMENT_PLAN.md`:

- three lessons at the top of `LAB_LOG.md`;
- adjust `AGENTS.md`, for example the rule to build through `broadsword`, and budgeting for a second review pass;
- review the agent cap and roles;
- tag `stage-1-done`.
