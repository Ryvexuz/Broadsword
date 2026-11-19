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
Runs made with run.py --attempts N (N > 1) have cost_usd_total (sum over all attempts) and per-attempt data. Then the cost
and time columns are the sums over all attempts, and a second table ranks the models by cost to pass, not cost per call:
  pass@1    tasks fully passed by the first attempt / tasks
  passed    tasks fully passed within the attempts / tasks
  tries     the tries a task needed ("1", "2", ...) or "no pass"; a cell is "<tries> $<cost of all its attempts>"
  cost-to-pass  cost of all attempts of the tasks that passed; "no pass at cost X" for the tasks that never passed
  Rows are sorted by tasks without a pass (fewest first), then models whose cost is known before models with any
  unknown or plan-allowance cost (their sum is only a lower bound), then by total cost. A cost that is n/a for some attempt is
  shown with "+": the sum of the known ones (never a 0).
"""
import json, pathlib, sys


def task_cost(r):
    """(cost over all attempts or None, how many attempts had an unknown cost)."""
    if "cost_usd_total" in r:
        return r["cost_usd_total"], r.get("cost_unknown_attempts", 0)
    return r["cost_usd"], 0 if r["cost_usd"] is not None else 1


def usd(known, unknown):
    if not known and unknown:
        return "n/a"
    return f"${sum(known):.3f}" + ("+" if unknown else "")


def attempts_table(groups, tasks):
    """The cost-to-pass table, or None when no record has more than one attempt allowed."""
    rows = []
    for (prov, model, effort), by_task in groups.items():
        if not any(r.get("max_attempts", 1) > 1 for r in by_task.values()):
            continue
        cells, p1, passed, pc, pu, npc, npu, nopass = [], 0, 0, [], 0, [], 0, 0
        for t in tasks:
            r = by_task.get(t)
            if r is None:
                cells.append("-")
                continue
            if r.get("graded") is False:
                cells.append("ungraded")
                continue
            c, u = task_cost(r)
            at = r.get("pass_at")
            if at == 1:
                p1 += 1
            if at:
                passed += 1
                pc += [c] if c is not None else []
                pu += u + (1 if c is None and not u else 0)
                cells.append(f"{at} {usd([c] if c is not None else [], u)}")
            else:
                nopass += 1
                npc += [c] if c is not None else []
                npu += u + (1 if c is None and not u else 0)
                cells.append(f"no pass {usd([c] if c is not None else [], u)}")
        n = len(by_task)
        tail = usd(pc, pu) if passed else "-"
        if nopass:
            tail += f"; no pass at cost {usd(npc, npu)} ({nopass})"
        total = sum(pc) + sum(npc)
        unknown = bool(pu or npu)  # an unknown or plan-allowance cost is never $0: such models rank after the known ones
        rows.append(((nopass, unknown, total), [f"{prov}:{model}", effort, f"{p1}/{n}", f"{passed}/{n}"] + cells + [tail]))
    if not rows:
        return None
    head = ["model", "effort", "pass@1", "passed"] + [t.split("-")[0] + "-" + t.split("-", 1)[1][:8] for t in tasks] + ["cost-to-pass"]
    return [head] + [r for _, r in sorted(rows, key=lambda x: x[0])]


def printable(rows):
    widths = [max(len(str(r[i])) for r in rows) for i in range(len(rows[0]))]
    for r in rows:
        print("  ".join(str(c).ljust(w) for c, w in zip(r, widths)))


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
        tcs = [task_cost(r) for r in by_task.values()]
        known = [c for c, _ in tcs if c is not None]
        missing = sum(1 for c, u in tcs if c is None or u)  # tasks with any attempt of unknown cost
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
                     cost, f"{sum(r.get('wall_s_total', r['wall_s']) for r in by_task.values()):.0f}s"])
    widths = [max(len(str(r[i])) for r in rows + [head]) for i in range(len(head))]
    for r in [head] + rows:
        print("  ".join(str(c).ljust(w) for c, w in zip(r, widths)))
    table = attempts_table(groups, tasks)
    if table:
        print()
        print("cost to pass (all attempts; ranked by tasks without a pass, then known-cost models before models with an "
              "unknown or plan cost, then cost; a + is a lower bound):")
        printable(table)


if __name__ == "__main__":
    main()
