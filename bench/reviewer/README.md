# Reviewer benchmark

Five known-bug cases (`cases.json`), each scored on whether a model flags the real bug as BLOCKING.
Claude half: `run.py` (headless Claude Code). OpenAI half: `run_openai.py` (Chat Completions, or the Responses API for the models `../models.json` marks `responses`). Codex half: `run_codex.py` (ChatGPT plan). Both use the same
system prompt (`run.py`'s `SYSTEM`) and the same user prompt, so results compare directly. Results from both go through `score.py`.

Results so far: `evidence/reviewer-benchmark-claude.md`.

## Before any run

1. Regenerate the diffs: `./make_cases.sh` (they are not committed). Each diff holds **every file the commit touched** (tests, `broadsword.toml`, `.gitignore`, lock files), only `__pycache__` is left out, and the script refuses to finish if a touched file or `broadsword.toml` is missing. These diffs are 2-3 times longer than the old source-only ones (which hid `broadsword.toml` and drew false "setting not added" findings). **Catch rates, false-positive counts and costs from runs on the old source-only diffs are not comparable with runs on these**; say which diffs a result used.
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
- A 200 answer that is empty or whose `finish_reason` is not `stop` (e.g. `length`: reasoning used up `--max-tokens`) is recorded as an error (`is_error: true`, `exit: 1`, `error` naming the finish_reason and usage), never as an empty review. `score.py` prints such rows with `ERROR` but still counts them in the run total as not caught, so read the ERROR rows before comparing; raise `--max-tokens` and rerun them.
- `model_used` in each result is the response's `model` field; check it matches the model asked for.
- Copy results worth keeping into `evidence/`; scratch can be wiped.

## Model table: `bench/models.json`

`run_openai.py` reads it. Per model: `api` (`chat` or `responses`; gpt-5-pro, gpt-5.2/5.4/5.5-pro and gpt-5.3-codex refuse Chat Completions), `max_output_tokens` (a larger `--max-tokens` is lowered to it, with a note; gpt-4o and gpt-4o-mini stop at 16384) and `service_tiers` (`--service-tier` must be one of them; `default` sends none). **A model not in the file is an error, not a default**: add a row first. Rows with a `note` are assumptions, not measurements. Results record `backend` (`openai-chat` / `openai-responses`) and `max_tokens_sent`. Responses runs send `instructions` (the system prompt), `input` and `max_output_tokens`, and `reasoning.effort` unless the effort is `default`.

## Codex half: `run_codex.py`

    python -I -B run_codex.py ../../scratch/bench-codex <model> default [case-id ...]

Same arguments and result files as `run_openai.py`, with `backend: "codex"` and `cost_usd: null`. Uses ChatGPT plan quota: ask Lazlo first. Only effort `default` is accepted, and the API-only flags (`--base-url`, `--max-tokens`, `--retries`, `--backoff`, `--price-*`, `--service-tier`, `--root`) are refused if given. **Security:** the diff is untrusted and codex reads any file it has a tool for, so the runner calls `clients/openai_review.py`'s `review_codex()` and never builds codex flags itself: no tool is offered (one-entry model catalog, `docs/SHADOW_SANDBOX.md`), the child environment drops the API key and every `.env` value, and a tool item in the event stream makes the answer an error. There is no API fallback.

## Score and compare

    python -I -B score.py ../../scratch/bench-openai

prints one row per run and a summary per model and effort. Run it on the Claude output folder too and put the two summaries side by side.
Hand-check every blocking line that matched neither the catch pattern nor a known false positive (`other`), as in the Claude write-up.
Token counts: OpenAI `prompt_tokens` / `completion_tokens` are written as `input_tokens` / `output_tokens` for `score.py`.

## Tests (no network, fake key)

    env -u OPENAI_API_KEY python -I -B -m unittest discover -s . -p "test_run_*.py" -v

(`test_run_codex.py` uses a fake codex on a temporary PATH; neither file makes a real call.)

(`python -I` leaves the current folder off the import path, so plain `-m unittest test_run_openai` cannot find the module.)

## Gate-prompt replay: `replay.py`

Re-runs the queue's low-risk (Haiku) gating call on saved queue prompts for two system-prompt arms: A is today's
`SYSTEM_PROMPT_GATE`, read from reviewer.rs through the canary's allow-listed parser; B is A plus the rules in a text
file (`replay-rules-2026-11-20.txt`) at the end of the gate block. `python -I -B replay.py --show` prints both arms'
length and sha256 without a call. A run checks every prompt file before the first paid call, reuses an existing reply
file instead of buying it again, and stops if a reply names a non-Haiku model. Usage and the call are in its docstring.
Tests (no model call): `python -I -B -m unittest discover -s bench/reviewer -p "test_replay.py" -v`.
Results: `results/prompt-rules-2026-11-20.md` (DROP; raw replies beside it).
