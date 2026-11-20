"""Tests for cold_move.py. Every test uses temp folders as the "internal" and "external" drives.

Run: python -I -B scripts/test_cold_move.py   (or python -I -m unittest scripts/test_cold_move.py -v)
"""
import contextlib
import io
import os
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import cold_move  # noqa: E402

GIT_ENV = dict(
    os.environ,
    GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.invalid",
    GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.invalid",
)
OLD = time.time() - 100 * 86400


def git(cwd, *args):
    p = subprocess.run(["git", "-C", str(cwd), *args], capture_output=True, text=True, env=GIT_ENV)
    assert p.returncode == 0, p.stderr
    return p.stdout


def put(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def age(path, when=OLD):
    for dirpath, _d, files in os.walk(path):
        for n in files:
            os.utime(Path(dirpath) / n, (when, when))
    os.utime(path, (when, when))


class Fixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        base = Path(self.tmp.name).resolve()
        self.root = base / "internal"
        self.ext = base / "external"
        self.root.mkdir()
        self.ext.mkdir()
        git(self.root, "init", "-q")
        put(self.root / "README.md", "hello")
        # a tracked folder under reference/
        put(self.root / "reference" / "tracked" / "a.txt", "tracked")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "init")
        # untracked cold items
        put(self.root / "cache" / "oldcache" / "x.bin", "x" * 1000)
        put(self.root / "cache" / "oldcache" / "sub" / "y.bin", "y" * 10)
        age(self.root / "cache" / "oldcache")
        put(self.root / "cache" / "newcache" / "z.bin", "fresh")
        put(self.root / "evidence" / "ev1" / "e.txt", "evidence")
        put(self.root / "reference" / "ref1" / "r.txt", "reference")
        (self.root / "state").mkdir()
        put(self.root / "state" / "records" / "rec.txt", "record")
        # worktrees
        for name in ("t-done", "t-run"):
            git(self.root, "worktree", "add", "-q", "-b", name, f"worktrees/proj/{name}")
        put(self.root / "state" / "board.tsv", "t-done\tdone\nt-run\trunning\n")
        self.board = self.root / "state" / "board.tsv"
        self.approval = self.root / "state" / "cold-move-approved.txt"

    def argv(self, *extra, with_worktrees=True):
        a = [
            "--root", str(self.root), "--target", str(self.ext),
            "--trigger-gb", "1e9", "--state-dir", "state", "--cold-days", "30",
            "--cache-dir", "cache", "--evidence-dir", "evidence", "--reference-dir", "reference",
        ]
        if with_worktrees:
            a += ["--worktrees-dir", "worktrees", "--board-file", str(self.board)]
        return a + list(extra)

    def go(self, *extra, **kw):
        out = io.StringIO()
        args = cold_move.parse(self.argv(*extra, **kw))
        try:
            code = cold_move.run(args, out)
        except cold_move.Refusal as e:
            return 2, str(e)
        return code, out.getvalue()

    def approve(self, classes=cold_move.CLASSES, date=None):
        put(self.approval, f"APPROVED {date or cold_move.utc_today()} {','.join(classes)}\n")

    def used_approvals(self):
        return sorted(p.name for p in self.approval.parent.iterdir() if ".used-" in p.name)

    YES = ("--yes", "--approval-file", "state/cold-move-approved.txt")
    ALL = ("--class", "build-cache", "--class", "evidence", "--class", "reference", "--class", "worktree")

    def snapshot(self):
        return sorted(str(p.relative_to(self.root)) for p in self.root.rglob("*") if ".git" not in p.parts[len(self.root.parts):])


class DryRun(Fixture):
    def test_dry_run_moves_nothing_and_lists_each_item(self):
        before = self.snapshot()
        code, out = self.go()
        self.assertEqual(code, 0, out)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(list(self.ext.iterdir()), [])
        for needle in ("build-cache", "oldcache", "evidence", "ev1", "reference", "ref1", "worktree", "t-done"):
            self.assertIn(needle, out)
        self.assertIn("1010", out)  # size of oldcache
        self.assertIn("dry run: 4 item(s)", out)  # oldcache, ev1, ref1, t-done


class Approval(Fixture):
    def test_yes_without_approval_file_is_refused_and_moves_nothing(self):
        before = self.snapshot()
        code, out = self.go(*self.YES, *self.ALL)
        self.assertEqual(code, 2)
        self.assertIn("approval file", out)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(list(self.ext.iterdir()), [])

    def test_approval_file_wrong_content_is_refused(self):
        put(self.approval, "yes please\n")
        code, out = self.go(*self.YES, *self.ALL)
        self.assertEqual(code, 2)
        self.assertEqual(list(self.ext.iterdir()), [])

    def test_yes_needs_a_class(self):
        self.approve()
        code, _ = self.go(*self.YES)
        self.assertEqual(code, 2)

    def test_approval_file_must_be_in_state(self):
        put(self.root / "elsewhere.txt", "APPROVED\n")
        code, out = self.go("--yes", "--approval-file", "elsewhere.txt", *self.ALL)
        self.assertEqual(code, 2)
        self.assertIn("state folder", out)


class Moves(Fixture):
    def test_yes_with_approval_moves_all_listed_classes(self):
        self.approve()
        code, out = self.go(*self.YES, *self.ALL)
        self.assertEqual(code, 0, out)
        moved = self.ext / "build-cache" / "cache" / "oldcache"
        self.assertEqual((moved / "x.bin").read_text(), "x" * 1000)
        self.assertEqual((moved / "sub" / "y.bin").read_text(), "y" * 10)
        self.assertFalse((self.root / "cache" / "oldcache").exists())
        self.assertTrue((self.ext / "evidence" / "evidence" / "ev1" / "e.txt").is_file())
        self.assertTrue((self.ext / "reference" / "reference" / "ref1" / "r.txt").is_file())
        self.assertFalse((self.root / "evidence" / "ev1").exists())
        # retired worktree moved with git, and git knows the new place
        newwt = self.ext / "worktree" / "worktrees" / "proj" / "t-done"
        self.assertTrue((newwt / "README.md").is_file())
        self.assertFalse((self.root / "worktrees" / "proj" / "t-done").exists())
        listing = git(self.root, "worktree", "list", "--porcelain").replace("\\", "/")
        self.assertIn(newwt.as_posix(), listing)

    def test_only_named_classes_move(self):
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertFalse((self.root / "evidence" / "ev1").exists())
        self.assertTrue((self.root / "cache" / "oldcache").exists())
        self.assertTrue((self.root / "reference" / "ref1").exists())
        self.assertTrue((self.root / "worktrees" / "proj" / "t-done").exists())

    def test_checksum_mismatch_keeps_source_and_removes_bad_copy(self):
        self.approve()
        real_copy = cold_move.copy_item

        def corrupting(src, dst):
            real_copy(src, dst)
            files = [dst] if dst.is_file() else [q for q in dst.rglob("*") if q.is_file()]
            files[0].write_bytes(b"CORRUPT" + files[0].read_bytes()[7:])  # same size, other bytes

        with mock.patch.object(cold_move, "copy_item", corrupting):
            code, out = self.go(*self.YES, "--class", "build-cache", "--class", "evidence")
        self.assertEqual(code, 1, out)
        self.assertIn("FAILED", out)
        self.assertEqual((self.root / "cache" / "oldcache" / "x.bin").read_text(), "x" * 1000)
        self.assertTrue((self.root / "evidence" / "ev1" / "e.txt").is_file())
        self.assertFalse((self.ext / "build-cache" / "cache" / "oldcache").exists())

    def test_size_mismatch_keeps_source(self):
        self.approve()
        real_copy = cold_move.copy_item

        def truncating(src, dst):
            real_copy(src, dst)
            q = next(p for p in dst.rglob("x.bin"))
            q.write_bytes(b"short")

        with mock.patch.object(cold_move, "copy_item", truncating):
            code, out = self.go(*self.YES, "--class", "build-cache")
        self.assertEqual(code, 1, out)
        self.assertTrue((self.root / "cache" / "oldcache" / "x.bin").is_file())

    def test_existing_target_is_never_overwritten(self):
        put(self.ext / "evidence" / "evidence" / "ev1" / "e.txt", "already here")
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertIn("target already exists", out)
        self.assertEqual((self.ext / "evidence" / "evidence" / "ev1" / "e.txt").read_text(), "already here")
        self.assertTrue((self.root / "evidence" / "ev1" / "e.txt").is_file())


class Safety(Fixture):
    """Deletion safety: a run only ever removes what it created itself."""

    EV_DST = ("evidence", "evidence", "ev1")

    def ev_dst(self):
        return self.ext.joinpath(*self.EV_DST)

    def test_dst_created_after_planning_is_never_removed(self):
        self.approve()
        real_plan = cold_move.build_plan

        def plan_then_other_run_wins(*a, **k):
            items = real_plan(*a, **k)
            put(self.ev_dst() / "e.txt", "other run's verified copy")
            return items

        with mock.patch.object(cold_move, "build_plan", plan_then_other_run_wins):
            code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual((self.ev_dst() / "e.txt").read_text(), "other run's verified copy")
        self.assertTrue((self.root / "evidence" / "ev1" / "e.txt").is_file())
        self.assertIn("FAILED", out)

    def test_copy_error_removes_only_its_own_temp_path(self):
        self.approve()
        real_copy = cold_move.copy_item

        def other_run_creates_dst_then_copy_fails(src, dst):
            real_copy(src, dst)  # our partial copy, at whatever name the mover chose
            put(self.ev_dst() / "e.txt", "other run's verified copy")
            raise OSError("disk went away")

        with mock.patch.object(cold_move, "copy_item", other_run_creates_dst_then_copy_fails):
            code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 1, out)
        self.assertEqual((self.ev_dst() / "e.txt").read_text(), "other run's verified copy")
        self.assertTrue((self.root / "evidence" / "ev1" / "e.txt").is_file())
        # our own partial copy is gone: only the other run's folder is left beside it
        self.assertEqual([p.name for p in self.ev_dst().parent.iterdir()], ["ev1"])

    def test_source_gone_before_copy_is_failed_and_touches_no_target(self):
        self.approve()
        real_plan = cold_move.build_plan

        def plan_then_source_vanishes(*a, **k):
            items = real_plan(*a, **k)
            import shutil
            shutil.rmtree(self.root / "evidence" / "ev1")
            return items

        with mock.patch.object(cold_move, "build_plan", plan_then_source_vanishes):
            code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 1, out)
        self.assertIn("FAILED", out)
        self.assertFalse(self.ev_dst().exists())

    def test_held_run_lock_refuses_and_is_left_alone(self):
        self.approve()
        lock = self.root / "state" / cold_move.LOCK_NAME
        put(lock, "pid of another run\n")
        before = self.snapshot()
        code, out = self.go(*self.YES, *self.ALL)
        self.assertEqual(code, 2, out)
        self.assertIn("lock", out)
        self.assertTrue(lock.is_file())
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(list(self.ext.iterdir()), [])

    def test_lock_is_released_after_a_run(self):
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertFalse((self.root / "state" / cold_move.LOCK_NAME).exists())
        self.approve()  # a new yes; the first approval was used up
        code, out = self.go(*self.YES, "--class", "reference")  # a second run is not blocked
        self.assertEqual(code, 0, out)

    def test_worktree_board_change_after_planning_keeps_source(self):
        self.approve()
        real_plan = cold_move.build_plan

        def plan_then_task_reopened(*a, **k):
            items = real_plan(*a, **k)
            put(self.board, "t-done\trunning\nt-run\trunning\n")
            return items

        with mock.patch.object(cold_move, "build_plan", plan_then_task_reopened):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 1, out)
        self.assertIn("board status is now running", out)
        self.assertTrue((self.root / "worktrees" / "proj" / "t-done" / "README.md").is_file())
        self.assertFalse((self.ext / "worktree").exists())

    def test_worktree_dst_appearing_after_planning_keeps_both(self):
        self.approve()
        real_plan = cold_move.build_plan
        dst = self.ext / "worktree" / "worktrees" / "proj" / "t-done"

        def plan_then_dst_appears(*a, **k):
            items = real_plan(*a, **k)
            put(dst / "other.txt", "someone else's")
            return items

        with mock.patch.object(cold_move, "build_plan", plan_then_dst_appears):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 1, out)
        self.assertIn("target appeared after planning", out)
        self.assertEqual((dst / "other.txt").read_text(), "someone else's")
        self.assertTrue((self.root / "worktrees" / "proj" / "t-done" / "README.md").is_file())


class ApprovalScope(Fixture):
    """Finding 1: an approval names its classes and date, and is used up by one --yes run."""

    def test_approval_for_one_class_does_not_cover_another(self):
        self.approve(["evidence"])
        before = self.snapshot()
        code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 2, out)
        self.assertIn("does not approve class worktree", out)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(list(self.ext.iterdir()), [])
        self.assertTrue(self.approval.is_file())  # a refused run uses nothing up

    def test_approval_must_name_every_requested_class(self):
        self.approve(["evidence"])
        code, out = self.go(*self.YES, "--class", "evidence", "--class", "reference")
        self.assertEqual(code, 2, out)
        self.assertIn("reference", out)
        self.assertTrue((self.root / "evidence" / "ev1").exists())

    def test_approval_with_old_date_is_refused(self):
        self.approve(date="2026-11-19" if cold_move.utc_today() != "2026-11-19" else "2026-11-18")
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 2, out)
        self.assertIn("not today", out)
        self.assertTrue((self.root / "evidence" / "ev1").exists())

    def test_old_free_text_approval_is_refused(self):
        put(self.approval, "APPROVED by Lazlo in chat\n")
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 2, out)
        self.assertTrue((self.root / "evidence" / "ev1").exists())

    def test_unknown_class_in_approval_is_refused(self):
        put(self.approval, f"APPROVED {cold_move.utc_today()} evidence,everything\n")
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 2, out)
        self.assertIn("unknown class", out)

    def test_yes_run_uses_up_the_approval_and_the_next_run_is_refused(self):
        self.approve(["evidence", "reference"])
        text = self.approval.read_text()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertFalse(self.approval.exists())
        used = self.used_approvals()
        self.assertEqual(len(used), 1, used)
        self.assertEqual((self.approval.parent / used[0]).read_text(), text)  # renamed, not deleted
        # a later first run of another class, even one the old file named, needs a new yes
        code, out = self.go(*self.YES, "--class", "reference")
        self.assertEqual(code, 2, out)
        self.assertIn("does not exist", out)
        self.assertTrue((self.root / "reference" / "ref1").exists())

    def test_approval_rename_failure_moves_nothing(self):
        self.approve()
        real_rename = os.rename

        def no_rename_for_approval(a, b, *r, **k):
            if Path(a).name == self.approval.name:
                raise OSError("rename blocked")
            return real_rename(a, b, *r, **k)

        before = self.snapshot()
        with mock.patch.object(cold_move.os, "rename", no_rename_for_approval):
            code, out = self.go(*self.YES, *self.ALL)
        self.assertEqual(code, 2, out)
        self.assertIn("use up", out)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(list(self.ext.iterdir()), [])


class RepoGuardTests(Fixture):
    """Finding 2: a copy-class candidate that is, holds or sits in a repo or worktree never moves."""

    def guard(self, with_wdir=True):
        state = self.root / "state"
        wdir = (self.root / "worktrees") if with_wdir else None
        return cold_move.RepoGuard(self.root, state, wdir, cold_move.registered_worktrees(self.root))

    def test_plain_untracked_folder_passes(self):
        self.assertIsNone(cold_move.repo_guard(self.guard(), self.root / "evidence" / "ev1"))

    def test_nested_git_folder_at_depth_is_refused(self):
        put(self.root / "reference" / "clone" / "deep" / "er" / ".git" / "HEAD", "ref: x")
        why = cold_move.repo_guard(self.guard(), self.root / "reference" / "clone")
        self.assertIsNotNone(why)
        self.assertIn(".git", why)

    def test_nested_git_file_is_refused(self):
        put(self.root / "reference" / "wt" / "sub" / ".git", "gitdir: elsewhere")
        why = cold_move.repo_guard(self.guard(), self.root / "reference" / "wt")
        self.assertIsNotNone(why)
        self.assertIn(".git", why)

    def test_registered_worktree_is_refused_even_without_worktrees_dir(self):
        g = self.guard(with_wdir=False)
        why = cold_move.repo_guard(g, self.root / "worktrees" / "proj" / "t-done")
        self.assertIn("worktree", why)

    def test_folder_holding_a_registered_worktree_is_refused(self):
        g = self.guard(with_wdir=False)
        why = cold_move.repo_guard(g, self.root / "worktrees" / "proj")
        self.assertIn("holds a git worktree", why)

    def test_path_inside_a_registered_worktree_is_refused(self):
        g = self.guard(with_wdir=False)
        why = cold_move.repo_guard(g, self.root / "worktrees" / "proj" / "t-done" / "README.md")
        self.assertIn("sits inside a git worktree", why)

    def test_overlap_with_worktrees_dir_is_refused(self):
        put(self.root / "worktrees" / "stray" / "f.txt", "not registered, no .git")
        why = cold_move.repo_guard(self.guard(), self.root / "worktrees" / "stray")
        self.assertIn("worktrees folder", why)

    def test_overlap_with_state_is_refused(self):
        why = cold_move.repo_guard(self.guard(), self.root / "state" / "records")
        self.assertIn("state", why)

    def test_unreadable_walk_fails_closed(self):
        put(self.root / "reference" / "r2" / "a" / "f.txt", "x")
        real_walk = os.walk

        def failing_walk(top, *a, onerror=None, **k):
            if onerror is not None:
                onerror(PermissionError("denied"))
            yield from real_walk(top, *a, onerror=onerror, **k)

        with mock.patch.object(cold_move.os, "walk", failing_walk):
            with self.assertRaises(OSError):
                cold_move.repo_guard(self.guard(), self.root / "reference" / "r2")

    def test_reference_dir_over_worktrees_moves_nothing_end_to_end(self):
        self.approve()
        before = self.snapshot()
        code, out = self.go(*self.YES, "--class", "reference", "--reference-dir", "worktrees/proj",
                            "--reference-dir", "worktrees/proj/t-done")
        self.assertEqual(code, 0, out)
        self.assertNotIn("moved " + str(self.root / "worktrees"), out)
        self.assertTrue((self.root / "worktrees" / "proj" / "t-done" / "README.md").is_file())
        self.assertTrue((self.root / "worktrees" / "proj" / "t-run" / "README.md").is_file())
        self.assertFalse((self.ext / "reference" / "worktrees").exists())

    def test_nested_clone_end_to_end_is_skipped(self):
        clone = self.root / "evidence" / "clone"
        clone.mkdir(parents=True)
        git(clone, "init", "-q")
        put(clone / "work.txt", "untracked by the root repo")
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertIn(".git", out)
        self.assertTrue((clone / "work.txt").is_file())
        self.assertFalse((self.ext / "evidence" / "evidence" / "clone").exists())

    def test_guard_runs_again_right_before_the_copy(self):
        self.approve()
        real_plan = cold_move.build_plan

        def plan_then_clone_appears(*a, **k):
            items = real_plan(*a, **k)
            put(self.root / "evidence" / "ev1" / "nested" / ".git" / "HEAD", "ref: x")
            return items

        with mock.patch.object(cold_move, "build_plan", plan_then_clone_appears):
            code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 1, out)
        self.assertIn("found right before the copy", out)
        self.assertTrue((self.root / "evidence" / "ev1" / "e.txt").is_file())
        self.assertFalse((self.ext / "evidence" / "evidence" / "ev1").exists())

    def test_retired_worktree_with_gitignored_env_is_never_moved(self):
        with open(self.root / ".git" / "info" / "exclude", "a", encoding="utf-8") as f:
            f.write("\n.env\n")
        put(self.root / "worktrees" / "proj" / "t-done" / ".env", "KEY=1")
        self.assertEqual(git(self.root / "worktrees" / "proj" / "t-done", "status", "--porcelain").strip(), "")
        self.approve()
        code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 0, out)
        self.assertIn("secret file", out)
        self.assertTrue((self.root / "worktrees" / "proj" / "t-done" / ".env").is_file())
        self.assertFalse((self.ext / "worktree").exists())


class Skips(Fixture):
    def test_active_and_unknown_worktrees_are_skipped(self):
        git(self.root, "worktree", "add", "-q", "-b", "t-ghost", "worktrees/proj/t-ghost")
        self.approve()
        code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 0, out)
        self.assertIn("active worktree (board status running)", out)
        self.assertIn("not on the board", out)
        self.assertTrue((self.root / "worktrees" / "proj" / "t-run" / "README.md").is_file())
        self.assertTrue((self.root / "worktrees" / "proj" / "t-ghost" / "README.md").is_file())
        self.assertFalse((self.root / "worktrees" / "proj" / "t-done").exists())

    def test_dirty_retired_worktree_is_skipped(self):
        put(self.root / "worktrees" / "proj" / "t-done" / "wip.txt", "unsaved")
        self.approve()
        code, out = self.go(*self.YES, "--class", "worktree")
        self.assertIn("dirty", out)
        self.assertTrue((self.root / "worktrees" / "proj" / "t-done" / "wip.txt").is_file())

    def test_unreadable_board_refuses_the_run(self):
        self.board.unlink()
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 2)
        self.assertIn("board", out)
        self.assertTrue((self.root / "evidence" / "ev1").exists())

    def test_git_tracked_folder_never_moves(self):
        self.approve()
        code, out = self.go(*self.YES, "--class", "reference")
        self.assertEqual(code, 0, out)
        self.assertIn("git-tracked", out)
        self.assertEqual((self.root / "reference" / "tracked" / "a.txt").read_text(), "tracked")
        self.assertFalse((self.ext / "reference" / "reference" / "tracked").exists())

    def test_folder_with_one_tracked_file_inside_never_moves(self):
        put(self.root / "evidence" / "mixed" / "keep.txt", "tracked inside")
        put(self.root / "evidence" / "mixed" / "other.txt", "untracked")
        git(self.root, "add", "-f", "evidence/mixed/keep.txt")
        git(self.root, "commit", "-q", "-m", "track one")
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertTrue((self.root / "evidence" / "mixed" / "keep.txt").is_file())
        self.assertTrue((self.root / "evidence" / "mixed" / "other.txt").is_file())

    def test_not_cold_cache_is_skipped(self):
        self.approve()
        self.go(*self.YES, "--class", "build-cache")
        self.assertTrue((self.root / "cache" / "newcache" / "z.bin").is_file())

    def test_secret_file_folder_is_skipped(self):
        put(self.root / "evidence" / "sec" / ".env", "KEY=1")
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertIn("secret file", out)
        self.assertTrue((self.root / "evidence" / "sec" / ".env").is_file())

    def test_symlink_folder_is_skipped(self):
        link = self.root / "evidence" / "lnk"
        try:
            os.symlink(self.root / "README.md", link)
        except (OSError, NotImplementedError):
            self.skipTest("symlinks not permitted here")
        self.approve()
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertIn("symlink", out)
        self.assertTrue(link.is_symlink())

    def test_state_folder_is_never_a_source(self):
        code, out = self.go("--evidence-dir", "state")
        self.assertEqual(code, 2)
        self.assertIn("state", out)
        self.assertTrue((self.root / "state" / "records" / "rec.txt").is_file())


class Trigger(Fixture):
    def test_not_under_trigger_refuses(self):
        args = cold_move.parse(self.argv())
        args.trigger_gb = 0.0
        with self.assertRaises(cold_move.Refusal) as cm:
            cold_move.run(args, io.StringIO())
        self.assertIn("not running", str(cm.exception))

    def test_force_list_allows_dry_run_only(self):
        args = cold_move.parse(self.argv("--force-list"))
        args.trigger_gb = 0.0
        out = io.StringIO()
        self.assertEqual(cold_move.run(args, out), 0)
        self.assertIn("dry run", out.getvalue())
        self.approve()
        code, msg = self.go("--force-list", *self.YES, *self.ALL)
        self.assertEqual(code, 2)
        self.assertIn("dry run only", msg)

    def test_target_inside_root_refused(self):
        args = cold_move.parse(self.argv())
        args.target = str(self.root / "cache")
        with self.assertRaises(cold_move.Refusal):
            cold_move.run(args, io.StringIO())

    def test_bad_flags_refused(self):
        for bad in (["--trigger-gb", "nan"], ["--cold-days", "-1"]):
            argv = self.argv()
            i = argv.index(bad[0])
            argv[i + 1] = bad[1]
            with self.assertRaises(cold_move.Refusal):
                cold_move.run(cold_move.parse(argv), io.StringIO())
        with self.assertRaises(cold_move.Refusal):
            cold_move.run(cold_move.parse(self.argv("--cache-dir", "../x")), io.StringIO())

    def test_main_returns_2_on_refusal(self):
        err = io.StringIO()
        with mock.patch.object(sys, "stderr", err):
            code = cold_move.main(self.argv(*self.YES, *self.ALL))
        self.assertEqual(code, 2)
        self.assertIn("refused", err.getvalue())


def tree_state(path):
    """{relative path: (size, mtime_ns, bytes)} for files, None for folders. Test-side, plain
    os.walk (the fixtures that use it hold no links), independent of cold_move.scan_tree."""
    out = {}
    for dirpath, dirnames, files in os.walk(path):
        for n in dirnames:
            out[(Path(dirpath) / n).relative_to(path).as_posix()] = None
        for n in files:
            q = Path(dirpath) / n
            st = q.stat()
            out[q.relative_to(path).as_posix()] = (st.st_size, st.st_mtime_ns, q.read_bytes())
    return out


def junction(link, target):
    """Make an NTFS junction (no admin right needed). Returns False when it cannot be made."""
    link.parent.mkdir(parents=True, exist_ok=True)
    p = subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(target)],
                       capture_output=True, text=True)
    return p.returncode == 0 and os.path.isjunction(link)


def read_only(path):
    os.chmod(path, stat.S_IREAD)


def is_read_only(path):
    return not os.stat(path).st_mode & stat.S_IWRITE


class CrossDrive(Fixture):
    """Requirement 1: a worktree across drives is copied, verified, repaired, listed, then removed.

    A second drive is not available, so `git worktree move` is made to fail the way it does across
    drives on Windows, and the FORCE_WORKTREE_COPY hook (which may only make the guard stricter)
    sends the move down the copy path.
    """

    def setUp(self):
        super().setUp()
        self.wt = self.root / "worktrees" / "proj" / "t-done"
        self.new = self.ext / "worktree" / "worktrees" / "proj" / "t-done"
        self.calls = []

    def cross_drive(self, fail=None):
        """fail(args) -> CompletedProcess or None: a git call to fake. `worktree move` always fails."""
        real = cold_move.run_git

        def fake(root, *args):
            self.calls.append(args)
            if args[:2] == ("worktree", "move"):
                return subprocess.CompletedProcess(args, 128, "", "fatal: simulated cross-drive move failure")
            if fail is not None and (r := fail(args)) is not None:
                return r
            return real(root, *args)

        stack = contextlib.ExitStack()
        stack.enter_context(mock.patch.object(cold_move, "FORCE_WORKTREE_COPY", True, create=True))
        stack.enter_context(mock.patch.object(cold_move, "run_git", fake))
        return stack

    def listing(self):
        return git(self.root, "worktree", "list", "--porcelain").replace("\\", "/")

    def assert_source_untouched_and_no_copy(self, before, out):
        self.assertEqual(tree_state(self.wt), before, out)
        self.assertEqual(list(self.new.parent.iterdir()) if self.new.parent.exists() else [], [], out)
        self.assertIn(self.wt.as_posix(), self.listing())
        self.assertNotIn(self.new.as_posix(), self.listing())
        self.assertEqual(git(self.wt, "status", "--porcelain").strip(), "")

    def test_cross_drive_copy_verify_repair_list_then_remove(self):
        read_only(self.wt / "README.md")  # read-only files, as git objects are
        self.approve(["worktree"])
        with self.cross_drive():
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 0, out)
        self.assertNotIn(("worktree", "move"), [c[:2] for c in self.calls])
        self.assertIn(("worktree", "repair", str(self.new)), self.calls)
        self.assertEqual((self.new / "README.md").read_text(), "hello")
        self.assertTrue((self.new / ".git").read_text().startswith("gitdir:"))
        self.assertFalse(self.wt.exists())
        self.assertIn(self.new.as_posix(), self.listing())
        self.assertNotIn(self.wt.as_posix() + "\n", self.listing())
        self.assertEqual(git(self.new, "status", "--porcelain").strip(), "")
        self.assertEqual(list(self.new.parent.iterdir()), [self.new])  # no temp copy left

    def test_cross_drive_verify_failure_keeps_source_and_removes_the_copy(self):
        read_only(self.wt / "README.md")  # the partial copy then holds a read-only file too
        before = tree_state(self.wt)
        self.approve(["worktree"])
        real_copy = cold_move.copy_item

        def corrupting(src, dst):
            real_copy(src, dst)
            q = dst / "README.md"
            os.chmod(q, stat.S_IWRITE | stat.S_IREAD)
            q.write_text("HELLO")  # same size, other bytes
            read_only(q)

        with self.cross_drive(), mock.patch.object(cold_move, "copy_item", corrupting):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 1, out)
        self.assertIn("step verify", out)
        self.assert_source_untouched_and_no_copy(before, out)

    def test_cross_drive_repair_failure_keeps_source_and_removes_the_copy(self):
        before = tree_state(self.wt)
        self.approve(["worktree"])

        def repair_fails(args):
            if args[:2] == ("worktree", "repair"):
                return subprocess.CompletedProcess(args, 1, "", "error: simulated repair failure")
            return None

        with self.cross_drive(repair_fails):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 1, out)
        self.assertIn("step repair", out)
        self.assert_source_untouched_and_no_copy(before, out)

    def test_cross_drive_list_failure_points_git_back_and_keeps_source(self):
        before = tree_state(self.wt)
        self.approve(["worktree"])
        state = {"repaired": False}

        def list_fails_after_repair(args):
            if args[:2] == ("worktree", "repair") and str(self.new) in args:
                state["repaired"] = True
            if args[:2] == ("worktree", "list") and state["repaired"]:
                return subprocess.CompletedProcess(args, 1, "", "error: simulated list failure")
            return None

        with self.cross_drive(list_fails_after_repair):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 1, out)
        self.assertIn("step list", out)
        self.assertIn("pointed back", out)
        # git's record was put back from its saved bytes, not by repairing (which would rewrite
        # the source's .git file): the source is byte- and mtime-identical, and git lists it again
        self.assertNotIn(("worktree", "repair", str(self.wt)), self.calls)
        self.assert_source_untouched_and_no_copy(before, out)

    def test_cross_drive_source_changed_after_verify_is_not_removed(self):
        self.approve(["worktree"])
        real_verify = cold_move.verify_copy

        def verify_then_someone_writes(src, dst):
            real_verify(src, dst)
            if src == self.wt:
                put(self.wt / "late.txt", "written after the verify")

        with self.cross_drive(), mock.patch.object(cold_move, "verify_copy", verify_then_someone_writes):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 1, out)
        self.assertIn("changed after it was verified", out)
        self.assertEqual((self.wt / "late.txt").read_text(), "written after the verify")
        self.assertTrue((self.wt / "README.md").is_file())

    def test_cross_drive_board_flip_after_planning_keeps_source(self):
        before = tree_state(self.wt)
        self.approve(["worktree"])
        real_plan = cold_move.build_plan

        def plan_then_task_reopened(*a, **k):
            items = real_plan(*a, **k)
            put(self.board, "t-done\trunning\nt-run\trunning\n")
            return items

        with self.cross_drive(), mock.patch.object(cold_move, "build_plan", plan_then_task_reopened):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 1, out)
        self.assertIn("board status is now running", out)
        self.assert_source_untouched_and_no_copy(before, out)
        self.assertTrue((self.root / "worktrees" / "proj" / "t-run" / "README.md").is_file())

    def test_same_drive_still_uses_git_worktree_move(self):
        real = cold_move.run_git

        def spy(root, *args):
            self.calls.append(args)
            return real(root, *args)

        self.approve(["worktree"])
        with mock.patch.object(cold_move, "run_git", spy):
            code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 0, out)
        self.assertIn(("worktree", "move"), [c[:2] for c in self.calls])
        self.assertIn(self.new.as_posix(), self.listing())

    def test_locked_worktree_is_skipped(self):
        git(self.root, "worktree", "lock", str(self.wt))
        self.approve(["worktree"])
        code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 0, out)
        self.assertIn("skipped: locked in git", out)
        self.assertTrue((self.wt / "README.md").is_file())
        self.assertFalse(self.new.exists())


class UsedApproval(Fixture):
    """Requirement 2: a used-up approval (.used-<UTC>) is never accepted again."""

    def test_used_approval_file_is_refused(self):
        used = self.root / "state" / "cold-move-approved.txt.used-20261120T000000Z-abcd1234"
        put(used, f"APPROVED {cold_move.utc_today()} evidence\n")
        code, out = self.go("--yes", "--approval-file", "state/" + used.name, "--class", "evidence")
        self.assertEqual(code, 2, out)
        self.assertIn("used-up approval", out)
        self.assertTrue((self.root / "evidence" / "ev1" / "e.txt").is_file())
        self.assertTrue(used.is_file())


class Links(Fixture):
    """Requirement 3: an item that is or holds a junction or symlink is skipped, never followed."""

    def setUp(self):
        super().setUp()
        self.outside = Path(self.tmp.name).resolve() / "outside"
        put(self.outside / "keep.txt", "not ours")
        read_only(self.outside / "keep.txt")
        self.addCleanup(os.chmod, self.outside / "keep.txt", stat.S_IWRITE | stat.S_IREAD)

    def link(self, at):
        if not junction(at, self.outside):
            self.skipTest("cannot make an NTFS junction here")

    def assert_outside_untouched(self):
        self.assertEqual((self.outside / "keep.txt").read_text(), "not ours")
        self.assertTrue(is_read_only(self.outside / "keep.txt"))  # never chmod'ed
        self.assertEqual(sorted(p.name for p in self.outside.iterdir()), ["keep.txt"])

    def test_junction_inside_an_item_is_skipped(self):
        self.link(self.root / "evidence" / "ev1" / "sub" / "j")
        self.approve(["evidence"])
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertIn("skipped: contains a link (sub/j)", out)
        self.assertTrue((self.root / "evidence" / "ev1" / "e.txt").is_file())
        self.assertTrue(os.path.isjunction(self.root / "evidence" / "ev1" / "sub" / "j"))
        self.assertFalse((self.ext / "evidence" / "evidence" / "ev1").exists())
        self.assert_outside_untouched()

    def test_item_that_is_a_junction_is_skipped(self):
        self.link(self.root / "evidence" / "jtop")
        self.approve(["evidence"])
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertIn("skipped: contains a link (.)", out)
        self.assertTrue(os.path.isjunction(self.root / "evidence" / "jtop"))
        self.assertFalse((self.ext / "evidence" / "evidence" / "jtop").exists())
        self.assert_outside_untouched()

    def test_worktree_holding_a_junction_is_skipped(self):
        with open(self.root / ".git" / "info" / "exclude", "a", encoding="utf-8") as f:
            f.write("\njl\n")
        wt = self.root / "worktrees" / "proj" / "t-done"
        self.link(wt / "jl")
        self.approve(["worktree"])
        code, out = self.go(*self.YES, "--class", "worktree")
        self.assertEqual(code, 0, out)
        self.assertIn("skipped: contains a link (jl)", out)
        self.assertTrue((wt / "README.md").is_file())
        self.assertFalse((self.ext / "worktree").exists())
        self.assert_outside_untouched()

    def test_junction_appearing_after_the_verify_stops_the_removal(self):
        self.approve(["evidence"])
        real_verify = cold_move.verify_copy
        ev1 = self.root / "evidence" / "ev1"

        def verify_then_link_appears(src, dst):
            real_verify(src, dst)
            if src == ev1:
                self.link(ev1 / "j")

        with mock.patch.object(cold_move, "verify_copy", verify_then_link_appears):
            code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 1, out)
        self.assertIn("link", out)
        self.assertTrue((ev1 / "e.txt").is_file())
        self.assertTrue(os.path.isjunction(ev1 / "j"))
        self.assert_outside_untouched()

    def test_delete_tree_refuses_a_link_and_deletes_nothing(self):
        d = self.root / "evidence" / "ev1"
        snap = cold_move.scan_tree(d)
        self.link(d / "j")
        with self.assertRaises(cold_move.LinkFound):
            cold_move.delete_tree(d, snap)
        self.assertTrue((d / "e.txt").is_file())
        self.assert_outside_untouched()

    def test_delete_tree_refuses_a_changed_tree_and_deletes_nothing(self):
        d = self.root / "evidence" / "ev1"
        snap = cold_move.scan_tree(d)
        put(d / "new.txt", "added after the snapshot")
        with self.assertRaises(cold_move.TreeChanged):
            cold_move.delete_tree(d, snap)
        self.assertTrue((d / "e.txt").is_file())
        self.assertTrue((d / "new.txt").is_file())


class ReadOnly(Fixture):
    """Requirement 4: a source with read-only files is removed; nothing outside it is chmod'ed."""

    def test_read_only_files_in_a_copied_source_are_removed(self):
        ev1 = self.root / "evidence" / "ev1"
        put(ev1 / "objects" / "ab" / "obj", "git object")
        read_only(ev1 / "objects" / "ab" / "obj")
        read_only(ev1 / "e.txt")
        other = self.root / "reference" / "ref1" / "r.txt"
        read_only(other)  # another item, not moved in this run: must stay read-only
        self.addCleanup(os.chmod, other, stat.S_IWRITE | stat.S_IREAD)
        self.approve(["evidence"])
        code, out = self.go(*self.YES, "--class", "evidence")
        self.assertEqual(code, 0, out)
        self.assertFalse(ev1.exists(), out)
        dst = self.ext / "evidence" / "evidence" / "ev1"
        self.assertEqual((dst / "objects" / "ab" / "obj").read_text(), "git object")
        self.assertTrue(is_read_only(dst / "e.txt"))  # the copy keeps the attribute
        self.assertTrue(is_read_only(other))


if __name__ == "__main__":
    unittest.main()
