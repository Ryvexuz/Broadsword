"""Totals the GPT spend ledger per call_type, split plan against credits.

    python -I -B scripts/usage_sum.py --state-dir <state folder> [--date YYYY-MM-DD]

Reads <state-dir>/gpt-spend/<date>.tsv (default: today, UTC) and prints one line per (call_type, plan|credits) with the call
count and the summed est_usd, then a total. Rows from before the call_type column (7 columns) count as "unknown". A row that
cannot be read (wrong column count, bad number, unknown plan_or_credits label) is an error, never skipped: exit 2.
Read only: it never writes or rewrites a ledger.
"""

import sys

sys.dont_write_bytecode = True

import argparse
import datetime
import re
from pathlib import Path


def split_label(label: str) -> str:
    if label.startswith("codex-plan"):
        return "plan"
    if label.startswith("api-credits"):
        return "credits"
    raise ValueError(f"unknown plan_or_credits label {label!r}")


def sum_ledger(path: Path) -> dict:
    """{(call_type, "plan"|"credits"): [calls, est_usd]}. Raises ValueError naming the line of a row that cannot be read."""
    totals = {}
    for n, line in enumerate(path.read_text(encoding="utf-8").split("\n"), 1):
        if not line.strip() or line.startswith("utc\t"):
            continue
        cols = line.rstrip("\r").split("\t")
        try:
            if len(cols) not in (7, 8):
                raise ValueError("wrong number of columns")
            usd = float(cols[5])
            if usd != usd or usd in (float("inf"), float("-inf")):
                raise ValueError("est_usd is not finite")
            kind = split_label(cols[6])
            call_type = (cols[7].strip() if len(cols) == 8 else "") or "unknown"
        except ValueError as e:
            raise ValueError(f"{path.name} line {n}: unreadable row ({e})")
        slot = totals.setdefault((call_type, kind), [0, 0.0])
        slot[0] += 1
        slot[1] += usd
    return totals


def main(argv=None) -> int:
    p = argparse.ArgumentParser(description="Sum the GPT spend ledger per call_type, plan against credits.")
    p.add_argument("--state-dir", required=True)
    p.add_argument("--date", default=None, help="UTC date YYYY-MM-DD (default today)")
    args = p.parse_args(argv)
    day = args.date or datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", day):
        print(f"error: --date must be YYYY-MM-DD, got {day!r}")
        return 2
    ledger = Path(args.state_dir) / "gpt-spend" / f"{day}.tsv"
    if not ledger.is_file():
        print(f"error: no ledger for {day}: {ledger}")
        return 2
    try:
        totals = sum_ledger(ledger)
    except (ValueError, OSError) as e:
        print(f"error: {e}")
        return 2
    print(f"date {day}")
    print("call_type\tsource\tcalls\test_usd")
    for (call_type, kind), (calls, usd) in sorted(totals.items()):
        print(f"{call_type}\t{kind}\t{calls}\t{usd:.6f}")
    print(f"total\tall\t{sum(v[0] for v in totals.values())}\t{sum(v[1] for v in totals.values()):.6f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
