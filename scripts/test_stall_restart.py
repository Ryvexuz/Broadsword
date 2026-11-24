"""Isolated counter/call tests; no test launches PowerShell or the real script."""

import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import patch

sys.dont_write_bytecode = True
SPEC = importlib.util.spec_from_file_location(
    "stall_restart", Path(__file__).with_name("stall_restart.py")
)
stall_restart = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(stall_restart)

STALL = "2026-11-24T04:00:00Z ORCH STALL (tick-loop)\n"
ORCH = "2026-11-24T04:00:00Z orchestrator working\n"
SETTINGS = "stall_restart_lines = 3\nstall_restart_timeout_seconds = 60\n"


class StallRestartTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="stall restart ")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "state").mkdir()
        (self.root / "scripts").mkdir()
        self.progress = self.root / "state" / "pc-progress.md"
        self.script = self.root / "scripts" / "restart-orch.ps1"
        self.fake = self.root / "fake restart.ps1"
        # Neither placeholder may execute: subprocess.run is always mocked.
        self.script.write_text("throw 'real script must not run'\n", encoding="utf-8")
        self.fake.write_text("throw 'test uses mocked launch'\n", encoding="utf-8")
        self.settings = self.root / "broadsword.toml"
        self.settings.write_text(SETTINGS, encoding="utf-8")
        self.progress.write_text(ORCH + STALL * 3, encoding="utf-8")
        self.launch = self.enterContext(patch.object(stall_restart.subprocess, "run"))
        self.launch.return_value = subprocess.CompletedProcess([], 0, "written\n", "")

    def invoke(self, dry_run=False):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = stall_restart.run(
                self.root, dry_run, _script_for_test=self.fake
            )
        return code, out.getvalue(), err.getvalue()

    def test_two_stalls_do_not_restart(self):
        self.progress.write_text(ORCH + STALL * 2, encoding="utf-8")
        code, out, _ = self.invoke()
        self.assertEqual(code, 0)
        self.assertEqual(
            out, "stall_restart: 2 ORCH STALL line(s) since the last "
            "orchestrator line; no restart\n",
        )
        self.launch.assert_not_called()
        # Direct lower-level checks, including large history and blank lines.
        self.assertEqual(stall_restart.count_stalls([STALL] * 100000 + [ORCH]), 0)
        self.assertEqual(stall_restart.count_stalls(["\r\n", " \n", STALL]), 1)

    def test_three_stalls_after_the_last_orchestrator_line_restart(self):
        self.progress.write_text(STALL * 4 + ORCH + STALL * 3, encoding="utf-8")
        code, out, _ = self.invoke()
        self.assertEqual(code, 0)
        self.assertIn("3 ORCH STALL lines; restart-orch exit 0: written", out)
        self.launch.assert_called_once_with(
            ["powershell", "-NoProfile", "-File", str(self.fake)],
            capture_output=True, text=True, encoding="utf-8", errors="replace",
            timeout=60, check=False,
        )

    def test_an_orchestrator_line_resets_the_count(self):
        # Counting all history instead would launch and report 5, not 2.
        self.progress.write_text(STALL * 3 + ORCH + STALL * 2, encoding="utf-8")
        code, out, _ = self.invoke()
        self.assertEqual(code, 0)
        self.assertIn("2 ORCH STALL line(s)", out)
        self.launch.assert_not_called()
        for line in (ORCH, "ORCH STALL but not a tick-loop line\n",
                     "mentions (tick-loop) but does not end with it\n"):
            with self.subTest(line=line):
                self.assertEqual(stall_restart.count_stalls([STALL, line]), 0)

    def test_other_tick_loop_lines_do_not_count_or_reset(self):
        # Resetting on AUTO-PUSH/NO-MERGE would suppress this restart.
        self.progress.write_bytes(
            (ORCH + STALL + "AUTO-PUSH (tick-loop)\n" + STALL
             + "NO-MERGE (tick-loop)   \n\n" + STALL.rstrip() + "  \r\n").encode()
            + b"\xff (tick-loop)\n"
        )
        code, out, _ = self.invoke()
        self.assertEqual(code, 0)
        self.assertIn("3 ORCH STALL lines;", out)
        self.launch.assert_called_once()
        self.assertEqual(
            stall_restart.count_stalls(["AUTO-PUSH (tick-loop)\n"] * 3), 0
        )

    def test_refusal_exit_2_gives_exit_3(self):
        self.launch.return_value = subprocess.CompletedProcess(
            [], 2, "stdout ignored\n", "refused: recent request\nsecond line\n"
        )
        code, out, _ = self.invoke()
        self.assertEqual(code, 3)
        self.assertEqual(
            out, "stall_restart: 3 ORCH STALL lines; restart-orch exit 2: "
            "refused: recent request\n",
        )

    def test_success_gives_exit_0(self):
        before = {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()}
        code, out, err = self.invoke()
        self.assertEqual((code, err), (0, ""))
        self.assertIn("restart-orch exit 0: written", out)
        self.assertEqual(
            {p: p.read_bytes() for p in self.root.rglob("*") if p.is_file()}, before
        )
        self.launch.return_value = subprocess.CompletedProcess([], 7, "", "")
        code, out, _ = self.invoke()
        self.assertEqual(code, 1)
        self.assertIn("exit 7: (no output)", out)

    def test_dry_run_runs_nothing(self):
        code, out, _ = self.invoke(dry_run=True)
        self.assertEqual(code, 0)
        self.assertIn(
            subprocess.list2cmdline(
                ["powershell", "-NoProfile", "-File", str(self.fake)]
            ), out,
        )
        self.launch.assert_not_called()
        # The production CLI has no script override.
        with redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            stall_restart.main(["--root", str(self.root), "--script", str(self.fake)])
        self.assertEqual(error.exception.code, 2)

    def test_missing_files_exit_2_before_anything_runs(self):
        for path in (self.progress, self.script, self.settings):
            with self.subTest(path=path):
                content = path.read_bytes()
                path.unlink()
                try:
                    for dry_run in (False, True):
                        code, _, err = self.invoke(dry_run)
                        self.assertEqual(code, 2)
                        self.assertEqual(
                            err, f"stall_restart: missing file {path}\n"
                        )
                        self.launch.assert_not_called()
                finally:
                    path.write_bytes(content)

    def test_missing_or_bad_settings_exit_2(self):
        for key in ("stall_restart_lines", "stall_restart_timeout_seconds"):
            for value in (None, "0", "-1", "true", "1.5", "nan", "inf",
                          '""', '"3"', "[]", "{}"):
                with self.subTest(key=key, value=value):
                    lines = [
                        line for line in SETTINGS.splitlines()
                        if not line.startswith(key + " ")
                    ]
                    if value is not None:
                        lines.append(f"{key} = {value}")
                    self.settings.write_text("\n".join(lines), encoding="utf-8")
                    code, _, err = self.invoke()
                    self.assertEqual(code, 2)
                    self.assertIn(key, err)
                    self.launch.assert_not_called()
        for content in (SETTINGS + "stall_restart_lines = 3\n", "[broken", "\udcff"):
            with self.subTest(content=repr(content)):
                self.settings.write_bytes(content.encode("utf-8", errors="surrogateescape"))
                self.assertEqual(self.invoke()[0], 2)
                self.launch.assert_not_called()
        # Settings are authoritative, not defaults hidden in the mechanism.
        self.settings.write_text(
            "stall_restart_lines = 4\nstall_restart_timeout_seconds = 7\n",
            encoding="utf-8",
        )
        self.assertEqual(self.invoke()[0], 0)
        self.launch.assert_not_called()
        self.progress.write_text(STALL * 4, encoding="utf-8")
        self.assertEqual(self.invoke()[0], 0)
        self.assertEqual(self.launch.call_args.kwargs["timeout"], 7)

    def test_timeout_gives_exit_1(self):
        for error, message in (
            (subprocess.TimeoutExpired("powershell", 60), "timed out"),
            (OSError("launch failed"), "launch error: launch failed"),
            (OverflowError("timeout out of range"), "timeout out of range"),
        ):
            with self.subTest(error=error):
                self.launch.reset_mock()
                self.launch.side_effect = error
                code, _, err = self.invoke()
                self.assertEqual(code, 1)
                self.assertIn(message, err)
                self.launch.assert_called_once()


if __name__ == "__main__":
    unittest.main()
