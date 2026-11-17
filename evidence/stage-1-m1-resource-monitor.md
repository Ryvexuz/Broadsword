# Stage 1, milestone 1: resource monitor — done check

**Check:** fill scratch past the floor on purpose; cleanup runs and agents pause.

**Result: PASS**, 2026-11-17, on `main` at merge commit `0a08176`.

## Steps and output

Free space was 25,916,657,664 bytes against a 20 GB floor. A file of that minus 20 GB plus 600 MB was created in `scratch/`. It is brand new, so cleanup's 24-hour rule cannot delete it, and cleanup has to fall through to the pause.

```text
$ fsutil file createnew scratch/fill-done-check.bin 5070966784
$ build/debug/broadsword.exe status
disk: 19.4 GB free of 447.5 GB (floor 20 GB) [WARN]
paused: no

$ build/debug/broadsword.exe cleanup
ALERT: Broadsword PAUSED: free space 19.4 GB is below the 20 GB floor after cleanup
cargo-sweep not installed, skipped
freed 0 bytes in 0 deleted + 0 capped files
free space: 19.4 GB before, 19.4 GB now
PAUSED: still below the floor

$ cat broadsword.paused
time: 2026-11-17 16:52:21 UTC
free: 19.4 GB
reason: free space 19.4 GB is below the 20 GB floor after cleanup

$ tail scratch/logs/monitor.log
[2026-11-17 16:52:21 UTC] Broadsword PAUSED: free space 19.4 GB is below the 20 GB floor after cleanup

$ echo '{"tool_name":"Write",...}' | build/debug/broadsword.exe hook pre-tool
Broadsword is PAUSED (low disk). ...
hook exit=2
```

**A real agent was paused.** The orchestrator session then tried a Write tool call. Claude Code refused it:

```text
PreToolUse:Write hook error: [command hook]: Broadsword is PAUSED (low disk). Only read-only tools and `broadsword status|resume|cleanup` are allowed.
```

**Recovery:**

```text
$ rm -f scratch/fill-done-check.bin && broadsword resume
resumed (free space 24.1 GB)
$ broadsword status
disk: 24.1 GB free of 447.5 GB (floor 20 GB) [ok]
paused: no
```

## Tests and review

- `cargo test --offline` on `main`: 17 unit tests and 15 CLI tests passed, 0 failed.
- Reviewer (Opus), first pass: **FAIL**. Agents in worktrees never saw the pause, because each worktree has its own `broadsword.toml`.
- Reviewer, second pass on `ddab379`: **PASS**. A real `git worktree` blocks while the main root is paused (exit 2).

## Not covered

- The alert reaches stderr and the log only. `alert_command` is empty, so nothing is sent to a phone.
- `cargo sweep` was skipped because `cargo-sweep` is not installed. Installing it needs the network.
- `broadsword watch`, the hourly loop, was checked by reading the code only. It is not running on this machine.
