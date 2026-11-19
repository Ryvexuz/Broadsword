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

# a CLI whose answer depends on how many times it was called: call 1 bad code, call 2 __SECOND__, call 3 good code
SCRIPTED_CLI = '''import json, pathlib, sys
prompt = sys.stdin.read()
log = pathlib.Path(__file__).with_name("prompts.log")
with open(log, "a", encoding="utf-8") as f:
    f.write(json.dumps(prompt) + chr(10))
n = len(log.read_text(encoding="utf-8").splitlines())
code = lambda body: "```rust" + chr(10) + body + chr(10) + "```"
if n == 2:
    out = {"result": __SECOND__, "is_error": True}
else:
    out = {"result": code("pub fn good() {}" if n == 3 else "pub fn bad() {}"), "is_error": False}
out.update(total_cost_usd=0.25, usage={"input_tokens": 10, "output_tokens": 20}, modelUsage={"fake-model": {}})
print(json.dumps(out))
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

    def scripted(self, second, argv_extra=("01-duration",), grade=fake_grade):
        tmp = pathlib.Path(tempfile.mkdtemp())
        self.addCleanup(__import__("shutil").rmtree, tmp, True)
        cli = tmp / "fake_cli.py"
        cli.write_text(SCRIPTED_CLI.replace("__SECOND__", second), encoding="utf-8")
        os.environ["BENCH_CLAUDE"] = str(cli)
        os.environ["BENCH_CWD"] = str(tmp / "cwd")
        run = load("run")
        run.H.grade_batch = grade
        out = tmp / "out"
        self.run_main(run, ["run.py", str(out), "claude", "fake-model", "default", "--attempts", "3", *argv_extra])
        prompts = [json.loads(x) for x in (tmp / "prompts.log").read_text(encoding="utf-8").splitlines()]
        rec = json.loads((out / "01-duration__claude__fake-model__default.json").read_text(encoding="utf-8"))
        return run, out, prompts, rec

    def test_each_attempt_records_its_own_prompt_and_result_after_a_failed_call(self):
        # call 2 fails with NO answer; call 3 must be asked the base prompt again, and each entry holds its own prompt
        run, out, prompts, rec = self.scripted('""')
        self.assertEqual(len(prompts), 3)
        self.assertEqual([a["prompt"] for a in rec["attempts"]], prompts)
        self.assertTrue(rec["attempts"][1]["call_failed"])
        self.assertEqual(rec["attempts"][1]["passed"], 0)
        self.assertEqual((rec["attempts"][2]["passed"], rec["pass_at"]), (3, 3))  # call 3 answers good code
        self.assertEqual(rec["passed"], rec["attempts"][2]["passed"])
        self.assertEqual(rec["first_error"], rec["attempts"][2]["first_error"])

    def test_failed_call_with_an_answer_but_no_code_is_its_own_attempt(self):
        run, out, prompts, rec = self.scripted('"no code here"')
        a = rec["attempts"]
        self.assertEqual([x["prompt"] for x in a], prompts)
        self.assertTrue(a[1]["format_failure"])
        self.assertFalse(a[1]["compiles"])
        self.assertEqual(a[1]["passed"], 0)
        self.assertIn("exactly one fenced rust code block", prompts[2])  # fed back from attempt 2, not attempt 1

    def ungraded_run(self):
        def refuse(items):
            raise SystemExit("job refused")
        return self.scripted('""', grade=refuse)

    def test_a_round_whose_grading_failed_is_recorded_ungraded_and_says_it_stopped(self):
        old = sys.stderr
        sys.stderr = io.StringIO()
        try:
            run, out, prompts, rec = self.ungraded_run()
        finally:
            sys.stderr = old
        self.assertEqual(len(prompts), 1)
        self.assertIs(rec["graded"], False)
        self.assertIs(rec["attempts"][0]["graded"], False)
        self.assertIsNone(rec["pass_at"])
        self.assertEqual(rec["attempts_made"], 1)
        self.assertIn("grading failed in attempt 1", rec["stopped_early"])

    def test_regrade_of_an_ungraded_multi_attempt_solution_recomputes_pass_at(self):
        old = sys.stderr
        sys.stderr = io.StringIO()
        try:
            run, out, prompts, rec = self.ungraded_run()
        finally:
            sys.stderr = old
        # the saved .rs is "bad" (call 1); make it a solution the fake grader passes, then regrade
        rs = out / "01-duration__claude__fake-model__default.rs"
        rs.write_text("pub fn good() {}", encoding="utf-8")
        run.H.grade_batch = fake_grade
        with contextlib.redirect_stdout(io.StringIO()):
            run.regrade(out)
        rec = json.loads((out / "01-duration__claude__fake-model__default.json").read_text(encoding="utf-8"))
        self.assertEqual(rec["pass_at"], 1)
        self.assertNotIn("stopped_early", rec)
        score = load("score")
        sys.argv = ["score.py", str(out)]
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            score.main()
        self.assertIn("1/1", buf.getvalue().split("cost to pass")[1])  # passed 1/1 in the cost-to-pass table
        self.assertNotIn("no pass", buf.getvalue().split("cost to pass")[1].split(chr(10), 1)[1])

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

    def score_text(self, recs):
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp)
            for i, r in enumerate(recs):
                base = {"provider": "claude", "effort": "default", "task": "01-duration", "wall_s": 1.0, "wall_s_total": 2.0,
                        "compiles": True, "passed": 3, "total": 3, "format_failure": False, "call_failed": False,
                        "graded": True, "max_attempts": 3, "attempts_made": 1, "pass_at": 1}
                base.update(r)
                base = {k: v for k, v in base.items() if v != "DEL"}  # "DEL" drops a default key (a one-shot record)
                (out / f"{base['task']}__{base['provider']}__{base['model']}__default.json").write_text(json.dumps(base), encoding="utf-8")
            score = load("score")
            sys.argv = ["score.py", str(out)]
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                score.main()
            return buf.getvalue()

    def test_unknown_cost_models_rank_after_known_ones(self):
        text = self.score_text([{"model": "aaa-plan", "cost_usd": None, "cost_usd_total": None, "cost_unknown_attempts": 1},
                                {"model": "zzz-paid", "cost_usd": 0.5, "cost_usd_total": 0.5, "cost_unknown_attempts": 0}])
        table = text.split("cost to pass")[1]
        self.assertLess(table.index("claude:zzz-paid"), table.index("claude:aaa-plan"))
        self.assertIn("unknown or plan", table.split(chr(10))[0])  # the header says how unknown costs rank

    def test_a_task_with_an_unknown_attempt_cost_shows_the_lower_bound_in_the_main_table(self):
        text = self.score_text([{"model": "m", "cost_usd": 0.1, "cost_usd_total": 0.1, "cost_unknown_attempts": 1}])
        self.assertIn("$0.100+ (1 n/a)", text.split("cost to pass")[0])

    def real_shape_grade(self, items):
        """The shapes bench/coder/harness.py really returns when a build fails or the tests cannot be listed: no
        failed_tests key at all (see grade_inner: it is only set after the per-test loop)."""
        out = []
        for rs, task in items:
            ok = "good" in rs.read_text(encoding="utf-8")
            if ok:
                out.append({"task": task.name, "compiles": True, "passed": 3, "failed": 0, "total": 3, "wall_s": 1.0,
                            "first_error": None, "timed_out": False, "failed_tests": []})
            elif "bad" in rs.read_text(encoding="utf-8"):  # tests could not be listed: compiled, nothing known per test
                out.append({"task": task.name, "compiles": True, "passed": 0, "failed": 3, "total": 3, "wall_s": 1.0,
                            "first_error": "listing the tests timed out after 5s", "timed_out": True})
            else:
                out.append({"task": task.name, "compiles": False, "passed": 0, "failed": 3, "total": 3, "wall_s": 1.0,
                            "first_error": "error: nope", "timed_out": False})
        return out

    def test_a_harness_result_without_failed_tests_gets_the_fallback_wording(self):
        run, out, prompts, rec = self.scripted('""', grade=self.real_shape_grade)
        retry = prompts[1]  # call 2 follows attempt 1: compiled, but no test names
        self.assertNotIn("(names only): .", "".join(prompts))
        self.assertNotIn("(names only): ", retry)
        self.assertIn("did not pass all of the hidden tests", retry)
        self.assertEqual(rec["attempts"][0]["failed_tests"], [])

    def test_attempt_one_files_are_kept_when_a_later_attempt_overwrites_the_latest(self):
        run, out, prompts, rec = self.scripted('""')
        tag = "01-duration__claude__fake-model__default"
        self.assertIn("bad", (out / f"{tag}__a1.rs").read_text(encoding="utf-8"))
        self.assertTrue((out / f"{tag}__a1.answer.txt").is_file())
        self.assertTrue((out / f"{tag}__a1.raw.json").is_file())

    def test_retries_flag_is_the_attempt_budget_and_http_retries_is_renamed(self):
        run = load("run")
        self.assertEqual(run.take_attempts(["a", "--retries", "3", "b"]), (["a", "b"], 3))
        self.assertEqual(run.take_attempts(["a", "--retries=2"]), (["a"], 2))
        self.assertEqual(run.take_attempts(["a", "--http-retries", "4"]), (["a", "--retries", "4"], 1))
        with self.assertRaises(SystemExit):
            run.take_attempts(["--retries", "9"])

    def test_the_brief_command_with_retries_3_writes_attempts_and_cost_to_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            tmp = pathlib.Path(tmp)
            run, plog = self.setup_run(tmp)
            out = tmp / "out"
            self.run_main(run, ["run.py", str(out), "claude", "fake-model", "default", "--retries", "3", "01-duration"])
            rec = json.loads((out / "01-duration__claude__fake-model__default.json").read_text(encoding="utf-8"))
            self.assertEqual((rec["attempts_made"], rec["pass_at"]), (2, 2))
            self.assertAlmostEqual(rec["cost_usd_total"], 0.5)

    def test_a_one_shot_pass_in_a_group_with_multi_attempt_records_is_a_pass_at_1(self):
        text = self.score_text([{"model": "m", "task": "01-duration", "cost_usd": 0.1, "cost_usd_total": 0.1,
                                 "cost_unknown_attempts": 0},
                                {"model": "m", "task": "02-ring-buffer", "cost_usd": 0.2, "max_attempts": "DEL", "pass_at": "DEL"}])
        tbl = text.split("cost to pass")[1].split(chr(10))[2]
        self.assertIn("2/2", tbl)
        self.assertNotIn("no pass", tbl)

    def test_a_model_with_only_ungraded_tasks_does_not_rank_as_free(self):
        text = self.score_text([{"model": "aaa", "graded": False, "cost_usd": 0.4, "cost_usd_total": 0.4, "pass_at": None,
                                 "passed": 0, "compiles": False},
                                {"model": "zzz", "cost_usd": 0.5, "cost_usd_total": 0.5, "cost_unknown_attempts": 0}])
        table = text.split("cost to pass")[1]
        self.assertLess(table.index("claude:zzz"), table.index("claude:aaa"))
        self.assertIn("1 ungraded", table)

    def test_codex_call_without_a_price_is_marked_plan_not_n_a(self):
        text = self.score_text([{"model": "gpt-x", "provider": "codex", "cost_usd": None, "cost_usd_total": None,
                                 "cost_unknown_attempts": 1}])
        self.assertIn("plan", text)
        self.assertNotIn("n/a", text)


if __name__ == "__main__":
    unittest.main()
