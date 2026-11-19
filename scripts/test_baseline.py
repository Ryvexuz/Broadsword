"""Tests for scripts/baseline.py. Each test builds a scratch Broadsword root (git repo, broadsword.toml, fake state/)
and runs a copy of the scripts there, so the real state/ is never read or written.

  python -I -B scripts/test_baseline.py -v
"""
import sys

sys.dont_write_bytecode = True

import calendar
import importlib.util
import json
import os
import pathlib
import re
import shutil
import subprocess
import tempfile
import time
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPTS = ("baseline.py", "dev_metrics.py", "review_costs.py")


def load_baseline():
    spec = importlib.util.spec_from_file_location("baseline_under_test", HERE / "baseline.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


bl = load_baseline()


def scratch_tmp():
    """A temp folder under the project's scratch folder (resolved the way baseline.py finds the root), removed by cleanup()."""
    root = bl.main_root()
    m = re.search(r'^scratch_dir\s*=\s*"([^"]+)"', (root / "broadsword.toml").read_text(encoding="utf-8"), re.M)
    if not m:
        raise RuntimeError("broadsword.toml has no scratch_dir")
    base = root / m.group(1)
    base.mkdir(parents=True, exist_ok=True)
    return tempfile.TemporaryDirectory(dir=base, prefix="test-baseline-", ignore_cleanup_errors=True)


def epoch(text):
    return calendar.timegm(time.strptime(text, "%Y-%m-%d %H:%M:%S"))


def stamp(t):
    return time.strftime("%Y-%m-%d %H:%M:%S", time.gmtime(t))


def git(root, *args):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    subprocess.run(["git", "-C", str(root), *args], check=True, capture_output=True, env=env)


class Root:
    """A scratch main checkout with its own copy of the scripts."""

    def __init__(self, tmp):
        self.root = pathlib.Path(tmp) / "bs"
        (self.root / "scripts").mkdir(parents=True)
        for name in SCRIPTS:
            shutil.copy(HERE / name, self.root / "scripts" / name)
        for d in ("tasks", "queue", "jobs"):
            (self.root / "state" / d).mkdir(parents=True)
        (self.root / "broadsword.toml").write_text("max_agents = 6\n", encoding="utf-8")
        (self.root / "state" / "agent-runs.tsv").write_text("date_utc\trole\tmodel\ttask\ttokens\tseconds\toutcome\tnote\n", encoding="utf-8")
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "add", "broadsword.toml")
        git(self.root, "commit", "-qm", "init")
        self.n = 0

    def write(self, rel, fields):
        (self.root / rel).write_text("".join(f"{k} = {v}\n" for k, v in fields.items()), encoding="utf-8")

    def merged_task(self, start, end, verdicts=("PASS",), coder_tokens=None, model="opus", cost="0.10"):
        """A task started at `start`, merged at `end`, with one queue entry (and review) per verdict, the last PASS."""
        self.n += 1
        tid = f"t-x-{self.n}"
        self.write(f"state/tasks/{tid}.task", {"id": tid, "project": "broadsword", "status": "done", "created": start,
                                              "started": start, "ended": end, "title": f"task {self.n}"})
        span = (end - start) // (len(verdicts) + 1)
        for i, v in enumerate(verdicts):
            eid = f"q-x-{self.n}-{i}"
            c = start + span * (i + 1)
            log = "\\n".join([f"{stamp(c)} UTC check: begin", f"{stamp(c + 10)} UTC review: begin",
                              f"{stamp(c + 15)} UTC main_check: begin", f"{stamp(c + 20)} UTC done: begin"])
            last = i == len(verdicts) - 1
            self.write(f"state/queue/{eid}.entry", {
                "id": eid, "task": tid, "created": c, "ended": c + 20, "status": "merged" if last else "rejected",
                "verdict": v, "attempt": 1, "review_file": f"state/queue/{eid}.review", "reviewer_model": model,
                "test_hook": "false", "log": log, "cost_usd": cost})
            (self.root / f"state/queue/{eid}.review").write_text(
                f"entry: {eid}\nattempt: 1\ntask: {tid}\ntest_hook: false\nmodel: {model}\ncost_usd: {cost}\n"
                f"input tokens: 1000\noutput tokens: 100\nverdict: {v} (answer followed the format: yes)\n"
                "--- raw reviewer output (exactly as printed) ---\n{}\n", encoding="utf-8")
        if coder_tokens is not None:
            with open(self.root / "state" / "agent-runs.tsv", "a", encoding="utf-8") as f:
                f.write(f"{time.strftime('%Y-%m-%d', time.gmtime(start))}\tcoder\tsonnet\t{tid}\t{coder_tokens}\t60\tdone\tx\n")
        return tid

    def run(self, *args):
        return subprocess.run([sys.executable, "-I", "-B", str(self.root / "scripts" / "baseline.py"), *args],
                              capture_output=True, text=True, timeout=120)

    def put_baseline(self, name, merged, per_hour, definitions=None, notes=(), samples=None, **metrics):
        d = self.root / "state" / "baselines"
        d.mkdir(parents=True, exist_ok=True)
        values = {m: None for m in bl.METRICS}
        values.update(merged_tasks=merged, merged_per_hour=per_hour, **metrics)
        data = {"schema": 1, "name": name, "window": {"since": "a", "until": "b", "hours": 1}, "main_commit": "0" * 40,
                "broadsword_toml_sha256": "f" * 64, "metrics": values, "samples": dict({"merged_tasks": merged}, **(samples or {})), "per_model": {}, "notes": list(notes)}
        if definitions is not None:
            data["definitions"] = definitions
        (d / f"{name}.json").write_text(json.dumps(data), encoding="utf-8")


T0 = epoch("2026-11-18 09:00:00")


class RecordTests(unittest.TestCase):
    def setUp(self):
        self._tmp = scratch_tmp()
        self.r = Root(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def record(self, name="b1"):
        return self.r.run("record", name, "--since", "2026-11-18T09:00:00Z", "--until", "2026-11-18T11:00:00Z")

    def test_record_measures_the_window(self):
        self.r.merged_task(T0 + 600, T0 + 1800, verdicts=("FAIL", "PASS"), coder_tokens=70000)
        self.r.merged_task(T0 + 900, T0 + 2700, verdicts=("PASS",), coder_tokens=90000)
        self.r.merged_task(T0 - 7200, T0 - 3600)  # merged before the window: not counted
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        data = json.loads((self.r.root / "state" / "baselines" / "b1.json").read_text(encoding="utf-8"))
        m = data["metrics"]
        self.assertEqual(m["merged_tasks"], 2)
        self.assertAlmostEqual(m["merged_per_hour"], 1.0)
        self.assertEqual(m["lead_median_s"], 1500)
        self.assertAlmostEqual(m["review_rounds_per_merged"], 1.5)
        self.assertAlmostEqual(m["first_pass_fail_rate"], 0.5)
        self.assertAlmostEqual(m["review_usd_per_merged"], 0.15)
        self.assertEqual(m["coder_tokens_per_merged"], 80000)
        self.assertEqual(m["check_median_s"], 10)
        self.assertEqual(m["review_median_s"], 5)
        self.assertEqual(m["recheck_median_s"], 5)
        self.assertIsNone(m["build_lock_wait_s"])
        self.assertEqual(len(data["main_commit"]), 40)
        self.assertEqual(len(data["broadsword_toml_sha256"]), 64)
        self.assertIn("reviewer:opus", data["per_model"])

    def test_record_never_overwrites(self):
        self.assertEqual(self.record().returncode, 0)
        res = self.record()
        self.assertEqual(res.returncode, 1)
        self.assertIn("never overwritten", res.stderr)

    def test_review_without_cost_is_counted_apart_not_as_zero(self):
        self.r.merged_task(T0 + 600, T0 + 1800, cost="0.20")
        self.r.merged_task(T0 + 900, T0 + 2700, cost="0.40")
        rev = sorted((self.r.root / "state" / "queue").glob("*.review"))[1]
        rev.write_text("".join(l for l in rev.read_text(encoding="utf-8").splitlines(True) if not l.startswith("cost_usd")), encoding="utf-8")
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        data = json.loads((self.r.root / "state" / "baselines" / "b1.json").read_text(encoding="utf-8"))
        self.assertEqual(data["samples"]["reviews_without_cost"], 1)
        self.assertIn("1 of 2 reviews have no cost recorded", " ".join(data["notes"]))
        self.assertIn("1 of 2 reviews have no cost recorded", res.stdout)

    def test_malformed_coder_run_number_is_an_input_error_naming_file_and_line(self):
        self.r.merged_task(T0 + 600, T0 + 1800, coder_tokens=1000)
        for col, bad in (("tokens", "1,234"), ("seconds", "abc")):
            runs = self.r.root / "state" / "agent-runs.tsv"
            good = runs.read_text(encoding="utf-8")
            cells = good.splitlines()[1].split("\t")
            cells[4 if col == "tokens" else 5] = bad
            runs.write_text(good + "\t".join(cells) + "\n", encoding="utf-8")
            res = self.record(f"bad-{col}")
            runs.write_text(good, encoding="utf-8")
            self.assertEqual(res.returncode, 1, f"{col}: {res.stdout} {res.stderr}")
            self.assertIn("agent-runs.tsv line 3", res.stderr)
            self.assertIn(bad, res.stderr)
            self.assertNotIn("Traceback", res.stderr)

    def test_empty_coder_token_cell_is_counted_in_a_note_not_a_silent_zero(self):
        self.r.merged_task(T0 + 600, T0 + 1800, coder_tokens="")
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("1 coder runs have no token count", res.stdout)

    def test_malformed_review_token_count_is_an_input_error(self):
        self.r.merged_task(T0 + 600, T0 + 1800)
        rev = next((self.r.root / "state" / "queue").glob("*.review"))
        rev.write_text(rev.read_text(encoding="utf-8").replace("input tokens: 1000", "input tokens: 1,000"), encoding="utf-8")
        res = self.record()
        self.assertEqual(res.returncode, 1, res.stdout)
        self.assertIn(rev.name, res.stderr)

    def test_review_without_entry_time_is_counted_in_a_note(self):
        self.r.merged_task(T0 + 600, T0 + 1800)
        (self.r.root / "state" / "queue" / "q-x-orphan.review").write_text(
            "entry: q-x-orphan\nattempt: 1\ntask: t-x-1\ntest_hook: false\nmodel: opus\ncost_usd: 0.1\n"
            "input tokens: 1\noutput tokens: 1\nverdict: FAIL (answer followed the format: yes)\n", encoding="utf-8")
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("1 review(s) of merged tasks have no queue entry or no created time", res.stdout)

    def test_unfinished_coder_work_is_left_out_of_coder_tokens(self):
        self.r.merged_task(T0 + 600, T0 + 1800, coder_tokens=1000)
        self.r.write("state/tasks/t-open.task", {"id": "t-open", "project": "broadsword", "status": "running",
                                                 "created": T0 + 900, "started": T0 + 900, "title": "still running"})
        with open(self.r.root / "state" / "agent-runs.tsv", "a", encoding="utf-8") as f:
            f.write("2026-11-18\tcoder\tsonnet\tt-open\t999000\t60\tdone\tx\n")
        self.assertEqual(self.record().returncode, 0)
        data = json.loads((self.r.root / "state" / "baselines" / "b1.json").read_text(encoding="utf-8"))
        self.assertEqual(data["metrics"]["coder_tokens_per_merged"], 1000)
        self.assertEqual(data["metrics"]["coder_runs_recorded"], 1)

    def test_merged_tasks_without_a_coder_run_are_not_counted_as_zero_tokens(self):
        a = self.r.merged_task(T0 + 600, T0 + 1800)
        self.r.merged_task(T0 + 700, T0 + 1900)  # merged, no coder run recorded
        self.runs((a, 40000, 60))
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        m = self.metrics()
        self.assertEqual(m["coder_tokens_per_merged"], 40000)
        data = json.loads((self.r.root / "state" / "baselines" / "b1.json").read_text(encoding="utf-8"))
        self.assertEqual(data["samples"]["merged_with_coder_run"], 1)
        self.assertEqual(data["samples"]["merged_tasks"], 2)
        self.assertIn("1 of 2 merged tasks have no coder run recorded", res.stdout)

    def test_no_merged_task_with_a_coder_run_gives_null_coder_tokens(self):
        self.r.merged_task(T0 + 600, T0 + 1800)
        self.assertEqual(self.record().returncode, 0)
        self.assertIsNone(self.metrics()["coder_tokens_per_merged"])

    def test_record_writes_the_definitions_map(self):
        self.assertEqual(self.record().returncode, 0)
        data = json.loads((self.r.root / "state" / "baselines" / "b1.json").read_text(encoding="utf-8"))
        self.assertEqual(set(data["definitions"]), set(bl.METRICS))
        self.assertEqual(data["definitions"]["coder_tokens_per_merged"], "v1")

    def test_shadow_reviews_are_not_counted_anywhere(self):
        self.r.merged_task(T0 + 600, T0 + 1800, model="opus")
        rev = next((self.r.root / "state" / "queue").glob("*.review"))
        shadow = rev.with_name(rev.name[:-len(".review")] + ".shadow.review")
        shadow.write_text(rev.read_text(encoding="utf-8").replace("model: opus", "model: sol"), encoding="utf-8")
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        data = json.loads((self.r.root / "state" / "baselines" / "b1.json").read_text(encoding="utf-8"))
        self.assertEqual(data["samples"]["reviews"], 1)
        self.assertNotIn("reviewer:sol", data["per_model"])
        self.assertAlmostEqual(data["metrics"]["review_rounds_per_merged"], 1.0)
        self.assertAlmostEqual(data["metrics"]["review_usd_per_merged"], 0.10)

    def runs(self, *rows):
        with open(self.r.root / "state" / "agent-runs.tsv", "a", encoding="utf-8") as f:
            for tid, tok, secs in rows:
                f.write(f"2026-11-18\tcoder\tsonnet\t{tid}\t{tok}\t{secs}\tdone\tx\n")

    def metrics(self):
        return json.loads((self.r.root / "state" / "baselines" / "b1.json").read_text(encoding="utf-8"))["metrics"]

    def test_every_coder_run_of_a_merged_task_counts_retries_included(self):
        tid = self.r.merged_task(T0 + 600, T0 + 1800)
        self.runs((tid, 40000, 60), (tid, 25000, 60))
        self.assertEqual(self.record().returncode, 0)
        m = self.metrics()
        self.assertEqual(m["coder_tokens_per_merged"], 65000)
        self.assertEqual(m["coder_runs_recorded"], 2)

    def test_coder_use_is_an_upper_bound_capped_at_the_task_overlap(self):
        # Task ran 08:50-09:10; the window starts 09:00, so only 600 s of it overlap. The run logged 5000 s.
        self.r.write("state/tasks/t-edge.task", {"id": "t-edge", "project": "broadsword", "status": "done", "created": T0 - 600,
                                                 "started": T0 - 600, "ended": T0 + 600, "note": "merged directly", "title": "edge"})
        self.runs(("t-edge", 1, 5000))
        self.assertEqual(self.record().returncode, 0)
        m = self.metrics()
        self.assertNotIn("coder_use", m)
        self.assertAlmostEqual(m["coder_use_upper_bound"], 600 / 7200)
        self.assertIn("UPPER BOUND", bl.METRICS["coder_use_upper_bound"][3])

    def test_coder_tokens_of_unmerged_finished_tasks_are_kept_in_and_reported_apart(self):
        tid = self.r.merged_task(T0 + 600, T0 + 1800)
        self.r.write("state/tasks/t-gone.task", {"id": "t-gone", "project": "broadsword", "status": "abandoned", "created": T0 + 700,
                                                 "started": T0 + 700, "ended": T0 + 2000, "title": "dropped"})
        self.runs((tid, 1000, 60), ("t-gone", 500, 60))
        self.assertEqual(self.record().returncode, 0)
        m = self.metrics()
        self.assertEqual(m["coder_tokens_per_merged"], 1500)
        self.assertEqual(m["coder_tokens_unmerged"], 500)
        self.assertIn("did not merge", bl.METRICS["coder_tokens_unmerged"][3])
        self.assertIn("done, failed or abandoned", bl.METRICS["coder_tokens_per_merged"][3])

    def test_non_finite_review_cost_is_an_input_error_naming_the_file(self):
        self.r.merged_task(T0 + 600, T0 + 1800)
        rev = next((self.r.root / "state" / "queue").glob("*.review"))
        for bad in ("nan", "inf", "-Infinity"):
            rev.write_text(rev.read_text(encoding="utf-8").replace("cost_usd: 0.10", f"cost_usd: {bad}").replace("cost_usd: nan", f"cost_usd: {bad}"), encoding="utf-8")
            res = self.record(f"nan-{bad.strip('-').lower()}")
            self.assertEqual(res.returncode, 1, f"{bad}: {res.stdout} {res.stderr}")
            self.assertIn(rev.name, res.stderr)
            self.assertIn("finite", res.stderr)
        self.assertFalse(list((self.r.root / "state" / "baselines").glob("*.json")) if (self.r.root / "state" / "baselines").is_dir() else [])

    def test_check_finite_refuses_any_non_finite_metric_or_per_model_figure(self):
        bl.check_finite({"a": 1.0, "b": None}, {"x": {"runs": 1, "usd": 0.1, "tokens": 2, "seconds": 3}})
        for values, pm in (({"a": float("nan")}, {}), ({"a": float("inf")}, {}),
                           ({}, {"x": {"runs": 1, "usd": float("inf"), "tokens": 2, "seconds": 3}})):
            with self.assertRaises(bl.InputError):
                bl.check_finite(values, pm)

    def test_orphan_later_fail_does_not_decide_the_first_pass(self):
        self.r.merged_task(T0 + 600, T0 + 1800)  # one PASS on its first (only) entry
        (self.r.root / "state" / "queue" / "q-x-orphan.review").write_text(
            "\n".join(["entry: q-x-orphan", "attempt: 1", "task: t-x-1", "test_hook: false", "model: opus", "cost_usd: 0.1",
                       "input tokens: 1", "output tokens: 1", "verdict: FAIL (answer followed the format: yes)", ""]),
            encoding="utf-8")
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        m = self.metrics()
        self.assertEqual(m["first_pass_fail_rate"], 0.0)
        self.assertAlmostEqual(m["review_rounds_per_merged"], 2.0)
        self.assertIn("left out of first_pass_fail_rate", res.stdout)

    def test_runs_of_a_finished_task_count_whatever_their_date_even_with_no_start_time(self):
        self.r.write("state/tasks/t-nostart.task", {"id": "t-nostart", "project": "broadsword", "status": "done", "created": T0,
                                                    "ended": T0 + 600, "note": "merged directly", "title": "no start"})
        with open(self.r.root / "state" / "agent-runs.tsv", "a", encoding="utf-8") as f:
            f.write("2026-11-10\tcoder\tsonnet\tt-nostart\t7000\t60\tdone\tx\n")  # dated well before the window
        self.assertEqual(self.record().returncode, 0)
        m = self.metrics()
        self.assertEqual(m["coder_tokens_per_merged"], 7000)
        self.assertEqual(m["coder_runs_recorded"], 1)
        self.assertEqual(m["coder_use_upper_bound"], 0)  # not in the window by date: no use counted

    def test_coder_runs_with_no_task_span_are_counted_and_named_in_a_note(self):
        self.runs(("t-unknown", 10, 99999))
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertAlmostEqual(self.metrics()["coder_use_upper_bound"], 1.0)  # capped at the whole window
        self.assertIn("1 runs have no task span: counted in full, capped at the window", res.stdout)

    def test_missing_main_branch_falls_back_to_head_with_a_note(self):
        git(self.r.root, "checkout", "-q", "-b", "other")
        git(self.r.root, "branch", "-q", "-D", "main")
        res = self.record()
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("no branch `main`", res.stdout)

    def test_a_broken_main_branch_is_an_input_error_not_a_head_fallback(self):
        git(self.r.root, "checkout", "-q", "-b", "other")
        (self.r.root / ".git" / "refs" / "heads" / "main").write_text("not a sha\n", encoding="utf-8")
        res = self.record()
        self.assertEqual(res.returncode, 1, res.stdout + res.stderr)
        self.assertIn("main", res.stderr)
        self.assertNotIn("Traceback", res.stderr)

    def test_record_never_overwrites_even_when_another_record_wins_the_race(self):
        """The other record lands after our existence check, while we measure: ours must fail, theirs must survive."""
        out = self.r.root / "state" / "baselines" / "race.json"
        calls = []

        def fake_measure(root, since, until):
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_text("WINNER", encoding="utf-8")
            calls.append(1)
            return {k: None for k in bl.METRICS}, {"merged_tasks": 0}, {}, "0" * 40, []

        saved = bl.measure, bl.main_root
        bl.measure, bl.main_root = fake_measure, (lambda: self.r.root)
        try:
            args = bl.parser().parse_args(["record", "race", "--since", "2026-11-18T09:00:00Z", "--until", "2026-11-18T11:00:00Z"])
            with self.assertRaises(bl.InputError) as cm:
                bl.cmd_record(args)
        finally:
            bl.measure, bl.main_root = saved
        self.assertEqual(calls, [1])
        self.assertIn("never overwritten", str(cm.exception))
        self.assertEqual(out.read_text(encoding="utf-8"), "WINNER")
        self.assertEqual([p.name for p in out.parent.iterdir()], ["race.json"])

    def test_missing_agent_runs_is_an_error_naming_the_file(self):
        (self.r.root / "state" / "agent-runs.tsv").unlink()
        res = self.record()
        self.assertEqual(res.returncode, 1)
        self.assertIn("agent-runs.tsv", res.stderr)
        self.assertFalse((self.r.root / "state" / "baselines" / "b1.json").exists())

    def test_missing_review_file_is_an_error_naming_the_file(self):
        self.r.merged_task(T0 + 600, T0 + 1800)
        rev = next((self.r.root / "state" / "queue").glob("*.review"))
        rev.unlink()
        res = self.record()
        self.assertEqual(res.returncode, 1)
        self.assertIn(rev.name, res.stderr)

    def test_missing_jobs_folder_is_an_error_naming_it(self):
        (self.r.root / "state" / "jobs").rmdir()
        res = self.record()
        self.assertEqual(res.returncode, 1)
        self.assertIn("jobs", res.stderr)

    def test_time_without_zone_is_refused(self):
        res = self.r.run("record", "b2", "--since", "2026-11-18T09:00:00")
        self.assertEqual(res.returncode, 1)
        self.assertIn("time zone", res.stderr)

    def test_bad_name_is_refused(self):
        res = self.r.run("record", "../x", "--since", "2026-11-18T09:00:00Z")
        self.assertEqual(res.returncode, 1)


class CompareTests(unittest.TestCase):
    def base(self, merged, per_hour, **m):
        values = {k: None for k in bl.METRICS}
        values.update(merged_tasks=merged, merged_per_hour=per_hour, **m)
        return {"metrics": values, "samples": {"merged_tasks": merged}}

    def verdict(self, rows, name):
        return next(r[4] for r in rows if r[0] == name)

    def test_thirty_percent_throughput_drop_is_worse(self):
        rows, overall = bl.compare(self.base(20, 3.0), self.base(20, 2.1), min_samples=5, noise=10)
        self.assertEqual(self.verdict(rows, "merged_per_hour"), "WORSE")
        self.assertEqual(overall, "WORSE")

    def test_below_min_samples_is_noise_never_better(self):
        rows, overall = bl.compare(self.base(3, 1.0), self.base(3, 5.0), min_samples=5, noise=10)
        self.assertEqual(self.verdict(rows, "merged_per_hour"), "NOISE")
        self.assertIn("fewer than 5", next(r[5] for r in rows if r[0] == "merged_per_hour"))
        self.assertEqual(overall, "NOISE")

    def test_within_noise_is_noise_and_a_real_gain_is_better(self):
        rows, overall = bl.compare(self.base(20, 3.0, lead_median_s=1000), self.base(20, 3.2, lead_median_s=600), 5, 10)
        self.assertEqual(self.verdict(rows, "merged_per_hour"), "NOISE")
        self.assertEqual(self.verdict(rows, "lead_median_s"), "BETTER")
        self.assertEqual(overall, "BETTER")

    def test_lower_is_better_metrics(self):
        rows, _ = bl.compare(self.base(20, 3.0, review_usd_per_merged=0.10), self.base(20, 3.0, review_usd_per_merged=0.20), 5, 10)
        self.assertEqual(self.verdict(rows, "review_usd_per_merged"), "WORSE")

    def test_unrecorded_metric_is_noise(self):
        rows, _ = bl.compare(self.base(20, 3.0), self.base(20, 3.0, coder_tokens_per_merged=5), 5, 10)
        self.assertEqual(self.verdict(rows, "coder_tokens_per_merged"), "NOISE")

    def test_metrics_filter_limits_the_overall_verdict(self):
        b, a = self.base(20, 3.0, lead_median_s=100), self.base(20, 2.0, lead_median_s=50)
        self.assertEqual(bl.compare(b, a, 5, 10)[1], "WORSE")
        self.assertEqual(bl.compare(b, a, 5, 10, metrics=["lead_median_s"])[1], "BETTER")

    def test_context_only_metric_is_noise_and_stays_out_of_the_overall_verdict(self):
        rows, overall = bl.compare(self.base(20, 3.0, direct_merges=1), self.base(20, 3.0, direct_merges=9), 5, 10)
        row = next(r for r in rows if r[0] == "direct_merges")
        self.assertEqual(row[4], "NOISE")
        self.assertEqual(row[5], "context only, no direction")
        self.assertEqual(overall, "NOISE")
        self.assertTrue(all(r[4] in ("BETTER", "WORSE", "NOISE") for r in rows))

    def test_below_min_samples_every_metric_is_noise(self):
        rows, _ = bl.compare(self.base(2, 1.0, lead_median_s=9, direct_merges=1), self.base(2, 9.0, lead_median_s=1, direct_merges=5), 5, 10)
        self.assertEqual({r[4] for r in rows}, {"NOISE"})

    def test_coder_tokens_below_min_samples_of_covered_tasks_is_noise(self):
        s = {"merged_with_coder_run": 2}
        b = self.base(20, 3.0, coder_tokens_per_merged=100)
        a = self.base(20, 3.0, coder_tokens_per_merged=10)
        b["samples"], a["samples"] = dict(b["samples"], **s), dict(a["samples"], **s)
        rows, overall = bl.compare(b, a, 5, 10)
        row = next(r for r in rows if r[0] == "coder_tokens_per_merged")
        self.assertEqual(row[4], "NOISE")
        self.assertIn("with a coder run", row[5])
        self.assertEqual(overall, "NOISE")

    def test_a_changed_or_missing_definition_is_noise_and_out_of_the_overall_verdict(self):
        old = self.base(20, 3.0, coder_tokens_per_merged=400000, lead_median_s=1000)
        new = self.base(20, 3.0, coder_tokens_per_merged=100000, lead_median_s=1000)
        new["definitions"] = {"coder_tokens_per_merged": "v1"}
        rows, overall = bl.compare(old, new, 5, 10)
        row = next(r for r in rows if r[0] == "coder_tokens_per_merged")
        self.assertEqual(row[4], "NOISE")
        self.assertEqual(row[5], "definition differs (before v0, after v1)")
        self.assertEqual(overall, "NOISE")
        new["definitions"] = {"coder_tokens_per_merged": "v0"}  # same version on both sides: a real verdict again
        self.assertEqual(self.verdict(bl.compare(old, new, 5, 10)[0], "coder_tokens_per_merged"), "BETTER")

    def test_record_vs_old_record_of_the_same_window_is_noise_for_every_changed_metric(self):
        old = self.base(20, 3.0, **{m: 100 for m in bl.CHANGED_V1})
        new = self.base(20, 3.0, **{m: 10 for m in bl.CHANGED_V1})
        new["definitions"] = dict(bl.DEFINITIONS)
        rows, overall = bl.compare(old, new, 5, 10)
        for m in bl.CHANGED_V1:
            self.assertEqual(self.verdict(rows, m), "NOISE", m)
        self.assertEqual(overall, "NOISE")

    def test_a_change_from_zero_is_noise_not_worse(self):
        rows, overall = bl.compare(self.base(20, 3.0, main_check_failed=0), self.base(20, 3.0, main_check_failed=1), 5, 10)
        row = next(r for r in rows if r[0] == "main_check_failed")
        self.assertEqual((row[4], row[5]), ("NOISE", "before is 0, no relative change"))
        self.assertEqual(overall, "NOISE")

    def test_unknown_metric_is_bad_input(self):
        with self.assertRaises(bl.InputError):
            bl.compare(self.base(20, 3.0), self.base(20, 3.0), 5, 10, metrics=["nope"])


class CompareCliTests(unittest.TestCase):
    def setUp(self):
        self._tmp = scratch_tmp()
        self.r = Root(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def test_cli_prints_verdicts_and_exits_0(self):
        self.r.put_baseline("before", 20, 3.0)
        self.r.put_baseline("after", 20, 2.1)
        res = self.r.run("compare", "before", "after", "--min-samples", "5", "--noise", "10")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertRegex(res.stdout, r"merged_per_hour .* -30\.0%\s+WORSE")
        self.assertIn("overall: WORSE", res.stdout)

    def test_missing_baseline_exits_1_naming_the_file(self):
        self.r.put_baseline("before", 20, 3.0)
        res = self.r.run("compare", "before", "ghost", "--min-samples", "5", "--noise", "10")
        self.assertEqual(res.returncode, 1)
        self.assertIn("ghost.json", res.stderr)

    def test_malformed_baseline_exits_1_naming_the_file_never_a_traceback(self):
        self.r.put_baseline("before", 20, 3.0)
        self.r.put_baseline("after", 20, 2.1)
        path = self.r.root / "state" / "baselines" / "after.json"
        good = json.loads(path.read_text(encoding="utf-8"))
        broken = {"no samples": lambda d: d.pop("samples"), "no window": lambda d: d.pop("window"),
                  "samples without merged_tasks": lambda d: d["samples"].clear(),
                  "text metric": lambda d: d["metrics"].update(merged_per_hour="fast"),
                  "no per_model": lambda d: d.pop("per_model")}
        for label, damage in broken.items():
            d = json.loads(json.dumps(good))
            damage(d)
            path.write_text(json.dumps(d), encoding="utf-8")
            for cmd in (("compare", "before", "after", "--min-samples", "5", "--noise", "10"), ("list",)):
                res = self.r.run(*cmd)
                self.assertEqual(res.returncode, 1, f"{label} {cmd}: {res.stderr}")
                self.assertIn("after.json", res.stderr, label)
                self.assertNotIn("Traceback", res.stderr, label)

    def test_non_finite_metric_is_bad_input_exit_1_naming_the_file(self):
        self.r.put_baseline("before", 20, 3.0)
        self.r.put_baseline("after", 20, 2.1)
        path = self.r.root / "state" / "baselines" / "after.json"
        for token in ("NaN", "Infinity", "-Infinity"):
            path.write_text(path.read_text(encoding="utf-8").replace('"merged_per_hour": 2.1', f'"merged_per_hour": {token}'), encoding="utf-8")
            self.assertIn(token, path.read_text(encoding="utf-8"))
            res = self.r.run("compare", "before", "after", "--min-samples", "5", "--noise", "10")
            self.assertEqual(res.returncode, 1, f"{token}: {res.stdout}")
            self.assertIn("after.json", res.stderr)
            self.assertNotIn("Traceback", res.stderr)
            self.r.put_baseline("after", 20, 2.1)

    def test_baseline_missing_a_metric_is_bad_input_but_a_later_metric_may_be_absent(self):
        self.r.put_baseline("before", 20, 3.0)
        self.r.put_baseline("after", 20, 2.1)
        path = self.r.root / "state" / "baselines" / "after.json"
        good = json.loads(path.read_text(encoding="utf-8"))
        cmd = ("compare", "before", "after", "--min-samples", "5", "--noise", "10")
        d = json.loads(json.dumps(good))
        for m in bl.LATER_METRICS:
            d["metrics"].pop(m)
        path.write_text(json.dumps(d), encoding="utf-8")
        self.assertEqual(self.r.run(*cmd).returncode, 0)
        d["metrics"].pop("lead_median_s")
        path.write_text(json.dumps(d), encoding="utf-8")
        res = self.r.run(*cmd)
        self.assertEqual(res.returncode, 1)
        self.assertIn("after.json", res.stderr)
        self.assertIn("lead_median_s", res.stderr)

    def test_cli_prints_both_sides_notes(self):
        self.r.put_baseline("before", 20, 3.0, notes=["lower bound here"])
        self.r.put_baseline("after", 20, 3.0, notes=["another note"])
        res = self.r.run("compare", "before", "after", "--min-samples", "5", "--noise", "10")
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("before note: lower bound here", res.stdout)
        self.assertIn("after note: another note", res.stdout)

    def test_non_finite_per_model_figure_is_bad_input(self):
        self.r.put_baseline("before", 20, 3.0)
        self.r.put_baseline("after", 20, 3.0)
        path = self.r.root / "state" / "baselines" / "after.json"
        for token in ("NaN", "Infinity", "1e999", '"x"'):
            d = json.loads(path.read_text(encoding="utf-8"))
            d["per_model"] = {"m": {"runs": 1, "usd": 0.5, "tokens": 5, "seconds": 1}}
            text = json.dumps(d).replace('"usd": 0.5', f'"usd": {token}')
            path.write_text(text, encoding="utf-8")
            res = self.r.run("compare", "before", "after", "--min-samples", "5", "--noise", "10")
            self.assertEqual(res.returncode, 1, f"{token}: {res.stdout}")
            self.assertIn("after.json", res.stderr)
            self.assertNotIn("Traceback", res.stderr)

    def test_flags_are_required(self):
        res = self.r.run("compare", "a", "b", "--min-samples", "5")
        self.assertEqual(res.returncode, 2)
        res = self.r.run("compare", "a", "b", "--noise", "10")
        self.assertEqual(res.returncode, 2)


if __name__ == "__main__":
    unittest.main()
