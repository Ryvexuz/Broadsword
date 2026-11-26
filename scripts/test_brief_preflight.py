"""Behavioural coverage for optional B rules and their CLI wiring."""
import contextlib
import importlib.util
import io
import pathlib
import sys
import tempfile
import types
import unittest

sys.dont_write_bytecode = True
HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("brief_lint", HERE / "brief_lint.py")
bl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bl)


class PreflightTests(unittest.TestCase):
    def check(self, text, **kwargs):
        options = dict(gpt_preflight=False, brief_max_kb=None, size_pct=None,
                       cap_margin_pct=None, root=None)
        options.update(kwargs)
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "brief.md"
            path.write_bytes(text.encode("utf-8") if isinstance(text, str) else text)
            return bl._brief.check(text, path, types.SimpleNamespace(**options),
                                   bl.split_sections, bl.claimed_paths,
                                   bl.claim_blocks, bl.claim_paths)

    def test_b1_read_pointer_fails(self):
        result = self.check("See scratch/briefs/x.md for the table\n"
                            "OPEN state\\table.md", gpt_preflight=True)
        self.assertEqual([r[0] for r in result], ["B1"])
        self.assertIn("line 1: scratch/briefs/x.md", result[0][2])
        self.assertIn("line 2: state\\table.md", result[0][2])

    def test_b1_written_path_and_evidence_pass(self):
        text = ("the script appends to state/pc-progress.md\n"
                "<!-- read scratch/hidden\nopen state/hidden -->\n"
                "read scratch/<name>.md and scratch/*.md\n"
                "consult state/x.md (manual)\n## Evidence\n"
                "see scratch/briefs/x.md\n## Requirements\nNo pointers")
        self.assertEqual(self.check(text, gpt_preflight=True), [])
        self.assertTrue(self.check(text.replace("## Evidence", "## Instructions"),
                                   gpt_preflight=True))

    def test_b1_pointer_with_brackets(self):
        for text, expected in (
                ("See scratch/[draft].md.", "scratch/[draft].md"),
                ("See scratch/[draft].md", "scratch/[draft].md"),
                ("(see scratch/x.md)", "scratch/x.md"),
                ("[See scratch/[draft].md]", "scratch/[draft].md"),
                ("See scratch/[draft]]", "scratch/[draft]"),
                ("See state\\[draft].md.", "state\\[draft].md")):
            with self.subTest(text=text):
                self.assertEqual(
                    self.check(text, gpt_preflight=True),
                    [("B1", "FAIL", f"line 1: {expected}; paste the content "
                      "(a coder cannot read scratch/ or state/)")])
        self.assertEqual(self.check(
            "<!-- See scratch/[draft].md -->\n"
            "See scratch/<name>.md and scratch/*.md",
            gpt_preflight=True), [])

    def test_b2_counts_bytes(self):
        self.assertEqual(self.check("a" * 29000, brief_max_kb=32, size_pct=90), [])
        result = self.check("a" * 29000 + "\u20ac" * 300,
                            brief_max_kb=32, size_pct=90)
        self.assertEqual(result[0][0], "B2")
        self.assertIn("29900 bytes", result[0][2])
        self.assertIn("(29491 bytes)", result[0][2])
        self.assertEqual(self.check("a" * 1024, brief_max_kb=1, size_pct=100), [])

    def test_b3_wording(self):
        self.assertEqual(self.check("You CANNOT\nrun\tcommands",
                                    gpt_preflight=True)[0][0], "B3")
        self.assertEqual(self.check("cannot run commands\nYOU CAN\nREAD EVERY FILE "
                                    "IN YOUR\tWORKTREE", gpt_preflight=True), [])

    def test_b4_near_cap(self):
        with tempfile.TemporaryDirectory() as folder:
            (pathlib.Path(folder) / "a.py").write_bytes(b"x\n" * 586)
            base = "## Claims\n- `a.py`{}\n## Requirements\n{}"
            template = "past 1,500 lines; over about 1,000 lines"
            text = base.format("", "must stay under 600 lines; " + template)
            result = self.check(text, root=folder, cap_margin_pct=5)
            self.assertEqual(result[0][0], "B4")
            self.assertIn("586 lines", result[0][2])
            self.assertIn("600-line cap", result[0][2])
            self.assertEqual(self.check(base.format(" (wiring only)", "under 600 lines"),
                                        root=folder, cap_margin_pct=5), [])
            self.assertEqual(self.check(base.format("", template),
                                        root=folder, cap_margin_pct=5), [])
            self.assertEqual(self.check(base.format(" under 600 lines", template),
                                        root=folder, cap_margin_pct=5)[0][0], "B4")
            self.assertEqual(self.check(base.format(" (new)", "under 600 lines"),
                                        root=folder, cap_margin_pct=5), [])

    def test_b4_cap_inside_html_comment(self):
        with tempfile.TemporaryDirectory() as folder:
            (pathlib.Path(folder) / "a.py").write_bytes(b"x\n" * 586)
            for cap in ("<!-- must stay under 600 lines -->",
                        "must stay under 600 lines"):
                with self.subTest(cap=cap):
                    text = ("## Claims\n- `a.py`\n## Requirements\n" + cap
                            + "\n<!-- See scratch/[draft].md -->")
                    self.assertEqual(bl._brief.caps(cap), [600])
                    self.assertEqual(
                        self.check(text, root=folder, cap_margin_pct=5,
                                   gpt_preflight=True),
                        [("B4", "FAIL",
                          'a.py has 586 lines, within 5% of the stated 600-line cap: '
                          'mark its claim "wiring" or split first')])

    def cli(self, path, *flags):
        output, errors = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
            code = bl.main(["--max-lines", "400", "--max-bullets", "80",
                            "--max-files", "80", "--max-done-lines", "10",
                            *flags, str(path)])
        return code, output.getvalue(), errors.getvalue()

    def test_flags_off_output_unchanged(self):
        # Keep an existing failure so enabling B rules does not change the exit code.
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "brief.md"
            path.write_text("see scratch/x.md\nYou cannot run commands", encoding="utf-8")
            off = self.cli(path)
            on = self.cli(path, "--gpt-preflight")
            self.assertEqual(off[0], on[0])
            self.assertNotIn("brief-B", off[1])
            self.assertNotIn("PASS brief", off[1])
            filtered = "\n".join(line for line in on[1].splitlines()
                                 if "brief-B" not in line) + "\n"
            # Only the labelled failed-rule total differs.
            self.assertEqual(off[1].splitlines()[1:], filtered.splitlines()[1:])
            self.assertIn("brief-B1", on[1])
            self.assertIn("brief-B3", on[1])

    def test_flag_validation_and_pass_output(self):
        with tempfile.TemporaryDirectory() as folder:
            path = pathlib.Path(folder) / "brief.md"
            path.write_text("", encoding="utf-8")
            for flags in (("--brief-max-kb", "32"), ("--size-pct", "90"),
                          ("--cap-margin-pct", "5"), ("--size-pct", "0")):
                code, _, errors = self.cli(path, *flags)
                self.assertEqual(code, 2)
                self.assertTrue(errors)
            self.assertIn("  PASS brief\n", self.cli(path, "--gpt-preflight")[1])

    def test_odd_input(self):
        self.assertEqual(self.check(b"\xff\x00", gpt_preflight=True), [])
        text = ("see state/x.md\n" * 10000)
        result = self.check(text, gpt_preflight=True)
        self.assertEqual(result[0][2].count("line "), 5)
        self.assertEqual(self.check("## Claims\n- ../outside.py\nunder 10 lines",
                                    root=HERE, cap_margin_pct=5), [])


if __name__ == "__main__":
    unittest.main()
