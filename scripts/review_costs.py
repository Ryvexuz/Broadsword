"""Summarise what merge-queue reviews cost and caught, from state/queue/*.review, for the retro's cost section.

  python -I scripts/review_costs.py                  every review
  python -I scripts/review_costs.py --since 20261118  reviews whose entry id starts on or after that date (YYYYMMDD)

Coder tokens are not recorded by Broadsword; the orchestrator notes them from each agent's report.
"""
import pathlib
import subprocess
import sys
from collections import defaultdict


def main_root() -> pathlib.Path:
    """The main checkout, where state/ lives, even when this script runs from a task worktree.
    Exits loudly if it can't be found: a report read from the wrong folder would be silently wrong."""
    here = pathlib.Path(__file__).resolve().parent
    try:
        common = subprocess.run(["git", "-C", str(here), "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        raise SystemExit(f"review_costs: cannot run git to find the main checkout ({e})")
    root = pathlib.Path(common).parent if common else None
    if root is None or not (root / "broadsword.toml").is_file() or not (root / "state" / "queue").is_dir():
        raise SystemExit(f"review_costs: no Broadsword main checkout with state/queue found (git said {common!r})")
    return root


def header(text: str) -> dict:
    out = {}
    for line in text.split("--- raw reviewer output", 1)[0].splitlines():
        k, sep, v = line.partition(": ")
        if sep and not line.startswith(" "):
            out[k.strip()] = v.strip()
    return out


def main() -> int:
    since = None
    if "--since" in sys.argv:
        since = sys.argv[sys.argv.index("--since") + 1]
    root = main_root()
    rows = []
    for f in sorted((root / "state" / "queue").glob("*.review")):
        h = header(f.read_text(encoding="utf-8", errors="replace"))
        entry = h.get("entry", f.stem)
        if since and entry[2:10] < since:
            continue
        if h.get("test_hook") == "true":
            continue  # fake reviewer in tests, not a real call
        rows.append(h)
    if not rows:
        print("no real reviews found")
        return 0
    by = defaultdict(lambda: {"n": 0, "pass": 0, "fail": 0, "cost": 0.0})
    for h in rows:
        m = by[h.get("model", "?")]
        m["n"] += 1
        verdict = h.get("verdict", "")
        m["pass" if verdict.startswith("PASS") else "fail"] += 1
        try:
            m["cost"] += float(h.get("cost_usd", "0") or 0)
        except ValueError:
            pass
    print(f"{'model':<8} {'reviews':>7} {'pass':>5} {'fail':>5} {'total $':>9} {'avg $':>8}")
    total = 0.0
    for model, m in sorted(by.items()):
        total += m["cost"]
        print(f"{model:<8} {m['n']:>7} {m['pass']:>5} {m['fail']:>5} {m['cost']:>9.4f} {m['cost'] / m['n']:>8.4f}")
    print(f"all      {len(rows):>7} {'':>5} {'':>5} {total:>9.4f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
