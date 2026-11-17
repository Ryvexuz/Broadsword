# Progress

Rewritten at the end of each session. Last updated 2026-11-17, at the end of Stage 1.

## Where things stand

**Stage 1 (Foundation) is done** and tagged `stage-1-done`. All five milestones passed their done checks:

| Milestone | Evidence |
| --- | --- |
| 1. Resource monitor | `evidence/stage-1-m1-resource-monitor.md` |
| 2. Starter crash sim (own repo `projects/crash-sim/`) | `evidence/stage-1-m2-starter-crash-sim.md` |
| 3. Job runner and test runner | `evidence/stage-1-m3-job-runner.md` |
| 4. Recovery (Git checkpoints) | `evidence/stage-1-m4-recovery.md` |
| 5. Tool registry | `evidence/stage-1-m5-tool-registry.md` |

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword tools`** lists the 13 tools: `status`, `cleanup`, `watch`, `resume`, `projects`, `build`, `test`, `check`, `run`, `recover`, `abandon`, `jobs` and `tools`. `tools <name>` describes one in full, and `tools --json` gives MCP-shaped JSON.
- **Dispatch and the hook:** the command line dispatches from that same table. `hook pre-tool` sits outside the table and blocks agent tools while paused.
- **Tests:** 108 Broadsword tests and 11 crash-sim tests, all passing.
- **Building:** `cargo build --offline` at the root builds `build/debug/broadsword.exe`.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **`AGENTS.md` changes from the Stage 1 checkpoint.**
  - **Applied 2026-11-17 with Lazlo's OK,** in both `AGENTS.md` and the Development Instructions doc, under Standing rules:
    - "Build through Broadsword";
    - "Review" (at least two passes; fix an invariant in one place).
  - **Still only proposed:** tool descriptions are part of the done check, meaning an AI reading only the description must get the behaviour right.
- **Agent cap and roles:** keep `max_agents = 2`. Stage 1 never ran two coders at once, so there is no evidence yet for raising it; that is Stage 3's job. Keep the reviewer on Opus. The test writer added little once the coder wrote real tests, so consider making it optional in the role list.
- **Not running yet:** nothing starts `broadsword watch` automatically. That needs Lazlo's OK.
- **Waiting on the network:** phone alerts and `cargo-sweep`. Both need network access and Lazlo's OK.
- **`IDEAS.md` items:**
  - the worktree tool branching from an old commit (happened three times);
  - plain `cargo` in a worktree;
  - `build broadsword` relinking the hook binary;
  - pruning recovery refs;
  - faster ignored-file capture.

## Failed

- Nothing is failing.

## Disk

128 GB free against the 20 GB floor.

## Next step

Stage 2: AI Integration. Milestone 1 is an MCP server that exposes the tool registry. Done check: Claude Code connects and lists the tools. The registry's `tools --json` is already in MCP `tools/list` shape, and each tool's handler can be called from the table.

Decide first how the server talks. Probably stdio JSON-RPC, which is what Claude Code uses for local MCP servers, as `broadsword mcp`. Still std only, so it needs a small hand-written JSON parser for requests. Then write the task spec and claim, and hand it to the coder.

Adding the server to Claude Code's MCP config changes this project's settings, so ask Lazlo before doing it.
