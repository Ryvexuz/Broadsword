# Reviewer benchmark

Five known-bug cases (`cases.json`), each scored on whether a model flags the real bug as BLOCKING.
Claude half: `run.py` (headless Claude Code). OpenAI half: `run_openai.py` (Chat Completions). Both use the same
system prompt (`run.py`'s `SYSTEM`) and the same user prompt, so results compare directly. Results from both go through `score.py`.

Results so far: `evidence/reviewer-benchmark-claude.md`.

## Before any run

1. Regenerate the diffs: `./make_cases.sh` (they are not committed).
2. Ask Lazlo first. OpenAI runs cost real money and use the API key.
3. The key comes from `OPENAI_API_KEY` or the root `.env` (never printed or written). If both exist and differ, the environment one is used and a warning is printed.

## OpenAI half: one command per model

From `bench/reviewer/`, with `<model>` the name Lazlo gives. Output goes in the scratch folder:

    python -I -B run_openai.py ../../scratch/bench-openai <model> default --price-in <USD/1M in> --price-out <USD/1M out>
    python -I -B run_openai.py ../../scratch/bench-openai <model> high    --price-in <USD/1M in> --price-out <USD/1M out>

- The second argument is the model, the third the `reasoning_effort` value (`low`, `medium`, `high`, ...) or `default` to send none. Models that do not accept `reasoning_effort` get `default`.
- Optional trailing case ids run only those cases, e.g. `... default a19ee1a fd98728`.
- Prices are never guessed: without both `--price-in` and `--price-out`, `cost_usd` is null.
- `--timeout`, `--max-tokens`, `--retries`, `--backoff` have defaults shown in `--help`.
- `model_used` in each result is the response's `model` field; check it matches the model asked for.
- Copy results worth keeping into `evidence/`; scratch can be wiped.

## Score and compare

    python -I -B score.py ../../scratch/bench-openai

prints one row per run and a summary per model and effort. Run it on the Claude output folder too and put the two summaries side by side.
Hand-check every blocking line that matched neither the catch pattern nor a known false positive (`other`), as in the Claude write-up.
Token counts: OpenAI `prompt_tokens` / `completion_tokens` are written as `input_tokens` / `output_tokens` for `score.py`.

## Tests (no network, fake key)

    env -u OPENAI_API_KEY python -I -B -m unittest discover -s . -p test_run_openai.py -v

(`python -I` leaves the current folder off the import path, so plain `-m unittest test_run_openai` cannot find the module.)
