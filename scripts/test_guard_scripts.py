"""Tests for scripts/run-test-a.ps1 and scripts/ff-reviewed.ps1 (the two guard scripts temporary allow rules point at).

Every case runs a COPY of the scripts inside a throwaway temp root (the scripts take their root from their own
location), so no real repo, queue or calibration is touched. run-test-a is always run with RUN_TEST_A_DRY=1, which only
prints: a refusal case must still exit 2 and an allowed case must exit 0 and print the command.

    python -I -B scripts/test_guard_scripts.py              run the suite
    python -I -B scripts/test_guard_scripts.py --prove-fail  run it against deliberately weakened copies; each
                                                             weakening must make its named tests FAIL
"""
import msvcrt
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid

SCRIPTS = os.path.dirname(os.path.abspath(__file__))
# Set only by --prove-fail: {script name: [(old, new), ...]} applied to the temp copies.
WEAKEN = {}

GIT_ENV = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}
FROZEN_START = "run crash-sim"


def ps(script, args, cwd, dry=False):
    env = dict(os.environ)
    env.pop("RUN_TEST_A_DRY", None)
    env.update(GIT_ENV)
    if dry:
        env["RUN_TEST_A_DRY"] = "1"
    return subprocess.run(["powershell", "-NoProfile", "-File", script] + list(args), cwd=cwd, env=env,
                          capture_output=True, text=True, timeout=120)


def wr(path, text):
    with open(path, "w") as f:
        f.write(text)


def git(repo, *args):
    env = dict(os.environ)
    env.update(GIT_ENV)
    r = subprocess.run(["git", "-C", repo] + list(args), env=env, capture_output=True, text=True)
    assert r.returncode == 0, (args, r.stderr)
    return r.stdout.strip()


def make_root():
    # hex name: no "_b" or "test-b" anywhere in the root path (run-test-a scans resolved paths).
    root = os.path.join(tempfile.gettempdir(), "gt" + uuid.uuid4().hex[:10])
    os.makedirs(os.path.join(root, "scripts"))
    for name in ("run-test-a.ps1", "ff-reviewed.ps1"):
        text = open(os.path.join(SCRIPTS, name), encoding="utf-8").read()
        for old, new in WEAKEN.get(name, []):
            assert old in text, f"weakening target missing in {name}: {old}"
            text = text.replace(old, new)
        with open(os.path.join(root, "scripts", name), "w", encoding="utf-8") as f:
            f.write(text)
    return root


class TestA(unittest.TestCase):
    def setUp(self):
        self.root = make_root()
        self.addCleanup(shutil.rmtree, self.root, True)
        for d in ("projects/crash-sim/scenarios", "worktrees/crash-sim/t-20261122-000001/scenarios", "scratch", "elsewhere"):
            os.makedirs(os.path.join(self.root, d))
        for f in ("projects/crash-sim/scenarios/validation_wall.bsn", "projects/crash-sim/scenarios/validation_wall_b.bsn",
                  "projects/crash-sim/scenarios/other.bsn", "projects/crash-sim/scenarios/notes.txt",
                  "worktrees/crash-sim/t-20261122-000001/scenarios/mine.bsn", "scratch/x.bsn", "elsewhere/y.bsn",
                  "scratch/meta.bsn", "scratch/test-b.bsn", "scratch/my_test_b.bsn", "scratch/CAR_B.bsn",
                  "scratch/reference.bsn"):
            wr(os.path.join(self.root, f), "x")
        self.script = os.path.join(self.root, "scripts", "run-test-a.ps1")

    def run_a(self, *args):
        return ps(self.script, args, self.root, dry=True)

    def assertRefused(self, *args):
        r = self.run_a(*args)
        self.assertEqual(r.returncode, 2, (args, r.stdout, r.stderr))
        self.assertIn("REFUSED", r.stderr)
        self.assertNotIn("dry run", r.stdout)

    def assertAllowed(self, *args):
        r = self.run_a(*args)
        self.assertEqual(r.returncode, 0, (args, r.stdout, r.stderr))
        return r.stdout

    def test_allowed_default_prints_frozen_command(self):
        out = self.assertAllowed()
        self.assertIn("run crash-sim -- cargo run --offline --release -- --body scenarios/validation_wall.bsn "
                      "--pulse-report validation/test-a.pulse.csv --gravity 0 --duration 0.2 --dt 0.001 --substeps 10 "
                      "--pulse-sensor sensor", out)
        self.assertIn("--margin-static-crush 0.20", out)
        self.assertIn("dry run, nothing executed", out)

    def test_allowed_task_body_substeps(self):
        out = self.assertAllowed("--task", "t-20261122-043813", "--body", "scratch/x.bsn", "--substeps", "20")
        self.assertIn("run crash-sim --task t-20261122-043813 -- cargo", out)
        self.assertIn("--substeps 20 --pulse-sensor", out)
        self.assertNotIn("--substeps 10", out)
        self.assertIn("scratch" + os.sep + "x.bsn", out)

    def test_allowed_bodies_in_each_allowed_folder(self):
        self.assertAllowed("--body", "projects/crash-sim/scenarios/other.bsn")
        self.assertAllowed("--body", "worktrees/crash-sim/t-20261122-000001/scenarios/mine.bsn")

    def test_refuses_test_b_markers_in_any_argument(self):
        self.assertRefused("--body", "projects/crash-sim/scenarios/validation_wall_b.bsn")
        self.assertRefused("--body", "scratch/test-b.bsn")
        self.assertRefused("--body", "scratch/my_test_b.bsn")
        self.assertRefused("--body", "scratch/CAR_B.bsn")
        self.assertRefused("--task", "t-20261122-test-b")
        self.assertRefused("--body", "scratch/x.bsn", "--substeps", "10_b")

    def test_refuses_reference_and_metadata_files(self):
        self.assertRefused("--body", "scratch/meta.bsn")
        self.assertRefused("--body", "scratch/reference.bsn")

    def test_refuses_body_outside_allowed_folders(self):
        self.assertRefused("--body", "elsewhere/y.bsn")
        self.assertRefused("--body", "scripts/run-test-a.ps1")
        self.assertRefused("--body", "scratch/../elsewhere/y.bsn")
        self.assertRefused("--body", os.path.join(self.root, "elsewhere", "y.bsn"))
        self.assertRefused("--body", "C:/Windows/win.ini")

    def test_refuses_bad_body_file(self):
        self.assertRefused("--body", "projects/crash-sim/scenarios/notes.txt")
        self.assertRefused("--body", "scratch/missing.bsn")

    def test_refuses_free_form_and_malformed_arguments(self):
        self.assertRefused("--gravity", "0")
        self.assertRefused("--pulse-report", "validation/other.csv")
        self.assertRefused("extra")
        self.assertRefused("--task")
        self.assertRefused("--task", "x")
        self.assertRefused("--task", "t-20261122-043813", "--task", "t-20261122-043814")
        self.assertRefused("--substeps", "0")
        self.assertRefused("--substeps", "-3")
        self.assertRefused("--substeps", "1.5")
        self.assertRefused("--substeps", "10 --margin-dv 9")
        self.assertRefused("--SUBSTEPS", "10")
        self.assertRefused("--body", "")

    def test_refuses_short_name_paths(self):
        # 8.3 short names (VALIDA~1.BSN) could hide a test B file name: any "~" in --body is refused.
        wr(os.path.join(self.root, "scratch", "x~1.bsn"), "x")  # exists, so only the "~" rule can refuse it
        self.assertRefused("--body", "scratch/x~1.bsn")
        self.assertRefused("--body", "scratch/VALIDA~1.BSN")
        self.assertRefused("--body", "SCRATCH~1/x.bsn")
        # A real short name of a test B file, when this machine can create one without admin.
        longname = os.path.join(self.root, "scratch", "validation_wall_b.bsn")
        wr(longname, "x")
        r = subprocess.run(["fsutil", "file", "setshortname", longname, "VALIDA~1.BSN"], capture_output=True)
        if r.returncode == 0:
            self.assertRefused("--body", "scratch/VALIDA~1.BSN")

    def test_refuses_link_to_elsewhere(self):
        link = os.path.join(self.root, "scratch", "lnk")
        r = subprocess.run(["cmd", "/c", "mklink", "/J", link, os.path.join(self.root, "elsewhere")], capture_output=True)
        if r.returncode != 0:
            self.skipTest("cannot create a junction")
        self.assertRefused("--body", "scratch/lnk/y.bsn")


class TestFF(unittest.TestCase):
    def setUp(self):
        self.root = make_root()
        self.addCleanup(shutil.rmtree, self.root, True)
        self.script = os.path.join(self.root, "scripts", "ff-reviewed.ps1")
        self.repo = os.path.join(self.root, "projects", "demo")
        os.makedirs(self.repo)
        git(self.repo, "init", "-q", "-b", "main")
        wr(os.path.join(self.repo, "a.txt"), "1")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "c1")
        self.c1 = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "checkout", "-q", "-b", "task")
        wr(os.path.join(self.repo, "a.txt"), "2")
        git(self.repo, "commit", "-q", "-am", "c2")
        self.c2 = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "checkout", "-q", "main")
        git(self.repo, "checkout", "-q", "-b", "side")
        wr(os.path.join(self.repo, "b.txt"), "3")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "c3 diverges")
        self.c3 = git(self.repo, "rev-parse", "HEAD")
        git(self.repo, "checkout", "-q", "main")
        self.q = os.path.join(self.root, "state", "queue")
        os.makedirs(self.q)
        for n in ("queue.lock", "demo.queue.lock"):
            wr(os.path.join(self.q, n), "")
        self.entry("q-1", self.c2, "approved")
        self.entry("q-3", self.c3, "approved")

    def entry(self, eid, tip, status, project="demo", log=""):
        with open(os.path.join(self.q, eid + ".entry"), "w") as f:
            f.write(f"id = {eid}\nproject = {project}\nreviewed_tip = {tip}\nstatus = {status}\nlog = {log}\n")

    def run_ff(self, project, commit):
        return ps(self.script, [project, commit], self.root)

    def head(self):
        return git(self.repo, "rev-parse", "HEAD")

    def assertRefused(self, project, commit):
        before = self.head()
        r = self.run_ff(project, commit)
        self.assertEqual(r.returncode, 2, (r.stdout, r.stderr))
        self.assertIn("REFUSED", r.stderr)
        self.assertEqual(self.head(), before, "main moved on a refusal")

    def test_fast_forwards_an_approved_commit(self):
        r = self.run_ff("demo", self.c2)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        self.assertIn("relying on queue entry q-1", r.stdout)
        self.assertEqual(self.head(), self.c2)
        # the locks are free again afterwards
        for n in ("queue.lock", "demo.queue.lock"):
            fd = os.open(os.path.join(self.q, n), os.O_RDWR)
            msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
            os.close(fd)

    def test_accepts_merged_and_log_approved_entries(self):
        os.remove(os.path.join(self.q, "q-1.entry"))
        self.entry("q-2", self.c2, "merged_check_failed")
        self.assertEqual(self.run_ff("demo", self.c2).returncode, 0)
        git(self.repo, "reset", "-q", "--hard", self.c1)
        os.remove(os.path.join(self.q, "q-2.entry"))
        self.entry("q-4", self.c2, "check_failed", log=r"2026-11-22 01:00:00 UTC review: approved: joins the train")
        self.assertEqual(self.run_ff("demo", self.c2).returncode, 0)

    def test_refuses_unapproved_commit(self):
        os.remove(os.path.join(self.q, "q-1.entry"))
        self.assertRefused("demo", self.c2)

    def test_refuses_rejected_or_wrong_project_entries(self):
        os.remove(os.path.join(self.q, "q-1.entry"))
        self.entry("q-5", self.c2, "rejected", log="UTC review: approved earlier")
        self.entry("q-6", self.c2, "approved", project="other")
        self.entry("q-7", self.c2, "queued")
        self.assertRefused("demo", self.c2)

    def test_refuses_dirty_main(self):
        wr(os.path.join(self.repo, "a.txt"), "dirty")
        self.assertRefused("demo", self.c2)

    def test_refuses_untracked_file_in_main(self):
        wr(os.path.join(self.repo, "stray.txt"), "x")
        self.assertRefused("demo", self.c2)

    def test_refuses_held_project_lock(self):
        fd = os.open(os.path.join(self.q, "demo.queue.lock"), os.O_RDWR)
        try:
            msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
            self.assertRefused("demo", self.c2)
        finally:
            os.close(fd)

    def test_refuses_held_global_lock(self):
        fd = os.open(os.path.join(self.q, "queue.lock"), os.O_RDWR)
        try:
            msvcrt.locking(fd, msvcrt.LK_NBLCK, 1)
            self.assertRefused("demo", self.c2)
        finally:
            os.close(fd)

    def test_refuses_when_locks_cannot_be_checked(self):
        os.remove(os.path.join(self.q, "queue.lock"))
        os.remove(os.path.join(self.q, "demo.queue.lock"))
        self.assertRefused("demo", self.c2)

    def test_refuses_non_fast_forward(self):
        self.assertRefused("demo", self.c3 if False else self._diverged())

    def _diverged(self):
        # main moves on; c2 (approved) no longer descends from main's HEAD
        wr(os.path.join(self.repo, "c.txt"), "m")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "main moves")
        return self.c2

    def test_refuses_bad_arguments(self):
        self.assertRefused("demo", "main")
        self.assertRefused("demo", self.c2[:12])
        self.assertRefused("demo", self.c2.upper())
        self.assertRefused("..", self.c2)
        self.assertRefused("nosuch", self.c2)
        self.assertEqual(ps(self.script, ["demo"], self.root).returncode, 2)
        self.assertEqual(ps(self.script, ["demo", self.c2, "--force"], self.root).returncode, 2)

    def test_refuses_when_main_is_not_on_branch_main(self):
        git(self.repo, "checkout", "-q", "side")
        self.assertRefused("demo", self.c2)


# --prove-fail: each weakening must make the named tests fail (the suite is the thing being proven).
WEAKENINGS = [
    ("run-test-a.ps1 without the test B markers", {"run-test-a.ps1": [("$forbidden = @('_b', 'test-b', 'test_b', 'car_b', 'reference', 'meta')", "$forbidden = @('zzzz')")]},
     ["test_refuses_test_b_markers_in_any_argument", "test_refuses_reference_and_metadata_files"]),
    ("run-test-a.ps1 without the allowed-folder rule", {"run-test-a.ps1": [("if (-not $okDir) {", "if ($false) {")]},
     ["test_refuses_body_outside_allowed_folders"]),
    ("ff-reviewed.ps1 without the lock check", {"ff-reviewed.ps1": [("$fs.Lock(0, [long]::MaxValue)", "")]},
     ["test_refuses_held_project_lock", "test_refuses_held_global_lock"]),
    ("run-test-a.ps1 without the short-name check", {"run-test-a.ps1": [("-match '~'", "-match '\x01'")]},
     ["test_refuses_short_name_paths"]),
    ("ff-reviewed.ps1 without the clean-main check", {"ff-reviewed.ps1": [("if ($dirty.Count -gt 0) {", "if ($false) {")]},
     ["test_refuses_dirty_main", "test_refuses_untracked_file_in_main"]),
]


def prove_fail():
    bad = 0
    for label, weaken, must_fail in WEAKENINGS:
        WEAKEN.clear()
        WEAKEN.update(weaken)
        suite = unittest.TestSuite()
        for cls in (TestA, TestFF):
            for name in must_fail:
                if hasattr(cls, name):
                    suite.addTest(cls(name))
        res = unittest.TextTestRunner(stream=open(os.devnull, "w"), verbosity=0).run(suite)
        failed = {t.id().split(".")[-1] for t, _ in res.failures + res.errors}
        ok = set(must_fail) <= failed
        print(f"prove-fail {'OK ' if ok else 'BAD'} {label}: failed {sorted(failed)}")
        bad += 0 if ok else 1
    return bad


if __name__ == "__main__":
    if "--prove-fail" in sys.argv:
        sys.exit(1 if prove_fail() else 0)
    unittest.main(verbosity=2)
