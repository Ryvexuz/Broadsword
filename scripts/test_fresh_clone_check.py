"""Tests for scripts/fresh_clone_check.py. Each test builds a throwaway git repo and root under the project's scratch
folder and removes it afterwards; the real repo is never cloned and no real build runs.

  python -I -B scripts/test_fresh_clone_check.py -v

BS_FRESH_CLONE_SCRIPT=<path> runs the same tests against another copy of the script (used to prove the tests can fail).
"""
import sys

sys.dont_write_bytecode = True

import importlib.util
import io
import os
import pathlib
import shutil
import stat
import subprocess
import tempfile
import unittest
from contextlib import redirect_stdout

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = pathlib.Path(os.environ.get("BS_FRESH_CLONE_SCRIPT") or HERE / "fresh_clone_check.py")
SCRATCH = HERE.parent / "scratch"
TICK = chr(96)


def load(path):
    spec = importlib.util.spec_from_file_location("fresh_clone_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


fc = load(SCRIPT)


def rmtree(path):
    def onerror(func, p, _exc):
        os.chmod(p, stat.S_IWRITE)
        func(p)

    shutil.rmtree(path, onerror=onerror)


def git(cwd, *args):
    subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *args], cwd=cwd, check=True,
                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT)


def doc(cmd, name="README.md"):
    return name, f"# fake\n\nOne command: {TICK}{cmd}{TICK}\n"


class Base(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(exist_ok=True)
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="test-fresh-clone-", dir=SCRATCH))
        self.addCleanup(rmtree, self.tmp)
        self.root = self.tmp / "root"
        (self.root / "scratch").mkdir(parents=True)
        self.repo = self.tmp / "repo"

    def make_repo(self, files, branch="main"):
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", branch)
        for name, text in files.items():
            (self.repo / name).write_text(text, encoding="utf-8")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "init")
        return subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.repo, stdout=subprocess.PIPE).stdout.decode().strip()

    def go(self, env=None, *extra):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = fc.main(["--repo", str(self.repo), "--root", str(self.root), *extra], env={} if env is None else env)
        return code, buf.getvalue()

    def leftovers(self):
        return [p.name for p in (self.root / "scratch").iterdir() if p.is_dir()]


class Run(Base):
    def test_pass_prints_commit_and_command_source_and_deletes_the_clone(self):
        n, t = doc("python -I check.py good")
        sha = self.make_repo({n: t, "check.py": "import sys; print('built', sys.argv[1])\n"})
        code, out = self.go()
        self.assertEqual(code, 0, out)
        self.assertIn(f"commit tested: {sha}", out)
        self.assertIn("one command (from README.md): python -I check.py good", out)
        self.assertIn("built good", out)
        self.assertIn(f"PASS: commit {sha[:12]}", out)
        self.assertEqual(self.leftovers(), [])
        saved = list((self.root / "scratch").glob("fresh-clone-*.txt"))
        self.assertEqual(len(saved), 1)
        self.assertIn("built good", saved[0].read_text(encoding="utf-8"))

    def test_failing_command_is_a_fail_and_still_cleans_up(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "import sys; print('boom'); sys.exit(3)\n"})
        code, out = self.go()
        self.assertEqual(code, 1, out)
        self.assertIn("exit: 3", out)
        self.assertIn("FAIL:", out)
        self.assertEqual(self.leftovers(), [])

    def test_agents_md_is_used_when_readme_has_no_command_and_is_named(self):
        n, t = doc("python -I check.py", "AGENTS.md")
        self.make_repo({"README.md": "# nothing here\n", n: t, "check.py": "print('ok')\n"})
        code, out = self.go()
        self.assertEqual(code, 0, out)
        self.assertIn("(from AGENTS.md)", out)

    def test_no_documented_command_fails_with_exit_2(self):
        self.make_repo({"README.md": "# no command written down\n", "check.py": "print('ok')\n"})
        code, out = self.go()
        self.assertEqual(code, 2)
        self.assertIn("no documented one command", out)
        self.assertEqual(self.leftovers(), [])

    def test_the_clone_gets_its_own_target_folder_and_only_committed_files(self):
        n, t = doc("python -I check.py")
        check = "import os, pathlib\nprint('TARGET=' + os.environ['CARGO_TARGET_DIR'])\nprint('CWD=' + os.getcwd())\n" \
                "print('UNTRACKED=' + str(pathlib.Path('untracked.txt').exists()))\n"
        self.make_repo({n: t, "check.py": check})
        (self.repo / "untracked.txt").write_text("x", encoding="utf-8")
        code, out = self.go()
        self.assertEqual(code, 0, out)
        target = [l for l in out.splitlines() if l.startswith("TARGET=")][0][7:]
        cwd = [l for l in out.splitlines() if l.startswith("CWD=")][0][4:]
        self.assertEqual(pathlib.Path(target).resolve(), (pathlib.Path(cwd) / "target").resolve())
        self.assertEqual(pathlib.Path(cwd).resolve().parent, (self.root / "scratch").resolve())
        self.assertIn("UNTRACKED=False", out)

    def test_clone_of_a_repo_without_main_is_refused(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "print('ok')\n"}, branch="trunk")
        code, out = self.go()
        self.assertEqual(code, 2)
        self.assertIn("git clone failed", out)
        self.assertEqual(self.leftovers(), [])

    def test_timeout_is_a_fail(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "import time; time.sleep(30)\n"})
        code, out = self.go(None, "--timeout", "1")
        self.assertEqual(code, 1, out)
        self.assertIn("timed out", out)
        self.assertEqual(self.leftovers(), [])


class Refusals(Base):
    def test_refused_inside_a_job_and_nothing_is_cloned(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "print('ok')\n"})
        for env in ({"BROADSWORD_JOB": "20261120-1"}, {"CARGO_TARGET_DIR": "build"}):
            code, out = self.go(env)
            self.assertEqual(code, 2)
            self.assertIn("inside a Broadsword job", out)
        self.assertEqual(list((self.root / "scratch").iterdir()), [])

    def test_safe_delete_only_removes_fresh_clone_folders_directly_in_scratch(self):
        scratch = self.root / "scratch"
        ok = scratch / "fresh-clone-1"
        ok.mkdir()
        (ok / "f").write_text("x")
        outside = self.tmp / "fresh-clone-outside"
        outside.mkdir()
        wrong_name = scratch / "keep-me"
        wrong_name.mkdir()
        nested = scratch / "keep-me" / "fresh-clone-nested"
        nested.mkdir()
        for bad in (outside, wrong_name, nested, scratch, self.root, scratch / "fresh-clone-missing"):
            with self.subTest(bad=str(bad)):
                with self.assertRaises(fc.Refused):
                    fc.safe_delete(bad, scratch)
                self.assertTrue(bad.exists() or bad.name == "fresh-clone-missing")
        fc.safe_delete(ok, scratch)
        self.assertFalse(ok.exists())

    def test_dotdot_path_that_resolves_outside_scratch_is_refused(self):
        scratch = self.root / "scratch"
        outside = self.root / "fresh-clone-sneaky"
        outside.mkdir()
        with self.assertRaises(fc.Refused):
            fc.safe_delete(scratch / ".." / "fresh-clone-sneaky", scratch)
        self.assertTrue(outside.exists())


if __name__ == "__main__":
    unittest.main()
