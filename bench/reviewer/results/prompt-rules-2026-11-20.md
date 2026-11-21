# Gate-prompt replay: 3 rules against speculative FALSE rejects (task t-20261120-225716), Haiku, 2026-11-20

Verdict: **DROP. reviewer.rs is not changed.** Arm B (the 3 rules) caught 1.0 REAL blockers per run against arm A's 7.5
(13% of A; the floor is 90%), and its FALSE blockers did not fall (1.5 per run against A's 0.5). Rule 1 of the three
moved real findings to MINOR in bulk; see "Why B lost recall".

## What was compared

- Arm A: the prompt the queue sends to a `--risk low` (Haiku) gate today, `SYSTEM_PROMPT_GATE` in reviewer.rs
  (`prompt_head!`, `gate_block!`, `prompt_tail!` joined by blank lines), 1530 characters,
  sha256 973c56e62316547c8e625d7a3ba4e477db595be760d165b4e197c9758c0ebfac.
- Arm B: arm A with the three lines of `bench/reviewer/replay-rules-2026-11-20.txt` added at the end of the gate block
  as rules 5 to 7, 2082 characters, sha256 8051d92c374ed3090e097a6035dea45f00227cd2af3a869c1f68d620c15c5eb5.
  The rules are the brief's three, worded as:
  - 5: a BLOCKING line that predicts a failure must name a command that was run and quote its output; if it cannot, and
    the brief or CHECK RESULT shows a passing run of that command or test, it is MINOR, not BLOCKING.
  - 6: a CHECK RESULT "NOT RUN for this change alone" (merge train) is normal and never a finding by itself.
  - 7: before flagging an edit outside the claims, look in the brief for a note that widens the claims.
- Both arm texts printed by `python -I -B bench/reviewer/replay.py --show`, which reads reviewer.rs through the canary's
  allow-listed parser (`scripts/shadow_canary.py` `prompt_macro`), not with a regex of its own.

Arm A is the gate prompt, not `SYSTEM_PROMPT`: the brief says to replay on Haiku, and Haiku only gates `--risk low`
entries, which get `SYSTEM_PROMPT_GATE` (`gate_system_prompt`). `SYSTEM_PROMPT` (shadow and `--risk high`) was not
replayed.

## Recall set and method

- Set: the 21 entries of evidence/haiku-vs-sonnet-replay-2026-11-19.md (replay plan "Set 1", the same set as the B1/B2
  replay in state/staging/reviewer-patterns/reviewer-prompt-block-2026-11.md). Ids:
  scratch/prompt-block-replay/ids.txt. Prompts: the 21 saved `scratch/prompt-block-replay/<id>.prompt.txt` files
  (rebuilt there as `reviewer::build_prompt` from each entry, +1 byte against the recorded prompt bytes). Labels: 20 REAL
  and 12 FALSE Haiku blockers; the 6 entries with no REAL point are 171935, 174836, 204355, 211149, 212315, 212610.
- Call, per entry, the queue's `ClaudeCli` call: `claude.exe -p --system-prompt <arm> --model haiku --tools ""
  --strict-mcp-config --max-turns 1 --output-format json`, prompt on stdin, cwd outside the repo, secret-named variables
  removed. Commands (from the main checkout root, `BENCH_CWD` = a temp folder outside the project,
  `BENCH_CLAUDE` = the Claude CLI):

      python -I -B worktrees/broadsword/t-20261120-225716/bench/reviewer/replay.py A 1 scratch/prompt-block-replay scratch/t225716-replay <ids>
      python -I -B worktrees/broadsword/t-20261120-225716/bench/reviewer/replay.py B 1 scratch/prompt-block-replay scratch/t225716-replay <ids>
      ... replay.py A 2 ... and ... replay.py B 2 ... (all 21 ids)

  Run 1 was split into two id chunks per arm; run 2 was one call per arm over all 21.
- Model check: every one of the 84 replies has `modelUsage` = `claude-haiku-5-5` and `is_error` false.
- Raw replies: bench/reviewer/results/prompt-rules-2026-11-20/<entry>.<arm>.r<run>.json (84 files).
- Scoring by hand, as the B1/B2 replay did: a REAL point counts as caught only if a BLOCKING line makes that point (same
  file, same root cause); a MINOR line or a PASS is a miss. A BLOCKING line that matches a labelled FALSE point is FALSE
  (that includes Sonnet's labelled-FALSE "3 merged vs RETRO 24" point on 171935 in findings.tsv). Any other BLOCKING
  line is UNSURE: not adjudicated and not credited to either column. A wrong reject = FAIL on one of the 6 entries with
  no REAL point.

## Per-entry table (REAL caught of the labelled Haiku REAL points; F = labelled FALSE blocker; U = UNSURE blocker)

| entry | REAL | A r1 | A r2 | B r1 | B r2 |
|---|---|---|---|---|---|
| q-20261119-170206 | 1 | 1 (+1U) | 1 | 0 PASS (ledger_check.py point is a MINOR) | 0 PASS |
| q-20261119-171935 | 0 | FAIL 1U (silent zero) | FAIL 1F (3 vs 24 merged) | FAIL 1F (3 vs 24 merged) | FAIL 1F (3 vs 13-14 merges, same point) |
| q-20261119-173033 | 1 | 1 | 1 (+1U b09 regex) | 0 PASS (hard1.diff is a MINOR) | 0 PASS |
| q-20261119-173300 | 1 | 0 (1U timeout) | 0 (1U timeout) | 0 (1U ring-buffer test) | 0 PASS |
| q-20261119-173952 | 1 | 0 PASS | 1 | 0 PASS | 0 PASS |
| q-20261119-174115 | 1 | 0 PASS | 0 PASS | 0 PASS | 0 PASS |
| q-20261119-174836 | 0 | FAIL 1U (b07 regex) | PASS | PASS | PASS |
| q-20261119-175346 | 2 | 1 (placeholder 2.8) | 1 (placeholder; +2U) | 0 PASS | 1 (placeholder 2.8) |
| q-20261119-175821 | 2 | 0 PASS | 0 (1U nan cost) | 0 PASS | 0 PASS |
| q-20261119-190437 | 1 | 1 | 1 | 0 PASS (tracking-ref point is a MINOR) | 0 PASS |
| q-20261119-192333 | 1 | 0 PASS | 0 PASS | 0 PASS | 0 PASS |
| q-20261119-193011 | 1 | 0 (1U call_failed) | 0 PASS | 0 PASS | 0 PASS |
| q-20261119-195613 | 1 | 1 | 1 | 0 PASS (call_failed point is a MINOR) | 0 PASS |
| q-20261119-201906-2 | 1 | 0 PASS | 0 PASS | 0 PASS | 0 PASS |
| q-20261119-204355 | 0 | PASS | PASS | PASS | PASS |
| q-20261119-204628 | 1 | 0 PASS | 1 (raw text; +1U header) | 0 PASS | 0 PASS |
| q-20261119-205504 | 2 | 0 (pipe, recover) | 0 (pipe, recover) | 0 (recover, pipe) | 0 (pipe) |
| q-20261119-205750 | 3 | 1 (recover says so) | 2 (one rule for callers, recover says so) | 0 PASS | 1 (recover says so) |
| q-20261119-211149 | 0 | PASS | PASS | PASS | PASS |
| q-20261119-212315 | 0 | PASS | PASS | PASS | FAIL 1F (19 unaccounted) |
| q-20261119-212610 | 0 | PASS | PASS | PASS | PASS |

On 205504 every run raised the pipe round-trip point, which findings.tsv labels REAL for that entry ("replay blocker:
pipe round-trip missing") but which is not one of Haiku's two labelled points there; as in the B1/B2 replay it is not
credited. It is the same in all four runs, so it does not change the comparison.

## Totals

| arm | run | REAL caught (of 20) | FALSE blockers | wrong rejects (of 6) | UNSURE blockers | FAIL / PASS | cost USD | call seconds |
|---|---|---|---|---|---|---|---|---|
| A (current gate prompt) | 1 | 6 | 0 | 2 (171935, 174836, both on UNSURE points) | 7 | 11 / 10 | 0.7846 | 991 |
| A (current gate prompt) | 2 | 9 | 1 | 1 (171935) | 8 | 12 / 9 | 0.2516 | 1037 |
| B (A + rules 5-7) | 1 | 0 | 1 | 1 (171935) | 3 | 3 / 18 | 0.7506 | 817 |
| B (A + rules 5-7) | 2 | 2 | 2 | 2 (171935, 212315) | 1 | 5 / 16 | 0.2280 | 988 |

| arm | mean REAL caught | mean FALSE blockers | mean wrong rejects |
|---|---|---|---|
| A | **7.5** (6 to 9) | **0.5** | 1.5 |
| B | **1.0** (0 to 2) | **1.5** | 1.5 |

Total spend: $2.0148 for 84 Haiku calls. Run 2 of each arm was cheaper through prompt caching of the identical prompts.

UNSURE blocking lines by run (A r1: 170206, 171935, 173300, 174836, 193011, 205504 x2 = 7; A r2: 173033, 173300,
175346 x2, 175821, 204628, 205504 x2 = 8; B r1: 173300, 205504 x2 = 3; B r2: 205504 = 1). Each run's REAL + FALSE + UNSURE
equals its count of non-`none` BLOCKING lines (A 13 and 18, B 4 and 5). They are listed for completeness; none is credited.

## Keep rule (from the brief) and verdict

Keep arm B only if its FALSE count falls and its recall is at least 90% of arm A's.
- Recall: B 1.0 against A 7.5, 13%; 90% of A is 6.75. **Fails**, and by more than the run-to-run range of either arm
  (A 6 to 9, B 0 to 2).
- FALSE: B 1.5 per run against A 0.5; wrong rejects equal (1.5 and 1.5). **Did not fall.**

**DROP.** reviewer.rs is unchanged; this commit holds only the replay runner, its tests, the rules text and this
evidence.

## Why B lost recall

Rule 5 asks the reviewer to "name a command that was run". The gate call has no tools (`--tools ""`), so the reviewer
can never run anything, and every CHECK RESULT in this set shows a passing test run. Haiku read that as "every predicted
failure is MINOR": in B run 1 the labelled REAL points of 170206 (ledger_check.py), 173033 (hard1.diff missing), 190437
(nested-repo tracking ref) and 195613 (call_failed with text) appear almost word for word as MINOR lines with
`VERDICT: PASS`. This is the same failure mode as B1 (5 of 20, lesson 40): a rule aimed at false rejects demotes real
findings.

The recall set has few of tonight's FALSE pattern (a predicted clippy or test failure contradicted by a pasted passing
run): its FALSE points are mostly settled brief points and "done-check output missing". So this replay can show the harm
of the rules but says little about their benefit on the Opus speculation rows of 2026-11-20 late.

## Limits

- Two runs per arm; per-entry results vary between runs of the same arm (A caught 173952 in one run only, 204628 in one
  run only).
- Labels cover only points from the recorded Haiku run, so new points are UNSURE and not credited in any arm.
- Haiku only, gate prompt only. Opus and `SYSTEM_PROMPT` (`--risk high` and shadow) were not replayed; the Opus FALSE
  rows that motivated the task are not in this set.
