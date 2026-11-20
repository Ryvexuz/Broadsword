# Reviewer consistency bench, hard1, 2026-11 (task t-20261119-171333-22)

**Status: PARTIAL. The Claude half is done (Sonnet 5.5, 5 of 5 runs). The GPT half (20 runs) is held by the daily GPT cap and has not run.** The done check is not met yet; see "Held: GPT half" below. Nothing in this file is a GPT score.

## Setup

- Case: `hard1` (12 planted bugs b01-b12, 3 decoys d1-d3), scored by `bench/reviewer/score_hard.py` unchanged. Diff: `bench/reviewer/cases/hard1.diff`, made by `bench/reviewer/hard/make_diff.sh` (21,818 bytes, 701 lines; the folder is git-ignored, so it is not committed).
- Prompt: `run.py`'s `SYSTEM` and the case brief plus diff, identical for every runner.
- Runner for Sonnet: `bench/reviewer/run.py`, headless Claude Code, `--tools ""`, no MCP, `BENCH_CWD` outside the project (`C:\Users\MrComputer\AppData\Local\Temp\bench-cwd`).
- Model argument: `claude-sonnet-5-5`. Answering model checked from `modelUsage` in each raw file: `claude-sonnet-5-5` in all 5 runs (playbook lesson 32).
- Raw results: `state/bench-results/consistency/run1` to `run5`, one `hard1__claude-sonnet-5-5__default.json` and `.raw.json` per run folder (5 JSON + 5 raw files).
- Scorer output: `scratch/score-sonnet-run{1..5}.txt` (summaries) and `scratch/score-sonnet-run1-v.txt` (verbose).
- Brief spec note: `run.py` names its output `<case>__<model>__<effort>`, so one folder can hold one run only. Each repeat run therefore has its own folder (`run1`..`run5`).
- The brief names `state/reviewer-verdicts.tsv`; that file does not exist. The live-merge verdicts are in `state/shadow-verdicts.tsv` (59 rows, see the live-merge section).

## Per-model table, hard1 (out of 12 bugs)

| Model | Runs | Mean | Min | Max | Spread | Decoy FPs (per run) | Extra (other) | Format failures | Seconds (mean) | Tokens per run (mean) | Cost, CLI-reported |
|---|---|---|---|---|---|---|---|---|---|---|---|
| claude-sonnet-5-5 | 5 of 5 | 8.8 | 8 | 10 | 2 | 1, 0, 0, 0, 0 (d1 only, run 1) | 0 | 0 of 5 | 16.0 | 16,712 | $0.1527 total |
| gpt-6.2-sol | 0 of 5 | held | | | | | | | | | |
| gpt-6.2-astra | 0 of 5 | held | | | | | | | | | |
| gpt-6.1-sol | 0 of 5 | held | | | | | | | | | |
| gpt-5.6-sol | 0 of 5 | held | | | | | | | | | |

Per Sonnet run:

| Run | Caught (BLOCKING) | Minor-only | Missed | Decoy FPs | Other | Output tokens | Input + cache tokens | Cost $ | Seconds |
|---|---|---|---|---|---|---|---|---|---|
| 1 | 10 | 1 (b10) | 1 (b12) | 1 (d1) | 0 | 1,436 | 15,214 | 0.0752 | 16.0 |
| 2 | 8 | 3 (b02, b10, b11) | 1 (b12) | 0 | 0 | 1,429 | 15,213 | 0.0185 | 15.8 |
| 3 | 8 | 4 (b02, b10, b11, b12) | 0 | 0 | 0 | 1,581 | 15,212 | 0.0200 | 15.7 |
| 4 | 9 | 3 (b02, b10, b12) | 0 | 0 | 0 | 1,494 | 15,211 | 0.0191 | 15.9 |
| 5 | 9 | 2 (b10, b11) | 1 (b12) | 0 | 0 | 1,558 | 15,213 | 0.0198 | 16.7 |

Token note: input + cache tokens are `input_tokens` + `cache_creation_input_tokens` + `cache_read_input_tokens` from the CLI usage block. Run 1 wrote the prompt cache (15,212 creation tokens, 0 read); runs 2-5 read it. Run 1's cost is higher for that reason.

Spread: 10 - 8 = 2, so by the brief's rule (2 or less) Sonnet 5.5 is **consistent** on hard1 (n = 5).

## Hand-check of `missed` and `other` lines

Every BLOCKING and MINOR line in the five raw reviews was read against the key. Results:

- **`other` = 0 in all five runs.** Every BLOCKING line matched a bug or the decoy it names. Run 5 also has a MINOR line that names no bug (`store.rs:219`, "no unit tests for lease expiry"); it is a MINOR line, not counted as other, and it is a fair remark.
- **Missed b12 (wrong backoff, `retry.rs:23`, `wrapping_shl`), runs 1, 2 and 5.** No line in these raw reviews mentions the shift or the wrap. Run 1 raw lines: none about `wrapping_shl`, `shift` or `backoff`. Run 2 raw lines: none. Run 5 raw lines: none. These are true misses.
- **b12 in runs 3 and 4 (minor-only, not missed).** Run 3: `MINOR: src/retry.rs:63 - ... wrapping_shl also wraps for attempt >= 64.` Run 4: `MINOR: src/retry.rs:22 - wrapping_shl wraps for attempts of 64 or more. The attempt count is capped at 10, so this does not matter in practice.` The scorer credits both as minor-only. Hand judgement: the reviewer saw the wrap and rated it minor; the scorer is right to count it as minor-only, not caught.
- **d1 flagged in run 1 only.** `MINOR: src/lib.rs:45 - peek uses is_none() followed by unwrap() where a match would be cleaner.` That is a style remark on the decoy, so a decoy FP as the scorer says.
- **Minor-only b10 and b11 (retry test, config doc).** Matched by their own lines (`retry.rs:67/44/63/68/69`, `config.rs:41/37/36`). Scorer correct.

## Verdict per model

- **claude-sonnet-5-5: consistent** (spread 2, n = 5, mean 8.8). Its misses are the same bug each time (b12, in 3 of 5 runs) and it never raised an extra real problem.

## Held: GPT half

- gpt-6.2-sol, gpt-6.2-astra, gpt-6.1-sol and gpt-5.6-sol (20 runs) were **not run**.
- Reason: the daily GPT cap is reached for UTC 2026-11-20. `state/gpt-spend/CAP-REACHED-2026-11-20.txt` says: `estimated 20.1636 USD of 20 USD. No GPT call is made until the next UTC day or until gpt_daily_cap_usd is raised. Orchestrator: tell Lazlo.` The gate (`clients/openai_review.py` `cap_gate`) refuses every codex call before it starts. No call was made, and no refusal records were written.
- The cap is Lazlo's rule ($20 a day). It is not raised here.
- To resume after UTC midnight (2026-11-21 00:00Z), from `bench/reviewer/` in the worktree, once per run folder `N` = 1..5, for each model in `gpt-6.2-sol gpt-6.2-astra gpt-6.1-sol gpt-5.6-sol`:
  `python -I -B run_codex.py <absolute path to state/bench-results/consistency/runN> <model> default hard1`
  (`hard1` restricts the run to this case). Then `python -I -B score_hard.py <run folder> -v`.

## Live-merge data (shadow verdicts, for the bake-off)

Source: `state/shadow-verdicts.tsv`, 59 finding verdicts over 30 queue entries, all dated 2026-11-19. Verdict totals: real 33, known 23, false 3. Judges: haiku (gating) 20, opus (gating) 1, orchestrator and mixed judges 38.

**Join method.** Each verdict row's `entry` was matched to `state/queue/<entry>.entry`. The shadow model is that file's `shadow_model` field, and `shadow_model_used` equals it in all 30 entries (lesson 32: no mismatch). Each finding was then attributed to a reviewer by checking whether its `finding_key` (file:line) appears in that entry's `state/queue/<entry>.shadow.review`. Every finding is attributed to the shadow model except one (below). The shadow model is the only reviewer attributed here. The gating reviewer's own findings are not split out, because the table does not say who raised each finding.

### Per-reviewer split (shadow reviewers in the live queue)

| Shadow reviewer | Entries | Findings (rows) | REAL | KNOWN | FALSE | False-alarm rate (FALSE / all) | Real rate (REAL / all) | Shadow tokens per entry, mean (median) | Shadow seconds per entry, mean (median) |
|---|---|---|---|---|---|---|---|---|---|
| gpt-5.6-sol | 12 | 24 | 10 | 11 | 3 | 3/24 = 12.5% | 10/24 = 41.7% | 49,394 (45,902) | 85.9 (112.5) |
| gpt-6.1-sol | 18 | 35 | 23 | 12 | 0 | 0/35 = 0% | 23/35 = 65.7% | 21,452 (14,772) | 23.4 (21.8) |
| gpt-6.2-sol | 0 | 0 | - | - | - | - | - | - | - |
| gpt-6.2-astra | 0 | 0 | - | - | - | - | - | - | - |
| Sonnet 5.5 | 0 | 0 | - | - | - | - | - | - | - |

Totals: 12 + 18 = 30 entries, 24 + 35 = 59 rows, which matches the file. REAL 10 + 23 = 33, KNOWN 11 + 12 + 0 = 23, FALSE 3 + 0 = 3.

Cost: the `shadow_cost_usd` field is blank in every entry, so no per-entry USD figure is available here. The GPT spend ledger (`state/gpt-spend/`) has per-call estimates, but this join does not use it.

Per-entry rows (the `REAL` / `FALSE` verdicts themselves are in the shadow-verdicts file; the rows here are the joined counts):
- gpt-5.6-sol entries: q-20261119-170206, 171712, 171935, 173033, 173952, 174115, 174836, 175739, 175346, 175821, 180353, 182610.
- gpt-6.1-sol entries: q-20261119-201906-2, 205504, 211149, 215916, 214719, 215254, 215840, 215947, 215947-2, 215948-2, 215949, 220003, 220412, 220839, 221115, 221115-2, 221838, 221903.

**Unattributed row:** `q-20261119-220839`, `spec_checklist.md:1`, verdict known, judge orchestrator. Its `finding_key` is not in the entry's shadow review text, so it is not counted for either model's totals beyond the 6.1-sol rows above (it is one of the 35). It does not change any FALSE count.

**Caveats.**
- Sample is small: 12 and 18 entries, all dated 2026-11-19. Both models ran only on the queue's shadow slot, so they do not face the same changes; the 5.6-sol entries are mostly Haiku-gated, and so are most of the 6.1-sol ones.
- Some entries have an Opus gating reviewer (`reviewer_model = opus`: 2 of the 5.6-sol entries and 6 of the 6.1-sol entries); the split above does not separate them by gate.
- gpt-6.2-sol, gpt-6.2-astra and Sonnet 5.5 have no shadow entries in this table, so their live-merge columns are empty, not zero.

## Ranked reviewer table and recommendation

**Not produced.** The live-merge split above exists for gpt-5.6-sol and gpt-6.1-sol only. The other three models have no live data and no GPT hard1 runs. Their hard1 consistency runs are held (see "Held: GPT half"). No recommendation is made; the current gating reviewer stays as it is (Lazlo approves any switch).

What the data so far shows: Sonnet 5.5 catches 8 to 10 of 12 on hard1 (consistent, 2-point spread). On the live shadow slot, gpt-6.1-sol gave 23 REAL and 0 FALSE over 35 findings, and gpt-5.6-sol gave 10 REAL and 3 FALSE over 24. These are two different sets of changes, so the two are not compared head to head.

What the Sonnet data alone shows: Sonnet 5.5 catches 8 to 10 of 12 on hard1, with no extra findings and one decoy FP in five runs, and it is consistent by the 2-point rule. Compare it with the GPT runs once they are in.
