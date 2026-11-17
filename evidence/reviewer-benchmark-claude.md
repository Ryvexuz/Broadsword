# Reviewer benchmark: Claude models

**Run:** 2026-11-17. Lazlo approved it as a way to choose reviewer models from data before Stage 3 milestone 3, the merge queue. This is the Claude half. The OpenAI half, including the model Lazlo called "Terra 6.1", runs the same cases once his models list is in.

**Result:** Opus caught the most real bugs (4/5), Haiku 3/5 at about 1/30 of the cost, and Sonnet 2/5. High effort changed no catches. No model flagged the silent-NaN bug as blocking from the diff alone. All six noticed it but rated it minor.

## Method

- **Cases.** There are five first-pass commits whose real blocking bug is known, because our live reviewer failed them. That gives ground truth for scoring whether a model catches the bug, not just whether its review sounds good.
- **Setup.** One fixed diff-only review per case, model and effort level:
  - headless Claude Code 2.1.325 with `--model` and `--effort`;
  - a replaced system prompt;
  - no tools and no MCP;
  - run from a folder outside the project, so no `CLAUDE.md`, `AGENTS.md` or project memory loads.
- **Prompt.** A neutral task brief with the done check, then the source diff. The answer has a fixed format: `VERDICT`, `BLOCKING:` lines and `MINOR:` lines.
- **Scoring.** A per-case pattern (`bench/reviewer/cases.json`) matched against the BLOCKING lines, followed by a hand check of every unmatched blocking line and every miss.
- **Files.** `bench/reviewer/`: `make_cases.sh`, `cases.json`, `run.py`, `score.py`.
- **Size.** 30 runs: 5 cases × Haiku, Sonnet and Opus × default and high effort. Total cost about $2.70.

**Known artifact.** The diffs include source files only. Every model therefore flagged "a new required setting is not added to `broadsword.toml`" on `36bca91` and `cadbb6d`. The real commits did add it, so these findings are excluded from scoring and are not counted as false positives.

## Results

✔ = flagged as BLOCKING; ◐ = noticed, but rated MINOR; ✘ = missed.

| Case | Real blocking bug | Haiku | Haiku high | Sonnet | Sonnet high | Opus | Opus high |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `a19ee1a` | unstable run prints NaN, exits 0 | ◐ | ◐ | ◐ | ◐ | ◐ | ◐ |
| `fd98728` | MCP child inherits `OPENAI_API_KEY` | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `3ad7320` | stale-lock takeover lets two hold the lock | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| `cadbb6d` | ignored `.env` deleted with the worktree | ✘ | ✘ | ✘ | ◐ | ✔ | ✔ |
| `36bca91` | `run` bypasses the build guard, so a worktree binary gets installed | ✔ | ✔ | ✘ | ✘ | ✔ | ✔ |
| **Caught as blocking** | | **3/5** | **3/5** | **2/5** | **2/5** | **4/5** | **4/5** |
| Cost for 5 runs (USD) | | 0.028 | 0.036 | 0.499 | 0.316 | 0.908 | 0.868 |
| Average time per run (s) | | 20 | 60 | 15 | 38 | 32 | 48 |

## Other blocking findings, checked by hand

- **Haiku on `cadbb6d`:** both runs raised two blocking claims about a worktree folder deleted by hand with an unmerged branch. They read as false positives; the code keeps the branch in that case. This matches the live Haiku review earlier, which was one real bug and one false positive.
- **Haiku on `fd98728`:** it also flagged that `--allow run` lets a model run arbitrary commands. That's true by design, and the task brief says so.
- **Haiku high on `3ad7320`:** it flagged that the lock file can't be removed while a waiter has it open, a plausible Windows-specific problem.
- **Opus on `3ad7320`:** it also found, as a minor point, that the takeover can leave an empty lock that blocks starts for 60 s. The live review called this blocking.
- **Several models on `36bca91`:** they flagged, as minor, that `install` checks the note but not `current_exe`. That was real, and fixed in the fix round.

## Conclusions

1. **Opus** has the best catch rate and was the only model to catch the data-loss case. At about $0.18 per review, use it for high-risk changes: deletion, locks, secrets, safety guards.
2. **Haiku** is about $0.006 per review. It caught 2 of the 3 subtle bugs (the lock race and the `run` bypass) but missed the data-loss case and raises false positives. It suits a cheap first pass whose findings are checked against the code.
3. **Sonnet** was not better than Haiku here (2/5, with both subtle misses), at about 15× Haiku's cost. With n=5 this may be noise, but it isn't a reason to choose Sonnet as reviewer.
4. **High effort** changed no catches. It made Haiku about 3× slower. Default effort is enough for diff review.
5. **Reading a diff is not enough for silent-failure bugs.** All six saw the NaN path but called it minor. The live reviewer in Stage 1 failed it as blocking because it ran the program with harsh settings. The merge queue's review step should include running the done check and adversarial inputs, not only reading the diff.

## Limits

- One run per case and combination, so the table shows large differences, not fine rankings.
- The cases are bugs our Opus reviewer already found, which may favour the kinds of bugs Opus finds.
- Diff-only reviews have no tools. Live reviews with tools behave differently. They found the NaN bug, but they also cost about 100–200k tokens each.
