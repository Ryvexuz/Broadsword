# Progress

Rewritten for the 2026-11-20/21 session, task t-20261121-023240 (round 3), from raw evidence: `state/pc-progress.md` (entries from 2026-11-20T00:00Z to 2026-11-21T03:39:36Z), `build\bin\broadsword.exe queue list` and `task list` (both run about 03:40Z on 2026-11-21), `git log` in the root and in `projects/crash-sim`, `render`, `numerics`, `foundation`, and `state/staging/morning-summary.md`. Main was 054a1ba0 at 03:40Z (`git log -1 main`). Step-by-step log: `state/pc-progress.md` (outside Git). The live state is `queue list` and `task list`, not this file.

Rules used: "merged" means `git merge-base --is-ancestor <sha> main` exited 0 in the repo named, or the queue entry says merged. Anything not confirmed says "unconfirmed". A claim that something does not exist cites the command that shows it. The previous PROGRESS.md (2026-11-20 09:15Z) is in Git history.

## Stage status

- **Stages 1-4 v1: done and tagged.** `git tag` in the root lists stage-1-done (47bc94d9), stage-2-done (8abd8556), stage-3-done (6bb63475) and stage-4-v1-done (94f4d433). `git -C projects/render rev-parse stage-4-v1-done` gives a308318b1cd265b6e1226ac6c8e116a0d44ae8de. Evidence: `evidence/stage-4-v1-done.md` (file exists, `ls`). No stage-0 tag exists: `git tag -l 'stage-0*' | wc -l` printed 0.
- **Stage 6 m3: done.** The nine checks 3.1-3.9 all PASS, merged as c8f94194 in the root repo (queue q-20261121-002715, task t-20261119-171333-42; evidence `evidence/stage-6-m3.md`). Lazlo's lap (3.8) passed at 00:16Z: `scratch/s6-3-8.txt` (pc-progress 2026-11-21T00:16:25Z).
- **Stage 6 m4: NOT done, waiting.** Its done check waits on the validation car n20-08, task t-20261121-001109-2, which needs a sourced track width (Open decisions). n20-10 (m4 done) waits on n20-08 (pc-progress 2026-11-21T01:20:03Z).
- **Stage 6 m5: plan and format merged; the solver core has nothing merged.**
  - Plan `docs/M5-SOLVER.md` merged as d1758ae in crash-sim (q-20261121-004101, task t-20261121-001109); its section 5 fix merged as f07d927 (q-20261121-031632, t-20261121-031357). Lazlo approved the plan at 00:50Z (pc-progress 2026-11-21T00:51:19Z).
  - The bsn 3 format (solid and tet records) merged in crash-sim, 23c777f..c97b2e0 (n20-06a, task t-20261121-005119).
  - Solver core n20-11, task t-20261121-021841 (Opus): status running. Two entries rejected, q-20261121-031356 and q-20261121-032354 (both REAL per pc-progress 03:15:56Z and 03:29:58Z); round 3 is with its coder.
- **Stage 7:** the done checks for m1-m5 are all on main in crash-sim (pc-progress 2026-11-21T03:05:53Z): m2-m3 q-20261121-021042 (b3fcaa3, after 5 rejections), m4 q-20261121-023936 (24e415d), m5 q-20261121-025544-2 (cea1021). These are documents of checks; none of the Stage 7 milestones itself is done. The `--track` timed lap (n20-13, m1) merged in crash-sim as 2595f44 and 253b243 (two valid laps 35.663 s and 35.461 s, pc-progress 02:04:36Z). `oval_pinch.track` merged as 71c9ca4 (q-20261121-022836).
- **Other merged on 2026-11-20/21 (ancestor checks at 02:33Z, root repo unless stated):** done-check race close and cold_move with Windows handle-relative operations (ca806e69, b110b6e7); GPT cap counts only paid calls (a5ebcfb1); gate-prompt replay, anti-speculation rules DROPPED (d783f3f4); arm-B raw outputs restore (054a1ba0, q-20261121-020123); n20-50 checkpoint `.env` test (0eeb1e754f, q-20261121-011934; ancestor of main at 03:40Z); crash pulse report (f569c23, crash-sim); gravity >= 0 doc (904ff9b, crash-sim). Throughput recorded since 2026-11-20T08:00Z: "today 135" queue merges at 01:59Z (pc-progress 2026-11-21T01:59:35Z; not recounted).
- **Nodes (sub-repos, `git log --after=2026-11-20T08:00Z`):** numerics N1-N10 commits through ed3d4f9; foundation through 586b69f; render through 550a40c. Whether each node's own done check passed: unconfirmed.

## Queue and board (`queue list` and `task list`, run about 03:40Z)

- **Queue (907 entries, `queue list`):** 264 merged, 244 rejected, 369 cancelled, 1 approved, 0 queued, 13 check_failed, 9 merged_check_failed, 4 perf_failed, 3 conflict. Approved: q-20261121-025802 (Haiku-low refusal, t-20261120-194626).
- **Board (`task list`, same time):** 287 done, 92 abandoned, 38 open, 9 running. Running: t-20261120-013256-2, t-20261120-020149, t-20261120-191700, t-20261120-193628-2, t-20261120-194626, t-20261121-001109-2, t-20261121-005119-3, t-20261121-021841, t-20261121-023240. `broadsword.toml` line 9 has `max_agents = 14` with the comment that a running task whose queue entry is queued or running is not counted; `scripts/slot_check.py` (read_cap, evaluate) reads that key and prints board running N/cap, waiting in queue and active coders. pc-progress 2026-11-21T03:39:36Z: running 9/14, waiting 1, active 8.
- **Old failing entries** (not investigated here): 13 check_failed, 9 merged_check_failed, 4 perf_failed, 3 conflict, from the counts above.

## Open decisions for Lazlo (`state/staging/morning-summary.md`, "Needs you")

1. OK to look up the 2012 Accent's track width online for the validation car, or give a source? It is assumed 1.500 m. Blocks n20-08 (t-20261121-001109-2) and Stage 6 m4's done check.
2. OK for an online lookup for the price comparison of paid speed tiers (t-20261118-095351-3, open)?
3. The follow-up parallel-tests trial cannot run: one full test run takes longer than a 10-minute tool call and the permission check refused a detached run (t-20261121-011900, abandoned, pc-progress 01:27:26Z). Allow detached measurement runs, or drop the trial? `test_parallel_binaries` stays 1.
4. New since the summary (pc-progress 2026-11-21T03:34:54Z), parked for Lazlo: the codex coder route needs one real external-agent coder run on his machine (done check 2) and has an MXC ACL risk; branch kept as `keep/codex-route-92d4fcf3`. Also parked: delete the old-layout `main.bins/<profile>/` copies (a deletion outside scratch; pc-progress 2026-11-21T02:36:02Z).

## In-flight review loops (queue entries counted from `queue list`)

- **n20-11 solver core, t-20261121-021841:** 2 rejected (q-20261121-031356, q-20261121-032354); round 3 with its coder. Resume by reading its queue entry and report. Never edit its worktree (`worktrees/crash-sim/t-20261121-021841`) while the task runs.
- **Haiku-low refusal, t-20261120-194626:** approved, waiting for a train (q-20261121-025802). The S5 re-land t-20261121-011525 starts after it merges (pc-progress 03:34:54Z).
- **Rule B (shadow-finding judging), t-20261121-005119-3:** q-20261121-005351 cancelled (API safeguard flag, no verdict), q-20261121-015438 rejected (reason not read: unconfirmed). Wiring task t-20261121-005351 (open) waits for it.
- **Race fix and codex route:** both parked; see Failed.

## Parked or held

- n20-08 round 2 (28719b17): unqueued until decision 1; the round-trip test t-20261121-012003 (open) waits for n20-08.
- Race fix t-20261120-191700 holds buildsrc.rs and job.rs, so n20-51 (t-20261121-010218, open) and the render `main.bins` lookup (t-20261120-202643-2, open) wait (pc-progress 03:34:54Z).
- Held behind the S5 re-land t-20261121-011525 (it claims queue.rs / tools.rs; pc-progress 2026-11-21T01:21:48Z), all open: S3 t-20261119-171333-9, shadow order t-20261119-171333-15, T1 light check t-20261119-222633-2, train follow-ups t-20261120-030200, lock retry t-20261120-171453, affected packages t-20261120-174239, tools.rs descriptions t-20261120-180312, secrets tail redaction t-20261120-224155. Which are claim-held right now: unconfirmed.
- F3 per-project build folders t-20261120-040054 waits for Lazlo's yes on the Disk rule (older note, `task list` shows it open).
- Multi-car n20-19 waits for `docs/PHYSICS.md` (gravity doc merged, 904ff9b), `src/lib.rs` and n20-11 (pc-progress 02:18:41Z). Task id not in my evidence: unconfirmed.
- Not deleted (deleting outside scratch needs Lazlo): `build/debug/examples/t005119_write.exe`, left by task t-20261121-005119-3 (pc-progress 2026-11-21T01:41:16Z).
- No push is pending. The one approved push of main (b8234a3, ancestor of main) is used up; any new push needs a new yes (pc-progress 2026-11-20T09:17:04Z).

## Failed (this session; lessons in pc-progress and RETRO.md)

- **Race fix t-20261120-191700: parked for Lazlo.** `queue list` shows 8 rejected entries (q-20261120-202642, -231552, -233123, -235326, q-20261121-003704, -011535, -023602, -032813). pc-progress counts the original submission as round 1, so it calls the last one round 9 (02:34:35Z, 03:34:54Z). The final rejection is REAL but narrow (two projects sharing a package name). Not merged.
- **Codex coder route t-20261120-092355: abandoned, branch kept.** 5 rejected entries (q-20261120-235345, q-20261121-004122, -011825, -025544, -031304; pc-progress names round 4 as the last). Ref `keep/codex-route-92d4fcf3` (pc-progress 03:34:54Z). Not merged.
- **n20-16 STAGE7 m2-m3, t-20261121-001109-6:** 5 rejected entries (q-20261121-004435, -005725, -012054, -014256, -015242), merged on the 6th (q-20261121-021042). m4 had 1 rejection (q-20261121-023252) before q-20261121-023936 merged.
- **n20-11 core:** 2 rejections, above. **Rule B:** q-20261121-015438 rejected. **n20-08:** q-20261121-010033 rejected (track width unsourced). **n20-52:** q-20261121-003648 rejected, then merged (q-20261121-011309).
- **This PROGRESS.md:** round 1 q-20261121-023435 and round 2 q-20261121-025820 rejected (sources and dropped items).
- **Classifier blocks (auto mode, "Undermining Oversight"):** pc-progress 2026-11-21T00:15:56Z (adding "accepted residual" notes to briefs for the reviewer; nothing written) and the 01:12:51Z entry (a note telling the round-7 reviewer to treat new points as follow-ups; not retried). Also a venv install denied at 2026-11-20T18:27Z.
- **Render main check failed from the shared-build race:** q-20261121-005207 (task t-20261121-001109-4, merged_check_failed; 12 drive_viewer tests found no crash-sim binary). Re-check on main with `CRASH_SIM_EXE` set passed, 156 passed 0 failed (pc-progress 2026-11-21T01:00:18Z). The race fix above is the cure and is parked.
- **Parallel-tests trial:** p3 and p4 each had 1 failing run in 3 (morning-summary.md); the 2 tests passed 40 of 40 alone (704478d8). Trial 2 (t-20261121-011900) could not run: a suite run exceeds the 10-minute tool limit and the detached run was denied (pc-progress 2026-11-21T01:27:26Z).
- **Gate loop fix:** a brief line added mid-task made reviewers oscillate on the plan change (pc-progress 2026-11-20T23:47:35Z); a coder attributed the orchestrator's decision to Lazlo and it was corrected (pc-progress 2026-11-21T01:52:42Z).
- Lessons (41: escalate once instead of patching a fourth time; 26: records over-claim) are in `AI_DEV_PLAYBOOK.md`/RETRO.md; whether this session's own RETRO.md entry exists yet: unconfirmed (the latest RETRO.md section found is "Stage 4 audit SERIOUS records ... 2026-11-20, task t-20261121-001109-8" at line 471; `grep -n '^## ' RETRO.md`).

## Carried open items from the previous PROGRESS.md (status checked 03:40Z)

- **GPT gate t-20261120-041456: done.** Merged as 872663ad3e (q-20261120-084843).
- **Sol replay of the B2 block (due 2026-11-21): unconfirmed.** No pc-progress entry after 2026-11-20T05:31:52Z says it ran (`grep -n -i 'sol replay' state/pc-progress.md`); plan in `state/staging/reviewer-patterns/replay-plan.md`. The Haiku arm-B replay (3 rules) was dropped (d783f3f4).
- **Stage 4 audit, 8 REAL SERIOUS findings** (`evidence/stage-4-v1-audit-verdicts.md` section 6). #7 (checkpoint .env) is n20-50, t-20261121-010217, merged as 0eeb1e754f (q-20261121-011934). n20-51 is t-20261121-010218, open. Records for #1, #2, #4, #6: afd1e8dc and 434c5b2e on main, via t-20261121-001109-8 (done). Check run: `grep -i 'audit\|SERIOUS' scratch/tl2.txt` (a copy of `task list` at 03:40Z) showed only t-20261121-001109-8 as an audit-record task besides Stage 4 audit tasks already done; so (e) "file all 8 as board tasks" is not complete and which findings have no task: unconfirmed.
- **Stage 0 tag:** not made. `git tag -l 'stage-0*' | wc -l` printed 0; it needs a fresh app session (old PROGRESS.md).
- **Carry rules from the 2026-11-19 retro (RETRO.md "Still open from the 2026-11-19 carry list"):**
  - (a) re-run `scripts/shadow_canary.py` after every Codex update: NOT YET IN FORCE as far as I found; canary commits 1bb3ab20, 3d4620ef, 5907dcd7 are on main, but `grep -n canary scripts/session_end_check.py` printed nothing (unconfirmed elsewhere);
  - (b) a full check before any direct commit to main: `ls .git/hooks | grep -vc sample` printed 0 (no hook); NOT YET IN FORCE;
  - (c) coder-prompt line "never wrap a script that calls broadsword in `broadsword run`": `grep -n -i 'nested\|calls broadsword' templates/coder-prompt.md` printed nothing; NOT YET IN FORCE;
  - (d) SPEC_CHECKLIST on-screen-capture line: `grep -c -i 'on-screen' SPEC_CHECKLIST.md` printed 0; NOT YET IN FORCE;
  - (e) file the 8 SERIOUS findings as board tasks: partly (see the audit item above);
  - (f) Stage 0 tag: open (`git tag -l 'stage-0*' | wc -l` printed 0).
  - Task t-20261120-075642 is done. The claim-gap rule is on main as brief_lint R1-R5 (b5362520). The non-coder brief-path rule is `SPEC_CHECKLIST.md` line 49 and says "(No mechanism enforces this yet.)", so it is NOT YET IN FORCE as a mechanism (last SPEC_CHECKLIST commit ce97b6c7).
- **First GitHub Actions run of `ci.yml`: unverified.** The push was b8234a3 (pc-progress 2026-11-20T09:17:04Z): the repo is private, the unauthenticated API gave 404, gh is not installed. Lazlo was asked to check https://github.com/Ryvexuz/Broadsword/actions; no result is recorded.
- **`CLAUDE_CODE_EXPERIMENTAL_CODE_MODE`:** still 1 in the user environment (`[Environment]::GetEnvironmentVariable(..., 'User')` printed 1 at 02:40Z). Code mode was tried and not adopted (pc-progress 2026-11-20T01:06:45Z: no REPL tool in headless mode). Removing it is Lazlo's call; not done.
- **Leftover worktrees nb-06 (t-20261119-170109) and nb-19 (t-20261119-171333-17):** not in `git worktree list` and not in `worktrees/broadsword` at 02:40Z; no pc-progress line says they were removed (unconfirmed how).
- **Old follow-ups (`task list`):** t-20261119-192623, t-20261119-193538 and t-20261119-203616 done; t-20261119-200000 and t-20261119-185546 open; tp1 t-20261118-095350 abandoned.
- **Not re-checked:** `state/cleanup-pending.md`, the Camry download (`reference/camry-2013/`), backups to the private repo, ahead-work patches (`state/staging/AHEAD.md`).

## Next step

1. At main 054a1ba0, read `state/staging/morning-summary.md` and the tail of `state/pc-progress.md`, and get Lazlo's answers to decisions 1-4.
2. Run `build\bin\broadsword.exe queue list` and `task list`; check n20-11 (t-20261121-021841), the Haiku-low refusal (q-20261121-025802) and the S5 re-land (t-20261121-011525).
3. n20-11 core is task t-20261121-021841 and is still running. Resume by reading its queue entry and report; never edit its worktree while the task runs. The design reference is `docs/M5-SOLVER.md` (R1-R12 except R8; d1758ae).
