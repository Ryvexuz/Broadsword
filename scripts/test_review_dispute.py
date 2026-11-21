"""Tests for scripts/review_dispute.py. No model is called: the test run and the review client are replaced by fakes (the two seams of
dispute()), and call_client() is exercised against a fake client script that records what it was given.

Run:  python -I -B -m unittest discover -s scripts -p "test_review_dispute.py" -v
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import importlib.util
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("review_dispute", HERE / "review_dispute.py")
rd = importlib.util.module_from_spec(_spec)
sys.modules["review_dispute"] = rd
_spec.loader.exec_module(rd)

TOML = """root = "."
install_dir = "build/bin"
state_dir = "state"
projects_dir = "projects"
reviewer_diff_max_kb = 50
reviewer_check_tail_kb = 1
reviewer_brief_max_kb = 4
reviewer_timeout_seconds = 60
shadow_reviewer_timeout_seconds = 120
redact_min_chars = 6
"""
FINDING = "src/lib.rs:10 - the parser accepts a negative length"
BRIEF_SENTINEL = "BRIEFSENTINEL-argue-for-the-coder"
PASS_OUT = "running 1 test\ntest tests::negative_length_is_refused ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored\n"


def git(root, *a):
    subprocess.run(["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t", *a], check=True, capture_output=True)


def good_reply(ruling="false", model="gpt-6.2-astra-2026-01-01"):
    return {"is_error": False, "result": f"The test calls the parser with -1 and it is refused.\nRULING: {ruling}", "modelUsage": {model: {}}}


class Env:
    """A scratch Broadsword root that is also the git repo of project `broadsword`, with one queue entry."""

    def __init__(self, tmp, reviewer="opus", coder="sonnet", test_line="fn negative_length_is_refused() {}", extra_diff=""):
        self.root = Path(tmp)
        (self.root / "broadsword.toml").write_text(TOML, encoding="utf-8")
        (self.root / "state" / "queue").mkdir(parents=True)
        git(self.root, "init", "-q")
        (self.root / "a.txt").write_text("base\n")
        git(self.root, "add", "a.txt")
        git(self.root, "commit", "-q", "-m", "base")
        self.base = subprocess.run(["git", "-C", str(self.root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        (self.root / "b.rs").write_text(test_line + "\n" + extra_diff)
        git(self.root, "add", "b.rs")
        git(self.root, "commit", "-q", "-m", "change")
        self.tip = subprocess.run(["git", "-C", str(self.root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        self.entry = "q-20261121-050000"
        lines = [
            "project = broadsword", "task = t-1", f"coder = {coder}", f"reviewer_model = {reviewer}",
            f"base_head = {self.base}", f"checked_tip = {self.tip}",
            f"blocking = {FINDING}\\nsecond finding", "shadow_blocking = ", "minor = ", f"brief = {BRIEF_SENTINEL}",
        ]
        (self.root / "state" / "queue" / f"{self.entry}.entry").write_text("\n".join(lines) + "\n", encoding="utf-8")
        self.prompts = []
        self.runs = []

    def runner(self, out=PASS_OUT, code=0):
        def f(root, cfg, project, task, test):
            self.runs.append((project, task, test))
            return code, out
        return f

    def caller(self, reply):
        def f(root, cfg, model, prompt, environ):
            self.prompts.append(prompt)
            return reply
        return f

    def go(self, model="gpt-6.2-astra", test="negative_length_is_refused", finding=FINDING, runner=None, caller=None, excludes=(), environ=None):
        return rd.dispute(self.root, self.entry, finding, test, model, excludes, environ or {}, runner or self.runner(), caller or self.caller(good_reply()))

    def rows(self):
        p = self.root / "state" / "review-disputes.tsv"
        return p.read_text(encoding="utf-8").split("\n")[:-1] if p.is_file() else []


class DisputeTests(unittest.TestCase):
    def setUp(self):
        self._t = tempfile.TemporaryDirectory()
        self.addCleanup(self._t.cleanup)
        self.env = Env(self._t.name)

    def test_false_ruling_is_recorded_and_prompt_holds_only_finding_diff_and_test_output(self):
        ruling, row = self.env.go()
        self.assertEqual(ruling, "false")
        rows = self.env.rows()
        self.assertEqual(rows[0], rd.HEADER)
        cols = rows[1].split("\t")
        self.assertEqual(len(cols), 10)
        self.assertEqual((cols[0], cols[3], cols[4][:6], cols[5], cols[7]), (self.env.entry, "negative_length_is_refused", "passed", "gpt-6.2-astra", "false"))
        self.assertIn("medium NOT applied", cols[6])
        self.assertEqual(self.env.runs, [("broadsword", "t-1", "negative_length_is_refused")])
        prompt = self.env.prompts[0]
        self.assertIn(FINDING, prompt)
        self.assertIn("+fn negative_length_is_refused", prompt)
        self.assertIn("1 passed", prompt)
        self.assertNotIn(BRIEF_SENTINEL, prompt)  # the brief never reaches the tie-break model
        self.assertNotIn("second finding", prompt)

    def test_real_and_unsure_rulings(self):
        for r in ("real", "unsure"):
            self.assertEqual(self.env.go(caller=self.env.caller(good_reply(r)), finding=FINDING + " ")[0], r)

    def test_failing_test_is_recorded_and_no_model_is_asked(self):
        ruling, _ = self.env.go(runner=self.env.runner("test result: FAILED. 0 passed; 1 failed", 101))
        self.assertEqual(ruling, "not_asked")
        self.assertEqual(self.env.prompts, [])
        self.assertIn("failed: exit code 101", self.env.rows()[1])

    def test_vacuous_pass_is_not_a_pass(self):
        ruling, _ = self.env.go(runner=self.env.runner("running 0 tests\ntest result: ok. 0 passed; 0 failed\n", 0))
        self.assertEqual(ruling, "not_asked")
        self.assertEqual(self.env.prompts, [])
        self.assertIn("no test reported as run", self.env.rows()[1])

    def test_unittest_output_counts_and_zero_tests_does_not(self):
        self.assertEqual(rd.test_passed("test_x ... ok\n\nRan 1 test in 0.001s\n\nOK\n", 0)[0], True)
        self.assertEqual(rd.test_passed("Ran 0 tests in 0.000s\n\nOK\n", 0)[0], False)
        self.assertEqual(rd.test_passed("Ran 1 test in 0.001s\n\nFAILED (failures=1)\n", 0)[0], False)

    def test_test_must_be_on_an_added_line(self):
        with self.assertRaises(rd.Refused) as c:
            self.env.go(test="some_other_test")
        self.assertIn("not on an added line", str(c.exception))
        self.assertEqual((self.env.runs, self.env.rows()), ([], []))

    def test_finding_must_have_been_raised(self):
        with self.assertRaises(rd.Refused) as c:
            self.env.go(finding="a finding nobody raised")
        self.assertIn("does not occur", str(c.exception))
        self.assertEqual(self.env.rows(), [])

    def test_finding_matches_with_whitespace_and_entry_newline_escapes(self):
        self.assertEqual(self.env.go(finding="the parser  accepts a negative length\nsecond finding".replace("the", "src/lib.rs:10 - the", 1))[0], "false")

    def test_model_must_be_neutral(self):
        cases = [
            (Env(tempfile.mkdtemp(), reviewer="gpt-6.1-sol"), "gpt-6.2-astra", "family of the gating reviewer"),
            (self.env, "claude-sonnet-5", "family of the coder"),
            (self.env, "opus", "family of the orchestrator"),
            (self.env, "gpt-6.2-astra", "--exclude"),
            (self.env, "haiku", "not an OpenAI model"),
            (self.env, "mystery-model", "unknown model family"),
        ]
        for env, model, text in cases:
            ex = ("gpt-6.1-sol",) if text == "--exclude" else ()
            with self.assertRaises(rd.Refused) as c:
                env.go(model=model, excludes=ex)
            self.assertIn(text, str(c.exception), model)
            self.assertEqual((env.runs, env.prompts), ([], []))

    def test_unknown_family_of_coder_or_reviewer_is_refused(self):
        with self.assertRaises(rd.Refused):
            rd.neutral_check("gpt-6.2-astra", {"coder": "mystery", "reviewer_model": "opus"}, ())
        with self.assertRaises(rd.Refused):
            rd.neutral_check("gpt-6.2-astra", {"coder": "sonnet"}, ())

    def test_bad_answers_are_errors_recorded_not_rulings(self):
        for reply in (
            {"is_error": True, "result": "boom"},
            {"is_error": False, "result": "no ruling line here", "modelUsage": {"gpt-6.2-astra": {}}},
            {"is_error": False, "result": "RULING: real\nRULING: false", "modelUsage": {"gpt-6.2-astra": {}}},
            {"is_error": False, "result": "RULING: real\nmore words after", "modelUsage": {"gpt-6.2-astra": {}}},
            good_reply(model="gpt-5.0-other"),  # another model answered
            {"is_error": False, "result": "RULING: false"},  # no modelUsage
        ):
            before = len(self.env.rows())
            with self.assertRaises(rd.Refused):
                self.env.go(caller=self.env.caller(reply))
            self.assertEqual(len(self.env.rows()), before + (1 if before else 2))  # header once, then one error row
            self.assertEqual(self.env.rows()[-1].split("\t")[7], "error")

    def test_secrets_are_redacted_before_the_prompt(self):
        (self.env.root / ".env").write_text("OPENAI_API_KEY=sk-dispute-SECRET-999\n")
        out = PASS_OUT + "token sk-dispute-SECRET-999 and ENVSECRETVALUE1\n"
        self.env.go(runner=self.env.runner(out), environ={"DEMO_TOKEN": "ENVSECRETVALUE1"})
        prompt = self.env.prompts[0]
        self.assertNotIn("sk-dispute-SECRET-999", prompt)
        self.assertNotIn("ENVSECRETVALUE1", prompt)
        self.assertIn("[REDACTED]", prompt)
        self.assertNotIn("sk-dispute-SECRET-999", "\n".join(self.env.rows()))

    def test_test_output_is_cut_to_its_tail(self):
        out = "EARLYMARK\n" + "x" * 5000 + "\n" + PASS_OUT
        self.env.go(runner=self.env.runner(out))
        self.assertNotIn("EARLYMARK", self.env.prompts[0])
        self.assertIn("1 passed", self.env.prompts[0])

    def test_oversized_diff_is_refused_never_cut(self):
        env = Env(tempfile.mkdtemp(), extra_diff="y" * 60000 + "\n")
        with self.assertRaises(rd.Refused) as c:
            env.go()
        self.assertIn("never cut", str(c.exception))

    def test_bad_entry_ids_test_names_and_missing_toml_key(self):
        for bad in ("../x", "q-1", "q-20261121-050000/../../x"):
            with self.assertRaises(rd.Refused):
                rd.dispute(self.env.root, bad, FINDING, "t", "gpt-6.2-astra", (), {}, self.env.runner(), self.env.caller(good_reply()))
        for bad in ("--help", "a b", "py:../x.py::t", "py:x.py", "py:x.txt::t", ""):
            with self.assertRaises(rd.Refused, msg=bad):
                rd.test_command(bad)
        argv, name = rd.test_command("py:scripts/test_x.py::TestA.test_b")
        self.assertEqual((argv[-3:], name), (["-k", "TestA.test_b", "-v"], "TestA.test_b"))
        self.assertIn("scripts", argv)
        toml = (self.env.root / "broadsword.toml").read_text().replace("reviewer_diff_max_kb = 50\n", "")
        (self.env.root / "broadsword.toml").write_text(toml)
        with self.assertRaises(rd.Refused) as c:
            self.env.go()
        self.assertIn("reviewer_diff_max_kb", str(c.exception))

    def test_corrupt_header_refuses_and_appends_nothing(self):
        self.env.go()
        p = self.env.root / "state" / "review-disputes.tsv"
        p.write_text("wrong\n")
        with self.assertRaises(rd.Refused):
            self.env.go()
        self.assertEqual(p.read_text(), "wrong\n")

    def test_dispute_never_touches_the_entry_or_the_brief(self):
        p = self.env.root / "state" / "queue" / f"{self.env.entry}.entry"
        before = p.read_bytes()
        self.env.go()
        self.assertEqual(p.read_bytes(), before)

    def test_client_call_uses_the_clients_own_cli_and_returns_its_json(self):
        fake = self.env.root / "fake_client.py"
        fake.write_text(
            "import sys, json, os\n"
            "open(os.environ['FAKE_OUT'], 'w', encoding='utf-8').write(json.dumps({'argv': sys.argv[1:], 'stdin': sys.stdin.read(), 'caller': os.environ.get('OPENAI_REVIEW_CALLER'), 'timeout': os.environ.get('OPENAI_REVIEW_TIMEOUT')}))\n"
            "print(json.dumps({'is_error': False, 'result': 'ok\\nRULING: false', 'modelUsage': {'gpt-6.2-astra': {}}}))\n",
            encoding="utf-8",
        )
        out = self.env.root / "seen.json"
        cfg = rd.load_config(self.env.root)
        data = rd.call_client(self.env.root, cfg, "gpt-6.2-astra", "PROMPT-TEXT", {**os.environ, "FAKE_OUT": str(out)}, script=fake)
        self.assertEqual(rd.parse_ruling(data, "gpt-6.2-astra")[0], "false")
        seen = json.loads(out.read_text(encoding="utf-8"))
        self.assertEqual(seen["stdin"], "PROMPT-TEXT")
        self.assertEqual((seen["caller"], seen["timeout"]), ("review_dispute", "90"))
        # the argv must be accepted by the real client's own parser, with the system prompt and model intact
        cspec = importlib.util.spec_from_file_location("orc", HERE.parent / "clients" / "openai_review.py")
        orc = importlib.util.module_from_spec(cspec)
        sys.modules["openai_mcp"] = importlib.util.module_from_spec(importlib.util.spec_from_file_location("openai_mcp", HERE.parent / "clients" / "openai_mcp.py"))
        cspec.loader.exec_module(orc)
        ns = orc.build_parser().parse_args(seen["argv"])
        self.assertEqual((ns.model, ns.system_prompt, ns.tools, ns.max_turns), ("gpt-6.2-astra", rd.SYSTEM_PROMPT, "", "1"))

    def test_client_without_json_or_missing_is_refused(self):
        cfg = rd.load_config(self.env.root)
        bad = self.env.root / "bad_client.py"
        bad.write_text("print('not json')\n")
        with self.assertRaises(rd.Refused):
            rd.call_client(self.env.root, cfg, "gpt-6.2-astra", "p", dict(os.environ), script=bad)
        with self.assertRaises(rd.Refused):
            rd.call_client(self.env.root, cfg, "gpt-6.2-astra", "p", dict(os.environ), script=self.env.root / "nope.py")

    def test_main_exit_codes(self):
        argv = ["--root", str(self.env.root), "--model", "gpt-6.2-astra", self.env.entry, "never raised", "t"]
        self.assertEqual(rd.main(argv), 2)


if __name__ == "__main__":
    unittest.main()
