# Model benchmarks, 2026-11-18 (first pass, provisional)

**Status:** each model ran once per case or task, so these numbers are provisional.
- **Who ran:** Lazlo pre-approved running every model. That covers 10 Claude configurations (current Claude 5.x models at default and high effort, plus three older 4.5 models) and 30 OpenAI text models (default effort, default service tier).
- **Raw outputs:** in `state/bench-results/` on the machine. They are not in Git; their tables are below.
- **Cost:** for Claude, the CLI reports it. For OpenAI it isn't known, because no prices were given; tokens are recorded.

**Read with care:**
- **One run each.** gpt-6.1-terra scored 5/5 in one run of the single-bug set and 4/5 in another run half an hour apart (the silent-NaN case flipped).
- **Hard-case scores are blind hand labels, not the pattern scorer.** A Sonnet labeller marked 322 shuffled, anonymised review lines against the bug definitions. The pattern scorer undercounted: gpt-6.2-sol went from 5 to 9.
- **Single-bug "other blk" counts are not hand-checked.**
- **Errors, not results:**
  - the pro models and gpt-5.3-codex refuse Chat Completions; they need the Responses API;
  - gpt-4o and gpt-4o-mini errored on the hard case and the coding run because of the orchestrator's 32k token setting (their limit is 16,384).
- **Lazlo's unproven lead (from YouTube):** the gpt-6.2 models may be inconsistent, and 6.2-astra may follow directions less well. A 5-run consistency test is planned.
- **Model probe:** `claude-opus-4-1` silently runs as Opus 5.5, so it was excluded. `claude-sonnet-4` is retired.

## 1. Hard review case (hard1: 12 proven planted bugs, 3 decoys), blind re-score

Columns: bugs named as BLOCKING (out of 12), bugs named only as MINOR, decoys flagged, extra real problems found, false alarms, unsure lines, cost, seconds.

```
model             effort   blocking/12  minor-only  decoy  extra-real  false-alarm  unsure  cost$   secs
sonnet            high        10/12        2          0       2           1          0   0.025    20.0
fable             default      9/12        2          0       3           0          1   0.448    57.6
fable             high         9/12        2          0       3           0          0   0.443    99.8
gpt-5.6-sol       default      9/12        1          0       0           0          0   0.000    45.3
gpt-6-astra       default      9/12        2          0       1           0          0   0.000    26.7
gpt-6-sol         default      9/12        1          0       0           0          0   0.000    20.7
gpt-6.1-sol       default      9/12        1          0       0           0          0   0.000    22.4
gpt-6.1-terra     default      9/12        1          0       2           0          1   0.000    51.0
gpt-6.2-astra     default      9/12        2          0       6           0          2   0.000    17.1
gpt-6.2-sol       default      9/12        2          0       3           0          3   0.000    29.2
opus              default      9/12        2          0       2           0          3   0.163    29.2
sonnet            default      9/12        3          0       0           0          0   0.071    17.4
chat-latest       default      8/12        3          0       1           0          1   0.000     5.0
gpt-5             default      8/12        2          0       0           0          0   0.000   129.8
gpt-5.6-terra     default      8/12        2          0       0           0          0   0.000    35.0
gpt-6-luna        default      8/12        1          0       0           0          0   0.000    33.6
haiku             default      8/12        2          0       0           0          0   0.007    38.5
opus              high         8/12        3          0       3           0          1   0.072    36.1
gpt-5.1           default      7/12        0          0       0           2          0   0.000    10.0
gpt-5.5           default      7/12        1          0       0           0          0   0.000    46.3
claude-sonnet-4-5 default      6/12        1          0       0           0          0   0.253   224.4
gpt-5.4           default      6/12        2          0       0           0          0   0.000    11.5
claude-opus-4-5   default      5/12        2          0       0           0          0   0.196    67.4
gpt-5-mini        default      5/12        1          0       0           0          0   0.000    89.5
gpt-5.2           default      5/12        2          0       0           0          0   0.000     8.2
gpt-5.6-luna      default      5/12        1          0       0           0          0   0.000     9.6
o3                default      5/12        1          0       0           0          0   0.000    16.4
claude-haiku-4-5  default      4/12        1          0       0           0          0   0.088    99.3
gpt-5.4-mini      default      3/12        3          0       0           1          0   0.000     3.6
gpt-4.1           default      2/12        1          0       0           0          1   0.000     2.1
gpt-5-nano        default      2/12        0          0       0           0          0   0.000    51.3
gpt-5.4-nano      default      2/12        1          0       0           3          0   0.000     4.2
gpt-4.1-mini      default      1/12        0          0       0           0          0   0.000     1.5
gpt-4o            default      0/12        0          0       0           0          0   0.000     0.6  ERROR
gpt-4o-mini       default      0/12        0          0       0           0          0   0.000     0.7  ERROR
gpt-5-pro         default      0/12        0          0       0           0          0   0.000     0.2  ERROR
gpt-5.2-pro       default      0/12        0          0       0           0          0   0.000     0.2  ERROR
gpt-5.3-codex     default      0/12        0          0       0           0          0   0.000     0.4  ERROR
gpt-5.4-pro       default      0/12        0          0       0           0          0   0.000     0.2  ERROR
gpt-5.5-pro       default      0/12        0          0       0           0          0   0.000     0.2  ERROR
```

## 2. Single-bug review cases (5 real first-pass bugs from Broadsword's history)

Columns: caught as BLOCKING; MINOR only; known false positives; other BLOCKING lines (not hand-checked); cost; average seconds; errors.

```
model   effort   caught   minor-only  known FP  other blk  cost $   avg secs errors
chat-latest default  0/0      0           0         0          0.000    -        5
claude-opus-4-5 default  1/2      0           0         0          0.336    53       0
fable   default  4/5      1           0         3          2.024    57       0
fable   high     4/5      1           0         1          2.228    114      0
gpt-4.1 default  0/5      0           0         0          0.000    1        0
gpt-4.1-mini default  0/5      2           0         0          0.000    5        0
gpt-4o  default  0/5      0           0         2          0.000    3        0
gpt-4o-mini default  0/5      1           0         2          0.000    2        0
gpt-5   default  1/5      0           0         0          0.000    113      0
gpt-5-mini default  0/5      0           0         2          0.000    61       0
gpt-5-nano default  0/5      0           0         2          0.000    78       0
gpt-5-pro default  0/0      0           0         0          0.000    -        5
gpt-5.1 default  0/5      0           0         11         0.000    5        0
gpt-5.2 default  2/5      0           0         6          0.000    5        0
gpt-5.2-pro default  0/0      0           0         0          0.000    -        5
gpt-5.3-codex default  0/0      0           0         0          0.000    -        5
gpt-5.4 default  2/5      0           0         5          0.000    5        0
gpt-5.4-mini default  1/5      0           0         11         0.000    3        0
gpt-5.4-nano default  1/5      1           0         11         0.000    4        0
gpt-5.4-pro default  0/0      0           0         0          0.000    -        5
gpt-5.5 default  4/5      0           0         3          0.000    61       0
gpt-5.5-pro default  0/0      0           0         0          0.000    -        5
gpt-5.6-luna default  2/5      0           0         5          0.000    20       0
gpt-5.6-sol default  4/5      0           0         4          0.000    41       0
gpt-5.6-terra default  4/5      0           0         2          0.000    33       0
gpt-6-astra default  5/5      0           0         6          0.000    29       0
gpt-6-luna default  3/5      0           0         3          0.000    32       0
gpt-6-sol default  4/5      0           0         8          0.000    18       0
gpt-6.1-sol default  4/5      1           0         7          0.000    24       0
gpt-6.1-terra default  5/5      0           0         17         0.000    55       0
gpt-6.2-astra default  5/5      0           0         20         0.000    24       0
gpt-6.2-sol default  4/4      0           0         18         0.000    25       1
haiku   default  3/5      0           0         5          0.013    25       0
o3      default  1/5      0           0         2          0.000    29       0
opus    default  4/5      1           0         3          0.314    31       0
opus    high     4/5      1           0         3          0.453    45       0
sonnet  default  2/5      0           0         3          0.095    16       0
sonnet  high     2/5      3           0         1          0.272    42       0

errored runs (excluded from caught x/n):
  36bca91 chat-latest default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  36bca91 gpt-5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  36bca91 gpt-5.2-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  36bca91 gpt-5.3-codex default: HTTP 404: the API refused the request; if it is the model name, check --model. API message: This model is not supported in the v1/chat/completions endpoint. Use the v1/responses endpoint instead.
  36bca91 gpt-5.4-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  36bca91 gpt-5.5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  36bca91 gpt-6.2-sol default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  3ad7320 chat-latest default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  3ad7320 gpt-5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  3ad7320 gpt-5.2-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  3ad7320 gpt-5.3-codex default: HTTP 404: the API refused the request; if it is the model name, check --model. API message: This model is not supported in the v1/chat/completions endpoint. Use the v1/responses endpoint instead.
  3ad7320 gpt-5.4-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  3ad7320 gpt-5.5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  a19ee1a chat-latest default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  a19ee1a gpt-5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  a19ee1a gpt-5.2-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  a19ee1a gpt-5.3-codex default: HTTP 404: the API refused the request; if it is the model name, check --model. API message: This model is not supported in the v1/chat/completions endpoint. Use the v1/responses endpoint instead.
  a19ee1a gpt-5.4-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  a19ee1a gpt-5.5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  cadbb6d chat-latest default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  cadbb6d gpt-5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  cadbb6d gpt-5.2-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  cadbb6d gpt-5.3-codex default: HTTP 404: the API refused the request; if it is the model name, check --model. API message: This model is not supported in the v1/chat/completions endpoint. Use the v1/responses endpoint instead.
  cadbb6d gpt-5.4-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  cadbb6d gpt-5.5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  fd98728 chat-latest default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  fd98728 gpt-5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  fd98728 gpt-5.2-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  fd98728 gpt-5.3-codex default: HTTP 404: the API refused the request; if it is the model name, check --model. API message: This model is not supported in the v1/chat/completions endpoint. Use the v1/responses endpoint instead.
  fd98728 gpt-5.4-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
  fd98728 gpt-5.5-pro default: HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining. Add credits to continue using the API at https://platform.openai.com/settings/organization/billing/.
```

## 3. Coding benchmark (6 graded Rust tasks, 97 hidden tests, one-shot, no tools)

Claude:
```
model          effort   01-duration  02-ring-buf  03-merge-in  04-ttl-lru  05-expressi  06-line-dif  score       compile  format  cost    time
claude:fable   default  12/12        13/13        14/14        19/19       20/20        0/19         78/97 80%   1        0       $0.714  169s
claude:fable   high     12/12        13/13        14/14        19/19       20/20        19/19        97/97 100%  0        0       $2.078  414s
claude:haiku   default  12/12        13/13        14/14        19/19       20/20        19/19        97/97 100%  0        0       $0.021  176s
claude:opus    default  12/12        13/13        14/14        19/19       20/20        19/19        97/97 100%  0        0       $0.444  201s
claude:opus    high     12/12        13/13        14/14        19/19       20/20        0/19         78/97 80%   1        0       $0.422  182s
claude:sonnet  default  12/12        13/13        14/14        19/19       20/20        19/19        97/97 100%  0        0       $0.122  83s 
claude:sonnet  high     12/12        13/13        14/14        19/19       20/20        19/19        97/97 100%  0        0       $0.143  101s
```

OpenAI (the main run was still in progress at session end; gpt-6.1-terra was run separately):
```
model                 effort   01-duration  02-ring-buf  03-merge-in  04-ttl-lru  05-expressi  06-line-dif  score      compile  format  cost  time
openai:gpt-4.1        default  11/12        13/13        14/14        0/19        0/20         0/19         38/97 39%  3        0       n/a   50s 
openai:gpt-4.1-mini   default  10/12        13/13        14/14        0/19        0/20         0/19         37/97 38%  3        0       n/a   128s
openai:gpt-4o         default  0/12         0/13         0/14         0/19        0/20         0/19         0/97 0%    6        6       n/a   2s  
openai:gpt-4o-mini    default  0/12         0/13         0/14         0/19        0/20         0/19         0/97 0%    6        6       n/a   2s  
openai:gpt-5          default  12/12        0/13         14/14        19/19       0/20         19/19        64/97 66%  2        0       n/a   861s
openai:gpt-5-mini     default  10/12        13/13        14/14        0/19        20/20        0/19         57/97 59%  2        0       n/a   334s
openai:gpt-5-nano     default  0/12         0/13         14/14        18/19       13/20        19/19        64/97 66%  2        0       n/a   551s
openai:gpt-5.1        default  11/12        0/13         14/14        0/19        0/20         0/19         25/97 26%  4        0       n/a   105s
openai:gpt-5.2        default  11/12        13/13        14/14        0/19        0/20         19/19        57/97 59%  1        0       n/a   132s
openai:gpt-5.3-codex  default  0/12         0/13         0/14         0/19        0/20         0/19         0/97 0%    6        6       n/a   1s  
openai:gpt-5.4-mini   default  10/12        13/13        14/14        19/19       13/20        19/19        88/97 91%  0        0       n/a   29s 
openai:gpt-5.4-nano   default  11/12        13/13        14/14        16/19       0/20         19/19        73/97 75%  1        0       n/a   64s 
openai:o3             default  11/12        13/13        14/14        19/19       16/20        5/19         78/97 80%  0        0       n/a   216s
openai:gpt-6.1-terra  default  12/12        13/13        14/14        19/19       20/20        19/19        97/97 100%  0        0       n/a   148s
```

## 4. OpenAI service tiers (probe)

- **Supported values** (from the API's own error message): `auto`, `default`, `fast`, `flex` and `priority`.
- **Speed:** on gpt-6.1-terra, `fast` reviewed a small case in about 22 s, against about 38 s at default (2 runs each).
- **Per-model support:**
  - flex works on terra and 5.6-sol, but not on 6.2-sol ("use background=true").
  - `ultrafast`, as named in the ChatGPT app, is refused on all three tested models with a distinct "Invalid service_tier argument", which suggests it is app-only or account-gated.
- **Price premium:** unknown.

## What this changed

- The merge queue now has a **shadow second reviewer, gpt-5.6-sol** (`bb7375c`). It is recorded only and never gates a merge. Lazlo chose shadow mode. It was chosen for a clean top-group record: 9/12 on the hard case with no false alarms, and 4/5 on the single-bug set.

## Next

- Repeat runs (5x) for consistency, starting with the gpt-6.2 models and a stable comparison.
- Re-run the errored models with the Responses API and a per-model token cap.
- Benchmark the `fast` and `flex` tiers.
- Finish the OpenAI coding run.
