# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 2 milestone 2.

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** in progress.

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. MCP server | passed | `evidence/stage-2-m1-mcp-server.md` |
| 2. Headless mode (`tools/call`) | passed, merged at `d188998` | `evidence/stage-2-m2-headless-mode.md` |
| 3. Second provider | not started | |
| 4. Local model (optional) | not started | |

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword mcp`.** An AI can list and call every tool except `watch`. A headless Claude Code session ran `check crash-sim` through MCP and got exit 0 with 11 tests passing.
- **`broadsword install`.** It copies a built binary to `build/bin/`. The MCP server and the pause hook run that copy, so rebuilds don't fight a running server.
- **After changing Broadsword's code:** build, then run `build/debug/broadsword.exe install`.
- **Not registered in any Claude Code MCP config.** Tests use a temporary `--mcp-config`.
- **Tests:** 199 Broadsword tests and 11 crash-sim tests, all passing.

## Unfinished

- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **Register Broadsword's MCP server for this project?** A `.mcp.json` at the root would make it available in every Claude Code session here. That needs Lazlo's OK.
- **Proposed `AGENTS.md` change, not applied:** tool descriptions are part of the done check.
- **Not running yet:** nothing starts `broadsword watch` automatically. That needs Lazlo's OK.
- **Waiting on the network:** phone alerts and `cargo-sweep`. Both need network access and Lazlo's OK.
- **`IDEAS.md` items:**
  - MCP minors, especially `run`'s argument joining;
  - the `check` description experiment;
  - worktree builds of Broadsword's own code;
  - the old worktree base commit;
  - pruning recovery refs;
  - faster ignored-file capture.

## Failed

- Nothing is failing.

## Disk

About 127 GB free against the 20 GB floor.

## Next step

Stage 2, milestone 3: second provider. Done check: a different cloud model connects through the same server, and both models run the same build.

This needs a second AI client that speaks MCP, and an API key for another provider. Both are a network service and credentials, so `AGENTS.md` says to ask Lazlo first: which provider, and where the key lives (a `.env` file, git-ignored).

Two options:

1. A small std-only Rust or Python MCP client script that sends the model's tool calls to the server.
2. A second existing MCP client app.

Also worth doing at the start of milestone 3: the `check` description experiment from `IDEAS.md`, since it fits the stage's lesson focus.
