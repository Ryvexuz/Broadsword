# Progress

Rewritten for the 2026-11-20/21 session, task t-20261121-023240, from raw evidence only: `state/pc-progress.md` (entries from 2026-11-20T00:00Z to 2026-11-21T02:32:54Z), `build\bin\broadsword.exe task list` and `queue list` (both read about 02:33Z on 2026-11-21), `git log` in the root and in `projects/crash-sim`, `render`, `numerics`, `foundation`, and `state/staging/morning-summary.md`. Main was 68a04c05 when written. Step-by-step log: `state/pc-progress.md` (outside Git). The live state is `queue list` and `task list`, not this file.

Rules used here: "merged" means `git merge-base --is-ancestor <sha> main` exited 0 in the repo named (checked 02:33Z), or the queue entry says merged. Anything I could not confirm says "unconfirmed". The previous PROGRESS.md (2026-11-20 09:15Z) is in Git history; its Stage 4 / merge-train / backup details were not re-checked here.

## Stage status

- **Stages 1-4 v1: done and tagged.** `git tag` in the root (02:40Z) lists stage-1-done 47bc94d9, stage-2-done 8abd8556, stage-3-done 6bb63475 and stage-4-v1-done 94f4d433; `projects/render` has a stage-4-v1-done tag. Evidence: `evidence/stage-4-v1-done.md` (file exists). There is no stage-0 tag (see Carried open items).
- **Stage 6 m3: done.** The nine checks 3.1-3.9 all PASS, merged as c8f94194 (queue q-20261121-002715, task t-20261119-171333-42; evidence `evidence/stage-6-m3.md`). Lazlo's lap (3.8) passed at 00:16Z: `scratch/s6-3-8.txt` (pc-progress 2026-11-21T00:16:25Z).
- **Stage 6 m4: NOT done, waiting.** Its done check waits on the validation car n20-08 (task t-20261121-001109-2), which waits on a sourced track width (see Open decisions). n20-10 (m4 done) waits on n20-08 (pc-progress 01:20:03Z).
- **Stage 6 m5: plan and format merged; the solver core has nothing merged.** The m5 plan (`docs/M5-SOLVER.md`) merged as d1758ae in crash-sim (q-20261121-004101, task t-20261121-001109, round 3). Lazlo approved the plan at 00:50Z. The bsn 3 format (solid and tet records) merged in crash-sim, 23c777f..c97b2e0 (n20-06a, task t-20261121-005119). Solver core n20-11 is running, Opus: t-20261121-021841 (`src/solver_m5.*`; no queue entry yet).
- **Stage 7 m1-m3 done checks merged** (morning-summary.md; q-20261121-021042 merged b3fcaa3 in crash-sim after 6 review rounds, task t-20261121-001109-6). The `--track` timed lap (n20-13, m1) merged in crash-sim as 2595f44 and 253b243 ( two valid laps 35.663 s and 35.461 s, pc-progress 02:04:36Z). `oval_pinch.track` merged as 71c9ca4 (crash-sim). m4 (t-20261121-014029) is running; m5 checks (t-20261121-014029-2) are open.
- **Other merged on 2026-11-20/21 (each checked as an ancestor of main):** done-check race close, cold_move with Windows handle-relative operations (ca806e69, b110b6e7); GPT cap counts only paid calls (a5ebcfb1); gate-prompt replay, anti-speculation rules DROPPED (d783f3f4); crash pulse report `--pulse-report` (f569c23, crash-sim); gravity >= 0 doc (904ff9b, crash-sim). Throughput recorded at 01:59Z: 1 h = 6, 3 h = 15, today 135 queue merges (pc-progress; I did not recount).
- **Nodes (sub-repos, git log only):** numerics N1-N10 commits through ed3d4f9 (N10 review fix); foundation up to 586b69f (trace docs); render up to 550a40c (text, drive viewer, golden frames). Whether each node's own done check passed: unconfirmed here.

## Queue and board (queue list / task list, about 02:33Z)

- **Queue (895 entries):** 254 merged, 232 rejected, 369 cancelled, 5 approved, 6 queued, 13 check_failed, 9 merged_check_failed, 4 perf_failed, 3 conflict.
- **Approved, waiting for a train:** q-20261121-010841 (t-20261120-223314), -011309 (t-20261121-001109-8), -011341 (t-20261121-005258), -011739 (t-20261121-003648), -022836 (crash-sim oval_pinch task t-20261121-015242; note 71c9ca4 is already on crash-sim main, so this status is stale or a different commit: unconfirmed).
- **Queued:** q-20261121-011825 (codex route t-20261120-092355), -011858 (t-20261120-193628-2), -011934 (t-20261121-010217), -015438 (rule B t-20261121-005119-3), -020123 (t-20261121-003922), -023252 (m4 t-20261121-014029).
- **Board (`task list` 02:33Z):** 277 done, 91 abandoned, 38 open, 17 running. "Running" includes tasks whose entry waits in the queue. `broadsword.toml` line 9 has `max_agents = 14`, with the comment that a running task whose queue entry is queued or running is not counted; `scripts/slot_check.py` (read_cap, evaluate) reads that key and prints board running N/cap, waiting in queue and active coders. pc-progress 2026-11-21T02:23:15Z: running 14/14, waiting 9, active 5-6, so the board count is not the coder count.
- **Old failures still listed** (not investigated in this rewrite): the 13 check_failed, 9 merged_check_failed, 4 perf_failed and 3 conflict entries in `queue list`. Examples with a stated cause: q-20261121-005207 (render n20-12) failed from the shared-build race (a missing crash-sim.exe); the re-check on main passed, 156 passed 0 failed (pc-progress 01:00Z).

## In-flight review loops (round counts from pc-progress and queue list)

- **Shared build race fix, t-20261120-191700:** 6 rejected queue entries (q-20261120-202642 ... q-20261121-011535). pc-progress 02:04:27Z calls the last one round 7 (bf888b07), rejected with a REAL blocking point. Lesson 41 applied: an Opus design change (drop the restore, key `main.bins` by project) was sent. No new queue entry yet. The count differs by one between sources: unconfirmed which is exact.
- **Codex coder route, t-20261120-092355:** 2 rejections (q-20261120-235345, q-20261121-004122), round 3 queued as q-20261121-011825 (commit cf9b6a82 per pc-progress 01:18:26Z). It holds `broadsword.toml`; the S5 re-land waits on it.
- **S5 secrets / linear re-land:** the old task t-20261119-171333-10 has 5 rejected or conflicting entries (last conflict q-20261121-003448). Re-landed as one squashed commit on t-20261121-011525 (open, waits for the codex route; branch `keep/s5-a730cf4c`).
- **STAGE7 m2-m3 (n20-16):** merged after 6 rounds (q-20261121-021042). m4 (t-20261121-014029): 1 queue entry, q-20261121-023252, a2454ca, not yet reviewed.
- **n20-08 validation car:** 1 rejection (q-20261121-010033, Opus: track width unsourced); round 2 (28719b17) is committed and parked, not queued.
- **n20-52 audit records (t-20261121-001109-8):** 1 rejection (q-20261121-003648), round 2 approved (q-20261121-011309), waiting for a train.
- **Direct-merge late review (t-20261121-003648):** commits 2d6a8bcb and 68a04c05 are on main (review: FAIL with 1 REAL on b9ae9e5, PASS on ec7a479); follow-up task t-20261121-004802. Its queue entry q-20261121-011739 shows approved, not merged: unconfirmed why the commits are already on main.
- **Rule B (shadow-finding judging, t-20261121-005119-3):** first entry q-20261121-005351 was blocked by an API safeguard flag (no verdict), cancelled and re-queued as q-20261121-015438. Wiring it into `scripts/session_end_check.py` is t-20261121-005351 (open, after rule B merges).

## Open decisions for Lazlo (state/staging/morning-summary.md "Needs you")

1. OK to look up the 2012 Accent's track width online for the validation car, or give a source? It is assumed 1.500 m, and the test report does not give it. Blocks n20-08 and so Stage 6 m4's done check.
2. OK for an online lookup for the price comparison of paid speed tiers (t-20261118-095351-3, open)?
3. The follow-up parallel-tests trial cannot run: one full test run takes longer than a 10-minute tool call, and the permission check refused a detached run (t-20261121-011900, abandoned, pc-progress 01:27:26Z). Allow detached measurement runs, or drop the trial? `test_parallel_binaries` stays 1 (the p3/p4 settings each had 1 failing run in 3; the two tests then passed 40 of 40 when run alone, 704478d8).

## Parked or held

- n20-08 round 2 (28719b17): unqueued until decision 1. The round-trip test t-20261121-012003 waits for n20-08.
- Held behind the S5 re-land t-20261121-011525 (claims queue.rs / tools.rs; pc-progress 2026-11-21T01:21:48Z), all open on the board: S3 t-20261119-171333-9, shadow order t-20261119-171333-15, T1 light check t-20261119-222633-2, train follow-ups t-20261120-030200, lock retry t-20261120-171453, affected packages t-20261120-174239, Haiku-low refusal t-20261120-194626, tools.rs descriptions t-20261120-180312, secrets tail redaction t-20261120-224155. Which of these are claim-held right now: unconfirmed.
- Render `main.bins` lookup t-20261120-202643-2 waits for the race fix.
- F3 per-project build folders t-20261120-040054 waits for Lazlo's yes on the Disk rule (older note, not re-checked).
- n20-51 t-20261121-010218 (install note) waits for buildsrc.rs and tools.rs.
- Multi-car n20-19 waits for `docs/PHYSICS.md` (the gravity doc merged, 904ff9b), `src/lib.rs` and n20-11 (pc-progress 2026-11-21T02:18:41Z). Its task id is not in my evidence: unconfirmed.
- Not deleted (deleting outside scratch needs Lazlo): `build/debug/examples/t005119_write.exe`, left by task t-20261121-005119-3 (pc-progress 2026-11-21T01:41:16Z).
- No push is pending approval in this evidence. The one approved push of main (b8234a3) is an ancestor of main; any new push needs a new yes.
- Older items: see Carried open items below.


## Carried open items from the previous PROGRESS.md (status checked 02:40Z)

- **GPT gate t-20261120-041456: DONE.** Merged as 872663ad3e (q-20261120-084843, check on main passed). The old "NOT STARTED" line is out of date.
- **Sol replay of the B2 block (due 2026-11-21): unconfirmed.** No pc-progress entry after 2026-11-20T05:31:52Z says it ran; plan in `state/staging/reviewer-patterns/replay-plan.md`. The Haiku arm-B replay (3 rules) was DROPPED and merged as d783f3f4; its raw outputs are task t-20261121-003922 (q-20261121-020123).
- **Stage 4 audit: 8 REAL SERIOUS findings** (`evidence/stage-4-v1-audit-verdicts.md` section 6). #7 (checkpoint .env) is n20-50, task t-20261121-010217, test-only commit 2c81b453, **queued, not merged** at 02:33Z (q-20261121-011934); the coordinator's message said merged, but `queue list` does not show that. n20-51 is t-20261121-010218, open. Records for #1, #2, #4, #6: afd1e8dc and 434c5b2e (on main), with follow-up t-20261121-001109-8 (q-20261121-011309 approved). Whether the other findings are boarded: unconfirmed.
- **Stage 0 tag:** not made. `git tag` shows no stage-0 tag; it needs a fresh app session (old PROGRESS.md; no later evidence).
- **Carry rules from the 2026-11-19 retro (RETRO.md "Still open from the 2026-11-19 carry list"):**
  - (a) re-run `scripts/shadow_canary.py` after every Codex update: NOT YET IN FORCE as far as I found; canary commits 1bb3ab20, 3d4620ef, 5907dcd7 are on main, but I found no step that runs it after an update (unconfirmed);
  - (b) a full check before any direct commit to main: `.git/hooks` holds only samples; NOT YET IN FORCE;
  - (c) coder-prompt line "never wrap a script that calls broadsword in `broadsword run`": `templates/coder-prompt.md` has no such line (grep 02:40Z); NOT YET IN FORCE;
  - (d) SPEC_CHECKLIST on-screen-capture line: `SPEC_CHECKLIST.md` has no such line (grep 02:40Z); NOT YET IN FORCE;
  - (e) file the 8 SERIOUS findings as board tasks: partly (see above);
  - (f) Stage 0 tag: open (above).
  - Task t-20261120-075642 is done. The claim-gap rule is on main as brief_lint R1-R5 (b5362520). The non-coder brief-path rule is `SPEC_CHECKLIST.md` line 49, which says "(No mechanism enforces this yet.)", so it stays NOT YET IN FORCE as a mechanism (last SPEC_CHECKLIST commit ce97b6c7).
- **First GitHub Actions run of `ci.yml`: unverified.** The push was b8234a3 (pc-progress 2026-11-20T09:17:04Z). The repo is private, the unauthenticated API gave 404 and gh is not installed. Lazlo was asked to check https://github.com/Ryvexuz/Broadsword/actions; no result is recorded.
- **`CLAUDE_CODE_EXPERIMENTAL_CODE_MODE`:** still 1 in the user environment (read at 02:40Z). Code mode was tried and not adopted (pc-progress 2026-11-20T01:06:45Z: no REPL tool in headless mode). Removing it is Lazlo's call; not done.
- **Leftover worktrees nb-06 (t-20261119-170109) and nb-19 (t-20261119-171333-17):** not in `git worktree list` and not in `worktrees/broadsword` at 02:40Z; no pc-progress line says they were removed (unconfirmed how).
- **Old follow-ups, board status:** t-20261119-192623, t-20261119-193538 and t-20261119-203616 done; t-20261119-200000 and t-20261119-185546 open; tp1 t-20261118-095350 abandoned.
- **Not re-checked:** `state/cleanup-pending.md`, the Camry download (`reference/camry-2013/`), backups to the private repo, ahead-work patches (`state/staging/AHEAD.md`).

## Next step

1. At main 68a04c05, read `state/staging/morning-summary.md`, then the tail of `state/pc-progress.md`, and get Lazlo's answers to the three decisions above.
2. Run `build\bin\broadsword.exe queue list` and `task list`; check whether n20-11 (t-20261121-021841), the race fix (t-20261120-191700), n20-16b (q-20261121-023252, t-20261121-014029) and the codex route (q-20261121-011825, t-20261120-092355) have moved.
3. Resume from n20-11's worktree `worktrees/crash-sim/t-20261121-021841` (task t-20261121-021841, running; the path is the coordinator's, it is not in the root's `git worktree list`), at the solver-core file `src/solver_m5.*` named in the task's claim in `task list` (whether it exists yet: unconfirmed) and `docs/M5-SOLVER.md` (R1-R12 except R8; merged d1758ae).
