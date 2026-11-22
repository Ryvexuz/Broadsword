"""Integration tests: all Git writes are confined to temporary repositories."""

from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import runpy


SCRIPT = Path(__file__).with_name("move_check.py").resolve()
CHECK = runpy.run_path(str(SCRIPT))


class ClassifierTests(unittest.TestCase):
    def test_header_like_content_and_multisets(self):
        diff = (
            "diff --git a/a b/b\n--- a/a\n+++ b/b\n"
            "@@ -1,3 +1,3 @@\n--- header\n-same\n-same\n"
            "+++ header\n+same\n+different\n"
        )
        moved, wiring, other, lines = CHECK["classify"](CHECK["parse_diff"](diff))
        self.assertEqual((moved, wiring, other), (1, 0, 4))
        self.assertEqual([line.number for line in lines], [1, 3, 1, 3])

    def test_truncated_and_binary_fail_closed(self):
        for diff in (
            "Binary files a/a and b/a differ\n",
            "--- a/a\n+++ b/a\n@@ -1,2 +0,0 @@\n-one\n",
        ):
            with self.subTest(diff=diff):
                with self.assertRaises(CHECK["CheckError"]):
                    CHECK["parse_diff"](diff)

    def test_large_duplicate_input(self):
        # Multiplicity, not set membership, determines the unmatched count.
        old = ["-same"] * 10000
        new = ["+same"] * 9999
        diff = "\n".join([
            "--- a/a", "+++ b/a", "@@ -1,10000 +1,9999 @@", *old, *new,
        ])
        moved, wiring, other, lines = CHECK["classify"](CHECK["parse_diff"](diff))
        self.assertEqual((moved, wiring, other), (9999, 0, 1))
        self.assertEqual(lines[0].number, 10000)


class MoveCheckTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init")
        self.git("config", "user.email", "test@example.invalid")
        self.git("config", "user.name", "Move Test")
        self.git("config", "core.autocrlf", "false")
        self.write("old.rs", "fn alpha() {}\nfn beta() {}\n")
        self.commit()
        self.git("branch", "trunk")

    def git(self, *args):
        return subprocess.run(
            ["git", "-C", str(self.repo), *args], check=True,
            capture_output=True, text=True,
        ).stdout

    def write(self, path, text):
        (self.repo / path).write_bytes(text.encode("utf-8"))

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-m", "fixture")

    def run_check(self, *extra, limit="0"):
        args = [sys.executable, "-I", str(SCRIPT), "--repo", str(self.repo),
                "--trunk", "trunk"]
        if limit is not None:
            args += ["--max-other", limit]
        return subprocess.run(args + list(extra), capture_output=True, text=True)

    def move(self, text="fn alpha() {}\nfn beta() {}\n"):
        self.write("old.rs", "")
        self.write("new.rs", text)
        self.commit()

    def test_pure_move(self):
        self.move()
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "move_check: ok (moved 2, wiring 0, other 0)\n")

    def test_visibility_and_whitespace(self):
        self.move("  pub(super) fn alpha() {}  \n pub(crate) fn beta() {}\n\n")
        self.assertEqual(self.run_check().stdout,
                         "move_check: ok (moved 2, wiring 0, other 0)\n")

    def test_changed_character(self):
        self.move("fn alphx() {}\nfn beta() {}\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn('- "old.rs":1 "fn alpha() {}"', result.stdout)
        self.assertIn('+ "new.rs":1 "fn alphx() {}"', result.stdout)
        self.assertTrue(result.stdout.endswith(
            "move_check: FAILED (moved 1, wiring 0, other 2, limit 0)\n"))

    def test_reorder(self):
        self.write("old.rs", "fn beta() {}\nfn alpha() {}\n")
        self.commit()
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "move_check: ok (moved 1, wiring 0, other 0)\n")

    def test_empty_and_uncommitted(self):
        self.write("old.rs", "dirty\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "move_check: FAILED (empty diff)\n")

    def test_wiring_budget(self):
        self.move("fn alpha() {}\nfn beta() {}\nmod a;\nuse b;\n")
        result = self.run_check(limit="1")
        self.assertEqual(result.returncode, 1)
        self.assertTrue(result.stdout.endswith(
            "move_check: FAILED (moved 2, wiring 2, other 0, limit 1)\n"))
        self.assertEqual(self.run_check(limit="2").returncode, 0)

    def test_missing_and_invalid_budget(self):
        for limit in (None, "-1", "nan", "1.5", ""):
            with self.subTest(limit=limit):
                result = self.run_check(limit=limit)
                self.assertEqual(result.returncode, 2)
                self.assertEqual(result.stdout, "")
                self.assertEqual(len(result.stderr.splitlines()), 1)
                self.assertIn("--max-other", result.stderr)

    def test_git_error(self):
        result = self.run_check("--trunk", "missing-branch")
        self.assertEqual(result.returncode, 2)
        self.assertEqual(len(result.stderr.splitlines()), 1)

    def test_multiset_and_quoted_path(self):
        self.move("fn alpha() {}\nfn beta() {}\nfn beta() {}\n")
        self.write("space ü.rs", "mod extra;\n")
        self.commit()
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn('+ "new.rs":3 "fn beta() {}"', result.stdout)
        self.assertIn('space \\u00fc.rs', result.stdout)
        self.assertTrue(result.stdout.endswith(
            "move_check: FAILED (moved 2, wiring 1, other 1, limit 0)\n"))


if __name__ == "__main__":
    unittest.main()
