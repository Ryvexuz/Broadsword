"""Tests for scripts/shadow_report.py (--open, summary). Each test builds a scratch
root with fake queue entries under the project's scratch folder and removes it afterwards.

  python -I -B scripts/test_shadow_report.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import pathlib
import re
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
REAL_ROOT = HERE.parent


def load(name):
    spec = importlib.util.spec_from_file_location(name, HERE / f"{name}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


sr = load("shadow_report")
sa = load("shadow_adjudicate")


def scratch_tmp():
    m = re.search(r'^scratch_dir\s*=\s*"([^"]+)"', (REAL_ROOT / "broadsword.toml").read_text(encoding="utf-8"), re.M)
    if not m:
        raise RuntimeError("broadsword.toml has no scratch_dir")
    base = REAL_ROOT / m.group(1)
    base.mkdir(parents=True, exist_ok=True)
    return tempfile.TemporaryDirectory(dir=base, prefix="test-shadow-adj-", ignore_cleanup_errors=True)


def entry(status, verdict, blocking, shadow_blocking, shadow_verdict="FAIL", model="gpt-x"):
    return (
        f"status = {status}\nverdict = {verdict}\nblocking = {blocking}\nshadow_verdict = {shadow_verdict}\n"
        f"shadow_blocking = {shadow_blocking}\nshadow_model_used = {model}\nmain_sha = abc123\nchecked_tip = def456\n"
    )


def run(fn, *args):
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = fn(list(args))
    return code, out.getvalue(), err.getvalue()


class Base(unittest.TestCase):
    def setUp(self):
        self._tmp = scratch_tmp()
        self.addCleanup(self._tmp.cleanup)
        self.root = pathlib.Path(self._tmp.name)
        self.q = self.root / "state" / "queue"
        self.q.mkdir(parents=True)

    def put(self, eid, text):
        (self.q / f"{eid}.entry").write_text(text, encoding="utf-8")

    def verdicts_text(self):
        p = self.root / "state" / "shadow-verdicts.tsv"
        return p.read_text(encoding="utf-8") if p.is_file() else ""

    def report(self, *a):
        return run(sr.main, "--root", str(self.root), *a)

    def add(self, *a):
        return run(sa.main, "--root", str(self.root), "add", *a)

    def fixture(self):
        # A merged, gating PASS: three shadow-only lines (real, false, known) plus one both reviewers found.
        self.put("q-A", entry("merged", "PASS", r"shared.rs:9 - both found it", "src/a.rs:1 - one\\nsrc/a.rs:2 - two\\nsrc/a.rs:3 - three\\nshared.rs:9 - both"))
        # B not merged, gating FAIL: a third bug next to the one that blocked.
        self.put("q-B", entry("failed", "FAIL", "src/b.rs:5 - blocked here", "src/b.rs:5 - blocked here\\nsrc/b.rs:6 - third bug"))
        # C: shadow errored, no findings.
        self.put("q-C", entry("merged", "PASS", "", "", shadow_verdict="ERROR"))


class OpenTests(Base):
    def test_lists_shadow_only_lines_and_exits_3(self):
        self.fixture()
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        for key in ("src/a.rs:1", "src/a.rs:2", "src/a.rs:3", "src/b.rs:6"):
            self.assertIn(key, out)
        self.assertIn("q-A\tabc123\tmerged", out)  # entry id, commit, finding
        self.assertIn("q-B\tdef456\tnot merged", out)
        self.assertNotIn("shared.rs:9", out)  # the gating review found it too
        self.assertNotIn("src/b.rs:5", out)
        self.assertNotIn("q-C", out)  # shadow ERROR: skipped

    def test_stops_listing_once_a_verdict_exists_and_exits_0_when_none_left(self):
        self.fixture()
        self.assertEqual(self.add("q-A", "src/a.rs:1", "real", "--evidence", "x", "--judge", "m")[0], 0)
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertNotIn("src/a.rs:1\t", out)
        self.assertIn("src/a.rs:2", out)
        for e, k in (("q-A", "src/a.rs:2"), ("q-A", "src/a.rs:3"), ("q-B", "src/b.rs:6")):
            self.assertEqual(self.add(e, k, "known", "--evidence", "x", "--judge", "m")[0], 0)
        code, out, _ = self.report("--open")
        self.assertEqual((code, out.strip()), (0, "no open shadow-only findings"))

    def test_multi_line_finding_is_one_open_line(self):
        self.put("q-A", entry("merged", "PASS", "", "src/a.rs:1 - quote: \\u201cBEGIN\\nmiddle\\nEND\\u201d passes\\nsrc/a.rs:2 - two"))
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("2 open shadow-only finding(s)", out)
        self.assertIn("BEGIN middle END", out)

    def test_text_only_bullet_is_its_own_finding(self):
        self.put("q-A", entry("merged", "PASS", "", "- a.py:1 - x\\n- the brief is not met"))
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("2 open shadow-only finding(s)", out)
        self.assertIn("the brief is not met", out)
        self.add("q-A", "a.py:1", "real", "--evidence", "e", "--judge", "m")
        code, out, _ = self.report("--open")  # judging the first must not hide the second
        self.assertEqual(code, 3)
        self.assertIn("1 open shadow-only finding(s)", out)
        self.assertIn("the brief is not met", out)

    def test_indented_line_and_numbered_bullets(self):
        self.put("q-A", entry("merged", "PASS", "", "1. a.py:1 - x\\n   deeper detail\\n2. b.py:2 - y\\n3. no path here"))
        _, out, _ = self.report("--open")
        self.assertIn("3 open shadow-only finding(s)", out)
        self.assertIn("x deeper detail", out)

    def two_at_one_line(self):
        self.put("q-A", entry("merged", "PASS", "", "a.py:10 - the loop never ends here\\na.py:10 - a second, different bug appears"))

    def hand_row(self, key):
        (self.root / "state" / "shadow-verdicts.tsv").write_text(
            sr.VERDICT_HEADER + f"\nq-A\t{key}\treal\tevidence\thaiku\t2026-11-19\n", encoding="utf-8")

    def test_two_findings_at_one_location_stay_two_open_lines(self):
        self.two_at_one_line()
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("2 open shadow-only finding(s)", out)
        self.assertIn("a.py:10#the loop never ends here", out)
        self.assertIn("a.py:10#a second different bug appears", out)

    def test_bare_location_verdict_closes_none_when_several_and_says_why(self):
        self.two_at_one_line()
        self.hand_row("a.py:10")
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("2 open shadow-only finding(s)", out)
        self.assertIn("verdict for bare a.py:10 ignored: 2 findings there", out)
        s = sr.summarize(sr.shadow_only(sr.load_entries(self.root)), sr.load_verdicts(self.root))["gpt-x"]
        self.assertEqual((s["findings"], s["judged"]), (2, 0))
        code, _, err = self.add("q-A", "a.py:10", "real", "--evidence", "e", "--judge", "m")
        self.assertEqual(code, 2)
        self.assertIn("ambiguous", err)

    def test_full_key_verdict_closes_exactly_one(self):
        self.two_at_one_line()
        self.hand_row("a.py:10#the loop never ends here")
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("1 open shadow-only finding(s)", out)
        self.assertIn("a second, different bug", out)
        self.assertNotIn("the loop never ends\t", out)
        self.assertEqual(self.add("q-A", "a.py:10#a second different bug appears", "false", "--evidence", "e", "--judge", "m")[0], 0)
        self.assertEqual(self.report("--open")[0], 0)

    def test_bare_location_verdict_closes_a_lone_finding(self):
        self.put("q-A", entry("merged", "PASS", "", "a.py:10 - only one here"))
        self.hand_row("a.py:10")
        self.assertEqual(self.report("--open")[0], 0)

    def test_text_only_first_line_starts_its_own_finding(self):
        self.put("q-A", entry("merged", "PASS", "", "the brief is not met\\na.py:1 - x"))
        _, out, _ = self.report("--open")
        self.assertIn("2 open shadow-only finding(s)", out)
        self.assertIn("the brief is not met", out)

    def test_tilde_line_key(self):
        self.assertEqual(sr.finding_key("README.md:~70 - x"), "readme.md:70")
        self.put("q-A", entry("merged", "PASS", "", "README.md:~70 - x"))
        self.assertEqual(self.add("q-A", "README.md:~70", "known", "--evidence", "e", "--judge", "m")[0], 0)
        self.assertEqual(self.add("q-A", "readme.md:70", "known", "--evidence", "e", "--judge", "m")[0], 0)
        self.assertEqual(self.report("--open")[0], 0)

    def test_second_shadow_bug_at_a_gating_location_stays_open(self):
        # gating blocked a.py:10 for the missing lock; the shadow reports that AND a different bug at the same line
        self.put("q-A", entry("failed", "FAIL", "a.py:10 - the lock is never released on error",
                              "a.py:10 - the lock is never released on error path\\na.py:10 - integer overflow in the size sum"))
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("1 open shadow-only finding(s)", out)
        self.assertIn("integer overflow in the size sum", out)
        self.assertNotIn("lock is never released on error path", out)

    def test_same_bug_at_a_gating_location_is_not_open_whatever_the_order(self):
        self.put("q-A", entry("failed", "FAIL", "a.py:10 - lock never released on error",
                              "a.py:10 - integer overflow in the size sum\\na.py:10 - the lock is never released on error"))
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("1 open shadow-only finding(s)", out)
        self.assertIn("integer overflow", out)
        self.assertNotIn("never released", out)

    def test_same_location_same_count_is_all_known(self):
        self.put("q-A", entry("failed", "FAIL", "a.py:10 - one thing", "a.py:10 - totally other words"))
        self.assertEqual(self.report("--open")[0], 0)

    def test_unbulleted_text_line_after_a_located_finding_is_a_continuation(self):
        # documented limit (reviewer round 2 and 3 minor): without a path or bullet a line cannot be told from a wrapped quote
        self.put("q-A", entry("merged", "PASS", "", "a.py:1 - first bug\\nplain follow-on words"))
        code, out, _ = self.report("--open")
        self.assertEqual(code, 3)
        self.assertIn("1 open shadow-only finding(s)", out)
        self.assertIn("first bug plain follow-on words", out)

    def test_no_entries_exits_0(self):
        code, out, _ = self.report("--open")
        self.assertEqual((code, out.strip()), (0, "no open shadow-only findings"))

    def test_hand_written_row_with_mixed_case_key_closes_a_line(self):
        self.put("q-A", entry("merged", "PASS", "", "docs/STAGE6.md:18 - x"))
        (self.root / "state" / "shadow-verdicts.tsv").write_text(
            sr.VERDICT_HEADER + "\nq-A\tdocs/STAGE6.md:18\tknown\tsame\torchestrator (matched)\t2026-11-19\n", encoding="utf-8")
        self.assertEqual(self.report("--open")[0], 0)


class ScoreTests(Base):
    def test_precision_and_misses(self):
        self.fixture()
        self.add("q-A", "src/a.rs:1", "real", "--evidence", "e", "--judge", "m")  # merged: a gating miss
        self.add("q-A", "src/a.rs:2", "false", "--evidence", "e", "--judge", "m")
        self.add("q-A", "src/a.rs:3", "known", "--evidence", "e", "--judge", "m")
        self.add("q-B", "src/b.rs:6", "real", "--evidence", "e", "--judge", "m")  # not merged: real, not a miss
        stats = sr.summarize(sr.shadow_only(sr.load_entries(self.root)), sr.load_verdicts(self.root))
        s = stats["gpt-x"]
        self.assertEqual((s["findings"], s["judged"], s["real"], s["false"], s["known"], s["misses"]), (4, 4, 2, 1, 1, 1))
        self.assertAlmostEqual(s["precision"], 2 / 3)
        code, out, _ = self.report()
        self.assertEqual(code, 0)
        self.assertIn("gpt-x\t4\t4\t2\t1\t1\t2/3 = 67%\t1", out)

    def test_precision_n_a_with_only_known_and_open_findings(self):
        self.fixture()
        self.add("q-A", "src/a.rs:3", "known", "--evidence", "e", "--judge", "m")
        s = sr.summarize(sr.shadow_only(sr.load_entries(self.root)), sr.load_verdicts(self.root))["gpt-x"]
        self.assertIsNone(s["precision"])
        self.assertEqual((s["findings"], s["judged"]), (4, 1))

    def test_per_model(self):
        self.put("q-A", entry("merged", "PASS", "", "a.rs:1 - x", model="m1"))
        self.put("q-B", entry("merged", "PASS", "", "b.rs:1 - y", model="m2"))
        self.add("q-A", "a.rs:1", "real", "--evidence", "e", "--judge", "j")
        self.add("q-B", "b.rs:1", "false", "--evidence", "e", "--judge", "j")
        s = sr.summarize(sr.shadow_only(sr.load_entries(self.root)), sr.load_verdicts(self.root))
        self.assertEqual((s["m1"]["precision"], s["m1"]["misses"]), (1.0, 1))
        self.assertEqual((s["m2"]["precision"], s["m2"]["misses"]), (0.0, 0))


if __name__ == "__main__":
    unittest.main()
