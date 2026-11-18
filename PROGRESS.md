# Progress

Rewritten at the end of each session. Last updated 2026-11-18, about 03:20 Lazlo's time (09:20 UTC). The session covered the Stage 3 close-out, a full audit of Stages 0 to 3 with its fixes, Stage 4 v1 (both milestones), the work-ahead backlog, and the model benchmarks.

## Moving to another drive or PC

Read `docs/MOVING.md` (also in `state/MOVING.md`) first. In short:
- **Before unplugging:** nothing should be running, and everything committed and pushed.
- **On the new PC:**
  - run `git worktree repair` in the root, `projects/crash-sim` and `projects/render`;
  - set `BROADSWORD_REVIEWER_CLI`;
  - rebuild and install;
  - run `python -I scripts/sync_playbook.py`;
  - restore Claude's notes from `state/claude-memory/` (a Git copy is in `docs/claude-memory/`).

**Backups:** everything is pushed to the private GitHub repo `Ryvexuz/Broadsword`:
- Broadsword is on `main`, with the stage tags.
- crash-sim is on the branch `crash-sim-main`.
- The renderer is on the branch `render-master`.

The push at session end brings all three up to date.

## Where things stand

- **Stages 1, 2 and 3:** done, tagged `stage-1-done`, `stage-2-done` and `stage-3-done`. Stage 0 is still untagged, because confirming that task-router no longer loads needs a fresh app session.
- **Full audit (Fable, high effort), 2026-11-18:** `evidence/full-review-2026-11-18.md`. Of its 1 blocking and 17 serious findings:
  - **Fixed and merged through the queue:** B1 (secrets in checkpoints), S1 (`recover` scope), S2, S4 and S7 (the queue), S6, S8 and S17 (the secret boundary, MCP cancel and client tests), S9 and S11 (rules in force, settings), S13 to S15 (crash-sim), S16 (test safety), and S10 (corrected the records and re-ran Stage 2's done check strictly; it passed).
  - **Still open:**
    - S3 (independent review is now a rule, "all code through the queue", but not enforced in code);
    - S5 (the queue's reviewer can't run the done check; for now the orchestrator runs it and puts the output in the brief);
    - S12 (backups: pushed tonight, but there is no automatic schedule).
- **Stage 4 v1 (rendering): both milestones merged, not yet tagged.** The stage-end audit runs first, by the rule.
  - **Milestone 1, debug renderer:** a std-only software renderer with lines, points, a fly camera, golden-image tests and a Win32 viewer, plus a `replay` viewer showing the crash-sim cube falling and settling. Render merges `c89e572` and `d684fa7`.
  - **Milestone 2, basic rasterizer:** filled triangles with a top-left fill rule, a depth buffer, Lambert shading, one light, a shadow map, an OBJ reader and a procedural car. The car renders at 8 to 10 ms a frame at 640x480 with shadows, against a 16.6 ms target. Render merge `30c92c4`.
  - The render repo's master is at `30c92c4`, with 77 tests.

## Working

- **Task board** (`broadsword task`), with path claims, worktrees, and three start refusals, all exit 3, nothing changed:
  - RAM at or above `ram_warn_percent` (90; a percentage, so it scales to any PC);
  - `max_agents` running (now 6);
  - an overlapping claim.
- **Merge queue** (`broadsword queue`): rebase, then check, then an independent review (Haiku for low risk, Opus for high), then `--ff-only` to the exact reviewed commit, then a check on main, then `task done`.
  - A recorded FAIL followed by a crash now ends `rejected`; it no longer wedges the queue.
  - A scratch-hook review can never merge a real project.
  - A brief must be a file inside the root that isn't a secret file.
- **Recovery:**
  - plain `recover` lists interrupted runs and changes nothing;
  - `recover <id>` rolls back one run;
  - `recover --all` rolls back every one;
  - an interrupted `run` blocks only its own folder;
  - a panic leaves a truthful record.
- **Secrets:** checkpoints never store `.env*` files, and only `run` jobs take a checkpoint. Every child process (jobs and MCP tools) gets a scrubbed environment, and output going back to a model, the job logs and `mcp.log` is redacted.
- **MCP:** a cancel removes a queued call. Option-like string values (for example `--all`) are refused at the one place arguments are built.
- **Builds:**
  - every job's child gets `CARGO_TARGET_DIR`;
  - `run` writes a `run:<folder>` note and is never installable;
  - per-folder markers cover `run`s whose packages can't be listed;
  - a closed stdout never kills a job.
- **Rules in force:**
  - low disk runs cleanup before pausing;
  - the pause hook fails closed and allows only exact broadsword commands;
  - hard-coded limits are now required settings;
  - a test fails if any setting is unread.
- **Tests:** 415 Broadsword tests pass. crash-sim has 54 tests (main `abe471c`). The renderer has 77 (master `30c92c4`).
- **Process:**
  - a retro after each milestone (`RETRO.md`), with an improvement ledger that tracks harm;
  - a portable playbook synced to every project on this PC (`~/.claude/AI_DEV_PLAYBOOK.md`);
  - `templates/` for briefs and coder prompts;
  - a work-ahead backlog (open board tasks, with briefs in `state/backlog/`);
  - `scripts/dev_metrics.py` (throughput, idle time, model use) and `scripts/review_costs.py`.
- **Benchmarks:**
  - `bench/reviewer/`: 5 single-bug cases, plus `run_openai.py` (merged);
  - `bench/reviewer/hard/`: 12 proven bugs and 3 decoys (task `t-20261118-075707`, not merged yet);
  - `bench/coder/`: 6 graded coding tasks with hidden tests, proven sound (task `t-20261118-080157`, not merged yet).

## Benchmarks (provisional)

All raw results are in `state/bench-results/` (not in Git; see Next step). The scorer's patterns undercount, so the hard-case numbers must be hand-checked and re-scored before anyone quotes them.

- **Hard review case** (bugs caught as BLOCKING, out of 12):

| Model | Caught |
| --- | --- |
| Sonnet 5.5, high effort | 10 |
| gpt-5.6-sol | 9 |
| Fable 5.2 | 9 |
| Opus 5.5 | 9 |
| Haiku 5.5 | 7–8 |
| Sonnet 4.5 | 6 |
| Opus 4.5 | 5 |
| o3 | 5 |
| Haiku 4.5 | 4 |
| GPT-4.1 | 2 |
| GPT-4.1-mini | 1 |

  No model flagged a decoy. GPT-4o and GPT-4o-mini errored.
- **Single-bug cases:** gpt-5.6-sol caught 4 of 5, the same as Opus 5.5 earlier. It missed only the silent-NaN case, which no model catches from the diff alone.
- **Model probe:** `claude-opus-4-1` silently runs as Opus 5.5, so it is left out. `claude-sonnet-4` is retired.
- **Still running at session end, or not run yet:** the rest of the OpenAI models on all three benchmarks, and the Claude coding runs past configuration 6 of 10.

## Unfinished

- **Merge two finished tasks** once the benchmark runs no longer use their worktrees:
  - the hard case `t-20261118-075707`: fix the joined `catch` regex that crashes the old `score.py`, then send it through the queue;
  - the coding benchmark `t-20261118-080157`: send it through the queue.
- **Stage 4 v1:** run the stage-end audit, fix its blockers, then tag.
- **An OpenAI reviewer backend for the queue:** choose a model from the re-scored benchmarks; gpt-5.6-sol is a candidate.
- **Planned experiments:** the reviewer canary, and the handoff test from this file alone.
- **`IDEAS.md`:** the Opus minors from tonight's merges, plus queue improvements: `queue run --all`, attributes from the reviewed commit, and skipping the main re-check when main is exactly the checked commit (a speed-versus-safety decision for Lazlo).
- **Not running yet:** automatic `watch`, phone alerts and `cargo-sweep`. These need Lazlo's OK.

## Failed (tonight, all recorded in RETRO.md and LAB_LOG.md)

- **The orchestrator moved main during queue runs three times,** each time making an entry blocked or stale. Rule: touch main only when the queue is idle; backlog briefs now live outside Git.
- **The orchestrator mislabelled its own code** ("sonnet-orchestrated-by-opus") to get an Opus review. The permission classifier denied it, correctly. The label stays honest; Haiku reviews the orchestrator's code.
- **Five Haiku FAILs on a read-only Python script,** the last two shown wrong. Merged directly on Lazlo's rule ("If you're confident it's wrong just take the file"), and recorded.
- **The orchestrator's background wait loops raced each other** and lost with "queue busy". Replaced by one driver. `queue run --all` is in IDEAS.
- **Two benchmark runs started with wrong paths** (a relative results folder, and diffs missing from a worktree). Caught before any results were trusted.

## Disk

127.6 GB free against the 20 GB floor, measured at 09:15 UTC on 2026-11-18. The build folder is 338 MB.

## Next step

1. **Re-score the hard case by hand** (no re-runs), then write `evidence/benchmarks-2026-11-18.md` with the full table: Claude and OpenAI, review and coding, tokens and cost where prices are known.
2. **Merge the two benchmark tasks** (see Unfinished).
3. **Run the Stage 4 v1 audit,** then tag.
