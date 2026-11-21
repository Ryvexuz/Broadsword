"""Tests for scripts/drive.ps1. They only ever run it with --dry-run (or with a bad argument, which is refused
before any step): nothing is built, no window opens. Each test copies drive.ps1 into a temp root's scripts folder, so
the root the script finds from its own path is the temp root and a stray write would show up there.

  python -I -B scripts/test_drive_ps1.py -v
"""
import sys

sys.dont_write_bytecode = True

import pathlib
import re
import shutil
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "drive.ps1"
POWERSHELL = shutil.which("powershell")


def run_script(root, args, cwd):
    cmd = [POWERSHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(root / "scripts" / "drive.ps1")] + args
    return subprocess.run(cmd, cwd=cwd, capture_output=True, text=True, timeout=120)


@unittest.skipIf(POWERSHELL is None, "powershell not found")
class DriveDryRun(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name).resolve() / "root"
        (self.root / "scripts").mkdir(parents=True)
        self.elsewhere = pathlib.Path(self.tmp.name).resolve() / "elsewhere"
        self.elsewhere.mkdir()
        self.assertTrue(SCRIPT.is_file(), "scripts/drive.ps1 is missing")
        shutil.copy(SCRIPT, self.root / "scripts" / "drive.ps1")

    def snapshot(self):
        return sorted(str(p.relative_to(self.root)) for p in self.root.rglob("*"))

    def dry(self, flag="-DryRun"):
        before = self.snapshot()
        r = run_script(self.root, [flag], self.elsewhere)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(r.stderr.strip(), "", "a dry run must print no errors")
        self.assertEqual(self.snapshot(), before, "a dry run must create nothing")
        return r.stdout

    def test_prints_every_command_from_any_folder(self):
        out = self.dry()
        bsw = str(self.root / "build" / "bin" / "broadsword.exe")
        self.assertIn(f"root: {self.root}", out)
        self.assertIn(f"{bsw}\" run crash-sim -- cargo build --offline --release", out)
        self.assertIn(
            f"{bsw}\" run render -- cargo run --offline --release --example drive -- --sim-exe ../../scratch/drive/crash-sim.exe"
            " --body ../crash-sim/scenarios/pad_and_wall.bsn --save-input ../../scratch/drive/drive-",
            out,
        )
        self.assertIn(f"cwd: {self.root / 'projects' / 'render'}", out)
        self.assertIn(f"cwd: {self.root}", out)
        for n in range(1, 6):
            self.assertIn(f"[{n}/5]", out)
        self.assertIn("drive: dry run done", out)
        self.assertNotIn(str(self.elsewhere), out)

    def test_copy_to_scratch_and_timestamped_input(self):
        out = self.dry()
        self.assertIn(
            f"Copy-Item -LiteralPath \"{self.root / 'build' / 'release' / 'crash-sim.exe'}\" -Destination \"{self.root / 'scratch' / 'drive' / 'crash-sim.exe'}\"",
            out,
        )
        stamps = set(re.findall(r"drive-(\d{8}-\d{6})\.input", out))
        self.assertEqual(len(stamps), 1, out)
        self.assertRegex(out, r"saved input \(timestamped\): .*drive-\d{8}-\d{6}\.input")
        self.assertIn("--input ../../scratch/drive/drive-" + stamps.pop() + ".input --drive-report", out)

    def test_double_dash_form_works(self):
        out = self.dry("--dry-run")
        self.assertIn("drive: dry run done", out)

    def test_main_bins_exe_is_used_when_it_exists(self):
        exe = self.root / "build" / "main.bins" / "crash-sim" / "release" / "crash-sim.exe"
        exe.parent.mkdir(parents=True)
        exe.write_bytes(b"")
        out = self.dry()
        self.assertIn("--sim-exe ../../build/main.bins/crash-sim/release/crash-sim.exe", out)
        self.assertNotIn("Copy-Item", out)

    def test_unknown_argument_is_refused_before_any_step(self):
        r = run_script(self.root, ["--wat"], self.elsewhere)
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("drive: FAIL unknown argument", r.stdout)
        self.assertNotIn("[1/5]", r.stdout)

    def test_real_run_without_broadsword_fails_closed(self):
        # No build\bin\broadsword.exe in the temp root: the script must stop before any step, not build anything.
        r = run_script(self.root, [], self.elsewhere)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("drive: FAIL broadsword not found", r.stdout)
        self.assertNotIn("[1/5]", r.stdout)


if __name__ == "__main__":
    unittest.main()
