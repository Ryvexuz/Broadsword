# Ideas for later

Ideas outside the current stage go here, not into code. Each one names the stage it belongs to.

- **Stage 1:** point Cargo at the shared build folder with a `.cargo/config.toml` at the project root (`target-dir` is resolved relative to that file, so it stays portable and every repo and worktree under the project inherits it). Needs a way to keep it in step with `build_dir` in `broadsword.toml`, since paths are meant to live in one file.
- **Stage 3:** Claude Code role files accept `isolation: worktree`, which gives a subagent its own temporary Git worktree. Worth testing for the coder role when the worktree-per-agent milestone starts.
