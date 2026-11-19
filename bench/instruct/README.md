# Instruction-following benchmark

20 prompts (`cases.json`), each with mechanical checks (`checks.py`). No model judges anything. It tests whether a model
follows exact format, length and forbidden-content rules, e.g. whether gpt-6.2-astra "follows directions less well".

- Cases 01-14 are short. Cases 15-20 are long: a synthetic 400-line server log (about 23k characters, about 6k tokens) with the
  rules stated at the TOP, before the log. The expected values (counts, timestamps) are computed from the log by `make_cases.py`.
- `cases.json` is generated: `python -I -B bench/instruct/make_cases.py`. A test fails if the file differs from what the script builds.
- A case PASSES only when every one of its checks holds. `score.py` prints the rule broken for each FAIL.
- Every case has a positive check, so an empty or junk answer cannot pass (tested: an empty answer fails all 20, and a junk reply scores 0 of 20).

## What each check measures

Whitespace: line endings are normalised, trailing spaces and blank lines are ignored. Nothing else is lenient.
A word is a whitespace-separated token; a sentence ends in `.`, `!` or `?`.

| check | measures |
|---|---|
| bullets | exactly N non-empty lines, each starting `- ` with text; optional max words per bullet. An intro line fails it |
| numbered | exactly N lines, line i starting `i. ` with text |
| lines_count | exactly N non-empty lines |
| json_keys | the whole answer is one JSON object (a code fence or text around it fails), exact key set, value types, list lengths, exact values |
| max_words / min_words / max_chars | length limits |
| forbidden | a banned word is absent (whole word, case-insensitive; `mode: substring` also bans it inside other words) |
| forbidden_chars | banned characters absent (`ignore_list_markers` skips a leading `1.` marker) |
| ascii_only | no emoji, curly quotes or other non-ASCII |
| lowercase | no uppercase letter |
| no_markdown | none of `* # `` ` `` _` |
| sentences | exactly N sentences |
| keep_line | given lines appear unchanged, character for character (inner double spaces count) |
| first_line / last_line | the first / last non-empty line is exactly the given text |
| full_match / equals | the whole answer matches a regex / equals a text |
| contains | required strings are present (guards against junk passing a pure length rule) |
| sorted_lines | the lines are in alphabetical order |
| table | exact header line, a separator row, exactly R data rows with C columns, optionally exact cell values |

## Run (costs real money, ask Lazlo first)

No provider code lives here: `run.py` imports the Claude CLI and OpenAI calls from `bench/coder/run.py`
(which reuses `bench/reviewer`). Same environment and options as there:

    BENCH_CWD=<folder outside the project> python -I -B bench/instruct/run.py <out-dir> claude <model> default
    python -I -B bench/instruct/run.py <out-dir> openai <model> <effort|default> --price-in <USD/1M> --price-out <USD/1M>
    python -I -B bench/instruct/score.py <out-dir>

`config.toml` holds `model_timeout_seconds` (required, no default in code). A failed call (provider refusal, timeout, CLI error) is
recorded with `is_error`, scored as ERROR and left out of every total. The cases' system prompt is deliberately bare
("Follow the user's instructions exactly."); the rules are in each prompt.

`score.py` totals per provider/model/effort: pass, fail, scored (= pass + fail), pass rate, errors, cases not run, cost, average seconds
per scored case.

## Tests (no network, no model)

    python -I -B -m unittest discover -s bench/instruct -v

`canned.py` holds hand-written answers: a good one per case and partly wrong or wrong-format ones naming the rule each must break.
Known limit: the checks are strict on form, so a fluent answer wrapped in a code fence counts as a FAIL for JSON/table cases by design.
