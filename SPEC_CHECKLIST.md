# Spec checklist

The orchestrator goes through this before handing any task to a coder. Each item comes from a real review failure; the failure is in brackets.

In Stage 2, naming these cases in the spec up front let milestones pass review on the first or second try. In Stage 1, leaving them out meant every milestone failed its first pass.

## Every spec states

- **Goal and done check,** in the plan's own words.
- **Claim:** the exact files and folders the coder may change. Anything else is out of bounds.
- **Starting point:** which commit the work must start from. Tell the coder to check it and fast-forward if needed.
- **Build route:** how to build and test, either `broadsword build|test|check` or the documented exception. Also how to confirm that the binary in the shared build folder is the one just built.
- **What not to touch:** real repos, `projects/*`, Claude Code settings, the network, the real `.env`.

## Risk checks: name each one that applies

| Area | What to tell the coder | From |
| --- | --- | --- |
| Deleting or overwriting anything | Save the current bytes first, verified, and check before every write or delete. Never `--force`, `-D` or a recursive delete. Ignored files count as work. | S1 m4 rollback; S3 m2 `.env` |
| One rule enforced in many places | Put the rule in one function that every path goes through, and test that function directly. | S1 m4 (five passes); S3 m1 project names |
| Locks and concurrency | Use OS file locks (`File::try_lock`), never stale-lock takeover. Keep slow work outside the lock. Add a gated race test, plus a mutation check showing the test fails without the lock. | S3 m1; S3 m2 |
| Reusing reviewed code | Re-read the reused code's known limitations against the new requirement. | S3 m1 |
| Secrets | Protect them where they enter a child process: scrub the environment by name and by value. Redact before content reaches a model or a log. Never follow redirects with credentials. | S2 m3 |
| Settings | Every limit is a required key in `broadsword.toml`. No silent defaults, no hard-coded numbers. | S1 m3; S3 m1 |
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
  - Record which tier was used and whether it caught anything.
- **Fix rounds:** start a fresh coder with a tight brief (the diff, the findings, the claim) instead of resuming a long-running coder whose context keeps growing.
- **Tests:**
  - The coder writes physics, behaviour and race tests.
  - The test-writer role is optional. Use it only when the coder's tests would share the code's blind spots, for example when the coder also designed the expected values.
- **Records:**
  - The Housekeeper (Haiku) writes `PROGRESS.md`, the `LAB_LOG.md` entry and the evidence file, from facts the orchestrator gives it.
  - The orchestrator checks them before committing.
