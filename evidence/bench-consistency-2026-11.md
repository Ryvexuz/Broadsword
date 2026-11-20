# Reviewer consistency bench, hard1, 2026-11 (task t-20261119-171333-22)

**Status: complete for the five models (5 runs each, 25 raw results). The live-merge split covers gpt-5.6-sol and gpt-6.1-sol only; see the ranked table.**

## Setup

- Case: `hard1` (12 planted bugs b01-b12, 3 decoys d1-d3). Scored by `bench/reviewer/score_hard.py` unchanged; the hand-check below overrides its counts where stated. Diff: `bench/reviewer/cases/hard1.diff`, made by `bench/reviewer/hard/make_diff.sh` (21,818 bytes, 701 lines; git-ignored, not committed).
- Prompt: `run.py`'s `SYSTEM` plus the case brief and diff. Identical for every runner. Input is about 7.7k tokens per call in every run.
- Runners: Sonnet 5.5 through `run.py` (headless Claude Code, `--tools ""`, no MCP, `BENCH_CWD` outside the project). The four GPT models through `run_codex.py` (ChatGPT plan, no tools offered).
- Raw results: `state/bench-results/consistency/run1` to `run5`. Each folder holds one result per model: 5 models x 5 runs = 25 `.json` plus 25 `.raw.json`.
- Run folders are per run because `run.py` and `run_codex.py` name their output by model, not by run.
- Answering model: Sonnet's `modelUsage` shows `claude-sonnet-5-5` in all 5 runs. Codex results carry an empty `model_used` and `model_reported: false` in all 20 runs, so the answering model **cannot be shown** for the GPT runs (playbook lesson 32). The model asked for is the one in the file name.
- The first GPT attempt (gpt-6.2-sol run 1) was refused by the canary gate. That refusal's result files were overwritten by the successful run 1 (same name). The refusal is recorded in the coordinator thread, not in the raw folder.
- The brief names `state/reviewer-verdicts.tsv`; that file does not exist. The live-merge data is `state/shadow-verdicts.tsv`, joined through `state/queue/<entry>.entry`.

## Hand-check method

The scorer undercounts. Its regexes miss several correct phrasings (for example "half-open range rejects `max_attempts = 10`", "parent-directory components", "claim forgets the lock guard"). So every `missed` and `other` line was read in the raw review and judged against the key:

- **Caught by hand:** a BLOCKING line at the bug's location (file and line within the item's range) that describes the planted behaviour. The scorer's miss is listed.
- **Minor-only by hand:** a MINOR line at the bug's location that describes the bug.
- **Extra (REAL):** a BLOCKING line about something that is not in the key, where the buggy source confirms the behaviour. Checked by reading the code path in `bench/reviewer/hard/crate/buggy/src/` (store.rs, paths.rs, lib.rs, config.rs). These are real properties of the code, not planted bugs.
- **Extra (UNSUP):** a BLOCKING line not in the key that the spec does not support as a defect.

No decoy was flagged by any GPT run (d1 `lib.rs:40-47`, d2 `config.rs:30-34`, d3 `log.rs:37-41`). Sonnet flagged d1 once (MINOR `lib.rs:45`).

## Per-model table, hard1 (out of 12 bugs, hand-checked)

| Model | Runs | Mean | Min | Max | Spread | Scorer mean (min-max) | Decoy FPs (5 runs) | Extras REAL / UNSUP (5 runs) | Format failures | Seconds, mean (min-max) | Tokens per run, mean | Cost per run | Verdict |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| claude-sonnet-5-5 | 5 | 8.8 | 8 | 10 | 2 | 8.8 (8-10) | 1 (run 1) | 0 / 0 | 0 of 5 | 16.0 (15.7-16.7) | 16,712 (in+cache+out) | $0.031 (CLI-reported; $0.153 total) | consistent |
| gpt-6.2-sol | 5 | 9.0 | 9 | 9 | 0 | 7.0 (6-9) | 0 | 23 / 3 | 0 of 5 | 53.2 (48.9-57.0) | 10,172 (in+out) | $0.53 (ledger est.; $2.65 total) | consistent |
| gpt-6.2-astra | 5 | 9.0 | 9 | 9 | 0 | 7.8 (6-9) | 0 | 21 / 0 | 0 of 5 | 17.3 (15.6-19.7) | 8,342 (in+out) | $0.31 (ledger est.; $1.55 total) | consistent |
| gpt-6.1-sol | 5 | 9.0 | 9 | 9 | 0 | 9.0 (9-9) | 0 | 0 / 0 | 0 of 5 | 30.1 (26.4-33.2) | 8,922 (in+out) | $0.38 (ledger est.; $1.90 total) | consistent |
| gpt-5.6-sol | 5 | 9.0 | 9 | 9 | 0 | 9.0 (9-9) | 0 | 0 / 0 | 0 of 5 | 43.9 (28.4-49.3) | 9,942 (in+out) | $0.50 (ledger est.; $2.51 total) | consistent |

Notes on the columns:
- "Scorer" is `score_hard.py` as it ran. Hand-check moved the GPT 6.2 numbers up: gpt-6.2-sol 6, 9, 6, 7, 7 became 9 in every run; gpt-6.2-astra 8, 6, 7, 9, 9 became 9 in every run.
- The scorer alone would call gpt-6.2-sol not consistent (spread 3). Hand-checked, it is consistent on catches. The earlier "6.2 may be inconsistent" lead is not supported on catches. Its extras vary a little (per run 6, 5, 5, 6, 4) but are real.
- Cost: Sonnet's cost is CLI-reported on the Claude plan. GPT costs are the `est_usd` values in `state/gpt-spend/2026-11-20.tsv` for these 20 calls (caller `unknown`, from 23:01Z to 23:13Z). They are estimates, not invoices. Ledger totals for the 20 bench calls: $8.61. The day's ledger sum after the runs: $52.57 of the $60 cap.
- Tokens: GPT figures are `input_tokens + output_tokens` from the codex `turn.completed` usage. Sonnet's figure includes the prompt-cache tokens.

### Per run, hand-checked

Caught = hand count; Minor = bugs seen only as MINOR; Missed = bugs not in any BLOCKING or MINOR line.

| Model | Run | Caught | Minor-only | Missed | Decoy FP | Extras (REAL / UNSUP) | Scorer caught |
|---|---|---|---|---|---|---|---|
| claude-sonnet-5-5 | 1 | 10 | b10 | b12 | d1 | 0 / 0 | 10 |
| claude-sonnet-5-5 | 2 | 8 | b02, b10, b11 | b12 | 0 | 0 / 0 | 8 |
| claude-sonnet-5-5 | 3 | 8 | b02, b10, b11, b12 | none | 0 | 0 / 0 | 8 |
| claude-sonnet-5-5 | 4 | 9 | b02, b10, b12 | none | 0 | 0 / 0 | 9 |
| claude-sonnet-5-5 | 5 | 9 | b10, b11 | b12 | 0 | 0 / 0 | 9 |
| gpt-6.2-sol | 1 | 9 | b10, b11 | b12 | 0 | 5 / 1 | 6 |
| gpt-6.2-sol | 2 | 9 | b10, b11 | b12 | 0 | 5 / 0 | 9 |
| gpt-6.2-sol | 3 | 9 | b10, b11 | b12 | 0 | 4 / 1 | 6 |
| gpt-6.2-sol | 4 | 9 | b10, b11 | b12 | 0 | 5 / 1 | 7 |
| gpt-6.2-sol | 5 | 9 | b10, b11 | b12 | 0 | 4 / 0 | 7 |
| gpt-6.2-astra | 1 | 9 | b10, b11 | b12 | 0 | 4 / 0 | 8 |
| gpt-6.2-astra | 2 | 9 | b10, b11 | b12 | 0 | 4 / 0 | 6 |
| gpt-6.2-astra | 3 | 9 | b10, b11 | b12 | 0 | 4 / 0 | 7 |
| gpt-6.2-astra | 4 | 9 | b10, b11 | b12 | 0 | 4 / 0 | 9 |
| gpt-6.2-astra | 5 | 9 | b10, b11 | b12 | 0 | 5 / 0 | 9 |
| gpt-6.1-sol | 1 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-6.1-sol | 2 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-6.1-sol | 3 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-6.1-sol | 4 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-6.1-sol | 5 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-5.6-sol | 1 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-5.6-sol | 2 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-5.6-sol | 3 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-5.6-sol | 4 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |
| gpt-5.6-sol | 5 | 9 | b11 | b10, b12 | 0 | 0 / 0 | 9 |

### Hand-check evidence (quoted raw lines)

- **b12 (wrong backoff, `retry.rs:23`, `wrapping_shl`)** is missed in every GPT run and in Sonnet runs 1, 2 and 5. No GPT raw review mentions a shift, wrap or `wrapping_shl`. Sonnet run 3 raw: `MINOR: src/retry.rs:63 - ... wrapping_shl also wraps for attempt >= 64.` (minor-only). Sonnet run 4 raw: `MINOR: src/retry.rs:22 - wrapping_shl wraps for attempts of 64 or more ... does not matter in practice.` (minor-only).
- **b10 (test that can never fail)** is minor-only in the 6.2 models and Sonnet, and missed in 6.1-sol and 5.6-sol. The 6.2 lines, quoted: 6.2-sol run 1 `MINOR: src/retry.rs:76 - The backoff-cap test never runs its assertion because the range starts at 10`; 6.2-astra run 4 `MINOR: src/retry.rs:76 - The cap test iterates over 10..8, so it passes without checking a single capped delay.` The scorer missed these phrasings ("passes without checking"), so they are minor-only by hand. The 6.1-sol and 5.6-sol raw reviews have no line about `retry.rs`.
- **b01 (`max_attempts = 10` rejected)**: the scorer missed it in several 6.2 runs. Quoted: 6.2-astra run 1 `BLOCKING: src/config.rs:63 - The half-open range rejects max_attempts = 10, which is a valid setting.` and 6.2-sol run 4 `BLOCKING: src/config.rs:63 - The parser rejects max_attempts = 10 even though 10 is in the required range.` Both caught by hand.
- **b09 (lock left behind)**: scorer missed it in 6.2-sol runs 1 and 3 and 6.2-astra run 2. Quoted: 6.2-sol run 3 `BLOCKING: src/store.rs:157 - claim forgets the lock guard and releases the lock only on success ...` Caught by hand.
- **b05 (ack ignores save error)**: scorer missed it in 6.2-sol run 3. Quoted: `BLOCKING: src/store.rs:182 - ack returns success even when saving the finished state fails` Caught by hand.
- **b03 (path traversal)**: scorer missed it in 6.2-sol run 3 and 6.2-astra run 3 ("parent-directory components", "for traversal"). Quoted: 6.2-astra run 3 `BLOCKING: src/paths.rs:27 - Only the first path component is checked, so an ID containing later parent-directory components can place a record outside the queue folder.` Caught by hand.
- **b08 (inverted lease guard)**: scorer missed it in 6.2-sol run 1 and 6.2-astra run 2. Quoted: 6.2-astra run 2 `BLOCKING: src/store.rs:162 - The comparison allows another worker to claim a job before its lease expires but rejects that worker after expiry.` Caught by hand.
- **b11 (doc says unknown keys ignored)**: minor in all GPT runs. Quoted: `MINOR: src/config.rs:39 - The parser documentation says unknown keys are ignored, but ... rejects` Scorer agrees.
- **Decoys**: no GPT raw line names `lib.rs` peek (d1), `config.rs` parse cast (d2) or `log.rs` write error (d3).

### Extra findings (BLOCKING, not in the key)

Verified against `bench/reviewer/hard/crate/buggy/src/`:

| Finding | Where | Verdict | Evidence |
|---|---|---|---|
| Overwrite truncates the record before writing (`fs::write`), so an interrupted write destroys the job | `store.rs:206-207` (save) | REAL | `save` calls `fs::write(path, ...)` directly (store.rs:206-208) |
| Symlinked directory followed by `collect` / enqueue writes outside the queue | `store.rs:107-108` (collect), `store.rs:138` / `paths.rs:30` (enqueue) | REAL | `path.is_dir()` follows symlinks (store.rs:108); `create_dir_all` and `fs::write` follow them (store.rs:138-145) |
| Retries reuse the `now` from before the backoff, so a lease can be expired when granted | `lib.rs:52` | REAL | `claim_next` calls `self.store.claim(&id, now)` inside `retry(...)` with a fixed `now` (lib.rs:52) |
| Lock file survives a killed process; no stale-lock recovery | `store.rs:217-221` | REAL | `.lock` is created with `create_new` and only `Drop` / `release` remove it (store.rs:215-226) |
| Trailing-slash ID `group/` writes `group/.job`, which `collect` skips (no extension) | `paths.rs:30`, `store.rs:110` | REAL | `record_path` joins `{id}.job` (paths.rs:30); `collect` filters on `extension() == "job"` (store.rs:110) |
| IDs `a` and `a.job/b` cannot coexist | `paths.rs:30`, `store.rs:138` | REAL | the record of `a` is `a.job`, a file that blocks the directory `a.job` needed by `a.job/b` |
| Enqueue publishes the record path before the write finishes, so a concurrent claim can see a partial record | `store.rs:144-145` | REAL | `put` calls `fs::write` on the final path (store.rs:145) |
| ID `.lock/task` creates a directory at the lock path and blocks all claims | `store.rs:140` | REAL | `put` calls `create_dir_all(dir)` on `root/.lock` (store.rs:138-139); `lock()` then fails with `Locked` (store.rs:219-224) |
| `claim_next` retry budget restarts per candidate, so one call can exceed `max_attempts` | `lib.rs:52` | UNSUP | `retry` is per id (lib.rs:52); the spec states no total cap across candidates, so not counted as a defect |

Counts of extras by run, total for each model: gpt-6.2-sol 23 REAL + 3 UNSUP (26 lines); gpt-6.2-astra 21 REAL (all REAL); gpt-6.1-sol and gpt-5.6-sol none; Sonnet none.

## Verdict per model (consistency, spread of 2 or less)

- **claude-sonnet-5-5: consistent** (spread 2, mean 8.8). Its variance is in b02 (BLOCKING in runs 1 and 5, MINOR in 2-4) and b12 (missed or minor in all runs).
- **gpt-6.2-sol: consistent** (spread 0 on catches; 4-6 extras per run, 3 of 26 unsupported).
- **gpt-6.2-astra: consistent** (spread 0 on catches; 4-5 real extras per run).
- **gpt-6.1-sol: consistent** (spread 0; same 9 bugs every run; no extras).
- **gpt-5.6-sol: consistent** (spread 0; same 9 bugs every run; no extras).

Lazlo's drift note: prompt length is identical across runs (about 7.7k input tokens). Output length is 607-806 tokens for gpt-6.2-astra. Its longest output (806, run 5) had the most extras (5). One case with one prompt size cannot test drift on long tasks; that needs a longer case.

## Live-merge split (shadow reviewers in the live queue)

Source: `state/shadow-verdicts.tsv`, 59 finding verdicts over 30 queue entries, all dated 2026-11-19 (totals: real 33, known 23, false 3).

**Join.** Each verdict row's `entry` matched `state/queue/<entry>.entry`. The shadow model is that file's `shadow_model` field. `shadow_model_used` equals it in all 30 entries (no mismatch). A finding is attributed to the shadow model when its `finding_key` (file:line) appears in that entry's `state/queue/<entry>.shadow.review`. 58 of 59 rows match that way; the one exception is below.

| Shadow reviewer | Entries | Rows | REAL | KNOWN | FALSE | False-alarm (FALSE / rows) | Real rate (REAL / rows) | Shadow tokens per entry, mean (median) | Shadow seconds per entry, mean (median) |
|---|---|---|---|---|---|---|---|---|---|
| gpt-5.6-sol | 12 | 24 | 10 | 11 | 3 | 12.5% | 41.7% | 49,394 (45,902) | 85.9 (112.5) |
| gpt-6.1-sol | 18 | 35 | 23 | 12 | 0 | 0% | 65.7% | 21,452 (14,772) | 23.4 (21.8) |
| gpt-6.2-sol | 0 | - | - | - | - | - | - | - | - |
| gpt-6.2-astra | 0 | - | - | - | - | - | - | - | - |
| Sonnet 5.5 | 0 | - | - | - | - | - | - | - | - |

Totals: 12 + 18 = 30 entries, 24 + 35 = 59 rows. REAL 10 + 23 = 33; KNOWN 11 + 12 = 23; FALSE 3 + 0 = 3.

- Rows per entry for gpt-5.6-sol: q-20261119-170206, 171712, 171935, 173033, 173952, 174115, 174836, 175739, 175346, 175821, 180353, 182610.
- Rows per entry for gpt-6.1-sol: q-20261119-201906-2, 205504, 211149, 215916, 214719, 215254, 215840, 215947, 215947-2, 215948-2, 215949, 220003, 220412, 220839, 221115, 221115-2, 221838, 221903.
- **Exception:** `q-20261119-220839`, `spec_checklist.md:1`, KNOWN, judge orchestrator. Its `finding_key` is not in the entry's shadow review text. It is counted under gpt-6.1-sol (the entry's shadow model); it does not change any FALSE count.
- Gating reviewers: most entries were gated by Haiku. Two of the gpt-5.6-sol entries and six of the gpt-6.1-sol entries were gated by Opus. The split does not separate by gate.
- Cost: `shadow_cost_usd` is blank in every entry, so no USD per live review.

Caveats: 12 and 18 entries, one day, different change sets. Sonnet 5.5 and the two 6.2 models have no shadow entries, so their live columns are empty, not zero.

## Ranked reviewer table and recommendation

Ranking rule: hand-checked hard1 mean first; then false alarms (hard1 decoy FPs and unsupported extras, plus live FALSE rate); then consistency; then cost and seconds.

| Rank | Reviewer | hard1 mean /12 (spread) | hard1 misses per run | hard1 false alarms (decoy + UNSUP) | Extras REAL (5 runs) | Live REAL / FALSE / rows | Live false-alarm rate | Seconds per run | Cost per run | Tokens per run |
|---|---|---|---|---|---|---|---|---|---|---|
| 1 | gpt-6.1-sol | 9.0 (0) | 2 (b10, b12) | 0 | 0 | 23 / 0 / 35 | 0% | 30.1 | $0.38 (est.) | 8,922 |
| 2 | gpt-6.2-astra | 9.0 (0) | 1 (b12) | 0 | 21 | not run | - | 17.3 | $0.31 (est.) | 8,342 |
| 3 | gpt-6.2-sol | 9.0 (0) | 1 (b12) | 3 | 23 | not run | - | 53.2 | $0.53 (est.) | 10,172 |
| 4 | gpt-5.6-sol | 9.0 (0) | 2 (b10, b12) | 0 | 0 | 10 / 3 / 24 | 12.5% | 43.9 | $0.50 (est.) | 9,942 |
| 5 | claude-sonnet-5-5 | 8.8 (2) | 1 (b12, in 3 of 5 runs) | 1 (decoy d1, run 1) | 0 | not run | - | 16.0 | $0.031 (CLI, Claude plan) | 16,712 |

Why this order:
- The four GPT models tie at 9 hand-checked catches with spread 0. Sonnet is 8.8 with one decoy flag.
- gpt-6.1-sol is the only one with zero hard1 false alarms, zero extras and zero live false alarms over a real sample (35 live findings). Its misses (b10, b12) are the same in gpt-5.6-sol.
- gpt-6.2-astra and gpt-6.2-sol find more real problems but have no live data yet. gpt-6.2-sol is the slowest and dearest per run.
- gpt-5.6-sol has the highest live false-alarm rate (3 of 24) among the models with live data.

**Recommendation: keep the current gating reviewer (Haiku, per the queue entries) in the main slot.** No switch is recommended now, and none is made (Lazlo approves any switch). Among the shadows, **gpt-6.1-sol is the candidate** for a future switch. Before a switch:
1. Put gpt-6.2-astra and gpt-6.2-sol through the live shadow slot, so all candidates have the same live measure.
2. Give the gating reviewer an explicit check for a test that can never fail: b10 (`backoff_stays_within_cap`) was missed by 6.1-sol and 5.6-sol in all 10 runs.
3. Fix the scorer's undercount (the 6.2 runs lost 2-3 bugs each to the regex) or hand-check every switch decision.
4. Get a codex answering-model check: `model_used` is empty for every codex run here, so lesson 32 cannot be confirmed for the GPT models.

The current gating reviewer (Haiku) has no hard1 run in this bench, so this table cannot say whether any shadow beats it. The 2026-11-18 single-run table gave Haiku 8/12 and Opus 9/12 on hard1 (`evidence/benchmarks-2026-11-18.md`, different scoring method). That comparison needs its own five-run bench.

## Spend

- Sonnet 5.5: $0.1527 total (CLI-reported, 5 runs, Claude plan).
- GPT: $8.61 total across 20 calls, from `state/gpt-spend/2026-11-20.tsv` (estimates; caller `unknown`, 23:01Z-23:13Z). Per model: gpt-6.2-sol $2.65, gpt-6.2-astra $1.55, gpt-6.1-sol $1.90, gpt-5.6-sol $2.51.
- Ledger day total after the runs: $52.57 of $60.00 (the cap on main). Spend stays under the cap. The shadow and gate calls the live queue made during the same window are not included above.
