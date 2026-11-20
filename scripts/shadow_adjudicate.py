"""Record a verdict on a shadow-only finding.

Run:  python -I -B scripts/shadow_adjudicate.py [--root <folder with broadsword.toml>] add <entry> <finding_key> <verdict> --evidence <text> --judge <model>

verdict is one of: real (a bug the gating reviewer missed), false (wrong; say why in the evidence), known (the gating reviewer also
found it, or it is already on the board or in the records). Appends one row to <root>/state/shadow-verdicts.tsv
(header: entry, finding_key, verdict, evidence, judge, date). Rows are never deleted; a later row for the same entry and key
replaces the earlier one when read. Refuses a verdict outside the three, an entry with no shadow record, a key that is not a
shadow-only BLOCKING line of that entry (shadow_report.py --open prints the keys), and empty evidence or judge.
Read-only on state/queue; never calls a model.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import importlib.util
import time
from pathlib import Path


def _load_report():
    """shadow_report.py sits next to this file; `python -I` keeps the script folder off sys.path, so load it by path."""
    spec = importlib.util.spec_from_file_location("shadow_report", Path(__file__).resolve().with_name("shadow_report.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def add_verdict(root: Path, entry: str, key: str, verdict: str, evidence: str, judge: str, date: str = "") -> str:
    """Append one verdict row; returns the row. Raises ValueError with the reason when it refuses."""
    sr = _load_report()
    if verdict not in sr.VERDICTS:
        raise ValueError(f"verdict must be one of {', '.join(sr.VERDICTS)}, not {verdict!r}")
    evidence = " ".join(evidence.split())
    judge = " ".join(judge.split())
    if not evidence:
        raise ValueError("evidence is empty: give a failing test, a quoted line or a concrete input")
    if not judge:
        raise ValueError("judge is empty: name the gating reviewer model")
    entries = sr.load_entries(root)
    if entry not in {e for e, _ in entries}:
        raise ValueError(f"unknown entry {entry!r}: no queue entry with that id has a shadow review")
    key = sr.norm_key(key)
    mine = [it for it in sr.shadow_only(entries) if it["entry"] == entry]
    keys = {it["key"] for it in mine}
    if key not in keys:
        same_loc = [it["key"] for it in mine if it["loc"] == key]
        if len(same_loc) > 1:
            raise ValueError(f"{key!r} is ambiguous: {len(same_loc)} findings at that location; give the full key: {', '.join(sorted(same_loc))}")
        if len(same_loc) != 1:
            raise ValueError(f"unknown finding key {key!r} for {entry}; its shadow-only keys: {', '.join(sorted(keys)) or '(none)'}")
    row = "\t".join([entry, key, verdict, evidence, judge, date or time.strftime("%Y-%m-%d", time.gmtime())])
    path = root / sr.VERDICT_FILE
    path.parent.mkdir(parents=True, exist_ok=True)
    new = not path.is_file() or path.stat().st_size == 0
    with open(path, "a", encoding="utf-8", newline="") as f:
        f.write((sr.VERDICT_HEADER + "\n" if new else "") + row + "\n")
    return row


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=".", help="the Broadsword root (holds broadsword.toml and state/)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    a = sub.add_parser("add", help="append one verdict row")
    a.add_argument("entry")
    a.add_argument("finding_key")
    a.add_argument("verdict")
    a.add_argument("--evidence", required=True)
    a.add_argument("--judge", required=True)
    args = ap.parse_args(argv)
    try:
        row = add_verdict(Path(args.root), args.entry, args.finding_key, args.verdict, args.evidence, args.judge)
    except ValueError as e:
        print(f"refused: {e}", file=sys.stderr)
        return 2
    print(f"recorded: {row}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
