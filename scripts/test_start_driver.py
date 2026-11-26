"""Windows process tests for start-driver.ps1, using isolated roots and lanes.

Run through Broadsword:
  broadsword run broadsword --task t-20261126-063718 -- python -I -B scripts/test_start_driver.py -v
"""

import sys

sys.dont_write_bytecode = True

import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest
import uuid


HERE = pathlib.Path(__file__).resolve().parent
POWERSHELL = shutil.which("powershell")


class StartDriverTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(POWERSHELL, "Windows PowerShell is required")
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name).resolve() / "root"
        (self.root / "scripts").mkdir(parents=True)
        self.script = self.root / "scripts" / "start-driver.ps1"
        shutil.copyfile(HERE / "start-driver.ps1", self.script)
        self.name = "sd-test-" + uuid.uuid4().hex[:8]
        self.cmd = pathlib.Path(os.environ["SystemRoot"]) / "System32" / "cmd.exe"
        self.fake = self.root / "build" / "bin" / "broadsword.exe"
        self.fake.parent.mkdir(parents=True)
        shutil.copyfile(self.cmd, self.fake)

    def stop_process(self, process):
        # Kill only trees rooted at our own Popen pids, before removing the temp root.
        if process.poll() is None:
            result = subprocess.run(
                ["taskkill", "/T", "/F", "/PID", str(process.pid)],
                capture_output=True, text=True, timeout=15,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        process.wait(timeout=15)

    def spawn(self, args):
        process = subprocess.Popen(
            args, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        self.addCleanup(self.stop_process, process)
        # Wait for CIM visibility rather than assuming Popen implies enumeration readiness.
        ready = subprocess.run(
            [
                POWERSHELL, "-NoProfile", "-Command",
                "$ErrorActionPreference = 'Stop'; "
                "for ($i = 0; $i -lt 100; $i++) { "
                f"$p = Get-CimInstance Win32_Process -Filter 'ProcessId={process.pid}'; "
                "if ($p -and $p.ExecutablePath -and $p.CommandLine) { "
                "Write-Output ('ready pid ' + $p.ProcessId); exit 0 }; "
                "Start-Sleep -Milliseconds 100 }; "
                "Write-Error 'process metadata did not become readable'; exit 1",
            ],
            capture_output=True, text=True, timeout=25,
        )
        self.assertEqual(ready.returncode, 0, ready.stdout + ready.stderr)
        self.assertEqual(ready.stdout.strip(), f"ready pid {process.pid}")
        self.assertIsNone(process.poll(), "fixture process exited before the check")
        return process

    def queue(self, name, exe=None, quoted=False):
        args = [
            str(exe or self.fake), "/d", "/c",
            "ping -n 30 127.0.0.1 >nul & rem",
            "queue", "run", "--all", "--project",
        ]
        if quoted:
            # list2cmdline would omit quotes for this name; force actual double quotes.
            return self.spawn(subprocess.list2cmdline(args) + ' "' + name + '"')
        return self.spawn(args + [name])

    def dry(self):
        result = subprocess.run(
            [
                POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass",
                "-File", str(self.script), self.name, "-DryRun",
            ],
            cwd=self.tmp.name, stdin=subprocess.DEVNULL,
            capture_output=True, text=True, timeout=25,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(result.stderr.strip(), "", result.stderr)
        return result.stdout.strip()

    def test_queue_run_of_the_project_under_this_root_counts_as_its_driver(self):
        process = self.queue(self.name)
        output = self.dry()
        self.assertEqual(
            output,
            f"driver for {self.name} already running (queue run pid {process.pid})",
        )
        self.assertNotIn("would start", output)

    def test_queue_run_with_quoted_or_differently_cased_project_counts(self):
        for quoted in (True, False):
            with self.subTest(quoted=quoted):
                process = self.queue(self.name.upper(), quoted=quoted)
                self.assertEqual(
                    self.dry(),
                    f"driver for {self.name} already running (queue run pid {process.pid})",
                )
                self.stop_process(process)

    def test_queue_run_of_another_project_or_a_prefix_does_not_count(self):
        self.queue(self.name + "2")
        self.queue(self.name[:-1])
        self.assertEqual(self.dry(), f"dry run: would start driver for {self.name}")

    def test_queue_run_from_an_exe_outside_this_root_does_not_count(self):
        self.queue(self.name, exe=self.cmd)
        # Also exercise the root-separator boundary with the correct executable name.
        outside = self.root.with_name("root-sibling") / "broadsword.exe"
        outside.parent.mkdir()
        shutil.copyfile(self.cmd, outside)
        self.queue(self.name, exe=outside)
        self.assertEqual(self.dry(), f"dry run: would start driver for {self.name}")

    def test_driver_script_process_counts_from_scripts_or_scratch(self):
        for folder in ("scripts", "scratch"):
            with self.subTest(folder=folder):
                # Whitespace before -Command's closing quote satisfies the kept
                # script-driver pattern's explicit whitespace-or-end boundary.
                process = self.spawn([
                    POWERSHELL, "-NoProfile", "-Command",
                    f"Start-Sleep 30 # {folder}/queue-driver-project.ps1 {self.name} ",
                ])
                self.assertEqual(
                    self.dry(),
                    f"driver for {self.name} already running (pid {process.pid})",
                )
                self.stop_process(process)

    def test_dry_run_with_no_driver_starts_nothing_and_writes_nothing(self):
        def snapshot():
            return sorted(str(p.relative_to(self.root)) for p in self.root.rglob("*"))

        before = snapshot()
        self.assertEqual(self.dry(), f"dry run: would start driver for {self.name}")
        self.assertEqual(snapshot(), before)

    def test_tracked_driver_has_git_bash_path_and_priority_lines(self):
        lines = (HERE / "queue-driver-project.ps1").read_text(encoding="utf-8").splitlines()
        self.assertIn(
            "try { (Get-Process -Id $PID).PriorityClass = 'AboveNormal' } catch {}",
            lines,
        )
        self.assertIn(
            "if (-not (Get-Command bash -ErrorAction SilentlyContinue) "
            "-and (Test-Path 'C:\\Program Files\\Git\\bin\\bash.exe')) "
            "{ $env:Path = 'C:\\Program Files\\Git\\bin;' + $env:Path }",
            lines,
        )

    def test_tracked_driver_refuses_missing_bash_before_queue_run(self):
        lines = (HERE / "queue-driver-project.ps1").read_text(encoding="utf-8").splitlines()
        fallback = next(
            i for i, line in enumerate(lines)
            if line.startswith("if (-not (Get-Command bash ") and "Test-Path" in line
        )
        refusal = [
            "if (-not (Get-Command bash -ErrorAction SilentlyContinue)) {",
            "    [Console]::Error.WriteLine('queue driver: bash not found on PATH "
            "(Git for Windows bin); not starting')",
            "    exit 2",
            "}",
        ]
        self.assertEqual(lines[fallback + 1:fallback + 1 + len(refusal)], refusal)
        queue_runs = [
            i for i, line in enumerate(lines)
            if line.lstrip().startswith("& build/bin/broadsword.exe queue run ")
        ]
        self.assertTrue(queue_runs, "no queue run invocation found")
        self.assertLess(fallback + len(refusal), min(queue_runs))


if __name__ == "__main__":
    unittest.main()
