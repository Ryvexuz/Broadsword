# Ideas for later

Ideas outside the current stage go here, not into code. Each one names the stage it belongs to.

- **Stage 1:** `broadsword build/test/check` now always use the main root's `build/`, but plain `cargo` in a worktree still makes its own build folder. The fix is for agents to always build through `broadsword`. That could become a rule in `AGENTS.md` at the Stage 1 checkpoint.
- **Stage 2:** fix or document the MCP minors in `evidence/stage-2-m2-headless-mode.md`. The most important: `run`'s command array loses element boundaries through `cmd /C` (quote each element, or document shell semantics in the description), and a cancel for a queued, not-yet-running call is ignored.
- **Stage 2 (lesson focus):** models picked `build`+`test` over `check` unless the prompt said "in one job". Try rewording `check`'s summary, for example "the one call to build and test a project", and measure tool choice over several runs. This is the stage's "rewrite descriptions and measure" experiment.
- **Stage 2/3:** `broadsword build|test|check broadsword` always builds the main checkout, so a coder can't build its own worktree's changes through Broadsword. That breaks the new "Build through Broadsword" rule for Broadsword's own code. Option: a `--worktree` flag, or resolve the `broadsword` project to the checkout the command runs from.
- **Stage 1:** `broadsword build broadsword` relinks `build/debug/broadsword.exe`, the same binary the pause hook runs, while it may be in use. Consider installing a copy of the binary for the hook, for example `build/bin/broadsword.exe`, updated only after a passing check.
- **Stage 1:** the pause hook's allow-list is a plain substring match, so `rm x; echo broadsword status` gets through while paused. Fine for agents that cooperate, not a guard.
- **Stage 1 (recovery):** prune old `refs/broadsword/*` refs, for example once a job's record is `done` and older than N days. Make ignored-file capture incremental, since 20k small ignored files cost about 29 s per job. See the known limitations in `evidence/stage-1-m4-recovery.md`.
- **Stage 3:** worktrees made by the subagent tool started on the Stage 0 commit, not `main`'s tip, in milestones 1, 3 and 5, so the coder had to `git merge --ff-only main` first. Worktree-per-agent (Stage 3, milestone 2) should create each worktree from `main`'s current tip.
- **Stage 3:** Claude Code role files accept `isolation: worktree`, which gives a subagent its own temporary Git worktree. Worth testing for the coder role when the worktree-per-agent milestone starts.
