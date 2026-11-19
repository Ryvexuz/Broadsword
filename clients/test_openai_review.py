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
sys.stdout.reconfigure(encoding="utf-8")  # codex writes UTF-8; raw U+2028 must survive
catalog = None
for a in args:
    if a.startswith("model_catalog_json='") and a.endswith("'"):
        catalog = json.load(open(a[len("model_catalog_json='"):-1], encoding="utf-8"))
json.dump({"args": args, "stdin": stdin, "cwd": os.getcwd(), "env": dict(os.environ), "catalog": catalog}, open(os.path.join(d, "seen.json"), "w", encoding="utf-8"))
sys.stderr.write("unrelated MCP connection error\n")
if mode == "fail":
    sys.exit(1)
if mode == "hang":
    import subprocess, time
    # a grandchild that inherits the output pipes and outlives the timeout, like node behind codex.cmd
    subprocess.Popen([sys.executable, "-c", "import time; time.sleep(25)"], stdin=subprocess.DEVNULL, stdout=sys.stdout, stderr=sys.stderr)
    time.sleep(60)
if mode == "slow":
    import time
    time.sleep(6)
out = args[args.index("-o") + 1]
open(out, "w", encoding="utf-8").write("" if mode == "empty" else "VERDICT: FAIL\nBLOCKING: add is wrong\nMINOR: none")
if mode == "garbage":
    print("this is not json")
    sys.exit(0)
print(json.dumps({"type": "thread.started"}))
print(json.dumps({"type": "item.completed", "item": {"id": "item_0", "type": "error", "message": "Falling back from WebSockets to HTTPS transport."}}))
print(json.dumps({"type": "item.completed", "item": {"id": "item_1", "type": "reasoning", "text": "thinking"}}))
if mode == "mcplist":
    # what codex 0.170.0 printed with b640db9's flags: the built-in resource listing, answering []
    print(json.dumps({"type": "item.completed", "item": {"id": "item_4", "type": "mcp_tool_call", "server": "codex", "tool": "list_mcp_resources", "arguments": {}, "result": {"content": [{"type": "text", "text": "{\"resources\":[]}"}]}, "status": "completed"}}))
if mode == "u2028":
    # a tool item whose text holds raw U+2028 / U+2029 / U+0085 (codex does not escape them): str.splitlines() cut it apart
    print(json.dumps({"type": "item.started", "item": {"id": "item_6", "type": "command_execution", "command": "type x y z\u0085w"}}, ensure_ascii=False))
if mode == "u2028ok":
    print(json.dumps({"type": "item.completed", "item": {"id": "item_7", "type": "reasoning", "text": "a b c\u0085d"}}, ensure_ascii=False))
if mode == "newevent":
    print(json.dumps({"type": "turn.tool_call", "name": "something new"}))
if mode == "readres":
    print(json.dumps({"type": "item.completed", "item": {"id": "item_5", "type": "mcp_tool_call", "server": "codex", "tool": "read_mcp_resource", "arguments": {"uri": "file:///x"}, "status": "completed"}}))
if mode == "tool":
    # what the real codex printed before the fix: the model ran a command that read a file
    print(json.dumps({"type": "item.started", "item": {"id": "item_2", "type": "command_execution", "command": "Get-Content ../planted-secret.txt"}}))
    open(out, "w", encoding="utf-8").write("CANARY: TOOLREAD-SECRET-42\nVERDICT: PASS")
print(json.dumps({"type": "item.completed", "item": {"id": "item_3", "type": "agent_message", "text": "VERDICT: FAIL"}}))
print(json.dumps({"type": "turn.completed", "usage": {"input_tokens": 50, "cached_input_tokens": 10, "output_tokens": 7, "reasoning_output_tokens": 3}}))
'''

# Flags that must reach codex so the model has no tool that reads a file or runs a command (docs/SHADOW_SANDBOX.md). Written out
# here on purpose, not imported from the wrapper, so a flag dropped from the wrapper fails this test.
NO_FILE_ACCESS_FLAGS = ["--sandbox read-only", "--ephemeral", "--skip-git-repo-check", "--ignore-user-config", "--ignore-rules",
                        "--strict-config", "-c tools.experimental_request_user_input.enabled=false",
                        "--disable shell_tool", "--disable unified_exec", "--disable view_image", "--disable apps", "--disable plugins",
                        "--disable remote_plugin", "--disable browser_use", "--disable browser_use_external", "--disable in_app_browser",
                        "--disable computer_use", "--disable image_generation", "--disable multi_agent", "--disable multi_agent_v2",
                        "--disable sleep_tool", "--disable goals", "--disable tool_suggest", "--disable skill_search",
                        "--disable skill_mcp_dependency_install", "--disable hooks", "--disable memories",
                        "--disable workspace_dependencies", '-c web_search="disabled"', "-c project_doc_max_bytes=0",
                        '-c approval_policy="never"']


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

    def run_script(self, prompt="the prompt", env=None, base=None, codex=None, cwd=None):
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
        p = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), *ARGS], input=prompt, capture_output=True, text=True, env=e, timeout=60, cwd=cwd)
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

    def test_codex_gets_the_no_file_access_settings(self):
        p = self.run_script(codex="ok")
        self.assertEqual(json.loads(p.stdout)["backend"], "codex", p.stdout)
        args = json.loads((self.codex_dir / "seen.json").read_text(encoding="utf-8"))["args"]
        pairs = {f"{a} {b}" for a, b in zip(args, args[1:])}
        for flag in NO_FILE_ACCESS_FLAGS:
            if " " in flag:
                self.assertIn(flag, pairs, f"codex must get `{flag}`")
            else:
                self.assertIn(flag, args, f"codex must get `{flag}`")
        self.assertNotIn("--sandbox workspace-write", pairs)
        self.assertNotIn("--dangerously-bypass-approvals-and-sandbox", args)
        self.assertNotIn("--search", args)
        self.assertLess(args.index("--strict-config"), args.index("-"), "a setting codex does not know stops it before the prompt is sent")

    def test_codex_gets_a_no_tools_catalog_for_exactly_the_requested_model(self):
        p = self.run_script(codex="ok")
        self.assertEqual(json.loads(p.stdout)["backend"], "codex", p.stdout)
        seen = json.loads((self.codex_dir / "seen.json").read_text(encoding="utf-8"))
        args = seen["args"]
        self.assertEqual(args[args.index("-m") + 1], "gpt-5.6-sol")
        self.assertEqual(sum(1 for a in args if a.startswith("model_catalog_json=")), 1)
        self.assertEqual(args[args.index([a for a in args if a.startswith("model_catalog_json=")][0]) - 1], "-c")
        cat = seen["catalog"]
        self.assertIsNotNone(cat, "the catalog file existed while codex ran")
        self.assertEqual(len(cat["models"]), 1)
        m = cat["models"][0]
        self.assertEqual(m["slug"], "gpt-5.6-sol", "the entry must match -m, or codex would use its own entry and its tools")
        self.assertEqual(m["tool_mode"], "direct", "not code_mode_only: no `exec` tool")
        self.assertEqual(m["shell_type"], "disabled")
        self.assertEqual(m["experimental_supported_tools"], [])
        self.assertIs(m["supports_search_tool"], False)
        for absent in ("apply_patch_tool_type", "multi_agent_version", "web_search_tool_type"):
            self.assertNotIn(absent, m, f"`{absent}` would bring a tool back")

    def test_codex_tool_use_discards_the_answer_and_falls_back(self):
        p = self.run_script(codex="tool")
        j = json.loads(p.stdout)
        self.assertEqual(j["backend"], "api", j)
        self.assertIn("codex used a tool or printed an unexpected event (command_execution)", j["codex_error"])
        self.assertNotIn("TOOLREAD-SECRET-42", p.stdout + p.stderr, "the answer of a codex that used a tool is never printed")
        self.assertEqual(len(Fake.seen), 1, "the API (which has no tools) answers instead")

    def test_any_mcp_call_trips_now_that_no_tool_is_offered(self):
        for mode, label in (("readres", "mcp_tool_call:codex/read_mcp_resource"), ("mcplist", "mcp_tool_call:codex/list_mcp_resources")):
            with self.subTest(mode=mode):
                Fake.seen = []
                j = json.loads(self.run_script(codex=mode).stdout)
                self.assertEqual(j["backend"], "api", j)
                self.assertIn(label, j["codex_error"])

    def test_a_tool_item_holding_unicode_line_separators_still_trips(self):
        p = self.run_script(codex="u2028")
        j = json.loads(p.stdout)
        self.assertEqual(j["backend"], "api", j)
        self.assertIn("command_execution", j["codex_error"])
        self.assertNotIn("unparseable", j["codex_error"], "the line is read whole, not cut at U+2028")

    def test_unicode_line_separators_inside_a_tool_free_item_pass(self):
        j = json.loads(self.run_script(codex="u2028ok").stdout)
        self.assertEqual(j["backend"], "codex", j)
        self.assertEqual(j["codex_items"], ["agent_message", "error", "reasoning"])

    def test_an_unknown_event_or_a_bad_line_trips(self):
        for mode, label in (("newevent", "event:turn.tool_call"), ("garbage", "unparseable")):
            with self.subTest(mode=mode):
                Fake.seen = []
                j = json.loads(self.run_script(codex=mode).stdout)
                self.assertEqual(j["backend"], "api", j)
                self.assertIn(label, j["codex_error"])

    def test_codex_items_are_reported_and_tool_free_ones_pass(self):
        p = self.run_script(codex="ok")
        j = json.loads(p.stdout)
        self.assertEqual(j["backend"], "codex", j)
        self.assertEqual(j["codex_items"], ["agent_message", "error", "reasoning"])

    def test_codex_child_env_has_no_api_key(self):
        p = self.run_script(codex="ok", env={"OPENAI_API_KEY": FAKE_KEY, "COPY_OF_KEY": FAKE_KEY})
        self.assertEqual(json.loads(p.stdout)["backend"], "codex", p.stdout)
        seen = json.loads((self.codex_dir / "seen.json").read_text(encoding="utf-8"))
        self.assertNotIn("OPENAI_API_KEY", {k.upper() for k in seen["env"]})
        self.assertNotIn("COPY_OF_KEY", seen["env"], "the same secret under another name is dropped too")
        self.assertNotIn(FAKE_KEY, json.dumps(seen["env"]))
        self.assertTrue(seen["env"].get("PATH"), "ordinary variables survive")

    def test_codex_env_drops_the_key_and_every_dotenv_value(self):
        root = Path(tempfile.mkdtemp(prefix="fakeroot-"))
        self.addCleanup(shutil.rmtree, root, True)
        (root / ".env").write_text('OPENAI_API_KEY=sk-from-file\nOTHER_THING="hello-secret-value"\nEMPTY=\n', encoding="utf-8")
        environ = {"OPENAI_API_KEY": "sk-env-key", "openai_api_key": "x", "RENAMED": "hello-secret-value", "COPY": "sk-env-key",
                   "FILECOPY": "sk-from-file", "PATH": "p", "APPDATA": "a", "BLANK": ""}
        sys.path.insert(0, str(HERE))
        try:
            import importlib.util
            spec = importlib.util.spec_from_file_location("openai_review_under_test", SCRIPT)
            m = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(m)
        finally:
            sys.path.remove(str(HERE))
        out = m.codex_env(environ, root)
        self.assertEqual(out, {"PATH": "p", "APPDATA": "a", "BLANK": ""})

    def test_a_codex_in_the_current_folder_is_not_picked_up(self):
        cur = Path(tempfile.mkdtemp(prefix="curdir-"))
        self.addCleanup(shutil.rmtree, cur, True)
        for name in ("codex.cmd", "codex.exe", "codex"):
            (cur / name).write_text("echo PWNED\n", encoding="utf-8")
        # PATH holds "." , an empty entry and a relative one; none may be searched
        p = self.run_script(env={"PATH": os.pathsep.join([".", "", "sub", str(cur / "nope")])}, cwd=str(cur))
        j = json.loads(p.stdout)
        self.assertEqual(j["backend"], "api", j)
        self.assertIn("not found", j["codex_error"])
        self.assertEqual(len(Fake.seen), 1)
        self.assertFalse((cur / "seen.json").exists())

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
        self.assertIn("unparseable", j["codex_error"], "a line that is not JSON trips the tripwire (fail closed)")
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

    def test_codex_timeout_then_api_error_names_both(self):
        Fake.status = 429
        Fake.reply = {"error": {"message": "You have no credits remaining"}}
        p = self.run_script(codex="hang", env={"OPENAI_REVIEW_TIMEOUT": "12", "OPENAI_REVIEW_CODEX_SHARE": "0.3"})  # codex gets 3.6 s
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1)
        self.assertTrue(j["is_error"])
        self.assertTrue(j["result"].startswith("codex: "), j["result"])
        self.assertIn("timed out", j["result"])
        self.assertIn("; api: ", j["result"])
        self.assertIn("429", j["result"])
        self.assertIn("timed out", j["codex_error"])
        self.assertEqual(j["backend"], "api")
        self.assertEqual(len(Fake.seen), 1)

    def test_invalid_codex_share_is_an_error_naming_it(self):
        for bad in ("0", "1.5", "x", "-1", "nan", ""):
            with self.subTest(share=bad):
                Fake.seen = []
                p = self.run_script(env={"OPENAI_REVIEW_TIMEOUT": "30", "OPENAI_REVIEW_CODEX_SHARE": bad})
                j = json.loads(p.stdout)
                self.assertEqual(p.returncode, 1)
                self.assertTrue(j["is_error"] and "OPENAI_REVIEW_CODEX_SHARE" in j["result"], j)
                self.assertEqual(Fake.seen, [], "no call is made on a bad setting")

    def test_invalid_timeout_is_an_error_naming_it(self):
        for bad in ("0", "-5", "x", "inf", ""):
            with self.subTest(timeout=bad):
                Fake.seen = []
                p = self.run_script(env={"OPENAI_REVIEW_TIMEOUT": bad, "OPENAI_REVIEW_CODEX_SHARE": "0.5"})
                j = json.loads(p.stdout)
                self.assertEqual(p.returncode, 1)
                self.assertTrue(j["is_error"] and "OPENAI_REVIEW_TIMEOUT" in j["result"], j)
                self.assertEqual(Fake.seen, [])

    def test_share_of_one_gives_codex_the_whole_deadline(self):
        # the fake codex answers after 6 s: with share 1.0 and 8 s it fits; with 0.5 (4 s) it would time out
        p = self.run_script(codex="slow", env={"OPENAI_REVIEW_TIMEOUT": "8", "OPENAI_REVIEW_CODEX_SHARE": "1.0"})
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 0, p.stdout)
        self.assertEqual(j["backend"], "codex", j)
        self.assertEqual(Fake.seen, [])
        self.assertNotIn("notes", j, "both settings were given, so no legacy-default note")

    def test_absent_settings_use_the_legacy_values_and_say_so(self):
        p = self.run_script()  # no codex: fallback; neither variable set
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 0, p.stdout)
        notes = " ".join(j["notes"])
        self.assertIn("OPENAI_REVIEW_TIMEOUT", notes)
        self.assertIn("OPENAI_REVIEW_CODEX_SHARE", notes)
        self.assertIn("OPENAI_REVIEW_TIMEOUT", j["codex_error"])
        p = self.run_script(codex="ok")
        self.assertIn("OPENAI_REVIEW_CODEX_SHARE", " ".join(json.loads(p.stdout)["notes"]))

    def test_unreachable_server_is_an_error(self):
        p = self.run_script(base="http://127.0.0.1:1/v1")
        self.assertEqual(p.returncode, 1)
        j = json.loads(p.stdout)
        self.assertTrue(j["is_error"])
        self.assertNotIn(FAKE_KEY, p.stdout + p.stderr)


if __name__ == "__main__":
    unittest.main()
