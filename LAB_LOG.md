# Lab log

One entry per session, newest first. At the end of each stage, three lessons for that stage go at the top.

## 2026-11-17 — Stage 1, milestone 4: recovery

**Models and roles**

- Claude Opus 5.5 as the orchestrator: spec, five review handoffs, merge, done check, records.
- Sonnet as the coder, in a Broadsword worktree, over five passes in one continuing agent.
- Opus as the reviewer, over five passes in one continuing agent.
- No test writer.

**What went wrong and what fixed it**

- This was the hardest milestone so far: five review passes, all on real ways to lose work, each reproduced by the reviewer in throwaway Git repos.
  1. A retried rollback overwrote later edits.
  2. Under `core.autocrlf=true` restored files weren't byte-exact.
  3. `.env` (ignored) was not protected.
  4. The ignored-file size cap could overwrite unsaved bytes, twice through different code paths.
  5. A file turned into a folder blocked recovery.
  6. Restoring the index dropped content staged after the kill.
- Patching each path separately failed twice for the same class of bug. What worked was changing the structure: one `Guard` that every working-tree write and delete passes through. It allows an action only on a missing path or on bytes already in this attempt's snapshot.
- The orchestrator also changed one design point: auto-recover no longer rolls back silently before an unrelated build. It refuses new jobs on that project until an explicit `recover` or `recover --abandon`.
- The done check ran on real projects: a file-changing job on `crash-sim` was rolled back, and a `check broadsword` killed mid-test was resumed.

**Rough cost and time**

- About 45 minutes of wall time. Subagent tokens: coder about 290k (cumulative over five passes in one agent), reviewer about 220k. This was the most expensive milestone. No dollar figure was available.

**Git commands used, beyond a plain commit**

These are used by Broadsword's code, explained here as AGENTS.md asks:

- `GIT_INDEX_FILE=<temp> git read-tree HEAD` / `update-index -z --index-info` / `write-tree` / `commit-tree`: build a commit of the exact working tree in a separate temporary index, so the real index, HEAD and branch are untouched.
- `git hash-object -w --no-filters`: store a file's raw bytes with no line-ending conversion.
- `git cat-file blob`: read those bytes back to restore a file exactly.
- `git update-ref <ref> <sha> ""`: create a ref only if it doesn't exist yet. Checkpoints can't overwrite each other.
- `git ls-tree -r`: list a snapshot's files and their blobs, to compare against what is on disk.
- `git read-tree <tree>` (no `-u`): reset the real index to a saved tree without touching any files.

Orchestrator: `git merge --no-ff`, `git worktree remove` and `git branch -d`, as in milestone 1.

**One lesson about AI development**

The coder fixed exactly the case the reviewer reported, each time, so the same class of bug came back through a neighbouring path. A review finding is a sample of a class, not the whole class. Asking for one function that enforces the invariant everywhere ended the loop where patching each path hadn't. The review loop was expensive but bought something real: each of the six data-loss bugs would have deleted someone's work.

## 2026-11-17 — Stage 1, milestone 3: job runner and test runner

**Models and roles**

- Claude Opus 5.5 as the orchestrator: spec, merge, done check, records.
- Sonnet as the coder, in a Broadsword worktree.
- Opus as the reviewer, over two passes.
- No test writer. The coder wrote both unit and integration tests, as in milestone 2.

**What went wrong and what fixed it**

- To test "kill the job when the disk runs low", the coder added a test-only environment variable to the real binary. It fakes the free-space reading from a file in `scratch/`. The reviewer showed that it could fake more space too, so any agent could switch off the disk guard. The fix caps every faked reading at the real free space. The lesson: a test seam must only be able to make the guard stricter.
- Same-second job logs overwrote each other, losing failure evidence. They now get `-2`, `-3` suffixes.
- Three new settings had silent defaults, against the rule that every value lives in `broadsword.toml`. They are now required keys.
- **The coder's worktree started on the Stage 0 commit,** not on `main`'s tip. The coder ran `git merge --ff-only main` inside its branch to catch up. The same thing happened in milestone 1. The worktree tool seems to branch from an older commit than `main`'s tip; worth checking before Stage 3.
- **Disk space jumped** from 23.9 GB to 128.5 GB free during the session. That was outside Broadsword, presumably Lazlo freeing space.

**Rough cost and time**

- About 15 minutes of wall time. Subagent tokens: coder about 250k over two passes, reviewer about 220k over two passes. No dollar figure was available.

**Git commands used, beyond a plain commit**

- `git merge --ff-only main`, run by the coder on its own branch: moved the branch forward to `main`'s tip without a merge commit, because the branch had no commits of its own yet.
- `git merge --no-ff`, `git worktree remove` and `git branch -d`: the same as milestone 1. A merge commit for the milestone, then removal of the merged worktree and branch.

**One lesson about AI development**

The reviewer failed all three Stage 1 milestones on their first pass, each time on a real problem the tests didn't catch:

1. The monitor ignored worktrees.
2. The sim failed silently.
3. The override could disable the guard.

Each fix was small. Budget for a second review pass as the normal case, not a failure.

## 2026-11-17 — Stage 1, milestone 2: starter crash sim

**Models and roles**

- Claude Opus 5.5 as the orchestrator. It wrote the spec, ran the done check and kept the records.
- Sonnet as the coder, working directly in the new `projects/crash-sim` repo. The repo was new and only the coder worked in it, so no worktree was needed.
- Opus as the reviewer, over two passes.
- **No test writer this time.** The spec told the coder to write physics tests (free fall against h0 − ½gt², energy never rising, bounce, settle, determinism). The reviewer checked that those tests drive the real sim. This was an experiment in skipping a role; see the lesson.

**What went wrong and what fixed it**

- The first version passed all 7 of its tests and its done check, and the reviewer still failed it. With too large a time step the sim blew up silently: it printed `inf`/`NaN` rows, reported 0% strain (`fold(0.0, f64::max)` drops NaN) and exited 0. The coder added a per-step finiteness and strain check that exits 1, and an up-front refusal of time steps past a stability limit computed from k and m.
- On the second pass the reviewer checked the stability bound by building the stiffness matrix. The true ω_max is 400 rad/s; the code's bound of 529 rad/s is safe.

**Rough cost and time**

- About 15 minutes of wall time. Subagent tokens: coder about 190k over two passes, reviewer about 180k over two passes. No dollar figure was available.

**Git commands used, beyond a plain commit**

- `git -C projects/crash-sim init -b main`: made the crash sim its own repository inside the project folder, as the plan asks, so it moves with the project. Broadsword's `.gitignore` excludes `projects/`.

**One lesson about AI development**

"Tests pass" and "the done check passes" both held for a program that would print nonsense and report success. The reviewer found it by trying inputs the coder never tried, rather than by reading the tests. Skipping the test writer cost nothing here, because the reviewer did the adversarial testing. The role that matters most is the one that tries to break the code.

## 2026-11-17 — Stage 1, milestone 1: resource monitor

**Models and roles**

- Claude Opus 5.5 as the orchestrator (main session). It wrote the task spec and claim, merged, and ran the done check.
- Sonnet as the coder, in its own worktree. Haiku as the test writer, in the same worktree. Opus as the reviewer, which cannot edit files.
- The role files in `.claude/agents/` couldn't be called by name: they were created after this session started, and a session reads them only at startup. Each subagent instead ran as a general-purpose agent on the role's model, with its role file as the brief.

**What went wrong and what fixed it**

- The reviewer failed the first version, on a bug that 28 passing tests missed. Each worktree has its own `broadsword.toml`, so a worktree agent checked its own pause flag and was never paused. The coder fixed it by following a worktree's `.git` file back to the main checkout. The reviewer then reproduced the original failure with a real `git worktree` and passed the fix.
- Still open: Cargo's `target-dir` is resolved per worktree, so worktree builds only share `build/` when `CARGO_TARGET_DIR` is set in the shell. The orchestrator set it in every subagent's brief. Noted in `IDEAS.md` for the job runner.
- The done check used a real 5 GB fill. The pause blocked the orchestrator's own Write call, which is the strongest evidence that agents pause.

**Rough cost and time**

- About 2 h of wall time, most of it waiting on subagents. Subagent token use: coder about 120k tokens over two passes, test writer about 92k, reviewer about 105k over two passes. No dollar figure was available.

**Git commands used, beyond a plain commit**

- `git merge --no-ff worktree-agent-…`: brought the reviewed branch into `main` as a merge commit, so the history shows the coder's, test writer's and fix commits as one milestone.
- `git worktree remove .claude/worktrees/agent-…`: deleted the coder's worktree folder now that its work is merged, per the disk rules.
- `git branch -d worktree-agent-…`: deleted the merged branch. `-d` refuses to delete unmerged work.

**One lesson about AI development**

A separate reviewer on a stronger model found the one bug that mattered. The coder's and test writer's tests all ran inside a single folder, so they tested the case the code was written for. The reviewer asked where agents actually run. Tests written by the side that wrote the code tend to share its blind spots.

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
