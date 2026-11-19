"""Tests for bench/instruct: no network and no model. Canned answers are hand-written (canned.py).

Run (from the repo root):  python -I -B -m unittest discover -s bench/instruct -v
Temp output goes under the scratch folder named in broadsword.toml and is removed after each test.
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, re, shutil, subprocess, tempfile, tomllib, unittest
from collections import Counter

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parents[1]
SCRATCH = ROOT / tomllib.loads((ROOT / "broadsword.toml").read_text(encoding="utf-8"))["scratch_dir"]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    return m


CK = load("t_checks", HERE / "checks.py")
MK = load("t_make", HERE / "make_cases.py")
CAN = load("t_canned", HERE / "canned.py")
CASES = {c["id"]: c for c in json.loads((HERE / "cases.json").read_text(encoding="utf-8"))}


def scratch_dir():
    SCRATCH.mkdir(parents=True, exist_ok=True)
    return tempfile.TemporaryDirectory(dir=SCRATCH, prefix="instruct-")


def log_lines(prompt):
    body = prompt.split("=== LOG START ===\n", 1)[1].split("\n=== LOG END ===", 1)[0]
    return [l for l in body.split("\n") if not l.startswith("#")]


def good_answer(cid):
    """The good answer of a case. Cases 15, 16 and 19 are derived by counting the log in the prompt a second way."""
    if cid in CAN.GOOD:
        return CAN.GOOD[cid]
    lines = log_lines(CASES[cid]["prompt"])
    parsed = [re.match(r"(\S+) (\w+)\s+(\S+)\s", l).groups() for l in lines]
    if cid == "15-top-warn":
        c = Counter(s for _, lv, s in parsed if lv == "WARN").most_common(3)
        return "\n".join(f"{s}: {n}" for s, n in c)
    if cid == "16-json-span":
        return json.dumps({"first_ts": parsed[0][0], "last_ts": parsed[-1][0], "total_lines": len(parsed)})
    if cid == "19-level-table":
        c = Counter(lv for _, lv, _ in parsed)
        return f"| Level | Count |\n|---|---|\n| INFO | {c['INFO']} |\n| WARN | {c['WARN']} |\n| ERROR | {c['ERROR']} |"
    raise KeyError(cid)


class CaseSet(unittest.TestCase):
    def test_cases_json_is_what_make_cases_builds(self):
        self.assertEqual(MK.build(), list(CASES.values()))

    def test_twenty_cases_mixed_short_and_long(self):
        self.assertEqual(len(CASES), 20)
        long_ = [c for c in CASES.values() if c["kind"] == "long"]
        self.assertGreaterEqual(len(long_), 5)
        for c in long_:
            self.assertGreater(len(c["prompt"]), 20000, c["id"])  # about 6k tokens
            self.assertTrue(c["prompt"].startswith("RULES:"), c["id"])  # the rule is at the top
            self.assertLess(c["prompt"].index("=== LOG START ==="), 700, c["id"])
        self.assertGreaterEqual(len([c for c in CASES.values() if c["kind"] == "short"]), 10)

    def test_every_check_type_is_known_and_used_and_proved(self):
        used = {k["type"] for c in CASES.values() for k in c["checks"]}
        self.assertEqual(used, set(CK.CHECKS), "every implemented check type must be used by a case, and the reverse")
        broken_by_bad = {rule for bad in CAN.BAD.values() for _, rule in bad}
        self.assertEqual(broken_by_bad & set(CK.CHECKS), set(CK.CHECKS), "every check type needs a canned answer that breaks it")

    def test_empty_answer_fails_every_case(self):
        for cid, c in CASES.items():
            self.assertTrue(CK.grade(c, ""), f"{cid}: an empty answer must not pass")
            self.assertTrue(CK.grade(c, "   \n\n"), cid)

    def test_good_answers_pass(self):
        for cid, c in CASES.items():
            self.assertEqual(CK.grade(c, good_answer(cid)), [], cid)

    def test_good_answers_exist_for_every_case_and_bad_too(self):
        for cid in CASES:
            good_answer(cid)
            self.assertTrue(CAN.BAD.get(cid), cid)

    def test_bad_answers_fail_with_the_named_rule(self):
        for cid, bad in CAN.BAD.items():
            for answer, rule in bad:
                broken = CK.grade(CASES[cid], answer)
                self.assertIn(rule, [t for t, _ in broken], f"{cid}: {answer[:50]!r} -> {broken}")

    def test_the_log_expected_values_match_an_independent_count(self):
        # the checks in cases.json hold the generator's numbers; the canned good answer used a second counting method
        for cid in ("15-top-warn", "16-json-span", "19-level-table"):
            self.assertEqual(CK.grade(CASES[cid], good_answer(cid)), [], cid)

    def test_a_check_can_fail_the_unit_pairs(self):
        # one direct pass/fail pair per check type, so a broken check function cannot hide behind a case
        pairs = {
            "bullets": ({"n": 2}, "- a\n- b", "- a\n- b\n- c"),
            "numbered": ({"n": 2}, "1. a\n2. b", "1. a\n3. b"),
            "lines_count": ({"n": 2}, "a\nb", "a"),
            "json_keys": ({"keys": ["a"]}, '{"a": 1}', '{"b": 1}'),
            "max_words": ({"k": 2}, "a b", "a b c"),
            "min_words": ({"k": 2}, "a b", "a"),
            "max_chars": ({"k": 3}, "abc", "abcd"),
            "forbidden": ({"words": ["cat"]}, "category", "A CAT sat"),
            "forbidden_chars": ({"chars": ","}, "a b", "a,b"),
            "ascii_only": ({}, "abc", "café"),
            "lowercase": ({}, "abc", "aBc"),
            "no_markdown": ({}, "abc", "a*c"),
            "sentences": ({"n": 1}, "One.", "One. Two."),
            "keep_line": ({"lines": ["x  y"]}, "a\nx  y", "a\nx y"),
            "first_line": ({"text": "A"}, "A\nB", "B\nA"),
            "last_line": ({"text": "A"}, "B\nA", "A\nB"),
            "full_match": ({"regex": r"\d+"}, "12", "12a"),
            "equals": ({"text": "a"}, "a\n", "ab"),
            "contains": ({"strings": ["x"]}, "axb", "ab"),
            "sorted_lines": ({}, "a\nb", "b\na"),
            "table": ({"header": "| A |", "columns": 1, "rows": 1}, "| A |\n|---|\n| 1 |", "| A |\n|---|\n| 1 | 2 |"),
        }
        self.assertEqual(set(pairs), set(CK.CHECKS))
        for name, (params, good, bad) in pairs.items():
            self.assertIsNone(CK.CHECKS[name](good, params), name)
            self.assertIsNotNone(CK.CHECKS[name](bad, params), name)

    def test_forbidden_chars_ignores_list_markers_only_when_asked(self):
        p = {"chars": "0123456789", "ignore_list_markers": True}
        self.assertIsNone(CK.c_forbidden_chars("1. a\n2. b", p))
        self.assertIsNotNone(CK.c_forbidden_chars("1. a 3\n2. b", p))
        self.assertIsNotNone(CK.c_forbidden_chars("1. a", {"chars": "0123456789"}))

    def test_forbidden_substring_mode_and_unknown_type(self):
        self.assertIsNone(CK.c_forbidden("many services", {"words": ["service"]}))  # whole word: not a hit
        self.assertIsNotNone(CK.c_forbidden("many services", {"words": ["service"], "mode": "substring"}))
        with self.assertRaises(ValueError):
            CK.grade({"id": "x", "checks": [{"type": "nope"}]}, "a")

    def test_sentences_real_model_style(self):
        c = CASES["09-lowercase-no-commas"]
        # the real Haiku answer: one lowercase sentence, no comma, no final full stop. Must pass.
        real = "cats are independent creatures that spend much of their day sleeping and still manage to rule the household with quiet confidence"
        self.assertEqual(CK.grade(c, real), [])
        self.assertEqual(CK.grade(c, real + "."), [])  # a final stop adds no empty sentence
        two = "cats sleep a lot. they hunt at night"  # two sentences, the last without a stop
        self.assertIn(("sentences", "expected exactly 1 sentence(s), got 2"), CK.grade(c, two))
        self.assertEqual(CK.count_sentences("Use tools, e.g. a hammer, and measure 3.5 cm. Then stop."), 2)
        self.assertEqual(CK.count_sentences("Dr. Who i.e. the doctor arrived vs. the daleks!"), 1)
        self.assertEqual(CK.count_sentences("Wait... what? Yes!"), 3)
        self.assertEqual(CK.count_sentences(""), 0)

    def test_blank_lines_and_trailing_newline_never_matter(self):
        single = {"02-json-keys", "10-exact-answer", "12-date-format"}  # one-line answers: no per-line trailing spaces
        for cid in ("01-bullets", "02-json-keys", "06-table-header", "07-first-last-line", "08-numbered",
                    "10-exact-answer", "11-sorted-lines", "12-date-format", "18-keep-header", "20-lowercase-numbered"):
            a = good_answer(cid)
            variants = ["\n\n" + a + "\n\n", a + "\r\n", a.replace("\n", "\r\n") + "\r\n"]
            if cid not in single:
                variants.append("  \n" + a.replace("\n", "  \n") + "  \n")
            for v in variants:
                self.assertEqual(CK.grade(CASES[cid], v), [], f"{cid}: {v!r}")

    def test_documented_strictness_stays_strict(self):
        # by design (README): the rule in the prompt is exact, so these are FAILs, not tolerated
        self.assertTrue(CK.grade(CASES["12-date-format"], "2024-02-29."))  # final stop on a date answer
        self.assertTrue(CK.grade(CASES["13-char-limit-ascii"], "Bean There opens Monday — come in"))  # non-ASCII dash
        self.assertTrue(CK.grade(CASES["01-bullets"], "• a\n• b\n• c\n• d"))  # bullet glyph, not '- '
        self.assertTrue(CK.grade(CASES["02-json-keys"], "```json\n" + good_answer("02-json-keys") + "\n```"))

    def test_formatting_tolerances_are_only_whitespace(self):
        c = CASES["18-keep-header"]
        trailing = "\n".join(CAN.HEADER) + "  \nEND-OF-HEADER\n\n"  # trailing spaces and blank lines are ignored
        self.assertEqual(CK.grade(c, trailing.replace("\n", "\r\n")), [])


def write_result(d, case, model, answer, **kw):
    rec = {"provider": "claude", "model": model, "effort": "default", "case": case, "answer": answer, "tokens": {},
           "cost_usd": 0.001, "wall_s": 1.0, "exit": 0, "is_error": False, "call_failed": False, "error": None, "model_used": []}
    rec.update(kw)
    (d / f"{case}__claude__{model}__default.json").write_text(json.dumps(rec), encoding="utf-8")
    (d / f"{case}__claude__{model}__default.raw.json").write_text("{}", encoding="utf-8")


class ScoreScript(unittest.TestCase):
    def test_totals_exclude_errors(self):
        with scratch_dir() as tmp:
            d = pathlib.Path(tmp)
            write_result(d, "01-bullets", "m", good_answer("01-bullets"))  # PASS
            write_result(d, "02-json-keys", "m", "```json\n" + good_answer("02-json-keys") + "\n```")  # FAIL: fence
            write_result(d, "10-exact-answer", "m", "")  # FAIL: empty answer, no error
            write_result(d, "12-date-format", "m", "", is_error=True, call_failed=True, error="model call timed out after 1s")  # ERROR
            write_result(d, "03-word-limit", "m", "", is_error=True, call_failed=True, error="the model refused: no")  # ERROR
            r = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(d)], capture_output=True, text=True, encoding="utf-8")
            self.assertEqual(r.returncode, 0, r.stderr)
            out = r.stdout
            self.assertRegex(out, r"01-bullets\s+claude/m/default\s+PASS")
            self.assertRegex(out, r"02-json-keys\s+claude/m/default\s+FAIL\s+json_keys: the whole answer must be one JSON object")
            self.assertRegex(out, r"10-exact-answer\s+claude/m/default\s+FAIL\s+equals")
            self.assertRegex(out, r"12-date-format\s+claude/m/default\s+ERROR\s+model call timed out")
            self.assertRegex(out, r"03-word-limit\s+claude/m/default\s+ERROR\s+the model refused")
            # pass 1, fail 2, scored 3, rate 33%, errors 2, not run 15 (20 cases - 5 seen)
            self.assertRegex(out, r"claude/m/default\s+1\s+2\s+3\s+33%\s+2\s+15\s")

    def test_a_model_with_only_errors_has_no_rate(self):
        with scratch_dir() as tmp:
            d = pathlib.Path(tmp)
            write_result(d, "01-bullets", "e", "", is_error=True, call_failed=True, error="x")
            r = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(d)], capture_output=True, text=True, encoding="utf-8")
            self.assertRegex(r.stdout, r"claude/e/default\s+0\s+0\s+0\s+n/a\s+1\s+19\s")

    def test_perfect_run_scores_100_percent(self):
        with scratch_dir() as tmp:
            d = pathlib.Path(tmp)
            for cid in CASES:
                write_result(d, cid, "p", good_answer(cid))
            r = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(d)], capture_output=True, text=True, encoding="utf-8")
            self.assertRegex(r.stdout, r"claude/p/default\s+20\s+0\s+20\s+100%\s+0\s+0\s")

    def test_all_wrong_format_run_scores_zero(self):
        with scratch_dir() as tmp:
            d = pathlib.Path(tmp)
            for cid in CASES:
                write_result(d, cid, "w", "Sure! Here is my answer, with **markdown**, 1, 2, 3.")
            r = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(d)], capture_output=True, text=True, encoding="utf-8")
            self.assertRegex(r.stdout, r"claude/w/default\s+0\s+20\s+20\s+0%\s+0\s+0\s")


FAKE_OK = ("import json, sys\n"
           "sys.stdin.buffer.read()\n"
           "print(json.dumps({'result': '391', 'is_error': False, 'total_cost_usd': 0.002, 'usage': {'input_tokens': 5, 'output_tokens': 1}, 'modelUsage': {'fake': {}}}))\n")
FAKE_SLOW = "import time\ntime.sleep(60)\n"


class RunScript(unittest.TestCase):
    def run_main(self, cli_text, timeout, cases):
        with scratch_dir() as tmp, tempfile.TemporaryDirectory() as work:
            tmp = pathlib.Path(tmp)
            cli = tmp / "fake_cli.py"
            cli.write_text(cli_text, encoding="utf-8")
            os.environ["BENCH_CLAUDE"] = str(cli)
            os.environ["BENCH_CWD"] = work  # outside the project: system temp
            run = load("t_run", HERE / "run.py")
            run.CR.CFG["model_timeout_seconds"] = timeout
            out = tmp / "out"
            buf = io.StringIO()
            with contextlib.redirect_stdout(buf):
                run.main([str(out), "claude", "fake", "default", *cases])
            res = {f.name: json.loads(f.read_text(encoding="utf-8")) for f in sorted(out.glob("*.json")) if not f.name.endswith(".raw.json")}
            sc = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(out)], capture_output=True, text=True, encoding="utf-8")
            return res, sc.stdout

    def test_fake_cli_answer_is_recorded_and_scored(self):
        res, score = self.run_main(FAKE_OK, 30, ["10-exact-answer", "12-date-format"])
        self.assertEqual(len(res), 2)
        r = res["10-exact-answer__claude__fake__default.json"]
        self.assertEqual((r["answer"], r["call_failed"], r["cost_usd"]), ("391", False, 0.002))
        self.assertRegex(score, r"10-exact-answer\s+claude/fake/default\s+PASS")
        self.assertRegex(score, r"12-date-format\s+claude/fake/default\s+FAIL\s+full_match")

    def test_timeout_is_an_error_not_a_fail_and_the_run_goes_on(self):
        res, score = self.run_main(FAKE_SLOW, 1, ["10-exact-answer", "12-date-format"])
        self.assertEqual(len(res), 2)
        for r in res.values():
            self.assertTrue(r["call_failed"])
        self.assertRegex(score, r"10-exact-answer\s+claude/fake/default\s+ERROR\s+model call timed out")
        self.assertRegex(score, r"claude/fake/default\s+0\s+0\s+0\s+n/a\s+2\s+18\s")


if __name__ == "__main__":
    unittest.main()
