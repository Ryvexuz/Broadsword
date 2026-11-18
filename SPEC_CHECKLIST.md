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

## Process

- **Review tier:**
  - High-risk changes get Opus as reviewer. High risk means deleting or rewriting files, locks, secrets, or anything that runs before or instead of a safety check.
  - Lower-risk changes get a cheaper reviewer from a different model family than the coder.
  - Record which tier was used and whether it caught anything. `python -I scripts/review_costs.py` totals cost, passes and fails by model for the retro.
- **Work ahead** (AGENTS.md rule): briefs for ready tasks live in `state/backlog/<task id>.md` (not in Git: a file written into a main checkout during a queue run blocks the merge), and the task is added to the board as `open`. Keep the next task's brief written and its claims checked, and start a coder on it while the queue checks and reviews the current one, provided its claims overlap nothing running or queued. Watch for ahead work that depends on an unmerged change. [S3 audit fixes: finished work waited up to 9 min for the queue; the pipeline was empty 62% of the time.]
- **Shadow-only findings** (Lazlo, 2026-11-18): after each queue run, every BLOCKING line only the shadow reviewer raised gets a verdict from an independent strong judge (neither the coder nor the gating reviewer), with evidence: real, false, dup or unsure. A real one on main becomes a fix task. *Not yet in force:* `shadow_report.py --open` (task `t-20261118-135124`) will make open ones fail the session-end check; until then the orchestrator reads `shadow_report.py` by hand.
- **Fix rounds:** start a fresh coder with a tight brief (the diff, the findings, the claim) instead of resuming a long-running coder whose context keeps growing.
- **Tests:**
  - The coder writes physics, behaviour and race tests.
  - The test-writer role is optional. Use it only when the coder's tests would share the code's blind spots, for example when the coder also designed the expected values.
- **Records:**
  - The Housekeeper (Haiku) writes `PROGRESS.md`, the `LAB_LOG.md` entry and the evidence file, from facts the orchestrator gives it.
  - The orchestrator checks them before committing.
