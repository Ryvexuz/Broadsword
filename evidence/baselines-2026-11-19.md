# Before-baselines recorded 2026-11-19 (nb-08, t-20261119-171333-6, fix round)

Records live in `state/baselines/` (outside Git) in the main checkout. The four JSON records were written once and are not changed by this round (the record tool never overwrites a record). Where a number below differs from the JSON, the live recomputation is labelled "live" and the JSON value is labelled "recorded". Commands were run from `C:\Users\MrComputer\Desktop\Broadsword` unless stated.

## Window and main commit

- Window: `2026-11-19T13:45:00Z` to `2026-11-19T21:45:00Z` (8 hours), the same for all four records.
- Main commit stored in the records: `c00393fc6aa857498ca9a67be62d87c5f5dba5cb` (`c00393f`). Main HEAD when this round ran: `43e3916`, whose commit time is 2026-11-19T21:42:43Z, inside the window.
- `broadsword.toml` SHA-256 stored in the records starts `3973524e34b7`.

## tp task ids, status and merge check

Board status from `build\bin\broadsword.exe task show <id>` (output redirected to `scratch/`):

| Target | Task id | Board status | Check |
|---|---|---|---|
| tp1 (queue checks scoped) | `t-20261118-095350` | abandoned, ended 2026-11-19 19:02:33 UTC | `task show` |
| tp2 (main re-check after fast-forward) | `t-20261118-095350-2` | open | `task show` |
| tp3 (build lock only while compiling) | `t-20261118-095350-3` | running, started 2026-11-19 21:48:59 UTC, base `43e3916` | `task show` |
| tp4 (queue run --all / overlap) | `t-20261118-095351` | open | `task show` |
| tp5 (parallel test binaries) | `t-20261118-095351-2` | open | `task show` |
| tp6 (build lock or shared check) | no board task found | see below | `task list` |
| shadows (two shadow reviewers) | `t-20261118-142614` | open | `task show` |

tp6: `build\bin\broadsword.exe task list > scratch\nb08-fix-task-list.txt`, then a search of that file for `lock`, `Lock`, `shared` and `check`. The only titles containing "lock" are `t-20261118-095350-3` (tp3, "Hold the build lock only while compiling") and `t-20261119-165659` (abandoned, "Baseline metrics ... build-lock wait", a baseline task, not a tp task). No task title mentions a shared check. Result: tp6: no board task found. Task `t-20261119-215700` ("build_jobs setting ... (after tp3)") mentions tp3 in its title but not the build lock; it was not assigned to tp6.

Merge check. Commands and results:

- `git log --oneline --since=2026-11-19T13:45:00Z --until=2026-11-19T21:45:00Z` lists 59 commits (`scratch/nb08-fix-main-window-oneline.txt`). A search of that list for the ids and tp keywords returns only two commits, `bf13755` (ADR 0001, matched on "overlap") and `81ea717` (baseline coder-use wording, matched on "overlap"). Neither is a tp merge.
- `git log --all --oneline --grep=t-20261118-095350 --grep=t-20261118-095351 --grep=t-20261118-142614` returns one commit, `a4f00c4` (PROGRESS, dated 2026-11-18, outside the window). No commit carries the tp1-tp5 ids.
- `git merge-base --is-ancestor task/t-20261118-095350 main` and the same for `task/t-20261118-095350-3`: not ancestors of main (exit 1). Branches for tp2, tp4, tp5 and the shadows (`task/t-20261118-095350-2`, `task/t-20261118-095351`, `task/t-20261118-095351-2`, `task/t-20261118-142614`) do not exist (`git branch --list ... --all` prints nothing).
- Queue: no queue entry for tp2, tp4, tp5, tp6 or the shadow task. tp1 (`t-20261118-095350`) has two entries, `q-20261118-100920` and `q-20261118-101835`, both `rejected`, both on 2026-11-18.
- The 20 tasks merged in the window (see the rounds table below) include none of the tp ids.

So no tp task merged into main inside the window. The window end is still fixed; a merge after 21:45:00Z would not change these records.

## Mapping for the RETRO ledger

| Backlog change | Task id | Status | "Before" baseline name |
|---|---|---|---|
| tp1 queue checks scoped | `t-20261118-095350` | abandoned | `pre-tp-queue` |
| tp2 main re-check after fast-forward | `t-20261118-095350-2` | open | `pre-tp-queue` |
| tp3 build lock only while compiling | `t-20261118-095350-3` | running | `pre-tp-buildlock` |
| tp4 queue run --all, overlapping check | `t-20261118-095351` | open | `pre-tp-queue` |
| tp5 parallel test binaries | `t-20261118-095351-2` | open | none in the brief; not recorded |
| tp6 build lock or shared check | no board task found | - | `pre-tp-buildlock` (as the brief says) |
| two shadow reviewers | `t-20261118-142614` | open | `pre-two-shadows` |
| astra sweep | no separate board task found | - | not recorded; the brief names no baseline for it |
| codex coder trial | `t-20261119-171333-19` (nb-21) | running | `pre-codex-coder` |

tp5 and the astra sweep have no baseline name in the brief, so none was recorded. The orchestrator decides whether they need one.

## Records (recorded values, from `state/baselines/*.json`)

Each record was written by `python -I scripts/baseline.py record <name> --since 2026-11-19T13:45:00Z --until 2026-11-19T21:45:00Z`, exit 0. Key values (`pre-tp-queue`, the other three print the same tool metrics):

```
merged_tasks 20, merged_per_hour 2.50, lead_median_s 1464, lead_p90_s 4255
review_rounds_per_merged 1.85, first_pass_fail_rate 45.0%
queue entry_median_s 592 (recorded over 50 entries that had ended; 51 entries existed)
test_job_median_s 98, build_lock_wait_s null (not recorded by Broadsword)
coder_tokens_per_merged 422877.00 (see the codex-coder section for the correction)
coder_tokens_unmerged 317950, coder_runs_recorded 5
```

`baseline.py list` after recording lists the four `pre-*` records (window 2026-11-19T13:45:00Z to 2026-11-19T21:45:00Z, 20 merged).

## pre-two-shadows: shadow numbers and entry times

Shadow-off cutoff. `git show -s cfadd9e --format='%aI %cI'` gives `2026-11-19T11:00:30-08:00` for both author and committer, which is 19:00:30Z. The first entry created without a shadow verdict is `q-20261119-190059` at 19:00:59Z. Shadow was switched back on by `2e56a97` (`git show -s 2e56a97 --format=%aI` gives `2026-11-19T12:08:11-08:00`, i.e. 20:08:11Z). The first shadow verdict after that is `q-20261119-201906` at 20:19:06Z.

Shadow stats (live, from `python -I scratch/nb08-shadow.py C:/Users/MrComputer/Desktop/Broadsword`, saved to `scratch/nb08-fix-shadow.out`):

```
entries created in window: 51
entries with a shadow verdict: 31
entries with no shadow verdict: 20
verdict counts: [('ERROR', 6), ('FAIL', 17), ('PASS', 8)]
ERROR rate among shadow runs: 6 of 31 = 19.4%
shadow_secs: n=31 median=35.3 min=3.4 max=146.1
entry created->ended (all entries in window): n=51 median=571s
```

Recorded (first run, before the record): 30 shadow entries, ERROR 6 / FAIL 17 / PASS 7, ERROR rate 20.0%, shadow_secs median 40.6 (n=30), entry median 592 (n=50). The difference is one in-window entry that was still running at the first count: `q-20261119-214452` (created 21:44:52Z, shadow PASS, ended 21:48:32Z, merged). The record's window end is fixed, but an entry created inside the window and ended after it is still counted on a later run. This is a caveat for every "before" taken from live state.

Entry time split (`python -I scratch/nb08-fix-entries.py C:/Users/MrComputer/Desktop/Broadsword`, saved to `scratch/nb08-fix-entries.out`):

```
ALL entries in window: entries=51 with_ended=51 median_s=571.0
SHADOW-ON (shadow_verdict present): entries=31 with_ended=31 median_s=616.0
SHADOW-OFF (no shadow_verdict): entries=20 with_ended=20 median_s=513.0
created before 19:00:30Z: entries=19 with_ended=19 median_s=823.0
created at/after 19:00:30Z: entries=32 with_ended=32 median_s=273.5
created before cutoff but no shadow_verdict: 0
created after cutoff but with shadow_verdict: 12
```

- Median entry time for shadow-on entries only: **616.0 s over 31 entries** (created 17:02:06Z to 21:44:52Z).
- Shadow-off entries: 20. Of these, 16 reached review and ran with no shadow (created 19:04:36Z to 20:08:12Z). The other 4 never reached review (`q-20261119-190059` and `q-20261119-190059-2` check_failed, `q-20261119-212541` cancelled, `q-20261119-214243` check_failed), so their "no shadow" is not a shadow-off effect. The shadow-off median above includes them.
- The 12 entries created after 19:00:30Z with a verdict are the entries from 20:19:06Z on, after the switch back on.

Shadow ERROR entries (live): `q-20261119-173300`, `q-20261119-175548`, `q-20261119-181652` (HTTP 429 rate limit, about 146 s each), `q-20261119-183236`, `q-20261119-184332`, `q-20261119-184830` (codex timed out after 144 s, then HTTP 429).

Machine-readable gap: `baseline.py` has no shadow metric, so the shadow ERROR rate and shadow seconds exist only in the numbers above. A later comparison needs `scratch/nb08-shadow.py` (in `scratch/`, which can be wiped) or a metric added to `baseline.py`. Not done here; `scripts/` is outside this claim.

## pre-tp-buildlock: build-lock wait

The Broadsword job records and logs do not record the build-lock wait (`build_lock_wait_s` is null in every record). The wait was therefore estimated from the job logs with `scratch/scale/lockwait.py`: the time from the job id (registration) to the first `=== <kind> ... (UTC) ===` header line in its log. This is an estimate that includes any setup before the lock, and jobs refused at the lock have no header, so they are not counted.

10-slot window (from `scratch/scale/lockwait-10.txt`, p50/p90 from the 10-slot window). Its exact arguments were not stored, so it was re-run on the window stated in `scratch/scale/baseline-10.txt` (19:20:00Z to 20:09:47Z):

```
python -I scratch/scale/lockwait.py scratch/logs/jobs 192000 200947
jobs 85  median 1s  p90 88s  max 105s  over 60s 10
```

This reproduces `lockwait-10.txt` exactly. The 10-slot setting was in force from 19:11:18Z (`04c916b`) to 20:19:06Z (`ce8c6ee`).

8-hour window used by `pre-tp-buildlock` (13:45Z to 21:45Z), same script:

```
python -I scratch/scale/lockwait.py scratch/logs/jobs 134500 214500
jobs 311  median 1s  p90 67s  max 873s  over 60s 34
```

This window mixes 6-slot and 10-slot periods, so it is a mixed baseline, not a clean one. The 900 s refusal at 17:14:44Z (`scratch/logs/jobs/20261119-171444-broadsword-run.log`) is not in these figures: it has no header line, so the script does not count it. Its 900 s wait is known from the log text only.

## pre-codex-coder: Sonnet coder per task

Rounds and sequences per merged task (`python -I scratch/nb08-fix-rounds.py C:/Users/MrComputer/Desktop/Broadsword`, saved to `scratch/nb08-fix-rounds.out`). A round is a queue entry with status `merged` or `rejected` (a review ran). `check_failed` and `cancelled` entries never reached review. The count uses all entries of the task, whatever their date. The window has 20 merged tasks (the record's 20). Entry `q-20261119-214452` merged but ended at 21:48:32Z, after the window, so it is not counted.

Sonnet-coded tasks merged in the window (12 of the 20):

| Task | Review rounds | Sequence | Sonnet coder time and tokens (`state/agent-runs.tsv`) |
|---|---|---|---|
| `t-20261118-075707` | 4 | FAIL, FAIL, FAIL, PASS | 1133 s, 174509 tokens (2026-11-18 row, outcome partial) |
| `t-20261118-080157` | 5 (+1 check_failed) | FAIL x4, check_failed, PASS | 141 s + 1631 s = 1772 s; 112345 + 240950 = 353295 tokens (two 2026-11-18 rows) |
| `t-20261119-171333-12` | 2 (+1 check_failed) | check_failed, FAIL, PASS | no row |
| `t-20261119-171333-13` | 1 | PASS | no row |
| `t-20261119-171333-14` | 1 | PASS | no row |
| `t-20261119-171333-23` | 4 | FAIL x3, PASS | no row |
| `t-20261119-171333-29` | 1 | PASS | no row |
| `t-20261119-171333-3` | 2 | FAIL, PASS | no row |
| `t-20261119-171333-31` | 1 | PASS | no row |
| `t-20261119-171333-46` | 2 | FAIL, PASS | no row |
| `t-20261119-171333-8` | 2 | FAIL, PASS | no row |
| `t-20261119-175203` | 1 | PASS | no row |

Commands: `grep -P "\t<task>\t" state/agent-runs.tsv` for each task (`scratch/nb08-fix-sonnet-runs.txt`).

- Review rounds are present for all 12 Sonnet tasks. Coder time and tokens are present for only 2 tasks (`t-20261118-075707`, `t-20261118-080157`). For the other 10, Sonnet coder time and tokens per task are not recorded, so no per-task time exists for them.
- Correction to the recorded `coder_tokens_per_merged` (422877): the record divides all five recorded coder runs (845754 tokens, `per_model`) by the 2 merged tasks that have a run. Those five runs include 317950 tokens of unmerged tasks (`coder_tokens_unmerged`). The merged tasks' own runs total 845754 - 317950 = 527804 tokens (174509 + 112345 + 240950), so the per-task figure for the two merged tasks is 527804 / 2 = 263902 tokens. The JSON is unchanged; the 422877 figure stays a recorded value with this definition.
- Aggregate first-pass figure for all merged tasks (recorded): `first_pass_fail_rate` 45.0%, `review_rounds_per_merged` 1.85.

## Not done or not verified

- The four JSON records are not re-recorded. The live recomputation differs (entry median 571 vs 592, shadow count 31 vs 30); the difference is an entry still running when the first record was taken.
- tp5 and the astra sweep have no baseline name in the brief; nothing was recorded for them.
- tp6 was not found on the board (see above).
- Build-lock wait is an estimate from job logs, not a Broadsword measurement, and the 8-hour figure mixes slot settings.
- Shadow statistics are not in any JSON record; the script that computes them is in `scratch/`.
- No code changed. No Broadsword build, test or run job was started in this round.
