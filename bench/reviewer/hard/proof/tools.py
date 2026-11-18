"""Helpers for prove.sh.

  python -I -B tools.py mutate <crate-dir>   remove the delay cap from backoff() in a copy of a crate version
  python -I -B tools.py report <work-dir>    read the proof output files and print the table; exit 1 if any row is wrong
"""
import pathlib, re, sys

CAP = ".min(cfg.max_delay_ms)"

BUGS = [
    ("bug01", "off-by-one at a boundary"),
    ("bug02", "race on a file (check, then write)"),
    ("bug03", "path traversal"),
    ("bug04", "secret written to a log"),
    ("bug05", "error swallowed where it matters"),
    ("bug06", "integer truncation of a size"),
    ("bug07", "wrong unit"),
    ("bug08", "inverted guard"),
    ("bug09", "lock left behind on early return"),
    ("bug10", "test that can never fail"),
    ("bug11", "doc contradicts the code"),
    ("bug12", "wrong backoff"),
]
DECOYS = [("decoy1", "unwrap proven safe"), ("decoy2", "range-checked cast"), ("decoy3", "documented ignored error")]


def mutate(d):
    f = pathlib.Path(d) / "src" / "retry.rs"
    text = f.read_text(encoding="utf-8")
    if text.count(CAP) != 1:
        sys.exit(f"mutation target not found exactly once in {f}")
    f.write_text(text.replace(CAP, ""), encoding="utf-8")


def outcomes(work, version):
    """test-name prefix -> True (passed) / False (failed), from the cargo test output."""
    res = {}
    out = (pathlib.Path(work) / f"{version}.proof.txt").read_text(encoding="utf-8", errors="replace")
    for m in re.finditer(r"^test (\w+) \.\.\. (ok|FAILED)", out, re.M):
        res[m.group(1).split("_")[0]] = m.group(2) == "ok"
    # bug10: the mutated copy's own test must fail (the mutation is caught) for the test to count as real
    code = int((pathlib.Path(work) / f"{version}.mut.exit").read_text().strip())
    lib = (pathlib.Path(work) / f"{version}.mut.txt").read_text(encoding="utf-8", errors="replace")
    ran = "backoff_stays_within_cap ... " in lib
    res["bug10"] = ran and code != 0
    return res


def report(work):
    clean, buggy = outcomes(work, "clean"), outcomes(work, "buggy")
    ok = True
    print(f"{'id':<7} {'kind':<36} {'buggy':<8} {'clean':<8} result")
    for key, kind in BUGS:
        b, c = buggy.get(key), clean.get(key)
        good = (b is False) and (c is True)
        ok &= good
        print(f"{key:<7} {kind:<36} {'FAIL' if b is False else 'pass' if b else '?':<8} {'pass' if c else 'FAIL' if c is False else '?':<8} "
              + ("fails on buggy, passes on clean" if good else "WRONG"))
    for key, kind in DECOYS:
        b, c = buggy.get(key), clean.get(key)
        good = b is True and c is True
        ok &= good
        print(f"{key:<7} {kind:<36} {'pass' if b else 'FAIL' if b is False else '?':<8} {'pass' if c else 'FAIL' if c is False else '?':<8} "
              + ("passes on both" if good else "WRONG"))
    nb = sum(1 for k, _ in BUGS if buggy.get(k) is False and clean.get(k) is True)
    nd = sum(1 for k, _ in DECOYS if buggy.get(k) is True and clean.get(k) is True)
    print(f"\n{nb}/12 bugs fail on buggy and pass on clean; {nd}/3 decoys pass on both")
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    {"mutate": mutate, "report": report}[sys.argv[1]](sys.argv[2])
