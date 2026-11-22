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
import os
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
        self.assertIn("-I -m pip install --isolated --only-binary=:all: -r ", out)
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

    def test_refuses_bad_lines(self):
        bad = ["https://evil.example/pkg.whl\n", "-e .\n", "--index-url https://evil\n", "-r other.txt\n",
               "numpy>=2.0\n", "numpy\n", "numpy[extra]==2.1.0\n", "numpy==2.1.0 ; python_version>'3'\n",
               "numpy==2.1.0\n\nscipy==1.14.1\n", "# comment\nnumpy==2.1.0\n", "numpy==2.1.0 --hash=sha256:00\n",
               "numpy @ https://evil/x.whl\n", "numpy==2.1.0\n--extra-index-url https://evil\n", "numpy==\u00e92\n"]
        for i, text in enumerate(bad):
            rel = self.commit_req(f"requirements-bad{i}.txt", text)
            self.assertRefused(rel, "--dry-run")

    def test_refuses_venv_folder_without_python(self):
        os.makedirs(os.path.join(self.root, "shared", "venv", "Lib"))
        self.assertRefused(self.REQ, "--dry-run")


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

    def spawn(self, args):
        p = subprocess.Popen(["powershell"] + args, cwd=self.root, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.procs.append(p)
        return p

    def driver(self, proj=None):
        return self.spawn(["-NoProfile", "-File", "scratch/queue-driver-project.ps1", proj or self.proj])

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
        self.assertIn("reason: newest handoff", text)
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

    def test_refuses_recent_orchestrator_line(self):
        # also a (tick-loop) line counts for this rule
        self.pc(f"{stamp(180)} fd03 queued", f"{stamp(30)} orchestrator 11 heartbeat (tick-loop)")
        self.assertRefusedNoWrite()
        self.pc(f"{stamp(100)} fd03 queued", f"{stamp(59)} Orchestrator 11 RESUMED (tick-loop)")
        self.assertRefusedNoWrite()

    def test_untimed_orchestrator_line_counts_as_recent(self):
        self.pc(f"{stamp(180)} fd03 queued", "Z orchestrator 11 RESUMED (tick-loop)", f"{stamp(5)} x (tick-loop)")
        self.assertRefusedNoWrite()
        self.pc(f"{stamp(180)} fd03 queued", f"{stamp(170)} y (tick-loop)", "orchestrator 11 RESUMED (tick-loop)")
        self.assertRefusedNoWrite()

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
     {"venv-install.ps1": [("if ($LASTEXITCODE -ne 0) { Refuse \"'$relIn' is modified", "if ($false) { Refuse \"'$relIn' is modified")]},
     ["test_refuses_modified_file"]),
    ("venv-install.ps1 without the name==version rule",
     {"venv-install.ps1": [("-cnotmatch '^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?==[0-9][A-Za-z0-9.!+]*$'", "-cnotmatch '.*'")]},
     ["test_refuses_bad_lines"]),
    ("abandon-stuck.ps1 without the live-pid check",
     {"abandon-stuck.ps1": [("$proc = Get-Process -Id ([int]$pidText) -ErrorAction SilentlyContinue", "$proc = $null")]},
     ["test_refuses_live_fresh_job"]),
    ("abandon-stuck.ps1 without the log-under-root rule",
     {"abandon-stuck.ps1": [("$logFull.StartsWith($root + $sep, [StringComparison]::OrdinalIgnoreCase) -and ", "")]},
     ["test_refuses_stale_log_outside_root"]),
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
    ("restart-orch.ps1 without the handoff-newer check",
     {"restart-orch.ps1": [("if (-not ($handoffTime -gt $lastTime)) {", "if ($false) {")]},
     ["test_refuses_handoff_older_than_last_line"]),
    ("restart-orch.ps1 without the 60-minute orchestrator check",
     {"restart-orch.ps1": [("if (($now - $t).TotalMinutes -lt $quietMinutes) {", "if ($false) {")]},
     ["test_refuses_recent_orchestrator_line", "test_untimed_orchestrator_line_counts_as_recent"]),
    ("restart-orch.ps1 without the 2-hour request check",
     {"restart-orch.ps1": [("if (($now - $reqWritten).TotalHours -lt $requestGapHours) {", "if ($false) {")]},
     ["test_refuses_recent_request"]),
]

CLASSES = (TestVenvInstall, TestAbandonStuck, TestReadBFiles, TestStopDriver, TestRestartOrch)


def prove_fail():
    bad = 0
    for label, weaken, must_fail in WEAKENINGS:
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
