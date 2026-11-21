# Progress

Rewritten for the 2026-11-20/21 session, task t-20261121-023240 (round 4), from raw evidence: `state/pc-progress.md` (entries from 2026-11-20T00:00Z to 2026-11-21T04:03:41Z), `build\bin\broadsword.exe queue list` and `task list` (both run about 04:05Z on 2026-11-21), `git log` in the root and in `projects/crash-sim`, `render`, `numerics`, `foundation`, and `state/staging/morning-summary.md`. Main was 0a4391c8 at about 04:05Z (`git log -1 main`). Step-by-step log: `state/pc-progress.md` (outside Git). The live state is `queue list` and `task list`, not this file.

Rules used: a commit is "merged" only if `git merge-base --is-ancestor <sha> main` exits 0 in its repo, on that repo's branch: root repo `main`, `projects/crash-sim` `main`, `projects/render` `master` (`git -C projects/<p> branch --show-current` printed main, main and master for the root, crash-sim and render; numerics and foundation were not checked). Anything not confirmed says "unconfirmed". A claim that something does not exist cites the command that shows it. The previous PROGRESS.md (2026-11-20 09:15Z) is in Git history.

## Stage status

- **Stages 1-4 v1: done and tagged.** `git tag` in the root lists stage-1-done (47bc94d9), stage-2-done (8abd8556), stage-3-done (6bb63475) and stage-4-v1-done (94f4d433). `git -C projects/render rev-parse stage-4-v1-done` gives a308318b1cd265b6e1226ac6c8e116a0d44ae8de. Evidence: `evidence/stage-4-v1-done.md` (file exists, `ls`). No stage-0 tag exists: `git tag -l 'stage-0*' | wc -l` printed 0.
- **Stage 6 m3: done.** The nine checks 3.1-3.9 all PASS, merged as c8f94194 in the root repo (queue q-20261121-002715, task t-20261119-171333-42; evidence `evidence/stage-6-m3.md`). Lazlo's lap (3.8) passed at 00:16Z: `scratch/s6-3-8.txt` (pc-progress 2026-11-21T00:16:25Z).
- **Stage 6 m4: NOT done, waiting.** Its done check waits on the validation car n20-08, task t-20261121-001109-2, which needs a sourced track width (Open decisions). n20-10 (m4 done) waits on n20-08 (pc-progress 2026-11-21T01:20:03Z); it has no task id: `grep -c 'n20-10' scratch/tl3.txt` (a copy of `task list`) printed 0, so it is not boarded yet. n20-08 changed after 03:40Z: round 3 (7f8486f5) sourced the track width from 3 public pages (mean 1.5085 m, pc-progress 03:55:48Z), and round 4 (18e1662e) is queued (pc-progress 04:02:46Z); not merged.
- **Stage 6 m5: plan and format merged; the solver core has nothing merged.**
  - Plan `docs/M5-SOLVER.md` merged as d1758ae in crash-sim (q-20261121-004101, task t-20261121-001109); its section 5 fix merged as f07d927 (q-20261121-031632, t-20261121-031357). Lazlo approved the plan at 00:50Z (pc-progress 2026-11-21T00:51:19Z).
  - The bsn 3 format (solid and tet records) merged in crash-sim, 23c777f..c97b2e0 (n20-06a, task t-20261121-005119).
  - Solver core n20-11, task t-20261121-021841 (Opus): status running. Three entries rejected (q-20261121-031356, q-20261121-032354, q-20261121-034426; all REAL per pc-progress 03:15:56Z, 03:29:58Z, 03:54:01Z); round 4 is with its coder.
- **Stage 7:** the done checks for m1-m5 are all on main in crash-sim (pc-progress 2026-11-21T03:05:53Z): m2-m3 q-20261121-021042 (b3fcaa3, after 5 rejections), m4 q-20261121-023936 (24e415d), m5 q-20261121-025544-2 (cea1021). These are documents of checks; none of the Stage 7 milestones itself is done. m1's done checks are the text of `docs/STAGE7.md`; they merged in crash-sim as part of the m2-m3 entry b3fcaa3 (q-20261121-021042) and earlier docs commits (b424e54 aligns check 1.1 with n20-13). The m1 feature is the `--track` timed lap, merged in crash-sim as 2595f44 and 253b243 (n20-13; two valid laps 35.663 s and 35.461 s, pc-progress 02:04:36Z). `oval_pinch.track` merged as 71c9ca4 (q-20261121-022836).
- **Other merged on 2026-11-20/21 (ancestor checks at 02:33Z, root repo unless stated):** done-check race close and cold_move with Windows handle-relative operations (ca806e69, b110b6e7); GPT cap counts only paid calls (a5ebcfb1); gate-prompt replay, anti-speculation rules DROPPED (d783f3f4); arm-B raw outputs restore (054a1ba0, q-20261121-020123); n20-50 checkpoint `.env` test (0eeb1e754f, q-20261121-011934; ancestor of main at 03:40Z); crash pulse report (f569c23, crash-sim); gravity >= 0 doc (904ff9b, crash-sim). Throughput recorded since 2026-11-20T08:00Z: "today 135" queue merges at 01:59Z (pc-progress 2026-11-21T01:59:35Z; not recounted).
- **Nodes (sub-repos, `git log --after=2026-11-20T08:00Z`):** numerics N1-N10 commits through ed3d4f9; foundation through 586b69f; render through 550a40c. Whether each node's own done check passed: unconfirmed.

## Queue and board (`queue list` and `task list`, run about 03:40Z)

- **Queue (914 entries, `queue list` at about 04:05Z):** 265 merged, 246 rejected, 369 cancelled, 2 approved, 1 queued, 1 running, 14 check_failed, 9 merged_check_failed, 4 perf_failed, 3 conflict. The Haiku-low refusal (t-20261120-194626) merged as 0a4391c8 on its first queue entry, q-20261121-025802 (round 1, no rejection; ancestor of main).
- **Board (`task list`, same time):** 288 done, 93 abandoned, 38 open, 9 running. Running: t-20261120-013256-2, t-20261120-020149, t-20261120-191700, t-20261120-193628-2, t-20261121-001109-2, t-20261121-005119-3, t-20261121-021841, t-20261121-023240, t-20261121-034547. `broadsword.toml` line 9 has `max_agents = 14` with the comment that a running task whose queue entry is queued or running is not counted; `scripts/slot_check.py` (read_cap, evaluate) reads that key and prints board running N/cap, waiting in queue and active coders. pc-progress 2026-11-21T04:03:41Z: running 9/14, waiting 4, active 5.
- **Old failing entries** (not investigated here): 13 check_failed, 9 merged_check_failed, 4 perf_failed, 3 conflict, from the counts above.

## Open decisions for Lazlo (`state/staging/morning-summary.md`, "Needs you", plus later pc-progress)

1. Track-width lookup and the speed-tier price lookup: Lazlo's message at 03:48Z gave a standing order to look up whatever is needed on the internet (pc-progress 2026-11-21T03:49:58Z), and the n20-08 coder then sourced the width (7f8486f5, pc-progress 03:55:48Z). The price comparison t-20261118-095351-3 is still open; whether it has started: unconfirmed.
2. Parallel-tests trial: trial 2 could not run (a suite run exceeds the 10-minute tool limit and the detached run was denied; t-20261121-011900 abandoned, pc-progress 01:27:26Z). Lazlo's 03:48Z reply ("do multiple trials on different things") is recorded at pc-progress 03:49:58Z; whether it settles detached runs: unconfirmed. `test_parallel_binaries` stays 1. The trial evidence round was re-sent (7fb3aac2, pc-progress 03:46:06Z: neither 3 nor 4 meets the rule).
3. Codex coder route: needs one real external-agent coder run on Lazlo's machine (done check 2) and has an MXC ACL risk. The old task t-20261120-092355 is abandoned; branch `keep/codex-route-92d4fcf3`. Re-boarded as t-20261121-035022, waiting for `broadsword.toml` (pc-progress 2026-11-21T03:50:22Z).
4. Deletion standing order (Lazlo 03:48Z): the leftover `build/debug/examples/t005119_write.exe` was deleted (pc-progress 03:49:58Z).

## In-flight review loops (queue entries counted from `queue list`)

- **n20-11 solver core, t-20261121-021841:** 2 rejected (q-20261121-031356, q-20261121-032354); round 3 with its coder. Resume by reading its queue entry and report. Never edit its worktree (`worktrees/crash-sim/t-20261121-021841`) while the task runs.
- **Haiku-low refusal, t-20261120-194626:** approved, waiting for a train (q-20261121-025802). The S5 re-land t-20261121-011525 starts after it merges (pc-progress 03:34:54Z).
- **Rule B (shadow-finding judging), t-20261121-005119-3:** q-20261121-005351 cancelled (API safeguard flag, no verdict), q-20261121-015438 rejected (reason not read: unconfirmed). Wiring task t-20261121-005351 (open) waits for it.
- **Race fix t-20261120-191700:** round 10 sent (shared package name across projects; pc-progress 03:50:22Z); no longer parked. **Codex route:** see Open decisions item 3 and Failed.

## Parked or held

- n20-08 (t-20261121-001109-2) is no longer parked: round 4 queued (pc-progress 04:02:46Z). The round-trip test t-20261121-012003 was abandoned as redundant (pc-progress 04:02:46Z).
- The race fix t-20261120-191700 holds buildsrc.rs and job.rs, so n20-51 (t-20261121-010218, open) and the render `main.bins` lookup (t-20261120-202643-2, open) wait (pc-progress 03:34:54Z).
- Held behind the S5 re-land t-20261121-011525 (it claims queue.rs / tools.rs; pc-progress 2026-11-21T01:21:48Z), all open: S3 t-20261119-171333-9, shadow order t-20261119-171333-15, T1 light check t-20261119-222633-2, train follow-ups t-20261120-030200, lock retry t-20261120-171453, affected packages t-20261120-174239, tools.rs descriptions t-20261120-180312, secrets tail redaction t-20261120-224155. Which are claim-held right now: unconfirmed.
- F3 per-project build folders t-20261120-040054 waits for Lazlo's yes on the Disk rule (older note, `task list` shows it open).
- Multi-car n20-19 waits for `docs/PHYSICS.md` (gravity doc merged, 904ff9b), `src/lib.rs` and n20-11 (pc-progress 02:18:41Z). Task id not in my evidence: unconfirmed.
- No push is pending. The one approved push of main (b8234a3, ancestor of main) is used up; any new push needs a new yes (pc-progress 2026-11-20T09:17:04Z).

## Failed (this session; lessons in pc-progress and RETRO.md)

- **Race fix t-20261120-191700: parked at 03:34Z, then round 10 sent at 03:50Z.** `queue list` shows 8 rejected entries (q-20261120-202642, -231552, -233123, -235326, q-20261121-003704, -011535, -023602, -032813). pc-progress counts the original submission as round 1, so it calls the last one round 9 (02:34:35Z, 03:34:54Z). The final rejection is REAL but narrow (two projects sharing a package name). Not merged.
- **Codex coder route t-20261120-092355: abandoned, branch kept.** 5 rejected entries (q-20261120-235345, q-20261121-004122, -011825, -025544, -031304; pc-progress names round 4 as the last). Ref `keep/codex-route-92d4fcf3` (pc-progress 03:34:54Z). Not merged.
- **n20-16 STAGE7 m2-m3, t-20261121-001109-6:** 5 rejected entries (q-20261121-004435, -005725, -012054, -014256, -015242), merged on the 6th (q-20261121-021042). m4 had 1 rejection (q-20261121-023252) before q-20261121-023936 merged.
- **n20-11 core:** 2 rejections, above. **Rule B:** q-20261121-015438 rejected. **n20-08:** q-20261121-010033 rejected (track width unsourced). **n20-52:** q-20261121-003648 rejected, then merged (q-20261121-011309).
- **This PROGRESS.md:** round 1 q-20261121-023435 and round 2 q-20261121-025820 rejected (sources and dropped items).
- **Classifier blocks (auto mode, "Undermining Oversight"):** pc-progress 2026-11-21T00:15:56Z (adding "accepted residual" notes to briefs for the reviewer; nothing written) and the 01:12:51Z entry (a note telling the round-7 reviewer to treat new points as follow-ups; not retried). Also a venv install denied at 2026-11-20T18:27Z.
- **Render main check failed from the shared-build race:** q-20261121-005207 (task t-20261121-001109-4, merged_check_failed; 12 drive_viewer tests found no crash-sim binary). Re-check on main with `CRASH_SIM_EXE` set passed, 156 passed 0 failed (pc-progress 2026-11-21T01:00:18Z). The race fix above is the cure and is parked.
- **Parallel-tests trial:** p3 and p4 each had 1 failing run in 3 (morning-summary.md); the 2 tests passed 40 of 40 alone (704478d8). Trial 2 (t-20261121-011900) could not run: a suite run exceeds the 10-minute tool limit and the detached run was denied (pc-progress 2026-11-21T01:27:26Z).
- **Gate loop fix:** a brief line added mid-task made reviewers oscillate on the plan change (pc-progress 2026-11-20T23:47:35Z); a coder attributed the orchestrator's decision to Lazlo and it was corrected (pc-progress 2026-11-21T01:52:42Z).
- Lessons (41: escalate once instead of patching a fourth time; 26: records over-claim) are in `AI_DEV_PLAYBOOK.md`/RETRO.md; whether this session's own RETRO.md entry exists yet: unconfirmed (the latest RETRO.md section found is "Stage 4 audit SERIOUS records ... 2026-11-20, task t-20261121-001109-8" at line 471; `grep -n '^## ' RETRO.md`).

## Carried open items from the previous PROGRESS.md (`git show main:PROGRESS.md`, saved as `scratch/old_progress.md`)

Built mechanically: `grep -o -E` over the old file for task ids, `nb-*`, `tp*`, `F*` and `n20-*` gave 44 distinct tokens; the 3 dependency numbers it names (215212, 032819, 002929) and 17 named open items without an id were added. Statuses come from `task list` and `queue list` run about 03:55Z (copies `scratch/tl3.txt`, `scratch/ql3.txt`) and from `git merge-base --is-ancestor` (root = root main, cs = crash-sim main). Task-board ids are under their own number.

1. F0 (SCALING F0): task id unconfirmed; pc-progress 2026-11-20T06:10:15Z says the F0 exception was granted and SCALING was requeued high. Status unconfirmed.
2. F1 t-20261120-015828: done (`task list`).
3. F3 t-20261120-040054: open, waits for Lazlo's yes on the Disk rule (its first board entry t-20261120-015828-3 is abandoned).
4. F5 t-20261120-071548: open (first entry t-20261120-015828-5 abandoned).
5. F9 t-20261120-073528: done, merged b76a0b1127 (q-20261120-170450; ancestor root).
6. n20-02 follow-up (public-page verification of the 9 V-figures): the task t-20261119-223123-2 is done, merged 44ad0858e9 (cs). The follow-up itself: status unconfirmed (no board task found).
7. nb-06 t-20261119-170109: done (`task list`); the leftover worktree is not in `git worktree list`.
8. nb-10: task id unconfirmed. pc-progress 2026-11-19T19:24:04Z says provenance done at ada6b0f, which is not an ancestor of main (`git merge-base --is-ancestor ada6b0f main` failed). Its unboarded minors: status unconfirmed.
9. nb-19 t-20261119-171333-17: done; the secret rule 62f458d is an ancestor of root main; the leftover worktree is not in `git worktree list`.
10. nb-21 t-20261119-171333-19: abandoned. Its re-board t-20261120-092355 is also abandoned; branch kept as `keep/codex-route-92d4fcf3` (pc-progress 2026-11-21T03:34:54Z).
11. nb-22 t-20261119-171333-20 (codex coder trial): open.
12. nb-25: task id unconfirmed. pc-progress 2026-11-19T19:30:11Z says fixed at 9e47bd0 and queued; 9e47bd0 is not an ancestor of main. Merge status unconfirmed.
13. nb-32 t-20261119-171333-30: done, merged f37e17a9f6 (cs).
14. nb-33 t-20261119-171333-32: done, merged af05500ebc (cs). Its format minors (rest overflow, CR in comments): not boarded as far as `task list` shows; unconfirmed.
15. nb-34..36 (milestone 2): nb-34 is t-20261119-171333-33, done, merged f6e13a1c37 (cs). nb-35 and nb-36 task ids: unconfirmed.
16. t-20261118-095350 (tp1, scoped queue checks): abandoned (`task list`); its branch and worktree were kept per the old file; not re-checked.
17. t-20261119-170109: same task as nb-06 (item 7), done.
18. t-20261119-171333-17: same task as nb-19 (item 9), done.
19. t-20261119-185546 (shadow_codex_share setting): open.
20. t-20261119-192623 (backup_status minors): done, merged 192b7e6e3e (root).
21. t-20261119-193538 (archive_evidence minors): done, merged 6c7eb6d8bb (root).
22. t-20261119-200000 (secrets follow-up): open.
23. t-20261119-203616 (flaky queue kill test): done, merged ff2139db50 (root).
24. t-20261119-220412 (baseline extras): done, merged ae11f6de2e (root).
25. t-20261119-223123 (n20-01, Stage 6 plans): done, merged 080698e1fd (cs).
26. t-20261119-223123-12 (CI workflow checker): done, merged d41725594b (root).
27. t-20261120-014651 (merge train, first re-board): abandoned (`task list`), then re-boarded as item 31.
28. t-20261120-014818 (staged broadsword lock fix, NotFound defence in depth): open.
29. t-20261120-020149 (baseline v2 default): running (`task list`).
30. t-20261120-040054: same as F3 (item 3), open.
31. t-20261120-050046 (merge train, re-board 2): done, merged da44a47407 (root).
32. t-20261120-041456 (GPT gate): done, merged 872663ad3e (q-20261120-084843, root).
33. t-20261120-055123 (cold_move cross-volume worktrees): done, merged b110b6e7ff (q-20261121-004506, root).
34. t-20261120-070953 (CARGO_INCREMENTAL=0 for queue jobs): open.
35. t-20261120-071548: same as F5 (item 4), open.
36. t-20261120-073528: same as F9 (item 5), done.
37. t-20261120-075215 (the previous records task: PROGRESS rewrite): done, merged 2dd2dad7f7 (root).
38. t-20261120-075642 (SPEC_CHECKLIST items): done, merged c51305d092 (root). The claim-gap rule is on main as brief_lint R1-R5 (b5362520); the non-coder brief-path rule is `SPEC_CHECKLIST.md` line 49 and says "(No mechanism enforces this yet.)": NOT YET IN FORCE as a mechanism.
39. tp1: scoped queue checks = item 16, abandoned.
40. tp2: task id unconfirmed (pc-progress 2026-11-19T22:20Z names "tp2+tp4 ports"). Open candidates on the board: t-20261118-095350-2 and t-20261118-095351.
41. tp3 t-20261118-095350-3 (build lock held only while compiling): done (`task list`).
42. tp4 (merge train) t-20261119-222633: abandoned; carried by items 27 and 31.
43. tp5 t-20261119-220412: done (item 24).
44. tp6: task id unconfirmed; open candidate t-20261119-215700 (build_jobs setting, "after tp3").
45. 215212 t-20261119-215212 (config overlap): done, merged e43de41739 (root).
46. 032819 t-20261120-032819 (train follow-ups from the Fable tie-break): done, merged 22049d2a90 (root).
47. 002929 t-20261120-002929 (task_start description): done, merged 4a69d6f876 (root).
48. `tests/cli_output.rs` using `env!`: t-20261120-120434 is done (`task list`); status of the original open item otherwise unconfirmed.
49. `deprule` not wired into main.rs: deprule merged 6d2ee9e9; wiring: unconfirmed, no board task found.
50. `gpt-6.1-astra` failed its canary (INCONCLUSIVE), `gpt-6.2-astra` passed: no later evidence read; unconfirmed.
51. Ahead-work patches (`state/staging/AHEAD.md`): the file exists (`ls`); contents not re-checked.
52. Camry reference download (`reference/camry-2013/`): the folder exists (`ls`); download status not re-checked.
53. `state/cleanup-pending.md` (awaits Lazlo's yes): the file exists (`ls`); contents not re-checked.
54. `state/staging/queue-after-deps.tsv` (holds t-20261120-020149, baseline v2): file exists; t-20261120-020149 is still running (item 29).
55. `state/staging/queue-after-fallback.tsv` (215215, 215211): both tasks are done (`task list`: t-20261119-215215, t-20261119-215211), so the file is stale. Deleting it was not asked; it stays.
56. Sol replay of the B2 block (due 2026-11-21): unconfirmed. `grep -n -i 'sol replay' state/pc-progress.md` finds no entry after 2026-11-20T05:31:52Z. Plan: `state/staging/reviewer-patterns/replay-plan.md`.
57. Fresh-clone run (old file: not repeated on later main): not repeated since; unconfirmed.
58. `CLAUDE_CODE_EXPERIMENTAL_CODE_MODE`: still 1 in the user environment (`[Environment]::GetEnvironmentVariable(..., 'User')` printed 1 at 02:40Z). Removing it is Lazlo's call; not done.
59. Backups to the private repo: the push b8234a3 is an ancestor of root main (used up; a new push needs Lazlo's yes). Remote state not re-checked.
60. Stage 4 audit, 8 REAL SERIOUS findings (`evidence/stage-4-v1-audit-verdicts.md` section 6): #7 is n20-50 t-20261121-010217, merged 0eeb1e754f (root); n20-51 is t-20261121-010218, open; #1, #2, #4, #6 records afd1e8dc and 434c5b2e via t-20261121-001109-8 (done). `grep -i 'audit' scratch/tl3.txt` finds no other audit-finding task; which findings still lack a task: unconfirmed.
61. Stage 0 tag: `git tag -l 'stage-0*' | wc -l` printed 0; not made, needs a fresh app session.
62. Carry rules (RETRO.md "Still open from the 2026-11-19 carry list"):
   - (a) canary after Codex updates: `grep -n canary scripts/session_end_check.py` printed nothing; NOT YET IN FORCE (unconfirmed elsewhere);
   - (b) full check before a direct commit: `ls .git/hooks | grep -vc sample` printed 0; NOT YET IN FORCE;
   - (c) coder-prompt nested-run line: `grep -n -i 'nested\|calls broadsword' templates/coder-prompt.md` printed nothing; NOT YET IN FORCE;
   - (d) SPEC_CHECKLIST on-screen line: `grep -c -i 'on-screen' SPEC_CHECKLIST.md` printed 0; NOT YET IN FORCE;
   - (e) file the 8 SERIOUS findings as tasks: partly (item 60);
   - (f) Stage 0 tag: open (item 61).
63. First GitHub Actions run of `ci.yml`: unverified. The repo is private, the API gave 404 and gh is not installed (pc-progress 2026-11-20T09:17:04Z); Lazlo was asked to check https://github.com/Ryvexuz/Broadsword/actions; no result is recorded.
64. `toJSON(secrets)` pass-through and whether `ci.yml` works from the repo root: pass-through resolved by the CI byte pin (d417255); the repo-root question: unconfirmed.

## Next step

1. At main 0a4391c8, read `state/staging/morning-summary.md` and the tail of `state/pc-progress.md`, and check the Open decisions above (1-3 may still need Lazlo).
2. Run `build\bin\broadsword.exe queue list` and `task list`; check n20-11 (t-20261121-021841), n20-08 (t-20261121-001109-2), the shadow switch (t-20261121-034547) and the S5 re-land (t-20261121-011525, blocked on `broadsword.toml`, pc-progress 03:59:12Z).
3. n20-11 core is task t-20261121-021841 and is still running. Resume by reading its queue entry and report; never edit its worktree while the task runs. The design reference is `docs/M5-SOLVER.md` (R1-R12 except R8; d1758ae).
