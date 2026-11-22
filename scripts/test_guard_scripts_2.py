"""Tests for guard scripts set 2: venv-install.ps1, abandon-stuck.ps1, read-b-files.ps1, stop-driver.ps1 and
restart-orch.ps1 (allow rules point at them, so each one is a safety boundary that refuses by default).

Every case runs a COPY of the scripts inside a throwaway temp root (the scripts take their root from their own
location), so no real repo, venv, job record, queue driver or pc-progress file is touched. Where the real side effect
is unsafe in a test (pip install, recover --abandon) the case uses the script's --dry-run, which runs every check and
then only prints. stop-driver's accept cases stop REAL processes, but only throwaway powershell processes this suite
started, under a random project name no real driver uses.

    python -I -B scripts/test_guard_scripts_2.py              run the suite
    python -I -B scripts/test_guard_scripts_2.py --prove-fail  run it against deliberately weakened copies; each
                                                               weakening must make its named tests FAIL
"""
import calendar
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest
import uuid

SCRIPTS = os.path.dirname(os.path.abspath(__file__))
NAMES = ("venv-install.ps1", "abandon-stuck.ps1", "read-b-files.ps1", "stop-driver.ps1", "restart-orch.ps1")
# Set only by --prove-fail: {script name: [(old, new), ...]} applied to the temp copies.
WEAKEN = {}
GIT_ENV = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}


def ps(script, args, cwd, timeout=120):
    env = dict(os.environ)
    env.update(GIT_ENV)
    return subprocess.run(["powershell", "-NoProfile", "-File", script] + list(args), cwd=cwd, env=env,
                          capture_output=True, text=True, timeout=timeout)


def wr(path, text, mode="w"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, mode, newline="") as f:
        f.write(text)


def rd(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def git(repo, *args):
    env = dict(os.environ)
    env.update(GIT_ENV)
    r = subprocess.run(["git", "-C", repo] + list(args), env=env, capture_output=True, text=True)
    assert r.returncode == 0, (args, r.stderr)
    return r.stdout.strip()


def age(path, minutes):
    t = time.time() - minutes * 60
    os.utime(path, (t, t))


def stamp(minutes_ago):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(time.time() - minutes_ago * 60))


def junction(link, target):
    """A directory junction (mklink /J needs no admin). False if it cannot be made."""
    os.makedirs(target, exist_ok=True)
    r = subprocess.run(["cmd", "/c", "mklink", "/J", link, target], capture_output=True)
    return r.returncode == 0


def outside_dir(test):
    d = os.path.join(os.path.realpath(tempfile.gettempdir()), "gsx" + uuid.uuid4().hex[:10])
    os.makedirs(d)
    test.addCleanup(shutil.rmtree, d, True)
    return d


def alive(pid):
    import ctypes
    k = ctypes.windll.kernel32
    h = k.OpenProcess(0x1000, False, int(pid))  # PROCESS_QUERY_LIMITED_INFORMATION
    if not h:
        return False
    try:
        code = ctypes.c_ulong()
        k.GetExitCodeProcess(h, ctypes.byref(code))
        return code.value == 259  # STILL_ACTIVE
    finally:
        k.CloseHandle(h)


def make_root():
    # realpath: the long form of the temp folder (git reports long paths; the 8.3 form would not compare equal).
    root = os.path.join(os.path.realpath(tempfile.gettempdir()), "gs" + uuid.uuid4().hex[:10])
    os.makedirs(os.path.join(root, "scripts"))
    for name in NAMES:
        text = rd(os.path.join(SCRIPTS, name))
        for old, new in WEAKEN.get(name, []):
            assert old in text, f"weakening target missing in {name}: {old}"
            text = text.replace(old, new)
        with open(os.path.join(root, "scripts", name), "w", encoding="utf-8") as f:
            f.write(text)
    return root


def tree(root):
    out = {}
    for d, _, fs in os.walk(root):
        for f in fs:
            p = os.path.join(d, f)
            out[p] = (os.path.getsize(p), os.path.getmtime(p))
    return out


class Base(unittest.TestCase):
    script_name = None

    def setUp(self):
        self.root = make_root()
        self.addCleanup(shutil.rmtree, self.root, True)
        self.script = os.path.join(self.root, "scripts", self.script_name)

    def run_s(self, *args, timeout=120):
        return ps(self.script, args, self.root, timeout=timeout)

    def assertRefused(self, *args):
        r = self.run_s(*args)
        self.assertEqual(r.returncode, 2, (args, r.stdout, r.stderr))
        self.assertIn("REFUSED", r.stderr)
        self.assertNotIn("dry run", r.stdout)
        return r

    def assertAllowed(self, *args):
        r = self.run_s(*args)
        self.assertEqual(r.returncode, 0, (args, r.stdout, r.stderr))
        return r.stdout


class TestVenvInstall(Base):
    script_name = "venv-install.ps1"
    REQ = "shared/requirements-demo.txt"

    def setUp(self):
        super().setUp()
        git(self.root, "init", "-q", "-b", "main")
        wr(os.path.join(self.root, self.REQ), "numpy==2.1.0\nscipy==1.14.1\n")
        wr(os.path.join(self.root, "shared", "requirements-crlf.txt"), "numpy==2.1.0\r\nPyYAML==6.0.2\r\n")
        wr(os.path.join(self.root, "shared", "notreq.txt"), "numpy==2.1.0\n")
        wr(os.path.join(self.root, "requirements-top.txt"), "numpy==2.1.0\n")
        wr(os.path.join(self.root, "shared", "sub", "requirements-x.txt"), "numpy==2.1.0\n")
        git(self.root, "add", "shared", "requirements-top.txt")
        git(self.root, "commit", "-q", "-m", "reqs")

    def commit_req(self, name, text):
        wr(os.path.join(self.root, "shared", name), text)
        git(self.root, "add", "shared/" + name)
        git(self.root, "commit", "-q", "-m", name)
        return "shared/" + name

    def test_accepts_tracked_pinned_file_dry_run(self):
        out = self.assertAllowed(self.REQ, "--dry-run")
        self.assertIn("python -I -m venv", out)
        self.assertIn("PIP_CONFIG_FILE=nul ", out)
        self.assertIn("-I -m pip install --isolated --only-binary=:all: --disable-pip-version-check --report ", out)
        head = git(self.root, "rev-parse", "HEAD")
        sha = hashlib.sha256(b"numpy==2.1.0\nscipy==1.14.1\n").hexdigest()
        self.assertIn(f"{self.REQ} at {head}, sha256 {sha}", out)
        self.assertIn("dry run, nothing executed", out)
        self.assertFalse(os.path.exists(os.path.join(self.root, "shared", "venv")), "dry run created the venv")
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")), "dry run wrote the copy")

    def test_accepts_crlf_and_backslash_path_with_existing_venv(self):
        wr(os.path.join(self.root, "shared", "venv", "Scripts", "python.exe"), "")
        out = self.assertAllowed("shared\\requirements-crlf.txt", "--dry-run")
        self.assertNotIn("-m venv", out)
        self.assertIn("pip install", out)

    def test_refuses_wrong_paths(self):
        self.assertRefused("shared/notreq.txt", "--dry-run")
        self.assertRefused("requirements-top.txt", "--dry-run")
        self.assertRefused("shared/sub/requirements-x.txt", "--dry-run")
        self.assertRefused("shared/../shared/requirements-demo.txt", "--dry-run")
        self.assertRefused(os.path.join(self.root, "shared", "requirements-demo.txt"), "--dry-run")
        self.assertRefused("shared/REQUIRE~1.TXT", "--dry-run")
        self.assertRefused("shared/requirements-missing.txt", "--dry-run")
        self.assertRefused("", "--dry-run")

    def test_refuses_extra_and_injected_arguments(self):
        self.assertRefused()
        self.assertRefused(self.REQ, "--dry-run", "x")
        self.assertRefused(self.REQ, "--index-url", "http://evil")
        self.assertRefused("--dry-run", self.REQ)
        self.assertRefused(self.REQ + "; rm -rf x", "--dry-run")
        self.assertRefused(self.REQ + " -r other.txt", "--dry-run")
        self.assertRefused(self.REQ, "--DRY-RUN")

    def test_refuses_untracked_file(self):
        wr(os.path.join(self.root, "shared", "requirements-new.txt"), "numpy==2.1.0\n")
        self.assertRefused("shared/requirements-new.txt", "--dry-run")

    def test_refuses_modified_file(self):
        wr(os.path.join(self.root, self.REQ), "numpy==2.1.0\nscipy==1.14.1\nrequests==2.32.3\n")
        self.assertRefused(self.REQ, "--dry-run")
        # staged but not committed is modified against HEAD too
        git(self.root, "add", self.REQ)
        self.assertRefused(self.REQ, "--dry-run")

    def test_refuses_skip_worktree_swap(self):
        # r3 B2: `git diff` cannot see a skip-worktree file, so other (well-formed, pinned) requirements in the worktree
        # passed the diff and their bytes reached pip. Now pip gets only the committed blob, and a worktree that differs
        # from it refuses.
        git(self.root, "update-index", "--skip-worktree", self.REQ)
        wr(os.path.join(self.root, self.REQ), "requests==2.32.3\n")
        r = subprocess.run(["git", "-C", self.root, "diff", "--quiet", "HEAD", "--", self.REQ])
        self.assertEqual(r.returncode, 0, "setup: git diff should not see the skip-worktree change")
        self.assertRefused(self.REQ, "--dry-run")
        self.assertRefused(self.REQ)
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")), "a refusal wrote the copy")

    def test_refuses_bad_lines(self):
        bad = ["https://evil.example/pkg.whl\n", "-e .\n", "--index-url https://evil\n", "-r other.txt\n",
               "numpy>=2.0\n", "numpy\n", "numpy[extra]==2.1.0\n", "numpy==2.1.0 ; python_version>'3'\n",
               "numpy==2.1.0\n\nscipy==1.14.1\n", "# comment\nnumpy==2.1.0\n", "numpy==2.1.0 --hash=sha256:00\n",
               "numpy @ https://evil/x.whl\n", "numpy==2.1.0\n--extra-index-url https://evil\n", "numpy==\u00e92\n"]
        for i, text in enumerate(bad):
            rel = self.commit_req(f"requirements-bad{i}.txt", text)
            self.assertRefused(rel, "--dry-run")

    def test_refuses_scratch_junction(self):
        # F3: the validated copy is written under scratch/venv-install; a junction there would put it elsewhere.
        out = outside_dir(self)
        if not junction(os.path.join(self.root, "scratch"), out):
            self.skipTest("cannot create a junction")
        self.assertRefused(self.REQ, "--dry-run")
        self.assertRefused(self.REQ)
        self.assertEqual(os.listdir(out), [], "something was written through the junction")

    def test_refuses_venv_junction(self):
        out = outside_dir(self)
        wr(os.path.join(out, "Scripts", "python.exe"), "")
        if not junction(os.path.join(self.root, "shared", "venv"), out):
            self.skipTest("cannot create a junction")
        self.assertRefused(self.REQ, "--dry-run")

    def test_refuses_venv_folder_without_python(self):
        os.makedirs(os.path.join(self.root, "shared", "venv", "Lib"))
        self.assertRefused(self.REQ, "--dry-run")


def bundled_pip_wheel():
    import ensurepip
    d = os.path.join(os.path.dirname(ensurepip.__file__), "_bundled")
    return ensurepip.version(), os.path.join(d, [f for f in os.listdir(d) if f.startswith("pip-")][0])


class TestVenvInstallReal(Base):
    """Real venv cases (no network): a real shared/venv made by this python, and the requirement `pip==<its version>`,
    which the venv already has, so a correct run is "already satisfied" and never contacts an index."""
    script_name = "venv-install.ps1"
    REQ = "shared/requirements-pip.txt"

    def setUp(self):
        super().setUp()
        self.pip_version, self.wheel = bundled_pip_wheel()
        git(self.root, "init", "-q", "-b", "main")
        wr(os.path.join(self.root, self.REQ), f"pip=={self.pip_version}\n")
        git(self.root, "add", "shared")
        git(self.root, "commit", "-q", "-m", "reqs")
        self.venv = os.path.join(self.root, "shared", "venv")
        r = subprocess.run([sys.executable, "-I", "-m", "venv", self.venv], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.vpy = os.path.join(self.venv, "Scripts", "python.exe")

    def test_site_config_cannot_redirect_the_install(self):
        # r3 B1: shared/venv/pip.ini (pip's SITE config, read even with --isolated) and a PIP_CONFIG_FILE / PIP_TARGET
        # in the caller's environment all say: install into a folder outside the venv, from a local wheel folder.
        # pip must read none of it: the run is "already satisfied" in the venv and nothing appears outside.
        outside = outside_dir(self)
        target = os.path.join(outside, "target")
        wheels = os.path.join(outside, "wheels")
        os.makedirs(wheels)
        shutil.copy(self.wheel, wheels)
        ini = (f"[global]\nno-index = true\nfind-links = {wheels}\ndisable-pip-version-check = true\n"
               f"[install]\ntarget = {target}\n")
        wr(os.path.join(self.venv, "pip.ini"), ini)
        envini = os.path.join(outside, "env-pip.ini")
        wr(envini, ini)
        env = dict(os.environ)
        env.update(GIT_ENV)
        env.update({"PIP_CONFIG_FILE": envini, "PIP_TARGET": target, "PIP_NO_INDEX": "1", "PIP_FIND_LINKS": wheels})
        r = subprocess.run(["powershell", "-NoProfile", "-File", self.script, self.REQ], cwd=self.root, env=env,
                           capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        self.assertIn("post-check ok", r.stdout)
        self.assertFalse(os.path.exists(target) and os.listdir(target), f"pip wrote outside the venv: {target}")

    def post_check(self, report, reqs):
        text = rd(self.script)
        m = re.search(r"\$postCheck = @'\r?\n(.*?)\r?\n'@", text, re.S)
        self.assertIsNotNone(m, "post-check block not found")
        d = os.path.join(self.root, "scratch")
        wr(os.path.join(d, "check.py"), m.group(1))
        wr(os.path.join(d, "report.json"), json.dumps(report))
        wr(os.path.join(d, "reqs.txt"), reqs)
        return subprocess.run([self.vpy, "-I", os.path.join(d, "check.py"), self.venv, os.path.join(d, "report.json"),
                               os.path.join(d, "reqs.txt")], capture_output=True, text=True)

    def test_post_check_refuses_what_is_not_in_the_venv(self):
        # r3 one-place rule: after pip, everything pip reports and every requirement must be installed IN the venv.
        req = f"pip=={self.pip_version}\n"
        ok = self.post_check({"install": [{"metadata": {"name": "pip", "version": self.pip_version}}]}, req)
        self.assertEqual(ok.returncode, 0, (ok.stdout, ok.stderr))
        self.assertIn("post-check ok", ok.stdout)
        missing = self.post_check({"install": [{"metadata": {"name": "gs-not-there", "version": "1.0"}}]}, req)
        self.assertEqual(missing.returncode, 1, (missing.stdout, missing.stderr))
        self.assertIn("gs-not-there is not installed in the venv", missing.stdout)
        wrong = self.post_check({"install": []}, "pip==1.0\n")
        self.assertEqual(wrong.returncode, 1, (wrong.stdout, wrong.stderr))
        self.assertIn("not 1.0", wrong.stdout)


class TestAbandonStuck(Base):
    script_name = "abandon-stuck.ps1"
    ID = "20261122-120000-demo-run"

    def setUp(self):
        super().setUp()
        self.pc = os.path.join(self.root, "state", "pc-progress.md")
        wr(self.pc, "2026-11-22T12:00:00Z start\n")
        self.log = os.path.join(self.root, "scratch", "logs", "jobs", self.ID + ".log")
        wr(self.log, "log\n")

    def rec(self, jid=None, status="running", pid="", pid_start="", log=None, rec_id=None, minutes=0):
        jid = jid or self.ID
        log = self.log.replace("\\", "\\\\") if log is None else log
        path = os.path.join(self.root, "state", "jobs", jid + ".rec")
        wr(path, f"command = x\nid = {rec_id or jid}\nlog = {log}\npid = {pid}\npid_start = {pid_start}\n"
                 f"project = demo\nstatus = {status}\n")
        if minutes:
            age(path, minutes)
            age(self.log, minutes)
        return path

    def dead_pid(self):
        p = subprocess.Popen(["cmd", "/c", "exit"], stdout=subprocess.DEVNULL)
        p.wait()
        return str(p.pid)

    def pc_text(self):
        return rd(self.pc)

    def test_accepts_dead_pid_dry_run(self):
        self.rec(pid=self.dead_pid())
        out = self.assertAllowed(self.ID, "--dry-run")
        self.assertIn("recover --abandon " + self.ID, out)
        self.assertIn("is not alive", out)
        self.assertNotIn("abandon-stuck", self.pc_text(), "a dry run appended to pc-progress")

    def test_accepts_stale_log_dry_run(self):
        self.rec(pid=str(os.getpid()), minutes=40)
        out = self.assertAllowed(self.ID, "--dry-run")
        self.assertIn("unchanged for", out)

    def test_accepts_reused_pid_dry_run(self):
        self.rec(pid=str(os.getpid()), pid_start="1")
        out = self.assertAllowed(self.ID, "--dry-run")
        self.assertIn("another process", out)

    def test_real_run_calls_recover_and_appends_one_line(self):
        # A stand-in broadsword.exe (whoami.exe: it rejects the arguments and exits non-zero); the point is that the
        # script runs `<root>/build/bin/broadsword.exe recover --abandon <id>` and logs exactly one line with its exit.
        who = os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "whoami.exe")
        if not os.path.exists(who):
            self.skipTest("no whoami.exe")
        os.makedirs(os.path.join(self.root, "build", "bin"))
        shutil.copy(who, os.path.join(self.root, "build", "bin", "broadsword.exe"))
        self.rec(pid=self.dead_pid())
        before = self.pc_text()
        r = self.run_s(self.ID)
        self.assertNotIn("REFUSED", r.stderr, (r.stdout, r.stderr))
        self.assertIn(f"recover --abandon {self.ID}", r.stdout)
        added = self.pc_text()[len(before):]
        self.assertEqual(added.count("\n"), 1, added)
        self.assertIn(f"abandon-stuck: recover --abandon {self.ID} (pid ", added)
        self.assertIn(f"-> exit {r.returncode}", added)

    def test_refuses_live_fresh_job(self):
        self.rec(pid=str(os.getpid()))
        self.assertRefused(self.ID, "--dry-run")
        self.assertRefused(self.ID)
        self.assertNotIn("abandon-stuck", self.pc_text())

    def test_refuses_when_neither_pid_nor_log_readable(self):
        self.rec(pid="", log="")
        self.assertRefused(self.ID, "--dry-run")
        self.rec(pid="abc", log="C:\\\\nowhere\\\\x.log")
        self.assertRefused(self.ID, "--dry-run")

    def test_refuses_unreadable_pid_with_fresh_log(self):
        self.rec(pid="")
        self.assertRefused(self.ID, "--dry-run")

    def test_refuses_stale_log_outside_root(self):
        outside = os.path.join(os.path.realpath(tempfile.gettempdir()), "gso" + uuid.uuid4().hex[:8] + ".log")
        wr(outside, "x")
        self.addCleanup(os.remove, outside)
        age(outside, 90)
        path = self.rec(pid="", log=outside.replace("\\", "\\\\"))
        age(path, 90)
        self.assertRefused(self.ID, "--dry-run")

    def test_refuses_job_not_running(self):
        for st in ("done", "failed", "abandoned", "rolled_back"):
            self.rec(pid=self.dead_pid(), status=st)
            self.assertRefused(self.ID, "--dry-run")

    def test_refuses_bad_ids_and_arguments(self):
        self.rec(pid=self.dead_pid())
        self.assertRefused()
        self.assertRefused(self.ID, "--dry-run", "x")
        self.assertRefused(self.ID, "--all")
        self.assertRefused("--all")
        self.assertRefused("x", "--dry-run")
        self.assertRefused("../" + self.ID, "--dry-run")
        self.assertRefused(self.ID + "; rm -rf x", "--dry-run")
        self.assertRefused(self.ID.upper(), "--dry-run")
        self.assertRefused("20261122-120000-other-run", "--dry-run")  # no record

    def test_refuses_build_junction(self):
        # F4: build/bin/broadsword.exe is run; a junction on build would run a program from elsewhere.
        out = outside_dir(self)
        wr(os.path.join(out, "bin", "broadsword.exe"), "")
        if not junction(os.path.join(self.root, "build"), out):
            self.skipTest("cannot create a junction")
        self.rec(pid=self.dead_pid())
        self.assertRefused(self.ID, "--dry-run")
        self.assertRefused(self.ID)
        self.assertNotIn("abandon-stuck", self.pc_text())

    def test_refuses_record_with_other_id(self):
        self.rec(pid=self.dead_pid(), rec_id="20261122-120001-demo-run")
        self.assertRefused(self.ID, "--dry-run")


class TestReadBFiles(Base):
    script_name = "read-b-files.ps1"
    TASK = "t-20261122-000001"
    FILES = ("scenarios/validation_car_b.bsn", "scenarios/validation_wall_b.bsn", "validation/CAR_B.md")

    def setUp(self):
        super().setUp()
        for base in ("projects/crash-sim", f"worktrees/crash-sim/{self.TASK}"):
            for f in self.FILES:
                wr(os.path.join(self.root, base, f), f"CONTENT {base} {f}\n")
            wr(os.path.join(self.root, base, "validation", "results_b.csv"), "SECRET-RESULT\n")
            wr(os.path.join(self.root, base, "results", "CAR_B.md"), "SECRET-RESULT\n")

    def test_prints_exactly_the_three_main_files(self):
        before = tree(self.root)
        out = self.assertAllowed()
        for f in self.FILES:
            self.assertIn(f"===== projects/crash-sim/{f} =====", out)
            self.assertIn(f"CONTENT projects/crash-sim {f}", out)
        self.assertEqual(out.count("====="), 6)
        self.assertNotIn("SECRET-RESULT", out)
        self.assertNotIn("results", out)
        self.assertEqual(tree(self.root), before, "read-b-files wrote something")

    def test_prints_the_task_worktree_files(self):
        out = self.assertAllowed("--task", self.TASK)
        for f in self.FILES:
            self.assertIn(f"CONTENT worktrees/crash-sim/{self.TASK} {f}", out)
        self.assertNotIn("CONTENT projects/crash-sim", out)

    def test_refuses_any_other_argument(self):
        self.assertRefused("projects/crash-sim/validation/results_b.csv")
        self.assertRefused("--task")
        self.assertRefused("--task", "x")
        self.assertRefused("--task", "../../projects/crash-sim")
        self.assertRefused("--task", self.TASK + "; type x")
        self.assertRefused("--task", self.TASK, "extra")
        self.assertRefused("--TASK", self.TASK)
        self.assertRefused("--file", "results/CAR_B.md")

    def test_refuses_missing_file_before_printing(self):
        os.remove(os.path.join(self.root, "projects", "crash-sim", "validation", "CAR_B.md"))
        r = self.assertRefused()
        self.assertEqual(r.stdout.strip(), "", "printed something before refusing")

    def test_refuses_link_to_results(self):
        # validation -> a junction to the results folder: the fixed path CAR_B.md would then print a result file.
        base = os.path.join(self.root, "worktrees", "crash-sim", self.TASK)
        shutil.rmtree(os.path.join(base, "validation"))
        link = os.path.join(base, "validation")
        r = subprocess.run(["cmd", "/c", "mklink", "/J", link, os.path.join(base, "results")], capture_output=True)
        if r.returncode != 0:
            self.skipTest("cannot create a junction")
        r = self.assertRefused("--task", self.TASK)
        self.assertNotIn("SECRET-RESULT", r.stdout)


DRIVER_BODY = "Start-Sleep -Seconds 120\n"


class TestStopDriver(Base):
    script_name = "stop-driver.ps1"

    def setUp(self):
        super().setUp()
        self.proj = "gtp" + uuid.uuid4().hex[:8]
        self.q = os.path.join(self.root, "state", "queue")
        os.makedirs(self.q)
        wr(os.path.join(self.root, "scratch", "queue-driver-project.ps1"), DRIVER_BODY)
        self.procs = []
        self.addCleanup(self.kill_all)

    def kill_all(self):
        for p in self.procs:
            if p.poll() is None:
                p.kill()
                p.wait()
            if p.stdin:
                p.stdin.close()

    def spawn(self, args, cwd=None):
        p = subprocess.Popen(["powershell"] + args, cwd=cwd or self.root, stdin=subprocess.PIPE,
                             stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        return p

    def driver(self, proj=None, cwd=None, script="scratch/queue-driver-project.ps1"):
        return self.spawn(["-NoProfile", "-File", script, proj or self.proj], cwd=cwd)

    def other_root(self):
        other = outside_dir(self)
        wr(os.path.join(other, "scratch", "queue-driver-project.ps1"), DRIVER_BODY)
        return other

    def test_leaves_a_driver_of_another_root_alone(self):
        # F1: the same relative command line, started in another copy of the project folder, is not this root's driver.
        other = self.other_root()
        theirs = self.driver(cwd=other)
        ours = self.driver()
        time.sleep(1.5)
        out = self.assertAllowed(self.proj)
        self.assertIn(f"stop pid {ours.pid}", out)
        self.assertNotIn(f"stop pid {theirs.pid}", out)
        self.assertTrue(self.gone(ours), "this root's driver is still running")
        time.sleep(1)
        self.assertIsNone(theirs.poll(), "a driver of another folder was stopped")

    def test_absolute_script_path_must_be_under_this_root(self):
        other = self.other_root()
        ours_abs = self.driver(cwd=other, script=os.path.join(self.root, "scratch", "queue-driver-project.ps1"))
        theirs_abs = self.driver(script=os.path.join(other, "scratch", "queue-driver-project.ps1"))
        time.sleep(1.5)
        out = self.assertAllowed(self.proj)
        self.assertIn(f"stop pid {ours_abs.pid}", out)
        self.assertTrue(self.gone(ours_abs), "this root's driver (absolute path) is still running")
        time.sleep(1)
        self.assertIsNone(theirs_abs.poll(), "a driver whose script is in another folder was stopped")

    def test_never_stops_the_queue_run_child(self):
        # F2: the child `broadsword.exe queue run --all --project <p>` is left to finish its drain. The stand-in child
        # is cmd.exe renamed broadsword.exe; it waits on the stdin pipe this test holds, so it outlives its driver.
        exe = os.path.join(self.root, "build", "bin", "broadsword.exe")
        os.makedirs(os.path.dirname(exe))
        shutil.copy(os.path.join(os.environ.get("SystemRoot", r"C:\Windows"), "System32", "cmd.exe"), exe)
        wr(os.path.join(self.root, "scratch", "queue-driver-project.ps1"),
           "$psi = New-Object Diagnostics.ProcessStartInfo((Join-Path (Get-Location) 'build\\bin\\broadsword.exe'), "
           "\"queue run --all --project $($args[0])\")\n$psi.UseShellExecute = $false\n"
           "$c = [Diagnostics.Process]::Start($psi)\n"
           "[IO.File]::WriteAllText((Join-Path (Get-Location) 'child.pid'), [string]$c.Id)\nStart-Sleep -Seconds 120\n")
        d = self.driver()
        pidfile = os.path.join(self.root, "child.pid")
        for _ in range(100):
            if os.path.exists(pidfile) and rd(pidfile).strip():
                break
            time.sleep(0.1)
        child = int(rd(pidfile).strip())
        self.addCleanup(subprocess.run, ["taskkill", "/PID", str(child), "/F"], capture_output=True)
        self.assertTrue(alive(child), "the stand-in child did not start")
        out = self.assertAllowed(self.proj)
        self.assertIn(f"stop pid {d.pid}", out)
        self.assertNotIn(f"stop pid {child}", out)
        self.assertIn(f"child pid {child} is left to finish its drain", out)
        self.assertTrue(self.gone(d), "the driver is still running")
        time.sleep(1)
        self.assertTrue(alive(child), "the queue-run child was stopped")

    def entry(self, eid, status, project=None):
        path = os.path.join(self.q, eid + ".entry")
        wr(path, f"id = {eid}\nproject = {project or self.proj}\nstatus = {status}\n")
        return path

    def gone(self, p, secs=15):
        try:
            p.wait(timeout=secs)
            return True
        except subprocess.TimeoutExpired:
            return False

    def test_stops_the_driver_and_nothing_else(self):
        d = self.driver()
        other = self.driver(self.proj + "x")
        decoy = self.spawn(["-NoProfile", "-Command", f"Start-Sleep 120 # -File scratch/queue-driver-project.ps1 {self.proj}"])
        self.entry("q-1", "merged")
        self.entry("q-2", "running", project="someother")
        time.sleep(1.5)
        out = self.assertAllowed(self.proj)
        self.assertIn(f"stop pid {d.pid}", out)
        self.assertTrue(self.gone(d), "the driver is still running")
        self.assertIsNone(other.poll(), "a driver of another project was stopped")
        self.assertIsNone(decoy.poll(), "a process that only mentions the driver was stopped")

    def test_dry_run_stops_nothing(self):
        d = self.driver()
        time.sleep(1.5)
        out = self.assertAllowed(self.proj, "--dry-run")
        self.assertIn(f"stop pid {d.pid}", out)
        self.assertIn("dry run, nothing stopped", out)
        time.sleep(1)
        self.assertIsNone(d.poll())

    def test_nothing_running_is_not_an_error(self):
        out = self.assertAllowed(self.proj)
        self.assertIn("nothing stopped", out)

    def test_waits_for_the_running_entry_when_asked(self):
        d = self.driver()
        e = self.entry("q-1", "running")
        threading.Timer(6, lambda: wr(e, f"id = q-1\nproject = {self.proj}\nstatus = merged\n")).start()
        time.sleep(1.5)
        out = self.assertAllowed(self.proj, "--when-idle-wait")
        self.assertIn("waiting up to 30 min", out)
        self.assertTrue(self.gone(d))

    def test_refuses_while_an_entry_is_running(self):
        d = self.driver()
        self.entry("q-1", "running")
        time.sleep(1.5)
        self.assertRefused(self.proj)
        self.assertRefused(self.proj, "--dry-run")
        time.sleep(1)
        self.assertIsNone(d.poll(), "the driver was stopped during a merge")

    def test_refuses_bad_arguments(self):
        d = self.driver()
        time.sleep(1.5)
        self.assertRefused()
        self.assertRefused(self.proj.upper())
        self.assertRefused("../" + self.proj)
        self.assertRefused(self.proj + "; Stop-Process -Name powershell")
        self.assertRefused("")
        self.assertRefused(self.proj, "--all")
        self.assertRefused(self.proj, "--when-idle-wait", "--when-idle-wait")
        self.assertRefused(self.proj, "--dry-run", "--dry-run")
        self.assertRefused(self.proj, "--force")
        self.assertIsNone(d.poll())

    def test_refuses_without_a_queue_folder(self):
        d = self.driver()
        shutil.rmtree(self.q)
        time.sleep(1.5)
        self.assertRefused(self.proj)
        self.assertIsNone(d.poll())

    def test_no_kill_by_pid(self):
        # r2 B3: the kill goes through the verified handle only; nothing in the script kills by pid or by name.
        text = rd(self.script)
        for bad in ("Stop-Process", "taskkill", ".Kill(", "wmic"):
            self.assertNotIn(bad, text)
        self.assertIn("[BsProcCwd]::Terminate($h)", text)
        self.assertIn("[BsProcCwd]::SameStart($h, $t.CreationDate)", text)

    def test_stop_goes_through_a_verified_handle(self):
        # r2 B3: the helper the stop uses, loaded from the script copy itself. The handle's creation time equals CIM's
        # CreationDate (and a different time does not), terminate through the handle kills exactly that process, and
        # once it has exited the same handle can neither pass as live nor kill anything (it never follows the pid).
        probe = os.path.join(self.root, "scratch", "probe.ps1")
        wr(probe, "$src = [IO.File]::ReadAllText($args[0])\n"
                  "$m = [regex]::Match($src, \"(?s)Add-Type -TypeDefinition @'\\r?\\n(.*?)\\r?\\n'@\")\n"
                  "Add-Type -TypeDefinition $m.Groups[1].Value\n"
                  "$pid2 = [int]$args[1]\n"
                  "$h = [BsProcCwd]::OpenForStop($pid2)\n"
                  "\"open=$($h -ne [IntPtr]::Zero)\"\n"
                  "$cim = Get-CimInstance Win32_Process -Filter \"ProcessId = $pid2\"\n"
                  "\"same=$([BsProcCwd]::SameStart($h, $cim.CreationDate))\"\n"
                  "\"other=$([BsProcCwd]::SameStart($h, $cim.CreationDate.AddMilliseconds(-1)))\"\n"
                  "\"exited=$([BsProcCwd]::HasExited($h))\"\n"
                  "\"term=$([BsProcCwd]::Terminate($h))\"\n"
                  "Start-Sleep -Milliseconds 1500\n"
                  "\"exited2=$([BsProcCwd]::HasExited($h))\"\n"
                  "\"term2=$([BsProcCwd]::Terminate($h))\"\n"
                  "[BsProcCwd]::Close($h)\n")
        victim = self.spawn(["-NoProfile", "-Command", "Start-Sleep 120"])
        bystander = self.spawn(["-NoProfile", "-Command", "Start-Sleep 120"])
        time.sleep(1)
        r = ps(probe, [self.script, str(victim.pid)], self.root)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        lines = r.stdout.split()
        for want in ("open=True", "same=True", "other=False", "exited=False", "term=True", "exited2=True", "term2=False"):
            self.assertIn(want, lines, (r.stdout, r.stderr))
        self.assertTrue(self.gone(victim), "terminate through the handle did not stop the process")
        self.assertIsNone(bystander.poll(), "another process was stopped")


class TestRestartOrch(Base):
    script_name = "restart-orch.ps1"

    def setUp(self):
        super().setUp()
        self.state = os.path.join(self.root, "state")
        self.req = os.path.join(self.state, "restart-request.md")
        self.handoff = os.path.join(self.state, "handoff-2026-11-22a.md")
        wr(self.handoff, "handoff\n")
        age(self.handoff, 90)
        old = os.path.join(self.state, "handoff-2026-11-21z.md")
        wr(old, "older handoff\n")
        age(old, 600)

    def pc(self, *lines):
        wr(os.path.join(self.state, "pc-progress.md"), "".join(l + "\n" for l in lines))

    def good_pc(self):
        self.pc(f"{stamp(180)} orchestrator 10 at context budget: handoff written",
                f"{stamp(120)} fd03 queued",
                f"{stamp(1)} AUTO-START t-1 (tick-loop)")

    def test_writes_a_request(self):
        self.good_pc()
        out = self.assertAllowed()
        text = rd(self.req)
        self.assertIn("handoff: state/handoff-2026-11-22a.md", text)
        self.assertRegex(text, r"(?m)^time: \d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertIn("reason: the last non-(tick-loop) pc-progress line", text)
        self.assertIn("wrote state/restart-request.md", out)

    def test_old_request_does_not_block(self):
        self.good_pc()
        wr(self.req, f"# restart request\n\ntime: {stamp(200)}\n")
        age(self.req, 200)
        self.assertAllowed()
        self.assertIn("handoff: state/handoff-2026-11-22a.md", rd(self.req))

    def assertRefusedNoWrite(self, *args):
        before = rd(self.req) if os.path.exists(self.req) else None
        self.assertRefused(*args)
        after = rd(self.req) if os.path.exists(self.req) else None
        self.assertEqual(after, before, "restart-request.md changed on a refusal")

    def test_refuses_arguments(self):
        self.good_pc()
        self.assertRefusedNoWrite("--force")
        self.assertRefusedNoWrite("now")
        self.assertRefusedNoWrite("; Start-Process claude")

    def test_refuses_handoff_older_than_last_line(self):
        self.pc(f"{stamp(180)} orchestrator 10 handoff", f"{stamp(70)} fd04 requeued", f"{stamp(1)} x (tick-loop)")
        self.assertRefusedNoWrite()

    def test_refuses_without_a_handoff(self):
        self.good_pc()
        os.remove(self.handoff)
        os.remove(os.path.join(self.state, "handoff-2026-11-21z.md"))
        self.assertRefusedNoWrite()

    def test_real_handoff_pattern_fires(self):
        # F6: the real pattern of 2026-11-22: handoff file written 17:22:15Z, "handoff written" line logged 17:22:16Z,
        # then only (tick-loop) lines. Shifted to yesterday's date so the line is over 60 min old.
        day = time.strftime("%Y-%m-%d", time.gmtime(time.time() - 86400))
        t = calendar.timegm(time.strptime(day + " 17:22:15", "%Y-%m-%d %H:%M:%S"))
        os.utime(self.handoff, (t, t))
        age(os.path.join(self.state, "handoff-2026-11-21z.md"), 3 * 1440)
        self.pc(f"{day}T16:58:02Z fd03 queued",
                f"{day}T17:22:16Z orchestrator 10 at context budget: handoff state/handoff-2026-11-22a.md written. PAUSED",
                f"{day}T17:40:00Z AUTO-START t-1 orchestrator relay (tick-loop)",
                f"{stamp(1)} AUTO-START t-2 (tick-loop)")
        out = self.assertAllowed()
        self.assertIn("handoff: state/handoff-2026-11-22a.md", rd(self.req))
        self.assertIn("wrote state/restart-request.md", out)

    def test_refuses_recent_last_line(self):
        # The handoff is newer than the line, but the orchestrator wrote that line under 60 min ago.
        age(self.handoff, 35)
        self.pc(f"{stamp(180)} orchestrator 10 handoff", f"{stamp(40)} fd04 requeued", f"{stamp(1)} x (tick-loop)")
        self.assertRefusedNoWrite()
        self.pc(f"{stamp(180)} orchestrator 10 handoff", f"{stamp(59)} fd04 requeued")
        self.assertRefusedNoWrite()

    def handoff_before_line(self, seconds):
        # The line is stamped (whole seconds) 90 min ago; the handoff file is `seconds` before that stamp.
        line_t = int(time.time() - 90 * 60)
        os.utime(self.handoff, (line_t - seconds, line_t - seconds))
        line = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(line_t))
        self.pc(f"{stamp(180)} x", f"{line} orchestrator 10 handoff written", f"{stamp(1)} y (tick-loop)")

    def test_handoff_up_to_60_s_before_the_line_fires(self):
        self.handoff_before_line(55)
        self.assertAllowed()

    def test_refuses_handoff_over_60_s_before_the_line(self):
        # r2 B1: the grace is 60 s, not 5 min. 61 s and 4 min before the line both refuse.
        self.handoff_before_line(61)
        self.assertRefusedNoWrite()
        self.handoff_before_line(240)
        self.assertRefusedNoWrite()

    def test_refuses_recent_line_mentioning_tick_loop(self):
        # r2 B2: only lines that END with "(tick-loop)" are skipped; a recent orchestrator line that mentions it in the
        # middle is the last orchestrator line, and it is under 60 min old, so the script refuses.
        age(self.handoff, 30)
        self.pc(f"{stamp(180)} orchestrator 10 handoff written",
                f"{stamp(20)} orchestrator 12 restarted the (tick-loop) helper, pid 1",
                f"{stamp(1)} AUTO-START t-1 (tick-loop)")
        self.assertRefusedNoWrite()
        # a (tick-loop) line with trailing spaces is still a tick-loop line
        self.pc(f"{stamp(180)} orchestrator 10 handoff written", f"{stamp(1)} AUTO-START t-1 (tick-loop)   ")
        age(self.handoff, 180.5)
        self.assertAllowed()

    def test_refuses_while_another_run_holds_the_lock(self):
        # F5: the lock file exists (another run, or a stale one): refuse, write nothing.
        self.good_pc()
        lock = os.path.join(self.state, "restart-request.lock")
        wr(lock, "")
        self.assertRefusedNoWrite()
        self.assertFalse(os.path.exists(self.req))

    def test_concurrent_runs_write_one_request(self):
        # F5: several runs at once: exactly one passes, the rest refuse; no lock file is left behind.
        self.good_pc()
        env = dict(os.environ)
        procs = [subprocess.Popen(["powershell", "-NoProfile", "-File", self.script], cwd=self.root, env=env,
                                  stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(4)]
        codes = [p.wait(timeout=120) for p in procs]
        for p in procs:
            p.stdout.close()
            p.stderr.close()
        self.assertEqual(sorted(codes), [0, 2, 2, 2], codes)
        self.assertFalse(os.path.exists(os.path.join(self.state, "restart-request.lock")), "the lock file was left")

    def test_refuses_untimed_last_line(self):
        self.pc(f"{stamp(180)} orchestrator 10", "garbled line without a time", f"{stamp(1)} x (tick-loop)")
        self.assertRefusedNoWrite()

    def test_refuses_recent_request(self):
        self.good_pc()
        wr(self.req, "# restart request\n\ntime: 2026-01-01T00:00:00Z\n")
        age(self.req, 60)
        self.assertRefusedNoWrite()
        # an old file whose time: line is recent refuses too
        wr(self.req, f"# restart request\n\ntime: {stamp(30)}\n")
        age(self.req, 300)
        self.assertRefusedNoWrite()


# --prove-fail: each weakening must make the named tests fail (the suite is the thing being proven).
WEAKENINGS = [
    ("venv-install.ps1 without the unmodified check",
     {"venv-install.ps1": [("if ($LASTEXITCODE -ne 0) { Refuse \"'$relIn' is modified", "if ($false) { Refuse \"'$relIn' is modified"),
                           ("if ($wh -ne (Sha256Hex $bytes) -and", "if ($false -and")]},
     ["test_refuses_modified_file"]),
    ("venv-install.ps1 reading the working file instead of the committed blob (the round-3 behaviour)",
     {"venv-install.ps1": [("$bytes = GitBlob \"${rev}:$relIn\"", "$bytes = [IO.File]::ReadAllBytes($full)")]},
     ["test_refuses_skip_worktree_swap"]),
    ("venv-install.ps1 without PIP_CONFIG_FILE=nul (pip reads its config files again)",
     {"venv-install.ps1": [("$env:PIP_CONFIG_FILE = 'nul'\n", "")]},
     ["test_site_config_cannot_redirect_the_install"]),
    ("venv-install.ps1 post-check without the not-installed rule",
     {"venv-install.ps1": [("        bad.append(f\"{name} is not installed in the venv\")\n", "        pass\n")]},
     ["test_post_check_refuses_what_is_not_in_the_venv"]),
    ("venv-install.ps1 without the name==version rule",
     {"venv-install.ps1": [("-cnotmatch '^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?==[0-9][A-Za-z0-9.!+]*$'", "-cnotmatch '.*'")]},
     ["test_refuses_bad_lines"]),
    ("abandon-stuck.ps1 without the live-pid check",
     {"abandon-stuck.ps1": [("$proc = Get-Process -Id ([int]$pidText) -ErrorAction SilentlyContinue", "$proc = $null")]},
     ["test_refuses_live_fresh_job"]),
    ("abandon-stuck.ps1 without the log-under-root rule",
     {"abandon-stuck.ps1": [("$logFull.StartsWith($root + $sep, [StringComparison]::OrdinalIgnoreCase) -and ", ""),
                            ("if (-not $p.StartsWith($root.TrimEnd($sep) + $sep, [StringComparison]::OrdinalIgnoreCase)) { return $false }", "")]},
     ["test_refuses_stale_log_outside_root"]),
    ("abandon-stuck.ps1 without the link check on the program it runs",
     {"abandon-stuck.ps1": [("if (-not (NoLinks $exe)) {", "if ($false) {")]},
     ["test_refuses_build_junction"]),
    ("venv-install.ps1 without the link check on the copy it writes",
     {"venv-install.ps1": [("foreach ($p in $copy, $report, $checkPy) { AssertNoLinks $p }\n", "")]},
     ["test_refuses_scratch_junction"]),
    ("read-b-files.ps1 without the link check",
     {"read-b-files.ps1": [("if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { Refuse \"'$q'", "if ($false) { Refuse \"'$q'")]},
     ["test_refuses_link_to_results"]),
    ("read-b-files.ps1 without the task id rule",
     {"read-b-files.ps1": [("$task -cnotmatch '^t-[0-9]{8}-[0-9]{6}(-[0-9]{1,3})?$'", "$false")]},
     ["test_refuses_any_other_argument"]),
    ("stop-driver.ps1 without the running-entry check",
     {"stop-driver.ps1": [("if ($running.Count -gt 0) {", "if ($false) {")]},
     ["test_refuses_while_an_entry_is_running"]),
    ("stop-driver.ps1 without the end anchor on the project name",
     {"stop-driver.ps1": [("+ $pq + '\\s*$'", "+ $pq")]},
     ["test_stops_the_driver_and_nothing_else"]),
    ("stop-driver.ps1 without the this-root check (any -File path counts as ours)",
     {"stop-driver.ps1": [("    $path = if ($Matches[1]) { $Matches[1] } else { $Matches[2] }\n", "    return 'own'\n")]},
     ["test_leaves_a_driver_of_another_root_alone", "test_absolute_script_path_must_be_under_this_root"]),
    ("stop-driver.ps1 stopping the queue-run child too (the round-1 behaviour)",
     {"stop-driver.ps1": [("    if ($kind -eq 'own') { $targets += $p }\n",
                           "    if ($kind -eq 'own') { $targets += $p }\n"
                           "    if ($p.Name -eq 'broadsword.exe' -and $p.CommandLine -match 'queue\\s+run\\s+--all\\s+--project\\s+' + $pq) { $targets += $p }\n")]},
     ["test_never_stops_the_queue_run_child"]),
    ("stop-driver.ps1 killing by pid after the CIM re-read (the round-2 behaviour)",
     {"stop-driver.ps1": [("if (-not [BsProcCwd]::Terminate($h)) {", "if (-not (Stop-Process -Id $id -Force -PassThru)) {")]},
     ["test_no_kill_by_pid"]),
    ("stop-driver.ps1 without the handle's creation-time check",
     {"stop-driver.ps1": [("return Math.Abs(c - when.ToFileTime()) < 10;", "return true;")]},
     ["test_stop_goes_through_a_verified_handle"]),
    ("restart-orch.ps1 without the handoff-just-before-the-line check",
     {"restart-orch.ps1": [("if ($handoffTime -lt $lastTime.AddSeconds(-$handoffGraceSeconds)) {", "if ($false) {")]},
     ["test_refuses_handoff_older_than_last_line", "test_refuses_handoff_over_60_s_before_the_line"]),
    ("restart-orch.ps1 with the round-2 5-minute grace",
     {"restart-orch.ps1": [("$lastTime.AddSeconds(-$handoffGraceSeconds)", "$lastTime.AddMinutes(-5)")]},
     ["test_refuses_handoff_over_60_s_before_the_line"]),
    ("restart-orch.ps1 skipping any line that CONTAINS (tick-loop) (the round-2 behaviour)",
     {"restart-orch.ps1": [("$lines[$i].TrimEnd().EndsWith('(tick-loop)', [StringComparison]::Ordinal)", "$lines[$i].Contains('(tick-loop)')")]},
     ["test_refuses_recent_line_mentioning_tick_loop"]),
    ("restart-orch.ps1 without the 60-minute quiet check",
     {"restart-orch.ps1": [("if (-not (($now - $lastTime).TotalMinutes -gt $quietMinutes)) {", "if ($false) {")]},
     ["test_refuses_recent_last_line"]),
    ("restart-orch.ps1 with the round-1 rule (handoff must be newer than the last line)",
     {"restart-orch.ps1": [("if ($handoffTime -lt $lastTime.AddSeconds(-$handoffGraceSeconds)) {", "if (-not ($handoffTime -gt $lastTime)) {")]},
     ["test_real_handoff_pattern_fires"]),
    ("restart-orch.ps1 without the lock",
     {"restart-orch.ps1": [("[IO.FileMode]::CreateNew", "[IO.FileMode]::OpenOrCreate"), ("[IO.FileOptions]::DeleteOnClose", "[IO.FileOptions]::None")]},
     ["test_refuses_while_another_run_holds_the_lock"]),
    ("restart-orch.ps1 without the 2-hour request check",
     {"restart-orch.ps1": [("if (($now - $reqWritten).TotalHours -lt $requestGapHours) {", "if ($false) {")]},
     ["test_refuses_recent_request"]),
]

CLASSES = (TestVenvInstall, TestVenvInstallReal, TestAbandonStuck, TestReadBFiles, TestStopDriver, TestRestartOrch)


def prove_fail():
    bad = 0
    for label, weaken, must_fail in WEAKENINGS:
        # A weakening whose target text is gone would "fail" the tests in setUp and look like a pass: count it BAD.
        missing = [old for name, pairs in weaken.items() for old, _ in pairs if old not in rd(os.path.join(SCRIPTS, name))]
        if missing:
            print(f"prove-fail BAD {label}: weakening target missing: {missing[0][:120]}")
            bad += 1
            continue
        WEAKEN.clear()
        WEAKEN.update(weaken)
        suite = unittest.TestSuite()
        for cls in CLASSES:
            for name in must_fail:
                if hasattr(cls, name):
                    suite.addTest(cls(name))
        res = unittest.TextTestRunner(stream=open(os.devnull, "w"), verbosity=0).run(suite)
        failed = {t.id().split(".")[-1] for t, _ in res.failures + res.errors}
        first = ""
        for _, tb in res.failures + res.errors:
            lines = [l for l in tb.strip().splitlines() if l.strip()]
            first = lines[-1][:200] if lines else ""
            break
        ok = set(must_fail) <= failed
        print(f"prove-fail {'OK ' if ok else 'BAD'} {label}: failed {sorted(failed)}: {first}")
        bad += 0 if ok else 1
    return bad


if __name__ == "__main__":
    if "--prove-fail" in sys.argv:
        sys.exit(1 if prove_fail() else 0)
    unittest.main(verbosity=2)
