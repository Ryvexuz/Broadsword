"""Tests for the reusable functions and the cost-to-completion table of scripts/dev_metrics.py.
They use made-up records (a scratch folder stands in for state/), never the real state/.

  python -I -B scripts/test_dev_metrics.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import pathlib
import re
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("dev_metrics_under_test", HERE / "dev_metrics.py")
dm = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(dm)

HEAD = "date_utc\trole\tmodel\ttask\ttokens\tseconds\toutcome\tnote\n"


def scratch_tmp():
    m = re.search(r'^scratch_dir\s*=\s*"([^"]+)"', (dm.ROOT / "broadsword.toml").read_text(encoding="utf-8"), re.M)
    if not m:
        raise RuntimeError("broadsword.toml has no scratch_dir")
    base = dm.ROOT / m.group(1)
    base.mkdir(parents=True, exist_ok=True)
    return tempfile.TemporaryDirectory(dir=base, prefix="test-dev-metrics-", ignore_cleanup_errors=True)


def task(tid, status="done", started=1000, ended=1600):
    return {"id": tid, "status": status, "created": str(started), "started": str(started), "ended": str(ended)}


def entry(eid, tid, verdict="PASS", coder="sonnet", **kw):
    e = {"id": eid, "task": tid, "created": eid[-4:].lstrip("0") or "1", "coder": coder, "reviewer_model": "haiku",
         "verdict": verdict, "tokens_in": "100", "tokens_out": "50", "cost_usd": "0.01", "blocking": ""}
    e.update(kw)
    return e


def run(tid, tokens, role="coder", model="sonnet", seconds="60"):
    return {"date_utc": "2026-11-19", "role": role, "model": model, "task": tid, "tokens": tokens, "seconds": seconds}


class ModelRows(unittest.TestCase):
    def test_coder_rows_from_the_runs_file(self):
        with scratch_tmp() as d:
            old = dm.ROOT
            root = pathlib.Path(d)
            (root / "state").mkdir()
            (root / "state" / "agent-runs.tsv").write_text(
                HEAD + "2026-11-19\tcoder\tsonnet\tt1\t1000\t120\tdone\t\n2026-11-19\tcoder\tsonnet\tt2\t\t60\tdone\tno tokens\n"
                "2026-11-18\tcoder\thaiku\tt3\t5\t10\tdone\t\n", encoding="utf-8")
            dm.ROOT = root
            try:
                rows = dm.model_rows([], "20261119", 0, 10)
                runs = dm.read_runs(None)
            finally:
                dm.ROOT = old
        self.assertEqual(rows, {("sonnet", "coder"): [2, 180, 1000, 0.0]})  # --since drops the 11-18 run; empty tokens add 0 here
        self.assertEqual(len(runs), 3)
        self.assertEqual(runs[1]["tokens"], "")  # read_runs keeps unknown as empty, so callers can tell it from 0

    def test_models_prints_the_same_numbers(self):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            dm.models([entry("q-0001", "t1", cost_usd="0.5", created="5")], None, 0, 10, 100)
        out = buf.getvalue()
        self.assertIn("haiku    reviewer        1", out)
        self.assertIn("150", out)
        self.assertIn("0.5000", out)


class Completion(unittest.TestCase):
    def costs(self, tasks, entries, runs, verdicts=()):
        by = {}
        for e in entries:
            by.setdefault(e["task"], []).append(e)
        return {r["id"]: r for r in dm.task_costs(tasks, by, runs, list(verdicts))}

    def test_sums_every_run_and_review_until_done(self):
        r = self.costs([task("t1")], [entry("q-0001", "t1", "FAIL", blocking="a.rs:1 - x\\nb.rs:2 - y"), entry("q-0002", "t1")],
                       [run("t1", "1000"), run("t1", "500", role="fix")])["t1"]
        self.assertEqual(r["claude"], [1000 + 500 + 150 + 150, 0])
        self.assertAlmostEqual(r["usd"], 0.02)
        self.assertEqual((r["rounds"], r["wall"], r["first_verdict"], r["first_lines"]), (2, 600, "FAIL", 2))

    def test_unknown_tokens_are_not_zero(self):
        r = self.costs([task("t1")], [], [run("t1", ""), run("t1", "400")])["t1"]
        self.assertEqual(r["claude"], [400, 1])  # known part kept, one unknown part counted
        only_unknown = self.costs([task("t1")], [], [run("t1", "")])["t1"]
        self.assertEqual(only_unknown["claude"], [0, 1])

    def test_codex_is_openai_plan_tokens_not_free_dollars(self):
        r = self.costs([task("t1")], [entry("q-0001", "t1", shadow_model="gpt-5.6-sol", shadow_tokens="7000", shadow_cost_usd="")],
                       [run("t1", "10")])["t1"]
        self.assertEqual(r["openai"], [7000, 0])
        self.assertAlmostEqual(r["usd"], 0.01)  # the gating review only; the shadow adds no $

    def test_open_tasks_never_enter_the_table(self):
        tasks = [task("t1"), task("t2", status="running", ended=0), task("t3", status="abandoned")]
        entries = [entry("q-0001", "t1"), entry("q-0002", "t2"), entry("q-0003", "t3")]
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            dm.completion(tasks, entries, [run("t1", "100"), run("t2", "99999999"), run("t3", "99999999")])
        out = buf.getvalue()
        self.assertIn("sonnet: 1 done, 1 open, 1 not merged", out)
        self.assertIn("Claude tokens         250 / 250", out)  # 100 + 150 review of the done task only

    def test_first_pass_rate_and_adjudicated_split(self):
        tasks = [task("t1"), task("t2")]
        entries = [entry("q-0001", "t1", "FAIL", blocking="a:1 - x\\nb:2 - y"), entry("q-0002", "t1"), entry("q-0003", "t2")]
        verdicts = [{"entry": "q-0001", "verdict": "real"}, {"entry": "q-0002", "verdict": "false"}]
        rows = self.costs(tasks, entries, [], verdicts)
        self.assertEqual(rows["t1"]["first_adjudicated"], ["real"])  # only the FIRST review's findings
        self.assertEqual(rows["t2"]["first_verdict"], "PASS")


if __name__ == "__main__":
    unittest.main()
