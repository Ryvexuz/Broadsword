# Retrospectives

After each milestone: what went wrong, why, and what changes, then the same for speed and cost. This is the rule in `AGENTS.md`.

A change counts only once it is enforced somewhere: Broadsword's code, a role file in `.claude/agents/`, or `SPEC_CHECKLIST.md`. Each retro starts by checking whether the previous retro's changes worked, including whether any of them hindered things (see the improvement ledger). Once retros keep coming back empty, the rule can be dropped.

## Portable lessons, for any project

These are the lessons from the retros and the stage lessons in `LAB_LOG.md`, with the Broadsword details removed so they apply to any project built with AI agents. Each retro adds to this list, or sharpens an item already on it. After any change, run `python -I scripts/sync_playbook.py`. It copies this section to `~/.claude/AI_DEV_PLAYBOOK.md`, which `~/.claude/CLAUDE.md` loads in every project. `--check` reports a stale copy.

**Planning and briefs**
1. **Small, exact tasks are the biggest cost lever.** Name the files, the API and the done check. Broadsword coder passes dropped from 250–400k tokens to about 72k this way. [S3]
2. **Name the hard cases in the spec up front:** what must never happen, the edge inputs and the security boundaries. Longer specs meant fewer review rounds. [S2]
3. **A list in a brief is either marked "for example" or is the complete rule.** Models read an unmarked list as exhaustive. [S3]
4. **Write the done check as commands with the output each must show,** in words that can only be read one way. Coder, reviewer and orchestrator then run the same check. If it can still be read two ways, check both before claiming done. [S3]
5. **Keep a brief template and a coder-prompt template.** The rules every task needs (claim, build route, what never to do, what to report) are then written once, not retyped and forgotten. [S3]

**Review**
6. **An independent reviewer on a strong model pays for itself.** Expect the first pass to fail; plan for two. Every first-pass failure was a real bug the coder's own tests had passed. [S1]
7. **Use a cheap reviewer for low-risk work and a strong one for risky code** such as locks, deletion and secrets. A cheap review costs a fraction of a cent, so reviewing everything is affordable. [S3]
8. **A reviewer working from the diff alone misses runtime bugs.** Give it the test output too. [S3]
9. **When a finding breaks a rule that must always hold, enforce the rule in one place,** not only in the reported case. [S1]

**Tests**
10. **Prove a new test can fail.** Run it once without the fix; a test that can't fail is worse than none. [S1, S3]
11. **A test seam built into the real program may only make a guard stricter,** never weaker. [S1]

**Running agents in parallel**
12. **Scaling up exposes blast radius before throughput.** First ask what one agent's failure can stop for everyone. [S3]
13. **Shared state needs real OS locks and isolated work areas:** one branch or worktree per agent, claimed files, and one owner for merging. [S3]
14. **A tool that agents call must survive their mistakes,** such as closed pipes, killed processes and bad input, without leaving shared state broken. [S3]
15. **Check that every setting is actually used.** A limit nobody reads looks like safety and isn't. [S3]

**Security**
16. **Secrets leak through side channels,** such as a child's environment or the same value under another name. Protect a secret where it enters a process, not where it is printed. [S2]

**Tools an AI uses**
17. **Test a tool surface with a real model as the client.** It follows the descriptions literally, into dead ends that protocol tests miss. [S2]

**Process**
18. **A fix counts only once something enforces it:** code, a role file or a checklist. A note alone is forgotten by the next session. [S3]
19. **Push long runs, such as full test suites and reviews, into the background,** and keep working. The orchestrator is the bottleneck, not the coders. [S3]
20. **Track whether each process change helps or hurts,** with a measure and a watch-for-harm signal, and revert the ones that hinder. A process only gets lighter if changes can be taken back out. [S3]

---

## Improvement ledger

Every process change goes here, with what it should improve and how we'll know. Each retro marks each open row:
- **helped:** keep it.
- **no effect:** keep it if it costs nothing; otherwise drop it.
- **hindered:** it made work slower, costlier, or caused false failures. Fix it or revert it, and say which.

A change can help one thing and hurt another, for example faster reviews but more false rejections. Record both. Nothing on this list is permanent.

| Since | Change | Should improve | Measured by | Watch for harm | Status |
| --- | --- | --- | --- | --- | --- |
| S1 | Independent reviewer, two passes planned per milestone | Bugs caught before merge | Real defects found per review pass | Rounds that only find style nits | helped (S1–S3) |
| S2 | `SPEC_CHECKLIST.md` risk table | First-pass review results | Review passes per milestone | Briefs so long that coders miss the point | helped (S2–S3) |
| S3 m3 | Review tier: Haiku for low risk, Opus for high | Review cost | `scripts/review_costs.py` | Bugs Haiku misses that Opus would catch; false rejections | open (1 false rejection so far, caused by a brief) |
| S3 m4 | Small one-file or two-file tasks | Coder tokens per task | Coder tokens from agent reports | Too many tiny tasks, more orchestration than coding | helped (72k vs 250–400k) |
| S3 m4 | Coders never pipe Broadsword output; never `recover` another agent's job | No project-wide blocks | Refused jobs per milestone | Coders unable to inspect long output | open |
| S3 m4 | Example lists in briefs say "for example" | No false rejections from wording | Rejections whose cause is the brief | — | open |
| S3 m4 | Every setting must be read by code, with a test | No dead settings | Review findings of unused settings | Test overhead for trivial settings | open |
| S3 | Retro after each milestone | Repeat mistakes, speed and cost | Repeats of a mistake already in a retro | Retro time growing past its value | open |
| S3 | Portable playbook loaded in every project | Lessons reach other projects | A lesson applied, or ignored, in another project | Rules that don't fit other projects causing friction there | open (not yet used outside Broadsword) |
| S3 | `templates/brief.md`, `templates/coder-prompt.md` | Time to write a brief; brief errors | Orchestrator time per brief; rejections caused by briefs | A template followed by rote, so missing task-specific cases | open |
| S3 | Done checks written as commands | One reading of "done" | Disputes or rework over what "done" meant | Checks that miss what can't be expressed as a command | open |
| S3 | `scripts/review_costs.py` | Exact cost numbers in retros | Whether retros use it | — | open |
| S3 | An interrupted run blocks only its own folder; panic guard for jobs | Blast radius of one agent's crash | Agents blocked by another agent's job | A half-changed folder that something outside it depends on | in progress |
| planned | Reviewer canary (planted bugs every few merges, across models once OpenAI is linked) | Knowing the reviewer still works, and which model is worth its price | Catch rate and cost per model | Canary cost or noise outweighing its value | planned |
| planned | Handoff test from `PROGRESS.md` alone | Session continuity | What a fresh agent gets wrong | — | planned |

## Retros, newest first

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

Read this list before the next milestone starts. The next retro checks each item.


1. Every new setting has a test that fails if no code reads it. This is added to `SPEC_CHECKLIST.md`.
2. Long runs (the full suite, reviews) go in the background.
3. Tasks stay the size of one or two files wherever possible.
4. Check: no coder piped Broadsword output, and no brief got a false rejection.
5. Briefs and coder prompts start from `templates/`, done checks are written as commands, and the cost section uses `scripts/review_costs.py`. Check that this made briefs faster to write and the retro numbers exact.
