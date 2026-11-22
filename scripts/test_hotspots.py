"""Tests for scripts/hotspots.py: a synthetic board and a throwaway Git repo under a temporary folder.

  python -I -B scripts/test_hotspots.py
"""
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "hotspots.py"
T0 = 1_700_000_000  # window end in the tests
HOUR = 3600

TOML = """root = "."
state_dir = "state"
projects_dir = "projects"
hotspot_lines = {lines}
hotspot_window_hours = 48
hotspot_wait_hours = {wait}
hotspot_claims = {claims}
hotspot_commits = {commits}
"""


def run_git(cwd, *args, when=None):
    env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
    if when is not None:
        env["GIT_AUTHOR_DATE"] = env["GIT_COMMITTER_DATE"] = f"{when} +0000"
    subprocess.run(["git", "-c", "commit.gpgsign=false", *args], cwd=cwd, env=env, check=True, capture_output=True)


class World:
    """A root folder with broadsword.toml, a Git repo on branch main, and a state/tasks board."""

    def __init__(self, tmp, lines=1500, wait=1, claims=5, commits=5):
        self.root = pathlib.Path(tmp)
        (self.root / "broadsword.toml").write_text(TOML.format(lines=lines, wait=wait, claims=claims, commits=commits), encoding="utf-8")
        run_git(self.root, "init", "-q", "-b", "main")
        self.n = 0

    def commit(self, path, text, when):
        p = self.root / path
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8", newline="\n")
        run_git(self.root, "add", path)
        run_git(self.root, "commit", "-q", "-m", f"c{when}", when=when)

    def task(self, tid, claims, created, started=None, ended=None, project="broadsword"):
        d = self.root / "state" / "tasks"
        d.mkdir(parents=True, exist_ok=True)
        lines = [f"id = {tid}", f"project = {project}", "claims = " + "\\n".join(claims), f"created = {created}"]
        if started is not None:
            lines.append(f"started = {started}")
        if ended is not None:
            lines.append(f"ended = {ended}")
        (d / f"{tid}.task").write_text("\n".join(lines) + "\n", encoding="utf-8")

    def report(self, *extra, hours="48"):
        proc = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--root", str(self.root), "--hours", hours, "--until", str(T0), *extra],
                              capture_output=True, text=True, cwd=self.root)
        assert proc.returncode == 0, proc.stderr
        lines = proc.stdout.splitlines()
        assert lines[0] == "project\tpath\tlines\tcommits\tclaims\twaited_tasks\twait_hours\tflag", lines[0]
        rows = {}
        for l in lines[1:-1]:
            f = l.split("\t")
            rows[f[1]] = dict(lines=int(f[2]), commits=int(f[3]), claims=int(f[4]), waited=int(f[5]), wait=float(f[6]), flag=f[7])
        return rows, lines[-1], lines


class HotspotTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.w = World(self._tmp.name)
        self.w.commit("a.rs", "x\n" * 10, T0 - HOUR)
        self.w.commit("b.rs", "x\n" * 10, T0 - HOUR)
        self.w.commit("dir/c.py", "x\n" * 10, T0 - HOUR)
        self.w.commit("broadsword.toml.txt", "ignored suffix\n", T0 - HOUR)

    def test_overlap_attribution(self):
        # B created at T0-10h, started T0-6h (waited 4h); A held a.rs from T0-9h to T0-7h: overlap 2h
        w = self.w
        w.task("A", ["a.rs"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 7 * HOUR)
        w.task("B", ["a.rs", "b.rs"], T0 - 10 * HOUR, T0 - 6 * HOUR, T0 - 5 * HOUR)
        rows, last, _ = w.report()
        self.assertAlmostEqual(rows["a.rs"]["wait"], 2.0, places=2)
        self.assertEqual(rows["a.rs"]["waited"], 1)
        self.assertEqual(rows["a.rs"]["claims"], 2)
        self.assertEqual(rows["b.rs"]["wait"], 0.0)  # nobody else held b.rs
        self.assertIn("board waits 2.0 task-hours from 1 tasks in 48 h", last)

    def test_unended_holder_counts_to_waiter_start(self):
        w = self.w
        w.task("A", ["a.rs"], T0 - 9 * HOUR, T0 - 9 * HOUR)  # never ended
        w.task("B", ["a.rs"], T0 - 8 * HOUR, T0 - 5 * HOUR)
        rows, _, _ = w.report()
        self.assertAlmostEqual(rows["a.rs"]["wait"], 3.0, places=2)

    def test_short_wait_and_unstarted_ignored(self):
        w = self.w
        w.task("A", ["a.rs"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 1 * HOUR)
        w.task("B", ["a.rs"], T0 - 5 * HOUR, T0 - 5 * HOUR + 60)  # waited exactly 60 s: not over 60
        w.task("C", ["a.rs"], T0 - 4 * HOUR)  # never started
        rows, _, _ = w.report()
        self.assertEqual(rows["a.rs"]["wait"], 0.0)
        self.assertEqual(rows["a.rs"]["claims"], 3)

    def test_keys_of_different_keys_do_not_collide(self):
        w = self.w
        w.task("A", ["a.rs#one"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 3 * HOUR)
        w.task("B", ["a.rs#two"], T0 - 8 * HOUR, T0 - 5 * HOUR, T0 - 4 * HOUR)
        rows, _, _ = w.report()
        self.assertEqual(rows["a.rs"]["wait"], 0.0)

    def test_same_key_and_whole_file_collide_with_key(self):
        w = self.w
        w.task("A", ["a.rs#one"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 3 * HOUR)
        w.task("B", ["a.rs#one"], T0 - 8 * HOUR, T0 - 5 * HOUR, T0 - 4 * HOUR)  # overlap with A: 3h
        w.task("C", ["a.rs"], T0 - 8 * HOUR, T0 - 4 * HOUR, T0 - 3 * HOUR)  # whole file vs key: waits on A; overlap 4h (B overlaps 1h)
        rows, _, _ = w.report()
        self.assertAlmostEqual(rows["a.rs"]["wait"], 7.0, places=2)
        self.assertEqual(rows["a.rs"]["waited"], 2)

    def test_folder_claim_covers_child(self):
        w = self.w
        w.task("A", ["dir"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 5 * HOUR)
        w.task("B", ["dir/c.py"], T0 - 8 * HOUR, T0 - 6 * HOUR, T0 - 5 * HOUR)
        rows, _, _ = w.report()
        self.assertAlmostEqual(rows["dir/c.py"]["wait"], 2.0, places=2)
        self.assertEqual(rows["dir/c.py"]["claims"], 2)
        self.assertEqual(rows["a.rs"]["claims"], 0)

    def test_other_project_does_not_collide(self):
        w = self.w
        w.task("A", ["a.rs"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 3 * HOUR, project="other")
        w.task("B", ["a.rs"], T0 - 8 * HOUR, T0 - 5 * HOUR, T0 - 4 * HOUR)
        rows, _, _ = w.report()
        self.assertEqual(rows["a.rs"]["wait"], 0.0)

    def test_window_filters_tasks_and_commits(self):
        # an older commit first: history dates must rise, as in a real repo (git log --since stops at the first old commit)
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        w = World(tmp.name)
        w.commit("old.rs", "x\n", T0 - 100 * HOUR)
        w.commit("a.rs", "x\n" * 10, T0 - HOUR)
        w.task("A", ["a.rs"], T0 - 70 * HOUR, T0 - 70 * HOUR, T0 - 60 * HOUR)  # created before the window
        w.task("B", ["a.rs"], T0 - 65 * HOUR, T0 - 55 * HOUR, T0 - 54 * HOUR)
        rows, last, _ = w.report()
        self.assertNotIn("old.rs", rows)
        self.assertEqual(rows["a.rs"]["claims"], 0)
        self.assertEqual(rows["a.rs"]["commits"], 1)
        self.assertIn("0.0 task-hours from 0 tasks", last)
        wide, _, _ = w.report(hours="200")
        self.assertEqual(wide["a.rs"]["claims"], 2)
        self.assertIn("old.rs", wide)

    def test_flag_rules(self):
        # exact rules, one file each
        cases = [
            # lines, wait_h, claims, commits -> flag, with limits lines 10, wait 2, claims 3, commits 4
            ((11, 2.0, 0, 0), "split"),   # over size and wait at the limit
            ((11, 0.0, 3, 0), "split"),   # over size and claims at the limit
            ((11, 0.0, 0, 4), "split"),   # over size and commits at the limit
            ((11, 1.9, 2, 3), ""),        # over size, nothing at its limit
            ((10, 2.0, 0, 0), "hot"),     # size at (not over) the limit: hot, not split
            ((3, 0.0, 3, 0), "hot"),
            ((3, 0.0, 0, 4), "hot"),
            ((3, 1.0, 1, 1), ""),
        ]
        sys.path.insert(0, str(HERE))
        try:
            import hotspots
        finally:
            sys.path.pop(0)
        cfg = {"hotspot_lines": 10, "hotspot_wait_hours": 2, "hotspot_claims": 3, "hotspot_commits": 4}
        for (lines, wait, claims, commits), want in cases:
            self.assertEqual(hotspots.flag(cfg, lines, wait, claims, commits), want, (lines, wait, claims, commits))

    def test_flag_in_report_and_last_line(self):
        with tempfile.TemporaryDirectory() as tmp:
            x = World(tmp, lines=2, wait=1, claims=50, commits=50)
            x.commit("big.rs", "x\n" * 5, T0 - HOUR)
            x.commit("small.rs", "x\n", T0 - HOUR)
            x.task("A", ["big.rs", "small.rs"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 6 * HOUR)
            x.task("B", ["big.rs", "small.rs"], T0 - 8 * HOUR, T0 - 5 * HOUR, T0 - 4 * HOUR)
            rows, last, lines = x.report()
            self.assertEqual(rows["big.rs"]["flag"], "split")
            self.assertEqual(rows["small.rs"]["flag"], "hot")
            self.assertTrue(last.startswith("hotspots: 1 split, 1 hot; board waits 2.0 task-hours from 1 tasks in 48 h"), last)
            self.assertEqual([l.split("\t")[1] for l in lines[1:-1]][:2], ["big.rs", "small.rs"])  # same wait, claims, commits: lines break the tie

    def test_missing_state_folder_gives_zero_waits(self):
        rows, last, _ = self.w.report()
        self.assertTrue(all(r["wait"] == 0.0 and r["claims"] == 0 for r in rows.values()))
        self.assertIn("0.0 task-hours from 0 tasks in 48 h", last)
        self.assertTrue(last.endswith("0 records skipped"), last)

    def test_unreadable_record_skipped_and_counted(self):
        w = self.w
        w.task("A", ["a.rs"], T0 - 9 * HOUR, T0 - 9 * HOUR, T0 - 7 * HOUR)
        (w.root / "state" / "tasks" / "bad.task").write_text("garbage\ncreated = notanumber\n", encoding="utf-8")
        rows, last, _ = w.report()
        self.assertEqual(rows["a.rs"]["claims"], 1)
        self.assertTrue(last.endswith("1 records skipped"), last)

    def test_write_is_atomic_copy_of_stdout(self):
        w = self.w
        proc = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--root", str(w.root), "--until", str(T0), "--write"], capture_output=True, text=True)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        out = (w.root / "state" / "hotspots.tsv").read_text(encoding="utf-8")
        self.assertEqual(out, proc.stdout)
        self.assertEqual([p.name for p in (w.root / "state").iterdir() if p.suffix == ".tmp"], [])

    def test_bad_setting_refused(self):
        w = self.w
        (w.root / "broadsword.toml").write_text(TOML.format(lines=0, wait=1, claims=5, commits=5), encoding="utf-8")
        proc = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--root", str(w.root)], capture_output=True, text=True)
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("hotspot_lines", proc.stderr)


if __name__ == "__main__":
    unittest.main()
