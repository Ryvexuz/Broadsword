"""Tests for run.py --attempts (retries with the failing test names fed back) and score.py's cost-to-pass table.

Run (from the repo root):
  python -I -B -m unittest discover -s bench/coder -p "test_run_attempts.py" -v

A fake CLI (BENCH_CLAUDE) answers "bad" code, or "good" code once the prompt carries feedback. grade_batch is replaced
by a fake that passes only a "good" solution of task 01-duration. No cargo, no network, no model call.
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, tempfile, unittest

HERE = pathlib.Path(__file__).resolve().parent
# the prompt of every call is appended to a log file next to the CLI, so the test can read what the model was told
FAKE_CLI = '''import json, pathlib, sys
prompt = sys.stdin.read()
with open(pathlib.Path(__file__).with_name("prompts.log"), "a", encoding="utf-8") as f:
    f.write(json.dumps(prompt) + chr(10))
good = "failed these hidden tests" in prompt
answer = "```rust" + chr(10) + ("pub fn good() {}" if good else "pub fn bad() {}") + chr(10) + "```"
print(json.dumps({"result": answer, "is_error": False, "total_cost_usd": 0.25,
                  "usage": {"input_tokens": 10, "output_tokens": 20}, "modelUsage": {"fake-model": {}}}))
'''


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


def fake_grade(items):
    out = []
    for rs, task in items:
        ok = "good" in rs.read_text(encoding="utf-8") and task.name == "01-duration"
        out.append({"compiles": True, "passed": 3 if ok else 1, "total": 3, "wall_s": 1.0, "first_error": None,
                    "failed_tests": [] if ok else ["hidden_a", "hidden_b"]})
    return out


class Attempts(unittest.TestCase):
    def run_main(self, run, argv):
        old = sys.argv
        sys.argv = argv
        buf = io.StringIO()
        try:
            with contextlib.redirect_stdout(buf):
                run.main()
        finally:
            sys.argv = old
        return buf.getvalue()

    def setup_run(self, tmp):
        cli = tmp / "fake_cli.py"
        cli.write_text(FAKE_CLI, encoding="utf-8")
        os.environ["BENCH_CLAUDE"] = str(cli)
        os.environ["BENCH_CWD"] = str(tmp / "cwd")
        run = load("run")
        run.H.grade_batch = fake_grade
        return run, cli.with_name("prompts.log")

    def test_retry_feeds_back_names_only_and_sums_the_cost(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            run, plog = self.setup_run(tmp)
            out = tmp / "out"
            self.run_main(run, ["run.py", str(out), "claude", "fake-model", "default", "--attempts", "3",
                                "01-duration", "02-ring-buffer"])
            r1 = json.loads((out / "01-duration__claude__fake-model__default.json").read_text(encoding="utf-8"))
            self.assertEqual((r1["attempts_made"], r1["pass_at"], r1["passed"]), (2, 2, 3))
            self.assertAlmostEqual(r1["cost_usd_total"], 0.5)
            self.assertEqual([a["passed"] for a in r1["attempts"]], [1, 3])
            self.assertEqual(r1["attempts"][0]["failed_tests"], ["hidden_a", "hidden_b"])
            r2 = json.loads((out / "02-ring-buffer__claude__fake-model__default.json").read_text(encoding="utf-8"))
            self.assertEqual((r2["attempts_made"], r2["pass_at"]), (3, None))
            self.assertAlmostEqual(r2["cost_usd_total"], 0.75)
            # prompts: 2 tasks in round 1, 1 retry for 01 ... 02 is asked in rounds 2 and 3 as well (it never passes)
            prompts = [json.loads(x) for x in plog.read_text(encoding="utf-8").splitlines()]
            self.assertEqual(len(prompts), 5)
            retry = [p for p in prompts if "failed these hidden tests" in p]
            self.assertEqual(len(retry), 3)
            for p in retry:
                self.assertIn("(names only): hidden_a, hidden_b.", p)
                self.assertNotIn("pub fn bad", p)  # the previous solution and any test output are never fed back
            # the latest attempt is in <tag>.rs, the second in <tag>__a2.rs
            self.assertIn("good", (out / "01-duration__claude__fake-model__default.rs").read_text(encoding="utf-8"))
            self.assertTrue((out / "01-duration__claude__fake-model__default__a2.rs").is_file())
            # score: cost to pass, "no pass at cost X" for the task that never passed
            score = load("score")
            sys.argv = ["score.py", str(out)]
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                score.main()
            text = buf.getvalue()
            self.assertIn("cost to pass", text)
            self.assertIn("pass@1", text)
            self.assertIn("$0.500; no pass at cost $0.750 (1)", text)
            self.assertIn("no pass $0.750", text)

    def test_one_attempt_keeps_the_one_shot_format(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            run, plog = self.setup_run(tmp)
            out = tmp / "out"
            self.run_main(run, ["run.py", str(out), "claude", "fake-model", "default", "02-ring-buffer"])
            rec = json.loads((out / "02-ring-buffer__claude__fake-model__default.json").read_text(encoding="utf-8"))
            self.assertNotIn("attempts", rec)
            self.assertNotIn("_failed", rec)
            self.assertEqual(len(plog.read_text(encoding="utf-8").splitlines()), 1)
            score = load("score")
            sys.argv = ["score.py", str(out)]
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                score.main()
            self.assertNotIn("cost to pass", buf.getvalue())

    def test_attempts_out_of_range_is_refused(self):
        run = load("run")
        for bad in ("0", "4", "x"):
            with self.assertRaises(SystemExit):
                run.take_attempts(["--attempts", bad])
        self.assertEqual(run.take_attempts(["a", "--attempts=2", "b"]), (["a", "b"], 2))

    def test_unknown_attempt_cost_is_not_zero_in_the_score(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp)
            rec = {"provider": "claude", "model": "m", "effort": "default", "task": "01-duration", "cost_usd": None,
                   "cost_usd_total": 0.1, "cost_unknown_attempts": 1, "wall_s": 1.0, "wall_s_total": 2.0,
                   "compiles": True, "passed": 3, "total": 3, "format_failure": False, "call_failed": False,
                   "graded": True, "max_attempts": 3, "attempts_made": 2, "pass_at": 2}
            (out / "01-duration__claude__m__default.json").write_text(json.dumps(rec), encoding="utf-8")
            score = load("score")
            sys.argv = ["score.py", str(out)]
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                score.main()
            self.assertIn("2 $0.100+", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
