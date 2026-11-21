# Parallel-tests trial: the two failed tests, repeated alone (t-20261121-005258)

Date: 2026-11-21 (session clock). Branch: task/t-20261121-005258, checkout at main c8f94194.

## Why

The parallel-tests trial (`evidence/parallel-tests-trial-2026-11.md`, branch task/t-20261120-193628-2, cbacf3aa) had one failure at p3 and one at p4. The rule needs 0 new failures, so `test_parallel_binaries` stays 1 until the cause is known.

- p3-2: `a_chain_rebase_a_killed_drain_left_half_done_is_aborted_and_the_entry_still_merges` (queue.rs:2584 on the trial branch)
- p4-3: `a_failing_hook_that_dirties_the_worktree_keeps_it_and_the_branch_and_names_them` (worktrees.rs:1200 on the trial branch)

## Method

Each test was run 20 times alone, one at a time, in the foreground, from the main checkout:

`build\bin\broadsword.exe run broadsword --task t-20261121-005258 -- cargo test --offline --test <queue|worktrees> <name> -- --exact`

`--exact` runs exactly one test per run, so the test-binary parallelism setting never came into play. CPU load at start is the same measure as `scratch/t-20261120-174501/runone.sh` (Win32_Processor average LoadPercentage, taken just before the run). Raw output of every run is in `scratch/t-20261121-005258/<test name>-<n>.txt` (scratch, may be wiped). The load is a snapshot before the run: other agents were using the machine, and the load changed during runs. The time is the test's own `finished in` figure.

Not controlled: the builds here use the task worktree at main c8f94194, not the trial branch, so the line numbers of the two asserts differ from the ones named above (here queue.rs:2584 is the exit-code assert of the second drain and worktrees.rs:1200 is the `LEFT BEHIND` assert). The test bodies were not changed by me and I did not diff them against the trial branch.

## Raw table (20 runs each)

| run | queue test: result | time | CPU load % at start | worktrees test: result | time | CPU load % at start |
|---|---|---|---|---|---|---|
| 1 | pass | 40.40s | 54 | pass | 1.29s | 72 |
| 2 | pass | 63.48s | 34 | pass | 0.99s | 45 |
| 3 | pass | 167.21s | 100 | pass | 10.06s | 23 |
| 4 | pass | 27.68s | 97 | pass | 9.79s | 91 |
| 5 | pass | 21.65s | 20 | pass | 10.34s | 90 |
| 6 | pass | 13.83s | 26 | pass | 11.04s | 100 |
| 7 | pass | 14.41s | 25 | pass | 10.71s | 100 |
| 8 | pass | 39.68s | 25 | pass | 11.15s | 97 |
| 9 | pass | 18.02s | 34 | pass | 10.91s | 99 |
| 10 | pass | 13.86s | 26 | pass | 10.78s | 100 |
| 11 | pass | 13.77s | 11 | pass | 7.18s | 97 |
| 12 | pass | 14.42s | 31 | pass | 5.05s | 99 |
| 13 | pass | 13.94s | 26 | pass | 1.30s | 90 |
| 14 | pass | 13.94s | 29 | pass | 0.99s | 77 |
| 15 | pass | 14.25s | 15 | pass | 0.83s | 43 |
| 16 | pass | 14.11s | 28 | pass | 0.86s | 10 |
| 17 | pass | 14.91s | 42 | pass | 1.51s | 41 |
| 18 | pass | 13.42s | 15 | pass | 0.98s | 46 |
| 19 | pass | 25.06s | 30 | pass | 1.01s | 81 |
| 20 | pass | 23.90s | 90 | pass | 1.01s | 73 |

Counts: queue test 20 pass, 0 fail; worktrees test 20 pass, 0 fail. All 40 runs exited 0.

## What the timings show

- queue test: median about 14s, but 5 of 20 runs took 25s or more and run 3 took 167s (CPU 100% at start). The test waits for a hook marker file with `wait_for(..., 240, ...)` (queue.rs, `ceiling(secs)`), so run 3 used about 70% of that 240s ceiling before any scaling by `ceiling`. A busy machine can push this test near its wait limit. I did not read `ceiling` to see whether it scales the 240s.
- worktrees test: bimodal, about 1s or about 10s (runs 3 to 12 were 5 to 11s, the rest about 1s). The slow runs were under CPU load 90 to 100 at start, with the exceptions of run 3 (load 23) and run 12/13. The test has no explicit wait: it runs `task start --worktree` with a failing hook and then asserts on stderr and files, so a failure at the `LEFT BEHIND` assert would mean the rollback took a different path (for example git's `worktree remove` succeeding or not seeing the untracked file), not a timeout. I did not find a timing assumption in the test body.

## Verdict

Neither test failed in 20 runs alone (0 of 20 each). This says only that the failure does not show up often when the test runs alone on this machine, in this checkout, under the loads seen (10 to 100%).

- With 0 failures in 20 runs, a flake of 5% per run would be missed about 36% of the time ((0.95)^20 = 0.358). Only a rate of about 14% or more would be caught with 90% probability. So a low-rate flake that does not depend on the setting is NOT ruled out.
- A parallel cause (two test binaries running at once, with the CPU and disk contention that brings) stays possible. These runs did not run any binary in parallel.
- The runs do not show whether the setting caused the p3 and p4 failures, and do not show they are independent of it. Claim: the setting stays 1 on this evidence; nothing here supports changing it.
- I did not capture the original failure messages from the trial, so I cannot say which assertion failed there.

## Next measure (not run)

Reproduce the trial's condition: run the full `test broadsword` with `test_parallel_binaries = 2` (and the same again with 1 as the control) at least 20 times each in a scratch clone, as `scratch/t-20261120-174501/runone.sh` does, saving each run's output and the name of any failing test, with CPU load at start. Twenty full runs is about the same detection power as here (36% miss at 5%), so use 40 per arm if the budget allows. Also read the trial's failing output for the actual assert text first, which costs nothing.
