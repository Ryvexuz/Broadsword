# Stage 3, milestone 4: scale-up test (max_agents 3, then 4)

Date: 2026-11-18 (UTC 02:30 to 02:37 for the two rounds). Orchestrator: Opus. Coders: Sonnet, one per task. Reviewers: Haiku in the merge queue for every crash-sim task; headless Opus for the two Broadsword changes.

## Done check

> A full session at the higher cap with clean merges and the disk under control.

**Passed.**
- **Round 1, cap 3:** 3 agents ran in parallel. All 3 merged.
- **Round 2, cap 4:** 4 agents ran in parallel. All 4 merged.
- **Merges:** every one went through `broadsword queue run`: rebase, check, independent review, `merge --ff-only`, check on main, `task done`. No merge commit, no conflict, no `merged_check_failed`.
- **Afterwards:** crash-sim main is at `9f1b5fc`, the status is clean, and there are no worktrees or task branches left.
- **Disk:** free space went from 127.9 to 127.0 GB, against a 20 GB floor. The shared build folder went from 377 to 415 MB.

## Prerequisite found on the way: the cap was not enforced

`max_agents` was in `broadsword.toml`, but no code read it. Raising the setting would have tested nothing.

**Fix, `16c9279`:**
- `max_agents` is now a required positive setting.
- `task start` refuses with exit 3 when `max_agents` tasks are already `running`, across all projects. A refused start changes nothing.
- The cap is checked under the board's OS file lock, in the same step as the claim-overlap check.

**Tests:**
- A start at the cap is refused.
- A freed slot can be taken.
- Lowering the cap leaves running tasks alone.
- An 8-process gated race against a cap of 3 runs for 3 rounds. Exactly 3 win each round, and the losers exit 3.

**Review:** headless Opus PASS, no blocking findings, $0.19. Its documentation minors were applied.

**Live proof:** at each cap, one extra probe task was refused on the real board:
```
broadsword: cannot start t-20261118-022949-4: 3 tasks are running, the max_agents cap in broadsword.toml is 3: t-20261118-022949 (crash-sim), t-20261118-022949-2 (crash-sim), t-20261118-022949-3 (crash-sim)
probe exit=3
broadsword: cannot start t-20261118-023322-5: 4 tasks are running, the max_agents cap in broadsword.toml is 4: ...
probe exit=3
```
The cap was raised to 4 in `4349051`. Both probe tasks were then abandoned (`task abandon`), so they hold no slots or claims.

"Coder jobs" in the table below counts Broadsword jobs (test and run) per coder. There was one coder agent per task, so 7 coders in all.

## The tasks

Every task created new files only, with no claim overlaps. They used crash-sim's public API.

| Round | Task | Files | Coder jobs | Queue result | Merged |
| --- | --- | --- | --- | --- | --- |
| cap 3 | drop-height sweep | examples/sweep.rs, tests/sweep.rs | 2 | PASS first time | `eafddc4` |
| cap 3 | physical invariants (16 configurations) | tests/invariants.rs | 4 | PASS first time | `17d8412` |
| cap 3 | energy report | examples/energy.rs, tests/energy.rs | 4 | FAIL (brief ambiguity), then PASS | `ff149c3` |
| cap 4 | CSV format | tests/csv.rs | 1 | PASS first time | `8099d4b` |
| cap 4 | determinism and run length | tests/determinism.rs | 1 | PASS first time | `4a81ea5` |
| cap 4 | per-node trajectory example | examples/trajectory.rs | 6 (4 refused) | PASS first time | `e593d90` |
| cap 4 | physics model document | docs/PHYSICS.md | 1 (refused) | PASS first time | `9f1b5fc` |

The crash-sim test count went from 13 to 31 tests (physics 13, plus sweep 2, invariants 1, energy 3, csv 4, determinism 8).

## Measurements

| | cap 3 | cap 4 |
| --- | --- | --- |
| Wall clock, first start to last merge | 2 min 27 s | 3 min 20 s |
| Coder time (each) | 29 to 44 s | 24 to 38 s |
| Coder tokens (each) | about 74k | 70k to 75k |
| Claim collisions (board refusals between agents) | 0 | 0 |
| Rebase conflicts | 0 | 0 |
| Build-lock waits reported | 0 | 0 |
| Agents blocked by another agent | 0 | 2 of 4, plus one queue run |
| Queue rejections | 1 (false positive) | 0 |
| Review cost (Haiku) | about $0.002 each | about $0.002 each |

**Total cost:**
- Coders: 7 × about 72k Sonnet tokens.
- Reviews: 8 Haiku reviews at about $0.002 each.
- Broadsword fixes: two headless Opus reviews, about $0.19 each.
- Orchestration: the rest of the cost was the orchestrator.

## Wasted work and its causes

1. **One agent's mistake blocked every agent on the project.** This was the real collision.
   - **What happened:**
     - The trajectory coder ran `broadsword run crash-sim --task … -- cargo run --example trajectory -- 50 | head -20`.
     - `head` closed the pipe. Rust's `println!` panics on a closed stdout, so the Broadsword supervisor died in the middle of the job.
     - The job's durable record stayed `running`, and Broadsword refuses every new job on a project with an interrupted job until `recover`.
   - **Who it blocked:**
     - The same coder, 4 times.
     - The physics-doc coder: its only test run was refused.
     - The queue's check for the CSV task: blocked, exit 3, nothing changed.
     - Both affected coders correctly stopped and reported. Neither ran `recover` on another agent's job.
   - **How I handled it:**
     - I confirmed the worktree was clean at the agent's commit `b7079d1`, then ran `recover --abandon`, which changes nothing.
     - I re-ran the blocked checks myself, without pipes:
       - stride 0 and `abc` each exit 2 with a message;
       - stride 50 prints 88 rows (8 nodes × 11 steps) plus the header;
       - the doc task's tests passed.
   - **Root-cause fix, in one place:**
     - `main.rs` now shadows `println!`, `print!`, `eprintln!` and `eprint!` for the whole crate, with versions that drop write errors instead of panicking.
     - A new test runs a job whose stdout reader closes early. It checks that the record ends `done` and that the next job is not refused.
     - The test was verified to FAIL without the fix and to pass with it. The suite has 346 tests, all passing.
     - Reproduced before the fix: `broadsword run crash-sim -- cargo run --example trajectory | head -2` panicked with `failed printing to stdout: The pipe is being closed. (os error 232)` and left the job `running`.
2. **A false rejection caused by my brief.**
   - The energy brief said "use only crash_sim's public API (Sim::new, advance, pos, vel, beams, cfg, the constants)".
   - Haiku read the list as complete, and FAILED the task for calling `max_speed()`, which is public.
   - The code was right. The brief was ambiguous.
   - I rewrote the sentence in every brief to say "anything `pub` in src/lib.rs (for example …)" and re-queued the same commit. It passed.
   - Cost: one extra review, about $0.002, plus orchestrator time.
   - **Lesson:** a parenthesised list in a brief reads as exhaustive to a reviewer.
3. **Normal iteration, not waste:**
   - The energy coder's first free-fall test failed. A node can bounce inside the substeps of one output step, so the coder fixed the stop condition.
   - The invariants coder found the 4 m, tilted, Moon-gravity drop still rocking at 8 s, and ran 20 s instead. It kept every threshold as briefed.

## What did not show up at this scale

- **The shared build folder did not thrash.** The build-source notes run `cargo clean -p crash-sim` whenever a different worktree builds, but crash-sim builds in about a second, so it cost nothing noticeable. A bigger project would show it.
- **No agent waited on the build lock.** That is because each coder ran only 1 to 4 short jobs.

## Not done or not verified

- **The tasks were designed not to overlap,** so the board's claim refusals between agents were not exercised in this session. Milestone 1's race tests cover them.
- **The doc coder flagged a comment in `src/lib.rs`, lines 12 to 13.** The comment says `sqrt(10 k / m)`, which is about 447, while the real top mode is 400 = `sqrt(8 k / m)`. The comment's "omega·dt about 0.40" is consistent with 400. No task claimed lib.rs, so this went to IDEAS.md rather than being fixed.
- **Refs keep accumulating.** Crash-sim now holds 142 `refs/broadsword/*` refs (checkpoints, queue pre-rebase tips). Pruning is already in IDEAS.md.

## Addendum: Stage 3 stage-level check, two reviewer models

AGENTS.md's Stage 3 done check: "two agents build two separate crash-sim features in parallel. Each is reviewed by a different model, both merge cleanly, and the reviewer catches at least one planted bug." The rounds above used Haiku for every review, so this run covers the stricter reading: two different reviewer models.

| Feature | Claims | Coder | Queue risk | Reviewer (from the CLI's modelUsage) | Cost | Verdict | Merged |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Damping stability limit in `Config::validate`, plus the `sqrt(8 k / m)` comment fix | src/lib.rs, tests/stability.rs | Sonnet, 1 job | low | `claude-haiku-5-5` | $0.0026 | PASS | `0e6805c` |
| `--summary` option | src/main.rs, README.md, tests/summary.rs | Sonnet, 2 jobs | high | `claude-opus-5-5` | $0.0875 | PASS | `9c016cc` (rebased onto `0e6805c` by the queue) |

- **How they ran:** both tasks started on the board together, at 03:29 UTC, each in its own worktree, with the coders running at the same time. Both merged with `--ff-only` after check and review, and the check on main passed: crash-sim has 40 tests, 0 failed. Main is `9c016cc`, with no worktrees left.
- **The planted bug:** caught in milestone 3. Haiku rejected `7ab23fc`, the `--gravity` flag that was parsed but never used. See `evidence/stage-3-m3-merge-queue.md`.
- **Reviewer minors:**
  - Haiku said a NaN step could pass the damping check. This doesn't apply: `validate` rejects a non-finite `dt` before that line, and `substeps` is an integer.
  - Opus noted that `--summary` prints nothing when a run aborts after starting, while CSV mode prints the partial rows. This is the coder's documented choice. Opus also noted that the test compares against values rounded from the CSV. Both are recorded in IDEAS.md.
- **Damping limit:** the coder found that no configuration breaks only the damping limit (h < 0.05 s) with the current constants, because the stiffness limit (h <= about 0.0038 s) always fails first. The damping functions are tested directly.

Stage 3's stage-level done check is met.
