"""Check that no spec.md leaks the hidden tests.

Usage:  python -I -B check_specs.py

For every task it takes from tests.rs
  - every test function name,
  - every string literal of 2 or more characters (the test inputs and expected values),
  - every integer pair like (1, 3) (interval and tuple test data),
and fails if any of them appears in that task's spec.md (searched as a plain substring, with a
word boundary for names). The only exceptions are ALLOWED below: the i64 limits, which the
expression spec must state to define its overflow rule.
Exit code 0 means no leak.
"""
import pathlib, re, sys

HERE = pathlib.Path(__file__).resolve().parent
ALLOWED = {"9223372036854775807", "9223372036854775808"}   # i64::MAX and i64::MAX + 1: part of the rule itself


def found(lit, spec):
    """Substring match, but a literal that starts or ends with a word character must not sit inside a
    longer word or number (so the literal 5s is not found inside 15s)."""
    pat = re.escape(lit)
    if re.match(r"\w", lit):
        pat = r"(?<!\w)" + pat
    if re.search(r"\w$", lit):
        pat = pat + r"(?!\w)"
    return re.search(pat, spec) is not None


def main():
    bad = 0
    for t in sorted((HERE / "tasks").iterdir()):
        if not t.is_dir():
            continue
        tests = (t / "tests.rs").read_text(encoding="utf-8")
        spec = (t / "spec.md").read_text(encoding="utf-8")
        names = set(re.findall(r"#\[test\]\s*(?:#\[[^\]]*\]\s*)*fn\s+(\w+)", tests))
        lits = set()
        for m in re.finditer(r'"((?:[^"\\]|\\.)*)"', tests):
            s = m.group(1)
            if len(s.strip()) >= 2 and "{" not in s and s not in ALLOWED:
                lits.add(s)
        pairs = set(re.findall(r"\(-?\d+, -?\d+\)", tests))
        hits = [f"test name {n!r}" for n in sorted(names) if re.search(rf"\b{re.escape(n)}\b", spec)]
        hits += [f"string literal {s!r}" for s in sorted(lits) if found(s, spec)]
        hits += [f"pair {p}" for p in sorted(pairs) if found(p, spec)]
        print(f"{t.name}: {len(names)} test names, {len(lits)} string literals, {len(pairs)} pairs checked: "
              f"{'CLEAN' if not hits else 'LEAK'}")
        for h in hits:
            print(f"    in spec.md: {h}")
        bad += len(hits)
    print("RESULT: " + ("no leaks" if bad == 0 else f"{bad} leaks"))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()
