"""Print a drift run's per-step table and its summary from the raw results (steps.jsonl and meta.json), re-scoring them.

Usage: python -I -B bench/drift/score.py <results dir> [<results dir> ...]
"""
import json
import pathlib
import sys

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import checks  # noqa: E402

SHORT = {r: f"R{i + 1}" for i, r in enumerate(checks.RULES)}


def load(run_dir):
    run_dir = pathlib.Path(run_dir)
    meta = json.loads((run_dir / "meta.json").read_text(encoding="utf-8"))
    lines = (run_dir / "steps.jsonl").read_text(encoding="utf-8").splitlines()
    records = [json.loads(ln) for ln in lines if ln.strip()]
    if not records:
        raise checks.ScoreError(f"{run_dir}: steps.jsonl is empty")
    return meta, records


def table(meta, records):
    cw = meta["context_window"]
    out = [f"run {meta['run_id']}  client {meta['client']}  model {meta['model']}  fake {meta['fake']}",
           "step | " + " | ".join(SHORT.values()) + " | hidden | ctx tokens | ctx share"]
    for r in records:
        if r.get("error"):
            out.append(f"{r['step']:>4} | ERROR: {r['error']}")
            continue
        h = r["hidden"]
        hid = "?" if h["passed"] is None else f"{h['passed']}/{h['total']}"
        tok = r.get("context_tokens")
        out.append(f"{r['step']:>4} | " + " | ".join("ok" if r["rules"][x]["holds"] else "BREAK" for x in checks.RULES)
                   + f" | {hid} | {tok if tok is not None else '?'} | {'?' if tok is None else f'{tok / cw:.1%}'}")
    return out


def summary_lines(s):
    out = [f"steps scored {s['steps_scored']}/{s['step_count']}  complete {s['complete']}  work score {s['work_score']}"
           + (f"  ERROR at step {s['error']['step']}: {s['error']['error']}" if s["error"] else "")]
    for rule, r in s["rules"].items():
        fb = r["first_break"]
        share = r["context_share_at_break"]
        out.append(f"{rule}: first break {fb if fb else 'never'}"
                   + (f" (context {'?' if share is None else f'{share:.1%}'}; {r['first_break_detail']})" if fb else "")
                   + f"; breaks by window {r['breaks_by_window']}")
    return out


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    if not argv:
        print(__doc__, file=sys.stderr)
        return 2
    for run_dir in argv:
        try:
            meta, records = load(run_dir)
            s = checks.summarize(records, meta["checkpoints"], meta["step_count"], meta["context_window"])
        except (OSError, ValueError, KeyError, checks.ScoreError) as e:
            print(f"{run_dir}: cannot score: {e}", file=sys.stderr)
            return 2
        print("\n".join(table(meta, records) + summary_lines(s)))
    return 0


if __name__ == "__main__":
    sys.exit(main())
