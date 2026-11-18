---
name: model-budget-routing
description: "OpenAI budget for Broadsword work can be reset whenever Lazlo wants, Claude's can't; use OpenAI where it helps, with no fixed routing rule"
metadata:
  node_type: memory
  type: user
  originSessionId: 4965c608-c73d-49ed-998f-9808d99fb3d0
  modified: 2026-11-17T23:22:44.840Z
---

Lazlo can reset his OpenAI usage "basically whenever". He doesn't seem able to do that for Claude. He also rates Claude's work quality clearly higher.

He was explicit (2026-11-17) that this is **a note, not a fixed rule**: "whatever works best for us". So OpenAI capacity is a flexible resource to draw on when it suits a task (cheap review, bulk experiment runs, and so on). Pick the best model for each job, and don't route by budget alone.

He also mentioned a cheap OpenAI model, "Terra 6.1", described as slightly worse than Sonnet at about a fifth of the price. The model id is unverified: check it against his OpenAI models list before use.

Related: [[broadsword-source-docs]]

**Direction, 2026-11-18:** "once we get the gpt ones tested, we can outsource a lot more to gpt and that way we can keep you as the orchestrator for longer as well." Once the benchmarks and shadow verdicts show which GPT models are trustworthy, move routine coding, reviewing and records work to GPT through Codex, on the ChatGPT allowance ([[openai-weekly-before-api]]). That saves the Claude plan for orchestration. Evidence comes first: the shadow verdicts ([[adjudicate-shadow-findings]]) and a GPT coder trial on a real task, reviewed by Claude.

**Lazlo's picks, 2026-11-18 (from his own use):**
- For coding: gpt-6.1-terra on "ultra" (whether that is reasoning effort or a speed tier is unconfirmed), gpt-6.1-sol at high, and Opus 5.5 at high.
- gpt-6.2-astra is good in short bursts but drifts on long tasks. Give it sections; split a full sweep into 2 or 3 sectioned calls.
