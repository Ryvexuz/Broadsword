"""Compare the gating review with the shadow review for every queue entry that has both.

Run:  python -I -B scripts/shadow_report.py [--root <folder with broadsword.toml>]

Reads <root>/state/queue/*.entry (the `key = value` records the queue writes). Prints one row per entry with both verdicts,
whether they agree, the BLOCKING lines one model found that the other did not (by file:line, else by text), the cost and the
time of the shadow review, and ends with the agreement rate over entries where the shadow gave a real PASS or FAIL.
Read-only; never calls a model.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import re
from pathlib import Path


def parse_entry(text: str) -> dict:
    """The queue's record format: `key = value` lines; a value with newlines is stored with `\\n` escapes."""
    rec = {}
    for line in text.splitlines():
        if " = " in line:
            k, _, v = line.partition(" = ")
        elif line.endswith(" ="):
            k, v = line[:-2], ""
        else:
            continue
        rec[k.strip()] = v.replace("\\n", "\n").replace("\\\\", "\\")
    return rec


def finding_key(line: str) -> str:
    """`src/a.rs:12 - text` is identified by its file:line; a finding without one by its normalised text."""
    m = re.match(r"\s*([\w./\\-]+:\d+)", line)
    return m.group(1).lower() if m else re.sub(r"\W+", " ", line).strip().lower()


def findings(rec: dict, key: str) -> dict:
    return {finding_key(l): l.strip() for l in rec.get(key, "").splitlines() if l.strip()}


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=".", help="the Broadsword root (holds broadsword.toml and state/)")
    args = ap.parse_args(argv)
    qdir = Path(args.root) / "state" / "queue"
    entries = []
    for p in sorted(qdir.glob("*.entry")) if qdir.is_dir() else []:
        try:
            rec = parse_entry(p.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
        if rec.get("shadow_verdict"):
            entries.append((p.stem, rec))
    if not entries:
        print("no queue entry has a shadow review yet")
        return 0
    print("entry\tstatus\tgating\tshadow\tagree\tshadow_cost_usd\tshadow_secs\tshadow_model_used")
    agree = compared = 0
    details = []
    for eid, r in entries:
        g, s = r.get("verdict", "") or "-", r["shadow_verdict"]
        real = g in ("PASS", "FAIL") and s in ("PASS", "FAIL")
        same = "yes" if real and g == s else "NO" if real else "n/a"
        compared += real
        agree += real and g == s
        print(f"{eid}\t{r.get('status', '')}\t{g}\t{s}\t{same}\t{r.get('shadow_cost_usd') or '-'}\t{r.get('shadow_secs') or '-'}\t{r.get('shadow_model_used') or '-'}")
        gb, sb = findings(r, "blocking"), findings(r, "shadow_blocking")
        only_g = [v for k, v in gb.items() if k not in sb]
        only_s = [v for k, v in sb.items() if k not in gb]
        if only_g or only_s or s in ("ERROR", "SKIPPED"):
            details.append((eid, only_g, only_s, r.get("shadow_reason", "") if s in ("ERROR", "SKIPPED") else ""))
    for eid, only_g, only_s, why in details:
        print(f"\n{eid}")
        if why:
            print(f"  shadow did not review: {why}")
        for l in only_g:
            print(f"  BLOCKING only in the gating review: {l}")
        for l in only_s:
            print(f"  BLOCKING only in the shadow review: {l}")
    rate = f"{agree}/{compared} = {100 * agree / compared:.0f}%" if compared else "n/a (no entry has a real verdict from both)"
    print(f"\nentries with a shadow record: {len(entries)}; compared (both PASS or FAIL): {compared}; agreement rate: {rate}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
