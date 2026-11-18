---
name: benchmark-runs-approved
description: "Lazlo pre-approves paying to run every model (old and new, Claude and OpenAI) on Broadsword's reviewer benchmarks, including the hard many-bug case"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 4965c608-c73d-49ed-998f-9808d99fb3d0
  modified: 2026-11-18T07:59:40.827Z
---

Lazlo said on 2026-11-18, about the reviewer benchmark's new hard case (12 planted bugs, 3 decoys): "My answer is going to be run them all. The data might be useless but I'll pay to have it."

**Why:** he values comparison data across models more than its cost. The project's goal is learning how AI-driven development works, including model choice ([[model-budget-routing]]).

**How to apply:**
- For reviewer-benchmark runs (`bench/reviewer/`), don't ask before spending. That covers every Claude model at each effort level, including older ones the CLI still offers, and his OpenAI models once he links them.
- Run them, then report the results with the cost.
- The first real pass includes older models too, for the data points. Later passes can drop models the results clearly show aren't worth running. Lazlo: "if the data obviously shows there's no reason for the old stuff we can drop them".
- The approval covers benchmark runs only, not other paid actions.
- The case must be ready first, with its proofs passing.
