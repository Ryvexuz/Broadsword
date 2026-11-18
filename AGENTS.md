# BROADSWORD — Development Instructions

Nov 17, 2026 · @Lazlo

## What this project is

Broadsword is my own development ecosystem, built by AI agents. The real goal is **learning how AI-driven development works**: orchestrating agents, splitting up work, verifying results and managing cost. The software is the training ground.

Every area is in scope (rendering, 3D creation, physics, games, CAD, media), as small versions that are mine to learn from and adjust, not to match Unreal or Blender.

**First workload: the Rust car crash simulator.** Until it works, every Broadsword feature must help build it.

## Standing rules (every session)

- **Orchestration:** a strong model handles architecture, hard bugs, integration and major decisions. Cheaper models handle routine code, tests and docs. An independent reviewer checks work before it merges. All code reaches main through the merge queue, the orchestrator's own included.
- **Parallel work:** each agent works in its own Git worktree and branch. Don't wait on unrelated tasks. One agent owns merging.
- **Scope:** work only on the current stage. Ideas for later go in `IDEAS.md`, not into code.
- **Done means verified:** a stage is finished only when its done check passes and the evidence (commands and output) is saved. Passing tests is not the same as delivering the capability.
- **Testing:** run targeted tests while working and the full suite before merging. Fix causes, not symptoms. Never stub or fake anything to make a test pass.
- **Build through Broadsword:** build and test only with `broadsword build|test|check <project>`, never plain `cargo`. Other cargo commands, such as clippy, run as `broadsword run <project> -- cargo ...`. This gives every build the disk guard, the shared build folder and a job log, and every `run` a Git checkpoint (secret files are never stored in one).
- **Review:** plan for at least two review passes per milestone; a first-pass FAIL is normal. When a finding breaks a rule that must always hold, the fix enforces that rule in one place, not only for the reported case. At the end of every stage, before its tag, a strong model that did not build it audits the whole project: code, rules and the claims in the records against the raw evidence. Its blocking findings are fixed before tagging.
- **Resources:** follow the Disk rules below. Never delete work, history or failure evidence.
- **Ask first:** before spending money, using credentials or network services, publishing, or deleting anything outside the scratch area.
- **Retrospective:** before a milestone starts, read the last retro's "Carry into the next milestone" list and apply it. After each milestone, add a short entry to `RETRO.md`: what went wrong, why, and what changes so it doesn't happen again; and the same for speed and cost. Each change goes where it is enforced (Broadsword's code, a role file, `SPEC_CHECKLIST.md`), not only into the notes, and the next retro checks whether it worked or hindered anything (the improvement ledger in `RETRO.md`); a change that hinders is fixed or reverted. Lessons that apply beyond Broadsword go in its "Portable lessons" section; then run `python -I scripts/sync_playbook.py` so every project on this machine gets them (`--check` reports a stale copy). When retros keep coming back empty, this rule can be dropped.
- **Continuity:** commit often. End every session by updating `PROGRESS.md` and adding a `LAB_LOG.md` entry.
- Keep this file in the repo as `AGENTS.md` with a CLAUDE.md next to it containing one line, @AGENTS.md, so Claude Code and any other tool loads these rules at startup.

## Disk rules

- **Shared build folder:** every agent and worktree uses one build folder (one `CARGO_TARGET_DIR`, outside the repos). No per-worktree copies of build output.
- **Scratch area:** all temp files, logs and test output go in one `scratch/` folder. Anything in it can be wiped at any time, so nothing important ever goes there.
- **Automatic cleanup:** every hour, and immediately whenever free space falls below the disk\_floor\_gb setting (see Settings), run `cargo sweep` on the build folder, delete scratch files older than 24 hours, and cap log sizes. If cleanup can't get back above the floor, pause all agents and send a phone alert.
- **Git is the backup:** commit often and tag each verified stage. No zip snapshots. Remove merged worktrees with `git worktree remove`.
- **Learning Git:** when an agent runs any Git command beyond a plain commit, it explains in one line what the command did and why, in `LAB_LOG.md`.

## Files, shared copies and VMs

- **Claim before editing:** each task lists the files or folders it will change. The orchestrator never runs two tasks with overlapping claims at the same time. Overlapping tasks run one after another.
- **Own what you edit, share what you read:** source code is edited in the agent's own worktree. Big read-only things (models, datasets, dependencies, VM base images, the build folder) exist as one shared copy in `shared/` or `build/`. No agent makes a personal copy.
- **VMs are disposable:** keep one base image. Clone it for a task and delete the clone when the task ends. No VM outlives its job.
- **Secrets:** API keys live in a `.env` file listed in `.gitignore`. Never put keys in code, config or commits.

## Languages

- **Rust:** the core platform, job runner, engine, rendering and physics.
- **C/C++ libraries called from Rust:** where Rust is weak, such as CAD kernels (OpenCascade) and video (FFmpeg). Use them, don't rewrite them.
- **Python:** ML training and quick tooling scripts.
- **Studio interface:** to be decided when stage 11 starts.

Any other language needs a one-line reason in `LAB_LOG.md`.

## Settings

All limits and paths live in one file, `broadsword.toml`, at the project root. Change a number there and every agent picks it up. Nothing is hard-coded anywhere else.

```toml
# broadsword.toml
root = "."              # everything is relative to this folder
disk_floor_gb = 20       # pause and clean up below this much free space
cleanup_every_hours = 1
scratch_max_age_hours = 24
max_agents = 2           # raise only after clean merges
```

**Portable by design:** Broadsword must run from any drive or folder, so the project can move from the work laptop to an external SSD.

- Every path is relative to `root`. No absolute paths like `C:/Users/...` anywhere in code, scripts or config.
- The shared build folder, scratch area and shared models all live inside the project folder, so they move with it.
- After moving the project, run `git worktree repair` once, because Git stores worktree locations as full paths.

## Agent roles (Claude Code)

Each role is a file in `.claude/agents/` that sets its model and its job. The main session is the orchestrator: it plans, assigns tasks and merges.

- **Orchestrator** (main session, Opus): architecture, task splitting, file claims, merging.
- **Coder** (Sonnet): implements one claimed task in its own worktree.
- **Test writer** (Haiku): writes and runs targeted tests for a finished task.
- **Reviewer** (Opus, or a different model from the coder): checks the work against the done check before merging. Never reviews its own code.
- **Housekeeper** (Haiku): disk checks, cleanup, `PROGRESS.md` and `LAB_LOG.md` upkeep.

Model choice per role is set in these files. A model-routing mod may pick models for the main session only, so the two never conflict.

## Stage 1 — Foundation

**Goal:** a core that runs jobs.

**Build:** a project and task runner, a tool registry, a resource monitor, a test runner, and Git checkpoints with recovery. Start the shared base (math, meshes, scene, job system, file format), but only the parts the crash sim needs now.

**Done check:**

1. One command builds and tests the crash-sim repo.
2. If Broadsword is killed mid-job, on restart it resumes or rolls back cleanly and nothing is lost.
3. It refuses to start a job that would break the disk reserve.

**Test project:** before anything else, create a tiny starter crash sim in its own repo: a cube of nodes and beams that falls and bounces off the ground. It's what Broadsword builds and tests during stage 1.

**First session:** build only the resource monitor (disk, RAM, CPU checks, the disk floor and hourly cleanup). It protects the machine for every session after it. The rest of stage 1 comes later.

## Stage 2 — AI Integration

**Goal:** any AI can connect and use Broadsword's tools.

**Build:** an MCP server exposing the tool registry, a headless mode, and support for at least one cloud model and one local model.

**Done check:**

1. A fresh AI session that has never seen the project connects, lists the tools and runs a crash-sim build with no interface open.
2. Optional for now: the same task works with a local model. Stage 2 can pass with cloud models only.

## Stage 3 — Autonomous Development

**Goal:** agents work together without stepping on each other.

**Build:** a task board, one worktree per agent, subagents with model choice, a reviewer step and a merge queue.

**Done check:** two agents build two separate crash-sim features in parallel. Each is reviewed by a different model, both merge cleanly, and the reviewer catches at least one planted bug.

## Later modules (stages 4–12)

Each one starts small, and only when the crash sim or another real project needs it. Write its done check before work starts.

- **Rendering:** a real-time rasterizer, then a small path tracer for offline renders.
- **Physics:** the crash sim's node-beam solver, behind a swappable interface.
- **3D creation:** select, move and extrude mesh faces, with undo.
- **Games:** the crash sim grows into a racing game.
- **CAD:** shapes driven by dimensions and constraints.
- **Media:** a layered image editor, then a timeline.
- **Advanced:** added as real needs come up.
- **Studio:** one editor window that hosts every module. Everything must still work without it.
- **Integration:** real workflows tested end to end.

## Experiments to run along the way

- Cheap model vs strong model on the same task: quality, cost, time.
- Reviewer vs no reviewer: how many planted bugs get caught.
- Big tasks vs small tasks: where agents start drifting or faking success.
- Handoffs: can a new agent resume from `PROGRESS.md` alone?

## Session reports

**`PROGRESS.md`** (rewrite each session): what's working, what's unfinished, what failed, and the exact next step and file to resume from.

**`LAB_LOG.md`** (add one entry per session):

- Date and stage
- Models used and their roles
- What went wrong and what fixed it
- Rough cost and time
- One lesson about AI development

**Primary objective:** build the best working Broadsword possible, and learn the process while doing it. Don't build the most complicated process.
