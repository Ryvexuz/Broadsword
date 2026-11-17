# Ideas for later

Ideas outside the current stage go here, not into code. Each one names the stage it belongs to.

- **Stage 1 (job runner):** `.cargo/config.toml` now sets `target-dir = "build"`, but Cargo resolves that per worktree, so an agent in a worktree that runs plain `cargo` gets its own build folder. Builds should go through a `broadsword build` command that sets `CARGO_TARGET_DIR` to the main root's `build_dir`.
- **Stage 1:** the pause hook's allow-list is a plain substring match, so `rm x; echo broadsword status` gets through while paused. Fine for agents that cooperate, not a guard.
- **Stage 3:** Claude Code role files accept `isolation: worktree`, which gives a subagent its own temporary Git worktree. Worth testing for the coder role when the worktree-per-agent milestone starts.
