# Stage 2, milestone 2: headless mode — done check

**Check:** a build runs through MCP with no window open.

**Result: PASS**, 2026-11-17, on `main` at merge commit `d188998`.

"No window open" is read here as follows. Broadsword runs only as an MCP server that the client launches over stdio. No terminal or interface is open, and no person is involved.

## Command

The stable binary was installed first:

```text
$ build/debug/broadsword.exe install
installed: C:\Users\setup\Desktop\Broadsword\build/bin\broadsword.exe (1481728 bytes)
```

> **Correction (2026-11-18, full audit S10):** this run is weaker than the done check's wording, "a fresh AI session that has never seen the project". Its prompt named the server, the project and the action. Only four tools were allowed, and it ran from the project folder, so it may have loaded `CLAUDE.md`. It proves that a headless session can drive a crash-sim build through MCP with no interface open. It does not prove that a session with no hints finds its own way. A stricter re-run is planned, with a neutral prompt, all tools allowed and a working folder outside the project.

A fresh headless Claude Code 2.1.325 session then ran with a temporary MCP config pointing at `build/bin/broadsword.exe mcp`. No Claude Code settings or `.mcp.json` were changed for this. Its own built-in tools were off, and only four Broadsword tools were allowed:

```bash
claude -p "Use the broadsword MCP tools to build and test the project named crash-sim in one job, then report the result in two lines." \
  --mcp-config scratch/mcp-check/mcp.json --strict-mcp-config --tools "" \
  --allowedTools "mcp__broadsword__check,mcp__broadsword__projects,mcp__broadsword__status,mcp__broadsword__tools" \
  --max-turns 6 --verbose --output-format stream-json < /dev/null
```

## Output

```text
model: claude-opus-5-5 | mcp: [{'name': 'broadsword', 'status': 'connected'}]
CALL mcp__broadsword__check {"project": "crash-sim"}
RESULT ... Running tests\physics.rs ... "truncated":false}
result: success, 2 turns, $0.120, 8.4 s
The `check crash-sim` job (id `20261117-202537-crash-sim-check`) passed: the build succeeded in 0.2 s and all 11 tests passed, with 0 failures and exit code 0.
```

Server log:

```text
started, version 0.1.0, protocol versions 2024-11-05 2025-03-26 2025-06-18 2025-11-25
tools run as C:\Users\setup\Desktop\Broadsword\build\bin\broadsword.exe
initialize: client asked "2025-11-25", negotiated 2025-11-25
tools/list
tools/call check
tools/call check -> exit 0
stopped
```

The job record is `20261117-202537-crash-sim-check  done`, and the crash sim's `git status` is clean.

## What was built

- **`tools/call`.** Each call runs the installed binary as a child process.
  - Arguments are validated against the tool's schema. A bad tool gives -32602; bad arguments give `isError`.
  - Output is capped by `mcp_output_max_kb`, and calls time out after `mcp_call_timeout_seconds`, with the process tree killed.
  - On protocol 2025-06-18 or later, results include `structuredContent` and each tool has an `outputSchema`.
  - `watch` is refused over MCP.
- **During a call.** One call runs at a time. Pings are still answered, `notifications/cancelled` kills the call, and other requests queue.
- **`broadsword install`.** It copies the binary atomically to `build/bin/`. A copy that is running gets renamed aside first.
- **The pause hook** now runs `build/bin/broadsword.exe`, falling back to `build/debug/`. While paused, it allows the read-only `mcp__broadsword__*` tools plus `resume` and `cleanup`.

## Tests

199 tests pass on `main`. Clippy runs with `-D warnings` and is clean.

## Review

**First pass: PASS.** The reviewer ran a real client end to end against a throwaway project, and its probes found:

- **Argument injection:** none. Values like `--help`, `../x` and quotes all reached the child as a single argument.
- **Timeout and cancel:** both kill the whole process tree, with no leftover processes.
- **Load:** 2000 pings during a call were answered in 0.05 s, and 400 queued requests were all answered in order.
- **`install`:** worked while a copy was running.
- **Hook:** blocked every odd input it was given.

## Known limitations (reviewer's minor items)

1. While 256 messages are queued, a cancel or ping for the running call goes unseen until it ends.
2. A cancel for a request that is still queued, not yet running, is ignored, and that call runs anyway.
3. `run`'s `command` array is joined with spaces and run through `cmd /C`, so element boundaries are lost and `&` chains commands. `run` is an arbitrary-command tool by design, but MCP callers may expect argv semantics. This needs documenting or quoting.
4. The text content is human-readable, not serialized JSON. The spec makes JSON a SHOULD, and `structuredContent` carries it.
5. In the reviewer's first run, the model chose `build` and then `test` over `check` even when only `check` was allowed. Here, with "in one job" in the prompt, it picked `check` directly. This is data for the lesson focus.
6. During an install there is a short window where `build/bin/broadsword.exe` is missing. The hook falls back to `build/debug/`.
7. A timed-out or cancelled job's record stays `running`. The next job auto-recovers it, as documented.
8. The supported protocol versions are still unchecked against the live spec.

## Stricter re-run (2026-11-18, after audit S10)

A headless Claude Code session (model `claude-haiku-5-5`) was started from a folder outside the project, so it had no `CLAUDE.md` and no project files. It used a temporary MCP config that starts `buildinroadsword.exe mcp` in the root. All Broadsword tools were allowed (`--allowedTools mcp__broadsword`), and its built-in tools were off. The prompt named neither a tool nor the project:

> You have an MCP server connected. Find out what it can do, then use it to build and test the crash simulator it manages, and tell me in two lines whether that worked.

Result: 4 turns, $0.0066. The server log shows `tools/call projects`, then `status`, then `check` (exit 0), and job record `20261118-053737-crash-sim-check` exists. The session's answer:

> The Broadsword MCP server offers project-scoped build, test, check and run jobs, a job log with recovery, disk and RAM status, a task board with claims and worktrees, and a queue that reviews and merges task branches. It manages two projects: `broadsword` and `crash-sim`.
>
> It worked: `check crash-sim` built the project and ran all 40 tests, which passed with 0 failures (job `20261118-053737-crash-sim-check`).
>
> Yes, the build and test both succeeded. The build step reported "Finished in 0.01s" because the artifacts were already current, so no fresh compile happened, but the test binaries ran and all 40 passed.

This meets Stage 2 done check 1 as worded: a session that had never seen the project connected, found the tools and ran a crash-sim build with no interface open.
