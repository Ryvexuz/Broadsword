"""Tests for clients/openai_review.py. No real API: a fake OpenAI server runs on 127.0.0.1 in a thread, with a fake key.

Run:  env -u OPENAI_API_KEY python -I -B -m unittest discover -s clients -p "test_openai_review.py" -v
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import json
import os
import shutil
import subprocess
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
SCRIPT = HERE / "openai_review.py"
FAKE_KEY = "sk-test-SECRET123"
ARGS = ["-p", "--system-prompt", "SYS", "--model", "gpt-5.6-sol", "--tools", "", "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"]


class Fake(BaseHTTPRequestHandler):
    reply = {}
    status = 200
    seen = []

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        Fake.seen.append({"auth": self.headers.get("Authorization"), "body": body, "path": self.path})
        data = json.dumps(Fake.reply).encode()
        self.send_response(Fake.status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, *a):
        pass


FAKE_CODEX_PY = r'''
import json, os, sys
d = os.path.dirname(os.path.abspath(__file__))
mode = open(os.path.join(d, "mode"), encoding="utf-8").read().strip()
args = sys.argv[1:]
stdin = sys.stdin.read()
json.dump({"args": args, "stdin": stdin, "cwd": os.getcwd()}, open(os.path.join(d, "seen.json"), "w", encoding="utf-8"))
sys.stderr.write("unrelated MCP connection error\n")
if mode == "fail":
    sys.exit(1)
if mode == "hang":
    import subprocess, time
    # a grandchild that inherits the output pipes and outlives the timeout, like node behind codex.cmd
    subprocess.Popen([sys.executable, "-c", "import time; time.sleep(25)"], stdin=subprocess.DEVNULL, stdout=sys.stdout, stderr=sys.stderr)
    time.sleep(60)
out = args[args.index("-o") + 1]
open(out, "w", encoding="utf-8").write("" if mode == "empty" else "VERDICT: FAIL\nBLOCKING: add is wrong\nMINOR: none")
if mode == "garbage":
    print("this is not json")
    sys.exit(0)
print(json.dumps({"type": "thread.started"}))
print(json.dumps({"type": "turn.completed", "usage": {"input_tokens": 50, "cached_input_tokens": 10, "output_tokens": 7, "reasoning_output_tokens": 3}}))
'''


def completion(text, finish="stop", model="gpt-5.6-sol-2026-01-01"):
    return {"model": model, "choices": [{"finish_reason": finish, "message": {"role": "assistant", "content": text}}], "usage": {"prompt_tokens": 100, "completion_tokens": 20}}


class ReviewTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.server = ThreadingHTTPServer(("127.0.0.1", 0), Fake)
        cls.url = f"http://127.0.0.1:{cls.server.server_address[1]}/v1"
        threading.Thread(target=cls.server.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.server.server_close()

    def setUp(self):
        Fake.seen = []
        Fake.status = 200
        Fake.reply = completion("VERDICT: PASS\nBLOCKING: none\nMINOR: none")

    def make_codex_dir(self, mode):
        """A folder holding a FAKE codex (a script that prints canned JSONL and writes the -o file). Never the real one."""
        d = Path(tempfile.mkdtemp(prefix="fakecodex-"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "fake_codex.py").write_text(FAKE_CODEX_PY, encoding="utf-8")
        if os.name == "nt":
            (d / "codex.cmd").write_text(f'@"{sys.executable}" -I -B "%~dp0fake_codex.py" %*\r\n', encoding="utf-8")
        else:
            (d / "codex").write_text(f'#!/bin/sh\nexec "{sys.executable}" -I -B "$(dirname "$0")/fake_codex.py" "$@"\n', encoding="utf-8")
            (d / "codex").chmod(0o755)
        (d / "mode").write_text(mode, encoding="utf-8")
        return d

    def run_script(self, prompt="the prompt", env=None, base=None, codex=None):
        """codex=None: no codex anywhere (PATH is an empty folder, APPDATA an empty folder). codex=<mode>: a fake codex first on PATH."""
        e = {k: v for k, v in os.environ.items() if k != "OPENAI_API_KEY" and not k.startswith("OPENAI_")}
        e["OPENAI_API_KEY"] = FAKE_KEY
        e["OPENAI_BASE_URL"] = base or self.url
        empty = Path(tempfile.mkdtemp(prefix="nocodex-"))
        self.addCleanup(shutil.rmtree, empty, True)
        e["APPDATA"] = str(empty)
        if codex is None:
            e["PATH"] = str(empty)
        else:
            self.codex_dir = self.make_codex_dir(codex)
            e["PATH"] = str(self.codex_dir)
        e.update(env or {})
        p = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), *ARGS], input=prompt, capture_output=True, text=True, env=e, timeout=60)
        return p

    def test_success_prints_the_claude_cli_json(self):
        p = self.run_script("PROMPT TEXT")
        self.assertEqual(p.returncode, 0, p.stderr)
        j = json.loads(p.stdout)
        self.assertFalse(j["is_error"])
        self.assertEqual(j["result"], "VERDICT: PASS\nBLOCKING: none\nMINOR: none")
        self.assertIsNone(j["total_cost_usd"])
        self.assertEqual((j["usage"]["input_tokens"], j["usage"]["output_tokens"]), (100, 20))
        self.assertEqual(list(j["modelUsage"]), ["gpt-5.6-sol-2026-01-01"], "keyed by the model the API says answered")
        seen = Fake.seen[0]
        self.assertEqual(seen["auth"], "Bearer " + FAKE_KEY)
        self.assertEqual(seen["body"]["model"], "gpt-5.6-sol")
        self.assertEqual(seen["body"]["messages"], [{"role": "system", "content": "SYS"}, {"role": "user", "content": "PROMPT TEXT"}])
        self.assertNotIn(FAKE_KEY, p.stdout + p.stderr)

    def test_cost_only_when_prices_are_configured(self):
        p = self.run_script(env={"OPENAI_PRICE_INPUT_PER_MTOK": "10", "OPENAI_PRICE_OUTPUT_PER_MTOK": "40"})
        self.assertAlmostEqual(json.loads(p.stdout)["total_cost_usd"], (100 * 10 + 20 * 40) / 1e6)
        p = self.run_script(env={"OPENAI_PRICE_INPUT_PER_MTOK": "10"})
        self.assertIsNone(json.loads(p.stdout)["total_cost_usd"], "one price alone is not enough")

    def test_empty_answer_is_an_error(self):
        Fake.reply = completion("   ")
        p = self.run_script()
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1)
        self.assertTrue(j["is_error"] and "empty" in j["result"])

    def test_finish_reason_other_than_stop_is_an_error(self):
        Fake.reply = completion("VERDICT: PASS\nBLOCKING: none", finish="length")
        p = self.run_script()
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1)
        self.assertTrue(j["is_error"] and "length" in j["result"], j)

    def test_api_error_is_an_error_without_the_key(self):
        Fake.status = 401
        Fake.reply = {"error": {"message": f"bad key {FAKE_KEY}"}}
        p = self.run_script()
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1)
        self.assertTrue(j["is_error"] and "401" in j["result"])
        self.assertNotIn(FAKE_KEY, p.stdout + p.stderr)

    def test_codex_answers_first_and_the_api_is_not_called(self):
        p = self.run_script("PROMPT TEXT", codex="ok")
        self.assertEqual(p.returncode, 0, p.stderr)
        j = json.loads(p.stdout)
        self.assertEqual(j["backend"], "codex")
        self.assertNotIn("codex_error", j)
        self.assertFalse(j["is_error"])
        self.assertTrue(j["result"].startswith("VERDICT: FAIL"))
        self.assertIsNone(j["total_cost_usd"])
        self.assertIs(j["model_reported"], False)
        self.assertEqual(j["usage"]["input_tokens"], 50)
        self.assertEqual(j["usage"]["output_tokens"], 7)
        self.assertEqual(list(j["modelUsage"]), ["gpt-5.6-sol"], "keyed by the requested model")
        self.assertEqual(Fake.seen, [], "the API was not called")
        seen = json.loads((self.codex_dir / "seen.json").read_text(encoding="utf-8"))
        for flag in ["exec", "--sandbox", "read-only", "--ephemeral", "--skip-git-repo-check", "--json", "-o", "-", "-m", "gpt-5.6-sol"]:
            self.assertIn(flag, seen["args"])
        self.assertLess(seen["stdin"].index("SYS"), seen["stdin"].index("PROMPT TEXT"), "system prompt first")
        self.assertIn("SYSTEM", seen["stdin"])
        cwd = Path(seen["cwd"]).resolve()
        self.assertFalse(cwd.exists(), "the temp working folder is deleted afterwards")
        self.assertTrue(str(cwd).startswith(str(Path(tempfile.gettempdir()).resolve())), "under the system temp folder")
        self.assertNotIn(FAKE_KEY, p.stdout + p.stderr)

    def test_codex_missing_falls_back_to_the_api(self):
        p = self.run_script()
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(j["backend"], "api")
        self.assertIn("not found", j["codex_error"])
        self.assertEqual(len(Fake.seen), 1)

    def test_codex_failing_falls_back_and_records_why(self):
        for mode, needle in (("fail", "code 1"), ("empty", "empty")):
            with self.subTest(mode=mode):
                Fake.seen = []
                p = self.run_script(codex=mode)
                j = json.loads(p.stdout)
                self.assertEqual(p.returncode, 0, p.stderr)
                self.assertEqual(j["backend"], "api")
                self.assertIn(needle, j["codex_error"])
                self.assertFalse(j["is_error"])
                self.assertEqual(len(Fake.seen), 1)

    def test_garbage_codex_output_falls_back_and_records_why(self):
        p = self.run_script(codex="garbage")
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(j["backend"], "api")
        self.assertIn("turn.completed", j["codex_error"])
        self.assertEqual(len(Fake.seen), 1)

    def test_hung_codex_with_a_grandchild_times_out_and_falls_back_within_one_deadline(self):
        t0 = time.monotonic()
        p = self.run_script(codex="hang", env={"OPENAI_REVIEW_TIMEOUT": "20"})
        elapsed = time.monotonic() - t0
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertEqual(j["backend"], "api")
        self.assertIn("timed out", j["codex_error"])
        self.assertEqual(len(Fake.seen), 1)
        self.assertLess(elapsed, 22, "codex timeout plus API fallback stay inside the one 20 s deadline (plus start-up)")

    def test_api_fallback_fails_clearly_when_under_five_seconds_remain(self):
        t0 = time.monotonic()
        p = self.run_script(codex="hang", env={"OPENAI_REVIEW_TIMEOUT": "6"})  # codex gets 3.6 s, 2.4 s remain
        elapsed = time.monotonic() - t0
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1)
        self.assertTrue(j["is_error"] and "remain" in j["result"], j)
        self.assertIn("timed out", j["codex_error"])
        self.assertEqual(Fake.seen, [], "the API was not called")
        self.assertLess(elapsed, 8)

    def test_both_failing_is_an_error_that_records_both(self):
        Fake.status = 401
        Fake.reply = {"error": {"message": "nope"}}
        p = self.run_script(codex="fail")
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1)
        self.assertTrue(j["is_error"] and "401" in j["result"])
        self.assertEqual(j["backend"], "api")
        self.assertIn("code 1", j["codex_error"])

    def test_unreachable_server_is_an_error(self):
        p = self.run_script(base="http://127.0.0.1:1/v1")
        self.assertEqual(p.returncode, 1)
        j = json.loads(p.stdout)
        self.assertTrue(j["is_error"])
        self.assertNotIn(FAKE_KEY, p.stdout + p.stderr)


if __name__ == "__main__":
    unittest.main()
