"""Tests that provider errors (HTTP 429, no credits, CLI errors, timeouts) are recorded as is_error by run.py and
left out of the percentages by score.py.

Run (from the repo root):
  python -I -B -m unittest discover -s bench/coder -p "test_provider_errors.py" -v

run.py is driven with a fake CLI (a .py file given as BENCH_CLAUDE); no answer holds a code block, so nothing is graded
(no cargo) and no network or paid call is made. score.py is run on hand-written result files in a temp folder.
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, subprocess, tempfile, unittest

HERE = pathlib.Path(__file__).resolve().parent
CLI_429 = ('import json\nprint(json.dumps({"result": "API Error: 429 {\\"error\\": \\"You have no credits remaining\\"}", '
           '"is_error": True, "total_cost_usd": 0}))\n')
CLI_EXIT = 'import json, sys\nprint(json.dumps({"result": "no code here", "is_error": False}))\nsys.exit(3)\n'
CLI_SLEEP = "import time\ntime.sleep(60)\n"
CLI_PLAIN = 'import json\nprint(json.dumps({"result": "no code here", "is_error": False}))\n'


def load_run():
    spec = importlib.util.spec_from_file_location("run_under_test_errors", HERE / "run.py")
    run = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(run)
    return run


def run_task(cli_source, timeout=None):
    """Runs run.py main() on one task with a fake CLI; returns the result record."""
    with tempfile.TemporaryDirectory() as tmp:
        tmp = pathlib.Path(tmp)
        (tmp / "fake_cli.py").write_text(cli_source, encoding="utf-8")
        out = tmp / "out"
        os.environ["BENCH_CLAUDE"] = str(tmp / "fake_cli.py")
        os.environ["BENCH_CWD"] = str(tmp / "cwd")
        run = load_run()
        if timeout:
            run.CFG["model_timeout_seconds"] = timeout
        old_argv, sys.argv = sys.argv, ["run.py", str(out), "claude", "fake", "default", "01-duration"]
        try:
            with contextlib.redirect_stdout(io.StringIO()):
                run.main()
        finally:
            sys.argv = old_argv
        return json.loads(next(out.glob("*__claude__fake__default.json")).read_text(encoding="utf-8"))


class RunRecordsProviderErrors(unittest.TestCase):
    def test_429_with_error_text_in_the_answer_is_an_error_not_a_format_failure(self):
        rec = run_task(CLI_429)
        self.assertTrue(rec["is_error"])
        self.assertTrue(rec["call_failed"])
        self.assertFalse(rec["format_failure"])
        self.assertIn("429", rec["error"])
        self.assertIn("no credits", rec["error"])
        self.assertIn("429", rec["first_error"])

    def test_nonzero_exit_is_an_error(self):
        rec = run_task(CLI_EXIT)
        self.assertTrue(rec["is_error"])
        self.assertFalse(rec["format_failure"])
        self.assertEqual(rec["error"], "CLI exit code 3")

    def test_timeout_is_an_error(self):
        rec = run_task(CLI_SLEEP, timeout=1)
        self.assertTrue(rec["is_error"])
        self.assertIn("timed out after 1s", rec["error"])
        self.assertFalse(rec["format_failure"])

    def test_a_real_format_failure_is_not_an_error(self):
        rec = run_task(CLI_PLAIN)
        self.assertFalse(rec["is_error"])
        self.assertIsNone(rec["error"])
        self.assertTrue(rec["format_failure"])
        self.assertFalse(rec["call_failed"])

    def test_provider_error_unit(self):
        run = load_run()
        self.assertIsNone(run.provider_error({"is_error": False, "exit": 0, "answer": "x"}))
        self.assertIsNone(run.provider_error({"is_error": None, "exit": None, "answer": "x"}))
        self.assertEqual(run.provider_error({"is_error": True, "error": "HTTP 429", "answer": ""}), "HTTP 429")
        self.assertEqual(run.provider_error({"is_error": True, "error": None, "answer": "API Error: 429"}), "API Error: 429")
        self.assertEqual(run.provider_error({"is_error": True, "error": None, "answer": ""}), "(no error text)")
        self.assertEqual(run.provider_error({"is_error": False, "exit": 2, "answer": "x"}), "CLI exit code 2")


def rec(model, task, passed, total, **kw):
    r = {"provider": "openai", "model": model, "effort": "default", "task": task, "tokens": {}, "cost_usd": None,
         "wall_s": 2.0, "exit": 0, "harness_wall_s": 1.0, "compiles": passed > 0, "passed": passed, "total": total,
         "format_failure": False, "first_error": None}
    r.update(kw)
    return r


ERR429 = "HTTP 429: rate limit or quota reached, still failing after 2 retries. API message: You have no credits remaining."


def write(folder, r, raw=None):
    stem = f"{r['task']}__{r['provider']}__{r['model']}__{r['effort']}"
    (folder / f"{stem}.json").write_text(json.dumps(r), encoding="utf-8")
    if raw is not None:
        (folder / f"{stem}.raw.json").write_text(json.dumps(raw), encoding="utf-8")


def score(folder):
    p = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(folder)], capture_output=True)
    return p.returncode, p.stdout.decode("utf-8"), p.stderr.decode("utf-8")


class ScoreExcludesProviderErrors(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.d = pathlib.Path(self._tmp.name)
        # good: one pass, one 429 recorded the new way (is_error)
        write(self.d, rec("good", "01-a", 10, 10))
        write(self.d, rec("good", "02-b", 0, 10, compiles=False, call_failed=True, is_error=True, error=ERR429,
                          first_error="model call failed: " + ERR429))
        # only429: every run an old-format row (no is_error key): format_failure with a provider error as first_error
        for t in ("01-a", "02-b"):
            write(self.d, rec("only429", t, 0, 10, compiles=False, format_failure=True, first_error="model call failed: " + ERR429),
                  raw={"error": ERR429})
        # oldcli: old claude row, the CLI's error text was the answer: first_error is a block count, only the raw file says error
        write(self.d, rec("oldcli", "01-a", 0, 10, compiles=False, format_failure=True,
                          first_error="0 code blocks in the answer (exactly one is required)"),
              raw={"is_error": True, "result": "API Error: 429 no credits"})
        write(self.d, rec("oldcli", "02-b", 5, 10))
        # real: a genuine old format failure (a normal answer without a block) must stay a format failure
        write(self.d, rec("real", "01-a", 0, 10, compiles=False, format_failure=True,
                          first_error="0 code blocks in the answer (exactly one is required)"),
              raw={"result": "I cannot do this", "is_error": False})
        write(self.d, rec("real", "02-b", 10, 10))

    def tearDown(self):
        self._tmp.cleanup()

    def rows(self, out):
        return {line.split()[0]: line for line in out.splitlines() if line.startswith("openai:")}

    def test_errors_are_excluded_from_the_percentage_and_listed(self):
        code, out, err = score(self.d)
        self.assertEqual(code, 0, err)
        rows = self.rows(out)
        self.assertIn("10/10 100%", rows["openai:good"])  # the 429 task is not a 0/10 in the sum
        self.assertIn("10/10  error", rows["openai:good"])  # task 01 passed, task 02 shows "error"
        self.assertIn("5/10 50%", rows["openai:oldcli"])  # the old-format CLI error is excluded too
        self.assertIn("10/20 50%", rows["openai:real"])  # a real format failure still counts as 0
        self.assertIn("errored runs (4;", out)  # good 1, only429 2, oldcli 1
        self.assertIn("02-b openai:good default: " + ERR429, out)
        self.assertIn("API Error: 429 no credits", out)  # the raw output of the old CLI row is the error text

    def test_a_model_with_only_errors_shows_no_valid_runs(self):
        code, out, err = score(self.d)
        row = self.rows(out)["openai:only429"]
        self.assertIn("no valid runs", row)
        self.assertNotIn("0%", row)

    def test_old_rows_relabelled_are_counted_and_real_format_failures_are_not(self):
        code, out, err = score(self.d)
        self.assertIn("3 old format_failure row(s) re-labelled", out)  # only429 x2 + oldcli x1; not "real", not good
        real = self.rows(out)["openai:real"].split()
        self.assertEqual(real[-3], "1")  # format column (then cost, time): the genuine format failure stays one

    def test_no_errors_means_no_error_section(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            write(tmp, rec("fine", "01-a", 4, 10))
            code, out, err = score(tmp)
            self.assertEqual(code, 0, err)
            self.assertNotIn("errored runs", out)
            self.assertNotIn("re-labelled", out)
            self.assertIn("4/10 40%", out)

    def test_attempts_table_marks_an_errored_task_and_does_not_rank_it_as_no_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            extra = dict(max_attempts=2, attempts_made=1, pass_at=1, cost_usd_total=0.5, cost_unknown_attempts=0, wall_s_total=2.0)
            write(tmp, rec("m", "01-a", 10, 10, cost_usd=0.5, **extra))
            write(tmp, rec("m", "02-b", 0, 10, compiles=False, call_failed=True, is_error=True, error=ERR429,
                           max_attempts=2, attempts_made=1, pass_at=None, cost_usd_total=None, cost_unknown_attempts=1, wall_s_total=2.0))
            write(tmp, rec("dead", "01-a", 0, 10, compiles=False, call_failed=True, is_error=True, error=ERR429,
                           max_attempts=2, attempts_made=1, pass_at=None, cost_usd_total=None, cost_unknown_attempts=1, wall_s_total=2.0))
            code, out, err = score(tmp)
            self.assertEqual(code, 0, err)
            self.assertNotIn("no pass", out)
            self.assertIn("1 errored (excluded)", out)
            self.assertIn("no valid runs (1 errored)", out)


if __name__ == "__main__":
    unittest.main()
