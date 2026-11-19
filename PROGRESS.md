# Progress

## PAUSE HANDOFF 2 (2026-11-19, about 23:45 UTC, restart to try Claude Code's experimental code mode: read this first)

Paused by Lazlo's choice via the coordinator (option 1: try code mode in the desktop app). Before restarting, Lazlo sets the user env var himself (this session's `setx` was refused by its permission check): `setx CLAUDE_CODE_EXPERIMENTAL_CODE_MODE 1`; undo: `reg delete HKCU\Environment /v CLAUDE_CODE_EXPERIMENTAL_CODE_MODE /f`.

State at pause: queue idle (the last running entry finished; 10 queued entries cancelled), no coder or ahead agent running, inbox poll deleted, queue driver exited. max_agents = 14. The RAM/CPU sampler (scratch/scale/sys-14-resume.csv) dies with the session.

**First check after restart:** is code mode active in the new session (env var visible to claude.exe; the session reports code mode)? Log the answer in state/pc-progress.md. Then the code-mode trial step 2 (before/after on a few comparable coder tasks: tokens, wall time, context size, review pass rate, bugs; keep only if better).

**Resume in this order:**
1. Log `RESUMED <UTC>` in state/pc-progress.md; read state/pc-inbox.md.
2. Re-add the queue entries in order from state/staging/records/pause-requeue-2026-11-19-b.tsv (entry, task, risk, coder) with `queue add <task> --risk <risk> --coder <coder> --brief scratch/briefs/<task>.md`. If scratch/briefs was wiped, copy from state/staging/records/pause-briefs-b/ (those copies carry the appended review verdicts for nb-21 and tp3). nb-33 plastic beams t-20261119-171333-32 MERGED at 23:46Z (af05500) during pause 3, so it is no longer in the list; t-20261119-215207 (src/main.rs) is now ready. Then start ONE driver: scratch/queue-driver.ps1 (sets BROADSWORD_REVIEWER_CLI itself).
3. Restart the 5-minute inbox poll and the sampler (scratch/scale/sampler.ps1 -Out scratch/scale/sys-14-resume2.csv).
4. Ahead-work paused (resume oldest first): t-20261119-232113 at state/staging/t-20261119-232113/pc-ahead (canary part done; openai_review.py gate + $20/day cap written, untested; tests to write are in its NOTES.md) and t-20261119-232113-2 at state/staging/t-20261119-232113-2/on-32797d3 (config/toml done, queue.rs half, uncompiled; add tests/support/mod.rs to its claims).
5. t-20261119-215207 (crash-sim stdout write) is ready once nb-33 frees src/main.rs.

**Decisions since the last handoff (all in state/pc-progress.md):** full GPT reviews use gpt-6.1-sol, not astra (Lazlo); GPT reviews may use up to $20/day of overflow credits (Lazlo), enforced by t-20261119-232113 part 2 once merged; gpt-6.1-sol second-gate trial boarded (t-20261119-232113-2), ships off, on after the credit reset; scrub follow-up t-20261119-233612 boarded; shadow canary re-run 23:20Z PASS on codex-cli 0.170.0.
**Merged this session:** t-20261119-215214 (bcbe897), nb-32 t-20261119-171333-30 (f37e17a), n20-03 t-20261119-223123-3 (60d2300), n20-01 t-20261119-223123 was REJECTED at 23:41Z by haiku (q-20261119-231529-4); its Opus fix round started and paused before any edit: plan in the brief's Paused section (state/backlog/t-20261119-223123.md).
**Camry reference:** another session downloaded reference/camry-2013/ccsa/2012-toyota-camry-detailed-v5a.zip (58.5 MB, complete) and was fetching 2015-toyota-camry-bm-v1.zip (.part); check whether the .part finished or was cut by the restart.

---

## PAUSE HANDOFF (2026-11-19, about 23:02 UTC, Lazlo's restart: read this first)

Paused by the inbox item 2026-11-19T22:56Z. State at pause: queue idle (0 queued, 0 running), no broadsword/cargo/rustc process, no build or recover lock held, no coder or ahead agent running, the inbox loop deleted, the queue driver and the RAM/CPU sampler stopped. max_agents = 14 (step 2 of the slot climb; not stepped to 18: about 7 merges/h vs 8.44/h at 10, see state/pc-progress.md).

**Resume in this order:**
1. Log `RESUMED <UTC>` in state/pc-progress.md, mark the 22:56Z PAUSE item [x] in state/pc-inbox.md, read the inbox.
2. Re-add the 9 queue entries cancelled for the pause, in this order (list in state/staging/records/pause-requeue-2026-11-19.tsv: entry, task, risk, coder): `broadsword queue add <task> --risk <risk> --coder <coder> --brief scratch/briefs/<task>.md` (if scratch/briefs was wiped, copy state/backlog/<task>.md there first). Then start ONE queue driver (set BROADSWORD_REVIEWER_CLI per command).
3. Resume the two paused coders from their briefs' "## Paused (2026-11-19)" sections: tp3 fix round t-20261118-095350-3 (WIP 32797d3: scrub via one cargo factory, cargo test order; next: full suite twice, lock-wait re-measure, then queue --risk high, Opus) and n20-23 menu t-20261119-223123-9 (WIP c769207; next: fails-before, full suite, final commit).
4. Fix round for nb-21 t-20261119-171333-19 (rejected 23:00Z by haiku, q-20261119-222244: claim-breach path when codex errors; read the .review/.shadow.review, apply rule 34 if the points refute).
5. Restart the inbox loop (every 5 min) and the sampler (scratch/scale/sampler.ps1 -> a new sys-*.csv).

**Approved by Lazlo, not yet acted on:** the n20-02 follow-up may read public pages (Euro NCAP, IIHS, SAE, OLC papers, steel data; no accounts or fees, report anything paywalled) to verify the 9 unverified V-figures; Lazlo is checking the NHTSA public-domain wording in a browser.

**Waiting work:** about 17 patches staged as ahead-work in state/staging/ (ledger state/staging/AHEAD.md); most sit on tp3's first tip d30c74e and need porting onto tp3's fixed tip. The merge train (t-20261119-222633) waits for tp3, tp4, nb-17, nb-18. Trial T1 boarded (t-20261119-222633-2); T2-T5 wait for T1's verdict. Six GPT trial-coder batch tasks wait for nb-21. 8 roadmap briefs boarded, not started. state/cleanup-pending.md (32 lines) awaits Lazlo's yes; scratch/autoclean.py runs every 2 h.

---

Rewritten at the end of each session. Last updated 2026-11-19, about 21:30 UTC, on the EVO-X3 PC (the PC session). The session covered: the work-ahead batch (46 briefs boarded), Lazlo's model routing, the Stage 4 v1 audit and close, a Secrets fix to the Codex shadow, the secret-file rule, the max_agents scale-up test, and many tooling merges. Step by step log: `state/pc-progress.md` (outside Git). Lazlo steers through `state/pc-inbox.md`: read it after every step.

## Moving to another drive or PC

Read `docs/MOVING.md` (also in `state/MOVING.md`). On this PC: `broadsword` is not on PATH; use `build/bin/broadsword.exe`. `BROADSWORD_REVIEWER_CLI` is not set system-wide; queue runs set it per command to `C:\Users\MrComputer\.local\bin\claude.exe`. Claude's notes were restored from `state/claude-memory/` into this PC's memory folder.

**Backups:** pushed to the private repo `Ryvexuz/Broadsword` on 2026-11-19 (Lazlo's explicit OK): `main` at c248537 then, `crash-sim-main`, `render-master`, tags `stage-4-v1-done` and `render/stage-4-v1-done`. Main has moved since (the allow-lists and nb-19); push again only with Lazlo's OK. `scripts/backup_status.py --map <map>` reports what is unpushed (read-only).

## Where things stand

- **Stages 1, 2, 3 and 4 v1 done and tagged** (`stage-4-v1-done` on main 4d49571 and render 9e1060b). Evidence: `evidence/stage-4-v1-done.md`. Milestone 1 shown in an on-screen window (window-only captures); milestone 2 re-measured: 6.05 ms per frame through the present path (about 165 fps, target 60). Stage 0 tag still needs a fresh app session.
- **Stage 4 audit:** Fable (28 findings), gpt-6.2-astra twice (220, 228), five Opus 5.5 high judges, merged: `evidence/stage-4-v1-audit-verdicts.md` (477 ids, each once). Two REAL BLOCKING, both resolved (A1 on-screen evidence, A6 shadow could read files). 8 REAL SERIOUS listed in section 6 as follow-ups (not yet filed as board tasks).
- **Model routing (Lazlo, 2026-11-19):** main coders Sonnet 5.5 and Opus 5.5 high; trial coders gpt-6.1-terra ultra, gpt-6.2-astra, gpt-6.2-sol (short, tight tasks; nb-22, also gpt-6.1-sol high per the project thread; check terra "ultra" is not a paid tier first); shadow reviewers gpt-6.1-sol, gpt-6.2-astra, gpt-6.2-sol (never gating; every shadow finding gets REAL/FALSE/KNOWN from the gating reviewer, REAL ones become tasks); Sonnet, astra, 6.1-sol, 6.2-sol compete for the reviewer slot in nb-24; Lazlo approves promotions. **Fable only as a rare one-off**, reason logged.
- **Shadow reviewer:** back on, gpt-6.1-sol only (the multi-shadow task t-20261118-142614 is open). Codex now runs with no tools offered (model catalog + flags, `codex_command()` in clients/openai_review.py); `scripts/shadow_canary.py` proves it (offline leg + one real call). Re-run the canary after every Codex update (not yet enforced). Shadow tally so far: 13 REAL, 12 KNOWN, 3 FALSE (`state/shadow-verdicts.tsv`, plus 2 later).
- **Secret files (Lazlo's set):** `.env`, `.envrc`, `.env.<name>`, `.env-*`, `.env_*` (bare `.env-`/`.env_` too), any case, trailing dots/spaces dropped, plus 8.3 short names and NTFS streams of these; `.envoy.yaml`, `.environment`, files in a `.env/` folder are ordinary. One rule (`git::is_secret_in`, merged 62f458d); records escape `|`; recover names every secret it leaves alone.
- **max_agents:** 6. A step to 10 raised merges/h 2.82 -> 8.44 but lock-wait p90 26 -> 88 s and test median 82 -> 100 s (baseline.py: WORSE), so reverted by Lazlo's rule. Retry after tp3. Evidence draft: `state/staging/scale-up/scale-up-2026-11.md`.

## Merged today (main, queue unless noted)

- Tooling: scripts/baseline.py (nb-06, direct merge on Fable PASS, Lazlo), codex wrapper minors (nb-16), codex time budget from env (t-...175203), codex no-tools sandbox + canary (t-...190059), backup_status.py (nb-14), archive_evidence.py (nb-15), main_provenance.py (nb-10; allow-list `docs/direct-merges.txt`, 19 unaccounted older commits remain), roadmap_status.py + ROADMAP.md (nb-45), shared-core ADR draft (nb-46, Proposed, Lazlo decides), secret-file rule (nb-19, direct merge on six Sonnet security reviews, Lazlo).
- Benchmarks: hard1 reviewer case (scorer anchored by file; decoys never credit bugs), coding bench (Myers reference, no spec leaks, forge-proof harness), instruction-following bench (Haiku 19/20).
- Crash sim: Stage 6 done checks `docs/STAGE6.md` (nb-30), Solver trait (nb-31), `.bsn` format + `.gitattributes` LF (nb-33).
- Render: near-plane shared-edge clip and OBJ normals (nb-04).
- Direct records/safety commits: cfadd9e (shadow off, safety stop), Stage 4 close 4d49571, merge of GitHub main c248537, allow-lists.

## Unfinished

- **Open board tasks:** 45 (`broadsword task list --status open`); briefs in `state/backlog/`. Ready next for crash sim: nb-32 (settle report), then nb-34..36 (milestone 2). Codex coder route nb-21 before the nb-22 trial.
- **Follow-ups boarded today:** t-20261119-200000 (secret content check for staged blobs), t-20261119-185546 (queue passes the shadow time budget; drop legacy defaults), t-20261119-192623 (backup_status minors: URL credentials, real no-force guard), t-20261119-193538 (archive_evidence minors), t-20261119-203616 (flaky queue kill test). Not yet boarded: nb-33's format minors (rest overflow, CR in comments), nb-10's minors, the 8 SERIOUS audit findings.
- **tp1** (scoped queue checks) is abandoned to free config.rs; its branch, worktree and staged patch are kept for a restart. tp2-tp6 still open.
- **Leftover worktrees** kept because an ignored file blocks removal (deleting outside scratch needs Lazlo): nb-06 (t-20261119-170109, a .pyc), nb-19 (t-20261119-171333-17).

## Failed (all recorded in RETRO.md, 2026-11-19 entry)

- A6: the Codex shadow could read any file (canary); fixed and proven.
- The orchestrator's safety-stop commit broke a test on main (no full suite before a direct commit).
- A `prove.py` run nested `broadsword run` and deadlocked builds for 15 minutes.
- Board claim edits silently failed (Python through a shell heredoc; playbook rule 23).
- Fable used by default twice (about 990k avoidable tokens).
- Haiku loops on Python tooling (nb-06, nb-25, the allow-lists): rule 34 applied each time.

## Next step

1. Read `state/pc-inbox.md`; run open items.
2. File the 8 SERIOUS Stage 4 audit findings as board tasks (`evidence/stage-4-v1-audit-verdicts.md` section 6) and the unboarded minors above.
3. Carry items from RETRO (2026-11-19): enforce the canary after Codex updates; a full check before any direct commit; the coder-prompt line about nested broadsword runs; the SPEC_CHECKLIST line about on-screen evidence; retry max_agents 10 after tp3.
4. Crash sim, Stage 6 milestone 1: nb-32 settle report next (STAGE6.md check 1.3-1.5), then milestone 2.
5. The tp chain (tp2, tp3, tp4, tp6) and t-20261119-185546 (claims config.rs/tests/toml; run them one after another).
