# Progress

Rewritten for the 2026-11-20/21 session, task t-20261121-023240, from raw evidence only: `state/pc-progress.md` (entries from 2026-11-20T00:00Z to 2026-11-21T02:32:54Z), `build\bin\broadsword.exe task list` and `queue list` (both read about 02:33Z on 2026-11-21), `git log` in the root and in `projects/crash-sim`, `render`, `numerics`, `foundation`, and `state/staging/morning-summary.md`. Main was 68a04c05 when written. Step-by-step log: `state/pc-progress.md` (outside Git). The live state is `queue list` and `task list`, not this file.

Rules used here: "merged" means `git merge-base --is-ancestor <sha> main` exited 0 in the repo named (checked 02:33Z), or the queue entry says merged. Anything I could not confirm says "unconfirmed". The previous PROGRESS.md (2026-11-20 09:15Z) is in Git history; its Stage 4 / merge-train / backup details were not re-checked here.

## Stage status

- **Stages 1-4 v1: done and tagged earlier** (older record; not re-checked in this rewrite).
- **Stage 6 m3: done.** The nine checks 3.1-3.9 all PASS, merged as c8f94194 (queue q-20261121-002715, task t-20261119-171333-42; evidence `evidence/stage-6-m3.md`). Lazlo's lap (3.8) passed at 00:16Z: `scratch/s6-3-8.txt` (pc-progress 2026-11-21T00:16:25Z).
- **Stage 6 m4: NOT done, waiting.** Its done check waits on the validation car n20-08 (task t-20261121-001109-2), which waits on a sourced track width (see Open decisions). n20-10 (m4 done) waits on n20-08 (pc-progress 01:20:03Z).
- **Stage 6 m5: core in progress, nothing merged.** The m5 plan (`docs/M5-SOLVER.md`) merged as d1758ae in crash-sim (q-20261121-004101, task t-20261121-001109, round 3). Lazlo approved the plan at 00:50Z. The bsn 3 format (solid and tet records) merged in crash-sim, 23c777f..c97b2e0 (n20-06a, task t-20261121-005119). Solver core n20-11 is running, Opus: t-20261121-021841 (`src/solver_m5.*`; no queue entry yet).
- **Stage 7 m1-m3 done checks merged** (morning-summary.md; q-20261121-021042 merged b3fcaa3 in crash-sim after 6 review rounds, task t-20261121-001109-6). The `--track` timed lap merged in crash-sim as 2595f44 and 253b243 (n20-13; two valid laps 35.663 s and 35.461 s, pc-progress 02:04:36Z). `oval_pinch.track` merged as 71c9ca4 (crash-sim). m4 (t-20261121-014029) is running; m5 checks (t-20261121-014029-2) are open.
- **Other merged on 2026-11-20/21 (each checked as an ancestor of main):** done-check race close, cold_move with Windows handle-relative operations (ca806e69, b110b6e7); GPT cap counts only paid calls (a5ebcfb1); gate-prompt replay, anti-speculation rules DROPPED (d783f3f4); crash pulse report `--pulse-report` (f569c23, crash-sim); gravity >= 0 doc (904ff9b, crash-sim). Throughput recorded at 01:59Z: 1 h = 6, 3 h = 15, today 135 queue merges (pc-progress; I did not recount).
- **Nodes (sub-repos, git log only):** numerics N1-N10 commits through ed3d4f9 (N10 review fix); foundation up to 586b69f (trace docs); render up to 550a40c (text, drive viewer, golden frames). Whether each node's own done check passed: unconfirmed here.

## Queue and board (queue list / task list, about 02:33Z)

- **Queue (895 entries):** 254 merged, 232 rejected, 369 cancelled, 5 approved, 6 queued, 13 check_failed, 9 merged_check_failed, 4 perf_failed, 3 conflict.
- **Approved, waiting for a train:** q-20261121-010841 (t-20261120-223314), -011309 (t-20261121-001109-8), -011341 (t-20261121-005258), -011739 (t-20261121-003648), -022836 (crash-sim oval_pinch task t-20261121-015242; note 71c9ca4 is already on crash-sim main, so this status is stale or a different commit: unconfirmed).
- **Queued:** q-20261121-011825 (codex route t-20261120-092355), -011858 (t-20261120-193628-2), -011934 (t-20261121-010217), -015438 (rule B t-20261121-005119-3), -020123 (t-20261121-003922), -023252 (m4 t-20261121-014029).
- **Board:** 277 done, 91 abandoned, 38 open, 17 running. "Running" includes tasks whose entry waits in the queue; the 14-slot cap counts working coders only.
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
- Held behind the S5 re-land (they claim queue.rs / tools.rs): the queue.rs tasks (S3, shadow order, T1 light check, train follow-ups, lock retry, affected-packages, Haiku-low refusal) and the tools.rs descriptions, e.g. t-20261120-174239 and t-20261120-180312.
- Render `main.bins` lookup t-20261120-202643-2 waits for the race fix.
- F3 per-project build folders t-20261120-040054 waits for Lazlo's yes on the Disk rule (older note, not re-checked).
- n20-51 t-20261121-010218 (install note) waits for buildsrc.rs and tools.rs.
- Multi-car n20-19 waits for `docs/PHYSICS.md`, `src/lib.rs` and n20-16 (pc-progress 01:46Z). The task id is not in my evidence: unconfirmed.
- Not deleted (deleting outside scratch needs Lazlo): `build/debug/examples/t005119_write.exe` (pc-progress 01:41Z).
- No push is pending approval in this evidence. The one approved push of main (b8234a3) is an ancestor of main; any new push needs a new yes.
- Older items (backups, `state/cleanup-pending.md`, leftover worktrees nb-06 and nb-19, camry download): carried from the previous PROGRESS.md, not re-checked.

## Next step

1. Read `state/staging/morning-summary.md`, then the tail of `state/pc-progress.md`, and get Lazlo's answers to the three decisions above.
2. Run `build\bin\broadsword.exe queue list` and `task list`; check whether n20-11 (t-20261121-021841), the race-fix design change, n20-16b (q-20261121-023252) and the codex route (q-20261121-011825) have moved.
3. Resume from n20-11's worktree `worktrees/crash-sim/t-20261121-021841` (unconfirmed path; use `task list`).
