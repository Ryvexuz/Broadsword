"""Offline behavioural tests; all git/gh calls use an injected runner."""

import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

# Isolated Python does not put scripts/ on sys.path.
spec = importlib.util.spec_from_file_location(
    "gh_verify", Path(__file__).with_name("gh_verify.py")
)
verify = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify)

SHA = "abcdef0123" * 4


class VerifyTests(unittest.TestCase):
    def setUp(self):
        self.runs = [{
            "databaseId": 123, "status": "completed", "conclusion": "success",
            "createdAt": "2026-11-25T16:24:13Z",
            "updatedAt": "2026-11-25T17:36:54Z",
            "headBranch": "main", "event": "push",
        }]
        self.detail = {
            "run_started_at": "2026-11-25T16:25:13Z",
            "updated_at": "2026-11-25T17:36:54Z",
        }
        self.timing = {
            "run_duration_ms": 120000,
            "billable": {"UBUNTU": {"total_ms": 60000},
                         "WINDOWS": {"total_ms": 180000}},
        }
        self.calls = []
        self.bad = None
        self.repo_dir = "."

    def runner(self, argv, **kwargs):
        self.assertIsInstance(argv, list)
        self.assertEqual(kwargs, {
            "cwd": self.repo_dir, "capture_output": True, "text": True,
            "timeout": 60,
        })
        self.calls.append(argv)
        if argv[0] == "git":
            return subprocess.CompletedProcess(argv, 0, SHA + "\n", "")
        if self.bad:
            return self.bad
        data = self.runs if argv[1] == "run" else (
            self.timing if argv[-1].endswith("/timing") else self.detail
        )
        return subprocess.CompletedProcess(argv, 0, json.dumps(data), "")

    def check(self, expected, extra=()):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = verify.main(
                ["--repo-dir", ".", "--sha", "main", *extra], run=self.runner
            )
        self.assertEqual(code, expected, output.getvalue())
        return output.getvalue()

    def test_success_exact(self):
        self.assertEqual(self.check(0),
                         "gh-verify: abcdef0123 ci completed success run=123 "
                         "branch=main queue_min=1.0 wall_min=2.0 billable_min=4.0\n")
        self.assertIn("--commit", self.calls[1])
        self.assertIn(SHA, self.calls[1])

    def test_relative_repo_dir_applied_once(self):
        self.repo_dir = "scripts"
        expected_directory = Path("scripts").resolve()

        def relative_runner(argv, **kwargs):
            if argv[0] == "git":
                effective_directory = Path(kwargs["cwd"]).resolve()
                if "-C" in argv:
                    effective_directory /= argv[argv.index("-C") + 1]
                self.assertEqual(effective_directory.resolve(),
                                 expected_directory)
            return self.runner(argv, **kwargs)

        with contextlib.redirect_stdout(io.StringIO()) as output:
            code = verify.main(
                ["--repo-dir", self.repo_dir, "--sha", "main"],
                run=relative_runner,
            )
        self.assertEqual(code, 0, output.getvalue())
        self.assertIn("completed success run=123", output.getvalue())
        self.assertEqual(self.calls[0][0], "git")
        self.assertEqual(len(self.calls), 4)

    def test_failure(self):
        self.runs[0]["conclusion"] = "failure"
        self.assertIn("completed failure", self.check(3))

    def test_cancelled(self):
        self.runs[0]["conclusion"] = "cancelled"
        self.assertIn("completed cancelled", self.check(3))

    def test_in_progress(self):
        self.runs[0].update(status="in_progress", conclusion=None)
        self.detail["run_started_at"] = "2026-11-25T16:24:13Z"
        line = self.check(4)
        self.assertIn("in_progress -", line)
        self.assertIn("wall_min=72.7 billable_min=unknown", line)
        self.assertFalse(any(call[-1].endswith("/timing") for call in self.calls))

    def test_newest(self):
        newer = copy.deepcopy(self.runs[0])
        newer.update(databaseId=456, createdAt="2026-11-25T16:25:00Z")
        self.runs.insert(0, newer)
        self.assertIn("run=456", self.check(0))

    def test_no_run(self):
        self.runs = []
        self.assertEqual(self.check(5), "gh-verify: abcdef0123 ci no run\n")

    def test_gh_error(self):
        self.bad = subprocess.CompletedProcess([], 1, "", "denied\nsecond")
        self.assertEqual(self.check(1), "gh-verify: gh failed: denied\n")

    def test_bad_json(self):
        self.bad = subprocess.CompletedProcess([], 0, "not JSON", "")
        self.assertIn("gh failed: bad JSON", self.check(1))

    def test_missing_billable(self):
        del self.timing["billable"]
        self.assertIn("billable_min=unknown", self.check(0))

    def test_empty_billable(self):
        self.timing["billable"] = {}
        self.assertIn("billable_min=unknown", self.check(0))

    def test_wall_fallback(self):
        del self.timing["run_duration_ms"]
        self.assertIn("wall_min=71.7", self.check(0))

    def test_queued_without_start(self):
        self.runs[0].update(status="queued", conclusion=None)
        self.detail["run_started_at"] = None
        self.assertIn("queue_min=unknown wall_min=unknown", self.check(4))

    def test_log(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "runs.tsv"
            for code, status in [(0, "completed"), (3, "completed"),
                                 (4, "waiting"), (5, None)]:
                if status is None:
                    self.runs = []
                else:
                    self.runs[0].update(status=status, conclusion="failure"
                                        if code == 3 else "success")
                self.check(code, ["--log", str(path)])
            rows = path.read_text(encoding="utf-8").splitlines()
            self.assertEqual(len(rows), 5)
            self.assertEqual(rows[0], "utc\trepo_dir_name\tsha10\trun\tstatus\t"
                             "conclusion\tqueue_min\twall_min\tbillable_min")
            self.assertEqual(rows[1].split("\t")[2:],
                             ["abcdef0123", "123", "completed", "success",
                              "1.0", "2.0", "4.0"])

    def test_unknown_commit(self):
        def unknown(argv, **kwargs):
            return subprocess.CompletedProcess(argv, 1, "", "bad revision")
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(verify.main(
                ["--repo-dir", ".", "--sha", "missing"], run=unknown), 1)
        self.assertEqual(output.getvalue(), "gh-verify: unknown commit missing\n")

    def test_invalid_resolved_commit_length(self):
        calls = []

        def invalid_commit(argv, **kwargs):
            calls.append(argv)
            self.assertEqual(argv[0], "git")
            return subprocess.CompletedProcess(argv, 0, SHA + "ab\n", "")

        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(verify.main(
                ["--repo-dir", ".", "--sha", "main"], run=invalid_commit), 1)
        self.assertEqual(output.getvalue(),
                         "gh-verify: gh failed: invalid resolved commit\n")
        self.assertEqual(len(calls), 1)

    def test_hostile_fields(self):
        self.runs[0]["headBranch"] = "main\rforged"
        self.assertIn("gh failed: invalid branch", self.check(1))

    def test_invalid_timing(self):
        for value in [-1, float("nan"), float("inf"), "120000", True]:
            with self.subTest(value=value):
                self.timing["run_duration_ms"] = value
                self.assertIn("gh failed: invalid milliseconds", self.check(1))

    def test_bad_shape(self):
        self.runs = {"unexpected": []}
        self.assertIn("gh failed: invalid run list", self.check(1))

    def test_negative_elapsed(self):
        self.detail["run_started_at"] = "2026-11-24T00:00:00Z"
        self.assertIn("gh failed: negative elapsed time", self.check(1))

    def test_timeout(self):
        def timeout(argv, **kwargs):
            raise subprocess.TimeoutExpired(argv, 60)
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(verify.main(
                ["--repo-dir", ".", "--sha", "main"], run=timeout), 1)
        self.assertIn("gh failed:", output.getvalue())


if __name__ == "__main__":
    unittest.main()
