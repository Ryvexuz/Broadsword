---
name: cost-to-completion
description: Compare models by total cost to finish a job (all attempts, fix rounds and reviews until merged or passing), not cost per call
metadata:
  type: feedback
---

Lazlo said on 2026-11-18: "track also how much it costs for a model to complete a job not just how much it costs per task. so if a model has to do it several times to get it right but costs more than one that does it once every time."

**Why:** a cheap model that needs three rounds can cost more, in money, Claude tokens and wall time, than a pricier one that gets it right first time. Per-call prices hide that.

**How to apply:**
- In every model comparison (benchmarks, routing decisions, retros), report cost to completion: the sum over all coder runs, fix rounds, reviews and checks until done, plus rounds and time to merge.
- In benchmarks, report cost-to-pass with a retry budget alongside pass@1.
- Codex (ChatGPT plan) calls count as "plan" usage with their tokens, never as free.
- Board task `t-20261118-141428-2` builds this.

Related: [[model-budget-routing]], [[benchmark-runs-approved]], [[self-optimize-process]].
