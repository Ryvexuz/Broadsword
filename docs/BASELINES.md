# Baselines: before/after evidence for process changes

Project rule: every process change needs before/after evidence (throughput, quality, cost). If the evidence does not show an improvement, switch back. `scripts/baseline.py` gathers the numbers. A later ledger check that reads these files is planned; it is not built yet, and nothing enforces the ledger rule today.

## Commands

```
python -I scripts/baseline.py record <name> --since <ISO time> [--until <ISO time>]
python -I scripts/baseline.py compare <before> <after> --min-samples <n> --noise <percent> [--metrics a,b]
python -I scripts/baseline.py list
```

- `record` measures one window and writes `state/baselines/<name>.json`. That folder is outside Git, under `state/`. The file also stores main's commit and the `broadsword.toml` SHA-256, so a baseline says what it measured.
- `record` never overwrites, even when two records race for one name (the file is created with an atomic link; the loser exits 1). A "before" is evidence, so pick a new name instead.
- A baseline file is checked in full when read: the schema, name, commit, toml hash, window, samples (`merged_tasks` must be a whole number), per-model rows, and every metric in the table below (a number or null) must be there. A missing key or wrong type is bad input (exit 1) and names the file. Two metrics added later, `coder_use_upper_bound` and `coder_tokens_unmerged`, may be absent from older baselines (those are never rewritten); `compare` shows `-` for them.
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
| pipeline | `coder_use_upper_bound` | context only | UPPER BOUND of coder use, from `state/agent-runs.tsv`. Runs have only a date and a duration, no start time, so a run overlapping a window edge cannot be clipped exactly. Each run's seconds are capped at its task's overlap with the window (the task's own started/ended), summed, / window. The true figure is at most this. Over 100% means coders ran in parallel. `dev_metrics.py` has no function for this, so baseline computes it itself |
| quality | `review_rounds_per_merged` | lower | every `.review` file of a merged task (the task's `task:` header), failed rounds included, whether or not its queue entry is known; a test-hook review is not counted |
| quality | `first_pass_fail_rate` | lower | merged tasks whose first review was FAIL; the first review is the one whose queue entry was created first (attempt number breaks ties); a review with no queue entry or no created time cannot be ordered and never decides it |
| quality | `reverted_or_reopened` | lower | `Revert...` commits on main in the window, plus tasks merged more than once |
| quality | `main_check_failed` | lower | entries with status `merged_check_failed` |
| quality | `direct_merges` | context only | tasks merged outside the queue |
| cost | `review_usd_per_merged`, `review_tokens_per_merged` | lower | every review in the window, retries and rejected attempts included, / merged |
| cost | `coder_tokens_per_merged` | lower | coder tokens of all tasks finished in the window (done, failed or abandoned; every run, retries included) / the merged tasks that have a coder run recorded; null if none. A merged task with no coder run is not counted as zero tokens: `samples.merged_with_coder_run` holds the divisor and `record` prints "N of M merged tasks have no coder run recorded". Below `--min-samples` of those covered tasks the metric is NOISE |
| cost | `coder_tokens_unmerged` | context only | how many of those tokens belong to tasks that did not merge |
| queue | `entry_median_s` | lower | entry created to ended (one attempt's merge time) |
| queue | `check_median_s`, `review_median_s`, `recheck_median_s` | lower | `dev_metrics.steps()`: `check`, `review`, `main_check` |
| queue | `test_job_median_s` | lower | `state/jobs/*.rec` of kind `check` or `test` |
| queue | `build_lock_wait_s` | lower | not recorded by Broadsword yet, so always `-` |

- **Per model:** runs, tokens, USD and seconds per `role:model`, for example `reviewer:opus` or `coder:sonnet`. These are shown as context, with no verdict.
- **Coder runs for use and per model:** a row counts when its task's running time overlaps the window; its seconds are capped at the task's overlap with the window (an upper bound, see above). A row whose task has no known start time counts by its date, capped at the window length, and `record` prints "N runs have no task span: counted in full, capped at the window", since these can inflate `coder_use_upper_bound`.
- **Coder tokens (cost):** every run of a task that ended inside the window counts, whatever the run's date, and no other run does, so work still running at the window end never inflates the cost. Abandoned or failed work that ended in the window stays in on purpose (cost to completion includes waste); `coder_tokens_unmerged` says how much of the total that is. This matches review cost, which counts finished attempts of the window. Rows of unfinished or unknown tasks are left out and counted in a note.
- **Reviews with no cost recorded:** a review whose `cost_usd` (or token fields) is missing is not counted as zero. `record` counts them (`samples.reviews_without_cost`, `reviews_without_tokens`) and prints "N of M reviews have no cost recorded"; the USD and token figures are then lower bounds. A `cost_usd` that is present but not a number is bad input naming the file.
- **Shadow reviews:** a `*.shadow.review` file is not a review of record. It is left out of every metric, of `samples.reviews` and of the per-model rows.
- **Definitions version:** every record has a `definitions` map (metric to version, for example `"coder_tokens_per_merged": "v1"`). The version is bumped whenever a metric's meaning changes. `compare` gives NOISE with the reason "definition differs (before v0, after v1)" for any metric whose version differs between the two sides, and keeps it out of the overall verdict. A record with no map, or no entry for a metric, used the old definition `v0` (all records before 2026-11-19 are v0). To compare across a change of definition, record the "before" window again with the current code, under a new name.
- **Shared helpers:** `fields`, `num`, `steps` and `covered` (idle share) come from `scripts/dev_metrics.py`, and the review header parser comes from `scripts/review_costs.py`. Both are imported by path, not copied. Coder use and coder tokens are not in dev_metrics as functions, so they are baseline's own (see above).

## Verdicts

- **NOISE:** the change is within `--noise` percent, or the metric is not recorded on both sides.
- **NOISE, context only:** a context-only metric (no good direction) is always NOISE with the reason "context only, no direction" and is left out of the overall verdict.
- **Bad numbers:** a present but malformed number in `state/agent-runs.tsv` (named with its line) or in a review's token fields, or a NaN/Infinity in a baseline file, is an input error (exit 1), never a silent zero. An empty cell stays "unknown" and is counted in a note (for example "N coder runs have no token count").
- **Reviews with no entry time:** a review of a merged task whose queue entry is missing or has no created time is counted in `review_rounds_per_merged` but left out of the first-pass rate and of review cost; `record` prints a note with the count.
- **Non-finite numbers:** `nan` and `inf` in a review's `cost_usd` are input errors naming the file, and `record` refuses to write any metric that is not finite.
- **From 0:** a change from 0 has no relative size. For a lower-is-better count (`reverted_or_reopened`, `main_check_failed`), 0 rising to more than 0 is WORSE with the reason "from 0", whatever `--noise` says, so harm can never hide behind a gain elsewhere; falling to 0 is a normal change. For any other metric (shares, times, rates such as `idle_share`) it is NOISE with the reason "before is 0, no relative change"; look at the numbers yourself.
- **NOISE, too few samples:** either side has fewer than `--min-samples` merged tasks. This is never BETTER, and the reason is printed.
- **BETTER or WORSE:** otherwise, by each metric's direction.
- **Overall:** WORSE if any compared metric is WORSE. This is the ledger's watch-for-harm rule. Otherwise it is BETTER if any metric is BETTER, and NOISE if none is.
- **`--metrics a,b`:** compares only the metrics the change should move, plus the harm you named. Without it, unrelated swings (for example a window with more Rust tasks) turn the overall verdict WORSE.

- **Notes:** `compare` prints each side's stored notes (for example "lower bound") under the table.

**Known limits.** (1) `idle_share` stays 0 while old tasks are still marked `running`, because it follows `dev_metrics`. (2) `state/` and `main` are fixed names here, like in `dev_metrics.py` and `review_costs.py` (a gap shared by all three). (3) The never-overwrite guard uses `os.link`, which a file system without hard links (exFAT) does not support: `record` then exits 1 with "cannot create".

**Compare like with like.** Use windows of similar length and task mix (project, risk tier, reviewer model). A window of crash-sim checks (6 s) against one of Broadsword checks (3 minutes) measures the mix, not the change.

## With the RETRO ledger

1. Before a process change merges, record its "before":
   `baseline.py record pre-<change> --since <8 h before> --until <now>`.
2. Add or update the ledger row. In Status, write `before: pre-<change> after: pending YYYY-MM-DD`. Optionally add `metrics: a,b`.
3. Once enough tasks have merged with the change, record `post-<change>` and replace `pending YYYY-MM-DD` with `post-<change>`.
4. Compare by hand: `baseline.py compare pre-<change> post-<change> --min-samples <n> --noise <percent>`. The change stays only if the overall verdict is BETTER. NOISE and WORSE both mean switch back (the row's Status says `reverted` or `fixed`), or extend the trial, record `post-<change>` again under a new name and compare again. (A later `ledger_check.py` is planned to do this check; it is not built yet.)

A row's Since cell starting with `planned` means the change is not in force yet.

## Starting settings (a suggestion, not a default)

- `--min-samples 5`: a 4-hour window on 2026-11-18 had 13 to 14 merges.
- `--noise 10`.

Check these against real before/after pairs at the next retro. Change them there with evidence, like any other process setting.
