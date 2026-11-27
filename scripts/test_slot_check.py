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
import stat
import tempfile
import threading
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
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
        self.root = pathlib.Path(self.tmp.name).resolve()
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
        reason = "long\tvalue\r\nproject = still reason\ncontinuation " + "x" * 10000
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
        self.assertEqual(fields[5], "registered by slot_check: long value project = still reason continuation "
                         + "x" * 10000)
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

    def test_concurrent_hold_registration_appends_one_pause_row(self):
        self.hold()
        original = b"# append-only\n"
        self.pauses.write_bytes(original)
        first_read = threading.Event()
        release_first = threading.Event()
        second_progress = threading.Event()
        worker = threading.local()
        attempts = []
        open_path, sleep = pathlib.Path.open, time.sleep
        gate_timeout = 5  # Test harness only: a missed gate is a failure, not a hang.

        def open_with_read_gate(path, *args, **kwargs):
            stream = open_path(path, *args, **kwargs)
            if path != self.pauses or not getattr(worker, "role", None):
                return stream

            def read(*args, **kwargs):
                content = stream.read(*args, **kwargs)
                if worker.role == "first":
                    first_read.set()
                    if not release_first.wait(gate_timeout):
                        raise AssertionError("first appender never released")
                else:
                    attempts.append("read")
                    second_progress.set()
                return content

            guarded = mock.MagicMock(wraps=stream)
            guarded.__enter__.return_value = guarded
            guarded.__exit__.side_effect = stream.__exit__
            guarded.read.side_effect = read
            return guarded

        def observe_lock_retry(seconds):
            if getattr(worker, "role", None) == "second":
                attempts.append("retry")
                second_progress.set()
            sleep(seconds)

        def poll(role, now):
            worker.role = role
            return slot_check.hold_lines(self.root, now)

        with (
            mock.patch.object(pathlib.Path, "open", open_with_read_gate),
            mock.patch.object(time, "sleep", observe_lock_retry),
            ThreadPoolExecutor(max_workers=2) as pool,
        ):
            first = pool.submit(poll, "first", self.now)
            try:
                self.assertTrue(first_read.wait(gate_timeout), "first appender never read pauses")
                # The second poll may carry an earlier clock reading than the first.
                second = pool.submit(poll, "second", self.now - timedelta(seconds=1))
                self.assertTrue(second_progress.wait(gate_timeout), "second appender never attempted registration")
                if attempts[0] == "read":
                    # Without the lock the second poll saw the same old snapshot. Let it
                    # finish appending before the first writes its now-stale decision.
                    second.result(timeout=gate_timeout)
            finally:
                release_first.set()
            first_lines = first.result(timeout=gate_timeout)
            second_lines = second.result(timeout=gate_timeout)

        after = self.pauses.read_bytes()
        self.assertTrue(after.startswith(original))
        added = after[len(original):].splitlines()
        self.assertEqual(len(added), 1, f"concurrent polls appended {len(added)} rows: {added!r}")
        self.assertEqual(added[0].split(b"\t")[:5], [b"PAUSE", b"2026-11-26T20:30:00Z", b"queue", b"hold-p", b""])
        self.assertEqual(attempts[0], "retry")
        self.assertIn("HOLD REGISTERED: hold-p until 2026-11-26T21:30:00Z", first_lines)
        self.assertEqual(second_lines, ["HOLD: p since 2026-11-26T20:24:40Z entry q-1"])

    def test_pauses_lock_timeout_reports_hold_without_writing(self):
        self.hold()
        original = b"# append-only\n"
        self.pauses.write_bytes(original)
        with slot_check._locked_pauses(self.root):
            with mock.patch.object(slot_check, "PAUSES_LOCK_WAIT_SECONDS", 0):
                lines = slot_check.hold_lines(self.root, self.now)
        self.assertEqual(lines[0], "HOLD: p since 2026-11-26T20:24:40Z entry q-1")
        self.assertIn("HOLD NOT REGISTERED: hold-p: timed out waiting for pauses file lock:", lines[1])
        self.assertEqual(self.pauses.read_bytes(), original)
        self.assertIn("HOLD REGISTERED: hold-p until 2026-11-26T21:30:00Z",
                      slot_check.hold_lines(self.root, self.now))

    def test_pauses_path_replaced_by_reparse_point_is_not_appended(self):
        self.hold()
        original = b"# append-only\n"
        self.pauses.write_bytes(original)
        original_stat = self.pauses.lstat()
        lstat = pathlib.Path.lstat
        pauses_stat = slot_check._pauses_stat
        checks = 0

        def simulate_replacement(path, *args, **kwargs):
            if path == self.pauses:
                return mock.Mock(wraps=original_stat, st_file_attributes=stat.FILE_ATTRIBUTE_REPARSE_POINT)
            return lstat(path, *args, **kwargs)

        def check(root):
            nonlocal checks
            checks += 1
            if checks < 3:
                return pauses_stat(root)
            # Simulate the swap just before append without creating a link.
            with mock.patch.object(pathlib.Path, "lstat", simulate_replacement):
                return pauses_stat(root)

        with mock.patch.object(slot_check, "_pauses_stat", side_effect=check):
            lines = slot_check.hold_lines(self.root, self.now)
        self.assertIn("HOLD NOT REGISTERED: hold-p: linked runtime path:", lines[1])
        self.assertEqual(self.pauses.read_bytes(), original)
        self.assertIn("HOLD REGISTERED: hold-p until 2026-11-26T21:30:00Z",
                      slot_check.hold_lines(self.root, self.now))

    def test_partial_append_fails_loudly_and_is_not_retried_on_torn_row(self):
        self.hold()
        original = b"# append-only\n"
        self.pauses.write_bytes(original)
        open_path = pathlib.Path.open

        def open_with_short_write(path, *args, **kwargs):
            stream = open_path(path, *args, **kwargs)
            if path != self.pauses:
                return stream
            guarded = mock.Mock(wraps=stream)
            guarded.write.side_effect = lambda row: stream.write(row[:len(b"PAUSE\t")])
            return guarded

        with mock.patch.object(pathlib.Path, "open", open_with_short_write):
            lines = slot_check.hold_lines(self.root, self.now)
        self.assertIn("HOLD NOT REGISTERED: hold-p: incomplete pauses append", lines)
        self.assertEqual(self.pauses.read_bytes(), original + b"PAUSE\t")
        lines = slot_check.hold_lines(self.root, self.now)
        self.assertTrue(lines[1].startswith("HOLD NOT REGISTERED: hold-p: malformed pauses row:"))
        self.assertEqual(self.pauses.read_bytes(), original + b"PAUSE\t")


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
