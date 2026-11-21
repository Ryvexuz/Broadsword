"""Spot check of `false` shadow-finding verdicts (SPEC_CHECKLIST.md, "Shadow-only findings", rule B).

Run:  python -I -B scripts/shadow_spotcheck.py [--root <folder with state/>] [--check]
      python -I -B scripts/shadow_spotcheck.py [--root <folder>] --record <row key> <real|false|unsure> <judge model> <evidence> [--exclude <model> ...]

Selection rule (fixed here, not an input of any judge): read <root>/state/shadow-verdicts.tsv (header: entry, finding_key, verdict,
evidence, judge, date; a later row for the same entry and key replaces the earlier one, as shadow_report.py reads it). A row is
DUE for a spot check when its effective verdict is `false` and int(SHA-256(entry + "\\n" + finding_key), 16) % 5 == 0, so about 1 in 5
false rows. The hash uses only the entry id and the finding key, never the verdict text, evidence or judge, so a judge cannot move a
row in or out of the sample by rewording. The row key printed and taken by --record is "<entry>|<finding_key>".

Default: print every due row that has no result yet in <root>/state/shadow-spotchecks.tsv (header: entry, finding_key, result,
judge, evidence, date); exit 0. --check: same listing, exit 1 while any due row is open (for the session-end check), else 0.
--record: append one result under an OS lock on <spotchecks file>.lock. Refuses a key that is not due, a key already recorded, a
result outside real|false|unsure, empty evidence or judge, and a judge whose model FAMILY (haiku, sonnet, opus, fable or gpt:
any name containing one, so `claude-opus-5-5` is opus) equals the family of the row's original judge (first word of the `judge`
column; `orchestrator` is ORCHESTRATOR_FAMILY, opus) or of any --exclude model (pass the coder and the gating reviewer; the tsv
has no such columns). An unknown or empty family, for the judge, the original judge or an --exclude, is refused. The judge being a strong model that is none of those is the
orchestrator's duty; this script cannot see it.
A malformed tsv (wrong header, wrong column count, empty entry or key, unknown verdict or result) fails closed: exit 2, nothing
written. Never writes state/shadow-verdicts.tsv. Standard library only.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import hashlib
import os
import time
from pathlib import Path

VERDICT_FILE = Path("state") / "shadow-verdicts.tsv"
SPOT_FILE = Path("state") / "shadow-spotchecks.tsv"
VERDICT_HEADER = "entry\tfinding_key\tverdict\tevidence\tjudge\tdate"
SPOT_HEADER = "entry\tfinding_key\tresult\tjudge\tevidence\tdate"
VERDICTS = ("real", "false", "known")
RESULTS = ("real", "false", "unsure")
SAMPLE_MOD = 5  # 1 in 5


class Malformed(ValueError):
    pass


def _read_rows(path: Path, header: str, allowed: tuple, label: str) -> list:
    """Parsed rows (list of column lists). Raises Malformed on anything off; a missing file is an empty list."""
    if not path.is_file():
        return []
    raw = path.read_bytes()
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError as e:
        raise Malformed(f"{path.name}: not UTF-8 ({e})")
    if text == "":
        return []
    lines = text.split("\n")
    if lines and lines[-1] == "":
        lines.pop()
    if "\r" in text:
        raise Malformed(f"{path.name}: carriage return in file")
    if not lines or lines[0] != header:
        raise Malformed(f"{path.name}: header is not {header!r}")
    n = len(header.split("\t"))
    rows = []
    for i, line in enumerate(lines[1:], start=2):
        cols = line.split("\t")
        if len(cols) != n:
            raise Malformed(f"{path.name} line {i}: {len(cols)} columns, expected {n}")
        if not cols[0].strip() or not cols[1].strip():
            raise Malformed(f"{path.name} line {i}: empty entry or finding_key")
        if cols[2] not in allowed:
            raise Malformed(f"{path.name} line {i}: {label} {cols[2]!r} is not one of {', '.join(allowed)}")
        rows.append(cols)
    return rows


def effective_verdicts(root: Path) -> dict:
    """{(entry, key): row} with the last row for each pair winning."""
    out = {}
    for cols in _read_rows(root / VERDICT_FILE, VERDICT_HEADER, VERDICTS, "verdict"):
        out[(cols[0], cols[1])] = cols
    return out


def is_selected(entry: str, key: str) -> bool:
    h = hashlib.sha256((entry + "\n" + key).encode("utf-8")).hexdigest()
    return int(h, 16) % SAMPLE_MOD == 0


def due_rows(root: Path) -> list:
    """Every verdict row that is `false` and in the sample, in file order of the winning row."""
    return [cols for (e, k), cols in effective_verdicts(root).items() if cols[2] == "false" and is_selected(e, k)]


def recorded(root: Path) -> dict:
    return {(c[0], c[1]): c for c in _read_rows(root / SPOT_FILE, SPOT_HEADER, RESULTS, "result")}


def open_rows(root: Path) -> list:
    done = recorded(root)
    return [c for c in due_rows(root) if (c[0], c[1]) not in done]


# The orchestrator is the main session, an Opus model (AGENTS.md, "Orchestrator (main session, Opus)"). The name `orchestrator`
# in the verdicts file therefore has this family. Change it here if the orchestrator's model changes.
ORCHESTRATOR_FAMILY = "opus"
# Mirrors reviewer::model_family in crates/broadsword/src/reviewer.rs (haiku, sonnet, opus by substring), plus fable and gpt,
# which that function leaves as the trimmed name. Anything else is unknown and refused.
FAMILIES = ("haiku", "sonnet", "opus", "fable", "gpt")


def family(name: str) -> str:
    """The family of a model name: `orchestrator` -> ORCHESTRATOR_FAMILY; a name containing a known family -> that family
    (`claude-opus-5-5` -> opus); empty or unknown -> "" (callers refuse)."""
    n = name.strip().lower()
    if n == "orchestrator":
        return ORCHESTRATOR_FAMILY
    for f in FAMILIES:
        if f in n:
            return f
    return ""


def judge_family(judge: str) -> str:
    """Family of a judge column or a --record judge: the first word names the model ('haiku (gating)', 'orchestrator (...)')."""
    parts = judge.split()
    return family(parts[0]) if parts else ""


class _Lock:
    """Exclusive OS lock on a sidecar file; waits up to ten seconds, then raises."""

    def __init__(self, path: Path):
        self.path = path

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.f = open(self.path, "a+b")
        deadline = time.monotonic() + 10
        while True:
            try:
                if os.name == "nt":
                    import msvcrt

                    self.f.seek(0)
                    msvcrt.locking(self.f.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl

                    fcntl.flock(self.f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except OSError:
                if time.monotonic() > deadline:
                    self.f.close()
                    raise ValueError(f"could not lock {self.path} within 10 seconds")
                time.sleep(0.05)

    def __exit__(self, *exc):
        try:
            if os.name == "nt":
                import msvcrt

                self.f.seek(0)
                msvcrt.locking(self.f.fileno(), msvcrt.LK_UNLCK, 1)
            else:
                import fcntl

                fcntl.flock(self.f.fileno(), fcntl.LOCK_UN)
        finally:
            self.f.close()


def record(root: Path, rowkey: str, result: str, judge: str, evidence: str, exclude=(), date: str = "") -> str:
    """Append one spot-check result; returns the row. Raises ValueError (Malformed included) with the reason when it refuses."""
    if "|" not in rowkey:
        raise ValueError(f"row key {rowkey!r} must be '<entry>|<finding_key>'")
    entry, key = rowkey.split("|", 1)
    if result not in RESULTS:
        raise ValueError(f"result must be one of {', '.join(RESULTS)}, not {result!r}")
    evidence = " ".join(evidence.split())
    judge = " ".join(judge.split())
    if not evidence:
        raise ValueError("evidence is empty")
    if not judge:
        raise ValueError("judge is empty")
    with _Lock(root / (str(SPOT_FILE) + ".lock")):
        verdicts = effective_verdicts(root)
        row = verdicts.get((entry, key))
        if row is None or row[2] != "false" or not is_selected(entry, key):
            raise ValueError(f"{rowkey!r} is not a due row (false verdict in the 1-in-{SAMPLE_MOD} sample)")
        if (entry, key) in recorded(root):
            raise ValueError(f"{rowkey!r} already has a spot-check result")
        jf = judge_family(judge)
        if not jf:
            raise ValueError(f"judge {judge!r} has an unknown model family (known: {', '.join(FAMILIES)}, orchestrator): refused")
        orig = judge_family(row[4])
        if not orig:
            raise ValueError(f"the original judge {row[4]!r} of {rowkey!r} has an unknown model family: cannot check independence")
        banned = {orig}
        for x in exclude:
            xf = family(x)
            if not xf:
                raise ValueError(f"--exclude {x!r} has an unknown model family: refused")
            banned.add(xf)
        if jf in banned:
            raise ValueError(f"judge {judge!r} (family {jf}) is in the family of the original judge, the gating reviewer or the coder: pick an independent judge")
        out = "\t".join([entry, key, result, judge, evidence, date or time.strftime("%Y-%m-%d", time.gmtime())])
        path = root / SPOT_FILE
        new = not path.is_file() or path.stat().st_size == 0
        with open(path, "a", encoding="utf-8", newline="") as f:
            f.write((SPOT_HEADER + "\n" if new else "") + out + "\n")
    return out


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=".", help="the Broadsword root (holds state/)")
    ap.add_argument("--check", action="store_true", help="exit 1 while any due row has no spot-check result")
    ap.add_argument("--record", nargs=4, metavar=("ROW_KEY", "RESULT", "JUDGE", "EVIDENCE"), help="append a spot-check result")
    ap.add_argument("--exclude", action="append", default=[], metavar="MODEL", help="with --record: also refuse this model (coder, gating reviewer)")
    args = ap.parse_args(argv)
    root = Path(args.root)
    try:
        if args.record:
            out = record(root, *args.record, exclude=args.exclude)
            print(f"recorded: {out}")
            return 0
        due, rows = due_rows(root), open_rows(root)
    except ValueError as e:  # Malformed is a ValueError
        print(f"refused: {e}", file=sys.stderr)
        return 2
    print(f"{len(rows)} of {len(due)} due spot check(s) open")
    for c in rows:
        print(f"{c[0]}|{c[1]}\t{c[4]}\t{c[3]}")
    return 1 if args.check and rows else 0


if __name__ == "__main__":
    sys.exit(main())
