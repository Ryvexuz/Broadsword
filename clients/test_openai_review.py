"""Tests for clients/openai_review.py. No real API: a fake OpenAI server runs on 127.0.0.1 in a thread, with a fake key.

Run:  env -u OPENAI_API_KEY python -I -B -m unittest discover -s clients -p "test_openai_review.py" -v
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import json
import os
import subprocess
import threading
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

    def run_script(self, prompt="the prompt", env=None, base=None):
        e = {k: v for k, v in os.environ.items() if k != "OPENAI_API_KEY" and not k.startswith("OPENAI_")}
        e["OPENAI_API_KEY"] = FAKE_KEY
        e["OPENAI_BASE_URL"] = base or self.url
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

    def test_unreachable_server_is_an_error(self):
        p = self.run_script(base="http://127.0.0.1:1/v1")
        self.assertEqual(p.returncode, 1)
        j = json.loads(p.stdout)
        self.assertTrue(j["is_error"])
        self.assertNotIn(FAKE_KEY, p.stdout + p.stderr)


if __name__ == "__main__":
    unittest.main()
