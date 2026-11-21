# Spec checklist

The orchestrator goes through this before handing any task to a coder. Each item comes from a real review failure; the failure is in brackets.

In Stage 2, naming these cases in the spec up front let milestones pass review on the first or second try. In Stage 1, leaving them out meant every milestone failed its first pass.

## Every spec states

Start from `templates/brief.md` for the brief and `templates/coder-prompt.md` for the coder's prompt. Both already carry the rules below that apply to every task.

- **Goal and done check,** in the plan's own words. Write the done check as commands with the output each must show, so the coder, the reviewer and the orchestrator all run the same thing. If any part could be read two ways, say which reading is meant. [S3 close-out: "each reviewed by a different model" had two readings.]
- **Claim:** the exact files and folders the coder may change. Anything else is out of bounds.
- **Starting point:** which commit the work must start from. Tell the coder to check it and fast-forward if needed.
- **Build route:** how to build and test, either `broadsword build|test|check` or the documented exception. Also how to confirm that the binary in the shared build folder is the one just built.
- **Examples are examples:** a list in a brief is either marked "for example" or is the complete rule. Reviewers read an unmarked list as complete. [Stage 3 m4: a correct `max_speed()` call was rejected because the brief listed some public functions in parentheses.]
- **Standing rules touched:** name each `AGENTS.md` rule the change touches (secrets, deletion, settings, build folder), and how the change keeps it. [S3 audit: checkpoints were documented as saving `.env`, breaking the secrets rule.]
- **Done-check output:** until the queue runs done-check commands itself, the orchestrator runs them before queueing and puts their output in the brief, because the reviewer cannot run anything. [S3 audit: an unreachable guard passed review.]
- **Measurable before the merge:** if the done check measures the tool being changed (the queue, the job runner, the build lock), the brief says how to measure it with the branch's own binary against a scratch root, before `queue add`. [S4 tp1: two Opus FAILs on timings only the installed binary could produce.]
- **New JobSpec field, CLI flag or required config key:** the claim must list the `tools.rs` parser entry, every struct-literal site, `config.rs`, `broadsword.toml` and `tests/support/mod.rs`, plus any test file that builds its own config. (No mechanism enforces this yet.) Orchestrator check (output pasted in the brief): run `grep -rln "JobSpec {" crates` and `grep -rln "disk_floor_gb" crates` (both recurse into nested modules and `tests/`). Decidable: every path either command prints appears in the brief's claim, or the brief names that path with the reason it needs no change; a printed path that is in neither place fails the check. [2026-11-20: F5, F9 and CARGO_INCREMENTAL each stopped on a claim gap.]
- **What not to touch:** real repos, `projects/*`, Claude Code settings, the network, the real `.env`.

## Risk checks: name each one that applies

| Area | What to tell the coder | From |
| --- | --- | --- |
| Deleting or overwriting anything | Save the current bytes first, verified, and check before every write or delete. Never `--force`, `-D` or a recursive delete. Ignored files count as work. | S1 m4 rollback; S3 m2 `.env` |
| One rule enforced in many places | Put the rule in one function that every path goes through, and test that function directly. | S1 m4 (five passes); S3 m1 project names |
| Locks and concurrency | Use OS file locks (`File::try_lock`), never stale-lock takeover. Keep slow work outside the lock. Add a gated race test, plus a mutation check showing the test fails without the lock. | S3 m1; S3 m2 |
| Reusing reviewed code | Re-read the reused code's known limitations against the new requirement. | S3 m1 |
| Secrets | Protect them where they enter a child process: scrub the environment by name and by value. Redact before content reaches a model or a log. Never follow redirects with credentials. | S2 m3 |
| Settings | Every limit is a required key in `broadsword.toml`. No silent defaults, no hard-coded numbers. Every setting is read by code, with a test that fails if it is ignored. | S1 m3; S3 m1; S3 m4 `max_agents` |
| Output streams | A closed stdout or stderr must never kill a job (the crate's print macros already drop write errors; never `unwrap` a write to stdout). | S3 m4 `\| head` |
| Test-only hooks in the real binary | Limit them to scratch, print TEST HOOK ACTIVE, cap them, and make sure they can only make a guard stricter. | S1 m3; S3 m1 |
| Output an AI reads | Descriptions, exit codes and flags must match the binary. Exit codes are tested through the real binary. Hints such as `destructive` and `idempotent` must be true. | S1 m5; S3 m2 `task_done` |
| Protocol streams | stdout carries protocol only. Notifications and responses get no reply. Set size and depth limits. | S2 m1 |
| Failing loudly | Non-finite or impossible state must give a nonzero exit, never a quiet success. | S1 m2 |
| Paths | Everything stays relative to root. Check that a folder really is what the record says before acting on it (stray folders, junctions, `..`, case). | S1 m1; S3 m2 |
| Shared build folder | Building the same package from two checkouts can leave the other checkout's binary in place. | S2 close-out |
| Claim gaps | Run `scripts/brief_lint.py` (with `--root <repo>`) on the brief before task start; fix every FAIL; read every WARN. | 2026-11-20: 6 coder stops on claims in about 45 starts |

## Process

- **Review tier:**
  - High-risk changes get Opus as reviewer. High risk means deleting or rewriting files, locks, secrets, or anything that runs before or instead of a safety check.
  - Lower-risk changes get a cheaper reviewer from a different model family than the coder.
  - Record which tier was used and whether it caught anything. `python -I scripts/review_costs.py` totals cost, passes and fails by model for the retro.
- **Work ahead** (AGENTS.md rule): briefs for ready tasks live in `state/backlog/<task id>.md` (not in Git: a file written into a main checkout during a queue run blocks the merge), and the task is added to the board as `open`. Keep the next task's brief written and its claims checked, and start a coder on it while the queue checks and reviews the current one, provided its claims overlap nothing running or queued. Watch for ahead work that depends on an unmerged change. [S3 audit fixes: finished work waited up to 9 min for the queue; the pipeline was empty 62% of the time.]
- **Shadow-only findings** (rule B, Lazlo 2026-11-21, replacing the 2026-11-18 rule that every one needs an independent strong judge):
  - After each queue run, the gating reviewer, or the orchestrator, gives every BLOCKING line only the shadow reviewer raised a verdict with evidence: `real`, `false` or `known` (a duplicate; these are the verdicts `scripts/shadow_adjudicate.py` accepts) and records it there. A `real` one on main becomes a fix task.
  - A deterministic 1 in 5 of the `false` verdicts then gets an independent strong judge: neither the coder, nor the gating reviewer, nor the original judge. The sample is SHA-256 of entry id and finding key, mod 5 == 0, fixed in `scripts/shadow_spotcheck.py`; the judge cannot change it. `python -I scripts/shadow_spotcheck.py` lists the due rows with no result; `--record <entry>|<key> <real|false|unsure> <judge model> <evidence> --exclude <coder> --exclude <gating reviewer>` records one in `state/shadow-spotchecks.tsv` and refuses the original judge and any excluded model; `--check` exits 1 while any is open.
  - A spot check that overturns a `false` verdict (result `real`) is logged and, if the finding is real on main, becomes a fix task.
  - In force: `shadow_report.py --open` (open shadow-only findings fail the session-end check, `scripts/session_end_check.py:237`). *Not yet in force:* `shadow_spotcheck.py --check` is not yet called by `session_end_check.py`; until a task wires it in, the orchestrator runs it by hand at session end.
- **Non-coder agents write outside the checkout:** a brief for a research, replay or records agent says it writes only under `scratch/` or `state/staging/`, never in the main checkout. (No mechanism enforces this yet.) Orchestrator check (output pasted in the brief): run `git status --porcelain` in the main checkout before the agent starts (paste that output in the brief) and again after it finishes (paste it in the report); the second run must show no path that the first did not show. A path that is new in the second run fails the check, whoever created it. [2026-11-20: an untracked evidence file blocked 9 queue entries.]
- **Disputing a review finding** (2026-11-21, process gap 2; Lazlo's OK of 04:39Z):
  - A dispute NEVER edits the brief or the review prompt, and no text aimed at the reviewer goes into either. On 2026-11-21 the orchestrator argued findings inside briefs three times and each was blocked as steering the reviewer.
  - The only path: the coder adds a runnable test to the change itself that shows the finding does not hold, then `python -I -B scripts/review_dispute.py [--root <root>] --model <neutral gpt model> [--exclude <model> ...] <queue entry> "<finding text>" <test name>`.
  - The script refuses unless the finding text occurs in that entry's blocking, shadow_blocking or minor lines (or its review files), the test name is on an added line of the entry's diff, and the test run (through `broadsword run`) exits 0 with at least one test passed. A failing or empty test is recorded as `not_asked` and no model is called.
  - It then asks the tie-break model through `clients/openai_review.py` (same cap and ledger) with ONLY the finding, the diff and the test output (redacted first). The model must be a different family from the entry's coder, its gating reviewer and the orchestrator (opus), and from every `--exclude`; unknown family is refused. The client reaches OpenAI models only, so a gpt gating reviewer leaves no neutral model and the script refuses.
  - The ruling (`real`, `false` or `unsure`) is appended to `state/review-disputes.tsv`. The script changes nothing else; the orchestrator acts on the ruling (a `false` ruling still needs the gating reviewer's own pass in the next round).
  - *Not yet in force:* the requested `medium` effort is NOT applied, because `clients/openai_review.py` has no effort flag (the tsv `effort` column says so); no step of `queue run` or `session_end_check.py` calls this script, and its test file is not in the `python_tests` list of `broadsword.toml` until a task claims that file.
  - Measure: disputes recorded, rulings, and whether a disputed finding that ruled `false` was later raised again as a REAL finding. Baseline from existing data: 3 brief-borne disputes blocked on 2026-11-21 and no `state/review-disputes.tsv` rows. Judge it at the next retro; revert if rulings are `unsure` or error most of the time.
- **Fix rounds:** start a fresh coder with a tight brief (the diff, the findings, the claim) instead of resuming a long-running coder whose context keeps growing.
- **Tests:**
  - The coder writes physics, behaviour and race tests.
  - The test-writer role is optional. Use it only when the coder's tests would share the code's blind spots, for example when the coder also designed the expected values.
- **Records:**
  - The Housekeeper (Haiku) writes `PROGRESS.md`, the `LAB_LOG.md` entry and the evidence file, from facts the orchestrator gives it.
  - The orchestrator checks them before committing.

## Milestone close: profiling pass

Run at every milestone close, before the stage audit. Lazlo, 2026-11-20: optimize performance early, before things get complex. Every command is run from the main checkout root through `build/bin/broadsword.exe` and was checked against `projects/crash-sim/examples` (`bench.rs`, `profile_step.rs`) and `projects/render/examples` (`drive.rs`) on 2026-11-20.

1. **Quiet start.** Run `build/bin/broadsword.exe status` and write down the `cpu:` line before each run below. Run only when it shows 30% or less; if it shows more, run anyway and write "loaded, cpu N%" next to that result. Done: every result in the record has its CPU % or "loaded".
2. **Phase profile.** Run `build/bin/broadsword.exe run crash-sim -- cargo run --offline --release --example profile_step`. It prints one `run N:` line per run (each must end `final state identical`) and a table of 8 phases with `share`. Done: the table is copied into the record and the run exited 0 (a non-zero exit, or a `run` line without `final state identical`, is a failure to fix, not a number).
3. **Bench.** Run `build/bin/broadsword.exe run crash-sim -- cargo run --offline --release --example bench`. It prints one JSON line per case (`box`, `cube`, `settle`, `car`) with `median_ms`. Done: all four lines are copied into the record, and `car` is among them (to run one case: add `-- --case car` after the example name).
4. **Compare with the last close.** Open the previous close's record and compare each phase `share` from item 2 and each `median_ms` from item 3. The record is a new file per close, `evidence/perf-close-<milestone>.md` (for example `evidence/perf-close-stage-6.md`); the previous one is the newest other `evidence/perf-close-*.md`. Done: the new file lists, for every phase and case, the old number, the new number and the change in %. If no earlier record exists, the file says "first record" and the numbers become the baseline.
5. **Slowdowns get a task.** Every `median_ms` that is more than 10% above the previous close, measured on a quiet run (item 1), gets a board task (`build/bin/broadsword.exe task add crash-sim --title <text> --claim <path>`) whose title or brief holds both numbers. A slowdown measured on a loaded run is repeated once on a quiet machine first. Done: the record lists each slowdown with its task id, or says "no slowdown over 10%".
6. **Viewer.** Build the sim, then run the viewer's scripted drive without a window:
   - `build/bin/broadsword.exe run crash-sim -- cargo build --offline --release --bin crash-sim`
   - `build/bin/broadsword.exe run render -- cargo run --offline --release --example drive -- --sim-exe ../../build/release/crash-sim.exe --body ../crash-sim/scenarios/car_basic.bsn --script 30 --no-window`

   Done: the record holds the `summary:` line (`average fps`) and the `timing: per drawn frame` line, and states that 16.7 ms (60 fps) was or was not met. `drive` does not print a p99 frame time today (it prints the mean draw time and the p90 of the child round trip), so write "p99 frame time: not measured" until `drive` prints it; never put a p99 estimated from the average in the record.
