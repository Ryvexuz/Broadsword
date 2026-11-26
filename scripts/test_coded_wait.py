"""Isolated behavioural tests; no live state and no child processes."""

import importlib.util
import io
import os
import pathlib
import tempfile
import unittest


spec = importlib.util.spec_from_file_location(
    "coded_wait", pathlib.Path(__file__).with_name("coded_wait.py")
)
coded_wait = importlib.util.module_from_spec(spec)
spec.loader.exec_module(coded_wait)


class CodedWaitTests(unittest.TestCase):
    task_id = "t-20260101-000000"
    now = 1_800_000_000_000_000_000

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        self.write("broadsword.toml", 'state_dir = "state"\nscratch_dir = "scratch"\n')
        self.task()

    def write(self, name, text, age=20):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(text if isinstance(text, bytes) else text.encode("utf-8"))
        stamp = self.now - age * coded_wait.MINUTE_NS
        os.utime(path, ns=(stamp, stamp))
        return path

    def task(self, status="running"):
        self.write("state/tasks/a.task",
                   f"id = {self.task_id}\nproject = broadsword\nstatus = {status}\n")

    def coder(self, suffix="", age=20, result=True):
        return self.write(f"scratch/codex-{self.task_id}{suffix}.txt",
                          '{"type": "result"}\n' if result else "working\n", age)

    def run_scan(self, count, running=1):
        out, err = io.StringIO(), io.StringIO()
        code = coded_wait.main(
            [str(self.root), "--min-age-min", "15"], out, err, self.now
        )
        self.assertEqual(code, 0, err.getvalue())
        self.assertEqual(err.getvalue(), "")
        lines = out.getvalue().splitlines()
        self.assertEqual(lines[-1], f"coded_wait: {count} of {running} running task(s) "
                         "coded and not verified (min age 15 min)")
        self.assertEqual(len(lines), count + 1)
        return lines

    def test_coded_task_waiting_is_reported(self):
        self.coder()
        self.assertEqual(self.run_scan(1)[0],
                         f"CODED, NOT VERIFIED: {self.task_id} broadsword 20 min")

    def test_subtask_files_do_not_count_for_parent(self):
        self.coder("-4")
        self.write(f"scratch/verify-{self.task_id}-4.md", "report", 0)
        self.run_scan(0)
        self.coder()
        self.run_scan(1)

    def test_round_suffix_files_count(self):
        for suffix in ("-fr1", "-r2", "-a1", ".r0", ".raw", "-r2b"):
            with self.subTest(suffix=suffix):
                path = self.coder(suffix)
                self.run_scan(1)
                path.unlink()

    def test_newer_verify_hides_task(self):
        self.coder()
        for age, count in ((19, 0), (20, 0), (21, 1)):
            with self.subTest(age=age):
                self.write(f"scratch/verify-{self.task_id}-fr1.md", "report", age)
                self.run_scan(count)

    def test_open_entry_hides_task(self):
        self.coder()
        for status in ("queued", "running", "blocked", "stale", "approved",
                       "merged", "rejected", "check_failed"):
            with self.subTest(status=status):
                self.write("state/queue/a.entry", f"task = {self.task_id}\nstatus = {status}\n")
                self.run_scan(0 if status in {
                    "queued", "running", "blocked", "stale", "approved"
                } else 1)

    def test_young_result_not_reported(self):
        self.coder(age=14)
        self.run_scan(0)
        self.coder(age=15)
        self.run_scan(1)

    def test_newest_file_without_result_not_reported(self):
        self.coder("-r1", age=21)
        self.coder("-r2", result=False)
        self.run_scan(0)

    def test_non_running_task_ignored(self):
        self.coder()
        for status in ("open", "done"):
            with self.subTest(status=status):
                self.task(status)
                self.run_scan(0, running=0)

    def test_unreadable_entry_exits_2(self):
        self.coder()
        self.write("state/queue/a.entry", b"\xff")
        out, err = io.StringIO(), io.StringIO()
        code = coded_wait.main(
            [str(self.root), "--min-age-min", "15"], out, err, self.now
        )
        self.assertEqual(code, 2)
        self.assertEqual(out.getvalue(), "")
        self.assertTrue(err.getvalue().startswith("coded_wait:"))
        self.assertEqual(len(err.getvalue().splitlines()), 1)

    def test_invalid_arguments_fail_closed(self):
        for arguments in ([], ["--min-age-min", "0"], ["--min-age-min", "-1"],
                          ["--min-age-min", "NaN"], ["--min-age-min", "1.5"]):
            with self.subTest(arguments=arguments):
                out, err = io.StringIO(), io.StringIO()
                code = coded_wait.main([str(self.root), *arguments], out, err, self.now)
                self.assertEqual(code, 2)
                self.assertEqual(out.getvalue(), "")
                self.assertTrue(err.getvalue().startswith("coded_wait:"))

    def test_missing_optional_folders(self):
        self.run_scan(0)

    def test_invalid_inputs_fail_closed(self):
        settings = self.root / "broadsword.toml"
        for text in ('scratch_dir = "scratch"\n', 'state_dir = "state"\n',
                     'state_dir = "../outside"\nscratch_dir = "scratch"\n',
                     'state_dir = "state"\nscratch_dir = "C:\\\\outside"\n',
                     'not toml', b"\xff"):
            with self.subTest(text=text):
                self.write("broadsword.toml", text)
                out, err = io.StringIO(), io.StringIO()
                self.assertEqual(coded_wait.main(
                    [str(self.root), "--min-age-min", "15"], out, err, self.now
                ), 2)
                self.assertEqual(out.getvalue(), "")
                self.assertTrue(err.getvalue().startswith("coded_wait:"))
        settings.unlink()
        out, err = io.StringIO(), io.StringIO()
        self.assertEqual(coded_wait.main(
            [str(self.root), "--min-age-min", "15"], out, err, self.now
        ), 2)
        self.assertEqual(out.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
