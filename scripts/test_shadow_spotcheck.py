"""Tests for scripts/shadow_spotcheck.py.

  python -I -B scripts/test_shadow_spotcheck.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import pathlib
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("shadow_spotcheck", HERE / "shadow_spotcheck.py")
sp = importlib.util.module_from_spec(spec)
spec.loader.exec_module(sp)

HEADER = sp.VERDICT_HEADER


def vrow(entry, key, verdict="false", judge="haiku (gating)"):
    return "\t".join([entry, key, verdict, "ev", judge, "2026-11-21"])


class Base(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = pathlib.Path(self._tmp.name)
        (self.root / "state").mkdir()

    def write_verdicts(self, rows, header=HEADER):
        (self.root / sp.VERDICT_FILE).write_text(header + "\n" + "".join(r + "\n" for r in rows), encoding="utf-8", newline="")

    def run_main(self, *args):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = sp.main(["--root", str(self.root), *args])
        return code, out.getvalue(), err.getvalue()

    def selected_keys(self, n=200):
        """n finding keys of one entry that the rule selects."""
        found, i = [], 0
        while len(found) < n:
            k = f"src/a.rs:{i}"
            if sp.is_selected("q-A", k):
                found.append(k)
            i += 1
        return found


class SelectionTests(Base):
    def test_deterministic_and_about_one_in_five(self):
        keys = [(f"q-{i // 7}", f"file{i}.rs:{i}") for i in range(1000)]
        first = [sp.is_selected(e, k) for e, k in keys]
        self.assertEqual(first, [sp.is_selected(e, k) for e, k in keys])
        n = sum(first)
        # binomial(1000, 0.2): mean 200, sd 12.6; bound is +-5 sd (137..263). The 1000 keys are fixed, so this never flakes.
        self.assertTrue(137 <= n <= 263, n)
        self.assertTrue(0 < n < 1000)

    def test_known_value_pins_the_rule(self):
        import hashlib

        h = int(hashlib.sha256(b"q-X\nsrc/x.rs:1").hexdigest(), 16)
        self.assertEqual(sp.is_selected("q-X", "src/x.rs:1"), h % 5 == 0)

    def test_non_false_rows_never_selected(self):
        keys = self.selected_keys(50)
        rows = [vrow("q-A", k, v) for k in keys for v in ("real",)] + [vrow("q-B", k, "known") for k in keys]
        self.write_verdicts(rows)
        self.assertEqual(sp.due_rows(self.root), [])
        code, out, _ = self.run_main("--check")
        self.assertEqual(code, 0)
        self.assertIn("0 of 0", out)

    def test_false_rows_selected_and_later_row_replaces(self):
        keys = self.selected_keys(3)
        self.write_verdicts([vrow("q-A", keys[0]), vrow("q-A", keys[1]), vrow("q-A", keys[1], "real"), vrow("q-A", keys[2])])
        due = {(c[0], c[1]) for c in sp.due_rows(self.root)}
        self.assertEqual(due, {("q-A", keys[0]), ("q-A", keys[2])})  # keys[1] was replaced by a real verdict


class RecordTests(Base):
    def test_forbidden_judge_refused_and_nothing_written(self):
        k = self.selected_keys(1)[0]
        self.write_verdicts([vrow("q-A", k, judge="haiku (gating)")])
        rk = f"q-A|{k}"
        code, _, err = self.run_main("--record", rk, "real", "Haiku", "ev")
        self.assertEqual(code, 2)
        self.assertIn("independent", err)
        code, _, err = self.run_main("--record", rk, "real", "opus", "ev", "--exclude", "Opus")
        self.assertEqual(code, 2)
        self.assertFalse((self.root / sp.SPOT_FILE).exists())
        code, out, _ = self.run_main("--record", rk, "false", "opus (spot)", "ev line")
        self.assertEqual(code, 0, out)
        text = (self.root / sp.SPOT_FILE).read_text(encoding="utf-8")
        self.assertTrue(text.startswith(sp.SPOT_HEADER + "\n"))
        self.assertEqual(text.splitlines()[1].split("\t")[:5], ["q-A", k, "false", "opus (spot)", "ev line"])

    def record_with(self, orig_judge, judge, *extra):
        k = self.selected_keys(1)[0]
        self.write_verdicts([vrow("q-A", k, judge=orig_judge)])
        (self.root / sp.SPOT_FILE).unlink(missing_ok=True)  # each call is a fresh case
        return self.run_main("--record", f"q-A|{k}", "real", judge, "ev", *extra)[0]

    def test_independence_is_by_family(self):
        self.assertEqual(self.record_with("orchestrator", "claude-opus-5-5"), 2)  # orchestrator is Opus
        self.assertEqual(self.record_with("orchestrator (haiku raised other points)", "opus"), 2)
        self.assertEqual(self.record_with("haiku (gating)", "claude-haiku-4-5"), 2)
        self.assertEqual(self.record_with("opus (gating)", "sonnet"), 0)
        self.assertEqual(self.record_with("orchestrator", "sonnet"), 0)
        self.assertEqual(self.record_with("haiku (gating)", "gpt-5", "--exclude", "GPT-5.2"), 2)  # exclude by family
        self.assertEqual(self.record_with("haiku (gating)", "claude-sonnet-5-5", "--exclude", "claude-sonnet-4"), 2)

    def test_unknown_family_refused(self):
        self.assertEqual(self.record_with("haiku (gating)", "mystery-model"), 2)
        self.assertEqual(self.record_with("haiku (gating)", "orchestrators"), 2)
        self.assertEqual(self.record_with("mystery (gating)", "opus"), 2)  # original judge unknown: cannot prove independence
        self.assertEqual(self.record_with("haiku (gating)", "opus", "--exclude", "mystery"), 2)
        self.assertEqual(sp.family(""), "")
        self.assertEqual(sp.family("orchestrator"), sp.ORCHESTRATOR_FAMILY)

    def test_refuses_not_due_duplicate_bad_result_empty_evidence(self):
        keys = self.selected_keys(1)
        k = keys[0]
        other = next(f"src/z.rs:{i}" for i in range(100) if not sp.is_selected("q-A", f"src/z.rs:{i}"))
        self.write_verdicts([vrow("q-A", k), vrow("q-A", other)])
        for args in (
            (f"q-A|{other}", "real", "opus", "ev"),  # selected? no: not in the sample
            (f"q-A|{k}", "maybe", "opus", "ev"),
            (f"q-A|{k}", "real", "opus", "  "),
            (f"q-A|{k}", "real", "  ", "ev"),
            ("nokey", "real", "opus", "ev"),
            (f"q-NO|{k}", "real", "opus", "ev"),
        ):
            self.assertEqual(self.run_main("--record", *args)[0], 2, args)
        self.assertFalse((self.root / sp.SPOT_FILE).exists())
        self.assertEqual(self.run_main("--record", f"q-A|{k}", "real", "opus", "ev")[0], 0)
        self.assertEqual(self.run_main("--record", f"q-A|{k}", "real", "opus", "ev")[0], 2)  # already recorded

    def test_never_edits_verdicts_file(self):
        k = self.selected_keys(1)[0]
        self.write_verdicts([vrow("q-A", k)])
        before = (self.root / sp.VERDICT_FILE).read_bytes()
        self.run_main("--record", f"q-A|{k}", "real", "opus", "ev")
        self.run_main("--check")
        self.assertEqual((self.root / sp.VERDICT_FILE).read_bytes(), before)


class CheckTests(Base):
    def test_check_exits_1_while_open_then_0(self):
        keys = self.selected_keys(2)
        self.write_verdicts([vrow("q-A", k) for k in keys])
        code, out, _ = self.run_main("--check")
        self.assertEqual(code, 1)
        self.assertIn("2 of 2", out)
        self.assertIn(f"q-A|{keys[0]}", out)
        self.assertEqual(self.run_main()[0], 0)  # plain listing never fails
        self.run_main("--record", f"q-A|{keys[0]}", "real", "opus", "ev")
        self.assertEqual(self.run_main("--check")[0], 1)
        self.run_main("--record", f"q-A|{keys[1]}", "false", "opus", "ev")
        code, out, _ = self.run_main("--check")
        self.assertEqual(code, 0)
        self.assertIn("0 of 2", out)


class MalformedTests(Base):
    def test_malformed_rows_fail_closed(self):
        k = self.selected_keys(1)[0]
        good = vrow("q-A", k)
        for label, header, rows in (
            ("short row", HEADER, [good, "q-A\tx\tfalse"]),
            ("extra column", HEADER, [good + "\textra"]),
            ("bad verdict", HEADER, [good, vrow("q-A", "k2", "maybe")]),
            ("empty key", HEADER, [good, vrow("q-A", " ")]),
            ("bad header", "entry\tkey\tverdict\tevidence\tjudge\tdate", [good]),
            ("blank line", HEADER, [good, ""]),
        ):
            self.write_verdicts(rows, header)
            for args in (("--check",), (), ("--record", f"q-A|{k}", "real", "opus", "ev")):
                code, _, err = self.run_main(*args)
                self.assertEqual(code, 2, (label, args))
                self.assertIn("refused", err)
            self.assertFalse((self.root / sp.SPOT_FILE).exists(), label)

    def test_malformed_spotcheck_file_fails_closed(self):
        k = self.selected_keys(1)[0]
        self.write_verdicts([vrow("q-A", k)])
        (self.root / sp.SPOT_FILE).write_text(sp.SPOT_HEADER + "\nq-A\t" + k + "\tmaybe\topus\tev\td\n", encoding="utf-8", newline="")
        self.assertEqual(self.run_main("--check")[0], 2)

    def test_missing_file_is_empty_not_error(self):
        self.assertEqual(self.run_main("--check")[0], 0)


if __name__ == "__main__":
    unittest.main()
