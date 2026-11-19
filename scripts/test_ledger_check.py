"""Tests for scripts/ledger_check.py, against fixture RETRO.md files in a scratch git repo (never the real one).

  python -I -B scripts/test_ledger_check.py -v
"""
import sys

sys.dont_write_bytecode = True

import datetime as dt
import hashlib
import json
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
HEAD = "| Since | Change | Should improve | Measured by | Watch for harm | Status |\n| --- | --- | --- | --- | --- | --- |\n"

def metric_names():
    import importlib.util
    spec = importlib.util.spec_from_file_location("_bl", HERE / "baseline.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return list(mod.METRICS)


def today():
    return dt.datetime.now(dt.timezone.utc).date()


class Repo:
    def __init__(self, tmp):
        self.root = pathlib.Path(tmp) / "bs"
        (self.root / "scripts").mkdir(parents=True)
        for name in ("baseline.py", "ledger_check.py", "dev_metrics.py", "review_costs.py"):
            shutil.copy(HERE / name, self.root / "scripts" / name)
        (self.root / "state" / "baselines").mkdir(parents=True)
        (self.root / "broadsword.toml").write_text("x = 1\n", encoding="utf-8")
        self.git("init", "-q", "-b", "main")
        self.git("add", "broadsword.toml")
        self.commit("init", today() - dt.timedelta(days=30))

    def git(self, *args, date=None):
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
        if date:
            env["GIT_AUTHOR_DATE"] = env["GIT_COMMITTER_DATE"] = f"{date.isoformat()}T12:00:00Z"
        subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True, env=env)

    def commit(self, msg, date):
        self.git("commit", "-qm", msg, date=date)

    def retro(self, rows, commit_date=None, header=HEAD):
        body = "# Retro\n\n## Improvement ledger\n\nIntro text.\n\n" + header + "".join(r + "\n" for r in rows) + "\n## Retros, newest first\n"
        (self.root / "RETRO.md").write_bytes(body.replace("\n", "\r\n").encode("utf-8"))
        if commit_date:
            self.git("add", "RETRO.md")
            self.commit("retro", commit_date)

    def baseline(self, name, merged, per_hour, **more):
        values = {m: None for m in metric_names()}
        values.update(merged_tasks=merged, merged_per_hour=per_hour, **more)
        data = {"schema": 1, "name": name, "window": {"since": "a", "until": "b", "hours": 1}, "main_commit": "0" * 40,
                "broadsword_toml_sha256": "f" * 64, "metrics": values, "samples": {"merged_tasks": merged}, "per_model": {}, "notes": []}
        (self.root / "state" / "baselines" / f"{name}.json").write_text(json.dumps(data), encoding="utf-8")

    def run(self, since=None, max_days="7", defaults=False):
        since = since or (today() - dt.timedelta(days=60)).isoformat()
        extra = [] if defaults else ["--min-samples", "5", "--noise", "10"]
        return subprocess.run([sys.executable, "-I", "-B", str(self.root / "scripts" / "ledger_check.py"), "--since", since,
                               "--max-pending-days", max_days, *extra],
                              capture_output=True, text=True, timeout=120)


def row(since, change, status):
    return f"| {since} | {change} | throughput | baseline.py | cost | {status} |"


class LedgerTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(ignore_cleanup_errors=True)
        self.r = Repo(self._tmp.name)
        self.r.baseline("pre", 20, 3.0)
        self.r.baseline("post-worse", 20, 2.0)
        self.r.baseline("post-better", 20, 4.0)
        self.r.baseline("pre-lock", 20, 3.0, test_job_median_s=80.0)
        self.r.baseline("post-lock", 20, 6.0, test_job_median_s=100.0)
        self.r.baseline("post-slower", 20, 2.0, test_job_median_s=100.0)

    def tearDown(self):
        self._tmp.cleanup()

    def test_better_and_pending_rows_pass(self):
        due = (today() + dt.timedelta(days=3)).isoformat()
        self.r.retro([row("S5", "Faster queue", "open, before: pre after: post-better"),
                      row("S5", "New check", f"open, before: `pre` after: pending ({due})")])
        res = self.r.run()
        self.assertEqual(res.returncode, 0, res.stdout + res.stderr)
        self.assertIn("pre -> post-better: BETTER", res.stdout)
        self.assertIn("ledger: ok", res.stdout)

    def test_worse_row_not_reverted_fails(self):
        self.r.retro([row("S5", "Bigger batches", "open, before: pre after: post-worse")])
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("WORSE and not marked", res.stdout)
        self.assertIn("ledger: 1 problem", res.stdout)

    def test_negated_or_future_marker_does_not_count(self):
        for status in ("open, not reverted yet", "open, will be fixed", "open, to be reverted if it hurts",
                       "open, never reverted in abc1234", "open, not fixed in abc1234"):
            with self.subTest(status=status):
                self.r.retro([row("S5", "Bigger batches", f"{status}; before: pre after: post-worse")])
                res = self.r.run()
                self.assertEqual(res.returncode, 1, res.stdout)
                self.assertIn("not marked", res.stdout)

    def test_clear_markers_count(self):
        for status in ("reverted in abc1234; before: pre after: post-worse", "fixed, before: pre after: post-worse",
                       "open, reverted in abc1234; before: pre after: post-worse",
                       "hindered: slower, reverted; before: pre after: post-worse"):
            with self.subTest(status=status):
                self.r.retro([row("S5", "Bigger batches", status)])
                res = self.r.run()
                self.assertEqual(res.returncode, 0, res.stdout)
                self.assertIn("marked", res.stdout)

    def test_defaults_when_flags_omitted(self):
        self.r.retro([row("S5", "Bigger batches", "open, before: pre after: post-worse")])
        res = self.r.run(defaults=True)
        self.assertEqual(res.returncode, 1, res.stdout + res.stderr)
        self.assertIn("WORSE", res.stdout)

    def test_row_removed_and_readded_is_dated_by_the_readd(self):
        r = row("S5", "Comes back", "open, before: pre after: post-better")
        self.r.retro([r], commit_date=today() - dt.timedelta(days=20))
        self.r.retro([], commit_date=today() - dt.timedelta(days=10))
        self.r.retro([r], commit_date=today() - dt.timedelta(days=1))
        res = self.r.run(since=(today() - dt.timedelta(days=5)).isoformat())
        self.assertIn("checked 1 of 1", res.stdout)

    def test_worse_row_marked_reverted_passes(self):
        self.r.retro([row("S5", "Bigger batches", "hindered, reverted in abc123; before: pre after: post-worse")])
        res = self.r.run()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("marked reverted", res.stdout)

    def test_scale_up_judged_on_throughput_lock_wait_is_a_warning(self):
        self.r.retro([row("S5", "max_agents 6 -> 10", "open, before: pre-lock after: post-lock")])
        res = self.r.run()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("warnings: test_job_median_s", res.stdout)

    def test_scale_up_with_falling_throughput_fails(self):
        self.r.retro([row("S5", "max_agents 6 -> 10", "open, before: pre-lock after: post-slower")])
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("not marked `reverted`", res.stdout)

    def test_same_lock_wait_rise_fails_a_non_scale_up_row(self):
        self.r.retro([row("S5", "Bigger batches", "open, before: pre-lock after: post-lock")])
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("WORSE and not marked", res.stdout)

    def test_metrics_filter_is_used(self):
        self.r.retro([row("S5", "Bigger batches", "open, before: pre after: post-worse metrics: lead_median_s")])
        res = self.r.run()
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("NOISE", res.stdout)

    def test_stale_pending_row_fails(self):
        due = (today() + dt.timedelta(days=30)).isoformat()
        self.r.retro([row("S5", "Old change", f"open, before: pre after: pending {due}")], commit_date=today() - dt.timedelta(days=10))
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("pending for 10 days", res.stdout)

    def test_pending_past_its_date_fails(self):
        due = (today() - dt.timedelta(days=1)).isoformat()
        self.r.retro([row("S5", "Late change", f"open, before: pre after: pending {due}")])
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("still pending", res.stdout)

    def test_pending_without_date_fails(self):
        self.r.retro([row("S5", "Vague change", "open, before: pre after: pending")])
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("no date", res.stdout)

    def test_row_without_baselines_fails_and_planned_is_skipped(self):
        self.r.retro([row("S5", "Unmeasured", "helped"), row("planned", "Later idea", "planned")])
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("names no baselines", res.stdout)
        self.assertIn("skipped (planned", res.stdout)
        self.assertIn("ledger: 1 problem", res.stdout)

    def test_missing_baseline_file_is_named(self):
        self.r.retro([row("S5", "Ghost", "open, before: nothere after: post-better")])
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("nothere.json", res.stdout)

    def test_rows_added_before_since_are_not_checked(self):
        self.r.retro([row("S1", "Ancient", "helped")], commit_date=today() - dt.timedelta(days=20))
        res = self.r.run(since=(today() - dt.timedelta(days=5)).isoformat())
        self.assertEqual(res.returncode, 0, res.stdout)
        self.assertIn("checked 0 of 1", res.stdout)

    def test_changed_header_fails_loudly(self):
        self.r.retro([row("S5", "x", "before: pre after: post-better")],
                     header="| Since | Change | Status |\n| --- | --- | --- |\n")
        res = self.r.run()
        self.assertEqual(res.returncode, 1)
        self.assertIn("expected exactly", res.stderr)
        self.assertIn("Watch for harm", res.stderr)

    def test_never_edits_retro(self):
        self.r.retro([row("S5", "Bigger batches", "open, before: pre after: post-worse")])
        before = hashlib.sha256((self.r.root / "RETRO.md").read_bytes()).hexdigest()
        self.r.run()
        self.assertEqual(before, hashlib.sha256((self.r.root / "RETRO.md").read_bytes()).hexdigest())

    def test_required_flags(self):
        res = subprocess.run([sys.executable, "-I", "-B", str(self.r.root / "scripts" / "ledger_check.py"), "--since", "2026-11-18"],
                             capture_output=True, text=True, timeout=60)
        self.assertEqual(res.returncode, 2)


if __name__ == "__main__":
    unittest.main()
