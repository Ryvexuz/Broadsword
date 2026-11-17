# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 2 milestone 1.

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** started.

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. MCP server | passed, merged at `80aa369` | `evidence/stage-2-m1-mcp-server.md` |
| 2. Headless mode (`tools/call`) | not started | |
| 3. Second provider | not started | |
| 4. Local model (optional) | not started | |

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword mcp`** is a stdio MCP server. A fresh Claude Code session connects and sees all 13 tools as `mcp__broadsword__*`.
- **Tool calls** are refused with -32601 until milestone 2.
- **Not added to any Claude Code config.** Testing uses a temporary config passed with `--mcp-config`.
- **Tests:** 154 Broadsword tests and 11 crash-sim tests, all passing.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **Proposed `AGENTS.md` change, not applied:** tool descriptions are part of the done check.
- **Not running yet:** nothing starts `broadsword watch` automatically. That needs Lazlo's OK.
- **Waiting on the network:** phone alerts and `cargo-sweep`. Both need network access and Lazlo's OK.
- **`IDEAS.md` items:**
  - `broadsword build broadsword` can't build a worktree's own code;
  - the worktree tool starts from an old commit;
  - the hook binary gets relinked by builds;
  - pruning recovery refs;
  - faster ignored-file capture.

## Failed

- Nothing is failing.

## Disk

About 137 GB free against the 20 GB floor.

## Next step

Stage 2, milestone 2: headless mode. Done check: a build runs through MCP with no window open. That means implementing `tools/call`. Two things to decide first:

1. **How a call runs.** Probably as a child `broadsword <cli form>` process, with stdout and stderr captured and returned as MCP text content, and `isError` set from the exit code. Tool handlers print to stdout, which would corrupt the protocol stream if they ran in-process. Long-running tools such as `watch` and `check` need a timeout or progress handling.
2. **A stable binary for MCP and the hook.** A long-lived `mcp` process locks `build/debug/broadsword.exe` on Windows, so rebuilds fail, and the pause hook uses the same file. Probably add `broadsword install`, which copies a binary that passed its check to `build/bin/`. MCP and the hook would then point there.

Registering the server in this project's `.mcp.json` changes Claude Code settings, so it needs Lazlo's OK. Ask before doing it.
