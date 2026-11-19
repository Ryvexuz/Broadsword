"""Tests for the last-pass.json record that scripts/shadow_canary.py writes. No codex, no model: leg A / leg B and `codex --version`
are replaced, and the checkout is a temporary folder with its own broadsword.toml.

Run:  python -I -B scripts/test_shadow_canary.py
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import importlib.util
import io
import json
import shutil
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("shadow_canary_under_test", HERE / "shadow_canary.py")
canary = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(canary)

TOML = 'scratch_dir = "scratch"\nstate_dir = "state"\nshadow_reviewer_model = "gpt-test"\nshadow_reviewer_timeout_seconds = 30\n'
REVIEWER_RS = 'pub const SYSTEM_PROMPT: &str = "be a reviewer";\n'


class LastPassTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="canary-test-"))
        self.addCleanup(shutil.rmtree, self.root, True)
        (self.root / "broadsword.toml").write_text(TOML, encoding="utf-8")
        (self.root / "reviewer.rs").write_text(REVIEWER_RS, encoding="utf-8")
        self.last = self.root / "state" / "canary" / "last-pass.json"

    def main(self, argv, a=0, b=0, version="codex-cli 9.9.9"):
        with mock.patch.object(canary, "CHECKOUT", self.root), mock.patch.object(canary, "REVIEWER_RS", self.root / "reviewer.rs"), \
                mock.patch.object(canary, "leg_offline", return_value=a), mock.patch.object(canary, "leg_real", return_value=b), \
                mock.patch.object(canary, "codex_version", return_value=version), redirect_stdout(io.StringIO()):
            return canary.main(argv)

    def test_offline_pass_writes_the_version_atomically(self):
        self.assertEqual(self.main(["--offline-only"]), canary.REFUSED)
        rec = json.loads(self.last.read_text(encoding="utf-8"))
        self.assertEqual(rec["codex_version"], "codex-cli 9.9.9")
        self.assertEqual(rec["legs"], ["A"])
        self.assertIsNone(rec["model"])
        self.assertRegex(rec["utc"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertEqual([p.name for p in self.last.parent.iterdir()], ["last-pass.json"], "no temp file is left behind")

    def test_full_pass_records_both_legs_and_the_model(self):
        self.assertEqual(self.main([]), canary.REFUSED)
        rec = json.loads(self.last.read_text(encoding="utf-8"))
        self.assertEqual((rec["legs"], rec["model"]), (["A", "B"], "gpt-test"))

    def test_a_fail_writes_nothing(self):
        for label, a, b, argv in (("leg A open", canary.LEAKED, 0, ["--offline-only"]), ("leg A inconclusive", canary.INCONCLUSIVE, 0, []),
                                  ("leg B leaked", 0, canary.LEAKED, []), ("leg B inconclusive", 0, canary.INCONCLUSIVE, [])):
            with self.subTest(label):
                self.assertNotEqual(self.main(argv, a=a, b=b), canary.REFUSED)
                self.assertFalse(self.last.exists())

    def test_a_fail_leaves_an_older_pass_untouched(self):
        self.last.parent.mkdir(parents=True)
        self.last.write_text('{"codex_version": "old"}\n', encoding="utf-8")
        self.main([], a=canary.LEAKED)
        self.assertEqual(json.loads(self.last.read_text(encoding="utf-8")), {"codex_version": "old"})

    def test_a_pass_with_no_codex_version_is_not_recorded(self):
        self.assertEqual(self.main(["--offline-only"], version=None), canary.INCONCLUSIVE)
        self.assertFalse(self.last.exists())


if __name__ == "__main__":
    unittest.main()
