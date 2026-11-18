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
  - **Shadow second reviewer** (`bb7375c`, Lazlo's choice of shadow mode): gpt-5.6-sol reviews the same prompt through `clients/openai_review.py`. Its verdict, blocking lines, tokens and time are recorded in the entry and in `<entry>.shadow.review`, and it never gates a merge. `scripts/shadow_report.py` compares the two reviewers. A live call was checked (key from `.env`, gpt-5.6-sol correctly FAILed a broken `add`). The first real shadowed merge is still to come.
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
- **Tests:** 422 Broadsword tests pass. crash-sim has 54 tests (main `abe471c`). The renderer has 77 (master `30c92c4`).
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

Full tables are in `evidence/benchmarks-2026-11-18.md`: hard case blind re-scored, single-bug cases, coding, and service tiers. In brief:
- **Hard review case** (12 bugs): Sonnet 5.5 at high effort caught 10. gpt-5.6-sol, gpt-6-sol, gpt-6.1-sol, gpt-6.2-sol, gpt-6-astra, gpt-6.2-astra, gpt-6.1-terra, Fable, Opus 5.5 and Sonnet 5.5 at default caught 9 each. The older Claude 4.5 models caught 4 to 6, and the GPT-4 generation 0 to 2. No model flagged a decoy.
- **Coding** (97 hidden tests):
  - 100%: Haiku, Sonnet, Opus at default, Fable at high, and gpt-6.1-terra;
  - 80%: Fable at default, Opus at high, and o3;
  - older OpenAI models 26% to 66%.
- **Consistency is untested.** One model scored 5/5 and then 4/5 on the same set. Repeat runs come next.

## Unfinished

- **Benchmark runs stopped at session end** for the move. Their results so far are in `state/bench-results/`. The Claude run had reached the older Claude 4.5 models (reviewer step for claude-sonnet-4-5). Re-run whatever is missing; the OpenAI rows should go through Codex once the benchmarks have a Codex backend.
- **Merge two finished tasks** once the benchmark runs no longer use their worktrees:
  - the hard case `t-20261118-075707`: fix the joined `catch` regex that crashes the old `score.py`, then send it through the queue;
  - the coding benchmark `t-20261118-080157`: send it through the queue.
- **Stage 4 v1:** run the stage-end audit, fix its blockers, then tag.
- **An OpenAI reviewer backend for the queue:** choose a model from the re-scored benchmarks; gpt-5.6-sol is a candidate.
- **Planned experiments:** the reviewer canary, and the handoff test from this file alone.
- **`IDEAS.md`:** the Opus minors from tonight's merges, plus queue improvements: `queue run --all`, attributes from the reviewed commit, and skipping the main re-check when main is exactly the checked commit (a speed-versus-safety decision for Lazlo).
- **Not running yet:** automatic `watch`, phone alerts and `cargo-sweep`. These need Lazlo's OK.

## Throughput backlog (Lazlo approved all on 2026-11-18; briefs in `state/backlog/<task id>.md`)

1. `t-20261118-095350`: queue checks scoped to the diff (Python-only diffs skip the Rust suite). **Parked, not merged.** Built and tested on its branch (fix commit `da0108e`). Opus failed both reviews on the done check: no real before and after timings, which can only be measured once it is installed. Opus also found a real minor: a child process that keeps the stdout or stderr pipe open stops the light-check timeout from firing, and a `try_wait` error returns without killing Python. Next: fix those two, then measure with the branch's binary on a throwaway queue in a scratch root (docs-only branch against a Rust branch), and queue it again with the timings in the brief. Its binary needs the two `light_check_*` keys, so it can't be installed before the merge.
2. `t-20261118-095350-2`: after a fast-forward to the checked commit, a quick build on main, with the full suite hourly (Lazlo's middle ground).
3. `t-20261118-095350-3`: the build lock covers compiling only; tests run from copied binaries.
4. `t-20261118-095351`: `queue run --all`, with the next entry's check overlapping the current review.
5. `t-20261118-095351-2`: test binaries run in parallel (after 3).
6. `t-20261118-095351-3`: a price comparison for paid speed tiers. **No fast tier and no Claude fast mode until Lazlo decides on it.**

8. `t-20261118-135124`: adjudicate shadow-only review findings (Lazlo's rule: look into every bug the shadow finds that the gating reviewer missed, and measure how right it is). Python only; brief ready.

7. **Use the ChatGPT allowance:** the shadow wrapper tries Codex first, then the API (`t-20261118-100744`; queue outcome: merged as `92d5c7c` after one fix round (Opus FAILed the first on a Windows process-tree timeout, a doubled time budget and unparseable output reported as success). A real smoke call answered with `backend: codex` on the ChatGPT plan; the first shadowed merge through Codex is the next one. Known minor: `OPENAI_API_KEY` still reaches the codex child's environment). The Codex CLI is installed and signed in with ChatGPT, and `codex exec` reviews through gpt-5.6-sol, terra and 6.2-sol work on the plan, not API credits. Next: a `codex` backend for the shadow reviewer and the benchmarks (see IDEAS), then re-run the credit-blocked OpenAI rows. On a new PC: `npm install -g @openai/codex`, then `codex login` (or `codex login --device-auth`). Separately, Lazlo will check the Platform billing after the move.

Process items:
- The orchestrator gets out of the critical path: a Sonnet drafts briefs from a short spec, the Housekeeper writes records while the pipeline runs.
- On the 128 GB PC, raise `max_agents`; the RAM gate is a percentage, so it adapts.

## Failed (tonight, all recorded in RETRO.md and LAB_LOG.md)

- **The orchestrator moved main during queue runs three times,** each time making an entry blocked or stale. Rule: touch main only when the queue is idle; backlog briefs now live outside Git.
- **The orchestrator mislabelled its own code** ("sonnet-orchestrated-by-opus") to get an Opus review. The permission classifier denied it, correctly. The label stays honest; Haiku reviews the orchestrator's code.
- **Five Haiku FAILs on a read-only Python script,** the last two shown wrong. Merged directly on Lazlo's rule ("If you're confident it's wrong just take the file"), and recorded.
- **The orchestrator's background wait loops raced each other** and lost with "queue busy". Replaced by one driver. `queue run --all` is in IDEAS.
- **Two benchmark runs started with wrong paths** (a relative results folder, and diffs missing from a worktree). Caught before any results were trusted.

## Disk

127.6 GB free against the 20 GB floor, measured at 09:15 UTC on 2026-11-18. The build folder is 338 MB.

## Next step

0. **After the move:** on the 128 GB unified-memory PC, first check the CPU/GPU memory split (see the section on it in `docs/MOVING.md`). Then follow `docs/MOVING.md` (worktree repair, `BROADSWORD_REVIEWER_CLI`, rebuild and install, the playbook and memory, `codex login`).
0b. **Backfill the shadow reviews (Lazlo said yes, 2026-11-18):** re-run the shadow review through Codex (gpt-5.6-sol, `clients/openai_review.py`) on tonight's four gated entries, whose shadow calls all ERRORed on the empty API credits: `q-20261118-100920` and `q-20261118-101835` (tp1, branch `task/t-20261118-095350`), `q-20261118-102429` (Codex wrapper, `3b67777`) and `q-20261118-103743` (its fix, `92d5c7c`). The queue keeps only the reviews, not the prompts, so rebuild each prompt from the entry's brief and the diff of its base against its reviewed commit, the same way `queue.rs` builds it. Save the results as `<entry>.shadow-backfill.review`, then judge every shadow-only finding (see `SPEC_CHECKLIST.md`, shadow-only findings) and record the verdicts.
**Lazlo's order (2026-11-18): clear the whole backlog first, then push on through the stages.** "There is an entire mountain behind this small hill." Run coders in parallel wherever the claims allow.

1. **Clear the backlog**, in roughly this order:
   - tp1 scoped checks: fix the pipe-holding timeout, measure with the branch binary in a scratch root, then requeue;
   - the Codex-wrapper minors;
   - tp2 main re-check, tp3 build lock compile-only, tp4 `queue run --all` with overlap, then tp6 parallel tests (after tp3);
   - `t-20261118-135124` shadow adjudication;
   - merge the two benchmark tasks (hard case and coding);
   - a Codex backend for the benchmarks;
   - the tp8 price comparison. It only informs Lazlo; Claude fast mode stays off regardless.

   - `t-20261118-141428`: GPT reviewers' 43 extra findings on the single-bug cases, blind-labelled:
     - 21 real bugs, 11 hardening, 4 restating the known bug, 6 false, 1 unsure, and 0 style;
     - 5 of the false ones were caused by the harness leaving `broadsword.toml` out of the diffs.
     Check which bugs are still on main and file fixes.
   - `t-20261118-141428-2`: cost to completion per model (Lazlo's rule: total cost until done, not per call).
   Then the benchmark follow-ups: the credit-blocked OpenAI rows through Codex, Responses API models, per-model token caps, 5x consistency on the gpt-6.2 models, an instruction-following test, a drift test, and labelling the extra single-bug lines.
   **Coder contest lineup (Lazlo, 2026-11-18):** gpt-6.1-terra on "ultra", gpt-6.1-sol at high and Opus 5.5 at high, all against Sonnet as the baseline. All three go through the coder bench (`--retries 3`, then a review step) and then a real-task trial, ranked by cost to completion and how rarely the reviewer flags them. Check what "ultra" maps to in Codex before running; if it's a paid speed tier, it waits for the price comparison.
   **astra works in short bursts** (Lazlo: it drifts on long tasks): give it sections, not the whole project in one go. Split the full sweep into 2 or 3 sections, say crates/core, queue and tools, and scripts/clients, each in its own Codex call, plus a short cross-cutting pass over the rules and records. Compare the result against one whole-project astra run to see whether the drift is real.
   **astra as a merge reviewer** (Lazlo, 2026-11-18):
   1. Check that `codex exec -m gpt-6.2-astra` works on the plan; tonight's Codex checks covered sol, terra and 6.2-sol only.
   2. Lazlo used astra through ChatGPT yesterday, so it should run. Run **both** gpt-5.6-sol and gpt-6.2-astra as shadow reviewers on every merge, as a contest for the reviewer job (task `t-20261118-142614`, after tp1/tp2 and the adjudication task).
   3. After about 15 to 20 shadowed merges with verdicts, compare with the gating reviewers (Haiku for low risk, Opus for high):
      - real bugs caught;
      - false alarms that would have blocked a merge;
      - time;
      - cost to completion.
   4. If astra holds up, try it as the gating reviewer for low risk first. High risk keeps an Opus gate until the numbers say otherwise.

   **Then move work to GPT** (Lazlo, 2026-11-18): this keeps the Claude plan for orchestration. Once the shadow verdicts and the benchmark re-runs show which GPT models to trust:
   - add a `codex` coder route: `codex exec` with write access inside the task's own worktree only, on the ChatGPT allowance;
   - trial it on one real low-risk task, with Claude as the gating reviewer;
   - compare it with a Sonnet coder on quality, review rounds, time and Claude tokens saved;
   - widen it if the trial holds up. Record it in the RETRO ledger.
2. **Stage 4 v1:** a trial of gpt-6.2-astra as a second whole-project sweep (Lazlo, 2026-11-18). It runs through Codex on the plan, beside the Claude auditor that didn't build the code, on the same snapshot. Every finding from either gets an independent verdict. Compare real finds, false alarms, unique finds, tokens and time, and run astra twice to see whether it's consistent. If it holds up, it becomes a standing part of every full sweep. Lazlo notes astra is much cheaper to run than Fable, so also compare cost to completion against Fable. If astra matches Fable's real finds, it can become the main auditor, with Fable used on a rotation or spot checks; that decision falls under the self-optimization rule. Then the stage-end audit, its blockers fixed, then the tag (and the pending Stage 0 tag).
3. **Then onward through the stages** in `DEVELOPMENT_PLAN.md`, with the crash sim as the first workload. Each stage gets its done check written before work starts.
