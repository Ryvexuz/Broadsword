# Baselines: before/after evidence for process changes

Project rule: every process change needs before/after evidence (throughput, quality, cost). If the evidence does not show an improvement, switch back. `scripts/baseline.py` gathers the numbers. `scripts/ledger_check.py` makes the `RETRO.md` improvement ledger follow the rule.

## Commands

```
python -I scripts/baseline.py record <name> --since <ISO time> [--until <ISO time>]
python -I scripts/baseline.py compare <before> <after> --min-samples <n> --noise <percent> [--metrics a,b]
python -I scripts/baseline.py list
```

- `record` measures one window and writes `state/baselines/<name>.json`. That folder is outside Git, under `state/`. The file also stores main's commit and the `broadsword.toml` SHA-256, so a baseline says what it measured.
- `record` never overwrites. A "before" is evidence, so pick a new name instead.
- Times need a zone, for example `2026-11-18T09:00:00Z`. A bare time is refused, because it would be read in the wrong zone.
- `compare` prints each metric's before, after, change and verdict, then an overall verdict.
- `--min-samples` and `--noise` are required, because the code has no defaults (the settings rule).
- Exit codes: 0 done, 1 bad input, 2 usage. A bad input is a missing or unreadable source file, a bad time, or an unknown baseline. The message names the file.

## What a baseline measures

A task counts if it merged (ended) inside `[since, until)`. A queue entry, review or job counts if it was created or started inside the window.

| Group | Metric | Better when | Source |
| --- | --- | --- | --- |
| throughput | `merged_tasks` | context only | tasks done with a merged queue entry, or a note saying it merged directly |
| throughput | `merged_per_hour` | higher | merged / window hours |
| throughput | `lead_median_s`, `lead_p90_s` | lower | task `started` to `ended` (merge) |
| pipeline | `idle_share` | lower | `dev_metrics.covered()` over task running spans |
| pipeline | `coder_use` | higher | coder-role seconds in `state/agent-runs.tsv` / window. Over 100% means coders ran in parallel |
| quality | `review_rounds_per_merged` | lower | `.review` files of merged tasks, failed rounds included |
| quality | `first_pass_fail_rate` | lower | merged tasks whose first review was FAIL |
| quality | `reverted_or_reopened` | lower | `Revert...` commits on main in the window, plus tasks merged more than once |
| quality | `main_check_failed` | lower | entries with status `merged_check_failed` |
| quality | `direct_merges` | context only | tasks merged outside the queue |
| cost | `review_usd_per_merged`, `review_tokens_per_merged` | lower | every review in the window, retries and rejected attempts included, / merged |
| cost | `coder_tokens_per_merged` | lower | coder rows in `agent-runs.tsv` / merged |
| queue | `entry_median_s` | lower | entry created to ended (one attempt's merge time) |
| queue | `check_median_s`, `review_median_s`, `recheck_median_s` | lower | `dev_metrics.steps()`: `check`, `review`, `main_check` |
| queue | `test_job_median_s` | lower | `state/jobs/*.rec` of kind `check` or `test` |
| queue | `build_lock_wait_s` | lower | not recorded by Broadsword yet, so always `-` |

- **Per model:** runs, tokens, USD and seconds per `role:model`, for example `reviewer:opus` or `coder:sonnet`. These are shown as context, with no verdict.
- **Coder runs:** a row counts when its task's running time overlaps the window. A row with no known task counts by its date.
- **Shared helpers:** `fields`, `num`, `steps` and `covered` come from `scripts/dev_metrics.py`, and the review header parser comes from `scripts/review_costs.py`. Both are imported by path, so the numbers match those reports.

## Verdicts

- **NOISE:** the change is within `--noise` percent, or the metric is not recorded on both sides.
- **NOISE, too few samples:** either side has fewer than `--min-samples` merged tasks. This is never BETTER, and the reason is printed.
- **BETTER or WORSE:** otherwise, by each metric's direction.
- **Overall:** WORSE if any compared metric is WORSE. This is the ledger's watch-for-harm rule. Otherwise it is BETTER if any metric is BETTER, and NOISE if none is.
- **`--metrics a,b`:** compares only the metrics the change should move, plus the harm you named. Without it, unrelated swings (for example a window with more Rust tasks) turn the overall verdict WORSE.

**Compare like with like.** Use windows of similar length and task mix (project, risk tier, reviewer model). A window of crash-sim checks (6 s) against one of Broadsword checks (3 minutes) measures the mix, not the change.

## With the RETRO ledger

1. Before a process change merges, record its "before":
   `baseline.py record pre-<change> --since <8 h before> --until <now>`.
2. Add or update the ledger row. In Status, write `before: pre-<change> after: pending YYYY-MM-DD`. Optionally add `metrics: a,b`.
3. Once enough tasks have merged with the change, record `post-<change>` and replace `pending YYYY-MM-DD` with `post-<change>`.
4. `ledger_check.py` compares the two. If the result is WORSE, the row fails until its Status says `reverted` or `fixed`.

```
python -I scripts/ledger_check.py --since <YYYY-MM-DD> --max-pending-days <n> --min-samples <n> --noise <percent>
```

Rows whose Since cell starts with `planned` are skipped. A row's "added" date is the date of the oldest commit that has its `| Since | Change |` text. A row that is not committed yet counts as added today.

## Starting settings (a suggestion, not a default)

- `--min-samples 5`: a 4-hour window on 2026-11-18 had 13 to 14 merges.
- `--noise 10`.
- `--max-pending-days 7`.

Check these against real before/after pairs at the next retro. Change them there with evidence, like any other process setting.
