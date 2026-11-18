---
name: self-optimize-process
description: "Lazlo's standing OK to change or drop any Broadsword process rule (his included) that measurably hurts throughput or cost, with evidence and a ledger row; safety rules excluded"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 4965c608-c73d-49ed-998f-9808d99fb3d0
  modified: 2026-11-18T13:54:43.152Z
---

Lazlo said on 2026-11-18: "we should let you automate your own optimization process. if i had put a rule in place and it doesn't hold up to being productive and helping us get more throughput and cost effective ness you should not be held back from just changing it so we become a well oiled production machine."

**Why:** he wants a production machine that improves itself, not a process frozen by his earlier guesses. Asking him about every process tweak is itself a throughput cost.

**How to apply:**
- Change, replace or drop process rules without asking when the numbers show they cost more than they give: review tiers and rounds, brief formats, check scope, agent counts, records cadence. This includes rules Lazlo wrote himself.
- Every change needs measured evidence, a ledger row in `RETRO.md`, a way to revert it (and a revert if the next retro shows harm), and a line in the session report. The rule is written into `AGENTS.md` under "Self-optimization".
- Never covered:
  - Ask first: money, credentials, network services, publishing and deletion. Paid fast tiers fall here too ([[fast-modes-need-price-check]]).
  - Secrets.
  - Never deleting work or history.
  - Done means verified.
  - Having an independent review before a merge. Its form may change.
- Don't game it: the orchestrator is the most tempted to skip process (playbook lesson 27), so the evidence must be real numbers, not convenience.

Related: [[retro-after-each-milestone]], [[adjudicate-shadow-findings]], [[model-budget-routing]].
