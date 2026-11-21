# Speed tiers: are they worth paying for? (prices only, no paid calls)

Task t-20261118-095351-3. Written 2026-11-21 (system clock ~04:30Z; the task sandbox date line said 2026-11-20). Scope set by the orchestrator note: official public pages only, no paid-tier API call, no account, no key. Nothing here was measured; every speed figure is a published claim and every dollars-per-minute figure is an ESTIMATE.

## 1. Prices read from official pages

All read 2026-11-21 (via fetch tool, which summarises the page; the verbatim rows below are the fetch's quotes, not re-checked in a browser). USD per 1M tokens, short context (<= 272K input).

### OpenAI

Source: https://developers.openai.com/api/docs/pricing (https://platform.openai.com/docs/pricing redirects there, 301). The page calls the priority tier "Fast": "Priority processing was renamed Fast mode on July 30, 2026." The API parameter is still `service_tier` (our `tier_probe.py` sends `priority`); that spelling was not re-verified on the page.

| Model | Tier | Input | Cached input | Output |
|---|---|---|---|---|
| gpt-5.6-sol | Standard | $4.00 | $0.40 | $20.00 |
| gpt-5.6-sol | Fast (priority) | $8.00 | $0.80 | $40.00 |
| gpt-5.6-sol | Flex | $2.00 | $0.20 | $10.00 |
| gpt-6.1-terra | Standard | $1.00 | $0.05 | $5.00 |
| gpt-6.1-terra | Fast (priority) | $2.00 | $0.10 | $10.00 |
| gpt-6.1-terra | Flex | $0.50 | $0.025 | $2.50 |
| gpt-6.2-sol | Standard | $2.00 | $0.10 | $10.00 |
| gpt-6.2-sol | Fast (priority) | $4.00 | $0.20 | $20.00 |
| gpt-6.2-sol | Flex | not listed | not listed | not listed |
| gpt-6.1-sol (used tonight, extra row) | Standard | $2.00 | $0.10 | $10.00 |
| gpt-6.1-sol | Fast (priority) | $4.00 | $0.20 | $20.00 |
| gpt-6.1-sol | Flex | $1.00 | $0.05 | $5.00 |

Verbatim source rows quoted by the fetch: `| gpt-6.2-sol | $2.00 | $10.00 | $0.10 | ...` (Standard), `| gpt-6.1-terra | $1.00 | $5.00 | $0.05 | ...` (Standard), `| gpt-5.6-sol | $4.00 | $20.00 | $0.40 | ...` (Standard), `| gpt-5.6-sol | $8.00 | $40.00 | $0.80 | ...` (Fast).

Notes from the page:
- No multiplier is stated; the listed Fast rates are exactly 2x Standard and Flex exactly 0.5x Standard (my arithmetic on the table).
- Flex: "Flex remains available on the models listed here as we wind down the beta. We won't be adding Flex to new models." So gpt-6.2-sol has no Flex, and Flex will not come to newer models.
- Long-context (> 272K) rates are higher (for gpt-5.6-sol Fast: $16 input / $60 output per the priority guide). Our review prompts are about 31K input, so short-context rates apply.

Source for the Fast speed claim and the premium: https://developers.openai.com/api/docs/guides/priority-processing (read 2026-11-21). Quotes: Fast mode delivers "up to 2.5× faster speeds and more consistent latency"; for gpt-5.6-sol it is "up to 2.5× faster than Standard processing"; it "charges a per-token premium over Standard processing" and cached-input discounts still apply. For GPT-6 Astra, Fast mode has no latency SLA. The page gives no tokens-per-second figure. It also says GPT-5.6 Sol promotional pricing runs "at least through November 21, 2026", so the gpt-5.6-sol rates above may change after tonight.

The ChatGPT pricing page https://openai.com/api/pricing/ shows only seat prices, no per-token API rates; not used.

### Claude Code fast mode

Sources: https://code.claude.com/docs/en/fast-mode and https://platform.claude.com/docs/en/about-claude/pricing (both read 2026-11-21).

| Model | Standard in / out | Fast mode in / out | Ratio |
|---|---|---|---|
| Claude Fable 5.2 | $8 / $40 | $16 / $80 | 2x |
| Claude Opus 5.5 (fast-mode default in Claude Code >= v2.1.280) | $4 / $20 | $8 / $40 | 2x |
| Claude Opus 5 / Opus 4.8 | $5 / $25 | $10 / $50 | 2x |

Quotes: "making the model up to 2.5x faster at a higher cost per token"; "Anthropic's fast mode pricing per MTok input/output: $16/$80 on Fable 5.2, $8/$40 on Opus 5.5, and $10/$50 on Opus 5 and Opus 4.8." Pricing page: "up to 2.5x higher output tokens per second at premium pricing" and "Fast mode is not available with the Batch API."

Conditions that matter for us:
- On Pro/Max/Ultra/Team/Enterprise subscriptions, fast mode is "available via usage credits only and not included in the subscription rate limits", and "draws directly from usage credits, even if you have remaining usage on your plan". It needs usage credits switched on. So it is real extra money even if the plan has headroom.
- Turning it on mid-conversation re-bills the whole context uncached at the fast rate; separate caches for fast and standard. Enable at session start or not at all.
- Opus 4.8 fast mode is removed on 2026-11-30.
- Which model the orchestrator runs is not stated in the brief; AGENTS.md says Opus. If it is Fable 5.2, use the Fable row.

## 2. Tonight's volumes (from our own logs)

`state/gpt-spend/2026-11-21.tsv` (read 04:14Z): 22 gate calls (gpt-6.1-sol, 689,698 input / 14,788 output tokens total) and 44 shadow calls (636,208 / 22,725), plus 1 gpt-6.2-sol call. The same file shows no gpt-5.6-sol or gpt-6.1-terra calls tonight; the gate model in use is gpt-6.1-sol. The brief's volumes: 40 reviews, 24 merges. I could not run `broadsword queue list` (command not on PATH in this shell), so the queue count was not re-checked.

Average gate call tonight: about 31,350 input and 672 output tokens (no cached split is logged, so no caching discount is assumed; real cost is somewhat lower).

Caveat: the `est_usd` column in that file (tonight 44.28 for the 66 calls) is about 15x what list prices give for the same tokens (about $3.03), and it is labelled `codex-plan`. I do not know its basis, so I did not use it. All dollars below are list-price arithmetic from tokens.

## 3. Cost per minute saved (ESTIMATE)

There is no timing data in the logs, so the baseline wall time per gate call is an assumption: T = 60 s (ESTIMATE, unmeasured). Speed-up s = 2.5x (published ceiling: "up to"), so best-case saving = 60 x (1 - 1/2.5) = 36 s = 0.6 min per call. At a more realistic 2x the saving is 30 s. Extra cost of Fast = the full Standard cost (Fast is 2x).

Standard cost of one average gate call (31,350 in, 672 out, no caching):

| Model | Standard $/call | Extra $/call on Fast | Extra $ per minute saved (T=60 s, 2.5x) | at 2x |
|---|---|---|---|---|
| gpt-6.1-terra | 0.0347 | 0.0347 | 0.058 | 0.069 |
| gpt-6.1-sol / gpt-6.2-sol | 0.0694 | 0.0694 | 0.116 | 0.139 |
| gpt-5.6-sol | 0.1388 | 0.1388 | 0.231 | 0.278 |

General formula: extra $/min = standard $/call / (T x (1 - 1/s) / 60). If a real call takes 120 s, the cost per minute halves; if 20 s, it triples.

Tonight at gpt-6.1-sol, Fast on gate reviews only (shadow reviews run beside the gate, not on the critical path, so Fast would buy nothing there):
- Per 40 reviews: +$2.78, saves up to 24 min if reviews were serial (ESTIMATE).
- Per merge (40/24 = 1.67 reviews per merge): +$0.116, saves up to 1.0 min.
- If Fast were also applied to all 66 gate+shadow calls: +$3.03 total; the shadow part (636,208 x $2 + 22,725 x $10 per 1M = $1.50) buys no merge time.
- Flex on shadow calls would save about half of their cost (about $0.75 tonight at list price) but Flex has no published speed and is slower by design; gpt-6.2-sol has no Flex.
- Benchmark runs: cost scales with call count; a run of N calls of this size costs N x $0.0694 extra on Fast. Benchmarks are batch work, not waiting for a human, so a time saving has little value there.

Claude Code fast mode (ESTIMATE): a turn with 20K uncached input and 2K output on Opus 5.5 costs $0.12 Standard, $0.24 Fast, extra $0.12; if that turn takes 60 s and Fast saves 36 s, that is about $0.20 per minute saved. On Fable 5.2 it is twice that. For an orchestrator that is the bottleneck, one minute of orchestrator time is worth more than a gate-review minute, but the usage-credit billing makes it the most expensive per minute of everything above.

## 4. Measured: not yet (needs Lazlo's OK to spend)

No tier was measured. Not done: 3 runs per tier per model with `state/bench-results/tier_probe.py` on a queue review prompt, a hard-case review and one coding task; per-benchmark-run cost from measured tokens; confirmation that `service_tier: priority` is accepted for gpt-6.2-sol and gpt-6.1-terra and that the response reports `priority`. The 2.5x is OpenAI's and Anthropic's "up to" claim; real gains on a 31K-token input with a short output could be much smaller, because Fast speeds up generation, and our reviews are input-heavy with about 670 output tokens. That is the main reason to measure before paying.

## 5. Recommendation (for Lazlo to decide; nothing enabled)

1. Do not turn on Claude Code fast mode for the orchestrator now. It bills usage credits at 2x on top of the plan, at roughly $0.20 per minute saved (estimate), and the saving is unmeasured.
2. Do not pay for OpenAI Fast on shadow reviews or benchmark runs: they are off the critical path.
3. OpenAI Fast on the gate review is the only plausible candidate: about +$0.07 per review and +$0.12 per merge on gpt-6.1-sol, about $2.8 per 40 reviews. If Lazlo wants evidence first, approve one small measurement: `tier_probe.py gpt-6.1-sol default priority`, which does 2 reps x 2 tiers = 4 calls at about $0.04 to $0.08 each (about $0.2 total at list price, estimate). Then decide on measured seconds, not the "up to 2.5x" claim.
4. Flex is a cheaper option only for non-urgent work (batch, benchmarks), is being wound down ("We won't be adding Flex to new models") and does not exist for gpt-6.2-sol; do not build on it.
5. gpt-5.6-sol is the dearest per minute, and its promotional pricing may end after 2026-11-21; re-read the page before relying on it.

## 6. Could not find

- Any published tokens-per-second or seconds figure for Fast on gpt-6.1-terra, gpt-6.1-sol or gpt-6.2-sol (only "up to 2.5x", stated for gpt-5.6-sol).
- Any published Flex speed or latency.
- A Flex price for gpt-6.2-sol (not listed).
- A basis for the `est_usd` column in `state/gpt-spend/*.tsv` (about 15x list price).
- Our real per-call wall times (not in the spend log); queue count (`broadsword` not on PATH here).
- The orchestrator's actual model (assumed Opus 5.5; Fable 5.2 given as the alternative).
- Page text was read through a summarising fetch tool, so table cells are as that tool returned them; a human spot-check of the OpenAI pricing page is advisable before money is spent.
