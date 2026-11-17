# Ideas for later

Ideas outside the current stage go here, not into code. Each one names the stage it belongs to.

- **Stage 1:** `broadsword build/test/check` now always use the main root's `build/`, but plain `cargo` in a worktree still makes its own build folder. The fix is for agents to always build through `broadsword`. That could become a rule in `AGENTS.md` at the Stage 1 checkpoint.
- **Stage 2:** fix or document the MCP minors in `evidence/stage-2-m2-headless-mode.md`. The most important: `run`'s command array loses element boundaries through `cmd /C` (quote each element, or document shell semantics in the description), and a cancel for a queued, not-yet-running call is ignored.
- **Stage 2:** `clients/openai_mcp.py` reads `OPENAI_API_KEY` from the environment before `.env`. If both are set and differ, warn (without showing either), so a stale key in Windows' user environment can't silently win, as it did on the first real run.
- **Stage 2 (lesson focus):** done. See `evidence/stage-2-description-experiment.md`: 16/16 runs chose `check` under both wordings. A follow-up worth doing: 20+ runs per condition with harder prompts, to test whether routing-first wording really cuts Claude's preamble calls. The variant is on branch `exp/check-routing-first`.
- **Stage 1:** `broadsword build broadsword` relinks `build/debug/broadsword.exe`, the same binary the pause hook runs, while it may be in use. Consider installing a copy of the binary for the hook, for example `build/bin/broadsword.exe`, updated only after a passing check.
- **Stage 1:** the pause hook's allow-list is a plain substring match, so `rm x; echo broadsword status` gets through while paused. Fine for agents that cooperate, not a guard.
- **Stage 1 (recovery):** prune old `refs/broadsword/*` refs, for example once a job's record is `done` and older than N days. Make ignored-file capture incremental, since 20k small ignored files cost about 29 s per job. See the known limitations in `evidence/stage-1-m4-recovery.md`.
- **Stage 3:** done in milestone 2. `task start --worktree` bases each worktree on the project's current main tip. The subagent tool's own worktrees still start on the Stage 0 commit. Once orchestration moves onto Broadsword's task board, agents should get worktrees from `broadsword task start --worktree` instead.
- **Stage 3:** prune `refs/broadsword/kept/*`, the orphan commits saved when a worktree is removed, once a task is old and its work is confirmed elsewhere. They accumulate, like the recovery refs.
- **Stage 3:** Claude Code role files accept `isolation: worktree`, which gives a subagent its own temporary Git worktree. Worth testing for the coder role when the worktree-per-agent milestone starts.
- **Stage 3:** the junction tests in `tests/buildsrc.rs` leave dangling junctions behind when their scratch root is deleted. They should remove each link before deleting the folder.
- **Stage 3:** `link_in_path` checks every parent up to the drive root, not only inside `root`. A reparse point above the project (for example a OneDrive-redirected Desktop, or an external drive mount) would make every clean refuse. Limit the check to `build_dir` and its parents inside `root`.
- **Stage 3:** `forget_notes` does nothing, silently, if `cargo metadata` fails. For `run`, the forget before the command should fail loudly.
