#!/usr/bin/env python3
"""Tests for scripts/session_end_check.py. Run: python -I -B scripts/test_session_end_check.py -v

Each test builds a throw-away Broadsword-shaped root (own Git repo, fake child scripts, fake tasks and jobs) under the main
checkout's scratch folder and removes it afterwards. The script under test runs as a real subprocess.
SESSION_END_SCRIPT overrides the script path (used to prove the tests fail against a deliberately broken copy).
"""
import datetime as dt
import os
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.environ.get("SESSION_END_SCRIPT") or os.path.join(HERE, "session_end_check.py")
GOOD = "import sys\nprint('fake ok', sys.argv[1:])\n"
OLD = "2026-01-01T00:00:00+00:00"
SINCE = "2026-06-01T00:00:00Z"


def main_checkout_scratch():
    here_root = os.path.dirname(HERE)
    out = subprocess.run(["git", "-C", here_root, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                         capture_output=True, text=True, check=True).stdout.strip()
    main_root = os.path.dirname(out)
    with open(os.path.join(main_root, "broadsword.toml"), "rb") as f:
        cfg = tomllib.load(f)
    d = os.path.join(main_root, cfg["scratch_dir"])
    os.makedirs(d, exist_ok=True)
    return d


def git(root, *args, date=None):
    env = dict(os.environ)
    if date:
        env["GIT_COMMITTER_DATE"] = env["GIT_AUTHOR_DATE"] = date
    subprocess.run(["git", "-C", root, "-c", "user.name=t", "-c", "user.email=t@t", *args], check=True,
                   capture_output=True, env=env)


class Case(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="sec-", dir=main_checkout_scratch())
        self.addCleanup(shutil.rmtree, self.root, True)
        for d in ("scripts", "state/tasks", "state/jobs"):
            os.makedirs(os.path.join(self.root, d))
        for n in ("shadow_report", "ledger_check", "main_provenance", "sync_playbook"):
            self.child(n, GOOD + ("# --open\n" if n == "shadow_report" else ""))
        subprocess.run(["git", "init", "-q", "-b", "main", self.root], check=True)
        self.write("README", "x")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "old", date=OLD)
        self.write("PROGRESS.md", "p")
        self.write("LAB_LOG.md", "l")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "records")  # now: after SINCE

    def write(self, rel, text):
        with open(os.path.join(self.root, rel), "w", encoding="utf-8") as f:
            f.write(text)

    def child(self, name, body):
        self.write(f"scripts/{name}.py", body)

    def task(self, tid, status="running", started=None, worktree="", base="abc"):
        started = int(time.time()) if started is None else started
        self.write(f"state/tasks/{tid}.task", f"id = {tid}\nstatus = {status}\nstarted = {started}\ncreated = {started}\n"
                   f"title = T {tid}\nworktree = {worktree}\nbase = {base}\n")

    def run_check(self, since=SINCE, timeout="20", stale="12", extra=()):
        p = subprocess.run([sys.executable, "-I", "-B", SCRIPT, "--root", self.root, "--since", since,
                            "--timeout-seconds", timeout, "--stale-hours", stale, "--max-pending-days", "7", *extra],
                           capture_output=True, text=True, encoding="utf-8")
        return p.returncode, p.stdout, p.stderr

    def line(self, out, name):
        return [l for l in out.splitlines() if l.startswith(name + ":")][0]


class Tests(Case):
    def test_all_ok_exits_zero(self):
        code, out, err = self.run_check()
        self.assertEqual(code, 0, out + err)
        self.assertEqual(out.splitlines()[-1], "session-end: ok")
        for n in ("shadow_report", "ledger_check", "main_provenance", "sync_playbook", "records_updated", "stale_tasks"):
            self.assertTrue(self.line(out, n).endswith(": ok"), out)

    def test_timeout_is_fail(self):
        self.child("ledger_check", "import time\ntime.sleep(30)\n")
        t0 = time.time()
        code, out, _ = self.run_check(timeout="1")
        self.assertLess(time.time() - t0, 15)
        self.assertEqual(code, 1)
        self.assertIn("FAIL  timed out", self.line(out, "ledger_check"))
        self.assertEqual(out.splitlines()[-1], "session-end: 1 failing")

    def test_missing_script_is_skipped_not_ok_and_not_failing(self):
        os.remove(os.path.join(self.root, "scripts", "main_provenance.py"))
        code, out, _ = self.run_check()
        self.assertEqual(code, 0, out)
        self.assertIn("main_provenance: SKIPPED  not installed", self.line(out, "main_provenance"))

    def test_shadow_report_without_open_flag_is_skipped(self):
        self.child("shadow_report", GOOD)
        code, out, _ = self.run_check()
        self.assertEqual(code, 0, out)
        self.assertIn("shadow_report: SKIPPED", self.line(out, "shadow_report"))

    def test_crash_is_fail_with_output(self):
        self.child("sync_playbook", "import sys\nprint('stale copy')\nsys.exit(3)\n")
        code, out, _ = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("FAIL  exit 3", self.line(out, "sync_playbook"))
        self.assertIn("stale copy", out)

    def test_flags_reach_children(self):
        self.child("shadow_report", "import sys\nsys.exit(0 if '--open' in sys.argv else 1)\n# --open\n")
        self.child("sync_playbook", "import sys\nsys.exit(0 if sys.argv[1:] == ['--check'] else 1)\n")
        code, out, _ = self.run_check()
        self.assertEqual(code, 0, out)

    def test_records_not_changed_since_start_fail(self):
        code, out, _ = self.run_check(since="2999-01-01")
        self.assertEqual(code, 1)
        self.assertIn("PROGRESS.md, LAB_LOG.md", self.line(out, "records_updated"))

    def age_records(self):
        """Make the last commit of both records old (before SINCE)."""
        git(self.root, "commit", "-q", "--amend", "--no-edit", date=OLD)

    def set_mtime(self, rel, iso):
        t = dt.datetime.fromisoformat(iso).timestamp()
        os.utime(os.path.join(self.root, rel), (t, t))

    def test_new_uncommitted_record_edit_counts_as_changed(self):
        self.age_records()
        self.write("PROGRESS.md", "edited now")  # dirty, mtime now (after SINCE); LAB_LOG commit is old
        code, out, _ = self.run_check()
        line = self.line(out, "records_updated")
        self.assertIn("LAB_LOG.md", line)
        self.assertNotIn("PROGRESS.md", line)

    def test_old_uncommitted_record_edit_fails(self):
        self.age_records()
        for n in ("PROGRESS.md", "LAB_LOG.md"):
            self.write(n, "left over from long ago")
            self.set_mtime(n, OLD)
        code, out, _ = self.run_check()
        self.assertEqual(code, 1, out)
        self.assertIn("PROGRESS.md, LAB_LOG.md", self.line(out, "records_updated"))

    def test_old_commit_fails_and_new_commit_passes(self):
        self.age_records()
        code, out, _ = self.run_check()
        self.assertIn("PROGRESS.md, LAB_LOG.md", self.line(out, "records_updated"))
        git(self.root, "commit", "-q", "--amend", "--no-edit")  # now: after SINCE
        code, out, _ = self.run_check()
        self.assertTrue(self.line(out, "records_updated").endswith("ok"), out)

    def test_since_offset_is_honoured(self):
        self.age_records()
        self.write("PROGRESS.md", "e")
        self.write("LAB_LOG.md", "e")
        self.set_mtime("PROGRESS.md", "2026-06-01T01:00:00+00:00")
        self.set_mtime("LAB_LOG.md", "2026-06-01T01:00:00+00:00")
        # 2026-06-01T02:00+02:00 is 00:00Z: the 01:00Z edits are after it
        _, out, _ = self.run_check(since="2026-06-01T02:00:00+02:00")
        self.assertTrue(self.line(out, "records_updated").endswith("ok"), out)
        # 2026-06-01T00:00-02:00 is 02:00Z: the 01:00Z edits are before it
        _, out, _ = self.run_check(since="2026-06-01T00:00:00-02:00")
        self.assertIn("PROGRESS.md", self.line(out, "records_updated"))

    def test_hung_git_is_bounded_by_timeout(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location("sec_mod", SCRIPT)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        mod.GIT_TIMEOUT[0] = 1
        real_run = subprocess.run

        def hung_git(cmd, **kw):  # stand-in: the git process never returns; everything else is real
            return real_run([sys.executable, "-I", "-c", "import time; time.sleep(30)"], **kw)

        mod.subprocess = type("S", (), {"run": staticmethod(hung_git), "TimeoutExpired": subprocess.TimeoutExpired})
        t0 = time.time()
        code, _, err = mod.git(self.root, "status")
        self.assertEqual(code, 124)
        self.assertIn("timed out", err)
        self.assertLess(time.time() - t0, 15)

    def test_stale_task_listed_and_fresh_one_not(self):
        self.task("t-old", started=int(time.time()) - 40 * 3600)
        self.task("t-new")
        self.task("t-done", status="merged", started=int(time.time()) - 400 * 3600)
        code, out, _ = self.run_check()
        self.assertEqual(code, 1)
        self.assertIn("FAIL  1 running task(s)", self.line(out, "stale_tasks"))
        self.assertIn("t-old", out)
        self.assertNotIn("t-new", out)
        self.assertNotIn("t-done", out)

    def test_recent_job_keeps_old_task_alive(self):
        old = int(time.time()) - 40 * 3600
        self.task("t-old", started=old)
        self.write("state/jobs/j1.rec", f"task = t-old\nstarted = {int(time.time()) - 3600}\nstatus = done\n")
        code, out, _ = self.run_check()
        self.assertEqual(code, 0, out)

    def test_recent_commit_in_worktree_keeps_old_task_alive(self):
        wt = os.path.join(self.root, "wt")
        subprocess.run(["git", "-C", self.root, "worktree", "add", "-q", "-b", "task/x", wt], check=True)
        base = subprocess.run(["git", "-C", wt, "rev-parse", "HEAD"], capture_output=True, text=True,
                              check=True).stdout.strip()
        self.write("wt/f.txt", "f")
        git(wt, "add", "-A")
        git(wt, "commit", "-q", "-m", "work")
        self.task("t-old", started=int(time.time()) - 40 * 3600, worktree="wt", base=base)
        code, out, _ = self.run_check()
        self.assertEqual(code, 0, out)

    def test_bad_arguments_exit_two(self):
        p = subprocess.run([sys.executable, "-I", "-B", SCRIPT, "--root", self.root, "--since", SINCE],
                           capture_output=True, text=True)
        self.assertEqual(p.returncode, 2)

    def test_read_only(self):
        def snap():
            return sorted((dp, f, os.path.getmtime(os.path.join(dp, f))) for dp, _, fs in os.walk(self.root)
                          if ".git" not in dp for f in fs)
        self.task("t-old", started=int(time.time()) - 40 * 3600)
        before = snap()
        self.run_check()
        self.assertEqual(before, snap())


if __name__ == "__main__":
    unittest.main()
