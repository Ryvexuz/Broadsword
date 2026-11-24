"""Tests for the reusable functions and the cost-to-completion table of scripts/dev_metrics.py.
They use made-up records (a scratch folder stands in for state/), never the real state/.

  python -I -B scripts/test_dev_metrics.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import os
import pathlib
import re
import subprocess
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
    def test_fix_rounds_count_only_entries_after_a_rejection(self):
        for statuses, expected in (
            (("check_failed", "rejected", "merged"), 1),
            (("rejected", "rejected", "merged"), 2),
            (("perf_failed", "merged_check_failed", "conflict", "cancelled", "merged"), 0),
        ):
            with self.subTest(statuses=statuses):
                entries = [entry(f"q-{i:04d}", "t1", status=status, created=str(100 - i))
                           for i, status in enumerate(statuses, 1)]
                r = self.costs([task("t1")], list(reversed(entries)), [])["t1"]
                self.assertEqual(r["rounds"], len(statuses))
                self.assertEqual(r["fix_rounds"], expected)
                out = self.completion_text([task("t1")], entries, [])
                self.assertRegex(out, rf"fix rounds\s+{expected} / {expected}")

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

    def test_adjudication_matches_entry_ids_case_insensitively_and_trimmed(self):
        entries = [entry("Q-0001", "t1", "FAIL", blocking="a:1 - x")]
        verdicts = [{"entry": " q-0001 ", "verdict": "real"}]
        rows = self.costs([task("t1")], entries, [], verdicts)
        self.assertEqual(rows["t1"]["first_adjudicated"], ["real"])

    def test_coder_dollars_are_summed_when_recorded_and_unknown_never_zero_when_not(self):
        priced = [dict(run("t1", "100"), cost_usd="0.50"), dict(run("t1", "100", role="fix"), cost_usd="0.25")]
        r = self.costs([task("t1")], [entry("q-0001", "t1")], priced)["t1"]
        self.assertEqual(r["coder_usd"], [0.75, 0])
        r = self.costs([task("t1")], [entry("q-0001", "t1")], [run("t1", "100"), dict(run("t1", "5"), cost_usd="0.5")])["t1"]
        self.assertEqual(r["coder_usd"], [0.5, 1])  # one run has no price: counted unknown, not 0
        out = self.completion_text([task("t1")], [entry("q-0001", "t1")], priced)
        self.assertRegex(out, r"coder \$\s+0\.7500 / 0\.7500")
        self.assertRegex(out, r"total \$\s+0\.7600 / 0\.7600")  # 0.75 coder + 0.01 review
        self.assertRegex(out, r"review \$\s+0\.0100")

    def test_cost_usd_column_read_from_the_runs_file_priced_and_unpriced_tasks(self):
        # a real agent-runs.tsv with the optional cost_usd column: t1 is priced on two runs, t2 has an empty price
        head = HEAD.rstrip("\n") + "\tcost_usd\n"
        with scratch_tmp() as d:
            old = dm.ROOT
            root = pathlib.Path(d)
            (root / "state").mkdir()
            (root / "state" / "agent-runs.tsv").write_text(
                head + "2026-11-19\tcoder\tsonnet\tt1\t100\t60\tdone\t\t0.40\n"
                "2026-11-19\tfix\tsonnet\tt1\t50\t30\tdone\t\t0.10\n"
                "2026-11-19\tcoder\tsonnet\tt2\t100\t60\tdone\t\t\n", encoding="utf-8")
            dm.ROOT = root
            try:
                runs = dm.read_runs(None)
            finally:
                dm.ROOT = old
        tasks = [task("t1"), task("t2")]
        entries = [entry("q-0001", "t1"), entry("q-0002", "t2")]
        by = {}
        for e in entries:
            by.setdefault(e["task"], []).append(e)
        rows = {r["id"]: r for r in dm.task_costs(tasks, by, runs, [])}
        self.assertEqual(rows["t1"]["coder_usd"], [0.5, 0])
        self.assertEqual(rows["t2"]["coder_usd"], [0.0, 1])  # empty price: unknown (counted), never a silent $0
        out = self.completion_text(tasks, entries, runs)
        self.assertRegex(out, r"coder \$\s+0\.5000 / 0\.5000.*known for 1 of 2 done tasks.*unknown for 1")
        self.assertRegex(out, r"coder \$[^\n]*\n\s+review \$\s+0\.0100 / 0\.0100")  # coder $ is printed right above review $
        self.assertRegex(out, r"total \$\s+0\.5100 / 0\.5100.*only for the 1 of 2")  # t2 has no total

    def test_total_is_unknown_not_review_only_when_coder_dollars_are_missing(self):
        out = self.completion_text([task("t1")], [entry("q-0001", "t1")], [run("t1", "100")])
        self.assertRegex(out, r"coder \$\s+- / -.*known for 0 of 1 done tasks.*never counted as \$0")
        self.assertRegex(out, r"total \$\s+- / -.*no total")
        self.assertNotRegex(out, r"total \$\s+0\.0100")

    def completion_text(self, tasks, entries, runs=()):
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            dm.completion(tasks, entries, list(runs))
        return buf.getvalue()

    def test_first_pass_is_reported_per_gating_reviewer_never_mixed(self):
        tasks = [task("t1"), task("t2")]
        entries = [entry("q-0001", "t1", "PASS", reviewer_model="haiku"),
                   entry("q-0002", "t2", "FAIL", reviewer_model="opus", blocking="a:1 - x")]
        rows = self.costs(tasks, entries, [])
        self.assertEqual(rows["t1"]["first_reviewer"], "haiku")
        out = self.completion_text(tasks, entries)
        self.assertIn("first-pass PASS under haiku  1/1", out)
        self.assertIn("first-pass PASS under opus  0/1", out)
        self.assertNotIn("first-pass PASS     1/2", out)

    def test_review_without_a_price_is_counted_not_silently_dropped(self):
        tasks = [task("t1")]
        entries = [entry("q-0001", "t1", "FAIL", cost_usd=""), entry("q-0002", "t1")]
        rows = self.costs(tasks, entries, [])
        self.assertEqual(rows["t1"]["usd_unknown"], 1)
        self.assertIn("1 review(s) without a price", self.completion_text(tasks, entries))


class TrainMemberCommits(unittest.TestCase):
    """A train member's merged entry has base_head older than the train below it: main_sha includes the other members'
    commits. Only the member's own commits (train_up..main_sha) may count as its coding time."""

    def git(self, repo, *args):
        return subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, text=True).stdout.strip()

    def commit(self, repo, msg, when):
        env = dict(os.environ, GIT_AUTHOR_DATE=f"{when} +0000", GIT_COMMITTER_DATE=f"{when} +0000")
        subprocess.run(["git", "-C", str(repo), "-c", "user.name=test", "-c", "user.email=test@example.invalid",
                        "commit", "-q", "--allow-empty", "-m", msg], check=True, capture_output=True, env=env)
        return self.git(repo, "rev-parse", "HEAD")

    def test_train_member_counts_only_its_own_commits(self):
        T = 1795150000  # task created and started at T
        with scratch_tmp() as d:
            old = dm.ROOT
            root = pathlib.Path(d)
            try:
                self.git(root, "init", "-q")
                base = self.commit(root, "base", T - 50)
                other = self.commit(root, "other task, train member below", T + 900)  # newer author time: wrong range would show it
                own = self.commit(root, "own task commit", T + 100)
                (root / "state" / "tasks").mkdir(parents=True)
                (root / "state" / "queue").mkdir()
                (root / "state" / "tasks" / "t-test-train.task").write_text(
                    f"id = t-test-train\nstatus = done\nproject = broadsword\ntitle = train member\n"
                    f"created = {T}\nstarted = {T}\nended = {T + 1000}\n", encoding="utf-8")
                (root / "state" / "queue" / "q-test-train.entry").write_text(
                    f"id = q-test-train\ntask = t-test-train\nstatus = merged\nproject = broadsword\nverdict = PASS\n"
                    f"created = {T + 200}\nended = {T + 300}\ncoder = sonnet\nreviewer_model = haiku\n"
                    f"base_head = {base}\nmain_sha = {own}\ntrain_up = {other}\n", encoding="utf-8")
                dm.ROOT = root
                buf = io.StringIO()
                with contextlib.redirect_stdout(buf):
                    dm.main()
            finally:
                dm.ROOT = old
        row = next(ln.split() for ln in buf.getvalue().splitlines() if ln.startswith("t-test-train"))
        # columns: id, project, status, lead, coding, ...; coding = own commit (T+100) - start (T) = 1m40s
        self.assertEqual(row[4], "1m40s", buf.getvalue())


if __name__ == "__main__":
    unittest.main()
