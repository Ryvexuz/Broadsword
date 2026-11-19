"""Score instruction-following results mechanically.

Usage: python -I -B score.py <out-dir>
Reads the <case>__<provider>__<model>__<effort>.json files run.py wrote. Per result:
  PASS   every check of the case holds
  FAIL   at least one check broke; the broken rules are printed (an empty answer with no error is a FAIL)
  ERROR  the model call failed (provider refusal, timeout, CLI error, no answer). Excluded from every total, listed
         separately: an error says nothing about instruction following. Its cost still counts in cost $.
Totals per (provider, model, effort): scored = PASS + FAIL, pass rate = PASS / scored, plus errors and cases not run.
"""
import importlib.util, json, pathlib, sys
from collections import defaultdict

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("instruct_checks", HERE / "checks.py")
CK = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(CK)


def load_cases():
    return {c["id"]: c for c in json.loads((HERE / "cases.json").read_text(encoding="utf-8"))}


def classify(rec, case):
    """Returns ('PASS'|'FAIL'|'ERROR', [broken rules as (type, message)], error text)."""
    # call_failed is decided once, in run.py (call_failed()); whatever text the failed call carried, it is never graded
    if rec["call_failed"]:
        return "ERROR", [], str(rec.get("error") or "model call failed")
    broken = CK.grade(case, rec.get("answer") or "")
    return ("FAIL" if broken else "PASS"), broken, ""


def score_dir(out, cases):
    rows = []
    for f in sorted(out.glob("*.json")):
        if f.name.endswith(".raw.json"):
            continue
        rec = json.loads(f.read_text(encoding="utf-8"))
        if rec["case"] not in cases:
            sys.exit(f"{f.name}: unknown case {rec['case']!r}")
        status, broken, err = classify(rec, cases[rec["case"]])
        rows.append(dict(case=rec["case"], key=(rec["provider"], rec["model"], rec["effort"]), status=status,
                         broken=broken, error=err, cost=rec.get("cost_usd") or 0, wall=rec.get("wall_s") or 0))
    return rows


def totals(rows, cases):
    agg = defaultdict(lambda: dict(pass_=0, fail=0, errors=0, cost=0.0, wall=0.0, seen=set()))
    for r in rows:
        a = agg[r["key"]]
        a["cost"] += r["cost"]
        a["seen"].add(r["case"])
        if r["status"] == "ERROR":
            a["errors"] += 1
            continue
        a["wall"] += r["wall"]
        a["pass_" if r["status"] == "PASS" else "fail"] += 1
    out = {}
    for k, a in agg.items():
        scored = a["pass_"] + a["fail"]
        out[k] = dict(passed=a["pass_"], failed=a["fail"], scored=scored, errors=a["errors"],
                      rate=(a["pass_"] / scored) if scored else None, cost=a["cost"], wall=a["wall"],
                      not_run=len(cases) - len(a["seen"]))
    return out


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    cases = load_cases()
    rows = score_dir(pathlib.Path(sys.argv[1]), cases)
    if not rows:
        sys.exit("no result files in that folder")
    for r in sorted(rows, key=lambda r: (r["key"], r["case"])):
        why = "; ".join(f"{t}: {m}" for t, m in r["broken"]) if r["status"] == "FAIL" else r["error"][:150]
        print(f"{r['case']:<24} {'/'.join(r['key']):<40} {r['status']:<6} {why}")
    print()
    print(f"{'provider/model/effort':<40} {'pass':<5} {'fail':<5} {'scored':<7} {'rate':<7} {'errors':<7} {'not run':<8} {'cost$':<8} {'avg s'}")
    for k, t in sorted(totals(rows, cases).items()):
        rate = "n/a" if t["rate"] is None else f"{t['rate'] * 100:.0f}%"
        avg = f"{t['wall'] / t['scored']:.1f}" if t["scored"] else "n/a"
        print(f"{'/'.join(k):<40} {t['passed']:<5} {t['failed']:<5} {t['scored']:<7} {rate:<7} {t['errors']:<7} {t['not_run']:<8} {t['cost']:<8.4f} {avg}")


if __name__ == "__main__":
    main()
