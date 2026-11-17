# Progress

Rewritten at the end of each session. Last updated 2026-11-17, after the Stage 0 session.

## Where things stand

Stage 0 (Setup) has passed its done check but is not closed or tagged. One item is open and needs a decision from Lazlo. No Stage 1 work has started and there is no Rust code yet.

## Working

- The rules load at startup: `CLAUDE.md` holds one line, `@AGENTS.md`.
- Settings are in `broadsword.toml`: disk floor 20 GB, 2 agents, hourly cleanup, 24 hour scratch age, and the `scratch`, `build` and `shared` paths.
- The `scratch/`, `build/` and `shared/` folders exist. Git tracks each folder through a `.gitkeep` and ignores everything else in them.
- Four role files in `.claude/agents/`: `coder` (Sonnet), `test-writer` (Haiku), `reviewer` (Opus, cannot edit files), `housekeeper` (Haiku). A fresh session lists all four.
- Stage 0 done check: PASS. Evidence in `evidence/stage-0-done-check.md`.
- The plan is in `DEVELOPMENT_PLAN.md` and the long-term map in `CAPABILITY_ROADMAP.md`. Both are copies of the Claude Docs as of 2026-11-17.

## Unfinished

- **"TokenWise kept, task-router removed"** (Stage 0, item 2). Lazlo turned task-router off; its folder and its manifest entry were gone at 10:36. A headless session started from the running app still listed it, likely because it inherits the app's skill list from before the change. Confirm in a new session started from the app. Nothing named TokenWise was found on this machine.
- **Stage 0 close-out.** Once the item above is settled: write three lessons at the top of `LAB_LOG.md`, adjust `AGENTS.md` if a rule got in the way, and tag `stage-0-done`.

## Failed

- Nothing is failing. The first two done-check runs did not finish; `LAB_LOG.md` has the cause and the fix.

## Watch before Stage 1

- **Disk.** 24.2 GB free against a 20 GB floor leaves 4.2 GB. Rust build output will use that quickly, and below the floor the rules say to clean up and pause.
- **Network.** Cargo downloads crates from crates.io. `AGENTS.md` says to ask before using network services, so the first build needs a yes from Lazlo.

## Next step

1. Settle the open item and close Stage 0 (above).
2. Start Stage 1, milestone 1: the resource monitor. Read "Stage 1 — Foundation" in `DEVELOPMENT_PLAN.md` and "First session" in `AGENTS.md`. Build only the monitor: disk, RAM and CPU checks, the disk floor, hourly cleanup, pause and alert. Its done check: fill `scratch/` past the floor on purpose; cleanup runs and agents pause.
3. The first file to create is the root `Cargo.toml`. `IDEAS.md` has a note on pointing Cargo at the shared build folder.
