"""Builds cases.json: 20 instruction-following cases, each with mechanical checks (see checks.py).

Run: python -I -B bench/instruct/make_cases.py        (rewrites cases.json; deterministic, no randomness beyond a fixed seed)
A test fails when cases.json is not what this script produces.

Cases 01-14 are short. Cases 15-20 are long: about 6k tokens of log (about 24k characters), with the rules stated
at the TOP, before the log. The expected values of the long cases are computed here from the generated log.
"""
import json, pathlib, random
from collections import Counter

HERE = pathlib.Path(__file__).resolve().parent
SERVICES = ["auth", "billing", "search", "cart", "mailer", "gateway"]
WARN_MSGS = ["slow response", "retry scheduled", "cache miss storm", "queue depth high"]
ERR_MSGS = ["upstream timeout", "connection reset", "bad checksum"]
INFO_MSGS = ["request ok", "session opened", "job finished", "config reloaded", "heartbeat"]
HEADER = [
    "# LOG EXPORT v2.3  |  host=node-07",
    "# Retention: 30 days  --  owner: ops-team",
    "# Format: <timestamp> <LEVEL> <service> <message> id=<n>",
    "# Timezone: UTC (no  daylight saving)",
    "# Contact: logs@example.invalid",
]


def make_log(seed=7, n=400):
    """Returns (log text, stats). The WARN counts are forced to be all different so a top-3 has no tie."""
    while True:
        rng = random.Random(seed)
        rows, t = [], 8 * 3600
        for i in range(n):
            t += rng.randint(1, 9)
            r = rng.random()
            level = "INFO" if r < 0.72 else "WARN" if r < 0.90 else "ERROR"
            svc = rng.choice(SERVICES)
            msg = rng.choice({"INFO": INFO_MSGS, "WARN": WARN_MSGS, "ERROR": ERR_MSGS}[level])
            ts = f"2026-11-19T{t // 3600:02d}:{t % 3600 // 60:02d}:{t % 60:02d}Z"
            rows.append((ts, level, svc, f"{ts} {level:<5} {svc:<8} {msg} id={1000 + i * 7}"))
        warn = Counter(s for _, l, s, _ in rows if l == "WARN")
        top = sorted(warn.values(), reverse=True)
        if len(warn) >= 4 and len(set(top[:4])) == 4:
            break
        seed += 1
    lv = Counter(l for _, l, _, _ in rows)
    stats = {"warn": warn, "levels": lv, "first": rows[0][0], "last": rows[-1][0], "n": len(rows)}
    return "\n".join(HEADER + [r[3] for r in rows]), stats


def long_prompt(rules, log):
    return f"{rules}\n\n=== LOG START ===\n{log}\n=== LOG END ===\n\nAnswer now, following the rules at the top."


def build():
    log, st = make_log()
    top3 = st["warn"].most_common(3)
    lv = st["levels"]
    cases = []

    def add(cid, kind, prompt, checks):
        cases.append({"id": cid, "kind": kind, "prompt": prompt, "checks": checks})

    add("01-bullets", "short",
        "List 4 benefits of writing unit tests. Reply with exactly 4 lines, each line starting with '- ' and holding at most 12 words. Write nothing else.",
        [{"type": "bullets", "n": 4, "marker": "-", "max_words_each": 12}])
    add("02-json-keys", "short",
        "Invent a fictional book. Reply with only a JSON object with exactly these keys: title (string), author (string), year (integer), tags (a list of exactly 3 strings). No code fence and no other text.",
        [{"type": "json_keys", "keys": ["title", "author", "year", "tags"],
          "types": {"title": "str", "author": "str", "year": "int", "tags": "list"}, "list_len": {"tags": 3}}])
    add("03-word-limit", "short",
        "Explain what a hash map is in at most 29 words, and use the word 'key'. Reply with the explanation only.",
        [{"type": "max_words", "k": 29}, {"type": "min_words", "k": 5}, {"type": "contains", "strings": ["key"]}])
    add("04-forbidden-words", "short",
        "Describe the ocean in exactly two sentences, using the word 'ocean' at least once. Never use the words 'blue', 'water' or 'wet'.",
        [{"type": "sentences", "n": 2}, {"type": "forbidden", "words": ["blue", "water", "wet"]}, {"type": "min_words", "k": 6}, {"type": "contains", "strings": ["ocean"]}])
    add("05-keep-line", "short",
        "Fix the spelling mistakes in the text below and reply with the corrected text. The first line is a reference code: keep it exactly unchanged, character for character.\n\n"
        "REF: ZX-9041/b  (do not edit)\nI will definately write it seperate from the rest, and it is ocurring tomorrow.",
        [{"type": "keep_line", "lines": ["REF: ZX-9041/b  (do not edit)"]},
         {"type": "contains", "strings": ["definitely", "separate", "occurring"]},
         {"type": "forbidden", "words": ["definately", "seperate", "ocurring"]}])
    add("06-table-header", "short",
        "Reply with a markdown table and nothing else. The header row must be exactly `| Planet | Moons | Rings |`, followed by the separator row and exactly 3 data rows, for Earth, Mars and Saturn (in that order). Moons and Rings are plain numbers.",
        [{"type": "table", "header": "| Planet | Moons | Rings |", "columns": 3, "rows": 3},
         {"type": "contains", "strings": ["Earth", "Mars", "Saturn"]}])
    add("07-first-last-line", "short",
        "Write a 3-line note about spring. The first line must be exactly BEGIN, the last line must be exactly END, and the middle line is one sentence about spring. Write nothing else.",
        [{"type": "lines_count", "n": 3}, {"type": "first_line", "text": "BEGIN"}, {"type": "last_line", "text": "END"},
         {"type": "contains", "strings": ["pring"]}])
    add("08-numbered", "short",
        "Give the steps to boil an egg as a numbered list with exactly 5 steps, each line written as '1. text', '2. text' and so on. No introduction and no closing line.",
        [{"type": "numbered", "n": 5}])
    add("09-lowercase-no-commas", "short",
        "Write exactly one sentence about cats. It must be all lowercase and must not contain any comma.",
        [{"type": "sentences", "n": 1}, {"type": "lowercase"}, {"type": "forbidden_chars", "chars": ","}, {"type": "min_words", "k": 4}])
    add("10-exact-answer", "short",
        "What is 17 * 23? Reply with the number only, as digits, and nothing else.",
        [{"type": "equals", "text": "391"}])
    add("11-sorted-lines", "short",
        "Sort these five fruits alphabetically: pear, fig, apple, mango, cherry. Reply with one fruit per line, lowercase, no bullets, no numbers, nothing else.",
        [{"type": "lines_count", "n": 5}, {"type": "sorted_lines"}, {"type": "contains", "strings": ["pear", "fig", "apple", "mango", "cherry"]}])
    add("12-date-format", "short",
        "What is the date one day after 2024-02-28? Reply with the date in the form YYYY-MM-DD and nothing else.",
        [{"type": "full_match", "regex": r"2024-02-29"}])
    add("13-char-limit-ascii", "short",
        "Write a one-line announcement for a new coffee shop called Bean There; the name must appear. At most 120 characters in total, plain ASCII only (no emoji, no curly quotes), and no hashtags.",
        [{"type": "max_chars", "k": 120}, {"type": "ascii_only"}, {"type": "forbidden_chars", "chars": "#"}, {"type": "contains", "strings": ["Bean There"]}])
    add("14-plain-text", "short",
        "Explain recursion in exactly two sentences. Plain text only: do not use any of the characters * # ` or _ .",
        [{"type": "sentences", "n": 2}, {"type": "no_markdown"}, {"type": "min_words", "k": 8}])

    add("15-top-warn", "long", long_prompt(
        "RULES: Reply with exactly 3 lines, nothing else. Each line has the form `<service>: <count>`. They are the three services with the most WARN lines in the log below, highest count first.", log),
        [{"type": "lines_count", "n": 3}, {"type": "equals", "text": "\n".join(f"{s}: {c}" for s, c in top3)}])
    add("16-json-span", "long", long_prompt(
        "RULES: Reply with only a JSON object, no code fence and no other text. Its keys are exactly: first_ts (the timestamp of the first log line, string), last_ts (the timestamp of the last log line, string), total_lines (the number of log lines, not counting the 5 header lines starting with #, integer).", log),
        [{"type": "json_keys", "keys": ["first_ts", "last_ts", "total_lines"],
          "types": {"first_ts": "str", "last_ts": "str", "total_lines": "int"},
          "values": {"first_ts": st["first"], "last_ts": st["last"], "total_lines": st["n"]}}])
    add("17-never-service", "long", long_prompt(
        "RULES: Summarise the log below in at least 15 and at most 40 words, as plain text. Never use the text 'service' anywhere in your answer (not even inside another word such as 'services').", log),
        [{"type": "min_words", "k": 15}, {"type": "max_words", "k": 40}, {"type": "forbidden", "words": ["service"], "mode": "substring"}])
    add("18-keep-header", "long", long_prompt(
        "RULES: Reply with exactly 6 lines. Lines 1 to 5 are the 5 header lines of the log (the ones starting with #), copied exactly, character for character. Line 6 is exactly END-OF-HEADER. Write nothing else.", log),
        [{"type": "lines_count", "n": 6}, {"type": "keep_line", "lines": HEADER}, {"type": "last_line", "text": "END-OF-HEADER"}, {"type": "first_line", "text": HEADER[0]}])
    add("19-level-table", "long", long_prompt(
        "RULES: Reply with a markdown table and nothing else. The header row is exactly `| Level | Count |`, then the separator row, then exactly 3 data rows for INFO, WARN and ERROR in that order, each giving how many log lines (not the # header lines) have that level.", log),
        [{"type": "table", "header": "| Level | Count |", "columns": 2, "rows": 3,
          "expect_rows": [["INFO", str(lv["INFO"])], ["WARN", str(lv["WARN"])], ["ERROR", str(lv["ERROR"])]]}])
    add("20-lowercase-numbered", "long", long_prompt(
        "RULES: Reply with exactly 2 numbered lines ('1. ...' and '2. ...'), nothing else. Together they name two problems you see in the log, in at most 25 words in total (the '1.' and '2.' markers are not counted), all lowercase, and without any digit character in the text after the markers (write numbers out in words if you need them).", log),
        [{"type": "numbered", "n": 2}, {"type": "lowercase"}, {"type": "max_words", "k": 27},
         {"type": "forbidden_chars", "chars": "0123456789", "ignore_list_markers": True}])
    return cases


def main():
    (HERE / "cases.json").write_text(json.dumps(build(), indent=1, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")


if __name__ == "__main__":
    main()
