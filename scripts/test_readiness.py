"""Readiness rule and batch gatherer tests (no real board or lint needed)."""
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("readiness", HERE / "readiness.py")
readiness = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(readiness)

OWN = "t-20261126-002533-2"
DEP = "t-20261125-000000"
BRIEF = "**Prerequisites:** none.\n"
FAKE_LINT = """import pathlib
import sys
args = sys.argv[1:]
root = pathlib.Path(args[args.index("--root") + 1])
with (root / "calls").open("a", encoding="utf-8") as stream:
    stream.write("called\\n")
mode = args[args.index("--mode") + 1]
paths = args[args.index("--root") + 2:]
for index, path in enumerate(paths):
    if mode != "missing":
        print(path + (": ok" if mode != "fail" and not (mode == "mixed" and index) else ": failed"))
    if mode == "duplicate":
        print(path + ": failed")
sys.exit(2 if mode == "crash" else 1 if mode in ("fail", "mixed") else 0)
"""


def row(task_id=OWN, claim="src/a.rs", project="p"):
    return [task_id, "open", "coder", project, "title", claim]


class RuleTests(unittest.TestCase):
    def test_missing_brief_not_startable(self):
        self.assertEqual(readiness.check(row(), None, {}, [], True), (False, "no brief"))

    def test_prerequisite_not_done_blocks(self):
        for status in ("open", "running", "unknown", "Done", ""):
            with self.subTest(status=status):
                self.assertEqual(
                    readiness.check(row(), f"Prerequisites: {DEP}", {DEP: status}, [], True),
                    (False, f"prerequisite {DEP} is {status}"),
                )
        self.assertEqual(
            readiness.check(row(), f"Depends on: {DEP}", {}, [], True),
            (False, f"prerequisite {DEP} is unknown"),
        )

    def test_prereq_line_without_id_blocks(self):
        for value in ("", "later", "not needed", "nonetheless"):
            self.assertFalse(readiness.check(row(), "Prerequisites: " + value, {}, [], True)[0])

    def test_all_prerequisite_lines_and_self_id(self):
        brief = f"- **Prerequisites:** {OWN}\r\n**Depends on:** {DEP}\n"
        self.assertEqual(readiness.check(row(), brief, {DEP: "done"}, [], True), (True, "ok"))
        self.assertFalse(readiness.check(row(), brief + "Prerequisites: later", {DEP: "done"}, [], True)[0])
        self.assertFalse(readiness.check(row(), "Task with no prerequisites field", {}, [], True)[0])
        for value in ("none, blocked", "none; waits on review", "none until Lazlo", f"{DEP} blocked"):
            self.assertEqual(
                readiness.check(row(), "Prerequisites: " + value, {DEP: "done"}, [], True),
                (False, "prerequisites blocked"),
            )

    def test_first_failure_order(self):
        held = [("p", "*")]
        self.assertEqual(readiness.check(row(), None, {}, held, False), (False, "no brief"))
        self.assertEqual(
            readiness.check(row(), "", {}, held, False), (False, "no Prerequisites line"),
        )
        self.assertTrue(readiness.check(row(), BRIEF, {}, held, False)[1].startswith("claim "))

    def test_lint_fail_is_never_startable(self):
        self.assertEqual(
            readiness.check(row(), BRIEF, {}, [], False), (False, "brief_lint not ok"),
        )

    def brief_claim_missing_from_board_is_not_startable(self):
        brief = BRIEF + "**Claims:** `src/a.rs`, `src/b.rs`\n"
        self.assertEqual(
            readiness.check(row(), brief, {}, [], True),
            (False, "brief claims not on the board: src/b.rs"),
        )

    # unittest discovers test_* methods; retain the amendment's exact name too.
    test_brief_claim_missing_from_board_is_not_startable = (
        brief_claim_missing_from_board_is_not_startable
    )

    def standard_claims_form_checks_the_first_path(self):
        brief = BRIEF + (
            "**Claims:** (the only files you may change) src/zzz.rs, src/a.rs (new)\n"
        )
        self.assertEqual(
            readiness.check(row(), brief, {}, [], True),
            (False, "brief claims not on the board: src/zzz.rs"),
        )
        self.assertEqual(
            readiness.check(row(claim="src/zzz.rs, src/a.rs"), brief, {}, [], True),
            (True, "ok"),
        )

    test_standard_claims_form_checks_the_first_path = (
        standard_claims_form_checks_the_first_path
    )

    def test_brief_claims_require_exact_board_membership(self):
        for board, claimed in (
            ("src", "src/a.rs"), ("*", "src/a.rs"),
            ("src/a.rs", "src/a.rs#key"), ("src/a.rs#key", "src/a.rs"),
            ("src/a.rs#one", "src/a.rs#two"),
            ("src/a.rs", r"src\a.rs"), ("src/a.rs", r"C:\src\a.rs"),
        ):
            with self.subTest(board=board, claimed=claimed):
                self.assertEqual(
                    readiness.check(row(claim=board), BRIEF + "Claims: " + claimed, {}, [], True),
                    (False, "brief claims not on the board: " + claimed),
                )

    def test_brief_claims_exact_matches_and_annotations(self):
        brief = BRIEF + (
            "- **Claims:** `src/a.rs` (existing), `config.toml#key` (new)\n"
            "Claims: src/a.rs\n"
        )
        self.assertEqual(
            readiness.check(row(claim="src/a.rs, config.toml#key"), brief, {}, [], True),
            (True, "ok"),
        )
        self.assertEqual(
            readiness.check(row(), brief, {}, [], True),
            (False, "brief claims not on the board: config.toml#key"),
        )

    def test_claims_with_comma_inside_parentheses_are_one_claim(self):
        brief = BRIEF + "Claims: first.rs (new, generated), second.rs\n"
        self.assertEqual(
            readiness.check(row(claim="first.rs, second.rs"), brief, {}, [], True),
            (True, "ok"),
        )

    def test_nested_parentheses_give_no_claim(self):
        for annotation in ("(a (b), c)", "(a, (b, (c)), d)"):
            with self.subTest(annotation=annotation):
                brief = BRIEF + f"Claims: {annotation} src/a.rs\n"
                self.assertEqual(
                    readiness.check(row(), brief, {}, [], True), (True, "ok"),
                )

    def test_backticked_key_claim_keeps_exact_text(self):
        brief = BRIEF + "Claims: `a.rs#k` (key only, existing)\n"
        self.assertEqual(
            readiness.check(row(claim="a.rs#k"), brief, {}, [], True), (True, "ok"),
        )
        for board in ("a.rs", "a.rs#other", "*"):
            with self.subTest(board=board):
                self.assertEqual(
                    readiness.check(row(claim=board), brief, {}, [], True),
                    (False, "brief claims not on the board: a.rs#k"),
                )

    def test_missing_claim_still_reported_after_annotation(self):
        brief = BRIEF + "Claims: src/a.rs (new, generated), missing.rs\n"
        self.assertEqual(
            readiness.check(row(), brief, {}, [], True),
            (False, "brief claims not on the board: missing.rs"),
        )

    def test_all_brief_claim_lines_checked_and_missing_paths_reported(self):
        brief = BRIEF + (
            "**Claims:** src/b.rs, src/c.rs\n"
            "Claims: src/b.rs, config.toml#key\n"
        )
        self.assertEqual(
            readiness.check(row(), brief, {}, [], True),
            (False, "brief claims not on the board: src/b.rs, src/c.rs, config.toml#key"),
        )
        self.assertEqual(
            readiness.check(row(), brief, {}, [], False), (False, "brief_lint not ok"),
        )

    def test_claim_overlap_board_rule(self):
        for a, b, expected in (
            ("*", "x", True), ("SRC/A.rs", "src/a.RS#key", True),
            ("file#one", "FILE#ONE", True), ("file#one", "file#two", False),
            ("src", "src/a.rs", True), ("src", "src2/a.rs", False),
            ("src#key", "src/a.rs", False), ("src/a", "src/ab", False),
        ):
            with self.subTest(a=a, b=b):
                self.assertEqual(readiness.claims_overlap(a, b), expected)
                self.assertEqual(readiness.claims_overlap(b, a), expected)
                self.assertEqual(
                    readiness.check(row(claim=a), BRIEF, {}, [("p", b)], True)[0],
                    not expected,
                )
        self.assertTrue(readiness.check(row(), BRIEF, {}, [("other", "*")], True)[0])

    def test_no_model_line_routes_gpt_high(self):
        self.assertEqual(readiness.model_route(BRIEF), ("gpt", "high"))

    def test_model_routes(self):
        for line, expected in (
            ("- Model: gpt-low", ("gpt", "low")),
            ("**Model:** GPT high", ("gpt", "high")),
            ("- **Model:** gpt low effort", ("gpt", "low")),
            ("Model: gpt slowly", ("gpt", "high")),
            ("Model: gpt", ("gpt", "high")),
            ("Model: opus", ("other", "opus")),
            ("Model: sonnet", ("other", "sonnet")),
            ("Model: haiku", ("other", "haiku")),
            ("Model: fable", ("other", "fable")),
        ):
            with self.subTest(line=line):
                self.assertEqual(readiness.model_route(line), expected)

    def test_invalid_rows_rejected_before_path_use(self):
        for task_id in ("../escape", r"C:\escape", "t-20261126-002533-2\r", ""):
            with self.subTest(task_id=task_id), self.assertRaisesRegex(ValueError, "invalid board row"):
                readiness.check(row(task_id), BRIEF, {}, [], True)


class GatherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        (self.root / "scripts").mkdir()
        (self.root / "scripts" / "brief_lint.py").write_text(FAKE_LINT, encoding="utf-8")
        self.backlog = self.root / "state" / "backlog"
        self.backlog.mkdir(parents=True)
        self.write_brief(OWN)
        self.configure("ok")

    def configure(self, mode):
        (self.root / "broadsword.toml").write_text(
            f'ready_lint_args = "--mode {mode}"\n', encoding="utf-8",
        )

    def write_brief(self, task_id, text=BRIEF):
        (self.backlog / f"{task_id}.md").write_text(text, encoding="utf-8")

    def gather(self, rows=None, running=()):
        return readiness.startable(self.root, running, rows or [row()], {})

    def test_lint_fail_is_never_startable(self):
        self.configure("fail")
        self.assertEqual(self.gather()[0][1:], (False, "brief_lint not ok"))

    def test_brief_claim_missing_from_board_is_not_startable(self):
        self.write_brief(OWN, BRIEF + "**Claims:** src/a.rs, src/b.rs\n")
        self.assertEqual(
            self.gather()[0][1:], (False, "brief claims not on the board: src/b.rs"),
        )
        self.assertEqual(
            self.gather([row(claim="src/a.rs, src/b.rs")])[0][1:], (True, "ok"),
        )

    def test_lint_crash_marks_all_not_startable(self):
        self.write_brief(DEP)
        self.configure("crash")
        results = self.gather([row(), row(DEP)])
        self.assertEqual(len(results), 2)
        for _, ok, reason in results:
            self.assertFalse(ok)
            self.assertIn("brief_lint exited 2", reason)

    def test_missing_or_duplicate_output_fails_closed(self):
        for mode in ("missing", "duplicate"):
            with self.subTest(mode=mode):
                self.configure(mode)
                self.assertFalse(self.gather()[0][1])

    def test_batch_once_and_individual_results_on_exit_one(self):
        self.write_brief(DEP)
        self.configure("mixed")
        results = self.gather([row(), row(DEP)])
        self.assertEqual([ok for _, ok, _ in results], [True, False])
        self.assertEqual((self.root / "calls").read_text(encoding="utf-8"), "called\n")

    def test_missing_brief_not_startable(self):
        results = self.gather([row(DEP)])
        self.assertEqual(results[0][1:], (False, "no brief"))
        self.assertFalse((self.root / "calls").exists())

    def test_empty_candidates_do_not_spawn(self):
        self.assertEqual(readiness.startable(self.root, [], [], {}), [])
        self.assertFalse((self.root / "calls").exists())

    def test_running_claims_are_applied(self):
        self.assertFalse(self.gather(running=[row(DEP, "*")])[0][1])
        self.assertTrue(self.gather(running=[row(DEP, "*", "other")])[0][1])

    def test_lint_script_missing_fails_closed(self):
        (self.root / "scripts" / "brief_lint.py").unlink()
        self.assertFalse(self.gather()[0][1])

    def test_prerequisite_failure_precedes_lint_failure(self):
        self.write_brief(OWN, f"Prerequisites: {DEP}")
        self.configure("crash")
        self.assertEqual(self.gather()[0][1:], (False, f"prerequisite {DEP} is unknown"))

    def test_missing_lint_key_exits_nonzero(self):
        (self.root / "broadsword.toml").write_text("", encoding="utf-8")
        code = (
            "import runpy, sys; "
            "m = runpy.run_path(sys.argv[1]); "
            "m['startable'](sys.argv[2], [], [], {})"
        )
        proc = subprocess.run(
            [sys.executable, "-I", "-B", "-c", code, str(HERE / "readiness.py"), str(self.root)],
            capture_output=True, text=True,
        )
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("ready_lint_args", proc.stderr)

    def test_invalid_lint_configuration_fails_loudly(self):
        for value in ('""', "12", '"unterminated \\"quote"'):
            with self.subTest(value=value):
                (self.root / "broadsword.toml").write_text(
                    "ready_lint_args = " + value, encoding="utf-8",
                )
                with self.assertRaisesRegex(SystemExit, "ready_lint_args"):
                    self.gather()


if __name__ == "__main__":
    unittest.main(verbosity=2)
