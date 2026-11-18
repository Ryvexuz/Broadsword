"""Tests for run_openai.py. No real network: a fake OpenAI server runs on 127.0.0.1 in a thread.

Run (from bench/reviewer/):  env -u OPENAI_API_KEY python -I -B -m unittest discover -s . -p test_run_openai.py -v
(plain `-m unittest test_run_openai` cannot find the module under -I)

The key below is fake. Every run passes --root to a throwaway folder, so the real root .env is never read.
Fake cases (cases.json plus a diff) live in a temp folder; score.py is copied next to them and run there.
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, shutil, subprocess, tempfile, threading, unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

REAL_HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("run_openai_under_test", REAL_HERE / "run_openai.py")
ro = importlib.util.module_from_spec(_spec)
sys.modules["run_openai_under_test"] = ro
_spec.loader.exec_module(ro)

FAKE_KEY = "sk-test-FAKEKEY123456"
REVIEW = "VERDICT: FAIL\nBLOCKING: src/a.rs:1 - the run prints NaN and exits 0 silently\nMINOR: none"


class Fake:
    """Fake server: `script` is a list of (status, body-dict) answered in order; the last one repeats."""

    def __init__(self, script):
        self.script, self.requests, self.auth = list(script), [], []
        outer = self

        class H(BaseHTTPRequestHandler):
            def do_POST(self):
                n = int(self.headers.get("Content-Length", 0))
                outer.requests.append(json.loads(self.rfile.read(n)))
                outer.auth.append(self.headers.get("Authorization"))
                status, body = outer.script.pop(0) if len(outer.script) > 1 else outer.script[0]
                data = json.dumps(body).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *a):
                pass

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), H)
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}/v1"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def ok_body(text=REVIEW, model="gpt-fake-2026-01-01"):
    return {"model": model, "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 1000, "completion_tokens": 200, "total_tokens": 1200}}


class RunOpenAI(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.mkdtemp(prefix="bench-openai-test-")
        self.tmp = pathlib.Path(self._tmp)
        self.here = self.tmp / "here"
        (self.here / "cases").mkdir(parents=True)
        (self.here / "cases" / "c1.diff").write_text("+ let x = f64::NAN;\n", encoding="utf-8")
        self.case = {"id": "c1", "kind": "k", "diff": "cases/c1.diff", "brief": "Task: do a thing. Done check: it works.",
                     "bug": "b", "catch": "(?i)nan.{0,100}(exit|silent)", "known_false_positives": []}
        (self.here / "cases.json").write_text(json.dumps([self.case]), encoding="utf-8")
        shutil.copy(REAL_HERE / "score.py", self.here / "score.py")
        (self.tmp / "root").mkdir()
        self.out = self.tmp / "out"
        self.old_here, ro.HERE = ro.HERE, self.here
        self.old_key = os.environ.pop("OPENAI_API_KEY", None)
        os.environ["OPENAI_API_KEY"] = FAKE_KEY
        ro.oc._SECRETS.clear()

    def tearDown(self):
        ro.HERE = self.old_here
        os.environ.pop("OPENAI_API_KEY", None)
        if self.old_key is not None:
            os.environ["OPENAI_API_KEY"] = self.old_key
        shutil.rmtree(self._tmp, ignore_errors=True)

    def run_it(self, fake, model="gpt-fake", effort="high", *extra):
        argv = [str(self.out), model, effort, "--base-url", fake.url, "--root", str(self.tmp / "root"),
                "--backoff", "0", *extra]
        so, se = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(so), contextlib.redirect_stderr(se):
            code = ro.main(argv)
        return code, so.getvalue(), se.getvalue()

    def record(self, model="gpt-fake", effort="high"):
        return json.loads((self.out / f"c1__{model}__{effort}.json").read_text(encoding="utf-8"))

    def test_request_carries_prompts_model_and_effort(self):
        f = Fake([(200, ok_body())])
        self.addCleanup(f.close)
        code, _, _ = self.run_it(f)
        self.assertEqual(code, 0)
        req = f.requests[0]
        self.assertEqual(req["model"], "gpt-fake")
        self.assertEqual(req["reasoning_effort"], "high")
        self.assertEqual(req["messages"][0], {"role": "system", "content": ro.run_mod.SYSTEM})
        diff = "+ let x = f64::NAN;\n"
        want = f"{self.case['brief']}\n\nDIFF:\n```diff\n{diff}\n```\n\nReview it now, in the required format."
        self.assertEqual(req["messages"][1], {"role": "user", "content": want})
        self.assertEqual(f.auth[0], "Bearer " + FAKE_KEY)

    def test_default_effort_omits_reasoning_effort(self):
        f = Fake([(200, ok_body())])
        self.addCleanup(f.close)
        self.run_it(f, "gpt-fake", "default")
        self.assertNotIn("reasoning_effort", f.requests[0])

    def test_output_fields_and_score_py_runs(self):
        f = Fake([(200, ok_body())])
        self.addCleanup(f.close)
        self.run_it(f)
        r = self.record()
        for k in ("case", "model", "effort", "wall_s", "exit", "cost_usd", "usage", "result", "is_error", "model_used"):
            self.assertIn(k, r)
        self.assertEqual((r["exit"], r["is_error"], r["result"]), (0, False, REVIEW))
        self.assertEqual(r["model_used"], ["gpt-fake-2026-01-01"])
        self.assertTrue((self.out / "c1__gpt-fake__high.raw.json").is_file())
        p = subprocess.run([sys.executable, "-I", "-B", str(self.here / "score.py"), str(self.out)],
                           capture_output=True, text=True, timeout=60)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("c1", p.stdout)
        self.assertIn("YES", p.stdout)  # the review flags the bug as BLOCKING
        self.assertNotIn("ERROR", p.stdout)

    def _body(self, content, finish, usage=None):
        return {"model": "gpt-fake", "choices": [{"message": {"role": "assistant", "content": content}, "finish_reason": finish}],
                "usage": usage or {"prompt_tokens": 1000, "completion_tokens": 16000}}

    def test_length_with_empty_content_is_an_error(self):
        f = Fake([(200, self._body("", "length"))])
        self.addCleanup(f.close)
        code, so, _ = self.run_it(f)
        self.assertEqual(code, 1)
        r = self.record()
        self.assertTrue(r["is_error"])
        self.assertEqual(r["exit"], 1)
        self.assertIn("finish_reason='length'", r["error"])
        self.assertIn("16000", r["error"])  # token usage named
        self.assertIn("ERROR", so)

    def test_empty_content_with_stop_is_an_error(self):
        f = Fake([(200, self._body("", "stop"))])
        self.addCleanup(f.close)
        code, _, _ = self.run_it(f)
        self.assertEqual(code, 1)
        r = self.record()
        self.assertTrue(r["is_error"])
        self.assertEqual(r["exit"], 1)
        self.assertIn("empty answer", r["error"])
        self.assertIn("finish_reason='stop'", r["error"])

    def test_truncated_text_with_length_is_an_error(self):
        f = Fake([(200, self._body("VERDICT: FAIL\nBLOCKING: cut off", "length"))])
        self.addCleanup(f.close)
        code, _, _ = self.run_it(f)
        self.assertEqual(code, 1)
        self.assertTrue(self.record()["is_error"])

    def test_normal_stop_answer_still_passes(self):
        f = Fake([(200, self._body(REVIEW, "stop"))])
        self.addCleanup(f.close)
        code, _, _ = self.run_it(f)
        self.assertEqual(code, 0)
        r = self.record()
        self.assertEqual((r["exit"], r["is_error"], r["result"]), (0, False, REVIEW))
        self.assertNotIn("error", r)

    def test_response_without_choices_is_a_per_case_error(self):
        f = Fake([(200, {"model": "gpt-fake", "choices": [{"finish_reason": "stop"}]})])
        self.addCleanup(f.close)
        code, _, _ = self.run_it(f)
        self.assertEqual(code, 1)
        r = self.record()
        self.assertTrue(r["is_error"])
        self.assertIn("no message", r["error"])  # rejected by openai_mcp.chat_completion, recorded per case

    def test_429_then_200_is_retried(self):
        f = Fake([(429, {"error": {"message": "slow down"}}), (200, ok_body())])
        self.addCleanup(f.close)
        code, _, _ = self.run_it(f)
        self.assertEqual(code, 0)
        self.assertEqual(len(f.requests), 2)
        self.assertFalse(self.record()["is_error"])

    def test_401_is_a_clear_error_without_the_key(self):
        f = Fake([(401, {"error": {"message": f"Incorrect API key provided: {FAKE_KEY}"}})])
        self.addCleanup(f.close)
        code, so, se = self.run_it(f)
        self.assertEqual(code, 1)
        r = self.record()
        self.assertTrue(r["is_error"])
        self.assertEqual(r["exit"], 1)
        self.assertIn("401", r["error"])
        self.assertIn("rejected the key", r["error"])
        self.assertEqual(len(f.requests), 1)  # not retried
        for text in (r["error"], so, se):
            self.assertNotIn(FAKE_KEY, text)

    def test_cost_only_with_prices(self):
        f = Fake([(200, ok_body())])
        self.addCleanup(f.close)
        code, _, se = self.run_it(f)
        self.assertIsNone(self.record()["cost_usd"])
        self.assertIn("cost_usd is null", se)
        self.run_it(f, "gpt-fake", "high", "--price-in", "2", "--price-out", "10")
        # 1000 * 2/1e6 + 200 * 10/1e6
        self.assertAlmostEqual(self.record()["cost_usd"], 0.004, places=9)

    def test_key_never_in_any_output_file(self):
        f = Fake([(200, ok_body(text=f"VERDICT: PASS\nBLOCKING: none\nMINOR: leaked {FAKE_KEY}"))])
        self.addCleanup(f.close)
        self.run_it(f)
        files = list(self.out.iterdir())
        self.assertEqual(len(files), 2)
        for p in files:
            self.assertNotIn(FAKE_KEY, p.read_text(encoding="utf-8"))

    def test_unknown_case_and_missing_key_refused(self):
        f = Fake([(200, ok_body())])
        self.addCleanup(f.close)
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                ro.main([str(self.out), "m", "high", "nope", "--base-url", f.url, "--root", str(self.tmp / "root")])
            del os.environ["OPENAI_API_KEY"]
            with self.assertRaises(SystemExit):
                ro.main([str(self.out), "m", "high", "--base-url", f.url, "--root", str(self.tmp / "root")])
        self.assertEqual(f.requests, [])

    def test_help_makes_no_network_call(self):
        p = subprocess.run([sys.executable, "-I", "-B", str(REAL_HERE / "run_openai.py"), "--help"],
                           capture_output=True, text=True, timeout=60, env={k: v for k, v in os.environ.items() if k != "OPENAI_API_KEY"})
        self.assertEqual(p.returncode, 0)
        self.assertIn("usage:", p.stdout)
        self.assertIn("--price-in", p.stdout)


if __name__ == "__main__":
    unittest.main()
