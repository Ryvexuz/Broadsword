"""Check that no spec.md leaks the hidden tests.

Usage:  python -I -B check_specs.py [tests-root]

For every task it takes from tests.rs
  - every test function name,
  - every string literal of 2 or more characters (the test inputs and expected values),
  - every tuple of 2 or more numbers or i64 constants, of any length, like (1, 3) or (i64::MIN, -7),
  - every list made only of string literals, with 1-character literals too, like ["a", "c"]
    (a lone "a" is too common to flag, a list of literals is test data),
  - every test's set of distinct integers and i64::MIN / i64::MAX constants: 3 or more of one test's
    numbers inside one spec sentence, example or line is a leak (this catches data that is no tuple or
    literal, like a time 5 / ttl 10 / now 14 / now 15 story),
and fails if any of them appears in that task's spec.md (plain substring, whitespace ignored, with a
word boundary for names and literals). The only exceptions are ALLOWED (literals) and ALLOWED_NUMBERS
(numbers), each with its reason. Exit code 0 means no leak.
tests-root (optional) is a folder laid out like tasks/ to check instead; the self-test uses it.
"""
import pathlib, re, sys

HERE = pathlib.Path(__file__).resolve().parent
# i64::MAX and i64::MAX + 1: part of the expression task's overflow rule itself
ALLOWED = {"9223372036854775807", "9223372036854775808"}
# numbers that every spec of a task may state without that being test data: (task folder, number text) -> reason
ALLOWED_NUMBERS = {}
# tasks whose number-set check is skipped, with the reason. The i64 constants never count in a set (the
# tuple check covers them, and the specs name them in their rules).
SKIP_NUMBER_SETS = {
    "05-expression": "its test inputs are expression strings, covered by the literal check; the integers outside "
                     "the strings are results and error positions, which any spec example shares by accident",
}

NUM = r"(?:-?\d+|i64::(?:MIN|MAX)(?:\s*[+-]\s*\d+)?)"
ATOM = rf'(?:{NUM}|"(?:[^"\\]|\\.)*")'
TUPLE = re.compile(rf"\(\s*{NUM}(?:\s*,\s*{ATOM})+\s*\)|\(\s*{ATOM}(?:\s*,\s*{NUM})+\s*\)")
STRLIST = re.compile(r'\[\s*"(?:[^"\\]|\\.)*"(?:\s*,\s*"(?:[^"\\]|\\.)*")*\s*,?\s*\]')
NUMTOK = re.compile(rf"(?<![\w.])(?:{NUM})(?![\w])")
SET_THRESHOLD = 3


def squash(x):
    return re.sub(r"\s+", "", x)


def found(lit, spec):
    """Substring match, but a literal that starts or ends with a word character must not sit inside a
    longer word or number (so the literal 5s is not found inside 15s)."""
    pat = re.escape(lit)
    if re.match(r"\w", lit):
        pat = r"(?<!\w)" + pat
    if re.search(r"\w$", lit):
        pat = pat + r"(?!\w)"
    return re.search(pat, spec) is not None


def test_bodies(tests):
    """(name, body text) for every #[test] fn."""
    parts = re.split(r"(?=#\[test\])", tests)
    out = []
    for p in parts[1:]:
        m = re.search(r"fn\s+(\w+)", p)
        if m:
            out.append((m.group(1), p))
    return out


def numbers_of(text):
    """Distinct integers and i64 constants of a text, as normalized tokens. Numbers inside string literals
    do not count."""
    text = re.sub(r'"(?:[^"\\]|\\.)*"', '""', text)
    return {squash(m.group(0)) for m in NUMTOK.finditer(text)}


def spec_units(spec):
    """Each line, and each sentence of a line, of the spec."""
    units = []
    for line in spec.splitlines():
        units.append(line)
        units += re.split(r"(?<=[.;:])\s+", line)
    return units


def leaks(task, tests, spec):
    """Returns (counts dict, list of hit descriptions)."""
    names = {n for n, _ in test_bodies(tests)}
    lits = set()
    for m in re.finditer(r'"((?:[^"\\]|\\.)*)"', tests):
        s = m.group(1)
        if len(s.strip()) >= 2 and "{" not in s and s not in ALLOWED:
            lits.add(s)
    sq_spec = squash(spec)
    tuples = {squash(m.group(0)) for m in TUPLE.finditer(tests)}
    lists = {squash(m.group(0)) for m in STRLIST.finditer(tests)}
    hits = [f"test name {n!r}" for n in sorted(names) if re.search(rf"\b{re.escape(n)}\b", spec)]
    hits += [f"string literal {s!r}" for s in sorted(lits) if found(s, spec)]
    hits += [f"tuple {t}" for t in sorted(tuples) if t.replace(",", ", ") and t in sq_spec]
    hits += [f"string list {t}" for t in sorted(lists) if t in sq_spec]
    units = spec_units(spec)
    nsets = 0
    for name, body in test_bodies(tests):
        if task in SKIP_NUMBER_SETS:
            break
        nums = {n for n in numbers_of(body) if (task, n) not in ALLOWED_NUMBERS and n not in ALLOWED
                and not n.startswith("i64::")}
        if len(nums) < SET_THRESHOLD:
            continue
        nsets += 1
        for u in units:
            common = sorted(nums & numbers_of(u))
            if len(common) >= SET_THRESHOLD:
                hits.append(f"numbers of test {name}: {', '.join(common)} together in {u.strip()[:90]!r}")
                break
    counts = {"names": len(names), "lits": len(lits), "tuples": len(tuples), "lists": len(lists), "sets": nsets}
    return counts, hits


def main():
    root = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else HERE / "tasks"
    bad = 0
    for t in sorted(root.iterdir()):
        if not t.is_dir():
            continue
        tests = (t / "tests.rs").read_text(encoding="utf-8")
        spec = (t / "spec.md").read_text(encoding="utf-8")
        c, hits = leaks(t.name, tests, spec)
        print(f"{t.name}: {c['names']} test names, {c['lits']} string literals, {c['tuples']} tuples, "
              f"{c['lists']} string lists, {c['sets']} number sets checked: {'CLEAN' if not hits else 'LEAK'}")
        for h in hits:
            print(f"    in spec.md: {h}")
        bad += len(hits)
    print("RESULT: " + ("no leaks" if bad == 0 else f"{bad} leaks"))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
