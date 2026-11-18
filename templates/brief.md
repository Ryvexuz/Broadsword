# Brief: <one-line task title>

<!-- Fill every section; delete none. A coder gets this file and nothing else about the task. Keep the task to one or two files when the code allows it: small exact tasks cost 3-5x fewer tokens (RETRO.md). -->

**Task:** <what to build, in one or two sentences, in the plan's own words>

**Claim (the only files you may change):** <path>, <path>
Do not edit any other file. If the task needs one, stop and report it.

**API you may use:** <for example: anything `pub` in src/lib.rs (for example X, Y)>. <!-- A list is either marked "for example" or is the complete rule. -->

**Requirements:**
- <behaviour, with exact formats and numbers>
- <the hard cases: what must never happen, edge inputs, failure behaviour> <!-- SPEC_CHECKLIST.md risk table: name each one that applies. -->

**If a check genuinely fails:** do not loosen it. Report the configuration and numbers.

**Build route:** say how long the full suite takes, and that the coder runs it in the foreground and reads the result (never "run it in the background": a subagent cannot wait for its own background job).

**Done check (each line is a command and what it must show):**
1. `broadsword test <project> --task <id>` → `test: ok`, including the new tests <name them if known>
2. `broadsword run <project> --task <id> -- <command>` → <expected output, exit code>
<!-- Write it so it can only be read one way. If a reviewer could read it two ways, say which one is meant. -->
