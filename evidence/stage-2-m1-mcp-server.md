# Stage 2, milestone 1: MCP server — done check

**Check:** Claude Code connects and lists the tools.

**Result: PASS**, 2026-11-17, on `main` at merge commit `80aa369`.

## Command

A fresh headless Claude Code 2.1.325 session ran with a temporary MCP config, passed on the command line. No project or user Claude Code settings were changed; there is no `.mcp.json` in the repo. The server ran from a copy of the binary in `scratch/mcp-check/`, so the shared `build/debug/broadsword.exe` stayed unlocked.

```bash
# scratch/mcp-check/mcp.json
{"mcpServers":{"broadsword":{"type":"stdio","command":"<root>/scratch/mcp-check/broadsword.exe","args":["mcp"]}}}

claude -p "Without calling any tool, list the names of every tool the broadsword MCP server offers, one per line, then stop." \
  --mcp-config scratch/mcp-check/mcp.json --strict-mcp-config --tools "" --max-turns 1 \
  --verbose --output-format stream-json < /dev/null > scratch/mcp-check/run.jsonl
```

## Output

From the session's `init` event:

```text
model: claude-opus-5-5
mcp_servers: [{'name': 'broadsword', 'status': 'connected', 'source': 'dynamic'}]
tools: ['mcp__broadsword__abandon', 'mcp__broadsword__build', 'mcp__broadsword__check', 'mcp__broadsword__cleanup',
        'mcp__broadsword__jobs', 'mcp__broadsword__projects', 'mcp__broadsword__recover', 'mcp__broadsword__resume',
        'mcp__broadsword__run', 'mcp__broadsword__status', 'mcp__broadsword__test', 'mcp__broadsword__tools',
        'mcp__broadsword__watch']
result: success, 1 turn, $0.098, 6.2 s
```

The model listed the same 13 names, then added: "Some tools load later, so I can't rule out more on the server. The `tools` tool would give the full list."

Server log (`scratch/logs/mcp.log`):

```text
started, version 0.1.0, protocol versions 2024-11-05 2025-03-26 2025-06-18 2025-11-25
server/discover                      (answered -32601; the client carried on)
initialize: client asked "2025-11-25", negotiated 2025-11-25
notifications/initialized (notification) -> no reply
tools/list
stopped
```

## What `broadsword mcp` does

- **Transport:** stdio JSON-RPC 2.0, one message per line. Nothing but protocol goes to stdout; diagnostics go to stderr and to `scratch/logs/mcp.log`.
- **Methods:**
  - `initialize`, with version negotiation.
  - `ping`.
  - `tools/list`, built from the same function as `broadsword tools --json`.
  - Notifications and client responses get no reply.
  - `tools/call` returns -32601 until milestone 2, and the server's instructions say so.
- **JSON parser:** strict and std-only, with a nesting limit of 64 and a line limit of 1 MiB.
- **Robustness (from the reviewer's fuzzing):**
  - no panics;
  - a 500 MiB line held the server to 7.8 MiB of memory;
  - 100k pings took 1.8 s.

## Tests

The full suite passes on `main`: 154 tests (83 unit, 15 CLI, 10 jobs, 22 MCP, 13 recovery, 11 tools). Clippy runs with `-D warnings` and is clean.

## Review

**First pass: PASS.** This is the first first-pass PASS in the project. The reviewer connected a real Claude Code client and found three minor items, which were fixed before the merge:

- **Instructions.** They told the client to "call the `tools` tool first", which can't work yet. The real client did exactly that as its first action.
- **Client responses.** Responses sent by the client were being answered.
- **Edge cases:** a trailing `\r` counted against the line limit, oversized or non-integer numeric ids, and the negotiated version wasn't logged.

The orchestrator read the fix diff.

## Not covered

- The supported protocol version list came from the coder's knowledge and was not checked against the live MCP spec, because that would need network access.
- `tools/call` is milestone 2.
- **Open risk for milestone 2:** a long-running `broadsword mcp` launched from the shared `build/debug/broadsword.exe` would lock it on Windows. Rebuilds would then fail, and the pause hook runs that same binary.
