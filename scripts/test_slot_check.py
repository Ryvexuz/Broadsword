"""Tests for scripts/slot_check.py. Integration tests build a temp root with a broadsword.toml, queue entries
and a fake broadsword CLI, then run slot_check.main. Claim tests exercise the blame helpers directly.

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
from datetime import datetime, timedelta, timezone
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("slot_check", HERE / "slot_check.py")
slot_check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(slot_check)

FAKE = """import sys
root = __import__("pathlib").Path(__file__).resolve().parent
statuses = [sys.argv[sys.argv.index("--status") + 1]] if "--status" in sys.argv else ["running", "open"]
for status in statuses:
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
        self.assertEqual(lines[1:], [
            "holders: t0 blocks 1 (held: a.rs x1)",
            "blocked by own claim: a.rs x1",
        ])
        self.assertIn("ready tasks (claims free) 0", lines[0])

    def test_headroom_flag(self):
        lines = Board(5, [task("t0", "a.rs")], [task("o1", "b.rs")], [], ram=20.0, cpu=10.0).run()
        self.assertTrue(lines[1].startswith("FLAG: large headroom"), lines)

    def test_loop_line_at_three_rejected_entries(self):
        entries = [("t0", "rejected"), ("t0", "check_failed"), ("t0", "rejected")]
        lines = Board(5, [task("t0", "a.rs")], [], entries).run()
        self.assertTrue(any(l.startswith("LOOP: t0 has 3 failed queue rounds") for l in lines), lines)

    def test_no_loop_line_at_two_rejected_entries(self):
        entries = [("t0", "rejected"), ("t0", "rejected")]
        lines = Board(5, [task("t0", "a.rs")], [], entries).run()
        self.assertFalse(any(l.startswith("LOOP") for l in lines), lines)

    def test_missing_binary_fails_loudly(self):
        with tempfile.TemporaryDirectory() as d:
            with self.assertRaises(SystemExit):
                slot_check.broadsword_cmd(pathlib.Path(d))

    def test_blame_preserves_summary_flag_and_loop_lines(self):
        entries = [("h", "queued")] + [("h", "rejected")] * 3
        self.assertEqual(Board(1, [task("h", "a.rs")],
                               [task("blocked", "a.rs"), task("ready", "b.rs")], entries).run(), [
            "slots: board running 1/1, waiting in queue 1, active coders 0, "
            "ready tasks (claims free) 1, RAM 80.0% CPU 80.0%",
            "FLAG: only 0 active coders of cap 1 while 1 ready tasks wait (board full of queued work)",
            "holders: h blocks 1 (held: a.rs x1)",
            "blocked by own claim: a.rs x1",
            "LOOP: h has 3 failed queue rounds: adjudicate, then split or escalate once; no plain retry",
        ])


class HoldConflictTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = pathlib.Path(self.tmp.name)
        self.queue = self.root / "state" / "queue"
        self.queue.mkdir(parents=True)
        self.pauses = self.root / "state" / "pauses.tsv"
        self.now = datetime(2026, 11, 26, 20, 30, tzinfo=timezone.utc)

    def hold(self, name="p", project="p", reason="whole reason"):
        path = self.queue / ("hold-" + name)
        # write_bytes: write_text would turn the reason's \r\n into \r\r\n on Windows.
        path.write_bytes((f"time = 2026-11-26 20:24:40 UTC\nentry = q-1\n"
                          f"project = {project}\nreason = {reason}\n").encode("utf-8"))
        return path

    def test_hold_line_per_hold_file(self):
        self.hold("z", "")
        self.hold("a", "other")
        lines = slot_check.hold_lines(self.root, self.now)
        self.assertEqual([line for line in lines if line.startswith("HOLD:")], [
            "HOLD: other since 2026-11-26T20:24:40Z entry q-1",
            "HOLD: z since 2026-11-26T20:24:40Z entry q-1",
        ])

    def test_unreadable_hold_is_printed_not_skipped(self):
        path = self.hold()
        original = pathlib.Path.read_text

        def read(target, *args, **kwargs):
            if target == path:
                raise PermissionError("denied")
            return original(target, *args, **kwargs)

        with mock.patch.object(pathlib.Path, "read_text", read):
            self.assertEqual(slot_check.hold_lines(self.root, self.now), [
                "HOLD: p since unknown entry unknown (unreadable: denied)"])

    def test_conflict_line_names_files_of_newest_entry_only(self):
        def entry(number, task_id, status, reason):
            (self.queue / f"q-{number}.entry").write_text(
                f"id = q-{number}\ntask = {task_id}\nstatus = {status}\nreason = {reason}\n",
                encoding="utf-8")
        reason = "rebase conflicts in: broadsword.toml, src/a.rs. The rebase was aborted"
        entry("001", "t1", "conflict", reason)
        entry("002", "t1", "queued", "")
        entry("003", "t2", "conflict", reason)
        entry("004", "t3", "conflict", "")
        entry("005", "t4", "conflict", reason)
        self.assertEqual(slot_check.conflict_lines(self.root, [
            ["t1", "open"], ["t2", "failed"], ["t3", "running"], ["t4", "done"]]), [
                "CONFLICT: t2 q-003 broadsword.toml, src/a.rs",
                "CONFLICT: t3 q-004 unknown",
            ])

    def test_unregistered_hold_gets_one_pause_row_across_two_runs(self):
        reason = "long\tvalue\r\ncontinuation " + "x" * 10000
        self.hold(reason=reason)
        original = b"# append-only\nEND\t2026-11-26T20:00:00Z\thold-p\n"
        self.pauses.write_bytes(original)
        first = slot_check.hold_lines(self.root, self.now)
        self.assertIn("HOLD REGISTERED: hold-p until 2026-11-26T21:30:00Z", first)
        after = self.pauses.read_bytes()
        self.assertTrue(after.startswith(original))
        added = after[len(original):].decode("utf-8").splitlines()
        self.assertEqual(len(added), 1)
        fields = added[0].split("\t")
        self.assertGreaterEqual(len(fields), 6)
        self.assertEqual(fields[:5], ["PAUSE", "2026-11-26T20:30:00Z", "queue", "hold-p", ""])
        self.assertEqual(fields[5], "registered by slot_check: long value continuation " + "x" * 10000)
        # Pasted guard format: valid UTC times, default expiry, no subsequent END.
        start = datetime.strptime(fields[1], "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)
        expiry = start + timedelta(minutes=60)
        self.assertLess(self.now, expiry)
        self.assertGreaterEqual(start, datetime(2026, 11, 26, 20, 24, 40, tzinfo=timezone.utc))
        self.assertFalse(any(row.startswith("END\t") for row in added[1:]))
        second = slot_check.hold_lines(self.root, self.now)
        self.assertFalse(any(line.startswith("HOLD REGISTERED:") for line in second))
        self.assertEqual(self.pauses.read_bytes(), after)

    def test_registered_hold_and_missing_pauses_file_write_nothing(self):
        self.hold()
        lines = slot_check.hold_lines(self.root, self.now)
        self.assertFalse(self.pauses.exists())
        self.assertTrue(any(line.startswith("HOLD NOT REGISTERED:") for line in lines))
        original = b"PAUSE\t2026-11-26T20:25:00Z\tMAIN\thold-p\t\talready held\n"
        self.pauses.write_bytes(original)
        lines = slot_check.hold_lines(self.root, self.now)
        self.assertEqual(lines, ["HOLD: p since 2026-11-26T20:24:40Z entry q-1"])
        self.assertEqual(self.pauses.read_bytes(), original)
        for bad in ("PAUSE\tbad\n", "PAUSE\tbad\tMAIN\thold-p\t\treason\n"):
            self.pauses.write_text(bad, encoding="utf-8")
            lines = slot_check.hold_lines(self.root, self.now)
            self.assertTrue(any(line.startswith("HOLD NOT REGISTERED:") for line in lines))
            self.assertEqual(self.pauses.read_text(encoding="utf-8"), bad)


class ClaimBlameTests(unittest.TestCase):
    def blame(self, running, opened):
        return slot_check.blame_lines(slot_check.rows("".join(running)), slot_check.rows("".join(opened)))

    def test_different_keys_do_not_overlap(self):
        a, b = "broadsword.toml#python_tests", "broadsword.toml#max_agents"
        self.assertFalse(slot_check.claims_overlap(a, b))
        self.assertEqual(self.blame([task("h", a)], [task("o", b)]), [])

    def test_whole_file_overlaps_every_key(self):
        for key in ("python_tests", "max_agents"):
            a, b = "broadsword.toml", f"broadsword.toml#{key}"
            with self.subTest(key=key):
                self.assertTrue(slot_check.claims_overlap(a, b))
                self.assertTrue(slot_check.claims_overlap(b, a))
        self.assertEqual(self.blame(
            [task("h", "broadsword.toml#python_tests, broadsword.toml#max_agents")],
            [task("o", "broadsword.toml")]), [
                "holders: h blocks 1 (held: broadsword.toml#max_agents x1)",
                "blocked by own claim: broadsword.toml (whole file) x1",
            ])

    def test_folder_overlap_is_by_segment(self):
        for a, b, expected in [
            ("src", "src/a.rs", True), ("src", "srcx/a.rs", False),
            ("src", "srcx", False), ("SRC", "src/Nested/a.rs", True),
            ("src", "src/a.rs#key", True), ("src/a.rs#key", "src/a.rs/child", False),
        ]:
            with self.subTest(a=a, b=b):
                self.assertEqual(slot_check.claims_overlap(a, b), expected)
                self.assertEqual(slot_check.claims_overlap(b, a), expected)

    def test_own_claim_line_counts_the_blocked_side(self):
        self.assertEqual(self.blame(
            [task("h", "broadsword.toml#python_tests")],
            [task("o1", "broadsword.toml, unrelated.rs"),
             task("o2", "broadsword.toml"),
             task("o3", "broadsword.toml#max_agents")]), [
                "holders: h blocks 2 (held: broadsword.toml#python_tests x2)",
                "blocked by own claim: broadsword.toml (whole file) x2",
            ])

    def test_rankings_top_three_and_distinct_task_counts(self):
        self.assertEqual(self.blame(
            [task("h4", "d"), task("h3", "c"), task("h2", "b"), task("h1", "a, a")],
            [task("o1", "a, a, b, c, d"), task("o2", "a")]), [
                "holders: h1 blocks 2 (held: a x2); h2 blocks 1 (held: b x1); h3 blocks 1 (held: c x1)",
                "blocked by own claim: a x2; b x1; c x1",
            ])

    def test_wildcard_case_and_project_isolation(self):
        self.assertTrue(slot_check.claims_overlap("*", "anything"))
        self.assertTrue(slot_check.claims_overlap("anything", "*"))
        self.assertTrue(slot_check.claims_overlap("BROADsword.toml#MAX_AGENTS", "broadsword.toml#max_agents"))
        self.assertEqual(self.blame([task("h", "*", "other")], [task("o", "a")]), [])
        self.assertEqual(self.blame([], [task("o", "*")]), [])
        self.assertEqual(self.blame([task("h", "*")], []), [])


if __name__ == "__main__":
    unittest.main(verbosity=2)
