# Ideas for later

Ideas outside the current stage go here, not into code. Each one names the stage it belongs to.

- **Stage 1:** `broadsword build/test/check` now always use the main root's `build/`, but plain `cargo` in a worktree still makes its own build folder. The fix is for agents to always build through `broadsword`. That could become a rule in `AGENTS.md` at the Stage 1 checkpoint.
- **Stage 1:** `broadsword build broadsword` relinks `build/debug/broadsword.exe`, the same binary the pause hook runs, while it may be in use. Consider installing a copy of the binary for the hook, for example `build/bin/broadsword.exe`, updated only after a passing check.
- **Stage 1:** the pause hook's allow-list is a plain substring match, so `rm x; echo broadsword status` gets through while paused. Fine for agents that cooperate, not a guard.
- **Stage 1 (recovery):** prune old `refs/broadsword/*` refs, for example once a job's record is `done` and older than N days. Make ignored-file capture incremental, since 20k small ignored files cost about 29 s per job. See the known limitations in `evidence/stage-1-m4-recovery.md`.
- **Stage 3:** Claude Code role files accept `isolation: worktree`, which gives a subagent its own temporary Git worktree. Worth testing for the coder role when the worktree-per-agent milestone starts.
