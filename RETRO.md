# Retrospectives

After each milestone: what went wrong, why, and what changes, then the same for speed and cost. This is the rule in `AGENTS.md`.

A change counts only once it is enforced somewhere: Broadsword's code, a role file in `.claude/agents/`, or `SPEC_CHECKLIST.md`. Each retro starts by checking whether the previous retro's changes worked. Once retros keep coming back empty, the rule can be dropped.

Newest first.

---

## Stage 3, milestones 3 and 4, and close-out (2026-11-18)

### Did the last changes work?

These were the first retros, so there were no earlier ones to check. The Stage 1 and 2 lessons at the top of `LAB_LOG.md` led to `SPEC_CHECKLIST.md` and the two-review-pass rule, and both held:

- Milestone 3 failed its first Opus pass with 3 blockers. All three were fixed in one round.
- Every crash-sim task in milestone 4 passed review on the first try, except one. That one failed because of the brief, not the code.

### What went wrong

| What | Why | What changes | Where it's enforced |
| --- | --- | --- | --- |
| `max_agents` was never enforced. The scale-up test would have tested nothing. | The setting was added in Stage 0 as a note. Nothing checked that a setting in `broadsword.toml` is actually read. | `task start` now enforces the cap. Every setting must be used by code. | Code (`16c9279`). This retro adds the rule below to the spec checklist. |
| One agent's `\| head` blocked every agent on crash-sim. | `println!` panics on a closed pipe. The dead job stayed "running", and an interrupted job blocks the whole project. | Broadsword can no longer be killed by a closed output. Coders never pipe Broadsword. | Code (`b578103`) and `coder.md`. Narrowing the block to a single worktree is in IDEAS. |
| Haiku rejected correct code. | My brief listed some public functions in parentheses, and the reviewer read the list as complete. | A list in a brief is either marked "for example" or is the full rule. | `SPEC_CHECKLIST.md` |
| My first broken-pipe test failed on its own setup. | I assumed the first output line was "job:". Broadsword prints its recovery line first. | Tests read until they find what they're looking for, not a fixed line number. | Habit only. Small; watch for it next retro. |
| The housekeeper wrote that the Stage 3 check had passed. One reading of it hadn't. | The check's wording, "each reviewed by a different model", has two readings, and the draft took the easier one. | The orchestrator checks every "done" claim in the records against the done check's exact words before committing. | Already the orchestrator's job; it caught this one. |

### Speed

- **The coders weren't the slow part.** Each took 24 to 44 seconds. A whole round of 3 or 4 agents, from first start to last merge, took 2.5 to 3.5 minutes.
- **The time went to orchestration:**
  - writing briefs;
  - the Broadsword fixes, each needing a full test run (about 3 minutes), a marker run and a review;
  - and one Opus review that ran past the 10-minute foreground limit.
- **Change:** run reviews and full test suites in the background from the start, and keep working while they run.

### Cost

- **Small, exact briefs cut the cost per coder pass by 3 to 5 times.**
  - Earlier in Stage 3, coders spent 250k to 400k tokens per pass on open-ended Broadsword work.
  - In milestone 4, each coder spent about 72k tokens on one new file, with a brief that named the claim, the API and the done check.
- **Review cost is negligible next to coder cost:**
  - Haiku: about $0.002 per review.
  - Opus: about $0.09 to $0.21.
  - The 10 queue reviews together cost less than $0.20.
- **Waste:**
  - one false rejection, about $0.002;
  - 4 refused jobs, plus one blocked coder test run, from the pipe incident;
  - one marker run that I chose to do. That one was worth it: it showed the new test really catches the bug.
- **Change:** split work into tasks the size of one or two files whenever the code allows it, since that is where the savings came from. Use Haiku for low-risk reviews and keep Opus for locks, deletion, secrets and the Broadsword core.

### Carry into the next milestone

1. Every new setting has a test that fails if no code reads it. This is added to `SPEC_CHECKLIST.md`.
2. Long runs (the full suite, reviews) go in the background.
3. Tasks stay the size of one or two files wherever possible.
4. Check: no coder piped Broadsword output, and no brief got a false rejection.
