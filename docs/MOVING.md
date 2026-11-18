# Moving Broadsword to another drive or PC

Broadsword is portable by design: every path is relative to the project root, and the pause hook finds the root through Git. A few things are tied to one machine, so after a move, do these once.

## Before unplugging

1. **Nothing running.**
   - `broadsword task list --status running` is empty (or only tasks you deliberately leave for later).
   - `broadsword queue list` has no `running` entry.
   - `broadsword jobs` shows nothing `running`.
2. **Everything committed.**
   - `git status` is clean in the root, `projects/crash-sim` and `projects/render`.
3. **Backed up.**
   - Push the three repos (see the commands in `PROGRESS.md`, under Backups).
   - An external drive is a single copy that travels, so don't skip this.

## On the new PC (or at the new drive letter)

1. **Install the tools:** Rust (stable, MSVC on Windows), Python 3, Git, and Claude Code.
2. **Repair the worktrees.** Git stores worktree locations as full paths. In the root, `projects/crash-sim` and `projects/render`, run `git worktree repair`, then `git worktree prune` to drop any that no longer exist.
3. **Point the reviewer at this PC's Claude CLI.** Set `BROADSWORD_REVIEWER_CLI` to its full path. Find it with `where claude` or under `%LOCALAPPDATA%`. It is never written into a file.
4. **Rebuild and install** from the root:
   - `cargo build --offline` (the one bootstrap exception to "build through Broadsword");
   - then copy `build/debug/broadsword.exe` to `scratch/bs-boot.exe`;
   - then `scratch/bs-boot.exe build broadsword`;
   - then `build/debug/broadsword.exe install`.
   - Then run `build/bin/broadsword.exe test broadsword`, which should show all tests passing.
5. **Restore the playbook.** Run `python -I scripts/sync_playbook.py`. It writes the playbook into this PC's `~/.claude/AI_DEV_PLAYBOOK.md`. Then copy `state/claude-memory/CLAUDE.md` to `~/.claude/CLAUDE.md` if that file doesn't exist yet.
6. **Restore Claude's notes.** Claude Code keys its project memory to the folder path, so at a new path it starts empty. The notes from the old PC are in `state/claude-memory/`. In the first session, ask Claude to read them and save them into its memory again.
7. **OpenAI through ChatGPT:** run `npm install -g @openai/codex`, then `codex login` (use `--device-auth` if no browser opens). It is needed for the `codex` reviewer and benchmark backend.
8. **The `.env` file** moves with the drive. If this PC has its own `OPENAI_API_KEY` in its environment, the OpenAI client warns when that key and the `.env` key differ.

## Machine-dependent settings

- `ram_warn_percent` is a percentage, so it scales from a 16 GB PC to a 128 GB one. `task start` will refuse new tasks above it once the RAM-gate task is merged; until then, the orchestrator checks it by hand.
- `disk_floor_gb` and `job_disk_reserve_gb` are absolute numbers for the drive that holds the project. Check they suit the external drive.
- `max_agents` (6) is the ceiling. With more RAM, it can go higher; raise it in `broadsword.toml`.
