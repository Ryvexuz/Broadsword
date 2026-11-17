# Lab log

One entry per session, newest first. At the end of each stage, three lessons for that stage go at the top.

## 2026-11-17 — Stage 0, Setup

**Models and roles**

- Claude Fable 5.2 as the orchestrator (main session). It did all of the setup itself; no subagents ran. `AGENTS.md` names Opus for this role, so this session did not match it.
- Claude Opus 5.5 in one fresh headless session, used only for the done check.

**What went wrong and what fixed it**

- The done check took three runs of a headless Claude Code session. The first produced no output for more than three minutes and was stopped by hand. The second, with `--strict-mcp-config` added to skip connectors, started and began its answer, then stalled until a 100 second limit cut it off. The third, with output written to a file instead of a pipe, finished in about 7 seconds and passed.
- The cause of the two stalls was not found. Two things changed between the runs (connectors skipped, output to a file), so it isn't known which one mattered, or whether it was a passing slowdown. Worth knowing before Stage 2, which runs builds through headless sessions.
- "TokenWise kept, task-router removed" is not done. The fresh session's startup record still lists `anthropic-skills:task-router`. It is an account-level skill, stored outside this repo, so nothing in the repo removes it. Nothing named TokenWise was found on this machine or in the plugin catalog. Both need a decision from Lazlo.

**Rough cost and time**

- About 25 minutes, 10:09 to 10:33.
- The done-check session reported $0.012. The main session's cost was not measured; no cost figure was available to it.

**Git commands used, beyond a plain commit**

- `git init -b main`: turned the folder into a Git repository so every later change has history. `-b main` names the first branch `main`; this Git version would otherwise call it `master`.
- `git add -A`: staged every new file for the first commit. `.gitignore` kept the contents of `scratch/`, `build/` and `shared/` out of it.
- `git status --short` and `git log --oneline --stat`: read-only checks of what was staged and what the commit holds.

**One lesson about AI development**

The rules file did change behavior: a fresh session with no tools gave the disk floor, the agent cap and the first task from `AGENTS.md` alone. But the same check showed what the rules file does not reach. task-router sits in the account, outside the repo, and it was still loaded. Check a setup from the fresh session's side, by reading what it loaded, not by reading the files that were meant to configure it.
