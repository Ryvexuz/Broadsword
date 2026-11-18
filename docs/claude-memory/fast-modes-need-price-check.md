---
name: fast-modes-need-price-check
description: "Don't turn on paid speed tiers (OpenAI service_tier fast/priority, Claude Code fast mode) for Broadsword until a price comparison is done; Lazlo wants Claude fast mode off for now"
metadata:
  node_type: memory
  type: feedback
  originSessionId: 4965c608-c73d-49ed-998f-9808d99fb3d0
  modified: 2026-11-18T13:55:49.451Z
---

On 2026-11-18, after the throughput list, Lazlo said: "we need to do a price comparison for the fast decisions and leave your fast off for the moment."

**Why:** faster tiers cost more. He wants the trade-off measured, with time saved per merge or run against the extra cost, before paying for it. This fits his "run them all" stance on benchmarks: data first, then decide ([[benchmark-runs-approved]], [[model-budget-routing]]).

**Update, 2026-11-18:** "i don't want you to be in fast mode unless i change it right now. there are other ways we can work around that pretty easily." Claude fast mode is Lazlo's switch alone. It stays off even if the price comparison favours it, and the self-optimization rule ([[self-optimize-process]]) never covers it. Get speed elsewhere: parallel coders, scoped checks, cheaper models for routine work. The comparison only informs him.

**How to apply:**
- Keep the orchestrator session's fast mode (`/fast`) off. Don't suggest turning it on until the comparison is done.
- Don't set the OpenAI `service_tier: fast` (or `priority`) for the shadow reviewer or the benchmarks in any default config.
- Benchmarking the tiers to produce the comparison is fine; that's the point.
- The comparison needs current prices per tier (from OpenAI's and Anthropic's pricing pages), set against measured speed-ups (terra's fast tier was about 1.7x on a review). Present it to Lazlo and let him decide.
