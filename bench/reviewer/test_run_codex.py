"""Tests for run_codex.py. No real codex, no network: a fake `codex` (a small Python script behind codex.cmd / a shell wrapper) is put
first on a PATH given to run_codex.main(), and records what it was asked (argv, stdin, the model catalog file, its environment).

Run (from bench/reviewer/):  env -u OPENAI_API_KEY python -I -B -m unittest discover -s . -p test_run_codex.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, shutil, stat, tempfile, unittest

REAL_HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("run_codex_under_test", REAL_HERE / "run_codex.py")
rc = importlib.util.module_from_spec(_spec)
sys.modules["run_codex_under_test"] = rc
_spec.loader.exec_module(rc)

FAKE_KEY = "sk-test-FAKEKEY123456"
REVIEW = "VERDICT: FAIL\nBLOCKING: src/a.rs:1 - the run prints NaN and exits 0 silently\nMINOR: none"

FAKE_CODEX = r'''
import json, os, sys
argv = sys.argv[1:]
if argv == ["--version"]:
    print("codex-cli 9.9.9-test")
    sys.exit(0)
stdin = sys.stdin.read()
mode = os.environ.get("FAKE_CODEX_MODE", "ok")
catalog = None
for i, a in enumerate(argv):
    if a == "-c" and i + 1 < len(argv) and argv[i + 1].startswith("model_catalog_json="):
        path = argv[i + 1].split("=", 1)[1].strip("'\"")
        catalog = open(path, encoding="utf-8").read()
rec = {"argv": argv, "stdin": stdin, "catalog": catalog, "env_names": sorted(os.environ), "env_values": sorted(os.environ.values())}
open(os.environ["FAKE_CODEX_LOG"], "w", encoding="utf-8").write(json.dumps(rec))
if mode == "fail":
    sys.stderr.write("boom")
    sys.exit(1)
out = argv[argv.index("-o") + 1]
events = [{"type": "thread.started", "thread_id": "t", **({"model": "gpt-answered-by"} if mode == "modelnamed" else {})}, {"type": "turn.started"}]
if mode == "tool":
    events.append({"type": "item.completed", "item": {"type": "command_execution", "command": "cat .env"}})
events.append({"type": "item.completed", "item": {"type": "agent_message", "text": "x"}})
if mode != "nousage":
    events.append({"type": "turn.completed", "usage": {"input_tokens": 1234, "output_tokens": 56}})
if mode != "noanswer":
    open(out, "w", encoding="utf-8").write(os.environ.get("FAKE_CODEX_TEXT", ""))
for e in events:
    print(json.dumps(e))
'''


class RunCodex(unittest.TestCase):
    def setUp(self):
        scratch = REAL_HERE.parent.parent / "scratch"  # git-ignored scratch folder of this checkout
        scratch.mkdir(exist_ok=True)
        self._tmp = tempfile.mkdtemp(prefix="bench-codex-test-", dir=str(scratch))
        self.tmp = pathlib.Path(self._tmp)
        self.here = self.tmp / "here"
        (self.here / "cases").mkdir(parents=True)
        (self.here / "cases" / "c1.diff").write_text("+ let x = f64::NAN;\n", encoding="utf-8")
        self.case = {"id": "c1", "kind": "k", "diff": "cases/c1.diff", "brief": "Task: do a thing. Done check: it works.",
                     "bug": "b", "catch": "(?i)nan.{0,100}(exit|silent)", "known_false_positives": []}
        (self.here / "cases.json").write_text(json.dumps([self.case]), encoding="utf-8")
        (self.tmp / "root").mkdir()  # find_main_root answer: no .env in it
        # the spend cap and canary gates sit under review_codex(): give them a temp root with the settings they read, a passed
        # canary for the fake codex's version, and the ledger folder inside it (never the real state/)
        (self.tmp / "root" / "broadsword.toml").write_text('state_dir = "state"' + chr(10), encoding="utf-8")
        (self.tmp / "clients").mkdir()
        (self.tmp / "state" / "canary").mkdir(parents=True)
        (self.tmp / "state" / "canary" / "last-pass.json").write_text(json.dumps({"codex_version": "codex-cli 9.9.9-test"}), encoding="utf-8")
        self.bin = self.tmp / "bin"
        self.bin.mkdir()
        (self.bin / "fake_codex.py").write_text(FAKE_CODEX, encoding="utf-8")
        if os.name == "nt":
            (self.bin / "codex.cmd").write_text(f'@"{sys.executable}" "%~dp0fake_codex.py" %*\r\n', encoding="utf-8")
        else:
            w = self.bin / "codex"
            w.write_text(f'#!/bin/sh\nexec "{sys.executable}" "$(dirname "$0")/fake_codex.py" "$@"\n', encoding="utf-8")
            w.chmod(w.stat().st_mode | stat.S_IEXEC)
        self.log = self.tmp / "log.json"
        self.out = self.tmp / "out"
        self.old_here, rc.HERE = rc.HERE, self.here
        self.old_rhere, rc.review.HERE = rc.review.HERE, self.tmp / "clients"
        self.old_root, rc.oc.find_main_root = rc.oc.find_main_root, lambda start: self.tmp / "root"
        self.env = {k: v for k, v in os.environ.items() if k.upper() != "OPENAI_API_KEY"}
        self.env["PATH"] = str(self.bin) + os.pathsep + self.env.get("PATH", "")
        self.env["FAKE_CODEX_LOG"] = str(self.log)
        self.env["FAKE_CODEX_TEXT"] = REVIEW

    def tearDown(self):
        rc.HERE = self.old_here
        rc.review.HERE = self.old_rhere
        rc.oc.find_main_root = self.old_root
        shutil.rmtree(self._tmp, ignore_errors=True)

    def run_it(self, *extra, model="gpt-fake", effort="default", env=None):
        so, se = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(so), contextlib.redirect_stderr(se):
            code = rc.main([str(self.out), model, effort, *extra], environ=env or self.env)
        return code, so.getvalue(), se.getvalue()

    def record(self, model="gpt-fake"):
        return json.loads((self.out / f"c1__{model}__default.json").read_text(encoding="utf-8"))

    def test_good_answer_writes_the_same_result_shape(self):
        code, so, _ = self.run_it()
        self.assertEqual(code, 0)
        r = self.record()
        for k in ("case", "model", "effort", "wall_s", "exit", "cost_usd", "usage", "result", "is_error", "model_used"):
            self.assertIn(k, r)
        self.assertEqual((r["exit"], r["is_error"], r["result"], r["backend"], r["cost_usd"]), (0, False, REVIEW, "codex", None))
        self.assertEqual((r["usage"]["input_tokens"], r["usage"]["output_tokens"]), (1234, 56))
        self.assertEqual(r["codex_items"], ["agent_message"])
        self.assertTrue((self.out / "c1__gpt-fake__default.raw.json").is_file())

    def test_record_says_whether_codex_named_the_model(self):
        self.run_it()
        r = self.record()
        self.assertIs(r["model_reported"], False)
        self.assertEqual(r["model_used"], [])
        self.env["FAKE_CODEX_MODE"] = "modelnamed"
        self.run_it()
        r = self.record()
        self.assertIs(r["model_reported"], True)
        self.assertEqual(r["model_used"], ["gpt-answered-by"])
        raw = json.loads((self.out / "c1__gpt-fake__default.raw.json").read_text(encoding="utf-8"))
        self.assertEqual(list(raw["modelUsage"]), ["gpt-answered-by"])

    def test_failed_run_records_model_reported_false(self):
        self.env["FAKE_CODEX_MODE"] = "fail"
        self.run_it()
        r = self.record()
        self.assertEqual((r["is_error"], r["model_reported"], r["model_used"]), (True, False, []))

    def test_codex_is_offered_no_tools_and_gets_the_prompt_on_stdin(self):
        self.run_it()
        rec = json.loads(self.log.read_text(encoding="utf-8"))
        a = rec["argv"]
        self.assertEqual(a[0], "exec")
        self.assertEqual(a[a.index("-m") + 1], "gpt-fake")
        self.assertEqual(a[a.index("--sandbox") + 1], "read-only")
        for flag in ("--ephemeral", "--ignore-user-config", "--strict-config", "--json"):
            self.assertIn(flag, a)
        self.assertEqual(a[-1], "-")
        cat = json.loads(rec["catalog"])
        self.assertEqual(len(cat["models"]), 1)
        self.assertEqual(cat["models"][0]["slug"], "gpt-fake")
        self.assertEqual(cat["models"][0]["tool_mode"], "direct")
        self.assertEqual(cat["models"][0]["shell_type"], "disabled")
        self.assertIn("=== SYSTEM INSTRUCTIONS ===", rec["stdin"])
        self.assertIn(rc.run_mod.SYSTEM, rec["stdin"])
        self.assertIn("+ let x = f64::NAN;", rec["stdin"])
        self.assertIn("Review it now, in the required format.", rec["stdin"])

    def test_command_line_is_exactly_the_shared_one(self):
        self.run_it()
        rec = json.loads(self.log.read_text(encoding="utf-8"))
        want = rc.review.codex_command("codex", "gpt-fake", self.tmp, self.tmp / "answer.txt")
        # same flags apart from the executable and the two temp paths
        norm = lambda c: [x for x in c if "no-tools-catalog" not in x and "answer.txt" not in x and "bsw-codex-" not in x]
        self.assertEqual(norm(rec["argv"]), norm(want[1:]))

    def test_api_key_never_reaches_codex(self):
        env = dict(self.env)
        env["OPENAI_API_KEY"] = FAKE_KEY
        env["COPY_OF_THE_KEY"] = FAKE_KEY
        self.run_it(env=env)
        rec = json.loads(self.log.read_text(encoding="utf-8"))
        self.assertNotIn("OPENAI_API_KEY", [n.upper() for n in rec["env_names"]])
        self.assertNotIn(FAKE_KEY, rec["env_values"])

    def test_tool_item_discards_the_answer(self):
        env = dict(self.env, FAKE_CODEX_MODE="tool")
        code, so, _ = self.run_it(env=env)
        self.assertEqual(code, 1)
        r = self.record()
        self.assertTrue(r["is_error"])
        self.assertEqual(r["result"], "")
        self.assertIn("command_execution", r["error"])

    def test_codex_failure_is_recorded_with_no_api_fallback(self):
        env = dict(self.env, FAKE_CODEX_MODE="fail")
        code, _, _ = self.run_it(env=env)
        self.assertEqual(code, 1)
        r = self.record()
        self.assertTrue(r["is_error"])
        self.assertEqual(r["exit"], 1)
        self.assertIn("code 1", r["error"])
        self.assertEqual(r["backend"], "codex")

    def test_missing_usage_and_empty_answer_are_errors(self):
        code, _, _ = self.run_it(env=dict(self.env, FAKE_CODEX_MODE="nousage"))
        self.assertEqual(code, 1)
        self.assertIn("turn.completed", self.record()["error"])
        code, _, _ = self.run_it(env=dict(self.env, FAKE_CODEX_TEXT="  "))
        self.assertEqual(code, 1)
        self.assertIn("empty", self.record()["error"])

    def test_key_in_answer_is_redacted_in_every_file(self):
        env = dict(self.env, FAKE_CODEX_TEXT=f"VERDICT: PASS\nMINOR: leaked {FAKE_KEY}")
        self.run_it(env=env)
        for p in self.out.iterdir():
            self.assertNotIn(FAKE_KEY, p.read_text(encoding="utf-8"))

    def test_inputs_checked_before_the_first_call(self):
        def refused(*args, **kw):
            self.log.unlink(missing_ok=True)
            with contextlib.redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit) as cm:
                    self.run_it(*args, **kw)
            self.assertFalse(self.log.exists())  # codex never started
            return str(cm.exception)

        self.assertIn("effort 'default'", refused(effort="high"))
        self.assertIn("unknown case", refused("nope"))
        self.assertIn("does not use --max-tokens", refused("--max-tokens", "100"))
        self.assertIn("not found", refused(env={"PATH": str(self.tmp / "empty")}))
        (self.here / "cases" / "c1.diff").unlink()
        self.assertIn("cannot read its diff", refused())
        self.assertFalse(self.out.exists())

    def test_output_folder_is_made_absolute(self):
        os.chdir(self.tmp)
        try:
            so, se = io.StringIO(), io.StringIO()
            with contextlib.redirect_stdout(so), contextlib.redirect_stderr(se):
                rc.main(["rel-out", "gpt-fake", "default"], environ=self.env)
        finally:
            os.chdir(REAL_HERE)
        self.assertTrue((self.tmp / "rel-out" / "c1__gpt-fake__default.json").is_file())


if __name__ == "__main__":
    unittest.main()
