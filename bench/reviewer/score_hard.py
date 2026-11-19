"""Score reviews of the hard case (hard1): 12 planted bugs and 3 decoys.

Usage: python -I -B score_hard.py <out-dir> [-v]
Reads the result JSON files that run.py and run_openai.py write (other cases in the folder are skipped).
Per review:
- caught: a BLOCKING line is at the bug's location and matches the bug's `catch` regex.
- minor-only: no BLOCKING line matches, but a MINOR line does (noticed, under-rated).
- missed: neither.
- decoy FPs: decoys that a BLOCKING or MINOR line flags (false alarms). A decoy counts once.
- other: BLOCKING lines that match no bug and no decoy. They can be real extra problems or false
  alarms, so they are checked by hand (-v prints them).
Location rule (the one place that keeps bugs and decoys apart): a line counts for a bug or decoy only if it names that
item's file (`src/log.rs`, `src/retry.rs`, ...) and its line number (`file:N` or `file:N-M`), if given, is not at a
DIFFERENT item of that file (within LINE_SLACK of that item's `lines`). A number that points at no item is not held
against the finding, because reviewers' line numbers are often far off. Only then is the item's `catch` regex applied. A line with no file counts for
nothing (it is "other"). A line without a number counts for every item in its file whose wording matches.
Items can still overlap where their locations do and the wording is loose (b08 and b09 share store.rs:157-169); the
tests in test_score_hard.py check that no sample line credits two items, so that is enforced, not assumed.
"""
import json, pathlib, re, sys

HERE = pathlib.Path(__file__).resolve().parent
CASE_ID = "hard1"


def load_case():
    for c in json.loads((HERE / "cases.json").read_text(encoding="utf-8")):
        if c["id"] == CASE_ID:
            return c
    raise SystemExit(f"{CASE_ID} is not in cases.json")


def _lines(text, tag):
    out = []
    for l in (text or "").splitlines():
        l = l.strip().lstrip("-*• ").strip()
        if l.upper().startswith(tag + ":") and "none" not in l.lower()[: len(tag) + 7]:
            out.append(l)
    return out


LINE_SLACK = 3  # a finding may point a few lines off the item's range
_LOC = re.compile(r"([\w./\-]+\.rs)(?::(\d+)(?:-(\d+))?)?")


def parse_loc(line):
    """-> (file basename, first line, last line) of the first `path.rs[:N[-M]]` in a finding; the lines are None when
    it gives none, and the file is None when it names no .rs file."""
    m = _LOC.search(line)
    if not m:
        return None, None, None
    a = int(m.group(2)) if m.group(2) else None
    return re.split(r"[/\\]", m.group(1))[-1], a, int(m.group(3)) if m.group(3) else a


def _basename(path):
    return path.rsplit("/", 1)[-1]


def _overlaps(item, a, z):
    return a <= item["lines"][1] + LINE_SLACK and z >= item["lines"][0] - LINE_SLACK


def at_item(line, item, case):
    """The finding names the item's file, and its line number does not point at a different item of that file.
    Reviewers often give line numbers that are off by a lot, so a number that points at no item at all is not held
    against the finding; the wording then decides."""
    f, a, z = parse_loc(line)
    if f is None or f != _basename(item["file"]):
        return False
    if a is None or _overlaps(item, a, z):
        return True
    return not any(_basename(i["file"]) == f and _overlaps(i, a, z) for i in case["bugs"] + case["decoys"])


def matches(line, item, case):
    """True if the finding is at the item's location and its wording matches the item's `catch`."""
    return at_item(line, item, case) and re.search(item["catch"], line) is not None


def matched_ids(line, case):
    return [i["id"] for i in case["bugs"] + case["decoys"] if matches(line, i, case)]


def score_text(text, case):
    """-> dict with the per-bug outcome and the counts, for one review text."""
    blocking, minor = _lines(text, "BLOCKING"), _lines(text, "MINOR")
    caught, minor_only, missed = [], [], []
    for b in case["bugs"]:
        if any(matches(l, b, case) for l in blocking):
            caught.append(b["id"])
        elif any(matches(l, b, case) for l in minor):
            minor_only.append(b["id"])
        else:
            missed.append(b["id"])
    fps = [d["id"] for d in case["decoys"] if any(matches(l, d, case) for l in blocking + minor)]
    other = [l for l in blocking if not matched_ids(l, case)]
    return dict(caught=caught, minor_only=minor_only, missed=missed, decoy_fps=fps, other=other)


def score_dir(out):
    case = load_case()
    rows = []
    for f in sorted(pathlib.Path(out).glob("*.json")):
        if f.name.endswith(".raw.json"):
            continue
        r = json.loads(f.read_text(encoding="utf-8"))
        if r.get("case") != CASE_ID:
            continue
        s = score_text(r.get("result") or "", case)
        s.update(model=r.get("model", "?"), effort=r.get("effort", "?"), cost=r.get("cost_usd") or 0,
                 wall=r.get("wall_s") or 0, err=bool(r.get("is_error")) or r.get("exit") not in (0, None), file=f.name)
        rows.append(s)
    return case, rows


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("-")]
    verbose = "-v" in sys.argv
    if len(args) != 1:
        sys.exit(__doc__)
    case, rows = score_dir(args[0])
    n = len(case["bugs"])
    print(f"{'model':<14} {'effort':<8} {'caught/' + str(n):<10} {'minor-only':<11} {'missed':<7} {'decoy FPs':<10} {'other':<6} {'cost $':<8} {'secs'}")
    for x in sorted(rows, key=lambda x: (x["model"], x["effort"], x["file"])):
        print(f"{x['model']:<14} {x['effort']:<8} {len(x['caught'])}/{n:<8} {len(x['minor_only']):<11} {len(x['missed']):<7} "
              f"{len(x['decoy_fps']):<10} {len(x['other']):<6} {x['cost']:<8.3f} {x['wall']}" + ("  ERROR" if x["err"] else ""))
    if verbose:
        for x in rows:
            print(f"\n{x['model']} {x['effort']} ({x['file']})")
            print("  caught    :", " ".join(x["caught"]) or "-")
            print("  minor-only:", " ".join(x["minor_only"]) or "-")
            print("  missed    :", " ".join(x["missed"]) or "-")
            print("  decoy FPs :", " ".join(x["decoy_fps"]) or "-")
            for l in x["other"]:
                print("  other BLOCKING:", l)


if __name__ == "__main__":
    main()
