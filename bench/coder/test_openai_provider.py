"""Tests for openai_provider.py and run.py's openai branch. No real network: a fake server on 127.0.0.1 in a thread.

Run (from the repo root):
  env -u OPENAI_API_KEY python -I -B -m unittest discover -s bench/coder -p "test_openai_provider.py" -v

The key is fake and every call passes --root to a throwaway folder, so the real root .env is never read.
The end-to-end tests run run.py with answers that have no code block, so nothing is graded (no cargo is started).
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, subprocess, tempfile, threading, unittest, shutil
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("openai_provider_under_test", HERE / "openai_provider.py")
op = importlib.util.module_from_spec(_spec)
sys.modules["openai_provider_under_test"] = op
_spec.loader.exec_module(op)

FAKE_KEY = "sk-test-FAKEKEY123456"
CODE = "```rust\npub fn f() {}\n```"
SYSTEM = "SYS PROMPT"


class Fake:
    """`script` is a list of (status, body-dict) answered in order; the last one repeats."""

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


def body(text=CODE, finish="stop", model="gpt-fake-2026-01-01", usage=None):
    return {"model": model, "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": finish}],
            "usage": usage or {"prompt_tokens": 1000, "completion_tokens": 200,
                               "prompt_tokens_details": {"cached_tokens": 64}}}


class Provider(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="bench-coder-openai-test-"))
        (self.tmp / "root").mkdir()
        self.old_key = os.environ.pop("OPENAI_API_KEY", None)
        os.environ["OPENAI_API_KEY"] = FAKE_KEY
        op._ro().oc._SECRETS.clear()
        op._state.pop("cfg", None)

    def tearDown(self):
        os.environ.pop("OPENAI_API_KEY", None)
        if self.old_key is not None:
            os.environ["OPENAI_API_KEY"] = self.old_key
        shutil.rmtree(self.tmp, ignore_errors=True)

    def go(self, fake, model="gpt-fake", effort="high", *extra, prompt="USER PROMPT"):
        with contextlib.redirect_stderr(io.StringIO()):
            op.prepare(model, effort, ["--base-url", fake.url, "--root", str(self.tmp / "root"), "--backoff", "0", *extra], 600)
        return op.call(SYSTEM, prompt, model, effort)

    def test_request_carries_prompts_model_effort_and_key(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        r = self.go(f)
        req = f.requests[0]
        self.assertEqual(req["model"], "gpt-fake")
        self.assertEqual(req["reasoning_effort"], "high")
        self.assertEqual(req["messages"], [{"role": "system", "content": SYSTEM}, {"role": "user", "content": "USER PROMPT"}])
        self.assertEqual(f.auth[0], "Bearer " + FAKE_KEY)
        self.assertEqual(len(f.requests), 1)
        self.assertEqual((r["answer"], r["exit"], r["is_error"], r["error"]), (CODE, 0, False, None))
        self.assertEqual(r["model_used"], ["gpt-fake-2026-01-01"])
        self.assertEqual(r["tokens"], {"input": 1000, "output": 200, "cache_read": 64, "cache_write": None})
        self.assertEqual(json.loads(r["raw"])["choices"][0]["finish_reason"], "stop")

    def test_default_effort_omits_reasoning_effort(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        self.go(f, "gpt-fake", "default")
        self.assertNotIn("reasoning_effort", f.requests[0])

    def test_cost_only_with_both_prices(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        self.assertIsNone(self.go(f)["cost_usd"])
        r = self.go(f, "gpt-fake", "high", "--price-in", "2", "--price-out", "10")
        self.assertAlmostEqual(r["cost_usd"], 0.004, places=9)  # 1000*2/1e6 + 200*10/1e6
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            op.prepare("m", "high", ["--price-in", "2", "--base-url", f.url, "--root", str(self.tmp / "root")], 600)

    def test_empty_and_non_stop_answers_are_errors(self):
        for text, finish, needle in (("", "length", "finish_reason='length'"), ("", "stop", "empty answer"),
                                     (CODE, "length", "incomplete answer")):
            f = Fake([(200, body(text, finish))])
            self.addCleanup(f.close)
            r = self.go(f)
            self.assertTrue(r["is_error"])
            self.assertEqual((r["exit"], r["answer"]), (1, ""))
            self.assertIn(needle, r["error"])

    def test_refusal_and_missing_choices_are_errors(self):
        f = Fake([(200, {"model": "m", "choices": [{"message": {"content": None, "refusal": "no can do"}, "finish_reason": "stop"}]})])
        self.addCleanup(f.close)
        self.assertIn("refused", self.go(f)["error"])
        g = Fake([(200, {"model": "m", "choices": [{"finish_reason": "stop"}]})])
        self.addCleanup(g.close)
        r = self.go(g)
        self.assertTrue(r["is_error"])
        self.assertIn("no message", r["error"])

    def test_429_is_retried_then_ok(self):
        f = Fake([(429, {"error": {"message": "slow down"}}), (200, body())])
        self.addCleanup(f.close)
        r = self.go(f)
        self.assertEqual(len(f.requests), 2)
        self.assertFalse(r["is_error"])

    def test_retries_are_bounded(self):
        f = Fake([(500, {"error": {"message": "boom"}})])
        self.addCleanup(f.close)
        r = self.go(f, "gpt-fake", "high", "--retries", "1")
        self.assertTrue(r["is_error"])
        self.assertEqual(len(f.requests), 2)

    def test_401_is_an_error_without_the_key_and_not_retried(self):
        f = Fake([(401, {"error": {"message": f"Incorrect API key provided: {FAKE_KEY}"}})])
        self.addCleanup(f.close)
        r = self.go(f)
        self.assertTrue(r["is_error"])
        self.assertEqual(len(f.requests), 1)
        self.assertIn("401", r["error"])
        self.assertNotIn(FAKE_KEY, r["error"])
        self.assertNotIn(FAKE_KEY, r["raw"].decode())

    def test_key_in_the_answer_is_redacted_in_raw(self):
        f = Fake([(200, body(f"{CODE}\nleak {FAKE_KEY}"))])
        self.addCleanup(f.close)
        self.assertNotIn(FAKE_KEY, self.go(f)["raw"].decode())

    def test_missing_key_refused_before_any_request(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        del os.environ["OPENAI_API_KEY"]
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            op.prepare("m", "high", ["--base-url", f.url, "--root", str(self.tmp / "root")], 600)
        self.assertEqual(f.requests, [])

    def test_plain_http_to_a_remote_host_refused(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            op.prepare("m", "high", ["--base-url", "http://example.com/v1", "--root", str(self.tmp / "root")], 600)

    def test_split_options(self):
        rest, opts = op.split_options(["out", "openai", "m", "high", "01-duration", "--price-in", "2", "--root=x", "--price-out", "10"])
        self.assertEqual(rest, ["out", "openai", "m", "high", "01-duration"])
        self.assertEqual(opts, ["--price-in", "2", "--root=x", "--price-out", "10"])


class RunPy(unittest.TestCase):
    """run.py end to end with answers that have no code block (so no cargo runs)."""

    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="bench-coder-openai-run-"))
        (self.tmp / "root").mkdir()
        self.env = {k: v for k, v in os.environ.items() if k != "OPENAI_API_KEY"}
        self.env.update(OPENAI_API_KEY=FAKE_KEY, BENCH_CWD=str(self.tmp / "cwd"))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_py(self, fake, *extra):
        out = self.tmp / "out"
        p = subprocess.run([sys.executable, "-I", "-B", str(HERE / "run.py"), str(out), "openai", "gpt-fake", "high",
                            "01-duration", "--base-url", fake.url, "--root", str(self.tmp / "root"), "--backoff", "0", *extra],
                           capture_output=True, text=True, timeout=120, env=self.env)
        return p, out

    def test_run_py_writes_the_result_fields_without_calling_the_real_api(self):
        f = Fake([(200, body("no code block here"))])
        self.addCleanup(f.close)
        p, out = self.run_py(f, "--price-in", "2", "--price-out", "10")
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads((out / "01-duration__openai__gpt-fake__high.json").read_text(encoding="utf-8"))
        for k in ("provider", "model", "effort", "task", "tokens", "cost_usd", "wall_s", "exit", "harness_wall_s", "compiles",
                  "passed", "total", "format_failure", "first_error"):
            self.assertIn(k, rec)
        self.assertEqual((rec["provider"], rec["model"], rec["task"], rec["exit"]), ("openai", "gpt-fake", "01-duration", 0))
        self.assertAlmostEqual(rec["cost_usd"], 0.004, places=9)
        self.assertTrue(rec["format_failure"])
        self.assertFalse(rec["compiles"])
        self.assertEqual(rec["tokens"]["input"], 1000)
        self.assertEqual(rec["model_used"], ["gpt-fake-2026-01-01"])
        req = f.requests[0]
        self.assertEqual(req["messages"][0]["content"].split("\n")[0], "You are an expert Rust programmer. You are given a specification for one Rust library file.")
        spec = (HERE / "tasks" / "01-duration" / "spec.md").read_text(encoding="utf-8")
        self.assertEqual(req["messages"][1]["content"], f"{spec}\n\nWrite the complete Rust source file now, in one ```rust block.")
        self.assertNotIn(FAKE_KEY, "".join(pth.read_text(encoding="utf-8") for pth in out.iterdir()))

    def test_run_py_records_an_api_error_and_null_cost_without_prices(self):
        f = Fake([(200, body("", "length"))])
        self.addCleanup(f.close)
        p, out = self.run_py(f)
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads((out / "01-duration__openai__gpt-fake__high.json").read_text(encoding="utf-8"))
        self.assertIsNone(rec["cost_usd"])
        self.assertEqual(rec["exit"], 1)
        self.assertTrue(rec["call_failed"])
        self.assertFalse(rec["format_failure"])
        self.assertIn("model call failed", rec["first_error"])
        self.assertIn("finish_reason='length'", rec["first_error"])

    def test_claude_provider_rejects_openai_options(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        out = self.tmp / "out"
        p = subprocess.run([sys.executable, "-I", "-B", str(HERE / "run.py"), str(out), "claude", "m", "default", "--price-in", "2"],
                           capture_output=True, text=True, timeout=60, env=self.env)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("openai", p.stderr)


if __name__ == "__main__":
    unittest.main()
