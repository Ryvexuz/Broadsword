"""Start-gate behaviour against isolated copies of the real lint and preflight."""

import sys

sys.dont_write_bytecode = True

import importlib.util
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("start_gate_under_test", HERE / "start_gate.py")
sg = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(sg)

BRIEF = (
    "## Requirements\nUpdate the notes.\n"
    "## Claims\nnotes.md\n"
    "## Done check\n- `broadsword test broadsword` succeeds.\n"
)


class StartGateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="test-start-gate-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        (self.root / "scripts").mkdir()
        for name in ("brief_lint.py", "brief_preflight.py"):
            shutil.copy(HERE.parent / "scripts" / name, self.root / "scripts" / name)
        self.brief = self.root / "brief.md"
        self.brief.write_text(BRIEF, encoding="utf-8")

    def check(self, text=BRIEF, board=("notes.md",), max_kb=4):
        self.brief.write_text(text, encoding="utf-8")
        return sg.check(self.root, "broadsword", self.brief, board, max_kb)

    def test_lint_fail_refuses_with_rule_name(self):
        pointer = "see " + "scratch" + "/x.md"
        problems = self.check(BRIEF.replace("Update the notes.", pointer))
        self.assertTrue(any(p.startswith("FAIL brief-B1:") for p in problems), problems)

    def test_passing_brief_and_claims_pass(self):
        self.assertEqual(self.check(), [])

    def test_key_claim_needs_same_key_or_whole_file(self):
        text = BRIEF.replace("notes.md", "broadsword.toml#a_key")
        self.assertEqual(sg.brief_claims(self.root, text), ["broadsword.toml#a_key"])
        for board in (["broadsword.toml#a_key"], ["broadsword.toml"]):
            with self.subTest(board=board):
                self.assertEqual(sg.claim_problems(self.root, text, board), [])
        self.assertEqual(
            sg.claim_problems(self.root, text, ["broadsword.toml#other_key"]),
            ["claims: not on the board claims: broadsword.toml#a_key"],
        )

    def test_whole_file_brief_claim_with_key_only_board_fails(self):
        text = BRIEF.replace("notes.md", "broadsword.toml")
        self.assertEqual(
            sg.claim_problems(self.root, text, ["broadsword.toml#a_key"]),
            ["claims: not on the board claims: broadsword.toml"],
        )

    def test_parenthetical_note_and_new_marker_are_not_claims(self):
        text = BRIEF.replace(
            "notes.md",
            "(the only files you may change) clients/a.py (new), "
            "clients/b.py (wiring: the call in clients/c.py (nested clients/d.py))",
        )
        self.assertEqual(sg.brief_claims(self.root, text), ["clients/a.py", "clients/b.py"])
        self.assertEqual(sg.claim_problems(self.root, text, ["clients/a.py", "clients/b.py"]), [])

    def test_folder_board_claim_covers_files_under_it(self):
        text = BRIEF.replace("notes.md", "clients/a.py")
        self.assertEqual(sg.claim_problems(self.root, text, ["clients/"]), [])
        text = BRIEF.replace("notes.md", "clients/")
        self.assertEqual(
            sg.claim_problems(self.root, text, ["clients/a.py"]),
            ["claims: not on the board claims: clients/"],
        )
        self.assertEqual(sg.claim_problems(self.root, text, ["clients"]), [])

    def test_missing_brief_lint_refuses(self):
        missing = self.root / "missing"
        with mock.patch.object(sg.subprocess, "run") as child:
            self.assertEqual(
                sg.lint_problems(missing, self.root, self.brief, 4),
                [f"brief_lint not found: {missing / 'scripts' / 'brief_lint.py'}"],
            )
        child.assert_not_called()

    def test_no_claims_refuses(self):
        problems = self.check(BRIEF.replace("## Claims\nnotes.md\n", ""))
        self.assertIn("claims: the brief's Claims line names no path", problems)
        self.assertTrue(any(p.startswith("FAIL files:") for p in problems), problems)

    def test_claim_token_grammar_and_order(self):
        text = (
            "## Claims\n[`clients\\a.py`, 'notes.md':]; \"clients/b.py\".\n"
            "notes.md, broadsword.toml#a_key; broadsword.toml#bad-key\n"
            "broadsword.toml#; broadsword.toml#a#b\n"
            "## Requirements\nDo the work.\n"
            "**Claims:** clients/c.py\n"
        )
        self.assertEqual(
            sg.brief_claims(self.root, text),
            ["clients/a.py", "notes.md", "clients/b.py", "broadsword.toml#a_key", "clients/c.py"],
        )

    def test_all_uncovered_claims_are_reported_in_order(self):
        text = BRIEF.replace("notes.md", "clients/a.py, clients/ab.py, notes.md")
        self.assertEqual(
            sg.claim_problems(self.root, text, ["clients/a"]),
            ["claims: not on the board claims: clients/a.py, clients/ab.py, notes.md"],
        )
        self.assertEqual(sg.claim_problems(self.root, text, ["*"]), [])

    def test_coverage_windows_casefold_and_separators(self):
        with mock.patch.object(sg.os, "name", "nt"):
            self.assertTrue(sg._covers("CLIENTS\\", "clients/a.py"))
            self.assertTrue(sg._covers("BROADSWORD.TOML#A_KEY", "broadsword.toml#a_key"))
            self.assertFalse(sg._covers("BROADSWORD.TOML#A_KEY", "broadsword.toml"))
        with mock.patch.object(sg.os, "name", "posix"):
            self.assertFalse(sg._covers("CLIENTS/", "clients/a.py"))

    def test_lint_command_and_output_limits(self):
        output = "  PASS lines: fine\nFAIL unindented: ignore\n"
        output += "\n".join(f"  FAIL rule-{i}: " + "x" * 100000 for i in range(12))
        result = subprocess.CompletedProcess([], 1, output, "")
        with mock.patch.object(sg.subprocess, "run", return_value=result) as child:
            problems = sg.lint_problems(self.root, self.root, self.brief, 7)
        self.assertEqual(problems, [f"FAIL rule-{i}: " + "x" * (300 - len(f"FAIL rule-{i}: ")) for i in range(8)])
        child.assert_called_once_with(
            [
                sys.executable, "-I", "-B", str(self.root / "scripts" / "brief_lint.py"),
                "--max-lines", "400", "--max-bullets", "80", "--max-files", "80",
                "--max-done-lines", "10", "--big-file-lines", "1000", "--gpt-preflight",
                "--size-pct", "90", "--cap-margin-pct", "5", "--brief-max-kb", "7",
                "--root", str(self.root), str(self.brief),
            ],
            cwd=self.root, capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=120,
        )

    def test_lint_exit_handling_fails_closed(self):
        cases = [
            (0, "  FAIL ignored-on-success", "", []),
            (1, "", "", ["brief_lint failed with no FAIL line"]),
            (1, "PASS only", "  FAIL stderr-rule: bad", ["FAIL stderr-rule: bad"]),
            (2, "", "usage error", ["brief_lint could not run: exit 2"]),
            (-1, "", "", ["brief_lint could not run: exit -1"]),
        ]
        for code, stdout, stderr, expected in cases:
            with self.subTest(code=code, stdout=stdout, stderr=stderr):
                result = subprocess.CompletedProcess([], code, stdout, stderr)
                with mock.patch.object(sg.subprocess, "run", return_value=result):
                    self.assertEqual(sg.lint_problems(self.root, self.root, self.brief, 4), expected)
        for error in (subprocess.TimeoutExpired("lint", 120), OSError("unavailable")):
            with self.subTest(error=type(error).__name__):
                with mock.patch.object(sg.subprocess, "run", side_effect=error):
                    problems = sg.lint_problems(self.root, self.root, self.brief, 4)
                self.assertEqual(len(problems), 1)
                self.assertTrue(problems[0].startswith("brief_lint could not run:"), problems)

    def test_missing_preflight_refuses(self):
        (self.root / "scripts" / "brief_preflight.py").unlink()
        problems = self.check()
        self.assertIn("brief_lint failed with no FAIL line", problems)
        self.assertIn("claims: could not load brief_lint: FileNotFoundError", problems)

    def test_unreadable_brief_refuses_without_child(self):
        path = self.root / "absent.md"
        with mock.patch.object(sg.subprocess, "run") as child:
            self.assertEqual(
                sg.check(self.root, "broadsword", path, ["notes.md"], 4),
                [f"cannot read the brief {path}: FileNotFoundError"],
            )
        child.assert_not_called()

    def test_project_root_selection(self):
        for project, expected in (
            ("broadsword", self.root),
            ("other-project", self.root / "projects" / "other-project"),
            ("other.project", self.root / "projects" / "other.project"),
        ):
            with self.subTest(project=project):
                with mock.patch.object(sg, "lint_problems", return_value=[]) as lint:
                    self.assertEqual(sg.check(self.root, project, self.brief, ["notes.md"], 4), [])
                lint.assert_called_once_with(self.root, expected, self.brief, 4)

    def test_invalid_project_and_size_refuse_without_child(self):
        for project in ("", ".", "..", "../elsewhere", "C:\\elsewhere", "bad/name", "bad\rname"):
            with self.subTest(project=project):
                with mock.patch.object(sg.subprocess, "run") as child:
                    self.assertEqual(
                        sg.check(self.root, project, self.brief, ["notes.md"], 4),
                        ["brief_lint could not run: invalid project name"],
                    )
                child.assert_not_called()
        for value in (0, -1, True, "", float("nan"), float("inf")):
            with self.subTest(max_kb=value):
                with mock.patch.object(sg.subprocess, "run") as child:
                    self.assertEqual(
                        sg.lint_problems(self.root, self.root, self.brief, value),
                        ["brief_lint could not run: max_kb must be a positive integer"],
                    )
                child.assert_not_called()

    def test_brief_size_uses_runner_setting_and_preserves_bytes(self):
        text = BRIEF.replace("Update the notes.", "x" * 1000)
        self.assertEqual(self.check(text, max_kb=4), [])
        before = self.brief.read_bytes()
        problems = sg.check(self.root, "broadsword", self.brief, ["notes.md"], 1)
        self.assertTrue(any(p.startswith("FAIL brief-B2:") for p in problems), problems)
        self.assertEqual(self.brief.read_bytes(), before)


if __name__ == "__main__":
    unittest.main(verbosity=2)
