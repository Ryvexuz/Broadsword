# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after Stage 2 milestone 3.

## Where things stand

- **Stage 1 (Foundation):** done, tagged `stage-1-done`.
- **Stage 2 (AI Integration):** the three required milestones are done. Milestone 4 is optional.

| Milestone | Status | Evidence |
| --- | --- | --- |
| 1. MCP server | passed | `evidence/stage-2-m1-mcp-server.md` |
| 2. Headless mode (`tools/call`) | passed | `evidence/stage-2-m2-headless-mode.md` |
| 3. Second provider (OpenAI) | passed, merged at `be33b37` | `evidence/stage-2-m3-second-provider.md` |
| 4. Local model (optional) | not started | |

Stage 0 is still untagged; see Unfinished.

## Working

- **`broadsword mcp`**, launched from `build/bin/`. Claude Code and OpenAI models can both list and call Broadsword's tools.
- **`clients/openai_mcp.py`** bridges OpenAI Chat Completions to the server.
  - It uses Python's standard library only. Run it with `python -I -B clients/openai_mcp.py --prompt "..."`.
  - The key comes from `OPENAI_API_KEY` in the environment, or from `.env`.
  - Default tools: read-only tools plus `check`, `build` and `test`.
- **Tests:** 199 Rust tests, 43 client tests and 11 crash-sim tests, all passing.

## Unfinished

- **Stale OpenAI key in Windows.** Lazlo's Windows user environment has an `OPENAI_API_KEY` that OpenAI rejects, and it differs from the key in `.env`. The client reads the environment first. So remove or update the Windows variable (System Properties → Environment Variables), or run with it cleared. It was left unchanged.
- **Stage 2 close-out.** Decide whether to do optional milestone 4 (local model). Then do the stage checkpoint:
  - three lessons at the top of `LAB_LOG.md`;
  - the `check` description experiment for the lesson focus;
  - tag `stage-2-done`.
- **Stage 0 close-out.** Confirm in a new session started from the app that task-router no longer loads, then tag `stage-0-done`.
- **Register Broadsword's MCP server** in this project's `.mcp.json`? Needs Lazlo's OK.
- **Proposed `AGENTS.md` change, not applied:** tool descriptions are part of the done check.
- **Not running yet:** `broadsword watch` doesn't start automatically. Phone alerts and `cargo-sweep` need network access and Lazlo's OK.
- **`IDEAS.md` items:** MCP minors, the env-versus-`.env` key warning, the `check` description experiment, worktree builds of Broadsword's own code, the old worktree base commit, pruning recovery refs, faster ignored-file capture.

## Failed

- Nothing is failing.

## Disk

About 127 GB free against the 20 GB floor.

## Next step

Two choices:

1. **Close out Stage 2 now.** Skip optional milestone 4, run the `check` description experiment from `IDEAS.md` (the stage's lesson focus: rewrite a description, measure tool choice over several runs with both models), write the stage lessons and tag `stage-2-done`.
2. **Do milestone 4 first.** The same build through a local model. That needs a local model runtime such as Ollama, a download of several GB, and Lazlo's OK for both.
