# Stage 2, milestone 3: second provider — done check

**Check:** a different cloud model connects through the same server, and both models run the same build.

**Result: PASS**, 2026-11-17, on `main` at merge commit `be33b37`. Lazlo approved using his OpenAI key for these runs.

Both runs used the same server (`build/bin/broadsword.exe mcp`) and the same seven allowed tools: `status`, `projects`, `build`, `test`, `check`, `jobs` and `tools`. Both got the same prompt:

> Use the broadsword tools to build and test the project named crash-sim in one job, then report the result in two lines.

## Side by side

| | OpenAI | Claude |
| --- | --- | --- |
| Model | `gpt-4.1-mini` | `claude-opus-5-5` |
| Client | `clients/openai_mcp.py` (Chat Completions tool calling) | Claude Code 2.1.325, headless |
| Tool calls | `check {"project":"crash-sim"}` | `check {"project":"crash-sim"}` |
| Result | exit 0, 11 passed, 0 failed | exit 0, 11 passed, 0 failed |
| Job | `20261117-205651-crash-sim-check` done | `20261117-205710-crash-sim-check` done |
| Turns | 2 | 2 |
| Usage | 3,679 prompt + 45 completion = 3,724 tokens | 221 output tokens, $0.061 |
| Wall time | about 5 s | 8.9 s |

**OpenAI's answer:** "The build and test of project crash-sim completed successfully with exit code 0. All 11 tests passed and 0 tests failed."

**Claude's answer:** "The `check` job on crash-sim passed (exit code 0, job `20261117-205710-crash-sim-check`): the build succeeded and all 11 tests passed, with none failing."

## OpenAI run, command and output

```text
$ env -u OPENAI_API_KEY python -I -B clients/openai_mcp.py --model gpt-4.1-mini --prompt "..." --json-log scratch/m3-openai.json
[setup] server broadsword.exe, model gpt-4.1-mini, tools offered: status, projects, build, test, check, jobs, tools
[tool call] check {"project":"crash-sim"}
[tool result] exit code: 0
[final answer]
The build and test of project crash-sim completed successfully with exit code 0. All 11 tests passed and 0 tests failed.
[summary] model=gpt-4.1-mini turns=2 tool_calls=1 (refused 0) prompt_tokens=3679 completion_tokens=45 total_tokens=3724
exit=0
```

`env -u OPENAI_API_KEY` was needed because of the first attempt, below.

## Key handling

- **First attempt: HTTP 401.** Windows has a different `OPENAI_API_KEY` saved at user level, and OpenAI rejected that key. The client reads the environment before `.env`, so it sent the older one. The client handled the failure cleanly: a fixed message, no key shown, no tool called, the server closed.
- **Second run.** The variable was cleared for that one process, so the client used the key in `.env`. The user-level Windows variable was not changed.
- **After the run.** A search of `scratch/`, `state/` and `evidence/` for a 20-character fragment of the key found 0 files.
- **What the client does with keys:**
  - it starts the server with every `*_API_KEY`, `*_KEY`, `*_TOKEN` and `*_SECRET` variable removed, plus any variable whose value contains the key;
  - it redacts key-like text in tool results before they reach the model;
  - it never follows redirects;
  - it only allows plain http to localhost.

## Tests and review

- `python -I -B -m unittest discover -s clients -p "test_openai_mcp.py"`: 43 tests pass. They use a fake OpenAI server and the real Broadsword binary, and never touch the network.
- **Reviewer (Opus), first pass: FAIL.** The server inherited `OPENAI_API_KEY`. With `run` allowed, a command that echoed the environment put the full key into the model's context and into Broadsword's job log on disk. The commit also included `.pyc` files with absolute paths. Minor items: redirects carried the Authorization header, odd arguments caused tracebacks, an unknown `--allow` name only warned, Ctrl+C was slow, and tests left temp files outside `scratch/`.
- **Second pass: FAIL.** The key still leaked to the job log when it was stored under another variable name.
- **Third pass:** the orchestrator read the fix diff. Variables are now dropped by value as well as by name.
- **Correction (2026-11-18, full audit S10):** this third pass was not an independent review. The secrets fix merged with no independent PASS, against the rule that an independent reviewer checks work before it merges. The same area (child environments and secrets) was re-examined in the audit, and is being fixed through the merge queue (audit finding S6).

## Not covered

- Only one model per provider, and one run each.
- The tool results sent to OpenAI include build output and local paths such as `C:\Users\setup\...`. This is expected for this tool and was approved with the run.
- Milestone 4 (local model) is optional and not started.
