"""Test that run.py records a model call past model_timeout_seconds as a failed call and goes on to the next task.

Run (from the repo root):
  python -I -B -m unittest discover -s bench/coder -p "test_run_timeout.py" -v

A fake CLI (a .py file given as BENCH_CLAUDE) sleeps far longer than the tiny timeout the test sets.
Nothing is graded (no code block in any answer), so no cargo is started and no network is used.
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, tempfile, unittest

HERE = pathlib.Path(__file__).resolve().parent
FAKE_CLI = "import time\ntime.sleep(60)\n"


class RunTimeout(unittest.TestCase):
    def test_timeout_is_a_failed_call_and_the_run_continues(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            cli = tmp / "fake_cli.py"
            cli.write_text(FAKE_CLI, encoding="utf-8")
            work = tmp / "cwd"
            out = tmp / "out"
            os.environ["BENCH_CLAUDE"] = str(cli)
            os.environ["BENCH_CWD"] = str(work)
            spec = importlib.util.spec_from_file_location("run_under_test", HERE / "run.py")
            run = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(run)
            run.CFG["model_timeout_seconds"] = 1  # the module's own copy of the settings, not the file
            old_argv = sys.argv
            sys.argv = ["run.py", str(out), "claude", "fake", "default", "01-duration", "02-ring-buffer"]
            buf = io.StringIO()
            try:
                with contextlib.redirect_stdout(buf):
                    run.main()  # an uncaught TimeoutExpired would end the test here
            finally:
                sys.argv = old_argv
            results = sorted(out.glob("*__claude__fake__default.json"))
            self.assertEqual(len(results), 2, "both tasks must get a result, the first timeout must not abort the run")
            for r in results:
                rec = json.loads(r.read_text(encoding="utf-8"))
                self.assertTrue(rec["format_failure"])
                self.assertIn("timed out after 1s", rec["first_error"])
                self.assertFalse(rec["compiles"])
                self.assertEqual(rec["passed"], 0)


if __name__ == "__main__":
    unittest.main()
