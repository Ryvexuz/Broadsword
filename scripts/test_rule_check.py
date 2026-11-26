#!/usr/bin/env python3
"""Usage: python -I scripts/test_rule_check.py (isolated fixture roots only)."""
import contextlib
import datetime as dt
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parent))
import rule_check as rc


EXPECTED_IDS = """
a-merge-queue a-worktree-per-agent a-backlog-briefs a-fill-free-slot
a-scope-current-stage a-done-verified a-build-through-broadsword a-stage-audit
a-disk-floor a-ask-first a-retro-ledger a-playbook-synced a-progress-fresh
a-lab-log-fresh a-claude-md a-scratch-cleanup a-worktree-removed a-git-explained
a-claim-before-edit a-env-ignored a-env-untracked a-no-abs-paths a-settings-read
a-fast-mode-off p-yes-before-destructive p-no-early-running p-context-40
p-no-4th-round p-no-retry-blocked p-audit-3h p-template-diff p-approval-per-project
s-batch-review s-tick-alive s-wake-watcher s-heartbeats-clean s-exe-fresh
s-throughput s-main-clean s-auto-push s-gpt-carries-load s-lazlo-waits
s-step-lessons s-greg-dropbox
""".split()


def toml_value(value):
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{k} = {toml_value(v)}" for k, v in value.items()) + " }"
    return json.dumps(value)


class RuleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)

    def write(self, path, text):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")
        return target

    def check(self, kind, status, **values):
        result = rc.run_check(self.root, dict(kind=kind, **values))
        self.assertEqual(result[0], status, result)
        return result[1]

    def registry(self, checks):
        rows = []
        for index, check in enumerate(checks):
            row = dict(id=f"r-{index}", source="fixture", text="fixture rule",
                       owner="reviewer", check=check)
            rows.append("[[rule]]\n" + "\n".join(f"{k} = {toml_value(v)}" for k, v in row.items()))
        return self.write("rules.toml", "\n".join(rows))

    def cli(self, *args):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            code = rc.main(["--root", str(self.root), "--rules",
                            str(self.root / "rules.toml"), *args])
        return code, output.getvalue()

    def git(self, *args):
        result = subprocess.run(["git", "-C", str(self.root), *args],
                                capture_output=True, timeout=10, check=True)
        return result.stdout

    def repo(self):
        self.git("init")
        self.write("code.rs", "fn main() {}\n")
        self.git("add", "--", "code.rs")
        self.git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-m", "fixture")

    def script(self, text):
        self.write("child.py", text)
        return ["{python}", "-I", "child.py"]

    def heartbeat(self, age=0, overdue=False):
        stamp = dt.datetime.now(dt.timezone.utc) - dt.timedelta(minutes=age)
        self.write("state/heartbeats.json", json.dumps({
            "written_utc": stamp.isoformat().replace("+00:00", "Z"),
            "jobs": {"batch_review": {"overdue": overdue, "result": "fixture result"}},
        }))

    def test_none_unchecked(self):
        self.assertEqual(self.check("none", "unchecked", note="no mechanism"), "no mechanism")

    def test_file_age_newest_ok(self):
        old = self.write("logs/old.md", "old")
        os.utime(old, (0, 0))
        self.write("logs/new.md", "new")
        self.check("file_age", "ok", paths=["logs/*.md"], max_hours=1)

    def test_file_age_stale(self):
        path = self.write("old.md", "")
        os.utime(path, (0, 0))
        self.assertIn("h old (max 1)", self.check("file_age", "broken", paths=["*.md"], max_hours=1))

    def test_file_age_missing(self):
        self.assertIn("no file matches", self.check("file_age", "broken", paths=["*.md"], max_hours=1))

    def test_git_clean_ok(self):
        self.repo()
        self.write("untracked.txt", "ignored by this check")
        self.check("git_clean", "ok", repos=["."])

    def test_git_clean_dirty(self):
        self.repo()
        self.write("code.rs", "// changed\n")
        self.assertIn(".: code.rs", self.check("git_clean", "broken", repos=["."]))

    def test_git_clean_missing(self):
        self.assertIn("missing dir", self.check("git_clean", "broken", repos=["missing"]))

    def test_abs_paths_hits_allow_binary(self):
        self.repo()
        # Construct the fixture so the repository itself has no absolute user path.
        absolute = "C:" + chr(92) + "Users" + chr(92) + "fixture"
        self.write("code.rs", "\n".join([absolute] * 6))
        (self.root / "binary.rs").write_bytes(b"\0" + absolute.encode())
        self.git("add", "--", "binary.rs")
        args = dict(pathspecs=["*.rs"], pattern=r"(?i)\b[a-z]:[\\/]+users[\\/]", allow=[])
        detail = self.check("no_abs_paths", "broken", **args)
        self.assertIn("6 hits:", detail)
        self.assertIn("code.rs:5", detail)
        self.assertNotIn("code.rs:6", detail)
        args["allow"] = [f"code.rs:{n}" for n in range(1, 7)]
        self.check("no_abs_paths", "ok", **args)

    def test_tracked_absent(self):
        self.repo()
        self.check("tracked_absent", "ok", names=["forbidden"])
        self.write("sub/forbidden", "fixture")
        self.git("add", "--", "sub/forbidden")
        self.assertIn("sub/forbidden", self.check("tracked_absent", "broken", names=["forbidden"]))

    def test_file_contains(self):
        self.write("config", "literal needle")
        self.check("file_contains", "ok", path="config", text="needle")
        self.check("file_contains", "broken", path="config", text="absent")
        self.check("file_contains", "broken", path="missing", text="needle")

    def test_setting_read(self):
        self.write("broadsword.toml", "max_agents = 2\n")
        self.write("src/nested/lib.rs", "let n = settings.max_agents;")
        self.check("setting_read", "ok", keys=["max_agents"], src="src")
        self.write("src/nested/lib.rs", "let max_agents_extra = 2;")
        detail = self.check("setting_read", "broken", keys=["max_agents", "missing"], src="src")
        self.assertIn("max_agents: read by no .rs file under src", detail)
        self.assertIn("missing: not in broadsword.toml", detail)

    def test_heartbeat_ok(self):
        self.heartbeat()
        self.check("heartbeat", "ok", jobs=["batch_review"], max_json_minutes=30)
        self.check("heartbeat", "ok", jobs=["*"], max_json_minutes=30)

    def test_heartbeat_overdue(self):
        self.heartbeat(overdue=True)
        self.assertIn("batch_review: fixture result", self.check(
            "heartbeat", "broken", jobs=["*"], max_json_minutes=30))

    def test_heartbeat_missing_result_not_yet_run(self):
        for overdue in (False, True):
            self.heartbeat()
            path = self.root / "state/heartbeats.json"
            data = json.loads(path.read_text(encoding="utf-8"))
            data["jobs"] = {"wake_watcher": {"overdue": overdue}}
            self.write("state/heartbeats.json", json.dumps(data))
            for jobs in (["wake_watcher"], ["*"]):
                with self.subTest(overdue=overdue, jobs=jobs):
                    detail = self.check("heartbeat", "broken" if overdue else "ok",
                                        jobs=jobs, max_json_minutes=30)
                    self.assertEqual(detail, "wake_watcher: not yet run" if overdue else "")

    def test_heartbeat_invalid_result_still_broken(self):
        self.heartbeat()
        path = self.root / "state/heartbeats.json"
        data = json.loads(path.read_text(encoding="utf-8"))
        data["jobs"]["batch_review"]["result"] = None
        self.write("state/heartbeats.json", json.dumps(data))
        self.assertIn("invalid heartbeat job: batch_review", self.check(
            "heartbeat", "broken", jobs=["*"], max_json_minutes=30))

    def test_heartbeat_stale_even_without_overdue(self):
        self.heartbeat(age=60)
        self.assertIn("min old", self.check("heartbeat", "broken", jobs=["*"], max_json_minutes=30))

    def test_heartbeat_missing_job_and_json(self):
        self.check("heartbeat", "broken", jobs=["*"], max_json_minutes=30)
        self.heartbeat()
        self.assertIn("missing: absent", self.check(
            "heartbeat", "broken", jobs=["missing"], max_json_minutes=30))

    def test_heartbeat_hostile_json(self):
        self.write("state/heartbeats.json", '{"written_utc":"bad","jobs":{}}')
        self.assertIn("check crashed:", self.check(
            "heartbeat", "broken", jobs=["*"], max_json_minutes=30))
        self.heartbeat()
        data = json.loads((self.root / "state/heartbeats.json").read_text())
        data["jobs"]["batch_review"]["overdue"] = "false"
        self.write("state/heartbeats.json", json.dumps(data))
        self.check("heartbeat", "broken", jobs=["*"], max_json_minutes=30)

    def test_brief_parser_ok_and_missing(self):
        self.write("state/backlog/t-present.md", "brief")
        self.assertEqual(rc.missing_briefs(self.root, "t-present\topen\tTitle\n"), [])
        self.assertEqual(rc.missing_briefs(
            self.root, "t-present\topen\tTitle\n\nt-missing\topen\tOther\n"), ["t-missing"])
        with self.assertRaisesRegex(ValueError, "invalid task id"):
            rc.missing_briefs(self.root, "../escape\topen")

    def test_open_tasks_missing_executable(self):
        self.assertIn("executable missing", self.check("open_tasks_have_briefs", "broken"))

    def test_open_tasks_executable_output(self):
        self.write("fixture.exe", "not executed: child mocked")
        with mock.patch.object(rc, "child", return_value=subprocess.CompletedProcess([], 0, b"t-one\topen\n", b"")):
            self.check("open_tasks_have_briefs", "broken", exe="fixture.exe")
            self.write("state/backlog/t-one.md", "brief")
            self.check("open_tasks_have_briefs", "ok", exe="fixture.exe")
        with mock.patch.object(rc, "child", return_value=subprocess.CompletedProcess([], 3, b"", b"failed")):
            self.assertIn("task list exit 3", self.check("open_tasks_have_briefs", "broken", exe="fixture.exe"))

    def test_disk_floor(self):
        self.write("broadsword.toml", "disk_floor_gb = 5\n")
        with mock.patch.object(rc.shutil, "disk_usage", return_value=shutil_usage(6_000_000_000)):
            self.assertIn("free 6.000 GB, disk_floor_gb 5", self.check("disk_floor", "ok"))
        with mock.patch.object(rc.shutil, "disk_usage", return_value=shutil_usage(4_000_000_000)):
            self.check("disk_floor", "broken")
        self.write("broadsword.toml", "disk_floor_gb = nan\n")
        self.check("disk_floor", "broken")

    def test_cmd_ok(self):
        self.check("cmd", "ok", argv=self.script("print('ok')"))

    def test_cmd_exit_three_is_broken(self):
        detail = self.check("cmd", "broken", argv=self.script("print('reason')\nraise SystemExit(3)"))
        self.assertEqual(detail, "exit 3: reason")

    def test_cmd_ok_exit(self):
        self.check("cmd", "ok", argv=self.script("raise SystemExit(3)"), ok_exit=[3])

    def test_cmd_stdout_reason_preferred_over_stderr_warning(self):
        detail = self.check("cmd", "broken", argv=self.script(
            "import sys\nprint('earlier\\nreason\\n  \\n')\n"
            "print('warn', file=sys.stderr)\nraise SystemExit(3)"))
        self.assertEqual(detail, "exit 3: reason")

    def test_cmd_empty_stdout_uses_stderr(self):
        detail = self.check("cmd", "broken", argv=self.script(
            "import sys\n"
            "print('earlier\\noops\\n\\n', file=sys.stderr)\nraise SystemExit(3)"))
        self.assertEqual(detail, "exit 3: oops")

    def test_cmd_timeout(self):
        self.assertEqual(self.check("cmd", "broken", argv=self.script(
            "import time\ntime.sleep(5)"), timeout_s=1), "timed out after 1 s")

    def test_cmd_inherited_pipe_deadline(self):
        pid_file = self.root / "grandchild.pid"
        grandchild = (
            "import os, pathlib, time; "
            "pathlib.Path('grandchild.pid').write_text(str(os.getpid())); time.sleep(4)")
        argv = self.script(
            "import subprocess, sys\n"
            f"subprocess.Popen([sys.executable, '-I', '-c', {grandchild!r}])\n")
        readers = []
        thread = rc.threading.Thread

        def record_reader(*args, **kwargs):
            reader = thread(*args, **kwargs)
            readers.append(reader)
            return reader

        try:
            with mock.patch.object(rc.threading, "Thread", side_effect=record_reader):
                start = time.monotonic()
                self.assertEqual(self.check("cmd", "broken", argv=argv, timeout_s=1),
                                 "timed out after 1 s")
                # Check definition: inherited pipes must not extend the one-second
                # deadline to the grandchild's four-second lifetime.
                self.assertLess(time.monotonic() - start, 3)
        finally:
            # Wait for EOF from the grandchild even if an assertion fails.
            # This wait is outside the deadline assertion and precedes fixture cleanup.
            for reader in readers:
                reader.join(timeout=10)
            if sys.platform == "win32":
                import ctypes
                from ctypes import wintypes

                kernel = ctypes.WinDLL("kernel32", use_last_error=True)
                kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
                kernel.OpenProcess.restype = wintypes.HANDLE
                kernel.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
                kernel.WaitForSingleObject.restype = wintypes.DWORD
                kernel.CloseHandle.argtypes = [wintypes.HANDLE]
                kernel.CloseHandle.restype = wintypes.BOOL
                pid = int(pid_file.read_text(encoding="utf-8"))
                handle = kernel.OpenProcess(0x00100000, False, pid)  # SYNCHRONIZE
                if handle:
                    try:
                        self.assertEqual(kernel.WaitForSingleObject(handle, 10_000), 0,
                                         "grandchild did not exit before fixture cleanup")
                    finally:
                        if not kernel.CloseHandle(handle):
                            raise ctypes.WinError(ctypes.get_last_error())
                else:
                    error = ctypes.get_last_error()
                    if error != 87:  # ERROR_INVALID_PARAMETER: pid already exited.
                        raise ctypes.WinError(error)
            self.assertEqual(len(readers), 2)
            for reader in readers:
                self.assertFalse(reader.is_alive(), "grandchild still holds an inherited pipe")

    def test_cmd_large_output_tail(self):
        detail = self.check("cmd", "broken", argv=self.script(
            "print('x' * 1_000_000)\nprint('final reason')\nraise SystemExit(3)"))
        self.assertEqual(detail, "exit 3: final reason")

    def test_cmd_error_line_truncation(self):
        detail = self.check("cmd", "broken", argv=self.script(
            "print('x' * 200)\nraise SystemExit(3)"))
        self.assertEqual(detail, "exit 3: " + "x" * 160)

    def test_cmd_missing_script_with_isolation_flag(self):
        self.assertIn("check script missing: missing.py", self.check(
            "cmd", "broken", argv=["{python}", "-I", "missing.py"]))

    def test_cmd_placeholders_cwd_and_closed_stdin(self):
        argv = self.script(
            "import sys, pathlib, datetime\n"
            "assert pathlib.Path.cwd() == pathlib.Path(sys.argv[1])\n"
            "datetime.date.fromisoformat(sys.argv[2])\n"
            "assert sys.stdin.read() == ''\n")
        self.check("cmd", "ok", argv=[*argv, "{root}", "{date_minus_7d}"])

    def test_check_exception_is_broken(self):
        with mock.patch.object(rc, "evaluate", side_effect=RuntimeError("fixture crash")):
            self.assertEqual(rc.run_check(self.root, {"kind": "none", "note": "x"}),
                             ("broken", "check crashed: RuntimeError: fixture crash"))

    def test_validation_rejects_bad_inputs_at_entry(self):
        cases = [
            {"kind": "file_age", "paths": ["../escape"], "max_hours": 1},
            {"kind": "file_age", "paths": ["x"], "max_hours": float("nan")},
            {"kind": "cmd", "argv": [], "timeout_s": 1},
            {"kind": "cmd", "argv": ["x"], "timeout_s": float("inf")},
            {"kind": "cmd", "argv": ["x"], "timeout_s": -1},
            {"kind": "cmd", "argv": ["x\rhidden"]},
            {"kind": "file_contains", "path": "Z:\\outside", "text": "x"},
            {"kind": "no_abs_paths", "pathspecs": ["*"], "pattern": "[", "allow": []},
        ]
        for check in cases:
            with self.subTest(check=check):
                self.assertEqual(rc.run_check(self.root, check)[0], "broken")

    def test_registry_errors_run_no_checks(self):
        path = self.registry([{"kind": "none", "note": "fixture"}])
        original = path.read_text()
        cases = [
            original + "\n" + original,
            original.replace('kind = "none"', 'kind = "unknown"'),
            original.replace('owner = "reviewer"\n', ""),
            original.replace('note = "fixture"', 'unexpected = "fixture"'),
            "not TOML [",
        ]
        for text in cases:
            with self.subTest(text=text):
                path.write_text(text)
                with mock.patch.object(rc, "run_check") as check:
                    code, output = self.cli()
                    self.assertEqual(code, 1)
                    self.assertTrue(output.startswith("rule_check:"))
                    check.assert_not_called()

    def test_summary_exit_and_quiet(self):
        self.write("present", "needle")
        self.registry([{"kind": "file_contains", "path": "present", "text": "needle"},
                       {"kind": "none", "note": "fixture"}])
        code, output = self.cli()
        self.assertEqual(code, 0)
        self.assertIn("RULE UNCHECKED r-1: fixture rule (owner reviewer)\n", output)
        self.assertTrue(output.endswith("rules: 2 total, 1 ok, 0 broken, 1 unchecked\n"))
        self.assertNotIn("RULE UNCHECKED", self.cli("--quiet")[1])
        self.write("present", "wrong")
        code, output = self.cli()
        self.assertEqual(code, 2)
        self.assertIn("RULE BROKEN r-0:", output)
        self.assertTrue(output.endswith("rules: 2 total, 0 ok, 1 broken, 1 unchecked\n"))

    def test_changed_first_second_and_fixed(self):
        self.registry([{"kind": "file_contains", "path": "present", "text": "needle"}])
        state = self.root / "changed.json"
        code, first = self.cli("--changed", str(state))
        self.assertEqual(code, 2)
        self.assertIn("RULE BROKEN r-0:", first)
        self.assertEqual(json.loads(state.read_text()), ["r-0"])
        code, second = self.cli("--changed", str(state))
        self.assertEqual(code, 2)
        self.assertNotIn("RULE BROKEN", second)
        self.assertEqual(second, "rules: 1 total, 0 ok, 1 broken, 0 unchecked\n")
        self.write("present", "needle")
        code, fixed = self.cli("--changed", str(state))
        self.assertEqual(code, 0)
        self.assertIn("RULE FIXED r-0\n", fixed)
        self.assertEqual(json.loads(state.read_text()), [])

    def test_changed_corrupt_state_fails_closed(self):
        self.registry([{"kind": "none", "note": "fixture"}])
        self.write("changed.json", '{"not":"a list"}')
        with mock.patch.object(rc, "run_check") as check:
            self.assertEqual(self.cli("--changed", str(self.root / "changed.json"))[0], 1)
            check.assert_not_called()

    def test_changed_partial_write_preserves_old_state(self):
        self.registry([{"kind": "none", "note": "fixture"}])
        state = self.root / "changed.json"
        state.write_bytes(b'[ "r-old" ]\n')
        old_bytes = state.read_bytes()
        write_text = Path.write_text
        replace = os.replace

        def partial_write(path, text, **kwargs):
            write_text(path, text[:1], **kwargs)
            raise OSError("fixture interrupted write")

        with mock.patch.object(Path, "write_text", autospec=True, side_effect=partial_write):
            with mock.patch.object(rc.os, "replace", wraps=replace) as replacement:
                code, output = self.cli("--changed", str(state))
                replacement.assert_not_called()
        self.assertEqual(code, 1)
        self.assertIn("fixture interrupted write", output)
        self.assertEqual(state.read_bytes(), old_bytes)
        self.assertEqual(state.with_name(state.name + ".tmp").read_bytes(), b"[")
        self.assertEqual(self.cli("--changed", str(state))[0], 0)
        self.assertEqual(state.read_text(encoding="utf-8"), "[]\n")
        self.assertFalse(state.with_name(state.name + ".tmp").exists())

    def test_changed_replace_failure_preserves_old_state(self):
        self.registry([{"kind": "none", "note": "fixture"}])
        state = self.write("changed.json", '[ "r-old" ]\n')
        old_bytes = state.read_bytes()
        with mock.patch.object(rc.os, "replace", side_effect=OSError("fixture replace failure")):
            code, output = self.cli("--changed", str(state))
        self.assertEqual(code, 1)
        self.assertIn("fixture replace failure", output)
        self.assertEqual(state.read_bytes(), old_bytes)

    def test_changed_write_failure_reported_with_summary(self):
        self.registry([{"kind": "none", "note": "fixture"}])
        code, output = self.cli("--changed", str(self.root / "missing" / "state.json"))
        self.assertEqual(code, 1)
        self.assertIn("rule_check:", output)
        self.assertTrue(output.endswith("rules: 1 total, 0 ok, 0 broken, 1 unchecked\n"))

    def test_duplicate_heartbeat_keys_fail_closed(self):
        self.write("state/heartbeats.json",
                   '{"written_utc":"old","written_utc":"new","jobs":{}}')
        self.assertIn("duplicate JSON key", self.check(
            "heartbeat", "broken", jobs=["*"], max_json_minutes=30))

    def test_list_runs_no_checks(self):
        self.registry([{"kind": "none", "note": "fixture"}])
        with mock.patch.object(rc, "run_check") as check:
            self.assertEqual(self.cli("--list"), (0, "r-0\treviewer\tnone\n"))
            check.assert_not_called()

    def test_real_registry_exact_order_and_validation(self):
        path = Path(__file__).resolve().parent.parent / "docs/RULES.toml"
        rules = rc.load_rules(path)
        self.assertEqual([rule["id"] for rule in rules], EXPECTED_IDS)
        self.assertEqual(len(rules), 44)
        checks = {rule["id"]: rule["check"] for rule in rules}
        self.assertEqual(checks["a-retro-ledger"]["timeout_s"], 180)
        allow = checks["a-no-abs-paths"]["allow"]
        self.assertIn("scripts/test_package_race.py:133", allow)
        self.assertIn("scripts/test_package_race.py:143", allow)
        self.assertNotIn("scripts/queue-driver-project.ps1:9", allow)
        for rule in rules:
            rc.validate_check(rule["check"])

    def test_registry_abs_path_exceptions_leave_real_violation_reported(self):
        path = Path(__file__).resolve().parent.parent / "docs/RULES.toml"
        check = next(rule["check"] for rule in rc.load_rules(path)
                     if rule["id"] == "a-no-abs-paths")
        absolute = "C:" + chr(92) + "Users" + chr(92) + "fixture"
        lines = [""] * 143
        lines[132] = lines[142] = absolute
        self.write("scripts/test_package_race.py", "\n".join(lines))
        self.write("scripts/queue-driver-project.ps1", "\n" * 8 + absolute)
        with mock.patch.object(rc, "git", return_value=(
                "scripts/test_package_race.py\0scripts/queue-driver-project.ps1\0")):
            self.assertEqual(rc.run_check(self.root, check),
                             ("broken", "1 hits: scripts/queue-driver-project.ps1:9"))


def shutil_usage(free):
    return type("Usage", (), {"free": free})()


if __name__ == "__main__":
    unittest.main()
