"""Scratch-only lane dry-run tests; fake Cargo is first on PATH.

Run through Broadsword: python -I scripts/test_lane_dryrun.py
No real Cargo or Broadsword subprocess is used by these tests.
"""
import sys

sys.dont_write_bytecode = True

import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import time
import types
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest import mock


HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("lane_dryrun", HERE / "lane_dryrun.py")
dryrun = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dryrun)


FAKE_CARGO = r'''
import json
import os
from pathlib import Path
import sys

with open(os.environ["FAKE_CARGO_CALLS"], "a", encoding="utf-8") as log:
    log.write(json.dumps({"cwd": os.getcwd(), "args": sys.argv[1:]}) + "\n")
if sys.argv[1:] != ["metadata", "--no-deps", "--format-version", "1", "--offline"]:
    print("unexpected cargo command", file=sys.stderr)
    sys.exit(90)
manifest = Path("Cargo.toml").read_text(encoding="utf-8")
if "# DESCENDANT" in manifest:
    import subprocess
    code = (
        "import os, sys, time\nfrom pathlib import Path\n"
        "Path(os.environ['FAKE_CHILD_READY']).write_text('ready')\n"
        "deadline = time.monotonic() + 20\n"
        "while not Path(os.environ['FAKE_CHILD_RELEASE']).exists() and time.monotonic() < deadline:\n"
        "    time.sleep(0.01)\n"
        "sys.stdout.close()\nsys.stderr.close()\n"
        "Path(os.environ['FAKE_CHILD_DONE']).write_text('done')\n"
    )
    subprocess.Popen([sys.executable, "-I", "-B", "-c", code])
if "# FAIL" in manifest:
    print("first metadata error", file=sys.stderr)
    print("second metadata error", file=sys.stderr)
    sys.exit(7)
if "# BAD_JSON" in manifest:
    print("not JSON")
elif "# NO_PACKAGES" in manifest:
    print("{}")
elif "# WRONG_PACKAGES" in manifest:
    print('{"packages": null}')
elif "# DUPLICATE" in manifest:
    print('{"packages":[],"packages":[{}]}')
elif "# NONFINITE" in manifest:
    print('{"packages":[NaN]}')
elif "# HUGE" in manifest:
    print('{"packages":[{}],"padding":"' + "x" * (2 * 1024 * 1024) + '"}')
elif "# EMPTY" in manifest:
    print('{"packages":[]}')
elif "[package]" in manifest and not Path("src/lib.rs").is_file():
    print("no targets specified in the manifest", file=sys.stderr)
    sys.exit(101)
else:
    print('{"packages":[{"name":"fake"}]}')
'''


class DryRunTests(unittest.TestCase):
    def setUp(self):
        scratch = HERE.parent / "scratch"
        scratch.mkdir(exist_ok=True)
        temporary = tempfile.TemporaryDirectory(prefix="test-lane-dryrun-", dir=scratch)
        self.addCleanup(temporary.cleanup)
        self.tmp = Path(temporary.name).resolve()
        self.root = self.tmp / "root with spaces"
        self.root.mkdir()
        self.write(self.root / "broadsword.toml", '''
projects_dir = "projects"
scratch_dir = "scratch"
build_dir = "build"
state_dir = "state"
worktrees_dir = "worktrees"
install_dir = "build/bin"
done_check_timeout_seconds = 20
mcp_output_max_kb = 1024
''')
        self.project(self.root)
        self.bin = self.tmp / "fake bin"
        self.bin.mkdir()
        fake = self.bin / "fake_cargo.py"
        self.write(fake, FAKE_CARGO)
        if os.name == "nt":
            self.write(self.bin / "cargo.cmd",
                       f'@"{sys.executable}" -I -B "{fake}" %*\n')
        else:
            launcher = self.bin / "cargo"
            # exec keeps the fake's lifetime identical to the child being checked.
            import shlex
            self.write(launcher, "#!/bin/sh\nexec " + shlex.quote(sys.executable)
                       + " -I -B " + shlex.quote(str(fake)) + ' "$@"\n')
            launcher.chmod(0o700)
        self.calls = self.tmp / "calls.jsonl"
        # No real Cargo can be a fallback if the fake launcher is broken.
        env = {"PATH": str(self.bin),
               "FAKE_CARGO_CALLS": str(self.calls)}
        patcher = mock.patch.dict(os.environ, env)
        patcher.start()
        self.addCleanup(patcher.stop)

    def write(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def project(self, path, marker=""):
        manifest = ('[workspace]\nmembers = []\n' if marker == "# EMPTY" else
                    '[package]\nname = "fixture"\nversion = "0.1.0"\n')
        self.write(path / "Cargo.toml", manifest + marker + "\n")
        self.write(path / "src/lib.rs", "pub fn actual_source() {}\n")
        return path

    def lane(self, name, marker=""):
        return self.project(self.root / "projects" / name, marker)

    def go(self, *args):
        stdout, stderr = io.StringIO(), io.StringIO()
        with redirect_stdout(stdout), redirect_stderr(stderr):
            code = dryrun.main(["--root", str(self.root), *map(str, args)])
        return code, stdout.getvalue(), stderr.getvalue()

    def copied(self):
        folders = list((self.root / "scratch").glob("lane-dryrun-*"))
        self.assertEqual(len(folders), 1, folders)
        return folders[0]

    def call_list(self):
        return [json.loads(line) for line in self.calls.read_text(encoding="utf-8").splitlines()]

    def test_one_ok_lane_and_exact_metadata_command(self):
        code, out, err = self.go()
        self.assertEqual((code, err), (0, ""), out)
        self.assertIn("lane broadsword OK non-empty packages\n", out)
        self.assertTrue(out.endswith("lane-dryrun: ok\n"), out)
        calls = self.call_list()
        self.assertEqual(len(calls), 1)
        self.assertEqual(Path(calls[0]["cwd"]), self.copied())
        self.assertEqual(calls[0]["args"],
                         ["metadata", "--no-deps", "--format-version", "1", "--offline"])
        self.assertEqual((self.copied() / "src/lib.rs").read_bytes(),
                         (self.root / "src/lib.rs").read_bytes())

    def test_empty_lane_blocks_and_opt_in_allows_it(self):
        empty = self.lane("empty", "# EMPTY")
        code, out, _ = self.go()
        self.assertEqual(code, 1, out)
        self.assertIn("lane empty EMPTY packages=[]\n", out)
        self.assertTrue(out.endswith("lane-dryrun: BLOCKED empty\n"), out)
        # Use another root scratch setting so no timestamp collision is possible.
        cfg = self.root / "broadsword.toml"
        self.write(cfg, cfg.read_text(encoding="utf-8").replace('"scratch"', '"other-scratch"'))
        code, out, _ = self.go("--empty-ok")
        self.assertEqual(code, 0, out)
        self.assertIn("lane empty EMPTY packages=[]\n", out)
        self.assertTrue(out.endswith("lane-dryrun: ok\n"), out)
        self.assertIn("# EMPTY", (empty / "Cargo.toml").read_text(encoding="utf-8"))

    def test_all_failures_are_reported_and_first_stderr_line_is_kept(self):
        self.lane("broken", "# FAIL")
        self.lane("empty", "# EMPTY")
        self.lane("working")
        code, out, _ = self.go("--empty-ok")
        self.assertEqual(code, 1, out)
        self.assertIn("lane broken FAIL first metadata error\n", out)
        self.assertNotIn("second metadata error", out)
        self.assertIn("lane working OK", out)
        self.assertTrue(out.endswith("lane-dryrun: BLOCKED broken\n"), out)
        self.assertEqual(len(self.call_list()), 4)

    def test_summary_names_every_blocking_lane(self):
        self.lane("a", "# FAIL")
        self.lane("b", "# EMPTY")
        code, out, _ = self.go()
        self.assertEqual(code, 1, out)
        self.assertTrue(out.endswith("lane-dryrun: BLOCKED a b\n"), out)

    def test_missing_supplied_paths_are_bad_input_before_any_copy(self):
        for flag in ("--add-project", "--toml"):
            with self.subTest(flag=flag):
                missing = self.tmp / "missing"
                code, out, err = self.go(flag, missing)
                self.assertEqual(code, 2, out)
                self.assertIn(str(missing), err)
                self.assertFalse((self.root / "scratch").exists())
                self.assertFalse(self.calls.exists())

    def test_missing_root_names_path_and_runs_nothing(self):
        missing = self.tmp / "missing-root"
        err = io.StringIO()
        with redirect_stderr(err):
            code = dryrun.main(["--root", str(missing)])
        self.assertEqual(code, 2)
        self.assertIn(str(missing), err.getvalue())
        self.assertFalse(self.calls.exists())

    def test_timestamp_collision_never_overwrites_retained_evidence(self):
        clock = mock.Mock()
        clock.now.return_value.strftime.return_value = "20261122-044309"
        with mock.patch.object(dryrun.datetime, "datetime", clock):
            code, out, _ = self.go()
            self.assertEqual(code, 0, out)
            saved = self.copied()
            before = {p.relative_to(saved): p.read_bytes()
                      for p in saved.rglob("*") if p.is_file()}
            calls = self.calls.read_bytes()
            code, _, err = self.go()
        self.assertEqual(code, 2, err)
        self.assertIn("lane-dryrun: bad input:", err)
        self.assertEqual(calls, self.calls.read_bytes())
        self.assertEqual(before, {p.relative_to(saved): p.read_bytes()
                                  for p in saved.rglob("*") if p.is_file()})

    def test_only_manifests_locks_and_real_rust_sources_are_copied(self):
        lane = self.lane("nested")
        self.project(lane / "crates/child")
        self.write(lane / "Cargo.lock", "# real lock\n")
        self.write(lane / "ignored.txt", "not source")
        for folder in ("target", ".git", "crates/child/target", "crates/.git"):
            self.write(lane / folder / "Cargo.toml", "must not copy")
            self.write(lane / folder / "artifact.rs", "must not copy")
        self.write(self.root / "worktrees/task/Cargo.toml", "must not copy")
        self.write(self.root / "build/Cargo.toml", "must not copy")
        before = {p.relative_to(lane): p.read_bytes() for p in lane.rglob("*") if p.is_file()}
        code, out, _ = self.go()
        self.assertEqual(code, 0, out)
        copied = self.copied() / "projects/nested"
        self.assertTrue((copied / "crates/child/Cargo.toml").is_file())
        self.assertEqual((copied / "Cargo.lock").read_text(encoding="utf-8"), "# real lock\n")
        self.assertFalse((copied / "ignored.txt").exists())
        self.assertFalse((self.copied() / "worktrees").exists())
        self.assertFalse((self.copied() / "build").exists())
        for folder in ("target", ".git", "crates/child/target", "crates/.git"):
            self.assertFalse((copied / folder).exists(), folder)
        self.assertEqual(before, {p.relative_to(lane): p.read_bytes()
                                  for p in lane.rglob("*") if p.is_file()})

    def test_add_project_and_candidate_projects_dir(self):
        self.lane("old")
        self.project(self.root / "alternate/existing")
        added = self.project(self.tmp / "new-lane", "# EMPTY")
        candidate = self.tmp / "candidate.toml"
        self.write(candidate, '# projects_dir = "decoy"\nprojects_dir = "alternate"\n')
        code, out, _ = self.go("--toml", candidate, "--add-project", added)
        self.assertEqual(code, 1, out)
        self.assertIn("lane existing OK", out)
        self.assertIn("lane new-lane EMPTY", out)
        self.assertNotIn("lane old ", out)
        self.assertTrue((self.copied() / "alternate/new-lane/Cargo.toml").is_file())
        self.assertTrue(out.endswith("lane-dryrun: BLOCKED new-lane\n"), out)

    def test_candidate_without_projects_dir_uses_root_value(self):
        self.lane("existing")
        candidate = self.tmp / "candidate.toml"
        self.write(candidate, '# projects_dir = "not-a-setting"\n[metadata]\npath = "unused"\n')
        code, out, _ = self.go("--toml", candidate)
        self.assertEqual(code, 0, out)
        self.assertIn("lane existing OK", out)

    def test_candidate_relative_path_is_used_with_native_path_semantics(self):
        self.lane("existing")
        candidate = self.tmp / "candidate.toml"
        self.write(candidate, 'projects_dir = "./projects"\n')
        code, out, _ = self.go("--toml", candidate)
        self.assertEqual(code, 0, out)
        self.assertIn("lane existing OK", out)
        self.assertEqual(dryrun.relative_path("nested\\projects", "projects_dir"),
                         Path("nested\\projects"))

    def test_invalid_metadata_never_passes(self):
        for name, marker in (("bad-json", "# BAD_JSON"), ("missing", "# NO_PACKAGES"),
                             ("wrong-type", "# WRONG_PACKAGES"), ("huge", "# HUGE"),
                             ("duplicate", "# DUPLICATE"), ("nonfinite", "# NONFINITE")):
            self.lane(name, marker)
        code, out, _ = self.go()
        self.assertEqual(code, 1, out)
        for name in ("bad-json", "missing", "wrong-type", "huge", "duplicate", "nonfinite"):
            self.assertIn(f"lane {name} FAIL ", out)
        self.assertIn("output exceeds mcp_output_max_kb", out)

    def test_bad_toml_and_unsafe_projects_dir_run_nothing(self):
        candidate = self.tmp / "candidate.toml"
        for text in ('projects_dir = "../escape"\n', 'projects_dir = "C:\\\\escape"\n',
                     'projects_dir = "\\\\escape"\n', 'projects_dir = ""\n',
                     'projects_dir = "a\\nb"\n', 'projects_dir = 1\n',
                     'projects_dir = "a"\nprojects_dir = "b"\n'):
            with self.subTest(text=text):
                self.write(candidate, text)
                code, out, err = self.go("--toml", candidate)
                self.assertEqual(code, 2, out)
                self.assertIn("lane-dryrun: bad input:", err)
                self.assertFalse(self.calls.exists())
                self.assertFalse((self.root / "scratch").exists())

    def test_escaping_manifest_is_a_lane_failure_and_other_lanes_are_checked(self):
        lane = self.lane("escape")
        with (lane / "Cargo.toml").open("a", encoding="utf-8") as manifest:
            manifest.write('[dependencies]\nforeign = {path = "../../../../outside"}\n')
        code, out, _ = self.go()
        self.assertEqual(code, 1, out)
        self.assertIn("lane escape FAIL ", out)
        self.assertIn("Cargo path escapes scratch", out)
        self.assertIn("lane broadsword OK ", out)
        self.assertEqual(len(self.call_list()), 1)

    def test_arbitrary_metadata_and_feature_named_path_are_not_cargo_paths(self):
        manifest = self.root / "Cargo.toml"
        with manifest.open("a", encoding="utf-8") as output:
            output.write('[features]\npath = []\n[package.metadata]\npath = "https://example.invalid"\n')
        code, out, _ = self.go()
        self.assertEqual(code, 0, out)
        self.assertIn("lane broadsword OK ", out)

    def test_copy_failure_midway_does_not_skip_final_judgment(self):
        self.lane("bad")
        self.lane("good")
        original = dryrun.read_plain

        def read(path):
            if path == self.root / "projects/bad/src/lib.rs":
                raise OSError("copy failed halfway")
            return original(path)

        with mock.patch.object(dryrun, "read_plain", side_effect=read):
            code, out, _ = self.go()
        self.assertEqual(code, 1, out)
        self.assertIn("lane bad FAIL copy failed halfway", out)
        self.assertIn("lane good OK", out)
        self.assertTrue(out.endswith("lane-dryrun: BLOCKED bad\n"), out)
        self.assertTrue((self.root / "projects/bad/src/lib.rs").is_file())

    def test_low_level_metadata_handles_missing_cargo(self):
        with mock.patch.object(dryrun.subprocess, "Popen", side_effect=FileNotFoundError("cargo absent")):
            status, detail = dryrun.metadata(self.root, self.tmp / "logs", 1, 1024, self.tmp / "build")
        self.assertEqual(status, "FAIL")
        self.assertIn("cargo absent", detail)

    def test_low_level_metadata_timeout_is_failure(self):
        child = mock.Mock()
        child.wait.side_effect = [subprocess.TimeoutExpired("cargo", 1), 0]
        with mock.patch.object(dryrun.subprocess, "Popen", return_value=child):
            status, detail = dryrun.metadata(self.root, self.tmp / "logs", 1, 1024, self.tmp / "build")
        self.assertEqual((status, detail), ("FAIL", "cargo metadata timed out"))
        child.kill.assert_called_once()

    def test_descendant_holding_output_handles_does_not_hold_up_judgment(self):
        self.project(self.root, "# DESCENDANT")
        ready, release, done = [self.tmp / name for name in ("ready", "release", "done")]

        def wait_for(path):
            deadline = time.monotonic() + 20
            while not path.exists() and time.monotonic() < deadline:
                time.sleep(0.01)
            self.assertTrue(path.exists(), f"fake descendant did not create {path}")

        with mock.patch.dict(os.environ, {"FAKE_CHILD_READY": str(ready),
                                         "FAKE_CHILD_RELEASE": str(release),
                                         "FAKE_CHILD_DONE": str(done)}):
            try:
                code, out, _ = self.go()
                self.assertEqual(code, 0, out)
                wait_for(ready)
                self.assertFalse(done.exists(), "descendant must still be holding output handles")
                self.assertTrue(out.endswith("lane-dryrun: ok\n"), out)
            finally:
                release.touch()
                wait_for(done)

    def test_low_level_copy_rechecks_links_after_initial_check(self):
        destination = self.tmp / "copy"
        source_file = self.root / "src/lib.rs"
        original = Path.lstat
        checks = 0

        def lstat(path, *args, **kwargs):
            nonlocal checks
            if path == source_file:
                checks += 1
                if checks > 1:
                    # Simulate replacement without creating any real links.
                    return types.SimpleNamespace(st_mode=stat.S_IFLNK)
            return original(path, *args, **kwargs)

        with mock.patch.object(Path, "lstat", lstat):
            with self.assertRaisesRegex(dryrun.BadInput, "links are not supported"):
                dryrun.copy_lane(self.root, destination, destination, set())
        self.assertFalse((destination / "src/lib.rs").exists())
        self.assertTrue(source_file.is_file())

    def test_numeric_settings_fail_closed(self):
        for value in (0, -1, True, "", float("nan"), float("inf")):
            with self.subTest(value=value):
                with self.assertRaisesRegex(dryrun.BadInput, "must be a positive integer"):
                    dryrun.positive_integer({"limit": value}, "limit")

    def test_diagnostics_stay_on_one_line_without_mangling_windows_paths(self):
        self.assertEqual(dryrun.one_line("C:\\lane\r\nblocked"), "C:\\lane\\u000d\\u000ablocked")

    def test_cli_returns_real_exit_status_with_fake_cargo(self):
        self.lane("broken", "# FAIL")
        result = subprocess.run(
            [sys.executable, "-I", str(HERE / "lane_dryrun.py"), "--root", str(self.root)],
            capture_output=True, text=True, timeout=20,
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertTrue(result.stdout.endswith("lane-dryrun: BLOCKED broken\n"), result.stdout)


if __name__ == "__main__":
    unittest.main()
