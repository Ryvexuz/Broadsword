"""Score benchmark results: did each review flag the known bug as BLOCKING?

Usage: python -I -B score.py <out-dir>
- caught: a BLOCKING line matches the case's `catch` pattern.
- minor_only: the bug only appears in a MINOR line, which counts as noticed but under-rated.
- known_fp: a BLOCKING line matches a known false positive.
- other_blocking: BLOCKING lines matching neither. These can be real extra bugs or false positives,
  and are checked by hand.
"""
import json, pathlib, re, sys
from collections import defaultdict

HERE = pathlib.Path(__file__).resolve().parent
cases = {c["id"]: c for c in json.loads((HERE / "cases.json").read_text(encoding="utf-8"))}
out = pathlib.Path(sys.argv[1])
rows = []
for f in sorted(out.glob("*.json")):
    if f.name.endswith(".raw.json"):
        continue
    r = json.loads(f.read_text(encoding="utf-8"))
    c = cases[r["case"]]
    text = r.get("result") or ""
    lines = [l.strip() for l in text.splitlines()]
    blocking = [l for l in lines if l.upper().startswith("BLOCKING:") and "none" not in l.lower()[:16]]
    minor = [l for l in lines if l.upper().startswith("MINOR:") and "none" not in l.lower()[:13]]
    verdict = next((l.split(":", 1)[1].strip() for l in lines if l.upper().startswith("VERDICT:")), "?")
    cat = re.compile(c["catch"])
    fps = [re.compile(p) for p in c["known_false_positives"]]
    caught = [l for l in blocking if cat.search(l)]
    known_fp = [l for l in blocking if not cat.search(l) and any(p.search(l) for p in fps)]
    other = [l for l in blocking if not cat.search(l) and not any(p.search(l) for p in fps)]
    minor_hit = [l for l in minor if cat.search(l)]
    u = r.get("usage") or {}
    tok_in = (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0)
    rows.append(dict(case=r["case"], model=r["model"], effort=r["effort"], verdict=verdict, caught=bool(caught),
                     minor_only=bool(minor_hit) and not caught, known_fp=len(known_fp), other=len(other),
                     n_block=len(blocking), cost=r.get("cost_usd") or 0, wall=r["wall_s"], tin=tok_in,
                     tout=u.get("output_tokens") or 0, err=r.get("is_error") or r.get("exit") != 0))

print(f"{'case':<9} {'model':<7} {'effort':<8} {'verdict':<8} {'caught':<7} {'minor':<6} {'kFP':<4} {'other':<6} {'cost$':<7} {'secs':<6} {'in_tok':<7} {'out_tok'}")
for x in sorted(rows, key=lambda x: (x["case"], x["model"], x["effort"])):
    print(f"{x['case']:<9} {x['model']:<7} {x['effort']:<8} {x['verdict'][:7]:<8} {'YES' if x['caught'] else 'no':<7} "
          f"{'yes' if x['minor_only'] else '':<6} {x['known_fp']:<4} {x['other']:<6} {x['cost']:<7.3f} {x['wall']:<6} {x['tin']:<7} {x['tout']}"
          + ("  ERROR" if x["err"] else ""))
print()
agg = defaultdict(lambda: dict(n=0, caught=0, minor=0, kfp=0, other=0, cost=0.0, wall=0.0))
for x in rows:
    a = agg[(x["model"], x["effort"])]
    a["n"] += 1; a["caught"] += x["caught"]; a["minor"] += x["minor_only"]; a["kfp"] += x["known_fp"]
    a["other"] += x["other"]; a["cost"] += x["cost"]; a["wall"] += x["wall"]
print(f"{'model':<7} {'effort':<8} {'caught':<8} {'minor-only':<11} {'known FP':<9} {'other blk':<10} {'cost $':<8} {'avg secs'}")
for (m, e), a in sorted(agg.items()):
    print(f"{m:<7} {e:<8} {a['caught']}/{a['n']:<6} {a['minor']:<11} {a['kfp']:<9} {a['other']:<10} {a['cost']:<8.3f} {a['wall']/a['n']:.0f}")
