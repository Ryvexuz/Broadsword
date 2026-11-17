# Stage 1, milestone 5: tool registry — done check

**Check:** every capability is a named tool with inputs and outputs. List all tools from the command line.

**Result: PASS**, 2026-11-17, on `main` at merge commit `deb0cc6`.

## Command and output

```text
$ build/debug/broadsword.exe tools                     # exit 0
status    Show free disk, RAM and CPU against the limits, and whether Broadsword is paused.
          inputs: none; flags: read-only
cleanup   Free disk space now: old scratch files, oversized logs, and old build artifacts if cargo-sweep is installed.
          inputs: none; flags: none
watch     Keep watching free disk and clean up on a schedule (runs until stopped).
          inputs: none; flags: long-running
resume    Lift the low-disk pause once there is enough free space.
          inputs: force: boolean (optional); flags: none
projects  List the projects Broadsword can build, test and run.
          inputs: none; flags: read-only
build     Run `cargo build` on a project, with the disk guard and a Git checkpoint.
          inputs: project: string; flags: long-running
test      Run `cargo test` on a project, with the disk guard and a Git checkpoint.
          inputs: project: string; flags: long-running
check     Build, then test, a project in one job (stops at the first failure).
          inputs: project: string; flags: long-running
run       Run any shell command inside a project, protected by a Git checkpoint.
          inputs: project: string, command: string-array; flags: long-running, changes-files
recover   Clean up after a crash: resume interrupted builds/tests, roll back interrupted runs.
          inputs: none; flags: long-running, changes-files
abandon   Give up on one interrupted job without touching any project file.
          inputs: id: string; flags: none
jobs      List every job record and its status.
          inputs: none; flags: read-only
tools     List these tools, or describe one in full.
          inputs: name: string (optional), json: boolean (optional); flags: read-only

$ build/debug/broadsword.exe tools --json | python -I -c "...json.load, name and schema checks..."
13 tools, valid JSON
['status', 'cleanup', 'watch', 'resume', 'projects', 'build', 'test', 'check', 'run', 'recover', 'abandon', 'jobs', 'tools']
MCP name and schema shape OK
```

`broadsword tools <name>` shows one tool in full: its description, CLI form, inputs, outputs, exit codes and flag meanings. `--json` prints MCP `tools/list` entries (`name`, `description`, `inputSchema`, plus `annotations` with the read-only, destructive and idempotent hints), with Broadsword's extras under `_meta.broadsword`. A Stage 2 server wraps the array as `{"tools": [...]}`.

## Design

- **One table, one dispatch.** `crates/broadsword/src/tools.rs` holds a static table that the CLI dispatches from. The CLI syntax and all aliases are unchanged.
- **The list can't drift.** A test checks that every registry entry is reachable from its CLI form, and that every CLI command except `hook pre-tool` maps to an entry. `hook pre-tool` is plumbing, not a tool.
- **Exit codes are tested on the real binary.** `tests/tools.rs` checks each documented code, except exit 4 (killed for low disk), which `tests/jobs.rs` covers.

## Tests

The full suite passes on `main`: 108 tests (59 unit, 15 CLI, 10 jobs, 13 recovery, 11 tools). Clippy runs with `-D warnings` and is clean.

## Review

The reviewer (Opus) failed two passes. All the findings were about descriptions an AI would act on wrongly:

- `run` documented exit 3 as "refused", but it passes the command's own code through.
- `cleanup` claimed it ran `cargo sweep`, which isn't installed.
- The "changes files" flag was undefined and contradicted itself.
- Two outputs texts said build output goes only to the log.
- A test could never fail, because it searched its own source for the strings it looked for.

The third pass was a PASS.

## Not covered

- No MCP server yet. That is Stage 2; this milestone only shapes the JSON for one.
- `tools` needs `broadsword.toml` to load, so it exits 1 outside a Broadsword folder.
