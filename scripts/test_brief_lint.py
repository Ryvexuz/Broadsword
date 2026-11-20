"""Tests for scripts/brief_lint.py: one test per rule shown failing, plus passing and CLI cases.

  python -I -B scripts/test_brief_lint.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import os
import pathlib
import shutil
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = pathlib.Path(os.environ.get("BRIEF_LINT_UNDER_TEST", str(HERE / "brief_lint.py")))
spec = importlib.util.spec_from_file_location("brief_lint", SCRIPT)
bl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bl)

LIMITS = (30, 8, 2, 10)  # lines, bullets, files, done-check lines excluded from the line count

GOOD = """# Brief: small task

**Board:**
- Title: `t: small`
- Claims: `scripts/a.py` (new), `scripts/test_a.py` (new)

**Requirements:**
- does one thing
- does another thing

**Done check:**
1. `python -I scripts/test_a.py` shows OK.
2. `broadsword test crash-sim --task <id>` -> `test: ok`.
"""


def lint(text):
    return {r: (ok, msg) for r, ok, msg in bl.lint(text, *LIMITS)}


def run_cli(args):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = bl.main(args)
    return code, out.getvalue(), err.getvalue()


class Rules(unittest.TestCase):
    def test_good_brief_passes_every_rule(self):
        res = lint(GOOD)
        self.assertEqual(set(res), {"lines", "bullets", "files", "donecheck"})
        for rule, (ok, msg) in res.items():
            self.assertTrue(ok, f"{rule}: {msg}")

    def test_lines_rule_fails_over_limit(self):
        filler = "".join(f"filler line {i}\n" for i in range(40))
        res = lint(GOOD.replace("**Requirements:**", filler + "**Requirements:**"))
        self.assertFalse(res["lines"][0])
        self.assertIn("non-blank lines", res["lines"][1])

    def test_lines_rule_ignores_a_short_done_check_block(self):
        # a done check of about 10 lines on top of GOOD's 8 other lines still passes the line count
        long_done = GOOD + "".join(f"{i}. `python -I x{i}.py`\n" for i in range(3, 11))
        res = lint(long_done)
        self.assertTrue(res["lines"][0], res["lines"][1])
        self.assertTrue(res["donecheck"][0], res["donecheck"][1])

    def test_bullets_rule_fails_over_limit(self):
        many = "".join(f"- requirement {i}\n" for i in range(9))
        res = lint(GOOD.replace("- does one thing\n- does another thing\n", many))
        self.assertFalse(res["bullets"][0])
        self.assertIn("10 requirement bullets", res["bullets"][1])  # 9 + the Board title bullet

    def test_bullets_rule_allows_exactly_the_limit_and_skips_nested(self):
        eight = "".join(f"- requirement {i}\n  - nested {i}\n" for i in range(8))
        res = lint(GOOD.replace("- does one thing\n- does another thing\n", eight))
        self.assertTrue(res["bullets"][0], res["bullets"][1])

    def test_files_rule_fails_with_three_files_and_no_reason(self):
        res = lint(GOOD.replace("`scripts/test_a.py` (new)",
                                "`scripts/test_a.py` (new), `docs/A.md` (new)"))
        self.assertFalse(res["files"][0])
        self.assertIn("no reason stated", res["files"][1])

    def test_files_rule_accepts_three_files_with_a_reason(self):
        res = lint(GOOD.replace("`scripts/test_a.py` (new)",
                                "`scripts/test_a.py` (new), `docs/A.md` (new); reason: docs ship with it"))
        self.assertTrue(res["files"][0], res["files"][1])

    def test_files_rule_fails_closed_without_a_claims_line(self):
        res = lint(GOOD.replace("- Claims: `scripts/a.py` (new), `scripts/test_a.py` (new)\n", ""))
        self.assertFalse(res["files"][0])
        self.assertIn("no Claims line", res["files"][1])

    def test_donecheck_rule_fails_when_missing(self):
        res = lint(GOOD.split("**Done check:**")[0])
        self.assertFalse(res["donecheck"][0])
        self.assertIn("no Done check", res["donecheck"][1])

    def test_donecheck_rule_fails_when_an_item_is_not_a_command(self):
        res = lint(GOOD + "3. Real run without `--push`, output pasted.\n")
        self.assertFalse(res["donecheck"][0])
        self.assertIn("1 of 3", res["donecheck"][1])

    def test_donecheck_rule_fails_when_section_has_no_items(self):
        res = lint(GOOD.split("**Done check:**")[0] + "**Done check:**\n\nit works\n")
        self.assertFalse(res["donecheck"][0])
        self.assertIn("no items", res["donecheck"][1])

    def test_donecheck_inline_command_passes(self):
        text = "- Claim: `a.py`\n- Requirements: do it\n- Done check: `python -I scripts/test_a.py` passes\n"
        res = lint(text)
        self.assertTrue(res["donecheck"][0], res["donecheck"][1])
        self.assertTrue(res["files"][0], res["files"][1])

    def test_exe_path_command_is_a_command(self):
        self.assertTrue(bl.is_command("build/bin/broadsword.exe check x"))
        self.assertFalse(bl.is_command("OK"))
        self.assertFalse(bl.is_command("--push"))


HEAD = "# Brief: x\n\n- Claims: `scripts/a.py` (new)\n\n"
DONE = "\n**Done check:**\n1. `python -I scripts/test_a.py` shows OK.\n"
TWENTY = "".join(f"- requirement {i}\n" for i in range(20))
CLAIMS_GOOD = "- Claims: `scripts/a.py` (new), `scripts/test_a.py` (new)"


class Bypasses(unittest.TestCase):
    """One test per bypass found in review q-20261120-034119 (each failed before the fix)."""

    def test_requirements_heading_forms_are_all_counted(self):
        for heading in ("Requirements:", "## Requirements", "**Requirements**", "**Requirements:**",
                        "### Requirements:", "- Requirements:"):
            res = lint(HEAD + heading + "\n" + TWENTY + DONE)
            self.assertFalse(res["bullets"][0], f"{heading!r}: {res['bullets'][1]}")
            self.assertIn("requirement bullets", res["bullets"][1], heading)
            self.assertTrue(res["bullets"][1].startswith("20 "), f"{heading!r}: {res['bullets'][1]}")

    def test_bullets_without_a_requirements_heading_is_an_error_not_zero(self):
        res = lint(HEAD + "**What to build**\n" + TWENTY + DONE)
        self.assertFalse(res["bullets"][0])
        self.assertIn("no Requirements section", res["bullets"][1])

    def test_a_bullet_that_starts_with_a_listed_word_does_not_end_requirements(self):
        res = lint(HEAD + "**Requirements:**\n- Order: lint first\n" + TWENTY + DONE)
        self.assertFalse(res["bullets"][0])
        self.assertTrue(res["bullets"][1].startswith("21 "), res["bullets"][1])

    def test_a_real_bold_header_does_end_requirements(self):
        # The header still ends the section (round-1 rule); its bullets are counted anyway (round 2).
        res = lint(HEAD + "**Requirements:**\n- one\n- two\n**Notes:**\n- three\n- four\n" + DONE)
        self.assertTrue(res["bullets"][0], res["bullets"][1])
        self.assertTrue(res["bullets"][1].startswith("4 "), res["bullets"][1])

    def test_bullets_under_an_also_section_count_toward_the_limit(self):
        nine = "".join(f"- extra {i}\n" for i in range(9))
        res = lint(HEAD + "**Requirements:**\n- one\n**Also:**\n" + nine + DONE)
        self.assertFalse(res["bullets"][0], res["bullets"][1])
        self.assertTrue(res["bullets"][1].startswith("10 "), res["bullets"][1])

    def test_paths_after_a_bold_line_following_claims_are_counted(self):
        res = lint(HEAD + "**Also:**\n- `scripts/b.py`\n- `docs/c.md`\n" + DONE)
        self.assertFalse(res["files"][0], res["files"][1])
        self.assertTrue(res["files"][1].startswith("3 "), res["files"][1])

    def test_long_done_check_prose_cannot_hide_lines(self):
        res = lint(GOOD + "".join(f"prose line {i}\n" for i in range(200)))
        self.assertFalse(res["lines"][0], res["lines"][1])

    def test_unlisted_header_such_as_notes_counts_toward_the_lines(self):
        res = lint(GOOD + "**Notes:**\n" + "".join(f"note line {i}\n" for i in range(200)))
        self.assertFalse(res["lines"][0], res["lines"][1])

    def test_a_heading_ends_the_done_check_so_later_prose_counts(self):
        res = lint(GOOD + "## Background\n" + "".join(f"background {i}\n" for i in range(40)))
        self.assertFalse(res["lines"][0], res["lines"][1])

    def test_unbackticked_claims_are_counted(self):
        res = lint(GOOD.replace(CLAIMS_GOOD, "- Claims: scripts/a.py, scripts/b.py, docs/c.md"))
        self.assertFalse(res["files"][0])
        self.assertTrue(res["files"][1].startswith("3 "), res["files"][1])

    def test_several_paths_in_one_span_count_separately(self):
        res = lint(GOOD.replace(CLAIMS_GOOD, "- Claims: `scripts/a.py scripts/b.py docs/c.md`"))
        self.assertFalse(res["files"][0])
        self.assertTrue(res["files"][1].startswith("3 "), res["files"][1])

    def test_paths_listed_as_bullets_under_a_claims_header_are_counted(self):
        text = (HEAD.replace("- Claims: `scripts/a.py` (new)", "**Claims:**\n- scripts/a.py\n- b/c.rs\n- d/e.md")
                + "**Requirements:**\n- x\n" + DONE)
        res = lint(text)
        self.assertFalse(res["files"][0])
        self.assertTrue(res["files"][1].startswith("3 "), res["files"][1])

    def test_a_reason_is_still_accepted_with_unbackticked_paths(self):
        res = lint(GOOD.replace(CLAIMS_GOOD,
                                "- Claims: scripts/a.py, scripts/b.py, docs/c.md; reason: docs ship with it"))
        self.assertTrue(res["files"][0], res["files"][1])

    def test_a_path_named_reason_is_not_a_stated_reason(self):
        res = lint(GOOD.replace(CLAIMS_GOOD, "- Claims: scripts/reasons.py, scripts/b.py, docs/because.md"))
        self.assertFalse(res["files"][0], res["files"][1])

    def test_a_done_check_over_the_cap_counts_only_its_excess(self):
        # 8 other lines + a 41-line done check: 31 lines beyond the cap of 10 are counted, 39 > 30
        text = GOOD + "".join(f"{i}. `python -I x{i}.py`\n" for i in range(3, 41))
        res = lint(text)
        self.assertFalse(res["lines"][0], res["lines"][1])


class Cli(unittest.TestCase):
    def setUp(self):
        self.dir = pathlib.Path(os.environ.get("BRIEF_LINT_SCRATCH",
                                               HERE.parent / "scratch")) / "test_brief_lint"
        shutil.rmtree(self.dir, ignore_errors=True)
        self.dir.mkdir(parents=True)
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def write(self, name, text):
        p = self.dir / name
        p.write_text(text, encoding="utf-8")
        return str(p)

    FLAGS = ["--max-lines", "30", "--max-bullets", "8", "--max-files", "2",
             "--max-done-lines", "10"]

    def test_good_brief_exit_0(self):
        code, out, _ = run_cli(self.FLAGS + [self.write("g.md", GOOD)])
        self.assertEqual(code, 0, out)
        self.assertIn("brief_lint: ok", out)

    def test_bad_brief_exit_1_and_prints_each_failed_rule(self):
        bad = GOOD.split("**Done check:**")[0] + "".join(f"- r{i}\n" for i in range(9))
        code, out, _ = run_cli(self.FLAGS + [self.write("b.md", bad)])
        self.assertEqual(code, 1)
        self.assertIn("FAIL bullets", out)
        self.assertIn("FAIL donecheck", out)
        self.assertIn("PASS lines", out)

    def test_missing_file_exit_2_names_the_file(self):
        code, _, err = run_cli(self.FLAGS + [str(self.dir / "nope.md")])
        self.assertEqual(code, 2)
        self.assertIn("nope.md", err)

    def test_limits_are_required_flags(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(bl.main([self.write("g.md", GOOD)]), 2)


if __name__ == "__main__":
    unittest.main()
