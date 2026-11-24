"""Ten watcher tests with isolated records and deterministic polling races."""

from contextlib import redirect_stderr, redirect_stdout
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


SCRIPT = Path(__file__).with_name("wake_on_event.py")
SPEC = importlib.util.spec_from_file_location("wake_on_event", SCRIPT)
watcher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(watcher)


class WatcherTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.queue = self.root / "state" / "queue"
        self.tasks = self.root / "state" / "tasks"
        self.queue.mkdir(parents=True)
        self.tasks.mkdir()
        self.entry = self.queue / "q.entry"
        self.entry.write_text("status = queued\nstep = review\nlog = old\n",
                              encoding="utf-8")

    def run_poll(self, mutation=None, timeout=2, poll=1):
        """Mutate at the first sleep, strictly after the initial snapshot."""
        now = 0
        mutated = False

        def sleep(seconds):
            nonlocal now, mutated
            now += round(seconds * watcher.NANOSECONDS)
            if mutation is not None and not mutated:
                mutated = True
                mutation()

        output = io.StringIO()
        with patch.object(watcher.time, "monotonic_ns", side_effect=lambda: now), \
                patch.object(watcher.time, "sleep", side_effect=sleep), \
                redirect_stdout(output):
            code = watcher.main([
                "--root", str(self.root), "--timeout-seconds", str(timeout),
                "--poll-seconds", str(poll),
            ])
        return code, output.getvalue(), now

    def assert_event(self, mutation, expected):
        code, output, elapsed = self.run_poll(mutation)
        self.assertEqual(code, 0, output)
        self.assertEqual(output, expected + "\nwoke: 1 change(s)\n")
        self.assertEqual(elapsed, watcher.NANOSECONDS)

    def test_new_entry_wakes(self):
        def add():
            (self.queue / "a.entry").write_text(
                "status = running\nstep = merge\n", encoding="utf-8")
            (self.tasks / "z.task").write_text("status = done\n", encoding="utf-8")
        code, output, _ = self.run_poll(add)
        self.assertEqual(code, 0)
        self.assertEqual(output, "EVENT queue a -/- -> running/merge\n"
                         "EVENT task z - -> done\nwoke: 2 change(s)\n")

    def test_status_change_wakes(self):
        self.assert_event(
            lambda: self.entry.write_text("status = done\nstep = review\n",
                                          encoding="utf-8"),
            "EVENT queue q queued/review -> done/review")

    def test_step_change_wakes(self):
        self.assert_event(
            lambda: self.entry.write_text("status = queued\nstep = merge\n",
                                          encoding="utf-8"),
            "EVENT queue q queued/review -> queued/merge")

    def test_task_status_change_wakes(self):
        task = self.tasks / "t.task"
        task.write_text("status = running\n", encoding="utf-8")
        self.assert_event(
            lambda: task.write_text("status = done\n", encoding="utf-8"),
            "EVENT task t running -> done")

    def test_removed_file_wakes(self):
        self.assert_event(self.entry.unlink, "EVENT queue q queued/review -> -/-")

    def test_no_change_times_out_with_exit_3(self):
        result = subprocess.run(
            [sys.executable, "-I", "-B", str(SCRIPT), "--root", str(self.root),
             "--timeout-seconds", "1", "--poll-seconds", "1"],
            capture_output=True, text=True, timeout=5,
        )
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertEqual(result.stdout, "TIMEOUT after 1 s, no change\n")
        # A poll longer than the timeout must not delay the timeout.
        code, output, elapsed = self.run_poll(timeout=1, poll=15)
        self.assertEqual((code, output, elapsed),
                         (3, "TIMEOUT after 1 s, no change\n", watcher.NANOSECONDS))

    def test_missing_folder_exits_2_before_waiting(self):
        for folder in (self.tasks, self.queue):
            with self.subTest(folder=folder):
                folder.rename(folder.with_name(folder.name + "-hidden"))
                error = io.StringIO()
                with patch.object(watcher.time, "sleep") as sleep, \
                        redirect_stderr(error), self.assertRaises(SystemExit) as exit_:
                    watcher.main(["--root", str(self.root)])
                self.assertEqual(exit_.exception.code, 2)
                self.assertIn("usage:", error.getvalue())
                self.assertIn(str(folder), error.getvalue())
                sleep.assert_not_called()
                folder.with_name(folder.name + "-hidden").rename(folder)

    def test_bad_numbers_exit_2(self):
        for flag in ("--timeout-seconds", "--poll-seconds"):
            for value in ("0", "-1", "1.5", "nan", "inf", "", "1e3", "1\r", " 1"):
                with self.subTest(flag=flag, value=value):
                    error = io.StringIO()
                    with redirect_stderr(error), self.assertRaises(SystemExit) as exit_:
                        watcher.main(["--root", str(self.root), flag + "=" + value])
                    self.assertEqual(exit_.exception.code, 2)
                    self.assertIn("usage:", error.getvalue())
                    self.assertIn(flag, error.getvalue())
        self.assertEqual(watcher.positive_seconds("9" * 100), int("9" * 100))

    def test_unreadable_or_vanishing_file_is_skipped(self):
        real_open = Path.open
        for failure in (PermissionError("unreadable"), FileNotFoundError("vanished")):
            def open_record(path, *args, **kwargs):
                if path == self.entry:
                    raise failure
                return real_open(path, *args, **kwargs)
            with self.subTest(failure=failure), patch.object(Path, "open", open_record):
                self.assertEqual(watcher.snapshot(self.root), {})
        self.entry.write_bytes(b"status = running\nstep = \xff\nunfinished")
        self.assertEqual(watcher.snapshot(self.root)[("q.entry", "queue", "q")],
                         ("running", "\ufffd"))
        # An empty/half-written file is still a present file, not a crash.
        self.entry.write_bytes(b"status")
        self.assertEqual(watcher.snapshot(self.root)[("q.entry", "queue", "q")],
                         ("-", "-"))
        # Listing failure must be a reported failure, not false removal events.
        error = io.StringIO()
        output = io.StringIO()
        with patch.object(Path, "iterdir", side_effect=PermissionError("listing denied")), \
                redirect_stderr(error), redirect_stdout(output):
            self.assertEqual(watcher.main(["--root", str(self.root)]), 1)
        self.assertIn("listing denied", error.getvalue())
        self.assertEqual(output.getvalue(), "")

    def test_other_keys_changing_do_not_wake(self):
        code, output, elapsed = self.run_poll(
            lambda: self.entry.write_text(
                "status = queued\nstep = review\nlog = new" + "x" * (1024 * 1024) + "\n",
                encoding="utf-8"))
        self.assertEqual(code, 3, output)
        self.assertEqual(output, "TIMEOUT after 2 s, no change\n")
        self.assertEqual(elapsed, 2 * watcher.NANOSECONDS)


if __name__ == "__main__":
    unittest.main()
