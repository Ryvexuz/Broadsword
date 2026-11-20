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
import tempfile
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
        seven = "".join(f"- requirement {i}\n  - nested {i}\n" for i in range(7))  # + the Board title bullet = 8
        res = lint(GOOD.replace("- does one thing\n- does another thing\n", seven))
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


class HiddenBullets(unittest.TestCase):
    def test_bullets_after_a_claims_line_are_counted(self):
        many = "".join(f"- req {i}\n" for i in range(20))
        text = "Requirements:\n- one\nClaims: a.py\n" + many + "Done check: `python t.py`\n"
        res = lint(text)
        self.assertFalse(res["bullets"][0], res["bullets"][1])

    def test_a_path_bullet_under_claims_is_not_a_requirement_bullet(self):
        text = "Requirements:\n- one\nClaims:\n" + "- a/b.py\n" * 9 + "Done check: `python t.py`\n"
        res = lint(text)
        self.assertTrue(res["bullets"][0], res["bullets"][1])

    def test_bullets_before_the_first_section_are_counted(self):
        many = "".join(f"- req {i}\n" for i in range(20))
        text = many + "Requirements:\n- one\nClaims: a.py\nDone check: `python t.py`\n"
        res = lint(text)
        self.assertFalse(res["bullets"][0], res["bullets"][1])


class SecondDoneCheck(unittest.TestCase):
    def test_more_than_one_done_check_section_is_an_error(self):
        many = "".join(f"- req {i}\n" for i in range(20))
        text = "Requirements:\n- one\nClaims: a.py\nDone check: `python t.py`\nDone check:\n" + many
        res = lint(text)
        self.assertFalse(res["donecheck"][0], res["donecheck"][1])
        self.assertIn("more than one Done check section", res["donecheck"][1])


def gaps(text, root=None):
    return [(r, lv, m) for r, lv, m in bl.claim_gaps(text, root)]


def brief(claims, req, done="`python -I scripts/test_a.py` shows OK.", extra=""):
    return (f"# Brief: x\n\n**Board:**\n- Claims: {claims}\n\n**Requirements:**\n{req}\n\n"
            f"**Done check:**\n1. {done}\n{extra}")


class ClaimGapRules(unittest.TestCase):
    """R1-R5: one test per rule, each shown failing before the rule existed."""

    def levels(self, text, root=None):
        return {(r, lv) for r, lv, _ in gaps(text, root)}

    def test_clean_brief_has_no_gap(self):
        self.assertEqual(gaps(GOOD), [])

    def test_r1_path_in_requirements_not_claimed_fails(self):
        g = gaps(brief("`src/jobs.rs`", "- add an empty `src/jobs/graph.rs`"))
        self.assertIn(("R1", "FAIL"), {(r, lv) for r, lv, _ in g})
        self.assertIn("src/jobs/graph.rs", " ".join(m for _, _, m in g))

    def test_r1_prefix_and_basename_cover(self):
        text = brief("`src/` (folder), `crates/x/tests/y.rs`", "- edit `src/a/b.rs` and `y.rs`")
        self.assertEqual(gaps(text), [])

    def test_r1_other_task_clause_and_other_repo_paths_are_skipped(self):
        req = "- n20-47 copies a summary into Broadsword's `docs/nodes/x.md` and updates `ROADMAP.md`"
        self.assertEqual(gaps(brief("`src/a.rs`", req)), [])

    def test_r1_bare_reference_names_and_prose_slashes_are_skipped(self):
        self.assertEqual(gaps(brief("`src/a.rs`", "- see AGENTS.md; save/load round trip; `queue.lock`")), [])

    def test_r1_done_check_command_paths_are_reads_not_edits(self):
        self.assertEqual(gaps(brief("`src/a.rs`", "- x", done="`git check-attr -a tests/data/f.save` ok")), [])

    def test_r1_docs_or_output_folder_only_warns(self):
        self.assertEqual(self.levels(brief("`src/a.rs`", "- results saved in `state/out/`")), {("R1", "WARN")})

    def test_r1c_claim_instruction_in_a_note_fails(self):
        text = brief("`src/a.rs`", "- x") + "\n## Orchestrator note\nClaim src/collide.rs and src/main.rs here.\n"
        g = gaps(text)
        self.assertIn(("R1c", "FAIL"), {(r, lv) for r, lv, _ in g})
        self.assertIn("src/collide.rs", g[0][2])
        ok = text.replace("`src/a.rs`", "`src/a.rs`, `src/collide.rs`, `src/main.rs`")
        self.assertEqual(gaps(ok), [])

    def test_r1b_bench_example_without_examples_claim_warns(self):
        text = brief("`src/a.rs`", "- measured in a bench example, not a test")
        self.assertEqual(self.levels(text), {("R1b", "WARN")})
        self.assertEqual(gaps(text.replace("`src/a.rs`", "`src/a.rs`, `examples/b.rs`")), [])

    def test_r2_pub_mod_without_stub_fails(self):
        g = self.levels(brief("`src/jobs.rs`", "- leave `pub mod graph;` declared"))
        self.assertIn(("R2", "FAIL"), g)
        self.assertNotIn(("R2", "FAIL"), self.levels(brief("`src/jobs.rs`, `src/jobs/graph.rs`",
                                                          "- leave `pub mod graph;` declared")))

    def test_r2_submodule_names_need_stubs(self):
        req = "- leave a `binary` and a `migrate` submodule declared empty in `serial.rs`"
        g = gaps(brief("`src/serial.rs`", req))
        self.assertIn("binary, migrate", " ".join(m for r, _, m in g if r == "R2"))
        self.assertNotIn("R2", {r for r, _, _ in gaps(brief("`src/serial.rs`, `src/serial/binary.rs`, "
                                                            "`src/serial/migrate/mod.rs`", req))})

    def test_r3_new_flag_without_tools_rs_fails_and_plain_flag_warns(self):
        claims = "`crates/broadsword/src/queue.rs`"
        self.assertEqual(self.levels(brief(claims, "- adds a new flag `--per-project` to queue run")),
                         {("R3", "FAIL")})
        self.assertEqual(self.levels(brief(claims, "- `recover --abandon <id>` is refused")), {("R3", "WARN")})
        self.assertEqual(gaps(brief(claims + ", `crates/broadsword/src/tools.rs`",
                                    "- adds a new flag `--per-project`")), [])
        self.assertEqual(gaps(brief("`src/a.rs`", "- adds a new flag `--x`")), [])  # not a broadsword brief

    def test_r4_new_required_key_needs_config_toml_and_support(self):
        req = "- a new required setting turns it on"
        g = gaps(brief("`crates/broadsword/src/queue.rs`", req))
        self.assertIn(("R4", "FAIL"), {(r, lv) for r, lv, _ in g})
        self.assertIn("config.rs, broadsword.toml, tests/support/mod.rs", " ".join(m for _, _, m in g))
        full = ("`crates/broadsword/src/config.rs`, `broadsword.toml`, "
                "`crates/broadsword/tests/support/mod.rs`")
        self.assertNotIn("R4", {r for r, _, _ in gaps(brief(full, req))})

    def test_r5_lists_unclaimed_files_that_use_a_symbol(self):
        import subprocess
        d = pathlib.Path(tempfile.mkdtemp(prefix="test_brief_lint_r5_"))
        self.addCleanup(shutil.rmtree, d, ignore_errors=True)
        (d / "a.rs").write_text("fn guard_refusal() {}\n", encoding="utf-8")
        (d / "b.rs").write_text("guard_refusal();\n", encoding="utf-8")
        subprocess.run(["git", "-C", str(d), "init", "-q"], check=True)
        subprocess.run(["git", "-C", str(d), "add", "."], check=True)
        text = brief("`a.rs`", "- change `guard_refusal()`")
        g = gaps(text, str(d))
        self.assertEqual([(r, lv) for r, lv, _ in g], [("R5", "WARN")])
        self.assertIn("b.rs", g[0][2])
        self.assertNotIn("a.rs", g[0][2])
        self.assertEqual(gaps(text), [])  # no --root: R5 is skipped


class Cli(unittest.TestCase):
    def setUp(self):
        self.dir = pathlib.Path(tempfile.mkdtemp(prefix="test_brief_lint_"))
        self.addCleanup(shutil.rmtree, self.dir, ignore_errors=True)

    def test_scratch_dir_is_a_fresh_temp_directory_not_a_repo_folder(self):
        self.assertTrue(self.dir.parent.samefile(tempfile.gettempdir()), self.dir)

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

    def test_claim_gap_fail_exits_1_and_warn_alone_exits_0(self):
        fail = brief("`src/jobs.rs`", "- add `src/jobs/graph.rs`")
        code, out, _ = run_cli(self.FLAGS + [self.write("f.md", fail)])
        self.assertEqual(code, 1)
        self.assertIn("FAIL claimgap-R1", out)
        warn = brief("`src/a.rs`", "- results in `state/out/`", extra="")
        code, out, _ = run_cli(self.FLAGS + [self.write("w.md", warn)])
        self.assertIn("WARN claimgap-R1", out)
        self.assertEqual(code, 0, out)

    def test_limits_are_required_flags(self):
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(bl.main([self.write("g.md", GOOD)]), 2)


if __name__ == "__main__":
    unittest.main()
