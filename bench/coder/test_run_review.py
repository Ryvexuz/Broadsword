"""Offline reviewer and judge regression tests; no model calls."""
import importlib.util
import json
import pathlib
import sys
import tempfile
import types
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


R = load("run_review_test", "run.py")
S = load("score_review_test", "score.py")


class ReviewTests(unittest.TestCase):
    def test_shared_codex_entry_and_no_fallback(self):
        calls = []
        def fake(args, prompt, env, deadline, timeout, share):
            calls.append((args.model, prompt, env["OPENAI_REVIEW_CALL_TYPE"], share))
            return {"result": '{"findings":[]}', "usage": {"input_tokens": 9}}
        with patch.object(R, "review_client", return_value=types.SimpleNamespace(review_codex=fake)):
            result = R.codex_review("fake", "spec and source", "review")
        self.assertFalse(result["failed"])
        self.assertEqual(calls, [("fake", "spec and source", "bench", 1.0)])
        self.assertIsNone(result["cost_usd"])

    def test_failures_are_unknown_not_zero(self):
        for error in (TimeoutError("deadline"), FileNotFoundError("codex"), ValueError("refused")):
            with self.subTest(error=error), patch.object(R, "review_client", side_effect=error):
                result = R.codex_review("fake", "data", "review")
            self.assertTrue(result["failed"])
            self.assertIn(str(error), result["error"])
            self.assertIsNone(result["tokens"])
            self.assertIsNone(result["cost_usd"])

    def test_review_eligibility_and_hidden_tests_excluded(self):
        with tempfile.TemporaryDirectory() as tmp:
            task = pathlib.Path(tmp)
            (task / "spec.md").write_text("PUBLIC SPEC", encoding="utf-8")
            (task / "tests.rs").write_text("HIDDEN SECRET", encoding="utf-8")
            source = task / "solution.rs"
            source.write_text("SOURCE", encoding="utf-8")
            with patch.object(R, "codex_review", return_value={"failed": False, "answer": '{"findings":["bug"]}'}) as call:
                for graded, compiles in ((False, True), (True, False)):
                    R.review_solution({"graded": graded, "compiles": compiles}, task, source, "fake")
                call.assert_not_called()
                rec = {"graded": True, "compiles": True}
                R.review_solution(rec, task, source, "fake")
                call.assert_called_once()
                prompt = call.call_args.args[1]
                self.assertIn("PUBLIC SPEC", prompt)
                self.assertIn("SOURCE", prompt)
                self.assertNotIn("HIDDEN SECRET", prompt)
                self.assertEqual(rec["review"]["findings"], [{"text": "bug"}])

    def test_bad_review_is_not_pass(self):
        with tempfile.TemporaryDirectory() as tmp:
            task = pathlib.Path(tmp)
            (task / "spec.md").write_text("spec", encoding="utf-8")
            source = task / "solution.rs"
            source.write_text("source", encoding="utf-8")
            for answer in ('PASS', '{"findings":[0]}', '{"findings":[""]}', '[]'):
                with patch.object(R, "codex_review", return_value={"failed": False, "answer": answer}):
                    rec = {"graded": True, "compiles": True}
                    R.review_solution(rec, task, source, "fake")
                    self.assertTrue(rec["review"]["failed"])

    def test_judge_records_per_finding_and_failed_rejudge_removes_verdict(self):
        with tempfile.TemporaryDirectory() as tmp:
            out = pathlib.Path(tmp)
            (out / "spec.md").write_text("spec", encoding="utf-8")
            (out / "a.rs").write_text("source", encoding="utf-8")
            path = out / "a.json"
            path.write_text(json.dumps({"task": "a", "review": {"failed": False, "findings": [{"text": "bug"}]}}), encoding="utf-8")
            with patch.object(R.H, "resolve_task", return_value=out), patch.object(
                    R, "codex_review", return_value={"failed": False, "answer": '{"verdict":"REAL","evidence":"reason"}'}):
                R.adjudicate(out, "judge")
            finding = json.loads(path.read_text())["review"]["findings"][0]
            self.assertEqual((finding["verdict"], finding["judge_model"]), ("REAL", "judge"))
            with patch.object(R.H, "resolve_task", return_value=out), patch.object(
                    R, "codex_review", return_value={"failed": True, "error": "timeout"}):
                with self.assertRaises(SystemExit):
                    R.adjudicate(out, "other")
            self.assertNotIn("verdict", json.loads(path.read_text())["review"]["findings"][0])

    def test_score_counts_only_adjudicated_real_and_plan_is_na(self):
        review = {"model": "fake", "backend": "codex", "failed": False, "cost_usd": None,
                  "findings": [{"verdict": "REAL"}, {"verdict": "FALSE", "judge_model": "j"},
                               {"verdict": "REAL", "judge_model": "j"}]}
        self.assertEqual(S.reviewer_table([{"review": review}])[1], ["fake", 1, 0, 1, "n/a"])
        review["findings"] = []
        self.assertEqual(S.reviewer_table([{"review": review}])[1][-1], "n/a")

    def test_review_option_validation(self):
        self.assertEqual(R.take_review(["out", "--review=fake"]), (["out"], "fake"))
        for args in (["--review"], ["--review="], ["--review=x\ry"], ["--review=a", "--review=b"]):
            with self.assertRaises(SystemExit):
                R.take_review(args)


if __name__ == "__main__":
    unittest.main()
