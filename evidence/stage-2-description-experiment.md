# Stage 2 lesson focus: does description wording change tool choice?

**Run:** 2026-11-17. The plan's lesson focus for Stage 2: "how well an AI understands tools from their descriptions alone. Rewrite descriptions and measure the difference."

**Result:** no difference in tool choice. Both wordings scored 8 of 8, so the test hit its ceiling. The variant removed Claude's extra preamble calls, but 4 runs per model per condition is too few to call that real. The variant was not adopted. It is kept on branch `exp/check-routing-first` at `06ff168`.

## Setup

- **Prompt.** The same for every run, with no "in one job" hint: "Use the broadsword tools to build and test the project named crash-sim, then report the result."
- **Models:**
  - `gpt-4.1-mini` through `clients/openai_mcp.py`;
  - `claude-opus-5-5` through headless Claude Code 2.1.325.
- **Allowed tools:** `status`, `projects`, `build`, `test`, `check`, `jobs` and `tools`, the same in both conditions.
- **Baseline:** the shipped descriptions. In the 1,200-character `build` description, the advice "use `check` to build and test in one go" is the last sentence. The same goes for `test`.
- **Variant:** one routing sentence moved to the front of each description. Nothing else changed.
  - `check`: "To build AND test a project, call this tool once; do not call `build` then `test`."
  - `build`: "Compiles only. To build and test, call `check` instead (one call)."
  - `test`: "Tests only. To build and test, call `check` instead (one call)."
- **Runs:** 4 per model per condition, 16 in all. They were driven by `scratch/exp/run.sh` and counted by `scratch/exp/tally.py`, both in `scratch/`, which may be wiped.

## Results

| | Baseline | Variant |
| --- | --- | --- |
| OpenAI: used `check` only | 4/4 | 4/4 |
| OpenAI: tool calls per run | 1, 1, 1, 1 | 1, 1, 1, 1 |
| OpenAI: total tokens per run | 3728, 3727, 3725, 3740 | 3843, 3841, 3843, 3840 |
| Claude: used `check` only | 4/4 | 4/4 |
| Claude: calls per run | 3, 1, 1, 3 | 1, 1, 1, 1 |
| Claude: extra `projects` + `status` before `check` | 2/4 | 0/4 |
| Claude: cost per run (USD) | 0.077, 0.035, 0.034, 0.044 | 0.127, 0.035, 0.036, 0.036 |
| Reported all 11 tests passing | 8/8 | 8/8 |

Notes on the numbers:

- The variant's descriptions cost OpenAI about 115 more prompt tokens per run (+3%), because they are longer.
- The first Claude run in each condition cost the most ($0.077 and $0.127). That is most likely a cold prompt cache, not the wording. Excluding run 1, the means are $0.038 for the baseline and $0.035 for the variant.

## What it shows

- **Tool choice:** both models already pick `check` without a hint. The build-then-test split seen in milestone 2's review did not happen once in 16 runs. That earlier run used a different prompt and a different allowed-tool set.
- **Preamble calls:** moving the routing sentence to the front may stop Claude's look-around calls (2/4 down to 0/4). At n=4 that's suggestive, not shown. A real answer needs about 20 or more runs per condition, and prompts that make the choice harder, for example "compile crash-sim, then run its tests".
- **Overall:** no change was justified by this evidence, so none was merged.

## Cost

- Claude runs: about $0.43 in total.
- OpenAI runs: about 30,300 tokens in total, a fraction of a cent at `gpt-4.1-mini` rates.
- Both were within the closeout run that Lazlo approved.
