# Progress

Rewritten for the 2026-11-20/21 session, task t-20261121-023240 (round 5), from ONE snapshot taken at **2026-11-21T04:37:22Z** (`scratch/snap-0445/time.txt`). Snapshot files, all in `scratch/snap-0445/`: `ql.txt` (`build\bin\broadsword.exe queue list`), `tl.txt` (`task list`), `git_.txt` (root), `gitprojects_crash-sim.txt`, `gitprojects_render.txt`, `gitprojects_numerics.txt`, `gitprojects_foundation.txt` (each `git log --after=2026-11-19T23:59:59Z`), and `head*.txt` (each repo's latest commit). Every status and count below comes from those files, plus `state/pc-progress.md` read to its entry at 2026-11-21T04:35:19Z and `state/staging/morning-summary.md`. Anything that moved after 04:37:22Z is not here: the live state is `queue list` and `task list`, not this file.

Heads in the snapshot: root `main` 8511fcf8, `projects/crash-sim` `main` 9b1314d, `projects/render` `master` 550a40c, `projects/numerics` `main` ed3d4f9, `projects/foundation` `main` 586b69f.

Rule for "merged": a commit is merged only if `git merge-base --is-ancestor <sha> main` exits 0 in its repo, on that repo's branch: root `main`, `projects/crash-sim` `main`, `projects/render` `master` (`git -C projects/<p> branch --show-current` printed main, main and master for the root, crash-sim and render; numerics and foundation were not checked). Anything not confirmed says "unconfirmed". A claim that something does not exist cites the command that shows it. The previous PROGRESS.md (2026-11-20 09:15Z) is in Git history and in `scratch/old_progress.md`.

## Stage status (snapshot 04:37:22Z)

- **Stages 1-4 v1: done and tagged.** `git tag` in the root lists stage-1-done (47bc94d9), stage-2-done (8abd8556), stage-3-done (6bb63475) and stage-4-v1-done (94f4d433). `git -C projects/render rev-parse stage-4-v1-done` gives a308318b1cd265b6e1226ac6c8e116a0d44ae8de. Evidence: `evidence/stage-4-v1-done.md` (file exists, `ls`). No stage-0 tag exists: `git tag -l 'stage-0*' | wc -l` printed 0.
- **Stage 6 m3: done.** Nine checks 3.1-3.9 PASS, merged as c8f94194 in the root repo (q-20261121-002715, task t-20261119-171333-42; evidence `evidence/stage-6-m3.md`). Lazlo's lap (3.8) passed at 00:16Z: `scratch/s6-3-8.txt` (pc-progress 2026-11-21T00:16:25Z).
- **Stage 6 m4: NOT done, blocked and parked for Lazlo.**
  - The validation car n20-08 (task t-20261121-001109-2, done) merged in crash-sim as 656ff83..18e1662 (q-20261121-040850; both are ancestors of crash-sim main). Its track width is sourced from 3 public pages, mean 1.5085 m (7f8486f5; pc-progress 03:55:48Z).
  - The m4 plan figure V6 is unsourced (gate 4.1b). The V6 sourcing task t-20261121-042915 is abandoned with no work: every primary source (NHTSA ESV paper, SAE 840201, SAE 2013-01-0762) was HTTP 403 or paywalled (pc-progress 04:33:17Z). Options for Lazlo, from that entry: (a) he opens or downloads the ESV 98-S11-O-02 PDF in his own browser; (b) derive V6 from the repeated Camry tests in `reference/camry-2013` (changes the plan's sourcing rule, his call); (c) re-base the margins.
  - n20-10a (t-20261121-042731) is abandoned with no work: its brief wrongly asked for test B, and its prerequisites were missing (pc-progress 04:29:15Z). n20-08b, the test B car and wall, is running: t-20261121-042915-2. n20-10b and the re-boarded n20-10a have no task id in `tl.txt` (`grep -c 'n20-10' scratch/snap-0445/tl.txt` printed 1, the n20-10a line only).
- **Stage 6 m5: plan, format and solver core merged; the rest of m5 is not done.**
  - Plan `docs/M5-SOLVER.md` merged as d1758ae in crash-sim (q-20261121-004101, task t-20261121-001109). Its section 5 fix: f07d927 (q-20261121-031632, t-20261121-031357). Lazlo approved the plan at 00:50Z (pc-progress 00:51:19Z).
  - bsn 3 format (solid and tet records): crash-sim 23c777f..c97b2e0 (n20-06a, task t-20261121-005119).
  - Solver core n20-11, task t-20261121-021841: **done, merged** in crash-sim as f05a8bb..9b1314d (q-20261121-042237, after 4 rejections q-20261121-031356, -032354, -034426, -040727; both shas are ancestors of crash-sim main). Remaining m5 work (for example n20-11c, the R8 test, and open tasks t-20261121-032355 and t-20261121-032355-2): not done.
- **Stage 7:** the done checks for m1-m5 are all in `docs/STAGE7.md` on crash-sim main (pc-progress 03:05:53Z): m2-m3 in q-20261121-021042 (b3fcaa3, after 5 rejections), m4 in q-20261121-023936 (24e415d), m5 in q-20261121-025544-2 (cea1021). m1's done checks are the text of that file (the m2-m3 entry and earlier docs commits; b424e54 aligns check 1.1 with n20-13); the m1 feature is the `--track` timed lap, merged in crash-sim as 2595f44 and 253b243 (n20-13; two valid laps 35.663 s and 35.461 s, pc-progress 02:04:36Z). `oval_pinch.track` merged as 71c9ca4 (q-20261121-022836). None of the Stage 7 milestones itself is done.
- **Other merged on 2026-11-20/21 (ancestor checks in the right repo):**
  - root: cold_move with Windows handle-relative operations (ca806e69, b110b6e7); GPT cap counts only paid calls (a5ebcfb1); gate-prompt replay, rules DROPPED (d783f3f4); arm-B raw outputs (054a1ba0); n20-50 checkpoint `.env` test (0eeb1e754f, q-20261121-011934); Haiku-low refusal (0a4391c8, q-20261121-025802, task t-20261120-194626, round 1, no rejection); shadow switch to gpt-6.2-sol (8511fcf8, q-20261121-034734, task t-20261121-034547); rule B with the spot check (e3a31982, d2d4a124, q-20261121-034235, task t-20261121-005119-3, merged after 1 rejection q-20261121-015438 and 1 cancelled q-20261121-005351);
  - crash-sim: pulse report (f569c23); gravity >= 0 doc (904ff9b).
  - Throughput since 2026-11-20T08:00Z: "today 135" queue merges at 01:59Z (pc-progress 01:59:35Z; not recounted).
- **Nodes (sub-repo git logs in the snapshot):** numerics N1-N10 through ed3d4f9; foundation through 586b69f; render through 550a40c. Whether each node's own done check passed: unconfirmed.

## Queue and board (snapshot 04:37:22Z)

- **Queue (`ql.txt`, 922 entries):** 269 merged, 251 rejected, 369 cancelled, 2 approved, 0 queued, 0 running, 14 check_failed, 9 merged_check_failed, 5 perf_failed, 3 conflict. Approved: q-20261121-041549 (price comparison, task t-20261118-095351-3) and q-20261121-043013 (spot-check wiring, task t-20261121-005351; commit 67e220da).
- **Board (`tl.txt`):** 292 done, 96 abandoned, 37 open, 9 running. Running: t-20261118-095351-3, t-20261120-013256-2, t-20261120-020149, t-20261120-191700 (race fix), t-20261121-005351, t-20261121-011525 (S5 re-land), t-20261121-023240 (this file), t-20261121-041549, t-20261121-042915-2 (n20-08b). `broadsword.toml` line 9 has `max_agents = 14` with the comment that a running task whose queue entry is queued or running is not counted; `scripts/slot_check.py` (read_cap, evaluate) reads that key and prints board running N/cap, waiting in queue and active coders. pc-progress 2026-11-21T04:32:18Z: running 10/14, waiting 6, active 4.
- **Old failing entries** (not investigated here): 14 check_failed, 9 merged_check_failed, 5 perf_failed, 3 conflict in `ql.txt`.

## Open decisions for Lazlo

1. **V6 for Stage 6 m4** (options (a)-(c) above; pc-progress 04:33:17Z). This is the one blocker for m4.
2. **Price comparison** t-20261118-095351-3 (public pages only; pc-progress 04:15:49Z: Fast = 2x Standard, Flex = 0.5x, Claude fast 2x; it recommends keeping Claude fast off and possibly one paid probe of gate Fast, about 0.2 USD, if Lazlo OKs). Its queue entry q-20261121-041549 is approved. The paid-tier measurement stays parked because it spends money.
3. **Parallel-tests trial:** trial 2 could not run (a suite run exceeds the 10-minute tool limit and the detached run was denied; t-20261121-011900 abandoned, pc-progress 01:27:26Z). Lazlo's 03:48Z reply is recorded at pc-progress 03:49:58Z; a live trial was boarded as t-20261121-043519 (open; pc-progress 04:35:19Z). `test_parallel_binaries` stays 1 until it reports.
4. **Standing orders from Lazlo (03:48Z, pc-progress 03:49:58Z):** look up what is needed online; delete what is not needed to keep the SSD clean. Applied: `build/debug/examples/t005119_write.exe` deleted (pc-progress 03:49:58Z).

## In-flight loops (snapshot)

- **Race fix t-20261120-191700 (running):** 9 rejected entries in `ql.txt` (q-20261120-202642, -231552, -233123, -235326, q-20261121-003704, -011535, -023602, -032813, -041528). pc-progress counts the original submission as round 1; the coordinator's message gives round 11. No later entry is in the snapshot. It holds buildsrc.rs and job.rs, so n20-51 (t-20261121-010218, open) and the render `main.bins` lookup (t-20261120-202643-2, open) wait for it.
- **S5 re-land t-20261121-011525:** running in the snapshot (the coordinator says it started about 04:45Z; the snapshot already shows it running); it claims queue.rs and `broadsword.toml`. The older S5 task t-20261119-171333-10 has 6 non-merged entries (q-20261120-180311, -211543, -220118, -224155, -230940, q-20261121-003448); the branch is `keep/s5-a730cf4c`.
- **Spot-check wiring t-20261121-005351:** approved (q-20261121-043013), waiting for a train.
- **est_usd price fix t-20261121-041549 (running):** q-20261121-043405 rejected (reason not read: unconfirmed).
- **Codex coder route re-land t-20261121-035022 (open):** waits for `broadsword.toml`; old task t-20261120-092355 is abandoned with 5 rejected entries (q-20261120-235345, q-20261121-004122, -011825, -025544, -031304); branch `keep/codex-route-92d4fcf3` (pc-progress 03:34:54Z, 03:50:22Z). Done check 2 needs one real external-agent coder run on Lazlo's machine, with an MXC ACL risk.
- **Parallel trial t-20261121-043519:** open. **Live check:** the toml queue order is whichever of S5, the codex re-land and the trial is ready first (pc-progress 04:35:19Z).

## Parked or held

- m4 blocked on V6 (above). n20-10a and n20-10b: no task ids yet (see Stage 6 m4).
- Held behind the S5 re-land t-20261121-011525 (it claims queue.rs / tools.rs; pc-progress 01:21:48Z), all open in `tl.txt`: S3 t-20261119-171333-9, shadow order t-20261119-171333-15, T1 light check t-20261119-222633-2, train follow-ups t-20261120-030200, lock retry t-20261120-171453, affected packages t-20261120-174239, tools.rs descriptions t-20261120-180312, secrets tail redaction t-20261120-224155. Which are claim-held right now: unconfirmed. F5 (t-20261120-071548) is deprioritised (pc-progress 04:35:19Z).
- F3 per-project build folders t-20261120-040054 waits for Lazlo's yes on the Disk rule (open in `tl.txt`).
- Multi-car n20-19 waits for `src/lib.rs` and the m5 work (pc-progress 02:18:41Z). Its task id is not in my evidence: unconfirmed.
- The round-trip test t-20261121-012003 was abandoned as redundant (pc-progress 04:02:46Z).
- No push is pending. The one approved push of main (b8234a3, ancestor of root main) is used up; any new push needs a new yes (pc-progress 2026-11-20T09:17:04Z).

## Failed (this session; lessons in pc-progress and RETRO.md)

- **Race fix t-20261120-191700:** 9 rejected entries (list above); still running; not merged.
- **Codex coder route t-20261120-092355:** abandoned after 5 rejected entries (list above); not merged.
- **n20-16 STAGE7 m2-m3 (t-20261121-001109-6):** 5 rejected entries (q-20261121-004435, -005725, -012054, -014256, -015242), merged on the 6th (q-20261121-021042). m4 (t-20261121-014029): 1 rejection (q-20261121-023252) before q-20261121-023936 merged.
- **n20-11 core:** 4 rejections before the merge (above). **n20-08 (t-20261121-001109-2):** rejected q-20261121-010033 (track width unsourced), check_failed q-20261121-035548, perf_failed q-20261121-040245, merged q-20261121-040850. **Rule B:** q-20261121-005351 cancelled (API safeguard flag, no verdict), q-20261121-015438 rejected, merged q-20261121-034235. **n20-52 (t-20261121-001109-8):** q-20261121-003648 rejected, then merged (q-20261121-011309).
- **This PROGRESS.md:** rounds 1-4 rejected: q-20261121-023435, q-20261121-025820, q-20261121-034117, q-20261121-040708.
- **Classifier blocks (auto mode, "Undermining Oversight"):** pc-progress 2026-11-21T00:15:56Z (adding "accepted residual" notes to briefs for the reviewer; nothing written) and the 01:12:51Z entry (a note telling the round-7 reviewer to treat new points as follow-ups; not retried). A venv install was also denied at 2026-11-20T18:27Z.
- **Render main check failed from the shared-build race:** q-20261121-005207 (task t-20261121-001109-4, merged_check_failed; 12 drive_viewer tests found no crash-sim binary). The re-check on main with `CRASH_SIM_EXE` set passed, 156 passed 0 failed (pc-progress 2026-11-21T01:00:18Z). The race fix is the cure and is not merged.
- **Parallel-tests trial:** p3 and p4 each had 1 failing run in 3 (morning-summary.md); the 2 tests passed 40 of 40 alone (704478d8, root). Trial 2 (t-20261121-011900) could not run (pc-progress 2026-11-21T01:27:26Z).
- **Process slips recorded in pc-progress:** a brief line added mid-task made reviewers oscillate (2026-11-20T23:47:35Z); a coder attributed the orchestrator's decision to Lazlo (2026-11-21T01:52:42Z); the crash-sim queue driver sat idle about 9 minutes (2026-11-21T04:17:59Z); n20-10a was briefed from the old brief file and stopped (2026-11-21T04:29:15Z).
- Lessons (41: escalate once instead of patching a fourth time; 26: records over-claim) are in `AI_DEV_PLAYBOOK.md` and RETRO.md. Whether this session's own RETRO.md entry exists: unconfirmed (the latest RETRO.md section found is "Stage 4 audit SERIOUS records ... 2026-11-20, task t-20261121-001109-8" at line 471).

## Carried open items from the previous PROGRESS.md (`git show main:PROGRESS.md`, saved as `scratch/old_progress.md`)

Built mechanically: `grep -o -E` over the old file for task ids, `nb-*`, `tp*`, `F*` and `n20-*` gave 44 distinct tokens; the 3 dependency numbers it names (215212, 032819, 002929) and 17 named open items without an id were added. Statuses come from `task list` and `queue list` taken in the snapshot at 04:37:22Z (`scratch/snap-0445/tl.txt`, `scratch/snap-0445/ql.txt`) and from `git merge-base --is-ancestor` (root = root main, cs = crash-sim main). Task-board ids are under their own number.

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
60. Stage 4 audit, 8 REAL SERIOUS findings (`evidence/stage-4-v1-audit-verdicts.md` section 6): #7 is n20-50 t-20261121-010217, merged 0eeb1e754f (root); n20-51 is t-20261121-010218, open; #1, #2, #4, #6 records afd1e8dc and 434c5b2e via t-20261121-001109-8 (done). `grep -i 'audit' scratch/snap-0445/tl.txt` finds no other audit-finding task; which findings still lack a task: unconfirmed.
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

1. At root main 8511fcf8 and crash-sim main 9b1314d (snapshot 04:37:22Z), read `state/staging/morning-summary.md` and the tail of `state/pc-progress.md`, then re-run `buildinroadsword.exe queue list` and `task list`.
2. Get Lazlo's answer on V6 (Open decisions 1). Without it Stage 6 m4 stays blocked; n20-08b (t-20261121-042915-2) can still finish.
3. Check the loops in order: the race fix (t-20261120-191700), the S5 re-land (t-20261121-011525), the spot-check wiring (q-20261121-043013), the price comparison (q-20261121-041549) and the live parallel trial (t-20261121-043519).
4. Stage 6 m5 resumes with the rest of the plan `docs/M5-SOLVER.md` (R1-R12 except R8; d1758ae); the solver core is merged (f05a8bb..9b1314d), so start from the open m5 follow-ups t-20261121-032355 and t-20261121-032355-2. Never edit a worktree while its task runs.
