"""The mechanical checks of the instruction-following benchmark. No model is ever asked to judge.

Each check is a function (text, params) -> None when the rule is kept, or a short sentence naming what broke.
A case lists checks; it passes only when every one returns None. grade(case, text) returns the broken rules.

Conventions (also in README.md):
- Line endings are normalised to \\n and every line is right-stripped. "Non-empty lines" are the lines left after
  dropping blank ones; blank lines are never counted and never penalised.
- A "word" is a whitespace-separated token. A "sentence" is a run of text ending in . ! or ?.
- Nothing is lenient about wrapping: a code fence or an intro line around a JSON answer, a table or a list is a failure.
"""
import json, re


def norm(text):
    return "\n".join(l.rstrip() for l in text.replace("\r\n", "\n").replace("\r", "\n").strip().split("\n"))


def nonempty(text):
    return [l for l in norm(text).split("\n") if l.strip()]


def words(text):
    return re.findall(r"\S+", text)


def c_bullets(text, p):
    ls = nonempty(text)
    marker = p.get("marker", "-")
    for l in ls:
        if not (l.startswith(marker + " ") and l[len(marker) + 1:].strip()):
            return f"every line must start with '{marker} ' and have text; got line {l[:60]!r}"
    if len(ls) != p["n"]:
        return f"expected exactly {p['n']} bullet lines, got {len(ls)}"
    mw = p.get("max_words_each")
    if mw:
        for l in ls:
            if len(words(l[len(marker) + 1:])) > mw:
                return f"a bullet has more than {mw} words: {l[:60]!r}"
    return None


def c_numbered(text, p):
    ls = nonempty(text)
    if len(ls) != p["n"]:
        return f"expected exactly {p['n']} numbered lines, got {len(ls)}"
    for i, l in enumerate(ls, 1):
        if not re.match(rf"^{i}\.\s+\S", l):
            return f"line {i} must start with '{i}. ' and have text; got {l[:60]!r}"
    return None


def c_lines_count(text, p):
    n = len(nonempty(text))
    return None if n == p["n"] else f"expected exactly {p['n']} non-empty lines, got {n}"


def c_json_keys(text, p):
    try:
        obj = json.loads(text.strip())
    except Exception:
        return "the whole answer must be one JSON object (no code fence, no text around it)"
    if not isinstance(obj, dict):
        return "the JSON must be an object"
    if set(obj) != set(p["keys"]):
        return f"keys must be exactly {sorted(p['keys'])}, got {sorted(obj)}"
    kinds = {"str": str, "int": int, "list": list, "bool": bool}
    for k, t in p.get("types", {}).items():
        v = obj[k]
        ok = isinstance(v, kinds[t]) and not (t == "int" and isinstance(v, bool))
        if not ok:
            return f"key {k!r} must be of type {t}"
    for k, n in p.get("list_len", {}).items():
        if len(obj[k]) != n:
            return f"key {k!r} must be a list of exactly {n} items, got {len(obj[k])}"
    for k, v in p.get("values", {}).items():
        if obj[k] != v:
            return f"key {k!r} must be {v!r}, got {obj[k]!r}"
    return None


def c_max_words(text, p):
    n = len(words(text))
    return None if n <= p["k"] else f"{n} words, the limit is {p['k']}"


def c_min_words(text, p):
    n = len(words(text))
    return None if n >= p["k"] else f"{n} words, at least {p['k']} are required"


def c_max_chars(text, p):
    n = len(text.strip())
    return None if n <= p["k"] else f"{n} characters, the limit is {p['k']}"


def c_forbidden(text, p):
    for w in p["words"]:
        pat = re.escape(w) if p.get("mode") == "substring" else r"\b" + re.escape(w) + r"\b"
        if re.search(pat, text, re.I):
            return f"the forbidden word {w!r} is used"
    return None


def c_forbidden_chars(text, p):
    if p.get("ignore_list_markers"):
        text = "\n".join(re.sub(r"^\s*\d+[.)]\s*", "", l) for l in text.split("\n"))
    for ch in p["chars"]:
        if ch in text:
            return f"the forbidden character {ch!r} is used"
    return None


def c_ascii_only(text, p):
    bad = [c for c in text if ord(c) > 127]
    return None if not bad else f"non-ASCII character {bad[0]!r} (emoji, curly quotes, ...) is used"


def c_lowercase(text, p):
    return None if text == text.lower() else "the answer must be all lowercase"


def c_no_markdown(text, p):
    for ch in "*#`_":
        if ch in text:
            return f"markdown character {ch!r} is used"
    return None


def c_sentences(text, p):
    n = len(re.findall(r"[.!?]+(?=\s|$)", text.strip()))
    return None if n == p["n"] else f"expected exactly {p['n']} sentence(s), got {n}"


def c_keep_line(text, p):
    have = set(norm(text).split("\n"))
    for l in p["lines"]:
        if l not in have:
            return f"the line {l[:60]!r} must appear unchanged, character for character"
    return None


def c_first_line(text, p):
    ls = nonempty(text)
    return None if ls and ls[0] == p["text"] else f"the first line must be exactly {p['text']!r}"


def c_last_line(text, p):
    ls = nonempty(text)
    return None if ls and ls[-1] == p["text"] else f"the last line must be exactly {p['text']!r}"


def c_full_match(text, p):
    return None if re.fullmatch(p["regex"], text.strip()) else f"the answer must be exactly of the form {p['regex']}"


def c_equals(text, p):
    return None if norm(text) == p["text"] else f"the answer must be exactly {p['text']!r}"


def c_contains(text, p):
    for s in p["strings"]:
        if s not in text:
            return f"the answer must contain {s!r}"
    return None


def c_sorted_lines(text, p):
    ls = [l.strip() for l in nonempty(text)]
    return None if ls == sorted(ls) else "the lines are not in alphabetical order"


def c_table(text, p):
    ls = nonempty(text)
    if not ls or ls[0] != p["header"]:
        return f"the first line must be exactly the header {p['header']!r}"
    if len(ls) < 2 or not re.match(r"^\|(\s*:?-{3,}:?\s*\|)+$", ls[1]):
        return "the second line must be the separator row (| --- | --- |)"
    rows = ls[2:]
    if len(rows) != p["rows"]:
        return f"expected exactly {p['rows']} data rows, got {len(rows)}"
    cells = []
    for r in rows:
        if not (r.startswith("|") and r.endswith("|")):
            return f"a line is not a table row: {r[:60]!r}"
        c = [x.strip() for x in r[1:-1].split("|")]
        if len(c) != p["columns"]:
            return f"a row has {len(c)} columns, expected {p['columns']}: {r[:60]!r}"
        cells.append(c)
    if "expect_rows" in p and cells != p["expect_rows"]:
        return f"the row values are wrong; expected {p['expect_rows']}, got {cells}"
    return None


CHECKS = {k[2:]: v for k, v in dict(globals()).items() if k.startswith("c_") and callable(v)}


def grade(case, text):
    """Returns a list of (check type, what broke) for every broken rule; [] means the case passes."""
    out = []
    for chk in case["checks"]:
        fn = CHECKS.get(chk["type"])
        if fn is None:
            raise ValueError(f"case {case['id']}: unknown check type {chk['type']!r}")
        msg = fn(text, chk)
        if msg:
            out.append((chk["type"], msg))
    return out
