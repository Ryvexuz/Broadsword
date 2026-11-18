# bench/coder: a coding benchmark for models

Six Rust tasks, from easy to hard, each graded by hidden tests. It compares how well different models write code: Claude old and new now, OpenAI later.

**One-shot track (this folder).** The model gets a spec and the exact public signatures, with no tools and no iteration, and returns one Rust source file. Every model gets the same single chance. The **agentic track** (tools, running tests, several turns) comes later; it is not built.

| # | task | difficulty |
|---|------|------------|
| 01 | `duration`: parse `2h10m5s` into seconds, with typed errors | easy |
| 02 | `ring-buffer`: fixed-capacity buffer, oldest-first double-ended iterator | easy/medium |
| 03 | `merge-intervals`: merge overlapping and adjacent closed `i64` intervals | medium |
| 04 | `ttl-lru`: LRU cache with per-entry time-to-live, O(1) operations | medium/hard |
| 05 | `expression`: evaluator with precedence, unary minus, overflow, error positions | hard |
| 06 | `line-diff`: minimal edit script between two line lists | hardest |

## Layout

```
config.toml        every setting (timeouts, paths); required keys, no defaults in code
harness.py         grade solutions against the hidden tests
prove.py           prove the tasks are sound (references pass, wrong solutions fail)
check_specs.py     check that no spec.md leaks test names or test data
run.py             ask a model for each task, extract the code, grade it
score.py           table of results
support/common.rs  `within(...)`: the per-test timeout, copied next to the tests
tasks/<nn>-<name>/
    spec.md        what the model sees: behaviour, signatures, errors, constraints
    reference.rs   a correct solution
    wrong/*.rs     at least two plausible but wrong solutions
    tests.rs       the hidden acceptance tests. NEVER sent to a model
```

## How grading works

`harness.py` writes the solution as `src/lib.rs` of a small crate (package `solution`, no dependencies, edition 2021, `#![forbid(unsafe_code)]` added on top) under `scratch/coder-bench/crates/<task>/`, puts `tests.rs` in as `tests/acceptance.rs`, and runs `cargo test --offline --test acceptance`.

It goes through Broadsword, never plain cargo: one `broadsword build/run`-style job per batch,

```
broadsword run broadsword [--task <id>] -- python -I -B harness.py --inner <spec.json> <result.json>
```

and the inner step runs cargo once per solution. So every grading gets the disk guard, the shared build folder, a Git checkpoint and a job log, and the wait for the build lock is paid once per batch (the lock can be busy for minutes). `prove.py` and `run.py` each grade all their solutions in one batch. Set `BENCH_BROADSWORD_TASK=<task id>` to run the job under a task's worktree.

Result per solution: `compiles`, `passed`, `failed`, `total`, `wall_s`, `first_error`, `timed_out`.

- A solution that does not compile scores 0 and keeps the first compiler error.
- `total` is the number of `#[test]` functions; `failed = total - passed`, so a crash that kills the test binary still fails the unfinished tests.
- **Per-test timeout:** every call into the solution runs inside `within(...)` (a thread with a timeout, `test_timeout_seconds` in `config.toml`), so a looping solution fails that test instead of hanging. Each cargo run is also killed after `harness_timeout_seconds`.
- A test that panics, or a solution with `unsafe`, fails or does not compile; nothing else is skipped.

## Proof that the tasks are sound

```
python -I -B bench/coder/prove.py          > scratch/coder-bench/prove.txt
python -I -B bench/coder/check_specs.py
```

`prove.py` runs every `reference.rs` and every `wrong/*.rs` through the harness and prints a table. The rules have no exceptions:

1. every reference compiles and passes **all** its tests;
2. every wrong solution compiles and fails **at least one** test (a wrong solution that does not compile does not count: the tests must really bite);
3. every task has at least two wrong solutions.

`check_specs.py` fails if a `spec.md` contains a test function name, a string literal from `tests.rs` (2 or more characters) or an integer pair from the tests. The only allowed overlap is the `i64` limits the expression spec must state.

A task without a passing proof is not done.

## Adding a task

1. Make `tasks/<nn>-<name>/` with `spec.md`, `reference.rs`, `tests.rs` and `wrong/`.
2. In `spec.md` give the behaviour, the exact public signatures, the error cases and the constraints (std only, no `unsafe`, one `lib.rs` file). Do not mention tests. Use different example values from the ones in `tests.rs`.
3. In `tests.rs` start with `mod common; use common::within; use solution::*;` and call the solution only through `within(...)` (closures must be `Send + 'static`). Name test functions `hidden_...`. Every `#[test]` is one scored test.
4. Write at least two wrong solutions that are plausible, such as an off-by-one, a missed edge case, a non-minimal answer or a too-slow one. Each must compile.
5. Run `prove.py` and `check_specs.py`. Fix the tests (never the proof rules) until both pass.

## Running a model

`run.py <out-dir> <provider> <model> <effort|default> [task ...]`

`provider claude` runs the headless Claude Code CLI exactly like `bench/reviewer/run.py`: `BENCH_CWD` is a folder **outside the project** (no `CLAUDE.md`, `AGENTS.md` or `.git` in it or any parent; the runner checks), `--tools ""`, `--strict-mcp-config`, `--max-turns 1`, the prompt on stdin, JSON output. The model sees only `spec.md` plus a short instruction. `BENCH_CLAUDE` may point to the CLI.

It extracts the code from the single fenced code block of the answer. No block, or several blocks, is a recorded format failure (score 0). Then it grades all tasks in one batch and writes `<out-dir>/<task>__<provider>__<model>__<effort>.json` with: provider, model, effort, task, tokens, cost_usd (from the CLI's JSON), wall_s (the model call), harness_wall_s, compiles, passed, total, format_failure, first_error. The raw CLI output, the answer and the extracted `.rs` are kept next to it.

`provider openai` is a stub that raises "not built yet". It will reuse the request code of `bench/reviewer/run_openai.py` once that exists, with cost computed only from prices given explicitly.

Every Claude model, at its default effort and at `high` (run from the project root; `<out>` is for example `scratch/coder-bench/results`, and `BENCH_CWD` any empty folder outside the project, for example one under the system temp folder):

```
BENCH_CWD=<outside-folder> python -I -B bench/coder/run.py <out> claude haiku  default
BENCH_CWD=<outside-folder> python -I -B bench/coder/run.py <out> claude sonnet default
BENCH_CWD=<outside-folder> python -I -B bench/coder/run.py <out> claude sonnet high
BENCH_CWD=<outside-folder> python -I -B bench/coder/run.py <out> claude opus   default
BENCH_CWD=<outside-folder> python -I -B bench/coder/run.py <out> claude opus   high
BENCH_CWD=<outside-folder> python -I -B bench/coder/run.py <out> claude fable  default
BENCH_CWD=<outside-folder> python -I -B bench/coder/run.py <out> claude fable  high
```

`--effort` is passed to the CLI only when the effort is not `default`. Which models accept it depends on the CLI version: haiku is listed at `default` only because it is the one most likely to reject `high`; if you want to try it, `... haiku high` is recorded like any other run (a rejected call shows as a failed call with the CLI's error in `first_error`). Running a model costs money: ask before you start (Broadsword rule).

Then:

```
python -I -B bench/coder/score.py <out>
```

prints one row per model and effort, one column per task (passed/total), the total score, compile failures, format failures, cost and model-call time.

## Settings (`config.toml`)

`broadsword_exe`, `broadsword_project`, `work_dir` (all relative to the Broadsword root, found as the nearest parent folder holding `build/bin/broadsword.exe`), `test_timeout_seconds`, `harness_timeout_seconds`, `model_timeout_seconds`, `lock_wait_seconds`. All are required.

## Known limits

- One batch at a time: the temp crates are reused, guarded by `scratch/coder-bench/harness.lock`. A killed run can leave that lock file behind; delete it if no harness is running.
- A timed-out test leaves its thread running until the test binary exits; that is fine for correctness but can slow the rest of the run.
- Tests run in a debug build, so the performance tests (ttl-lru, line-diff) are sized for that.
- One run per model and effort: model answers vary, so a single run is a sample, not a rank.
