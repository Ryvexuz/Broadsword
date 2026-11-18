# Reviewer step and merge queue — done check

**Check:** a planted bug gets caught before merging.

**Result: PASS**, 2026-11-18, merged to Broadsword `main` at merge commit `9ec7442`.

This is Stage 3 milestone 3. The queue runs a task's branch through a check, an independent review and a fast-forward merge, and only a PASS merges.

Related commits: coder `965d52f` (first pass), fresh coder `671ca93` (fix round), same fresh coder `a173b7a` (follow-up). In the done check, the planted commit was `7ab23fc` and the fix was `9cd162f`, both on crash-sim `main`.

## What was built

- **Commands:** `queue add <task> --risk low|high --coder <model> [--brief <file>]`, `queue list`, `queue show <entry>`, `queue run [--entry <id>]` and `queue cancel <entry>`. All are registry tools. The tool count is now 32.
- **Storage:** entries in `state/queue/<id>.entry`, reviews in `<id>.review`. A review record holds the reviewer CLI path and whether a test hook was used, the model requested and the model the backend reported, the reviewed commit and the diff base, the cost and tokens, the verdict, the findings and the raw output.
- **`queue run` steps:**
  1. Preconditions: the main checkout is clean and on a branch, the task is running, and the worktree is clean. Ignored files are not counted.
  2. Rebase onto the main HEAD. On a conflict the rebase is aborted and the branch tip is verified unchanged. The pre-rebase tip is kept under `refs/broadsword/queue/<id>/pre-rebase`, created only if absent.
  3. `check --task`. On failure the entry is `check_failed` and no review is called, so no money is spent.
  4. Review. The model is chosen by risk: `reviewer_model_low = haiku`, `reviewer_model_high = opus`. The reviewer must be a different model family from the coder. The reviewer sees exactly `<base>...<checked_tip>`, the check summary and the check output tail. It is Claude CLI, run headless, with the prompt on stdin. Its environment is scrubbed of secrets by name and by value, and every `.env` value is redacted from the prompt and from stored text. Output that is not the CLI's JSON, or comes from the wrong model family, counts as a FAIL. Calls are capped by `reviewer_max_calls_per_entry = 3`.
  5. Merge: `git merge --ff-only <the exact checked and reviewed sha>`. If the branch or main moved, the entry is `stale`.
  6. Check on main. On failure the entry is `merged_check_failed`, reported loudly, and main is not reset.
  7. `task done`, which removes the worktree and the branch.
- **Locks:** `queue.lock` is held for the whole run. `entries.lock` is held briefly.
- **Crash-resume:** a run resumes from the recorded step.
- **Statuses:** queued, running, merged, blocked, conflict, stale, check_failed, rejected, merged_check_failed, cancelled.
- **Exit codes:** 0 merged, 1 internal, 2 bad input, 3 blocked/conflict/stale, 4 busy, 5 rejected/check_failed, 6 merged_check_failed.
- **Settings:** 10 new required settings, including the reviewer timeout, the diff, brief and check-tail size limits, the queue lock wait and the call cap.
- **Reviewer CLI:** `reviewer_cli = "claude"`, found on PATH. On this machine the environment variable `BROADSWORD_REVIEWER_CLI` overrides it with the real install path. That path is outside the project and is never written to a file. An override inside the root is refused, unless it is inside scratch, where it counts as a test hook and is announced with `TEST HOOK ACTIVE`.

## How it was built

| Pass | Who | Commit | Tokens | Time |
| --- | --- | --- | --- | --- |
| First | Coder (Sonnet) | `965d52f` | about 384k | 23 min |
| Fix round | FRESH coder (Sonnet) | `671ca93` | about 250k | 14 min |
| Follow-up | same fresh coder | `a173b7a` | about 3k more | |

The follow-up made the fake reviewer report `fake-model` by default, and documented the redaction limits.

## Review

**Opus reviewer (high risk, because it merges and rebases), first pass: FAIL**, about 182k tokens.

- **B1.** A commit added between check and review could be merged unchecked. The review re-read the branch tip and never compared it with the checked tip. The reviewer reproduced this: it killed a run mid-review, committed non-compiling code, resumed, and the code landed on main.
- **B2.** `.env` values with ordinary names (`DB_PASSWORD`) reached the reviewer's prompt, and environment variables such as `GITHUB_PAT` reached its environment.
- **B3.** `BROADSWORD_REVIEWER_CLI` could point at the shipped fake reviewer silently. The reported model was never checked, and non-JSON output was parsed as a verdict.
- **Minor points:**
  - M1: an empty `BLOCKING:` header followed by items passed.
  - M2: there was no cap on review calls.
  - M3: the same task could be queued twice.
  - M4: model names were compared as raw text.
  - M5: the check output was stored unredacted.
  - M6: an ignored `.env` blocked the queue permanently.

**Fix round.** All items were fixed. 12 of the new tests fail against the old code.

**Opus reviewer, second pass: PASS**, cumulative about 228k tokens.

- The reproductions were re-run.
- The attack attempts failed: the gap between the tip check and the merge is closed, because the merge is by exact sha; the call cap holds across resumes; path tricks (junctions, `..`, case) failed.
- One minor point remained: a copy of the fake reviewer placed outside the root passed as real. `a173b7a` fixed it.

**Before the fix round,** the orchestrator checked the 30 real benchmark runs. Each reported exactly one model, matching the requested family, so the strict model-family check does not trip on real reviews.

## Tests

- 40 queue tests, using a fake reviewer (`tests/support/fake_claude.rs`, built as a `[[bin]]`). By default the fake reports `fake-model`, so a stray copy fails the model check.
- Broadsword's full suite, run through Broadsword with `broadsword test broadsword`: 343 passed, 0 failed.

## Done check

The check ran on the real crash sim, with real paid reviews.

- **Task** `t-20261118-010116`: "Add --gravity option to the crash sim". Claims `src`, `tests` and `README.md`. It ran in a worktree. The coder was recorded as `opus` (the orchestrator), and the risk was low, so the reviewer was Haiku.
- **Brief** (`scratch/m3-3/brief.md`): add `--gravity <m/s^2>`, default 9.81, finite and greater than 0, documented. The simulation must use it: `--gravity 1.62` must fall more slowly.
- **Planted commit `7ab23fc`.** `gravity` was added to `Config` with default and validation, plus the `--gravity` flag, the README line and a validation test. The physics still used the `GRAVITY` constant, so the flag did nothing. The check passed: 12 of 12 tests.
- **Entry `q-20261118-010133`: rejected.** Haiku, about 11 s, cost $0.0014. Finding: "src/lib.rs:59 - The diff only sets `gravity: GRAVITY` as the default and never changes the integrator to read `cfg.gravity`, so … `crash-sim --gravity 1.62` falls at 9.81 and the required slower fall does not happen." Main stayed at `8f9b2c6`. The task and worktree were kept.
- **Fix commit `9cd162f`.** The integrator uses `self.cfg.gravity`. The new test `lower_gravity_makes_the_cube_fall_more_slowly` checks first contact against about √(2h/g), and that the moon time is more than twice the earth time. The test FAILS on the planted code (checked by reverting the line) and passes on the fix.
- **Entry `q-20261118-010224`: merged.** Steps in order:
  1. preconditions;
  2. rebase, not needed;
  3. check, 13 of 13;
  4. review: Haiku PASS, cost $0.0017;
  5. merge `--ff-only`;
  6. check on main, 13 of 13, after the shared build folder cleaned crash-sim because its build-source note had changed;
  7. `task done`: worktree and branch removed.
- crash-sim main is now `9cd162f`, status clean.
- **Both reviews together cost $0.0031.** Each review record shows `model: haiku`, `models reported by the backend: claude-haiku-5-5` and `test_hook: false`.
- **Flag check.** First ground contact at t = 0.65 s with the default gravity, and at 1.58 s with `--gravity 1.62`. Theory gives √(2·2/1.62) = 1.57 s.

## Problems and limits, recorded

- **The fast-forward merge kept both commits on crash-sim main:** the buggy `7ab23fc`, then the fix `9cd162f`. The final tree is correct and tested, but the history contains a commit that was rejected on review. IDEAS: squash a task's commits into one reviewed commit before merging.
- **The fake reviewer ships as a `[[bin]]` in the product crate,** so it is built into `build/debug`. It is guarded: it reports `fake-model`, and it is refused from inside root except under scratch. Making it test-only is cleaner. IDEAS item.
- **Redaction limits are documented.** A secret split across two diff lines is not redacted. A `.env` value that is also normal code (for example `return`) is blanked everywhere, which hides that code from the reviewer.
- **Remaining minor point from the review:** an edit made and undone in the worktree during the check leaves the tip unchanged, so the check may have run on uncommitted files. This needs deliberate effort to trigger. IDEAS item: check from a clean export of the commit, not the live worktree.
- **The coder's worktree started on the Stage 0 commit again,** the ninth time.
- **`BROADSWORD_REVIEWER_CLI` is needed on this machine** for real reviews, because `claude` is not on PATH.

## Cost

- Subagent tokens: coder about 384k, fresh coder about 253k, Opus reviewer about 228k (cumulative).
- Real reviews in the done check: $0.0031 together.
- Wall time: about 2 h 15 min.
