"""Print the score table for a results folder written by run.py.

Usage:  python -I -B score.py <out-dir>

One row per provider, model and effort. One column per task (passed/total). Then:
  score     passed tests over all tests, as a percentage (a task that is missing counts as not run: shown "-", not in the sums)
  compile   graded tasks that did not produce a compiling solution (compile errors and format failures; not ungraded
            tasks, not failed model calls)
  call      tasks whose model call failed or timed out (call_failed)
  format    tasks where the answer did not hold exactly one code block
  cost      sum of the CLI's cost in USD. If some tasks have no cost (a failed or timed-out call), the sum of the
            others with a "+" and the count: "$1.234+ (2 n/a)"; "n/a" only if no task has a cost
  A task whose call was made but not graded (graded=false, see run.py regrade) shows "ungraded" and is not in the sums.
  time      sum of model-call seconds
"""
import json, pathlib, sys


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    recs = []
    for f in sorted(pathlib.Path(sys.argv[1]).glob("*.json")):
        if f.name.endswith(".raw.json"):
            continue
        recs.append(json.loads(f.read_text(encoding="utf-8")))
    if not recs:
        sys.exit("no result files found")
    tasks = sorted({r["task"] for r in recs})
    groups = {}
    for r in recs:
        groups.setdefault((r["provider"], r["model"], r["effort"]), {})[r["task"]] = r
    head = ["model", "effort"] + [t.split("-")[0] + "-" + t.split("-", 1)[1][:8] for t in tasks] + \
           ["score", "compile", "call", "format", "cost", "time"]
    rows = []
    for (prov, model, effort), by_task in sorted(groups.items()):
        cells, passed, total = [], 0, 0
        for t in tasks:
            r = by_task.get(t)
            if r is None:
                cells.append("-")
                continue
            if r.get("graded") is False:
                cells.append("ungraded")
                continue
            cells.append(f"{r['passed']}/{r['total']}")
            passed += r["passed"]; total += r["total"]
        costs = [r["cost_usd"] for r in by_task.values()]
        known = [c for c in costs if c is not None]
        missing = len(costs) - len(known)
        if not known:
            cost = "n/a"
        elif missing:
            cost = f"${sum(known):.3f}+ ({missing} n/a)"
        else:
            cost = f"${sum(known):.3f}"
        rows.append([f"{prov}:{model}", effort] + cells +
                    [f"{passed}/{total} {100 * passed / total:.0f}%" if total else "-",
                     str(sum(1 for r in by_task.values() if r.get("graded") is not False and not r["compiles"]
                             and not r.get("call_failed"))),
                     str(sum(1 for r in by_task.values() if r.get("call_failed"))),
                     str(sum(1 for r in by_task.values() if r["format_failure"])),
                     cost, f"{sum(r['wall_s'] for r in by_task.values()):.0f}s"])
    widths = [max(len(str(r[i])) for r in rows + [head]) for i in range(len(head))]
    for r in [head] + rows:
        print("  ".join(str(c).ljust(w) for c, w in zip(r, widths)))


if __name__ == "__main__":
    main()
