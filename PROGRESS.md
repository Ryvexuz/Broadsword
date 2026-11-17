# Progress

Rewritten at the end of each session. Last updated 2026-11-17, at the end of Stage 2.

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** done, tagged `stage-2-done`.

| Milestone | Evidence |
| --- | --- |
| 1. MCP server | `evidence/stage-2-m1-mcp-server.md` |
| 2. Headless mode (`tools/call`) | `evidence/stage-2-m2-headless-mode.md` |
| 3. Second provider (OpenAI) | `evidence/stage-2-m3-second-provider.md` |
| 4. Local model | skipped (optional) |
| Lesson-focus experiment | `evidence/stage-2-description-experiment.md` |

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword mcp`**, from `build/bin/`, serves 13 tools.
  - Claude Code and OpenAI models can both list and call them.
  - The same `check crash-sim` build passed through both.
- **`clients/openai_mcp.py`** bridges OpenAI to the server.
  - Run it with `python -I -B clients/openai_mcp.py --prompt "..."`.
  - The key comes from `OPENAI_API_KEY` in the environment, or from `.env`.
- **After changing Broadsword's code:** build, then `build/debug/broadsword.exe install`.
- **Tests:** 199 Rust tests, 43 client tests and 11 crash-sim tests, all passing.

## Unfinished

- **Stale OpenAI key in Windows.** Lazlo's user environment has an `OPENAI_API_KEY` that OpenAI rejects, and it differs from `.env`. Remove or update it. It was left unchanged.
- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **Register Broadsword's MCP server** in this project's `.mcp.json`? Needs Lazlo's OK.
- **Proposed `AGENTS.md` change, not applied:** tool descriptions are part of the done check.
- **Not running yet:** `broadsword watch` doesn't start automatically. Phone alerts and `cargo-sweep` need network access and Lazlo's OK.
- **`IDEAS.md` items:**
  - MCP minors;
  - the env-versus-`.env` key warning;
  - a larger description experiment;
  - the stale-binary hazard of the shared build folder;
  - worktree builds of Broadsword's own code;
  - the old worktree base commit;
  - pruning recovery refs;
  - faster ignored-file capture.

## Failed

- Nothing is failing.

## Disk

About 127 GB free against the 20 GB floor.

## Next step

Stage 3: Autonomous Development. Milestone 1 is a task board: tasks with file claims, status and owner. Done check: two tasks with overlapping claims never run at once.

Until now the orchestrator kept claims in its head and in each brief. This makes them a Broadsword tool, so Stage 3's parallel agents can use it through MCP.

Stage 3 also covers several `IDEAS.md` items:

- worktrees created from `main`'s tip;
- the shared-build-folder hazards;
- whether to raise `max_agents` above 2.
