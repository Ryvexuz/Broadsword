---
name: adjudicate-shadow-findings
description: "When the shadow reviewer (GPT) flags a bug the gating reviewer (Claude) didn't, investigate it and record whether it was real; measure the shadow's precision"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 4965c608-c73d-49ed-998f-9808d99fb3d0
  modified: 2026-11-18T13:51:32.595Z
---

Lazlo said on 2026-11-18, from the airport: "when our shadow reviewer finds a possible code bug our reviewer doesn't we should look into it. especially now at the start, they may not agree on finding the same ones all the time. and we need to see how right it is."

**Why:** shadow mode only means something if disagreements are checked. Each one is either a bug the gating reviewer missed, which is a bug on main, or a shadow false alarm. Without verdicts, the shadow's value can't be measured.

**How to apply:**
- After every queue run, list the shadow-only BLOCKING lines (board task `t-20261118-135124` builds `shadow_report.py --open`; until then, read `shadow_report.py` output by hand).
- Have an independent strong model judge each one against the code, with evidence: neither the coder nor the gating reviewer, so Fable when Opus gated.
- Record real, false, dup or unsure. A real bug on main becomes a fix task.
- Report the shadow's precision and the gating reviewer's misses in the retros.

Related: [[openai-weekly-before-api]], [[benchmark-runs-approved]], [[model-budget-routing]].
