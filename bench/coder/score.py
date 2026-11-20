"""Print the score table for a results folder written by run.py.

Usage:  python -I -B score.py <out-dir>

One row per provider, model and effort. One column per task (passed/total). Then:
  score     passed tests over all tests, as a percentage (a task that is missing counts as not run: shown "-", not in the sums)
  compile   graded tasks that did not produce a compiling solution (compile errors and format failures; not ungraded
            tasks, not failed model calls)
  call      provider errors (the "errored" runs): runs whose model call failed (HTTP error such as 429 or no credits, CLI error, timeout;
            is_error / call_failed). They are NOT results: they are left out of the percentage, the compile, format,
            cost-n/a and time columns, and listed below the table with their error text. A task that errored shows
            "error" in its cell; a model with only errored runs shows "no valid runs" as its score.
            Older result files have no is_error field and recorded a refused call as format_failure (a 0%): a
            format_failure row whose first_error is "model call failed: ..." or whose raw output is a provider error is
            re-labelled as an error when scoring (the file is not changed), and the count is printed.
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


def raw_provider_error(raw_path):
    """The error text if the saved raw output of a call is a provider error (the CLI's JSON with is_error true, or the
    openai provider's {"error": ...}), else None. A missing or unreadable raw file is not an error."""
    try:
        raw = json.loads(raw_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(raw, dict):
        return None
    if raw.get("is_error") is True:
        return str(raw.get("error") or raw.get("result") or "(no error text)")[:500]
    if isinstance(raw.get("error"), str) and raw["error"]:
        return raw["error"][:500]
    return None


def classify(r, raw_path):
    """Marks one record: r["_err"] is the error text of an errored run (else None), r["_relabelled"] is True when an old
    format_failure row was re-labelled as a provider error. An old record has no is_error key."""
    r["_relabelled"] = False
    if r.get("is_error") is True or r.get("call_failed") is True:
        r["_err"] = str(r.get("error") or r.get("first_error") or "(no error text)")
    elif "is_error" not in r and r.get("format_failure"):
        fe = r.get("first_error") or ""
        text = fe.split("model call failed:", 1)[1].strip() if fe.startswith("model call failed:") else raw_provider_error(raw_path)
        r["_err"] = text
        r["_relabelled"] = text is not None
    else:
        r["_err"] = None
    return r


def task_cost(r):
    """(cost over all attempts or None, how many attempts had an unknown cost)."""
    if "cost_usd_total" in r:
        return r["cost_usd_total"], r.get("cost_unknown_attempts", 0)
    return r["cost_usd"], 0 if r["cost_usd"] is not None else 1


def pass_at(r):
    """The attempt that first fully passed, or None. A one-shot record (no pass_at field) that fully passed is a pass at 1."""
    if "pass_at" in r:
        return r["pass_at"]
    ok = (r.get("graded") is not False and r.get("compiles") and r.get("total") and r.get("passed") == r.get("total")
          and not r.get("call_failed") and not r.get("format_failure"))
    return 1 if ok else None


def usd(known, unknown, plan=False):
    """plan: a Codex (plan allowance) call has tokens but no price: shown as "plan", never $0 and never plain n/a."""
    if not known and unknown:
        return "plan" if plan else "n/a"
    return f"${sum(known):.3f}" + ("+" if unknown else "")


def attempts_table(groups, tasks):
    """The cost-to-pass table, or None when no record has more than one attempt allowed."""
    rows = []
    for (prov, model, effort), by_task in groups.items():
        if not any(r.get("max_attempts", 1) > 1 for r in by_task.values()):
            continue
        cells, p1, passed, pc, pu, npc, npu, nopass, ungraded, errs = [], 0, 0, [], 0, [], 0, 0, 0, 0
        ec = []
        uc, uu = [], 0
        plan = prov == "codex"
        for t in tasks:
            r = by_task.get(t)
            if r is None:
                cells.append("-")
                continue
            c, u = task_cost(r)
            if r["_err"] is not None:
                # not a result: no pass and no failure. Its cost is real money, so a known cost is still summed.
                errs += 1
                ec += [c] if c is not None else []
                cells.append("error")
                continue
            if r.get("graded") is False:
                # no pass has been shown, and its calls cost something: it counts as unresolved, never as free
                ungraded += 1
                uc += [c] if c is not None else []
                uu += u + (1 if c is None and not u else 0)
                cells.append("ungraded")
                continue
            at = pass_at(r)
            if at == 1:
                p1 += 1
            if at:
                passed += 1
                pc += [c] if c is not None else []
                pu += u + (1 if c is None and not u else 0)
                cells.append(f"{at} {usd([c] if c is not None else [], u, plan)}")
            else:
                nopass += 1
                npc += [c] if c is not None else []
                npu += u + (1 if c is None and not u else 0)
                cells.append(f"no pass {usd([c] if c is not None else [], u, plan)}")
        n = len(by_task) - errs
        if not n:
            rows.append(((10 ** 9, True, 0), [f"{prov}:{model}", effort, "-", "-"] + cells + [f"no valid runs ({errs} errored)"]))
            continue
        tail = usd(pc, pu, plan) if passed else "-"
        if nopass:
            tail += f"; no pass at cost {usd(npc, npu, plan)} ({nopass})"
        if errs:
            tail += f"; {errs} errored (excluded)"
        if ungraded:
            tail += f"; {ungraded} ungraded (cost {usd(uc, uu, plan)} so far, not counted as a pass)"
        total = sum(pc) + sum(npc) + sum(uc) + sum(ec)
        unknown = bool(pu or npu or uu)  # an unknown or plan-allowance cost is never $0: such models rank after the known ones
        rows.append(((nopass + ungraded, unknown, total), [f"{prov}:{model}", effort, f"{p1}/{n}", f"{passed}/{n}"] + cells + [tail]))
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
        recs.append(classify(json.loads(f.read_text(encoding="utf-8")), f.with_suffix(".raw.json")))
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
        valid = {t: r for t, r in by_task.items() if r["_err"] is None}
        errored = [r for r in by_task.values() if r["_err"] is not None]
        for t in tasks:
            r = by_task.get(t)
            if r is None:
                cells.append("-")
                continue
            if r["_err"] is not None:
                cells.append("error")
                continue
            if r.get("graded") is False:
                cells.append("ungraded")
                continue
            cells.append(f"{r['passed']}/{r['total']}")
            passed += r["passed"]; total += r["total"]
        tcs = [task_cost(r) for r in valid.values()]
        # an errored run's known cost is real money and is summed; an errored run without a cost is not "n/a" (it never ran)
        known = [c for c, _ in tcs if c is not None] + [c for c, _ in (task_cost(r) for r in errored) if c is not None]
        missing = sum(1 for c, u in tcs if c is None or u)  # tasks with any attempt of unknown cost
        if not known:
            cost = "plan" if prov == "codex" else "n/a"
        elif missing:
            cost = f"${sum(known):.3f}+ ({missing} n/a)"
        else:
            cost = f"${sum(known):.3f}"
        rows.append([f"{prov}:{model}", effort] + cells +
                    [f"{passed}/{total} {100 * passed / total:.0f}%" if total else ("no valid runs" if errored and not valid else "-"),
                     str(sum(1 for r in valid.values() if r.get("graded") is not False and not r["compiles"])),
                     str(len(errored)),
                     str(sum(1 for r in valid.values() if r["format_failure"])),
                     cost, f"{sum(r.get('wall_s_total', r['wall_s']) for r in valid.values()):.0f}s"])
    widths = [max(len(str(r[i])) for r in rows + [head]) for i in range(len(head))]
    for r in [head] + rows:
        print("  ".join(str(c).ljust(w) for c, w in zip(r, widths)))
    table = attempts_table(groups, tasks)
    if table:
        print()
        print("cost to pass (all attempts; ranked by tasks without a pass, then known-cost models before models with an "
              "unknown or plan cost, then cost; a + is a lower bound):")
        printable(table)
    errored = [(k, r) for k, by_task in sorted(groups.items()) for _, r in sorted(by_task.items()) if r["_err"] is not None]
    if errored:
        relabelled = sum(1 for _, r in errored if r["_relabelled"])
        print()
        print(f"errored runs ({len(errored)}; provider errors, not results; left out of the percentages above):")
        for (prov, model, effort), r in errored:
            print(f"  {r['task']} {prov}:{model} {effort}: {r['_err']}")
        if relabelled:
            print(f"{relabelled} old format_failure row(s) re-labelled as provider errors (their raw output or first_error is a "
                  f"provider error; the files are unchanged)")


if __name__ == "__main__":
    main()
