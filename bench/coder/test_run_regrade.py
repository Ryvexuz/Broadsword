"""Tests for run.py: a grading failure keeps the model call's data, `regrade` grades later, score.py prints a partial cost.

Run (from the repo root):
  python -I -B -m unittest discover -s bench/coder -p "test_run_regrade.py" -v

A fake CLI (BENCH_CLAUDE) answers with a canned code block and cost. grade_batch is replaced by a fake that
fails (busy lock) the first time, so no cargo is started and no network is used.
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, tempfile, unittest

HERE = pathlib.Path(__file__).resolve().parent
FAKE_CLI = '''import json, sys
sys.stdin.read()
answer = "```rust" + chr(10) + "pub fn x() {}" + chr(10) + "```"
print(json.dumps({"result": answer, "is_error": False, "total_cost_usd": 0.25,
                  "usage": {"input_tokens": 10, "output_tokens": 20}, "modelUsage": {"fake-model": {}}}))
'''


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


class Regrade(unittest.TestCase):
    def run_main(self, run, argv):
        old = sys.argv
        sys.argv = argv
        buf, err = io.StringIO(), io.StringIO()
        try:
            with contextlib.redirect_stdout(buf), contextlib.redirect_stderr(err):
                run.main()
        finally:
            sys.argv = old
        return buf.getvalue(), err.getvalue()

    def test_grade_failure_keeps_data_and_regrade_fills_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            cli = tmp / "fake_cli.py"
            cli.write_text(FAKE_CLI, encoding="utf-8")
            out = tmp / "out"
            os.environ["BENCH_CLAUDE"] = str(cli)
            os.environ["BENCH_CWD"] = str(tmp / "cwd")
            run = load("run")
            calls = []

            def busy(items):
                calls.append(len(items))
                raise SystemExit("another harness run holds the lock")

            run.H.grade_batch = busy
            self.run_main(run, ["run.py", str(out), "claude", "fake-model", "default", "01-duration", "02-ring-buffer"])
            results = sorted(out.glob("*__claude__fake-model__default.json"))
            self.assertEqual(len(results), 2, "every task keeps a result JSON although grading failed")
            for r in results:
                rec = json.loads(r.read_text(encoding="utf-8"))
                self.assertIs(rec["graded"], False)
                self.assertIn("another harness run holds the lock", rec["first_error"])
                self.assertEqual(rec["cost_usd"], 0.25)
                self.assertEqual(rec["tokens"]["output"], 20)
                self.assertEqual(rec["model_used"], ["fake-model"])
                self.assertTrue(r.with_suffix(".rs").is_file())
            # score.py shows them as ungraded and still sums the cost
            score = load("score")
            sys.argv = ["score.py", str(out)]
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                score.main()
            self.assertIn("ungraded", buf.getvalue())
            self.assertIn("$0.500", buf.getvalue())

            # regrade: no model call (the CLI is gone), the fake grader answers
            cli.unlink()

            def fine(items):
                return [{"compiles": True, "passed": 3, "total": 4, "wall_s": 1.5, "first_error": None} for _ in items]

            run.H.grade_batch = fine
            self.run_main(run, ["run.py", "regrade", str(out)])
            for r in results:
                rec = json.loads(r.read_text(encoding="utf-8"))
                self.assertIs(rec["graded"], True)
                self.assertEqual((rec["compiles"], rec["passed"], rec["total"]), (True, 3, 4))
                self.assertEqual(rec["cost_usd"], 0.25, "the model call's data is kept")

    def test_score_partial_cost(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp)
            for task, cost in (("01-duration", 0.5), ("02-ring-buffer", None), ("03-merge-intervals", None)):
                rec = {"provider": "claude", "model": "m", "effort": "default", "task": task, "cost_usd": cost,
                       "wall_s": 1.0, "compiles": True, "passed": 1, "total": 2, "format_failure": False}
                (out / f"{task}.json").write_text(json.dumps(rec), encoding="utf-8")
            score = load("score")
            sys.argv = ["score.py", str(out)]
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                score.main()
            self.assertIn("$0.500+ (2 n/a)", buf.getvalue())
            self.assertNotIn("n/a  ", buf.getvalue().replace("(2 n/a)", ""))


if __name__ == "__main__":
    unittest.main()
