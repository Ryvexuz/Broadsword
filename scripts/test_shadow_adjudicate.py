"""Tests for scripts/shadow_adjudicate.py (add). Shares its fixtures with test_shadow_report.py.

  python -I -B scripts/test_shadow_adjudicate.py -v
"""
import sys

sys.dont_write_bytecode = True

import importlib.util
import pathlib
import unittest

HERE = pathlib.Path(__file__).resolve().parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


_t = load("test_shadow_report")
sr, sa, entry, Base = _t.sr, _t.sa, _t.entry, _t.Base


class AddTests(Base):
    def test_refuses_bad_verdict_unknown_key_unknown_entry_empty_evidence(self):
        self.fixture()
        for args, why in (
            (("q-A", "src/a.rs:1", "maybe", "--evidence", "x", "--judge", "m"), "verdict"),
            (("q-A", "src/a.rs:99", "real", "--evidence", "x", "--judge", "m"), "unknown finding key"),
            (("q-A", "shared.rs:9", "real", "--evidence", "x", "--judge", "m"), "unknown finding key"),
            (("q-ZZ", "src/a.rs:1", "real", "--evidence", "x", "--judge", "m"), "unknown entry"),
            (("q-C", "src/a.rs:1", "real", "--evidence", "x", "--judge", "m"), "unknown finding key"),
            (("q-A", "src/a.rs:1", "real", "--evidence", "   ", "--judge", "m"), "evidence is empty"),
            (("q-A", "src/a.rs:1", "real", "--evidence", "x", "--judge", " "), "judge is empty"),
        ):
            code, _, err = self.add(*args)
            self.assertEqual(code, 2, args)
            self.assertIn(why, err)
        self.assertEqual(self.verdicts_text(), "")  # nothing written

    def test_appends_row_with_header_and_never_rewrites(self):
        self.fixture()
        self.add("q-A", "src/a.rs:1", "real", "--evidence", "line\twith tab", "--judge", "haiku")
        first = self.verdicts_text()
        self.assertTrue(first.startswith(sr.VERDICT_HEADER + "\n"))
        row = first.splitlines()[1].split("\t")
        self.assertEqual(len(row), 6)
        self.assertEqual(row[:5], ["q-A", "src/a.rs:1", "real", "line with tab", "haiku"])
        self.add("q-A", "src/a.rs:2", "false", "--evidence", "wrong", "--judge", "haiku")
        self.assertTrue(self.verdicts_text().startswith(first))  # earlier rows untouched

    def test_later_verdict_replaces_earlier(self):
        self.fixture()
        self.add("q-A", "src/a.rs:1", "false", "--evidence", "first", "--judge", "m")
        self.add("q-A", "src/a.rs:1", "real", "--evidence", "second", "--judge", "m")
        v = sr.load_verdicts(self.root)
        self.assertEqual(v[("q-A", "src/a.rs:1")]["verdict"], "real")
        self.assertEqual(v[("q-A", "src/a.rs:1")]["evidence"], "second")
        self.assertEqual(len(self.verdicts_text().splitlines()), 3)  # header + both rows kept


if __name__ == "__main__":
    unittest.main()
