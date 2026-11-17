# Stage 0 done check

**Check:** a fresh Claude Code session in the folder can state the disk floor, the agent cap and the first task without being told.

**Result: PASS**, 2026-11-17.

## Command

Run from the project root in Git Bash. `claude` is the Claude Code 2.1.325 binary that ships with the desktop app (it is not on `PATH`, so it was called by its full path).

```bash
claude -p "What is the disk floor, the agent cap, and the first task for this project? Answer in three short lines." \
  --tools "" --max-turns 1 --strict-mcp-config --verbose --output-format stream-json \
  < /dev/null > scratch/stage0-check.jsonl
```

## What the session had to work with

Taken from the session's own startup record (`init` line of the output):

| Field | Value |
| --- | --- |
| Working folder | the project root |
| Session | new session id, no history |
| Model | `claude-opus-5-5` |
| Tools | `[]` (none, so it could not open a file) |
| MCP servers | `[]` (none) |
| Turns | 1 |

With no tools and one turn, the answer can only come from what Claude Code loads at startup: `CLAUDE.md`, which pulls in `AGENTS.md`.

## Output

```text
- **Disk floor:** 20 GB (`disk_floor_gb = 20`)
- **Agent cap:** 2 (`max_agents = 2`, raise only after clean merges)
- **First task:** build only the resource monitor (disk/RAM/CPU checks, disk floor, hourly cleanup); the tiny falling-cube crash sim repo is the test project that follows.
```

Result line: `subtype=success is_error=False num_turns=1 duration_ms=6012 cost_usd=0.0122876`

All three match `AGENTS.md` and `broadsword.toml`.

## Also visible in the startup record

- **Role files are picked up.** The session's agent list includes `coder`, `test-writer`, `reviewer` and `housekeeper`.
- **task-router is still loaded.** The skill list includes `anthropic-skills:task-router`, so the "task-router removed" item of Stage 0 is not done. It is an account-level skill, outside this repo.

## Earlier attempts

1. Same command without `--strict-mcp-config` and with `--output-format json`: no output after more than three minutes. Stopped by hand.
2. With `--strict-mcp-config`, output piped to the terminal, under a 100 second limit: the session started and began answering, then stalled until the limit cut it off.

The run above is the third. It wrote its output to a file and finished in about 7 seconds. Why the first two stalled was not established.

The raw output is in `scratch/stage0-check.jsonl`, which can be wiped at any time; everything needed is copied here.
