"""Tests for scripts/usage_sum.py. Temp state folders only; the real ledger is never read.

  python -I -B scripts/test_usage_sum.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import pathlib
import shutil
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("usage_sum", HERE / "usage_sum.py")
usage_sum = importlib.util.module_from_spec(spec)
spec.loader.exec_module(usage_sum)

OLD_HEADER = "utc\tcaller\tmodel\tinput_tokens\toutput_tokens\test_usd\tplan_or_credits\n"
NEW_HEADER = OLD_HEADER.rstrip("\n") + "\tcall_type\n"


def row(usd, label, call_type=None):
    cols = ["2026-01-01T00:00:00Z", "c", "m", "1", "1", usd, label]
    return "\t".join(cols + ([call_type] if call_type is not None else [])) + "\n"


class UsageSumTests(unittest.TestCase):
    def run_sum(self, text, date="2026-01-01"):
        d = pathlib.Path(tempfile.mkdtemp(prefix="usagesum-"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "gpt-spend").mkdir()
        if text is not None:
            (d / "gpt-spend" / "2026-01-01.tsv").write_text(text, encoding="utf-8")
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = usage_sum.main(["--state-dir", str(d), "--date", date])
        return code, out.getvalue()

    def test_totals_each_call_type_split_plan_and_credits(self):
        code, out = self.run_sum(NEW_HEADER + row("1.5", "codex-plan", "gate") + row("2.0", "codex-plan", "gate")
                                 + row("0.25", "api-credits", "gate") + row("4.0", "api-credits", "bench") + row("0.5", "codex-plan-failed", "shadow"))
        self.assertEqual(code, 0, out)
        lines = out.split("\n")
        self.assertIn("bench\tcredits\t1\t4.000000", lines)
        self.assertIn("gate\tcredits\t1\t0.250000", lines)
        self.assertIn("gate\tplan\t2\t3.500000", lines)
        self.assertIn("shadow\tplan\t1\t0.500000", lines)
        self.assertIn("total\tall\t5\t8.250000", lines)

    def test_old_rows_count_as_unknown_and_a_mixed_file_works(self):
        code, out = self.run_sum(OLD_HEADER + row("1.0", "codex-plan") + row("2.0", "api-credits") + row("3.0", "codex-plan", "full"))
        self.assertEqual(code, 0, out)
        lines = out.split("\n")
        self.assertIn("unknown\tplan\t1\t1.000000", lines)
        self.assertIn("unknown\tcredits\t1\t2.000000", lines)
        self.assertIn("full\tplan\t1\t3.000000", lines)

    def test_an_empty_call_type_cell_is_unknown(self):
        code, out = self.run_sum(NEW_HEADER + row("1.0", "codex-plan", ""))
        self.assertEqual(code, 0, out)
        self.assertIn("unknown\tplan\t1\t1.000000", out.split("\n"))

    def test_a_malformed_row_is_an_error_not_skipped(self):
        for body in (row("abc", "codex-plan", "gate"), row("nan", "codex-plan", "gate"), row("1.0", "mystery", "gate"),
                     "a\tb\tc\n", row("1.0", "codex-plan", "gate").rstrip("\n") + "\textra\n"):
            with self.subTest(body=body):
                code, out = self.run_sum(NEW_HEADER + row("1.0", "codex-plan", "gate") + body)
                self.assertEqual(code, 2, out)
                self.assertIn("unreadable row", out)
                self.assertNotIn("total", out)

    def test_missing_ledger_and_bad_date_are_errors(self):
        code, out = self.run_sum(None)
        self.assertEqual(code, 2, out)
        code, out = self.run_sum(NEW_HEADER, date="../x")
        self.assertEqual(code, 2, out)


if __name__ == "__main__":
    unittest.main()
