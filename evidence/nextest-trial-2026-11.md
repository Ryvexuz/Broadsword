# cargo-nextest 0.9.152 vs `cargo test` trial (2026-11-20)

Task t-20261120-142852. Measurement only: no code or setting changed. Raw logs: `scratch/nextest/*.log` and `*.load` (scratch can be wiped; the numbers and quoted lines are copied here). Every command went through `build/bin/broadsword.exe run <project> [--task t-20261120-142852] -- cargo ...`, so each took the build lock and the shared build folder. broadsword = worktree of this task; crash-sim = its main checkout (read-only use).

Commands:
- A: `cargo test --offline`
- B: `cargo nextest run --offline --no-fail-fast`
- R: `cargo nextest run --offline --no-fail-fast --retries 2`

Order: broadsword A1 B1 A2 B2 A3 B3 (A1 was a first probe run by hand, the rest by script), then crash-sim A1 B1 A2 B2 A3 B3, then R for broadsword and crash-sim. Wall = the `run: ok (N s)` line of the job summary (includes the 4-8 s compile that `run` always does, because it cleans the project's package in the shared folder first). "Summary" = nextest's own test-phase time.

## Runs

CPU load = Win32_Processor LoadPercentage average at the start of the run. NOISY = load >= 50 at start (other agents building) or an obvious outlier.

### broadsword (629 tests, 18 binaries)

| run | load at start | wall (job summary) | passed / failed / skipped | note |
|---|---|---|---|---|
| A1 cargo test | 7 | 398.7 s | 629 / 0 / 0 | quiet |
| B1 nextest | 19 | 329.6 s | 629 / 0 / 0 | |
| A2 cargo test | 34 | 704.4 s | 629 / 0 / 0 | NOISY outlier: the 85 queue tests took 450.00 s here vs 220.03 s (A1) and 216.11 s (A3) |
| B2 nextest | 56 | 327.5 s | 629 / 0 / 0 | NOISY by load, but not slower |
| A3 cargo test | 23 | 384.6 s | 629 / 0 / 0 | |
| B3 nextest | 17 | 329.1 s | 629 / 0 / 0 | |
| R nextest --retries 2 | 97 | 681.1 s | 629 / 0 / 0 | NOISY (load 97); not a timing sample |

Medians (3 runs): A = 398.7 s, B = 329.1 s (B is 17% faster). Excluding the noisy A2, A is 384.6 and 398.7 s (mean 391.7 s): B is still about 16% faster. B's three runs are within 2 s of each other (327.5 to 329.6 s) even with load 19, 56, 17; A's spread is 384.6 to 704.4 s. nextest's wall is bounded by its slowest test group (the queue tests, single biggest: `PASS [ 197.906s] ... one_bad_entry_in_a_train_of_four_is_found_by_bisection...`), so it is less sensitive to load than the serial per-binary run of cargo test, but only one quiet pair exists, so treat the size as +-10%.

Raw lines:
- A1: `test result: ok. 85 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 220.03s` ... `run: ok (398.7 s)`
- A2: `test result: ok. 85 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 450.00s` ... `run: ok (704.4 s)`
- A3: `run: ok (384.6 s)`
- B1: `Summary [ 328.961s] 629 tests run: 629 passed (45 slow), 0 skipped` / `run: ok (329.6 s)`
- B2: `Summary [ 326.965s] 629 tests run: 629 passed (44 slow), 0 skipped` / `run: ok (327.5 s)`
- B3: `Summary [ 328.686s] 629 tests run: 629 passed (44 slow), 0 skipped` / `run: ok (329.1 s)`
- R: `Summary [ 670.216s] 629 tests run: 629 passed (105 slow), 0 skipped` / `run: ok (681.1 s)`

cargo test counts: the per-binary `test result:` lines sum to 629 passed, 0 failed, 0 ignored in A1, A2 and A3 (199+25+19+18+17+7+23+22+24+5+85+23+2+12+45+13+52+38). Same 629 as nextest, so nextest ran every test cargo test ran. The broadsword package has no doc-tests (`grep -c "Doc-tests" broadsword-A1.log` = 0).

### crash-sim (378 tests, 31 binaries)

| run | load at start | wall (job summary) | passed / failed / skipped | note |
|---|---|---|---|---|
| A1 cargo test | 15 | 22.6 s | 378 / 0 / 0 | quiet |
| B1 nextest | 32 | 12.7 s | 378 / 0 / 0 | |
| A2 cargo test | 65 | 23.3 s | 378 / 0 / 0 | NOISY by load (no visible slowdown) |
| B2 nextest | 79 | 14.6 s | 378 / 0 / 0 | NOISY by load |
| A3 cargo test | 96 | 25.4 s | 378 / 0 / 0 | NOISY by load |
| B3 nextest | 97 | 14.6 s | 378 / 0 / 0 | NOISY by load |
| R nextest --retries 2 | 58 | 12.8 s | 378 / 0 / 0 | |
| (stdout probe) | - | 14.4 s | 378 / 0 / 0 | extra run to see which stream carries the summary |

Medians: A = 23.3 s, B = 14.6 s (B is 37% faster). Both arms had mostly noisy starts, yet they are consistent (A 22.6 to 25.4, B 12.7 to 14.6). The wall includes the compile, e.g. `Finished `test` profile [unoptimized + debuginfo] target(s) in 4.23s` (A1) and `in 4.11s` (B1); the test phase alone is nextest `Summary [   4.593s] 378 tests run: 378 passed, 0 skipped` (B1), `Summary [   6.486s]` (B2), `Summary [   6.121s]` (B3). crash-sim has a doc-test binary but it holds no tests (cargo-test totals 378 = nextest 378).

## Flakes

- Across 3 A runs and 3 B runs per project: no test failed in one run and passed in another. Every run was 0 failed.
- `--retries 2` run, broadsword: no `FLAKY` line, no `TRY 2` line (`grep -h "FLAKY\|TRY" scratch/nextest/*.log` printed nothing). Final line `Summary [ 670.216s] 629 tests run: 629 passed (105 slow), 0 skipped`.
- `--retries 2` run, crash-sim: same, `Summary [   4.391s] 378 tests run: 378 passed, 0 skipped`, no FLAKY.
- So this trial found no flakes. Limit: the retries run on broadsword was at load 97 and took 681 s, and only 1 retries run per project was done. The flake detection works only if a flake happens during that run; this is a sample of one.

## Output-format finding (what Broadsword's test step parses)

`crates/broadsword/src/job.rs`, parsing (lines 205-224):

```
/// Sum every `test result: ... N passed; N failed; N ignored; ...` line. None if no such line exists.
pub fn parse_test_results(output: &str) -> Option<TestCounts> {
    ...
        let Some(rest) = line.trim_start().strip_prefix("test result:") else { continue };
```

and the summary line (lines 724-740):

```
let counts = parse_test_results(&run.stdout);
...
(Outcome::Exited(Some(0)), None) => "test: ok — no `test result:` lines found, so no counts".to_string(),
(Outcome::Exited(c), None) => format!("test: FAILED ({}) — no `test result:` lines found", exit_text(*c)),
```

What nextest prints instead: `     Summary [ 328.961s] 629 tests run: 629 passed (45 slow), 0 skipped` and per-test `PASS [ 26.812s] (613/629) ...` lines. There is no `test result:` line at all (`grep -c "test result:"` on the nextest stderr = 0).

Further facts measured with a stdout-only vs stderr-only capture (`crash-sim-B-stdout-probe.log`, `crash-sim-B-stderr-probe.log`):
- nextest writes its whole report to STDERR. Stdout of the job holds only Broadsword's own lines (8 lines: `job: run crash-sim ...`, ..., `run: ok (14.4 s)`). The Summary line was in stderr only (`grep -c Summary`: stdout 0, stderr 1).
- Broadsword parses `run.stdout` only (`parse_test_results(&run.stdout)`; `tee(out, false, ...)` keeps stdout, stderr is printed and logged but not kept).

Would it break the parse?
- Exit codes: no. nextest exits 0 on success and non-zero on failure (101 for test failure); `run: ok` was printed for every run above.
- Counts: yes. With the current parser a nextest test step would give `test: ok — no `test result:` lines found, so no counts` on success (the passed/failed counts would be lost) and `test: FAILED (exit N) — no `test result:` lines found` on failure. `broadsword check` and the queue's check step use the same line, so the pass counts in reports would disappear, and a failing step would lose its `N failed` figure. The pass/fail decision itself still follows the exit code, so there is no false pass.
- A second, bigger point: Broadsword's `test` and `check` steps do not run `cargo test`. They run `cargo test --no-run --message-format=json-render-diagnostics` under the build lock, copy the test executables to `scratch/test-bins/<job id>/`, release the lock and run the copies one after another (module doc, lines 7-17), then `cargo test --doc` last. nextest cannot run those copies as they are: it needs `cargo nextest archive` plus `cargo nextest run --archive-file` (or `--binaries-metadata` with `--target-dir-remap`), and it does not run doc-tests at all (`cargo test --doc` would stay as a separate step). Also, the 3 gates in this document were measured with `broadsword run`, which runs the cargo command directly with the lock held for its whole length, so the lock-hold time in these B numbers (329 s for broadsword) is not the lock time the real test step has (the copies run outside the lock).

What a runner setting would have to change (not done):
1. A required key in `broadsword.toml` (and every test config), e.g. `test_runner = "cargo-test"` or `"nextest"`, read where `job.rs` chooses the test step; no default in code.
2. With `"nextest"`: build the executables as now, then run them through nextest from an archive built under the lock (archive step in place of `copy_all`), keeping the lock released while they run; keep a `cargo test --doc` step after it.
3. Parser: add a second parser for `Summary [ ...] N tests run: N passed (K slow), S skipped` (with `, F failed` when there is a failure: format for failures was not measured here, no test failed; it must be read from a planted failure before coding) reading STDERR (the run keeps only stdout today), and keep `parse_test_results` for the cargo-test setting. Failing tests appear as `FAIL` lines; flakes as `FLAKY`/`TRY n` lines (not seen).
4. Update the existing tests that pin the lines (`parses_test_results`, `summary_lines`, `test_results_match_plain_cargo_test`).

## Recommendation

Do not adopt as a setting yet; keep `cargo test` for the merge queue and `check`. Adopt for the run-by-hand case only if desired: `broadsword run <project> -- cargo nextest run --offline --no-fail-fast` already works today, needs no code, and was faster (broadsword 329.1 s vs 398.7 s median, crash-sim 14.6 s vs 23.3 s). Reasons not to wire it into `test`/`check` now: (a) the real test step runs copied executables outside the build lock, which nextest does not do without an archive step; (b) counts are on stderr and in a different format, so a runner setting needs a new parser and changes to 3 existing tests; (c) no doc-tests; (d) the benefit seen is 17% (broadsword) of a step that is dominated by one 220 s queue-test binary, and the trial found no flake to detect. A rerun on a quiet board is still worth doing before a code task: only the first broadsword A run was at load below 20 (A1 load 7), and the retries run was at load 97.

For the RETRO ledger:
- Measure: median wall of `broadsword run <project> -- cargo nextest run --offline --no-fail-fast` vs `cargo test --offline`, 3 alternating runs each, quiet board (load < 30 at start), plus the count of tests that nextest reports as FLAKY across 20 queue checks. Baseline today: broadsword A 398.7 s / B 329.1 s, crash-sim A 23.3 s / B 14.6 s, FLAKY 0 in 2 retries runs.
- Watch for harm: a `test:` summary line that says `no `test result:` lines found` (lost counts); any test that passes under `cargo test` but fails under nextest (process-per-test isolation changes shared-state behaviour; none seen here, 629 and 378 identical counts); a queue check that does not run the doc-tests; lock-hold time of the test step rising (nextest would run under the lock if wired as `cargo nextest run`).
- Revert signal: either of those first two, or no median gain on a quiet-board rerun.

## Limits of this evidence

- Load was measured only at the start of each run; other agents' builds during a run are not recorded. Only broadsword A1 (7) and B3 (17) / B1 (19) started below 20.
- One run (A2) is a clear outlier by a factor of about 1.8 and is kept in the median as the task asked; the median is unaffected (398.7 s is A1).
- Failure output of nextest was not measured (no failing test was planted); the Summary format for failures is therefore not quoted.
- A1 was run as a standalone first probe (not through the script); its load reading (7) was taken just before that command.
