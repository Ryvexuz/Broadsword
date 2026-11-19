# Before-baselines recorded 2026-11-19 (nb-08, t-20261119-171333-6)

Records live in `state/baselines/` (outside Git) in the main checkout. Commands were run from `C:\Users\MrComputer\Desktop\Broadsword`.

## Window and main commit

- No tp task had merged when these were recorded. The known tp ids are still open or abandoned: `t-20261118-095350` (tp1) is `abandoned` (ended 2026-11-19 19:02:33 UTC), `t-20261118-095350-3` (tp3) is `open`, `t-20261118-142614` (shadows) is `open`. tp2, tp4 and tp6 ids were not found in the files read, so their merge state was not checked.
- So the window is the 8 hours before the record time, fixed at `2026-11-19T13:45:00Z` to `2026-11-19T21:45:00Z`. All four records use this same window. The end is fixed so the record stays the same if later merges land.
- The main commit stored in the records is `c00393fc6aa857498ca9a67be62d87c5f5dba5cb` (`c00393f`, "RETRO: lesson 36"). Main was `853b25d` when the task started and moved during the run. The records store the commit they were taken from.
- The `broadsword.toml` SHA-256 stored in the records starts `3973524e34b7`.
- The existing windows `pc-agents-6` (17:00:00Z to 19:07:46Z) and `pc-agents-10` (19:20:00Z to 20:09:47Z) were not re-recorded. They were not used as the "before" for these changes, because the 8-hour rule in the brief sets the window.

## Commands run

Record commands (main checkout, exit codes):

```
python -I scripts/baseline.py record pre-tp-queue --since 2026-11-19T13:45:00Z --until 2026-11-19T21:45:00Z   -> exit 0
python -I scripts/baseline.py record pre-tp-buildlock --since 2026-11-19T13:45:00Z --until 2026-11-19T21:45:00Z -> exit 0
python -I scripts/baseline.py record pre-two-shadows --since 2026-11-19T13:45:00Z --until 2026-11-19T21:45:00Z -> exit 0
python -I scripts/baseline.py record pre-codex-coder --since 2026-11-19T13:45:00Z --until 2026-11-19T21:45:00Z -> exit 0
```

Output of `pre-tp-queue` (the other three print the same metric values; their headers differ only in name):

```
baseline pre-tp-queue: 2026-11-19T13:45:00Z to 2026-11-19T21:45:00Z (8.0 h)
main c00393fc6a, broadsword.toml sha256 3973524e34b7
  throughput merged_tasks                         20  tasks merged in the window (queue or approved direct merge)
  throughput merged_per_hour                    2.50  merged tasks per hour of the window
  throughput lead_median_s                     1464s  median task start -> merged
  throughput lead_p90_s                        4255s  p90 task start -> merged
  pipeline   idle_share                         2.4%  share of the window with no task running (dev_metrics 'idle')
  pipeline   coder_use_upper_bound             12.7%  UPPER BOUND of coder use ... context only
  quality    review_rounds_per_merged           1.85  reviews per merged task, failed ones included
  quality    first_pass_fail_rate              45.0%  merged tasks whose first review was FAIL
  quality    reverted_or_reopened                  0  Revert commits on main + tasks merged more than once
  quality    main_check_failed                     0  entries that merged but failed the check on main
  quality    direct_merges                         0  tasks merged outside the queue (approved direct merges)
  cost       review_usd_per_merged            0.4054  all review cost in the window (retries included) / merged
  cost       review_tokens_per_merged      123326.70  all review tokens in the window / merged
  cost       coder_tokens_per_merged       422877.00  ... / merged tasks that have a coder run recorded
  cost       coder_tokens_unmerged            317950  of those coder tokens, how many belong to tasks that did not merge
  cost       coder_runs_recorded                   5  coder runs of tasks that ended in the window
  queue      entry_median_s                     592s  median queue entry created -> ended (merge time per attempt)
  queue      check_median_s                      99s  median `check` step per entry
  queue      review_median_s                     50s  median `review` step per entry
  queue      recheck_median_s                    97s  median `main_check` step (re-check on main) per entry
  queue      test_job_median_s                   98s  median check/test job run time (state/jobs, kind check or test)
  queue      build_lock_wait_s                     -  build-lock wait inside jobs: not recorded by Broadsword yet
  per model: coder:sonnet 5 run(s), 845754 tokens, $0.0000; reviewer:haiku 34 run(s), 1651956 tokens, $0.9634; reviewer:opus 12 run(s), 814578 tokens, $7.1453
note: build-lock wait inside a job is not recorded by Broadsword yet (dev_metrics says the same)
note: 18 of 20 merged tasks have no coder run recorded: coder_tokens_per_merged divides by the 2 that have one
written: C:\Users\MrComputer\Desktop\Broadsword\state\baselines\pre-tp-queue.json
```

The record JSON files also show `samples`: `merged_tasks` 20, `merged_with_coder_run` 2, `queue_entries` 51, `reviews` 46, `test_jobs` 107, and no reviews without cost or tokens. The `per_model` seconds are `coder:sonnet` 3650, `reviewer:haiku` 2549, `reviewer:opus` 1310.

Check that the four files exist (done check 1):

```
ls state/baselines   (run from the main checkout)
```

Output (the four new files are the `pre-*` lines; the rest are earlier records):

```
pc-agents-10.json
pc-agents-6.json
pc-session-2026-11-19.json
pre-codex-coder.json
pre-tp-buildlock.json
pre-tp-queue.json
pre-two-shadows.json
review-fable-check-2.json
review-fable-check-3.json
review-fable-check.json
s4-second-half-r2.json
s4-second-half-r3.json
s4-second-half-r4.json
s4-second-half-r5.json
s4-second-half-r6.json
s4-second-half-r7.json
s4-second-half.json
s4-session.json
```

`python -I scripts/baseline.py list` after recording shows:

```
pre-codex-coder	2026-11-19T13:45:00Z	2026-11-19T21:45:00Z	20 merged
pre-tp-buildlock	2026-11-19T13:45:00Z	2026-11-19T21:45:00Z	20 merged
pre-tp-queue	2026-11-19T13:45:00Z	2026-11-19T21:45:00Z	20 merged
pre-two-shadows	2026-11-19T13:45:00Z	2026-11-19T21:45:00Z	20 merged
```

## What each record can and cannot show

`baseline.py` has no shadow metric and no build-lock wait metric (`build_lock_wait_s` is `null` in every record). The brief's shadow and build-lock numbers therefore come from raw files, below, and are not in the JSON. The `pre-two-shadows` and `pre-tp-buildlock` records hold the same tool metrics as `pre-tp-queue`.

### pre-two-shadows: shadow numbers from `state/queue/*.entry`

Script: `scratch/nb08-shadow.py` (read-only, reads entries with `created` in the window). Output:

```
entries created in window: 51
entries with a shadow verdict: 30
entries with no shadow verdict: 21
verdict counts: [('ERROR', 6), ('FAIL', 17), ('PASS', 7)]
ERROR rate among shadow runs: 6 of 30 = 20.0%
shadow_secs: n=30 median=40.6 min=3.4 max=146.1
entry created->ended (all entries in window): n=50 median=592s
```

The six ERROR entries were `q-20261119-173300`, `q-20261119-175548`, `q-20261119-181652`, `q-20261119-183236`, `q-20261119-184332`, `q-20261119-184830`. Their reasons include `HTTP 429: rate limit or quota reached` and `codex: codex timed out after 144 s`. Shadow was switched off at about 19:00:59Z (commit `cfadd9e`, per PC progress), so the 21 entries with no verdict are mostly after that point.

### pre-tp-buildlock: build-lock wait

Broadsword records no build-lock wait time in job records (`state/jobs/*.rec` have no lock field) or job logs. The only build-lock event in the window found was a refusal in `scratch/logs/jobs/20261119-171444-broadsword-run.log`:

```
broadsword: build folder busy: another build, test or check job held the build lock for the whole 900 s wait (build_lock_wait_seconds); nothing was built, cleaned or recorded.
```

So a median or p90 wait for this window cannot be computed from the records. PC progress has an earlier figure, quoted here only as a pointer and not as a baseline: "pc-agents-6 ... lock wait median 1 s p90 26 s" (`state/pc-progress.md`, 19:11:27Z line).

### pre-codex-coder: coder numbers

From the record: `coder:sonnet` 5 runs, 3650 seconds, 845754 tokens (per_model). `review_rounds_per_merged` 1.85, `first_pass_fail_rate` 0.45, `coder_tokens_per_merged` 422877.0 (2 merged tasks with a coder run), `coder_tokens_unmerged` 317950, `coder_use_upper_bound` 12.7%. Note from the record: 18 of 20 merged tasks have no coder run recorded, so the per-task coder figure rests on 2 tasks.

## Mapping: which backlog task each "before" is for

Per the brief. The mapping is for the orchestrator to copy into the RETRO ledger. No task file was edited.

- `pre-tp-queue`: tp1 (`t-20261118-095350`, abandoned), tp2 (id not resolved), tp4 (id not resolved).
- `pre-tp-buildlock`: tp3 (`t-20261118-095350-3`, open), tp6 (id not resolved).
- `pre-two-shadows`: the two-shadow change (`t-20261118-142614`, open).
- `pre-codex-coder`: the codex coder trial (the brief did not name the task id).

## Not done or not verified

- tp2, tp4 and tp6 ids were not found, so the merge state of those three was not checked.
- The task's required window end is a fixed time, not a merge time. If one of those tasks merged before 21:45:00Z, its "before" would be wrong. The check above covered only the known ids.
- No code changed. `scripts/baseline.py` was not edited.
