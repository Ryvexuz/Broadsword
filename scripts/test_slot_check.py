"""Tests for scripts/slot_check.py. Each test builds a temp root with a broadsword.toml, fake queue entries and a
fake broadsword (a small Python script standing in for build/bin/broadsword.exe), and runs slot_check.main on it.

  python -I -B scripts/test_slot_check.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("slot_check", HERE / "slot_check.py")
slot_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(slot_check)

FAKE = """import sys
root = __import__("pathlib").Path(__file__).resolve().parent
status = sys.argv[sys.argv.index("--status") + 1]
sys.stdout.write((root / ("tasks-" + status + ".tsv")).read_text(encoding="utf-8"))
"""


def task(i, claim, project="p"):
    return f"{i}\t{'running'}\tcoder\t{project}\ttitle {i}\t{claim}\n"


class Board:
    def __init__(self, cap, running, open_rows, entries, ram=80.0, cpu=80.0):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = pathlib.Path(self.tmp.name)
        (self.root / "broadsword.toml").write_text(f"max_agents = {cap}\n", encoding="utf-8")
        (self.root / "state" / "queue").mkdir(parents=True)
        for n, (tid, status) in enumerate(entries):
            (self.root / "state" / "queue" / f"q-{n}.entry").write_text(
                f"id = q-{n}\nstatus = {status}\ntask = {tid}\n", encoding="utf-8")
        (self.root / "fake.py").write_text(FAKE, encoding="utf-8")
        (self.root / "tasks-running.tsv").write_text("".join(running), encoding="utf-8")
        (self.root / "tasks-open.tsv").write_text("".join(open_rows), encoding="utf-8")
        self.ram, self.cpu = ram, cpu

    def run(self):
        saved = slot_check.broadsword_cmd, slot_check.machine_load
        slot_check.broadsword_cmd = lambda root: [sys.executable, "-I", str(root / "fake.py")]
        slot_check.machine_load = lambda: (self.ram, self.cpu)
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                slot_check.main(["slot_check.py", str(self.root)])
        finally:
            slot_check.broadsword_cmd, slot_check.machine_load = saved
            self.tmp.cleanup()
        return out.getvalue().splitlines()


class SlotCheckTests(unittest.TestCase):
    def test_active_is_running_minus_open_entries(self):
        running = [task(f"t{i}", f"f{i}.rs") for i in range(4)]
        entries = [("t0", "queued"), ("t1", "running"), ("t2", "merged"), ("t3", "rejected")]
        lines = Board(10, running, [], entries).run()
        self.assertIn("board running 4/10, waiting in queue 2, active coders 2", lines[0])

    def test_blocked_entry_still_counts_as_active(self):
        running = [task("t0", "a.rs"), task("t1", "b.rs")]
        lines = Board(10, running, [], [("t0", "blocked"), ("t1", "approved")]).run()
        self.assertIn("waiting in queue 0, active coders 2", lines[0])

    def test_flag_when_board_full_of_waiting_work_and_ready_tasks(self):
        running = [task(f"t{i}", f"f{i}.rs") for i in range(3)]
        entries = [(f"t{i}", "queued") for i in range(3)]
        lines = Board(3, running, [task("o1", "new.rs")], entries, ram=90.0, cpu=90.0).run()
        self.assertEqual(len(lines), 2, lines)
        self.assertTrue(lines[1].startswith("FLAG: only 0 active coders of cap 3 while 1 ready"), lines)

    def test_no_flag_when_board_full_of_active_coders(self):
        running = [task(f"t{i}", f"f{i}.rs") for i in range(3)]
        lines = Board(3, running, [task("o1", "new.rs")], [], ram=90.0, cpu=90.0).run()
        self.assertEqual(len(lines), 1, lines)
        self.assertIn("active coders 3", lines[0])

    def test_no_flag_without_ready_tasks(self):
        running = [task("t0", "a.rs")]
        lines = Board(3, running, [task("o1", "a.rs")], [("t0", "queued")], ram=90.0, cpu=90.0).run()
        self.assertEqual(len(lines), 1, lines)
        self.assertIn("ready tasks (claims free) 0", lines[0])

    def test_headroom_flag(self):
        lines = Board(5, [task("t0", "a.rs")], [task("o1", "b.rs")], [], ram=20.0, cpu=10.0).run()
        self.assertTrue(lines[1].startswith("FLAG: large headroom"), lines)

    def test_missing_binary_fails_loudly(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                slot_check.broadsword_cmd(pathlib.Path(d))


if __name__ == "__main__":
    unittest.main()
