"""Tests for scripts/baseline_extras.py. Run: python -I -B scripts/test_baseline_extras.py"""
import sys

sys.dont_write_bytecode = True

import importlib.util
import json
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("baseline_extras_under_test", HERE / "baseline_extras.py")
bx = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bx)


def write_entry(root, name, **fields):
    q = pathlib.Path(root) / "state" / "queue"
    q.mkdir(parents=True, exist_ok=True)
    (q / f"{name}.entry").write_text("".join(f"{k} = {v}\n" for k, v in fields.items()), encoding="utf-8")


class Entries(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        write_entry(self.root, "a", id="a", task="t1", created=1000, ended=1100, status="merged", verdict="PASS",
                    shadow_verdict="PASS", shadow_secs=10, coder="sonnet")
        write_entry(self.root, "b", id="b", task="t1", created=1200, ended=1500, status="rejected", verdict="FAIL",
                    shadow_verdict="ERROR", shadow_secs=30, shadow_model_used="m", shadow_reason="boom", coder="sonnet")
        write_entry(self.root, "c", id="c", task="t2", created=2000, ended=2400, status="merged", verdict="PASS", coder="haiku")
        write_entry(self.root, "bad", id="bad", task="t3", created="nope")  # no numeric created: skipped
        write_entry(self.root, "old", id="old", task="t4", created=10, ended=20, status="merged")  # before the window

    def tearDown(self):
        self.tmp.cleanup()

    def rows(self, since=500, until=5000):
        return bx.in_window(bx.load_entries(self.root), since, until)

    def test_window_and_bad_created_skipped(self):
        self.assertEqual([r[1] for r in self.rows()], ["a.entry", "b.entry", "c.entry"])

    def test_window_end_is_exclusive(self):
        self.assertEqual([r[1] for r in self.rows(1000, 2000)], ["a.entry", "b.entry"])

    def test_shadow_counts_and_error_rate(self):
        text = "\n".join(bx.shadow_lines(self.rows()))
        self.assertIn("entries with a shadow verdict: 2", text)
        self.assertIn("entries with no shadow verdict: 1", text)
        self.assertIn("ERROR rate among shadow runs: 1 of 2 = 50.0%", text)
        self.assertIn("shadow_secs: n=2 median=20.0 min=10.0 max=30.0", text)
        self.assertIn("ERROR entry: b model m secs 30 reason: boom", text)

    def test_first_occurrence_of_a_field_wins(self):
        q = pathlib.Path(self.root) / "state" / "queue" / "dup.entry"
        q.write_text("created = 1000\nstatus = first\nstatus = second\n", encoding="utf-8")
        self.assertEqual(bx.read_fields(q)["status"], "first")

    def test_entries_split_on_off_and_cutoff(self):
        text = "\n".join(bx.entries_lines(self.rows(), 1500))
        self.assertIn("SHADOW-ON (shadow_verdict present): entries=2 with_ended=2 median_s=200.0", text)
        self.assertIn("SHADOW-OFF (no shadow_verdict): entries=1 with_ended=1 median_s=400.0", text)
        self.assertIn("created at/after the cutoff: entries=1", text)
        self.assertIn("created after cutoff but with shadow_verdict: 0", text)
        self.assertIn("t1\tcoder=sonnet\trounds=2\tmerged:PASS,rejected:FAIL", text)

    def test_rounds_counts_review_entries_of_merged_tasks(self):
        rows = bx.load_entries(self.root)
        text = "\n".join(bx.rounds_lines(rows, 500, 5000))
        self.assertIn("tasks with a merged entry ended in window: 2", text)
        self.assertIn("t1\tcoder=sonnet\tentries=2\treview_entries=2", text)
        self.assertNotIn("t4", text)  # merged before the window


class V2(unittest.TestCase):
    def test_v2_excludes_unmerged_tokens(self):
        # the recorded pre-codex-coder numbers: v1 422877 over 2 merged tasks, 317950 tokens of unmerged tasks
        v = bx.coder_tokens_v2({"coder_tokens_per_merged": 422877.0, "coder_tokens_unmerged": 317950},
                               {"merged_with_coder_run": 2})
        self.assertEqual(v, 263902.0)

    def test_v2_none_without_merged_coder_runs(self):
        self.assertIsNone(bx.coder_tokens_v2({"coder_tokens_per_merged": None, "coder_tokens_unmerged": None},
                                             {"merged_with_coder_run": 0}))

    def test_v2_with_no_unmerged_equals_v1(self):
        self.assertEqual(bx.coder_tokens_v2({"coder_tokens_per_merged": 100.0, "coder_tokens_unmerged": 0},
                                            {"merged_with_coder_run": 3}), 100.0)


class FakeBase:
    """Stand-in for baseline.py's measurement only (the state it reads is a whole tree); the file writing is the real code."""
    NAME_RE = __import__("re").compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,80}$")
    SCHEMA = 1
    DEFINITIONS = {"coder_tokens_per_merged": "v1", "merged_tasks": "v0"}

    class InputError(Exception):
        pass

    @staticmethod
    def baselines_dir(root):
        return root / "state" / "baselines"

    @staticmethod
    def iso(epoch):
        return "T%d" % epoch

    @staticmethod
    def check_finite(values, per_model):
        pass

    @staticmethod
    def measure(root, since, until):
        return ({"coder_tokens_per_merged": 422877.0, "coder_tokens_unmerged": 317950, "merged_tasks": 20},
                {"merged_tasks": 20, "merged_with_coder_run": 2}, {}, "abc123", ["a note"])


class RecordV2(unittest.TestCase):
    def test_writes_v2_record_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "broadsword.toml").write_text("root = \".\"\n", encoding="utf-8")
            out, data = bx.record_v2(root, "x-v2", 0, 3600, FakeBase)
            on_disk = json.loads(out.read_text(encoding="utf-8"))
            self.assertEqual(on_disk["metrics"]["coder_tokens_per_merged"], 263902.0)
            self.assertEqual(on_disk["definitions"]["coder_tokens_per_merged"], "v2")
            self.assertEqual(on_disk["definitions"]["merged_tasks"], "v0")
            self.assertTrue(any("422877" in n for n in on_disk["notes"]))
            before = out.read_text(encoding="utf-8")
            with self.assertRaises(bx.InputError):
                bx.record_v2(root, "x-v2", 0, 7200, FakeBase)
            self.assertEqual(out.read_text(encoding="utf-8"), before)
            self.assertEqual([p.name for p in out.parent.iterdir()], ["x-v2.json"])  # no temp file left

    def test_bad_name_refused(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(bx.InputError):
                bx.record_v2(pathlib.Path(d), "../evil", 0, 10, FakeBase)


if __name__ == "__main__":
    unittest.main()
