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
import time
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


def pid_alive(pid):
    if os.name == "nt":
        out = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"], stdout=subprocess.PIPE).stdout.decode(errors="replace")
        return str(pid) in out
    try:
        os.kill(pid, 0)
        return True
    except OSError:
        return False


def kill_pid(pid):
    if os.name == "nt":
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(pid)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    elif pid_alive(pid):
        os.kill(pid, 9)


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
        (self.root / "broadsword.toml").write_text('root = "."\nbuild_dir = "build"\n', encoding="utf-8")
        self.repo = self.tmp / "repo"

    def make_repo(self, files, branch="main"):
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", branch)
        for name, text in files.items():
            (self.repo / name).parent.mkdir(parents=True, exist_ok=True)
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


    def test_relative_root_still_gives_the_clone_an_absolute_target_folder(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "import os\nprint('TARGET=' + os.environ['CARGO_TARGET_DIR'])\n"})
        old = os.getcwd()
        os.chdir(self.tmp)
        try:
            buf = io.StringIO()
            with redirect_stdout(buf):
                code = fc.main(["--repo", "repo", "--root", "root"], env={})
        finally:
            os.chdir(old)
        target = [l for l in buf.getvalue().splitlines() if l.startswith("TARGET=")][0][7:]
        self.assertEqual(code, 0, buf.getvalue())
        self.assertTrue(os.path.isabs(target), target)
        self.assertEqual(pathlib.Path(target).parent.parent.resolve(), (self.root / "scratch").resolve())

    def test_backslash_paths_in_the_documented_command_survive_on_windows(self):
        if os.name != "nt":
            self.skipTest("backslash paths are a Windows form")
        n, t = doc("python -I sub\\check.py")
        self.make_repo({n: t, "sub/check.py": "print('ran sub check')\n"})
        code, out = self.go()
        self.assertEqual(code, 0, out)
        self.assertIn("ran sub check", out)

    def test_forward_slash_relative_program_in_the_documented_command_runs_on_windows(self):
        if os.name != "nt":
            self.skipTest("CreateProcess path rule is Windows only")
        n, t = doc("sub/run.bat arg1")
        self.make_repo({n: t, "sub/run.bat": "@echo ran-bat %1\n"})
        code, out = self.go()
        self.assertEqual(code, 0, out)
        self.assertIn("ran-bat arg1", out)

    def test_timeout_kills_a_grandchild_that_would_hang_the_script_and_keeps_the_output(self):
        pidfile = self.tmp / "grandchild.pid"
        n, t = doc("python -I check.py")
        check = (
            "import subprocess, sys, time\n"
            "print('before hang', flush=True)\n"
            f"g = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(120)'])\n"
            f"open({str(pidfile)!r}, 'w').write(str(g.pid))\n"
            "time.sleep(120)\n"
        )
        self.make_repo({n: t, "check.py": check})
        start = time.monotonic()
        code, out = self.go(None, "--timeout", "3")
        took = time.monotonic() - start
        self.addCleanup(lambda: kill_pid(int(pidfile.read_text())) if pidfile.exists() else None)
        self.assertEqual(code, 1, out)
        self.assertLess(took, 60, f"took {took:.0f} s: the script waited for the grandchild")
        self.assertIn("FAIL:", out)
        self.assertIn("timed out", out)
        self.assertIn("before hang", out)
        self.assertEqual(self.leftovers(), [])
        time.sleep(1)
        self.assertFalse(pid_alive(int(pidfile.read_text())), "the grandchild survived the timeout")

    def test_a_clone_that_cannot_be_deleted_is_reported_without_a_traceback(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "print('ok')\n"})
        real = fc.safe_delete
        fc.safe_delete = lambda *a, **k: (_ for _ in ()).throw(PermissionError("locked file"))
        try:
            code, out = self.go()
        finally:
            fc.safe_delete = real
        self.assertEqual(code, 0, out)
        self.assertIn("PASS:", out)
        self.assertIn("clone NOT deleted: locked file", out)


MOVING_TEXT = ("# Moving\n\n4. Rebuild:\n   - `cargo build --offline`;\n   - copy `build/debug/broadsword.exe` to `scratch/bs-boot.exe`;\n"
               "   - then `scratch/bs-boot.exe build broadsword`;\n   - then `build/debug/broadsword.exe install`.\n")
REAL_STEPS = getattr(fc, "bootstrap_steps", None)
TOML = 'root = "."\nbuild_dir = "build"\ninstall_dir = "build/bin"\nprojects_dir = "projects"\n'


class Bootstrap(Base):
    """The clone has no build/bin/broadsword.exe (it is built, not committed): the check builds it per docs/MOVING.md first."""

    def setUp(self):
        super().setUp()
        self.crash = self.tmp / "crash"
        self.crash.mkdir()
        git(self.crash, "init", "-q", "-b", "main")
        (self.crash / "marker.txt").write_text("crash-sim", encoding="utf-8")
        git(self.crash, "add", "-A")
        git(self.crash, "commit", "-q", "-m", "c")
        self.log = self.tmp / "steps.log"

    def repo_with_command(self, moving=MOVING_TEXT):
        cmd = "build/bin/broadsword.exe check crash-sim"
        files = {"README.md": f"One command: {TICK}{cmd}{TICK}\n", "broadsword.toml": TOML, "docs/MOVING.md": moving}
        self.make_repo(files)

    def fake_steps(self, fail_at=None):
        """Replace the real cargo/broadsword steps by python one-liners that record their order and create the same files."""
        log = str(self.log)

        def steps(clone, cfg):
            real = REAL_STEPS(clone, cfg)  # the real list is still built (and checked against MOVING.md)
            self.assertEqual([s[0].split("  ")[0] for s in real][0], "cargo build --offline")
            out = []
            for i, (label, step) in enumerate(real):
                if step[0] == "copy":
                    out.append((label, ("run", [sys.executable, "-I", "-c", f"open({log!r},'a').write('copy\\n')"], {})))
                    continue
                code = 7 if fail_at == i else 0
                make = ""
                if i == 2:
                    make = f"import pathlib; p=pathlib.Path({str(clone / 'build' / 'bin')!r}); p.mkdir(parents=True, exist_ok=True); (p/'broadsword.exe').write_text('x')"
                src = f"import sys; {make}; open({log!r},'a').write('step{i}\\n'); sys.exit({code})" if make else f"import sys; open({log!r},'a').write('step{i}\\n'); sys.exit({code})"
                out.append((label, ("run", [sys.executable, "-I", "-c", src], {})))
            return out

        return steps

    def test_bootstrap_runs_the_moving_steps_in_order_provisions_crash_sim_then_runs_the_command(self):
        self.repo_with_command()
        # the one command is the broadsword.exe the fake step 2 made: replace it by a python script via a stub exe is not
        # possible, so the fake step also provisions nothing; the final command is checked through its failure message below
        real = fc.bootstrap_steps
        fc.bootstrap_steps = self.fake_steps()
        try:
            code, out = self.go({}, "--crash-sim", str(self.crash))
        finally:
            fc.bootstrap_steps = real
        self.assertIn("bootstrap: build/bin/broadsword.exe is not committed", out)
        self.assertEqual(self.log.read_text().split(), ["step0", "copy", "step2", "step3"], out)
        self.assertIn("== bootstrap step: provision projects/crash-sim", out)
        self.assertIn("== bootstrap step: cargo build --offline  (the one plain-cargo bootstrap exception in docs/MOVING.md)", out)
        # step 2 made a file named broadsword.exe that is not a program: the one command ran (and failed), after the bootstrap
        self.assertIn("one command (from README.md): build/bin/broadsword.exe check crash-sim", out)
        self.assertEqual(code, 1, out)
        self.assertEqual(self.leftovers(), [])

    def test_a_failing_bootstrap_step_is_a_fail_naming_the_step_and_the_command_is_not_run(self):
        self.repo_with_command()
        real = fc.bootstrap_steps
        fc.bootstrap_steps = self.fake_steps(fail_at=0)
        try:
            code, out = self.go({}, "--crash-sim", str(self.crash))
        finally:
            fc.bootstrap_steps = real
        self.assertEqual(code, 1, out)
        self.assertIn("FAIL: bootstrap step failed: cargo build --offline", out)
        self.assertIn("the one command was not run", out)
        self.assertEqual(self.log.read_text().split(), ["step0"])
        self.assertEqual(self.leftovers(), [])

    def test_the_real_step_list_is_exactly_the_moving_md_commands(self):
        self.repo_with_command()
        clone = self.tmp / "clone"
        subprocess.run(["git", "clone", "-q", str(self.repo), str(clone)], check=True)
        steps = fc.bootstrap_steps(clone, fc.clone_config(clone))
        self.assertEqual(steps[0][1][1], ["cargo", "build", "--offline"])
        self.assertEqual(steps[0][1][2], {"CARGO_TARGET_DIR": str(clone / "build")})
        self.assertEqual(steps[1][1][0], "copy")
        self.assertEqual(pathlib.Path(steps[2][1][1][0]), clone / "scratch" / "bs-boot.exe")
        self.assertEqual(steps[2][1][1][1:], ["build", "broadsword"])
        self.assertEqual(pathlib.Path(steps[3][1][1][0]), clone / "build" / "debug" / "broadsword.exe")
        self.assertEqual(steps[3][1][1][1:], ["install"])

    def test_refused_when_moving_md_does_not_give_the_command(self):
        self.repo_with_command(moving="# Moving\nrun `cargo build` and hope\n")
        code, out = self.go({}, "--crash-sim", str(self.crash))
        self.assertEqual(code, 2, out)
        self.assertIn("docs/MOVING.md does not contain `cargo build --offline`", out)
        self.assertEqual(self.leftovers(), [])

    def test_a_crash_sim_url_or_missing_folder_is_refused_not_cloned(self):
        self.repo_with_command()
        real = fc.bootstrap_steps
        fc.bootstrap_steps = self.fake_steps()
        try:
            for bad in ("https://example.invalid/crash-sim.git", str(self.tmp / "nope")):
                with self.subTest(bad=bad):
                    code, out = self.go({}, "--crash-sim", bad)
                    self.assertEqual(code, 2, out)
                    self.assertIn("is not a local Git repo folder", out)
                    self.assertEqual(self.leftovers(), [])
        finally:
            fc.bootstrap_steps = real

    def test_no_bootstrap_when_the_command_is_not_the_installed_binary(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "print('ok')\n", "broadsword.toml": TOML})
        code, out = self.go({})
        self.assertEqual(code, 0, out)
        self.assertNotIn("bootstrap", out)

    def test_provision_clones_the_local_crash_sim_repo(self):
        clone = self.tmp / "c"
        clone.mkdir()
        rc, out = fc.provision_crash_sim(clone, {"projects_dir": "projects"}, self.crash)
        self.assertEqual(rc, 0, out)
        self.assertEqual((clone / "projects" / "crash-sim" / "marker.txt").read_text(), "crash-sim")


class Refusals(Base):
    def test_refused_when_cargo_target_dir_is_the_shared_build_folder_a_job_sets(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "print('ok')\n"})
        build = self.root / "build"
        # job.rs sets CARGO_TARGET_DIR to root.join(build_dir); spelled with .. and mixed case it is still that folder
        for target in (str(build), str(self.root / "x" / ".." / "build"), str(build).upper()):
            code, out = self.go({"CARGO_TARGET_DIR": target})
            self.assertEqual(code, 2, target)
            self.assertIn("shared build folder", out)
            self.assertIn("CARGO_TARGET_DIR", out)
        self.assertEqual(list((self.root / "scratch").iterdir()), [])

    def test_refused_in_a_worktree_whose_job_got_the_main_checkouts_shared_build_folder(self):
        # job.rs gives a job in a worktree the MAIN root's build folder, not the worktree's own
        n, t = doc("python -I check.py")
        self.repo.mkdir()
        git(self.repo, "init", "-q", "-b", "main")
        files = {n: t, "check.py": "print(1)\n", "broadsword.toml": "root = '.'\nbuild_dir = 'build'\n"}
        for name, text in files.items():
            (self.repo / name).write_text(text, encoding="utf-8")
        git(self.repo, "add", "-A")
        git(self.repo, "commit", "-q", "-m", "init")
        wt = self.tmp / "wt"
        git(self.repo, "worktree", "add", "-q", "-b", "task", str(wt))
        (self.root / "broadsword.toml").write_text("root = '.'\nbuild_dir = 'other'\n", encoding="utf-8")
        env = {"CARGO_TARGET_DIR": str(self.repo / "build")}
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = fc.main(["--repo", str(wt), "--root", str(self.root)], env=env)
        self.assertEqual(code, 2, buf.getvalue())
        self.assertIn("shared build folder", buf.getvalue())
        self.assertEqual(list((self.root / "scratch").iterdir()), [])

    def test_not_refused_for_a_plain_shell_or_an_unrelated_target_dir_or_the_invented_marker(self):
        n, t = doc("python -I check.py")
        self.make_repo({n: t, "check.py": "print('ok')\n"})
        for env in ({}, {"CARGO_TARGET_DIR": str(self.tmp / "my-own-target")}, {"BROADSWORD_JOB": "20261120-1"}):
            with self.subTest(env=env):
                code, out = self.go(env)
                self.assertEqual(code, 0, out)
                self.assertEqual(self.leftovers(), [])
                self.assertFalse(fc.in_job(env, [self.root]))

    def test_unreadable_broadsword_toml_never_refuses_by_guessing(self):
        (self.root / "broadsword.toml").write_text("not = [valid", encoding="utf-8")
        self.assertIsNone(fc.in_job({"CARGO_TARGET_DIR": str(self.root / "build")}, [self.root]))

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
