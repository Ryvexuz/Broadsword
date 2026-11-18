"""Prove the benchmark is sound: run every task's reference.rs and wrong/*.rs through the harness.

Usage:  python -I -B prove.py [task ...]      (default: every task)

Rules, no exceptions:
  - the reference must compile and pass every hidden test;
  - every wrong solution must compile and fail at least one hidden test (one that does not even
    compile is not a valid wrong solution: the tests must really bite).
Every solution is graded in one Broadsword job (see harness.py). Prints the table and exits 0 only if
every rule holds for every task.
"""
import importlib.util, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("coder_harness", HERE / "harness.py")
H = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(H)


def main():
    want = set(sys.argv[1:])
    tasks = [d for d in sorted((HERE / "tasks").iterdir()) if d.is_dir() and (not want or d.name in want)]
    items, labels, ok = [], [], True
    for t in tasks:
        wrongs = sorted((t / "wrong").glob("*.rs"))
        if len(wrongs) < 2:
            print(f"{t.name}: needs at least two wrong solutions", file=sys.stderr)
            ok = False
        items.append((t / "reference.rs", t)); labels.append("reference")
        for w in wrongs:
            items.append((w, t)); labels.append(f"wrong/{w.stem}")
    results = H.grade_batch(items)
    rows = []
    for (path, t), label, r in zip(items, labels, results):
        if label == "reference":
            good = r["compiles"] and r["total"] > 0 and r["passed"] == r["total"]
            verdict = "PASS (all tests)" if good else "BROKEN: reference does not pass everything"
        else:
            good = r["compiles"] and r["failed"] >= 1
            verdict = "caught" if good else ("INVALID: does not compile" if not r["compiles"] else "MISSED: passes every test")
        ok = ok and good
        rows.append((t.name, label, "yes" if r["compiles"] else "NO", f"{r['passed']}/{r['total']}", f"{r['wall_s']}s", verdict))
        if label == "reference" and not good:
            print(f"{t.name} reference failing tests: {r.get('failed_tests')}\n{r.get('failure_detail')}", file=sys.stderr)
        if not r["compiles"]:
            print(f"{t.name} {label} first error: {r['first_error']}", file=sys.stderr)
    head = ("task", "solution", "compiles", "passed", "time", "verdict")
    widths = [max(len(str(x[i])) for x in rows + [head]) for i in range(len(head))]
    for r in [head] + rows:
        print("  ".join(str(c).ljust(w) for c, w in zip(r, widths)))
    refs = [r for r in rows if r[1] == "reference"]
    wrongs = [r for r in rows if r[1] != "reference"]
    print(f"\nreferences passing every test: {sum(1 for r in refs if r[5].startswith('PASS'))} of {len(refs)}")
    print(f"wrong solutions failing at least one test: {sum(1 for r in wrongs if r[5] == 'caught')} of {len(wrongs)}")
    print("PROOF: " + ("OK" if ok else "FAILED"))
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
