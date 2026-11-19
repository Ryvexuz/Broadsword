# Haiku vs Sonnet 5.5 replay, haiku-rejected queue entries of 2026-11-19

Measured 2026-11-19 (PC session). No recommendation on the gate rule is made here; these are the numbers.

## Method

1. Entries: `state/queue/q-20261119-*.entry` with `status = rejected` and `reviewer_model = haiku`, as of the start of the replay: 22 entries. (A later one, q-20261119-214719, appeared while the replay ran and is not included.)
2. Prompt rebuilt exactly as `reviewer::build_prompt` does (crates/broadsword/src/reviewer.rs): `{brief}`, a blank line, `CHECK RESULT (the branch was rebased onto main, then built and tested; this is what running it printed):`, `{check_summary}`, `End of the check output:` with `{check_tail}` in a text fence, `DIFF:` with the diff in a diff fence, then `Review it now, in the required format.` The brief, check_summary and check_tail come from the entry (unescaped as record.rs does). The diff is `git diff --no-color --no-ext-diff <base_head>...<checked_tip>` (wtgit::diff_since) in the repo of the entry's `project` (repo root, projects/crash-sim or projects/render). The queue's `.env` value redaction was not applied (it cannot be without reading `.env`; it only changes text that contains a secret value, and the byte counts match).
3. Byte check: every rebuilt prompt is +1 byte against the recorded `prompt bytes` (all 22, +0.0%). The 1 byte is a constant offset (most likely a newline difference in how the entry text round-trips); no entry differs by more than that.
4. Call (same as `ClaudeCli::call` in reviewer.rs), prompt on stdin, from `C:\Users\MRCOMP~1\AppData\Local\Temp\replay-cwd` (outside the repo), with variables whose names contain KEY, TOKEN, SECRET or PASSWORD removed from the environment:

   `C:\Users\MrComputer\.local\bin\claude.exe -p --system-prompt <reviewer::SYSTEM_PROMPT verbatim> --model sonnet --tools "" --strict-mcp-config --max-turns 1 --output-format json`

   Every reply reports `modelUsage = claude-sonnet-5-5` (model checked). Raw replies: `scratch/replay/<entry>.sonnet.json` (scratch is wipeable; the per-entry section below quotes the blocking points). One call per entry; no retries.
5. Labels. REAL = a later fix round changed exactly that point, or the orchestrator, the shadow adjudication (state/shadow-verdicts.tsv) or a stronger reviewer confirmed it. FALSE = the orchestrator or a stronger reviewer showed it wrong (state/pc-progress.md, the "Fix round" sections of later entry briefs, the Sonnet decisive review of the nb-19 allow-list entries). UNSURE = no verdict recorded. A Sonnet point that matches a Haiku point inherits that point's label.
6. Haiku cost and seconds are the recorded `total_cost_usd` and `duration_ms` of the queue's own call. Sonnet seconds are the CLI's `duration_ms`.

## Per-entry table

Columns: haiku blockers (REAL/FALSE), Sonnet verdict, Sonnet blockers (REAL/FALSE), Sonnet cost and seconds, prompt bytes (rebuilt/recorded, difference).

| entry | task | project | haiku blockers | sonnet verdict | sonnet blockers | sonnet $ / s | prompt bytes |
|---|---|---|---|---|---|---|---|
| `q-20261119-170206` | t-20261119-170109 | broadsword | 1R/0F | PASS | 0R/0F | $0.139 / 10s | 64627/64626 (+1 B) |
| `q-20261119-171935` | t-20261119-170109 | broadsword | 0R/2F | FAIL | 0R/1F | $0.162 / 15s | 75033/75032 (+1 B) |
| `q-20261119-173033` | t-20261118-075707 | broadsword | 1R/0F | FAIL | 1R/0F | $0.238 / 20s | 106591/106590 (+1 B) |
| `q-20261119-173300` | t-20261118-080157 | broadsword | 1R/0F | PASS | 0R/0F | $0.448 / 12s | 232678/232677 (+1 B) |
| `q-20261119-173952` | t-20261119-170109 | broadsword | 1R/1F | PASS | 0R/0F | $0.173 / 11s | 84967/84966 (+1 B) |
| `q-20261119-174115` | t-20261119-171333-3 | render | 1R/0F | PASS | 0R/0F | $0.072 / 9s | 26146/26145 (+1 B) |
| `q-20261119-174836` | t-20261118-075707 | broadsword | 0R/1F | PASS | 0R/0F | $0.248 / 23s | 109799/109798 (+1 B) |
| `q-20261119-175346` | t-20261119-171333-28 | crash-sim | 2R/0F | PASS | 0R/0F | $0.147 / 38s | 46621/46620 (+1 B) |
| `q-20261119-175548` | t-20261118-080157 | broadsword | 0R/0F/3U | NOT REPLAYED | - | - | 242067/242066 (+1 B, prompt built, call not run) |
| `q-20261119-175821` | t-20261119-170109 | broadsword | 2R/0F | PASS | 0R/0F | $0.190 / 14s | 93264/93263 (+1 B) |
| `q-20261119-190437` | t-20261119-171333-12 | broadsword | 1R/0F | FAIL | 1R/0F | $0.105 / 13s | 42364/42363 (+1 B) |
| `q-20261119-192333` | t-20261119-171333-46 | broadsword | 1R/0F | PASS | 0R/0F | $0.114 / 9s | 51102/51101 (+1 B) |
| `q-20261119-193011` | t-20261119-171333-23 | broadsword | 1R/0F | PASS | 0R/0F | $0.492 / 12s | 231166/231165 (+1 B) |
| `q-20261119-195613` | t-20261119-171333-23 | broadsword | 1R/0F | PASS | 0R/0F | $0.496 / 12s | 233360/233359 (+1 B) |
| `q-20261119-201906-2` | t-20261119-171333-23 | broadsword | 1R/0F | PASS | 0R/0F | $0.501 / 11s | 236587/236586 (+1 B) |
| `q-20261119-204355` | t-20261119-204250 | broadsword | 0R/1F | PASS | 0R/0F | $0.063 / 6s | 22520/22519 (+1 B) |
| `q-20261119-204628` | t-20261119-204250 | broadsword | 1R/1F | PASS | 0R/0F | $0.066 / 6s | 23607/23606 (+1 B) |
| `q-20261119-205504` | t-20261119-171333-17 | broadsword | 2R/0F | FAIL | 2R/0F | $0.177 / 7s | 84231/84230 (+1 B) |
| `q-20261119-205750` | t-20261119-171333-17 | broadsword | 3R/0F | FAIL | 1R/0F | $0.179 / 8s | 85126/85125 (+1 B) |
| `q-20261119-211149` | t-20261119-171333-17 | broadsword | 0R/2F | PASS | 0R/0F | $0.247 / 11s | 127130/127129 (+1 B) |
| `q-20261119-212315` | t-20261119-212220 | broadsword | 0R/2F | PASS | 0R/0F | $0.066 / 7s | 23667/23666 (+1 B) |
| `q-20261119-212610` | t-20261119-212220 | broadsword | 0R/2F | PASS | 0R/0F | $0.071 / 8s | 25919/25918 (+1 B) |

### Why each label (short)

- 170206 (nb-06 r1): Haiku's ledger_check.py point REAL (fix round 1 item 1 changed it). Sonnet listed it only as MINOR and PASSed. Real points neither listed as blocking: overwrite race, silent zero cost (shadow REAL).
- 171935 (nb-06 r2): both Haiku points FALSE (orchestrator checked them against 302cfd0). Sonnet's one blocker (3 merged vs RETRO's 24) FALSE: different window, explained in the next brief. Real points from the shadow (coder_use unclipped, validation gaps) were in neither list.
- 173033 (hard1 r1): gitignored cases/hard1.diff REAL for both models.
- 173300 (coding bench r1): Haiku's 96 MB LCS table REAL; Sonnet PASS (listed the run.py timeout as MINOR, missed the table).
- 173952 (nb-06 r3): Haiku's silent zero on a malformed cell REAL; "no unittest evidence" FALSE (the queue's check runs Rust only; the evidence was pasted in the brief). Sonnet PASS, missed the silent zero.
- 174115 (nb-04): Haiku f32 underflow REAL (fixed 9e1060b); Sonnet had it as a hedged MINOR ("assuming length() squares") and PASSed.
- 174836 (hard1 r2): Haiku's single combined blocker (bug10 has no proof test; false FAIL in the table) FALSE (orchestrator: accepted deviation; the property fails on buggy). Sonnet PASS. A real shadow point (traversal test writes escape.job) was not blocking for either.
- 175346 (nb-30): both Haiku points REAL (placeholders in 2.8/3.6; no check requires a break; fixed with an exact 2.8 and a new 2.9). Sonnet PASS, placeholders only as MINOR, missed the no-break point.
- 175548 (coding bench r2): NOT REPLAYED (budget). Haiku's 3 blockers UNSURE: no per-point verdict was recorded; the next review was Opus, which failed it on spec leaks and the 50k-line test and cited none of the three.
- 175821 (nb-06 r4): both Haiku points REAL (orphan-review first pass; runs of tasks without a start; both in fix round 4). Sonnet PASS.
- 190437 (nb-14): nested-repo tracking ref stale REAL for both (fixed 18b8630).
- 192333 (nb-45): Windows `file:..\` escape REAL (fixed a357fca). Sonnet had it as a MINOR ("low risk") and PASSed.
- 193011 (nb-25 r1): keep_line ignores order REAL (fixed e5dd909). Sonnet PASS, missed it.
- 195613 (nb-25 r3): call_failed with text REAL (fixed 41daaa1). Sonnet PASS, missed it.
- 201906-2 (nb-25 r4): refusals as normal replies REAL (fixed c08a4dd). Sonnet PASS, missed it. Shadow-REAL scorer gaps (cases 06, 09) were in neither list.
- 204355 (allow-list r1): Haiku's "done-check output absent" FALSE (the queue check runs Rust only; the same point was refuted in nb-06 r3). Sonnet PASS.
- 204628 (allow-list r2): "UNACCOUNTED still 19" FALSE; "evidence file holds a summary, not the reviewer's raw words" REAL (the next brief says "haiku was right"; round 2 quoted the reply verbatim). Sonnet PASS; it noted the same raw-text point only as a MINOR.
- 205504 (nb-19 r1): Haiku's two points share one root cause: a secret staged under a custom short name whose file is then deleted stays in the index ref. Orchestrator: real, but outside the claim (follow-up item 6, a known limit). Labelled REAL (deferred), counted as 2. Haiku missed the in-scope blocker the brief required (pipe round-trip; shadow-verdicts REAL); Sonnet raised that one plus "every .env test through one function", both REAL (the second was confirmed in the next round).
- 205750 (nb-19 r2): all 3 Haiku points REAL (orchestrator: "correct call": pipe round-trip, recovery says so, one rule for callers). Sonnet raised only the pipe point (REAL) and missed the other two as blockers.
- 211149 (nb-19 r3): both Haiku points FALSE (documented known limit in the brief; documented Lazlo override; Haiku itself said the second was not blocking). Sonnet PASS. Neither listed the real point from the shadow (job-created unstaged secrets not named in recover).
- 212315 and 212610 (allow-list, nb-19): all 4 Haiku points FALSE (Sonnet decisive review: all Haiku points FALSE on the facts; 19 unaccounted = older cases; db76686 = db7668699c76; LISTED is accounted). Sonnet PASS on both, as the decisive review did.

## Totals (21 replayed entries)

| | Haiku (queue, recorded) | Sonnet 5.5 (replay) |
|---|---|---|
| entries failed | 21 of 21 (all were rejections) | 5 of 21 |
| entries passed | 0 | 16 |
| blocking points, total | 32 | 6 |
| REAL blockers | 20 | 5 |
| FALSE blockers | 12 | 1 |
| entries with at least one REAL blocker | 15 | 4 |
| entries failed only on FALSE blockers | 6 (171935, 174836, 204355, 211149, 212315, 212610) | 1 (171935) |
| cost, USD | 0.7568 | 4.3932 |
| seconds (sum of calls) | 785.1 | 262.3 |

REAL counts include 2 Haiku points on 205504 that are real but a deferred known limit. Haiku's three UNSURE points on the not-replayed entry 175548 are excluded from every count. The two models' REAL points overlap on three entries (173033, 190437, and the pipe point of 205750); Sonnet's two REAL points on 205504 are a different root cause from Haiku's two there.

Split by kind:

| subset | entries | Haiku REAL / FALSE | Haiku $ | Sonnet verdicts | Sonnet REAL / FALSE | Sonnet $ | Sonnet s |
|---|---|---|---|---|---|---|---|
| code entries | 17 | 19 / 6 | 0.7432 | 12 PASS, 5 FAIL | 5 / 1 | 4.1273 | 235.3 |
| records-only (allow-list) | 4 | 1 / 6 | 0.0136 | 4 PASS, 0 FAIL | 0 / 0 | 0.2660 | 27.0 |

## Records-only entries (allow-list tasks t-20261119-204250 and t-20261119-212220)

| entry | task | haiku | sonnet |
|---|---|---|---|
| q-20261119-204355 | t-20261119-204250 | FAIL, 0 REAL / 1 FALSE | PASS, 0 / 0 |
| q-20261119-204628 | t-20261119-204250 | FAIL, 1 REAL / 1 FALSE | PASS, 0 / 0 |
| q-20261119-212315 | t-20261119-212220 | FAIL, 0 REAL / 2 FALSE | PASS, 0 / 0 |
| q-20261119-212610 | t-20261119-212220 | FAIL, 0 REAL / 2 FALSE | PASS, 0 / 0 |
| total (4) | | 4 FAIL, 1 REAL / 6 FALSE, $0.0136, 17.7 s | 4 PASS, 0 REAL / 0 FALSE, $0.2660, 27.0 s |

Outcomes: t-20261119-204250 merged on a Haiku PASS (q-20261119-205048) after the evidence file was fixed; the nb-19 allow-list (t-20261119-212220) was committed as records (e2955e9) after the Sonnet decisive review PASS. The replay's Sonnet PASS agrees with both. The one REAL Haiku point (raw reviewer text missing from the evidence file) shows up in the Sonnet replay as a MINOR on both t-20261119-204250 entries.

## Did not replay, and caveats

- q-20261119-175548 (242,067 byte prompt): the call was not run. The total was $4.393 after 21 calls and a call of that size costs about $0.45 to $0.50; the stop rule was "stop before $5", so it was skipped. Its prompt was rebuilt (+1 byte).
- One sample per entry; Sonnet's replies were not repeated, so run-to-run variation is unmeasured.
- Sonnet answered in the required format on every entry. The queue's same-model rule (a model never reviews its own code) would have blocked Sonnet as reviewer on the entries Sonnet coded; the replay ignores that rule.
- Label basis is the orchestrator's records. Four labels rest on a single sentence there (171935, 174836, 211149 FALSE; 205504 REAL-deferred). The "real points neither model listed" in the per-entry notes come from state/shadow-verdicts.tsv and are not counted in either model's REAL totals.
- Haiku seconds are the API call time recorded in the queue; Sonnet's are the CLI duration measured here: comparable but not from one clock.
