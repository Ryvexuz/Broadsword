"""Exercise the real reapply CLI with linked worktrees in disposable Git repos."""

from dataclasses import replace
import importlib.util
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch


HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "reapply.py"
with (HERE.parent / "broadsword.toml").open("rb") as stream:
    SETTINGS = tomllib.load(stream)
TIMEOUT = SETTINGS["mcp_call_timeout_seconds"]

STUB = """\
param([string]$Task, [string]$Risk, [string]$Coder, [string]$Brief, [string]$Adjudicated)
$ErrorActionPreference = 'Stop'
$values = @($Task, $Risk, $Coder, $Brief, $Adjudicated, (Get-Location).Path)
[IO.File]::AppendAllText((Join-Path $PSScriptRoot '../qadd.log'), (($values -join "`t") + "`n"))
"stub qadd last line: $Task"
exit 0
"""


def load_script():
    spec = importlib.util.spec_from_file_location("reapply_under_test", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    with patch.object(sys, "dont_write_bytecode", True):
        spec.loader.exec_module(module)
    return module


class ReapplyTests(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(dir=HERE.parent / SETTINGS["scratch_dir"], prefix="ra-")
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name).resolve() / "root with space"
        self.queue = self.root / SETTINGS["state_dir"] / "queue"
        self.queue.mkdir(parents=True)
        self.tsv = self.queue.parent / "reapply.tsv"
        self.qadd = self.root / "scripts" / "qadd.ps1"
        self.qadd.parent.mkdir()
        self.qadd.write_text(STUB, encoding="utf-8")
        self.scratch = self.root / SETTINGS["scratch_dir"]
        (self.scratch / "briefs").mkdir(parents=True)
        self.verdict_dir = self.root / "verdicts"
        self.verdict_dir.mkdir()
        self.log = self.root / "qadd.log"
        keys = ("root", "state_dir", "projects_dir", "worktrees_dir", "scratch_dir",
                "mcp_call_timeout_seconds", "mcp_output_max_kb")
        (self.root / "broadsword.toml").write_text(
            "".join(f"{key} = {SETTINGS[key]!r}\n" for key in keys), encoding="utf-8",
        )
        self.env = {key: val for key, val in os.environ.items() if not key.upper().startswith("GIT_")}
        self.env.update(HOME=temp.name, XDG_CONFIG_HOME=temp.name, PSExecutionPolicyPreference="Bypass")
        self.git(self.root, "init", "-q", "--template=", "-b", "main")
        self.git(self.root, "config", "user.name", "Reapply Test")
        self.git(self.root, "config", "user.email", "reapply@example.invalid")
        self.git(self.root, "config", "commit.gpgSign", "false")
        self.git(self.root, "config", "core.autocrlf", "false")
        self.commit("x.txt", "base\n")

    def git(self, directory, *args):
        proc = subprocess.run(
            ["git", "-C", str(directory), *args], env=self.env, capture_output=True,
            text=True, encoding="utf-8", errors="replace", timeout=TIMEOUT)
        self.assertEqual(proc.returncode, 0, f"fixture git {args}: {proc.stdout}{proc.stderr}")
        return proc.stdout.strip()

    def commit(self, name, contents):
        (self.root / name).write_text(contents, encoding="utf-8")
        self.git(self.root, "add", "--", name)
        self.git(self.root, "commit", "-qm", f"change {name}")
        return self.git(self.root, "rev-parse", "HEAD")

    def entry(self, entry_id, task, status, **fields):
        path = self.queue / f"{entry_id}.entry"
        record = dict(id=entry_id, task=task, project="broadsword", status=status)
        record.update(fields)
        path.write_text("".join(f"{key} = {val}\n" for key, val in record.items()), "utf-8")
        return path

    def row(self, n, *, conflict=False, train=False, custom_brief=False):
        task = f"t-20261126-205950-{n}"
        entry_id = f"q-20261126-205950-{n}"
        pre = self.git(self.root, "rev-parse", "HEAD")
        neighbor = self.commit("neighbor.txt", "other task\n") if train else ""
        path = f"{task}.txt"
        first = self.commit(path, "first task commit\n")
        tip = self.commit("x.txt" if conflict else path, "second task commit\n")
        wt = self.root / SETTINGS["worktrees_dir"] / "broadsword" / task
        wt.parent.mkdir(parents=True, exist_ok=True)
        self.git(self.root, "worktree", "add", "-q", "-b", f"task/{task}", str(wt), tip)
        picks = [tip, first, *([neighbor] if neighbor else [])]
        self.git(self.root, "revert", "--no-commit", *picks)
        self.git(self.root, "commit", "-qm", "automatic revert")
        revert = self.git(self.root, "rev-parse", "HEAD")
        if conflict:
            self.commit("x.txt", "later main edit\n")
        self.entry(entry_id, task, "merged_check_failed", branch=f"task/{task}",
                   train_up=neighbor, pre_merge_sha=pre, main_sha=tip, revert_sha=revert)
        verdict = self.verdict_dir / f"{task} verdict.md"
        verdict.write_text("Judged innocent\n", "utf-8")
        brief = self.scratch / "briefs" / ("custom brief.md" if custom_brief else f"{task}.md")
        brief.write_text("Task brief\n", "utf-8")
        row = "\t".join((task, entry_id, str(verdict.relative_to(self.root)), "high", "coder-codex",
                         str(brief.relative_to(self.root)) if custom_brief else ""))
        return task, wt, row, verdict, brief

    def rows(self, *rows):
        self.tsv.write_bytes(("\r\n".join(rows) + "\r\n").encode("utf-8"))

    def run_cli(self, code=0, *, error=None):
        proc = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), str(self.root)], env=self.env, capture_output=True,
            text=True, encoding="utf-8", errors="replace", timeout=TIMEOUT)
        self.assertEqual(proc.returncode, code, proc.stdout + proc.stderr)
        if error:
            self.assertIn(error, proc.stderr)
        return proc.stdout

    def calls(self):
        return self.log.read_text("utf-8").splitlines() if self.log.exists() else []

    def assert_call(self, line, task, brief, verdict):
        self.assertEqual(line.split("\t"),
                         [task, "high", "coder-codex", str(brief), str(verdict), str(self.root)])

    def test_clean_row_is_cherry_picked_and_queued_once(self):
        task, wt, row, verdict, brief = self.row(2, train=True, custom_brief=True)
        self.rows(row)
        main_sha = self.git(self.root, "rev-parse", "HEAD")
        out = self.run_cli()
        self.assertIn(f"REAPPLY QUEUED: {task} onto {main_sha[:10]} (2 commits; verdict {verdict})",
                      out.splitlines())
        self.assertIn(f"stub qadd last line: {task}", out.splitlines())
        self.assertRegex(out, r"GIT: [^\n]*rev-list --reverse")
        self.assertEqual(len(self.calls()), 1, self.calls())
        self.assert_call(self.calls()[0], task, brief, verdict)
        self.assertEqual(self.git(wt, "rev-list", "--count", "main..HEAD"), "2")
        self.assertEqual((wt / f"{task}.txt").read_text("utf-8"), "second task commit\n")
        self.assertFalse((wt / "neighbor.txt").exists(), "train predecessor was reverted, not replayed")
        self.assertEqual(self.git(self.root, "rev-parse", "HEAD"), main_sha)
        self.assertFalse((self.root / f"{task}.txt").exists())
        self.entry("q-20261126-205950-3", task, "merged")
        self.assertEqual(self.run_cli(), "")
        self.assertEqual(len(self.calls()), 1, self.calls())

    def test_conflicting_row_prints_conflict_and_leaves_worktree_clean(self):
        clean, _, clean_row, verdict, brief = self.row(2)
        other, wt, conflict_row, _, _ = self.row(3, conflict=True)
        later, later_wt, later_row, _, _ = self.row(4)
        self.rows(clean_row, conflict_row, later_row)
        main_sha = self.git(self.root, "rev-parse", "HEAD")
        later_sha = self.git(later_wt, "rev-parse", "HEAD")
        self.assertIn(f"REAPPLY QUEUED: {clean}", self.run_cli())
        self.entry("q-20261126-205950-5", clean, "merged")
        out = self.run_cli(2)
        self.assertIn(f"REAPPLY CONFLICT: {other} x.txt", out.splitlines())
        self.assertIn("cherry-pick --abort", out)
        self.assertNotIn(later, out)
        self.assertEqual(self.git(wt, "status", "--porcelain"), "")
        self.assertEqual(self.git(wt, "rev-parse", "HEAD"), main_sha)
        self.assertEqual(self.git(self.root, "rev-parse", "HEAD"), main_sha)
        self.assertEqual(self.git(later_wt, "rev-parse", "HEAD"), later_sha)
        self.assertEqual((wt / "x.txt").read_text("utf-8"), "later main edit\n")
        self.assertFalse((wt / f"{other}.txt").exists(), "first pick must also be aborted")
        self.assertEqual(len(self.calls()), 1, self.calls())
        self.assert_call(self.calls()[0], clean, brief, verdict)
        # An operation present before this run is never ours to abort.
        self.rows(conflict_row, later_row)
        meta = Path(self.git(wt, "rev-parse", "--absolute-git-dir"))
        pending = meta / "CHERRY_PICK_HEAD"
        pending.write_text(main_sha + "\n", "utf-8")
        refused = self.run_cli(2)
        self.assertIn(f"REAPPLY CONFLICT: {other} an existing Git operation", refused)
        self.assertNotIn("cherry-pick --abort", refused)
        self.assertNotIn(later, refused)
        self.assertEqual(pending.read_text("utf-8").strip(), main_sha)
        pending.unlink()
        # No Git mutation is allowed through a .git pointer to another checkout.
        git_file = wt / ".git"
        original = git_file.read_bytes()
        git_file.unlink()  # Windows Git marks .git hidden: it cannot be truncated in place
        git_file.write_bytes(b"gitdir: " + str(self.root / ".git").encode("utf-8") + b"\n")
        try:
            refused = self.run_cli(2)
            self.assertIn(f"REAPPLY CONFLICT: {other}", refused)
            self.assertNotIn("merge --ff-only", refused)
        finally:
            git_file.unlink()
            git_file.write_bytes(original)
        local = wt / "untracked.txt"
        local.write_text("local work\n", "utf-8")
        dirty = self.git(wt, "status", "--porcelain")
        refused = self.run_cli(2)
        self.assertIn(f"REAPPLY CONFLICT: {other} task worktree is not clean", refused)
        self.assertIn(f"REAPPLY QUEUED: {later}", refused)
        self.assertEqual(self.git(wt, "rev-parse", "HEAD"), main_sha)
        self.assertEqual(self.git(wt, "status", "--porcelain"), dirty)
        self.assertEqual(local.read_text("utf-8"), "local work\n")
        self.assertEqual([line.split("\t")[0] for line in self.calls()], [clean, later])

    def test_row_without_adjudication_file_is_never_queued(self):
        task, wt, row, verdict, _ = self.row(2)
        before = self.git(wt, "rev-parse", "HEAD")
        self.rows(row)
        verdict.unlink()
        for create, remove in ((None, None), (verdict.touch, verdict.unlink), (verdict.mkdir, verdict.rmdir)):
            with self.subTest(create=create):
                if create:
                    create()
                self.assertIn(f"REAPPLY SKIP: {task} no adjudication file {verdict}", self.run_cli())
                if remove:
                    remove()
                self.assertEqual(self.calls(), [])
                self.assertEqual(self.git(wt, "rev-parse", "HEAD"), before)
        # A verdict swapped for a link at the spawn gate must leave no replay behind.
        verdict.write_text("Judged innocent\n", "utf-8")
        app = load_script()
        cfg = app.Settings.load(self.root)
        parsed = app.Row.parse(cfg, row.encode())
        old_entry = app.eligible(cfg, parsed)
        self.assertIsNotNone(old_entry)
        actual_lstat, enqueue = Path.lstat, app.enqueue
        main_sha = self.git(self.root, "rev-parse", "HEAD")

        def switched(path):
            if path == verdict:
                return os.stat_result((stat.S_IFLNK,) + (0,) * 9)
            return actual_lstat(path)

        def late_enqueue(*args):
            self.assertNotEqual(self.git(wt, "rev-parse", "HEAD"), main_sha)
            with patch.object(Path, "lstat", switched):
                return enqueue(*args)

        with patch.dict(os.environ, self.env, clear=True), patch.object(app, "enqueue", late_enqueue):
            self.assertIs(app.replay(cfg, parsed, old_entry), app.Outcome.STOP)
        self.assertEqual(self.calls(), [])
        self.assertEqual(self.git(wt, "rev-parse", "HEAD"), main_sha)
        self.assertEqual(self.git(wt, "status", "--porcelain"), "")

    def test_held_lane_or_open_entry_queues_nothing(self):
        task, wt, row, _, _ = self.row(2)
        self.rows(row)
        before = self.git(wt, "rev-parse", "HEAD")
        hold = self.queue / "hold-broadsword"
        for make, remove in ((hold.touch, hold.unlink), (hold.mkdir, hold.rmdir)):
            make()
            self.assertIn(f"REAPPLY SKIP: {task} lane held or busy", self.run_cli())
            remove()
        app = load_script()
        cfg = app.Settings.load(self.root)
        parsed = app.Row.parse(cfg, row.encode())
        round_entry = app.eligible(cfg, parsed)
        self.assertIsNotNone(round_entry)
        actual_lstat = Path.lstat

        def unreadable(path):
            if path == hold:
                raise PermissionError("unreadable hold")
            return actual_lstat(path)

        with patch.object(Path, "lstat", unreadable):
            self.assertIs(app.enqueue(cfg, parsed, round_entry, before, 2), app.Handoff.CHECK_QUEUE)
        self.assertEqual(self.calls(), [])
        for status in ("queued", "running", "approved", "interrupted"):
            with self.subTest(status=status):
                path = self.entry("q-20261126-205950-3", "t-other", status, project="Broadsword")
                self.assertIn(f"REAPPLY SKIP: {task} lane held or busy", self.run_cli())
                self.assertEqual(self.git(wt, "rev-parse", "HEAD"), before)
                self.assertEqual(self.calls(), [])
        with path.open("a", encoding="utf-8") as stream:
            stream.write("status = merged\n")
        self.assertIn("malformed or duplicate queue field", self.run_cli())
        path.unlink()
        entry_id = row.split("\t")[1]
        for status, revert in (("merged", before), ("merged_check_failed", "")):
            with self.subTest(status=status, revert=revert):
                self.entry(entry_id, task, status, revert_sha=revert)
                self.assertIn(f"REAPPLY SKIP: {task} not the recorded reverted round", self.run_cli())
        self.assertEqual(self.git(wt, "rev-parse", "HEAD"), before)
        self.assertEqual(self.calls(), [])

    def test_only_one_row_per_run(self):
        first, _, first_row, _, _ = self.row(2)
        second, wt, second_row, _, _ = self.row(3)
        later, later_wt, later_row, _, _ = self.row(4)
        self.rows(first_row, second_row, later_row)
        before, later_sha = [self.git(tree, "rev-parse", "HEAD") for tree in (wt, later_wt)]
        main_sha = self.git(self.root, "rev-parse", "HEAD")
        app = load_script()
        with app.open_input(self.root, self.tsv) as stream:
            self.assertTrue(app.try_lock_tsv(stream))
            self.assertIn("REAPPLY SKIP: another reapply run is using reapply.tsv", self.run_cli())
            self.assertEqual(self.calls(), [])
        out = self.run_cli()
        self.assertIn(f"REAPPLY QUEUED: {first}", out)
        self.assertEqual(len(self.calls()), 1, self.calls())
        self.assertEqual(self.git(wt, "rev-parse", "HEAD"), before)
        self.rows(second_row, later_row)
        self.qadd.write_text(STUB.replace("exit 0", f"if ($Task -eq '{second}') {{ exit 7 }}\nexit 0"), "utf-8")
        out = self.run_cli(2)
        self.assertIn("cherry-pick ", out)
        self.assertIn(f"REAPPLY QADD FAILED: {second} exit 7", out)
        self.assertNotIn(later, out)  # A failure after a replay must not reach the next row.
        self.assertEqual(self.git(later_wt, "rev-parse", "HEAD"), later_sha)
        self.assertIn("reset --keep", out)
        self.assertEqual(self.git(wt, "rev-parse", "HEAD"), main_sha)
        self.assertEqual(self.git(wt, "status", "--porcelain"), "")
        self.assertEqual(len(self.calls()), 2, self.calls())
        # Retry reaches qadd again. This time queue add succeeds but the driver fails.
        self.rows(second_row)
        accepted_id = "q-20261126-205950-6"
        accepted = self.entry(accepted_id, second, "queued")
        accepted.rename(self.root / accepted.name)
        move = f"Move-Item -LiteralPath '{accepted.name}' -Destination '{accepted.relative_to(self.root)}'"
        self.qadd.write_text(STUB.replace("exit 0", f"{move}\nexit 7"), "utf-8")
        out = self.run_cli(2, error="cleanup failed: newer queue entry exists")
        self.assertIn(f"REAPPLY QADD FAILED: {second} exit 7", out)
        self.assertNotIn("reset --keep", out)
        self.assertEqual(self.git(wt, "rev-list", "--count", "main..HEAD"), "2")
        self.assertEqual(len(self.calls()), 3, self.calls())
        self.assertEqual(self.run_cli(), "")
        self.entry(accepted_id, second, "merged")
        self.rows(later_row)
        relative_wt = str(later_wt.relative_to(self.root)).replace("'", "''")
        move_head = f"& git -C '{relative_wt}' commit --allow-empty -qm 'advanced after replay'"
        self.qadd.write_text(STUB.replace("exit 0", f"{move_head}\nif ($?) {{ exit 7 }}\nexit 8"), "utf-8")
        out = self.run_cli(2, error="cleanup failed: task HEAD changed after replay")
        self.assertNotIn("reset --keep", out)
        self.assertEqual(self.git(later_wt, "log", "-1", "--format=%s"), "advanced after replay")
        self.assertEqual(self.git(later_wt, "rev-list", "--count", "main..HEAD"), "3")
        # Low-level capture must fail on a large reply or inherited pipe.
        cfg = app.Settings.load(self.root)
        with self.subTest(child="output cap"):
            child = f"import sys; sys.stdout.write('x' * {cfg.max_bytes + 1})"
            with self.assertRaisesRegex(app.CommandError, "output exceeds mcp_output_max_kb"):
                app.capture(cfg, [sys.executable, "-I", "-c", child], self.root)
        with self.subTest(child="inherited pipe"):
            child = ("import subprocess, sys; "
                     "subprocess.Popen([sys.executable, '-I', '-c', 'import time; time.sleep(5)'], "
                     "stdout=sys.stdout)")
            with self.assertRaisesRegex(app.CommandError, "output did not close"):
                # Do not let a sleeping process retain the disposable root as its cwd on Windows.
                app.capture(replace(cfg, seconds=2), [sys.executable, "-I", "-c", child], HERE)

    def test_already_requeued_row_is_skipped(self):
        task, wt, row, _, _ = self.row(9)
        second, _, next_row, _, _ = self.row(2)
        wedged, wedged_wt, wedged_row, _, _ = self.row(3)
        self.git(wedged_wt, "commit", "--allow-empty", "-qm", "existing unpublished commit")
        before, wedged_sha = [self.git(tree, "rev-parse", "HEAD") for tree in (wt, wedged_wt)]
        # Same-second suffix 10 is newer than suffix 9; an older entry is not a veto.
        self.entry("q-20261126-205950-10", task, "rejected")
        self.entry("q-20261126-205950", second, "rejected", brief=r"Other text\nstatus = queued")
        self.rows(row, wedged_row, next_row)
        out = self.run_cli(2)
        self.assertNotIn(task, out)
        self.assertIn(f"REAPPLY CONFLICT: {wedged} task tip is not an ancestor of main", out)
        self.assertIn(f"REAPPLY QUEUED: {second}", out)
        self.assertEqual([line.split("\t")[0] for line in self.calls()], [second])
        self.assertEqual(self.git(wt, "rev-parse", "HEAD"), before)
        self.assertEqual(self.git(wedged_wt, "rev-parse", "HEAD"), wedged_sha)

    def test_malformed_row_is_reported_not_guessed(self):
        task, _, row, _, _ = self.row(2)
        fields = row.split("\t")
        malformed = [
            "\t".join(fields[:-1]),
            "\t".join([fields[0], "q-20261126-995950", *fields[2:]]),
            "\t".join([*fields[:3], "medium", *fields[4:]]),
            "\t".join([*fields[:2], r"..\verdict.md", *fields[3:]]),
            "\t".join([*fields[:2], r"C:verdict.md", *fields[3:]]),
            "\t".join([*fields[:4], "coder\rcodex", fields[-1]]),
        ]
        oversized = "x" * (SETTINGS["mcp_output_max_kb"] * 1024 + 1)
        self.rows("\ufeff#" + oversized, "", *malformed, oversized, row)
        out = self.run_cli()
        reports = [line for line in out.splitlines() if line.startswith("REAPPLY BAD ROW:")]
        self.assertEqual(len(reports), len(malformed) + 1, reports)
        for ordinal, report in enumerate(reports, 3):
            self.assertTrue(report.startswith(f"REAPPLY BAD ROW: {ordinal} "), report)
        self.assertIn(f"REAPPLY QUEUED: {task}", out)
        self.assertEqual(len(self.calls()), 1, self.calls())
        self.tsv.unlink()
        self.assertEqual(self.run_cli(1), "")


if __name__ == "__main__":
    unittest.main()
