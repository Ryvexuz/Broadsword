"""Behavioural checks for the report, using real disposable Git repositories."""
from datetime import datetime, timezone
import os
from pathlib import Path
import shutil
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch


SCRIPT = Path(__file__).with_name("stale_claims.py").resolve()
HEADER = "id\tproject\towner\tidle_hours\tlast_activity_utc\tclaims\tnote"
START = 1700000000
HOURS = 4
NOW = START + HOURS * 3600


class StaleClaimsTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp()).resolve()
        self.addCleanup(self.cleanup_root)
        self.env = dict(os.environ, GIT_AUTHOR_DATE=f"@{START} +0000",
                        GIT_COMMITTER_DATE=f"@{START} +0000", GIT_CONFIG_NOSYSTEM="1",
                        GIT_CONFIG_GLOBAL=os.devnull)
        self.git("init")
        (self.root / ".gitignore").write_text(
            "state/\nworktrees/\nprojects/\nbroadsword.toml\nignored.txt\n", encoding="utf-8")
        (self.root / "tracked.txt").write_text("initial\n", encoding="utf-8")
        self.git("add", ".")
        self.git("commit", "-m", "initial")
        self.git("branch", "task/held")
        self.worktree = self.root / "worktrees" / "held"
        self.git("worktree", "add", str(self.worktree), "task/held")
        self.tasks = self.root / "state" / "tasks"
        self.tasks.mkdir(parents=True)
        # Model a task checkout separately from the main checkout's board.
        self.checkout = self.root / "reporter"
        (self.checkout / "scripts").mkdir(parents=True)
        self.script = self.checkout / "scripts" / SCRIPT.name
        shutil.copyfile(SCRIPT, self.script)
        self.config()
        self.record()

    def cleanup_root(self):
        # Git objects can be read-only on Windows. Retry only inside this temp root.
        def retry(function, path, error):
            if not Path(path).resolve().is_relative_to(self.root):
                raise ValueError(f"cleanup escaped temporary root: {path}") from error[1]
            os.chmod(path, 0o700)
            function(path)

        shutil.rmtree(self.root, onerror=retry)

    def git(self, *args, cwd=None, epoch=None):
        env = self.env.copy()
        if epoch is not None:
            env.update(GIT_AUTHOR_DATE=f"@{epoch} +0000", GIT_COMMITTER_DATE=f"@{epoch} +0000")
        return subprocess.run(
            ["git", "-C", str(cwd or self.root), "-c", "user.name=Test",
             "-c", "user.email=test@example.invalid", *args],
            capture_output=True, check=True, env=env)

    def config(self, value="4"):
        text = 'state_dir = "state"\nprojects_dir = "projects"\n'
        if value is not None:
            text += f"stale_claim_hours = {value}\n"
        (self.checkout / "broadsword.toml").write_text(text, encoding="utf-8")

    def record(self, name="held", **overrides):
        fields = dict(id=name, project="broadsword", owner="coder-codex", status="running",
                      started=str(START), branch="task/held", worktree="worktrees/held",
                      claims=r"src/held.rs\nsrc/other.rs")
        fields.update(overrides)
        path = self.tasks / f"{name}.task"
        path.write_bytes("".join(f"{k} = {v}\r\n" for k, v in fields.items()).encode())
        return path

    def run_report(self, now=NOW):
        return subprocess.run(
            [sys.executable, "-I", str(self.script), "--root", str(self.root), "--now", str(now)],
            capture_output=True, text=True, env=self.env)

    def summary(self, stale=1, running=1, skipped=0):
        return (f"stale claims: {stale} of {running} running tasks idle >= {HOURS} h; "
                f"{skipped} records skipped")

    def row(self, name="held", hours="4.0", epoch=START, note=""):
        utc = datetime.fromtimestamp(epoch, timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
        return f"{name}\tbroadsword\tcoder-codex\t{hours}\t{utc}\tsrc/held.rs,src/other.rs\t{note}"

    def assert_report(self, lines):
        proc = self.run_report()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(proc.stdout, "\n".join(lines) + "\n")
        self.assertEqual(proc.stderr, "")

    def test_old_branch_and_clean_worktree_is_stale(self):
        # Exactly the threshold: changing >= to > must remove this row and fail.
        self.assert_report([HEADER, self.row(), self.summary()])

    def test_recent_commit_is_not_stale(self):
        # The committer timestamp, not the author timestamp, is activity.
        self.env["GIT_COMMITTER_DATE"] = f"@{NOW - 60} +0000"
        self.git("commit", "--allow-empty", "-m", "recent", cwd=self.worktree)
        self.assert_report([HEADER, self.summary(stale=0)])

    def test_recent_worktree_edit_is_not_stale(self):
        for name in ("tracked.txt", "untracked with spaces.txt", 'quote"tab\tfile.txt',
                     "back\\slash.txt", "line\nbreak.txt"):
            # Windows forbids double quotes and tabs in filenames.
            if os.name == "nt" and any(c in name for c in '"\t\\\n'):
                continue
            with self.subTest(name=name):
                path = self.worktree / name
                previous = path.read_bytes() if path.exists() else None
                path.write_text("recent edit\n", encoding="utf-8")
                os.utime(path, (NOW - 60, NOW - 60))
                self.assert_report([HEADER, self.summary(stale=0)])
                if previous is None:
                    path.unlink()
                else:
                    path.write_bytes(previous)

    def test_missing_worktree_and_branch_use_started(self):
        self.record(branch="", worktree="worktrees/missing")
        self.assert_report([HEADER, self.row(), self.summary()])
        self.record(branch="task/missing", worktree="worktrees/missing")
        error = subprocess.run(
            ["git", "-C", str(self.root), "log", "-1", "--format=%ct",
             "--end-of-options", "task/missing", "--"],
            capture_output=True, text=True, env=self.env).stderr.splitlines()[0]
        # Escape the same TSV metacharacters, independently of the reporter.
        error = error.replace("\\", "\\\\").replace("\t", "\\t")
        self.assert_report([HEADER, self.row(note="git: " + error), self.summary()])

    def test_corrupt_record_is_skipped_and_counted(self):
        path = self.record("broken", started="not-an-integer")
        escaped = str(path).replace("\\", "\\\\")
        self.assert_report([
            HEADER, f"skipped {escaped}: started: required integer Unix seconds",
            self.row(), self.summary(skipped=1)])

    def test_unconvertible_timestamp_is_skipped_and_counted(self):
        # Force the Windows conversion failure on every platform, without
        # narrowing the accepted Unix timestamp range.
        namespace = runpy.run_path(str(self.script))
        report = namespace["report"]
        latest = namespace["MAX_TIMESTAMP"]
        path = self.record("unsupported", started=str(latest - HOURS * 3600),
                           branch="", worktree="")
        escaped = str(path).replace("\\", "\\\\")
        for error in (OSError, OverflowError, ValueError):
            with self.subTest(error=error.__name__):
                def convert(value, zone):
                    if value == latest - HOURS * 3600:
                        raise error("platform timestamp conversion failed")
                    return datetime.fromtimestamp(value, zone)

                with patch.dict(report.__globals__, datetime=Mock(fromtimestamp=convert)):
                    lines = report(
                        self.root, namespace["settings"](self.checkout),
                        namespace["timestamp"](latest))
                self.assertEqual(lines, [
                    HEADER,
                    f"skipped {escaped}: last_activity_utc: platform timestamp conversion failed",
                    self.row(hours=f"{(latest - START) / 3600:.1f}"),
                    self.summary(running=2, skipped=1)])

    def test_missing_or_zero_setting_exits_2(self):
        for value in (None, "0", "-1", "true", '"4"', "4.0"):
            with self.subTest(value=value):
                self.config(value)
                proc = self.run_report()
                self.assertEqual(proc.returncode, 2)
                self.assertIn("stale_claim_hours", proc.stderr)
                self.assertEqual(proc.stdout, "")

    def test_settings_come_from_script_checkout(self):
        # The main checkout may not yet have the task checkout's new key.
        (self.root / "broadsword.toml").write_text(
            'state_dir = "wrong-state"\nprojects_dir = "wrong-projects"\n',
            encoding="utf-8")
        self.assert_report([HEADER, self.row(), self.summary()])

    def test_report_writes_nothing(self):
        path = self.worktree / "tracked.txt"
        path.write_text("dirty index must not refresh\n", encoding="utf-8")
        os.utime(path, (START, START))

        def snapshot():
            return {p.relative_to(self.root): (p.stat().st_mtime_ns, p.stat().st_size)
                    for p in self.root.rglob("*") if p.is_file()}

        before = snapshot()
        self.assertTrue(before)
        self.assert_report([HEADER, self.row(), self.summary()])
        self.assertEqual(snapshot(), before)

    def test_ignored_and_deleted_paths_do_not_count(self):
        ignored = self.worktree / "ignored.txt"
        ignored.write_text("ignored\n", encoding="utf-8")
        os.utime(ignored, (NOW, NOW))
        (self.worktree / "tracked.txt").unlink()
        self.assert_report([HEADER, self.row(), self.summary()])

    def test_renamed_path_and_external_project(self):
        project = self.root / "projects" / "other"
        project.parent.mkdir()
        self.git("init", str(project))
        (project / "old.txt").write_text("old\n", encoding="utf-8")
        self.git("add", ".", cwd=project)
        self.git("commit", "-m", "other", cwd=project)
        self.git("branch", "task/other", cwd=project)
        self.record(project="other", branch="task/other", worktree="projects/other")
        self.git("mv", "old.txt", "new name.txt", cwd=project)
        os.utime(project / "new name.txt", (NOW - 60, NOW - 60))
        self.assert_report([HEADER, self.summary(stale=0)])

    def test_sorting_and_nonrunning_tasks(self):
        self.record("older", started=str(START - 3600), branch="", worktree="")
        self.record("done", status="done", started="")
        self.assert_report([HEADER, self.row("older", hours="5.0", epoch=START - 3600),
                            self.row(), self.summary(stale=2, running=2)])

    def test_invalid_now_and_record_fields(self):
        for now in ("NaN", "Infinity", "-1", "1e1000", "garbage"):
            with self.subTest(now=now):
                proc = self.run_report(now)
                self.assertEqual(proc.returncode, 2, proc.stderr)
                self.assertIn("timestamp", proc.stderr)
        self.record(worktree="../escape")
        self.assert_report([HEADER, self.row(note="git: worktree: path must stay inside root"),
                            self.summary()])

    def test_default_root_is_main_checkout(self):
        proc = subprocess.run(
            [sys.executable, "-I", str(self.script), "--now", str(NOW)],
            cwd=self.worktree, capture_output=True, text=True, env=self.env)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(proc.stdout, "\n".join([HEADER, self.row(), self.summary()]) + "\n")

    def test_corrupt_formats_and_timestamp_bounds(self):
        path = self.tasks / "broken.task"
        for content in (b"\xff", b"status = running\n", b"status = running\nstarted = -1\n",
                        b"status = running\nstarted = 999999999999999999999\n",
                        b"status = running\nstatus = done\nstarted = 1\n"):
            with self.subTest(content=content):
                path.write_bytes(content)
                proc = self.run_report()
                self.assertEqual(proc.returncode, 0, proc.stderr)
                lines = proc.stdout.splitlines()
                self.assertEqual(lines[0], HEADER)
                self.assertTrue(lines[1].startswith("skipped "))
                self.assertEqual(lines[2:], [self.row(), self.summary(skipped=1)])

    def test_fractional_boundary_is_not_rounded_before_judgment(self):
        proc = self.run_report(f"{NOW - 1}.999999999")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(proc.stdout, "\n".join([HEADER, self.summary(stale=0)]) + "\n")


if __name__ == "__main__":
    unittest.main(verbosity=2)
