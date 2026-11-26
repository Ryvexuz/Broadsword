#!/usr/bin/env python3
"""Isolated behavioral tests; no test invokes the installed Claude."""

from __future__ import annotations

from contextlib import redirect_stderr, redirect_stdout
from datetime import datetime, timezone
from decimal import Decimal, ROUND_FLOOR, localcontext
import importlib.util
import io
import json
import math
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock


sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "auto_agent", Path(__file__).with_name("auto_agent.py")
)
agent = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(agent)

NOW = datetime(2026, 11, 25, 18, 0, tzinfo=timezone.utc)
# An independent complete permission oracle, intentionally not built from the
# implementation's tool list.
EXPECTED_COMMON = [
    "Read", "Grep", "Glob", "Bash(git log *)", "Bash(git show *)",
    "Bash(git diff *)", "Bash(git -C * log *)", "Bash(git -C * show *)",
    "Bash(build/bin/broadsword.exe queue list*)",
    "Bash(build/bin/broadsword.exe queue show *)",
    "Bash(build/bin/broadsword.exe task list*)",
    "Bash(build/bin/broadsword.exe task show *)",
    "Bash(build/bin/broadsword.exe task check *)",
    "Bash(build/bin/broadsword.exe task add *)",
    "Bash(python -I scratch/slot_check.py *)",
    "Bash(python -I state/staging/exe_staleness.py *)",
    "Bash(python -I state/staging/reviewer-patterns/stats2.py*)",
    "Bash(powershell -NoProfile -File scratch/resource-check.ps1*)",
    "Write(state/backlog/*.md)",
]
EXPECTED_EXTRA = {
    "process_audit": ["Write(state/staging/process-audit/audit-*.md)"],
    "batch_review": [
        "Write(state/staging/batch-review/batch-*.md)",
        "Bash(gh run list *)", "Bash(gh run view *)",
        "Bash(build/bin/broadsword.exe run crash-sim -- cargo test *)",
    ],
}

# The fake's mode is a file in its own temp root, not a production CLI escape.
# A grandchild inherits the output handles, as a real misbehaving tool might.
FAKE = r'''
import json
import os
from pathlib import Path
import subprocess
import sys
import time

root = Path.cwd()
mode = json.loads((root / "fake-mode.json").read_text())
if mode.get("ignore_stdin"):
    (root / "fake-ready").write_text(str(os.getpid()))
    time.sleep(30)
with (root / "invocations.jsonl").open("a", encoding="utf-8") as stream:
    stream.write(json.dumps({"argv": sys.argv[1:], "stdin": sys.stdin.read()}) + "\n")
if mode.get("change"):
    (root / "state/backlog/existing.md").write_text("changed and longer")
    (root / "state/backlog/new.md").write_text("expected new brief")
if mode.get("large_output"):
    sys.stdout.write("x" * mode["large_output"])
    sys.stderr.write("e" * mode["large_output"])
if mode.get("tree"):
    child = subprocess.Popen([
        sys.executable, "-I", "-c",
        "import time; from pathlib import Path; "
        "Path('grandchild-ready').write_text('ready'); "
        "time.sleep(3); Path('grandchild-survived').write_text('alive'); time.sleep(30)"
    ])
    (root / "grandchild.pid").write_text(str(child.pid))
(root / "fake-ready").write_text(str(os.getpid()))
if mode.get("gate"):
    while not (root / "release").exists():
        time.sleep(0.01)
if mode.get("sleep"):
    time.sleep(mode["sleep"])
sys.exit(mode.get("exit", 0))
'''


def expected_argv(job: str) -> list[str]:
    return [
        "claude-test", "-p", "--model", "opus", "--permission-mode", "dontAsk",
        "--permission-prompts", "none", "--setting-sources", "project",
        "--strict-mcp-config", "--no-session-persistence", "--output-format",
        "json", "--allowedTools", *EXPECTED_COMMON, *EXPECTED_EXTRA[job],
    ]


class AutoAgentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="auto-agent-test-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.state = self.root / "state"
        self.state.mkdir()
        (self.state / "backlog").mkdir()
        (self.state / "backlog/existing.md").write_text("original", encoding="utf-8")
        for job in ("process_audit", "batch_review"):
            folder = self.state / "staging" / job.replace("_", "-")
            folder.mkdir(parents=True)
            (folder / "BRIEF.md").write_text(
                f"Full brief for {job}.\nRead-only.\n", encoding="utf-8"
            )
        self.settings()
        self.heartbeats()
        self.fake = self.root / "fake_claude.py"
        self.fake.write_text(FAKE, encoding="utf-8")
        self.mode()

    def settings(self, *, timeout=90, gap=150, cap=6, omit=None):
        values = {
            "auto_agent_timeout_minutes": timeout,
            "auto_agent_min_gap_minutes": gap,
            "auto_agent_max_starts_per_day": cap,
        }
        text = "\n".join(f"{key} = {value}" for key, value in values.items() if key != omit)
        (self.root / "broadsword.toml").write_text(text + "\n", encoding="utf-8")

    def heartbeats(self, *, written="2026-11-25T18:00:00Z", audit=True, batch=False):
        data = {
            "written_utc": written,
            "jobs": {
                "process_audit": {"overdue": audit},
                "batch_review": {"overdue": batch},
            },
        }
        (self.state / "heartbeats.json").write_text(json.dumps(data), encoding="utf-8")

    def mode(self, **values):
        (self.root / "fake-mode.json").write_text(json.dumps(values), encoding="utf-8")

    def inventory(self):
        return {
            str(path.relative_to(self.root)): (path.read_bytes(), path.stat().st_mtime_ns)
            for path in self.root.rglob("*") if path.is_file()
        }

    def checked(self, *, dry=False):
        stdout, stderr = io.StringIO(), io.StringIO()
        with (
            mock.patch.object(agent, "utc_now", return_value=NOW),
            mock.patch.object(agent, "start_detached", return_value=4321) as spawn,
            mock.patch.object(agent, "find_claude", return_value="claude-test"),
            redirect_stdout(stdout), redirect_stderr(stderr),
        ):
            code = agent.check(self.root, dry)
        return code, stdout.getvalue(), stderr.getvalue(), spawn

    def execute(self, job="process_audit", **kwargs):
        return agent.run(
            self.root, job, _claude_for_test=[sys.executable, "-I", str(self.fake)],
            **kwargs,
        )

    def worker(self, *, timeout=15, kind="--worker"):
        proc = subprocess.Popen(
            [
                sys.executable, "-I", "-B", str(Path(__file__).resolve()),
                kind, str(self.root), str(timeout),
            ],
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
        )
        self.addCleanup(self.stop_worker, proc)
        return proc

    def stop_worker(self, proc):
        (self.root / "release").touch()
        (self.root / "reservation-release").touch()
        if proc.poll() is None:
            try:
                proc.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.communicate(timeout=10)
        else:
            proc.communicate(timeout=10)

    def wait_for(self, path: Path, proc=None):
        deadline = time.monotonic() + 10
        while not path.exists():
            if proc is not None and proc.poll() is not None:
                self.fail(f"worker exited early: {proc.communicate(timeout=5)}")
            if time.monotonic() >= deadline:
                self.fail(f"timed out awaiting {path.name}")
            time.sleep(0.01)

    def records(self):
        path = self.root / "invocations.jsonl"
        return [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines()]

    def log(self):
        return (self.state / "pc-progress.md").read_text(encoding="utf-8")

    def test_second_run_refused_while_lock_held(self):
        self.mode(gate=True)
        first = self.worker()
        self.wait_for(self.root / "fake-ready", first)
        second = self.worker(timeout=2)
        out, err = second.communicate(timeout=10)
        self.assertEqual(second.returncode, 0, err)
        self.assertIn("auto_agent: process_audit already running", out)
        self.assertEqual(len(self.records()), 1)
        (self.root / "release").touch()
        _, err = first.communicate(timeout=10)
        self.assertEqual(first.returncode, 0, err)
        with agent.file_lock(agent.job_lock_path(self.root, "process_audit")) as lock:
            self.assertIsNot(lock, False)

    def test_timeout_kills_tree_and_logs_end(self):
        self.mode(tree=True, sleep=30)
        # The fake signals readiness after spawning its grandchild.
        first = self.worker(timeout=2)
        self.wait_for(self.root / "grandchild.pid", first)
        self.wait_for(self.root / "grandchild-ready", first)
        _, err = first.communicate(timeout=15)
        self.assertEqual(first.returncode, 124, err)
        log = self.log()
        self.assertIn("exit 124", log)
        self.assertIn("TIMEOUT killed", log)
        self.assertTrue(all(line.endswith("(tick-loop)") for line in log.splitlines()))
        with agent.file_lock(agent.job_lock_path(self.root, "process_audit")) as lock:
            self.assertIsNot(lock, False)
        # If only the parent is killed, its child writes this marker. Waiting
        # past that independent deadline proves the tree kill behaviorally.
        time.sleep(3.5)
        survived = (self.root / "grandchild-survived").exists()
        if survived:
            # Keep mutation runs isolated too: the parent-only-kill mutation
            # leaves this known sleeping child alive, with its output open.
            pid = int((self.root / "grandchild.pid").read_text())
            if os.name == "nt":
                subprocess.run(
                    ["taskkill", "/T", "/F", "/PID", str(pid)],
                    capture_output=True, timeout=10, check=False,
                )
            else:
                os.kill(pid, signal.SIGKILL)
        self.assertFalse(survived, "grandchild survived the timeout tree kill")

    def test_min_gap_blocks_restart(self):
        self.settings(gap=10)
        last = self.state / "auto-agent-process_audit.last"
        last.write_text("2026-11-25T17:51:00Z\n", encoding="utf-8", newline="\n")
        code, out, _, spawn = self.checked()
        self.assertEqual(code, 0)
        self.assertIn("minimum gap (10 min)", out)
        spawn.assert_not_called()
        self.assertEqual(last.read_text(encoding="utf-8"), "2026-11-25T17:51:00Z\n")
        self.settings(gap=9)
        _, out, _, spawn = self.checked()
        spawn.assert_called_once_with(self.root, "process_audit")
        self.assertIn("started process_audit run pid 4321", out)

    def test_daily_cap_blocks_start(self):
        self.settings(cap=2)
        ledger = self.state / "auto-agent-starts.tsv"
        ledger.write_text(
            "2026-11-25T01:00:00Z\tprocess_audit\n"
            "2026-11-25T02:00:00Z\tprocess_audit\n"
            "2026-11-25T02:00:00Z\tbatch_review\n"
            "2026-11-24T02:00:00Z\tprocess_audit\n", encoding="utf-8", newline="\n",
        )
        _, out, _, spawn = self.checked()
        self.assertIn("daily cap (2)", out)
        spawn.assert_not_called()
        self.settings(cap=3)
        _, _, _, spawn = self.checked()
        spawn.assert_called_once()
        self.assertEqual(len(ledger.read_text(encoding="utf-8").splitlines()), 5)

    def test_stale_or_bad_heartbeats_start_nothing(self):
        path = self.state / "heartbeats.json"
        cases = [
            None, "{", "[]",
            '{"written_utc":"2026-11-25T18:00:00Z","jobs":[]}',
            '{"written_utc":"2026-11-25T18:00:00Z","jobs":{"process_audit":true}}',
            '{"written_utc":"2026-11-25T18:00:00Z","jobs":{},"jobs":{}}',
            '{"written_utc":"2026-11-25T18:00:00Z","jobs":{},"x":NaN}',
        ]
        for content in cases:
            with self.subTest(content=content):
                if content is None:
                    path.unlink(missing_ok=True)
                else:
                    path.write_text(content, encoding="utf-8")
                before = self.inventory()
                code, out, _, spawn = self.checked()
                self.assertEqual(code, 0)
                self.assertIn("auto_agent: heartbeats unreadable or stale", out)
                spawn.assert_not_called()
                self.assertEqual(self.inventory(), before)
        for written in (
            "2026-11-25T17:29:59Z", "2026-11-25T18:00:01Z", "",
            "2026-11-25T18:00:00+00:00", "2026-11-25T18:00:00Z\r",
        ):
            with self.subTest(written=written):
                self.heartbeats(written=written)
                before = self.inventory()
                code, out, _, spawn = self.checked()
                self.assertEqual(code, 0)
                self.assertIn("heartbeats unreadable or stale", out)
                spawn.assert_not_called()
                self.assertEqual(self.inventory(), before)
        for overdue in ("true", 1, None, False, [], {}):
            with self.subTest(overdue=overdue):
                self.heartbeats(audit=overdue)
                before = self.inventory()
                _, _, _, spawn = self.checked()
                spawn.assert_not_called()
                self.assertEqual(self.inventory(), before)
        self.heartbeats()
        data = json.loads(path.read_text(encoding="utf-8"))
        del data["jobs"]["process_audit"]["overdue"]
        path.write_text(json.dumps(data), encoding="utf-8")
        before = self.inventory()
        _, _, _, spawn = self.checked()
        spawn.assert_not_called()
        self.assertEqual(self.inventory(), before)
        self.heartbeats(written="2026-11-25T17:30:00Z")
        _, _, _, spawn = self.checked()
        spawn.assert_called_once()

    def test_missing_setting_refuses_before_write(self):
        for key in (
            "auto_agent_timeout_minutes", "auto_agent_min_gap_minutes",
            "auto_agent_max_starts_per_day",
        ):
            with self.subTest(key=key):
                self.settings(omit=key)
                before = self.inventory()
                code, _, err, spawn = self.checked()
                self.assertEqual(code, 2)
                self.assertIn(key, err)
                spawn.assert_not_called()
                errors = io.StringIO()
                with redirect_stderr(errors):
                    self.assertEqual(self.execute(), 2)
                self.assertIn(key, errors.getvalue())
                self.assertEqual(self.inventory(), before)

    def test_invalid_settings_refuse_before_write(self):
        for field in ("timeout", "gap", "cap"):
            for bad in (0, -1, "true", '"1"', "1.5", "nan", "inf"):
                with self.subTest(field=field, bad=bad):
                    self.settings(**{field: bad})
                    before = self.inventory()
                    code, _, err, spawn = self.checked()
                    self.assertEqual(code, 2)
                    self.assertIn("auto_agent_", err)
                    spawn.assert_not_called()
                    self.assertEqual(self.inventory(), before)

    def test_argv_is_exact_per_job(self):
        for job in ("process_audit", "batch_review"):
            with self.subTest(job=job):
                self.assertEqual(agent.claude_argv("claude-test", job), expected_argv(job))
                self.assertEqual(self.execute(job), 0)
                record = self.records()[-1]
                self.assertEqual(record["argv"], expected_argv(job)[1:])
                brief_path = self.state / "staging" / job.replace("_", "-") / "BRIEF.md"
                brief = brief_path.read_text(encoding="utf-8")
                patterns = ", ".join(
                    tool for tool in EXPECTED_COMMON + EXPECTED_EXTRA[job]
                    if tool.startswith("Bash(")
                )
                expected = (
                    "You were started by the Broadsword tick loop (scripts/auto_agent.py) "
                    f"because state/heartbeats.json marks {job} overdue. Root: {self.root}. "
                    "You run headless: nobody answers questions, and any tool outside your "
                    "allowed list is denied. Use exactly these command forms (forward slashes): "
                    f"{patterns}. Follow the brief below; finish by writing your output file."
                    f"\n\n{brief}"
                )
                self.assertEqual(record["stdin"], expected)

    def test_log_lines_end_with_tick_loop_and_name_changed_briefs(self):
        self.mode(change=True, exit=7)
        self.assertEqual(self.execute(), 7)
        lines = self.log().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertTrue(all(line.endswith("(tick-loop)") for line in lines))
        self.assertIn("AUTO-AGENT start process_audit: claude -p opus", lines[0])
        self.assertIn("timeout 90 min", lines[0])
        self.assertIn("exit 7", lines[1])
        self.assertIn("changed existing briefs: existing.md (tick-loop)", lines[1])
        self.assertNotIn("new.md", lines[1])
        self.assertIn("output scratch/auto-agent/process_audit-", lines[1])

    def test_dry_run_prints_both_commands_and_writes_nothing(self):
        self.heartbeats(batch=True)
        before = self.inventory()
        code, out, _, spawn = self.checked(dry=True)
        self.assertEqual(code, 0)
        spawn.assert_not_called()
        for job in ("process_audit", "batch_review"):
            self.assertIn(subprocess.list2cmdline(agent.start_command(self.root, job)), out)
            self.assertIn(subprocess.list2cmdline(expected_argv(job)), out)
        self.assertEqual(self.inventory(), before)

    def test_check_starts_each_job_once_and_reserves_before_launch(self):
        self.heartbeats(batch=True)

        def launch(root, job):
            self.assertTrue((self.state / f"auto-agent-{job}.last").is_file())
            self.assertIn(
                f"2026-11-25T18:00:00.000000Z\t{job}\n",
                (self.state / "auto-agent-starts.tsv").read_text(encoding="utf-8"),
            )
            return 4321

        with (
            mock.patch.object(agent, "utc_now", return_value=NOW),
            mock.patch.object(agent, "start_detached", side_effect=launch) as spawn,
        ):
            self.assertEqual(agent.check(self.root), 0)
            self.assertEqual(
                spawn.call_args_list,
                [mock.call(self.root, "process_audit"), mock.call(self.root, "batch_review")],
            )
        _, out, _, spawn = self.checked()
        spawn.assert_not_called()
        self.assertEqual(out.count("minimum gap"), 2)

    def test_check_skips_held_job_even_in_dry_run(self):
        # flock and Windows byte locks are process-independent, so use a worker
        # to own the job lock while the parent probes it.
        self.mode(gate=True)
        first = self.worker()
        self.wait_for(self.root / "fake-ready", first)
        for dry in (False, True):
            _, out, _, spawn = self.checked(dry=dry)
            spawn.assert_not_called()
            self.assertIn("already running", out)
        (self.root / "release").touch()
        first.communicate(timeout=10)
        self.assertEqual(first.returncode, 0)

    def test_concurrent_ticks_serialize_the_shared_ledger(self):
        first = self.worker(kind="--reservation-worker")
        self.wait_for(self.root / "reservation-ready", first)
        second = self.worker(kind="--check-worker")
        out, err = second.communicate(timeout=10)
        self.assertEqual(second.returncode, 0, err)
        self.assertIn("starts ledger locked", out)
        self.assertFalse((self.root / "test-starts").exists())
        (self.root / "reservation-release").touch()
        out, err = first.communicate(timeout=10)
        self.assertEqual(first.returncode, 0, err)
        self.assertIn("started process_audit", out)
        self.assertEqual(
            (self.root / "test-starts").read_text(encoding="utf-8").splitlines(),
            ["process_audit"],
        )

    def test_large_inputs_do_not_bypass_the_cap_or_json_validation(self):
        ledger = self.state / "auto-agent-starts.tsv"
        ledger.write_text(
            "2026-11-25T01:00:00Z\tprocess_audit\n" * 10000,
            encoding="utf-8", newline="\n",
        )
        _, out, _, spawn = self.checked()
        spawn.assert_not_called()
        self.assertIn("daily cap (6)", out)
        ledger.unlink()
        path = self.state / "heartbeats.json"
        path.write_text(
            '{"padding":"' + "x" * 1000000 + '",'
            '"written_utc":"2026-11-25T18:00:00Z",'
            '"jobs":{"process_audit":{"overdue":true,"overdue":false}}}',
            encoding="utf-8",
        )
        before = self.inventory()
        _, out, _, spawn = self.checked()
        spawn.assert_not_called()
        self.assertIn("duplicate JSON key", out)
        self.assertEqual(self.inventory(), before)

    def test_failed_spawn_consumes_reservation_and_does_not_loop(self):
        with (
            mock.patch.object(agent, "utc_now", return_value=NOW),
            mock.patch.object(agent, "start_detached", side_effect=OSError("launch denied")),
            redirect_stdout(io.StringIO()) as out,
        ):
            self.assertEqual(agent.check(self.root), 0)
        self.assertIn("launch denied", out.getvalue())
        _, out, _, spawn = self.checked()
        spawn.assert_not_called()
        self.assertIn("minimum gap", out)

    def test_bad_ledger_or_last_start_fails_closed(self):
        ledger = self.state / "auto-agent-starts.tsv"
        for content in (
            "garbage\n", "2026-11-25T12:00:00Z\tother\n",
            "2026-11-26T12:00:00Z\tprocess_audit\n",
            "2026-11-25T12:00:00Z\tprocess_audit\textra\n",
        ):
            with self.subTest(content=content):
                ledger.write_text(content, encoding="utf-8")
                _, out, _, spawn = self.checked()
                spawn.assert_not_called()
                self.assertIn("skipped process_audit", out)
        ledger.unlink()
        last = self.state / "auto-agent-process_audit.last"
        for value in ("", "bad", "2026-11-25T18:00:01Z", "2026-11-25T12:00:00Z\r\n"):
            with self.subTest(value=value):
                last.write_bytes(value.encode("utf-8"))
                _, out, _, spawn = self.checked()
                spawn.assert_not_called()
                self.assertIn("skipped process_audit", out)

    def test_missing_brief_and_missing_claude_are_logged_refusals(self):
        brief = self.state / "staging/process-audit/BRIEF.md"
        brief.unlink()
        self.assertEqual(self.execute(), 2)
        self.assertIn("brief missing", self.log())
        self.assertFalse((self.root / "invocations.jsonl").exists())
        brief.write_text("restored", encoding="utf-8")
        with mock.patch.object(agent, "find_claude", return_value=None):
            self.assertEqual(agent.run(self.root, "process_audit"), 2)
        self.assertIn("claude not found", self.log())
        self.assertTrue(all(line.endswith("(tick-loop)") for line in self.log().splitlines()))

    def test_spawn_failure_logs_end_and_releases_lock(self):
        with (
            mock.patch.object(agent.subprocess, "Popen", side_effect=OSError("spawn denied")),
            redirect_stderr(io.StringIO()) as errors,
        ):
            self.assertEqual(self.execute(), 2)
        self.assertIn("spawn denied", errors.getvalue())
        lines = self.log().splitlines()
        self.assertEqual(len(lines), 2)
        self.assertIn("exit 2", lines[1])
        self.assertIn("spawn denied", lines[1])
        self.assertIn("changed existing briefs: none (tick-loop)", lines[1])
        self.assertTrue(all(line.endswith("(tick-loop)") for line in lines))
        outputs = list((self.root / "scratch/auto-agent").iterdir())
        self.assertEqual(len(outputs), 2)
        self.assertTrue(all(path.stat().st_size == 0 for path in outputs))
        self.assertFalse((self.root / "invocations.jsonl").exists())
        with agent.file_lock(agent.job_lock_path(self.root, "process_audit")) as lock:
            self.assertIsNot(lock, False)

    def test_snapshot_detects_mtime_only_changes_and_ignores_new_briefs(self):
        existing = self.state / "backlog/existing.md"
        before = agent.snapshot(self.root)
        info = existing.stat()
        os.utime(existing, ns=(info.st_atime_ns, info.st_mtime_ns + 2_000_000_000))
        (self.state / "backlog/expected-new.md").write_text("new", encoding="utf-8")
        self.assertEqual(agent.changed_briefs(self.root, before), ["existing.md"])

    def test_find_claude_portable_fallback(self):
        with (
            mock.patch.object(agent.shutil, "which", return_value="from-path"),
            mock.patch.object(agent.Path, "home", return_value=self.root),
        ):
            self.assertEqual(agent.find_claude(), "from-path")
        fallback = self.root / ".local/bin/claude.exe"
        fallback.parent.mkdir(parents=True)
        with (
            mock.patch.object(agent.shutil, "which", return_value=None),
            mock.patch.object(agent.Path, "home", return_value=self.root),
        ):
            self.assertIsNone(agent.find_claude())
            fallback.touch()
            self.assertEqual(agent.find_claude(), str(fallback))

    def test_shortening_override_rejects_invalid_or_longer_timeout_before_write(self):
        for value in (0, -1, math.nan, math.inf, -math.inf, True, "1", 5401):
            with self.subTest(value=value):
                before = self.inventory()
                with redirect_stderr(io.StringIO()):
                    self.assertEqual(self.execute(_timeout_seconds_for_test=value), 2)
                self.assertEqual(self.inventory(), before)

    def test_huge_integer_settings_do_not_overflow(self):
        self.settings(timeout=10**100, gap=10**100, cap=10**100)
        last = self.state / "auto-agent-process_audit.last"
        last.write_text("2026-11-24T18:00:00Z\n", encoding="utf-8", newline="\n")
        _, out, _, spawn = self.checked()
        spawn.assert_not_called()
        self.assertIn("minimum gap", out)
        self.assertEqual(self.execute(), 0)

    def test_float_timeout_conversion_uses_exact_nanosecond_budget(self):
        values = (
            math.ulp(0.0), sys.float_info.min, sys.float_info.max,
            math.nextafter(1.0, 0.0), 1.0, math.nextafter(1.0, math.inf),
        )
        with localcontext() as context:
            context.prec = 2000
            for value in values:
                with self.subTest(value=value):
                    reference = int(
                        (Decimal.from_float(value) * Decimal(1000000000))
                        .to_integral_value(rounding=ROUND_FLOOR)
                    )
                    self.assertEqual(agent.timeout_nanoseconds(value), reference)

    def test_large_prompt_and_output_do_not_deadlock(self):
        brief = self.state / "staging/process-audit/BRIEF.md"
        brief.write_text("brief\n" * 200000, encoding="utf-8")
        self.mode(large_output=1000000)
        self.assertEqual(self.execute(_timeout_seconds_for_test=10), 0)
        self.assertTrue(self.records()[0]["stdin"].endswith("brief\n" * 200000))
        output = list((self.root / "scratch/auto-agent").glob("*.json"))
        self.assertEqual(len(output), 1)
        self.assertEqual(output[0].stat().st_size, 1000000)
        self.assertEqual(output[0].with_suffix(".err").stat().st_size, 1000000)

    def test_child_that_never_reads_large_stdin_still_times_out(self):
        brief = self.state / "staging/process-audit/BRIEF.md"
        brief.write_text("x" * 1000000, encoding="utf-8")
        self.mode(ignore_stdin=True)
        # Run in a bounded worker so a broken Windows stdin timeout fails the
        # test instead of hanging the suite.
        proc = self.worker(timeout=2)
        self.wait_for(self.root / "fake-ready", proc)
        _, err = proc.communicate(timeout=15)
        self.assertEqual(proc.returncode, 124, err)
        self.assertIn("TIMEOUT killed", self.log())

    def test_configured_timeout_reaches_the_wait_boundary(self):
        self.settings(timeout=2)
        with mock.patch.object(agent, "communicate", wraps=agent.communicate) as wait:
            self.assertEqual(self.execute(), 0)
        self.assertEqual(wait.call_args.args[2], 120)

    def test_log_escapes_line_breaks_before_appending_suffix(self):
        agent.progress(self.root, "test\r\nforged\u2028entry")
        lines = self.log().splitlines()
        self.assertEqual(len(lines), 1)
        self.assertTrue(lines[0].endswith("(tick-loop)"))
        self.assertIn(r"test\r\nforged\u2028entry", lines[0])

    def test_detached_spawn_has_exact_command_and_null_streams(self):
        # Mock only Popen: this tests the actual detached spawn boundary without
        # starting a production worker (which could discover installed Claude).
        with mock.patch.object(agent.subprocess, "Popen") as spawn:
            spawn.return_value.pid = 123
            self.assertEqual(agent.start_detached(self.root, "batch_review"), 123)
        args, kwargs = spawn.call_args
        self.assertEqual(args[0], [
            sys.executable, "-I", str(Path(agent.__file__).resolve()),
            "--root", str(self.root), "--run", "batch_review",
        ])
        for stream in ("stdin", "stdout", "stderr"):
            self.assertEqual(kwargs[stream], subprocess.DEVNULL)
        if os.name == "nt":
            self.assertEqual(
                kwargs["creationflags"],
                subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP,
            )
        else:
            self.assertIs(kwargs["start_new_session"], True)

    def test_cli_rejects_unknown_job_and_invalid_mode(self):
        for tail in (["--run", "other"], ["--run", "process_audit", "--dry-run"], []):
            with self.subTest(tail=tail):
                result = subprocess.run(
                    [sys.executable, "-I", "-B", str(Path(agent.__file__)),
                     "--root", str(self.root), *tail],
                    capture_output=True, text=True, timeout=10, check=False,
                )
                self.assertEqual(result.returncode, 2)
                self.assertIn("usage:", result.stderr)

    def test_cli_dry_run_without_heartbeats(self):
        (self.state / "heartbeats.json").unlink()
        before = self.inventory()
        result = subprocess.run(
            [sys.executable, "-I", "-B", str(Path(agent.__file__)),
             "--root", str(self.root), "--check", "--dry-run"],
            capture_output=True, text=True, timeout=10, check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("auto_agent: heartbeats unreadable or stale", result.stdout)
        self.assertEqual(self.inventory(), before)


if __name__ == "__main__":
    if len(sys.argv) > 1 and sys.argv[1] == "--worker":
        root = Path(sys.argv[2])
        sys.exit(agent.run(
            root, "process_audit",
            _claude_for_test=[sys.executable, "-I", str(root / "fake_claude.py")],
            _timeout_seconds_for_test=float(sys.argv[3]),
        ))
    if len(sys.argv) > 1 and sys.argv[1] in ("--reservation-worker", "--check-worker"):
        root = Path(sys.argv[2])
        pause = sys.argv[1] == "--reservation-worker"
        original_gate = agent.gate_reason

        def gated_reason(*args):
            reason = original_gate(*args)
            if pause and reason is None:
                (root / "reservation-ready").touch()
                deadline = time.monotonic() + 15
                while not (root / "reservation-release").exists():
                    if time.monotonic() >= deadline:
                        raise OSError("test reservation gate timed out")
                    time.sleep(0.01)
            return reason

        def record_start(_root, job):
            with (root / "test-starts").open("a", encoding="utf-8") as stream:
                stream.write(job + "\n")
            return os.getpid()

        with (
            mock.patch.object(agent, "utc_now", return_value=NOW),
            mock.patch.object(agent, "gate_reason", side_effect=gated_reason),
            mock.patch.object(agent, "start_detached", side_effect=record_start),
        ):
            sys.exit(agent.check(root))
    unittest.main()
