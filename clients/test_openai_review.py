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
args = sys.argv[1:]
if args == ["--version"]:
    v = open(os.path.join(d, "version"), encoding="utf-8").read().strip()
    if v == "FAIL":
        sys.exit(3)
    print(v)
    sys.exit(0)
open(os.path.join(d, "started"), "w").write("codex was started")  # the marker: a refusal must leave it absent
mode = open(os.path.join(d, "mode"), encoding="utf-8").read().strip()
stdin = sys.stdin.read()
sys.stdout.reconfigure(encoding="utf-8")  # codex writes UTF-8; raw U+2028 must survive
catalog = None
for a in args:
    if a.startswith("model_catalog_json='") and a.endswith("'"):
        catalog = json.load(open(a[len("model_catalog_json='"):-1], encoding="utf-8"))
json.dump({"args": args, "stdin": stdin, "cwd": os.getcwd(), "env": dict(os.environ), "catalog": catalog}, open(os.path.join(d, "seen.json"), "w", encoding="utf-8"))
if mode != "silentfail":
    sys.stderr.write("unrelated MCP connection error\n")
if mode in ("fail", "silentfail"):
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
if mode == "modelnamed":  # a codex that names the answering model on thread.started, and a different one on turn.completed
    print(json.dumps({"type": "thread.started", "model": "gpt-answered-by"}))
elif mode == "modeltwo":
    print(json.dumps({"type": "thread.started", "model": "model-a"}))
elif mode == "modelnoise":  # a "model" that is not a string, or on an item: must not count
    print(json.dumps({"type": "thread.started", "model": {"x": 1}}))
else:
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
print(json.dumps({"type": "turn.completed", **({"model": "model-b"} if mode == "modeltwo" else {}), "usage": {"input_tokens": 50, "cached_input_tokens": 10, "output_tokens": 7, "reasoning_output_tokens": 3}}))
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


CODEX_VERSION = "codex-cli 9.9.9-test"

# The stand-in for scripts/shadow_canary.py inside the temp ROOT. It records that it ran (and its arguments), then passes (writing
# last-pass.json for the version in ROOT/canary_version, like the real one) or fails, as ROOT/canary_mode says.
FAKE_CANARY_PY = r'''
import json, os, sys
here = os.path.dirname(os.path.abspath(__file__))
root = os.path.dirname(here)
open(os.path.join(here, "canary-ran"), "w").write(" ".join(sys.argv[1:]))
if open(os.path.join(root, "canary_mode"), encoding="utf-8").read().strip() == "fail":
    print("leg A: FAIL, the model was offered a tool")
    print("a second line")
    sys.exit(1)
if open(os.path.join(root, "canary_mode"), encoding="utf-8").read().strip() == "pass-unrecorded":
    sys.exit(0)  # exit 0 but no last-pass.json: must not count
os.makedirs(os.path.join(root, "state", "canary"), exist_ok=True)
json.dump({"codex_version": open(os.path.join(root, "canary_version"), encoding="utf-8").read().strip()},
          open(os.path.join(root, "state", "canary", "last-pass.json"), "w"))
print("canary: PASS")
'''

MODELS_JSON = {"models": {"gpt-5.6-sol": {}}}


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

    models_json = MODELS_JSON

    def setUp(self):
        Fake.seen = []
        Fake.status = 200
        Fake.reply = completion("VERDICT: PASS\nBLOCKING: none\nMINOR: none")

    def make_codex_dir(self, mode, version=CODEX_VERSION):
        """A folder holding a FAKE codex (a script that prints canned JSONL and writes the -o file). Never the real one."""
        d = Path(tempfile.mkdtemp(prefix="fakecodex-"))
        self.addCleanup(shutil.rmtree, d, True)
        (d / "fake_codex.py").write_text(FAKE_CODEX_PY, encoding="utf-8")
        if os.name == "nt":
            (d / "codex.cmd").write_text(f'@"{sys.executable}" -I -B "%~dp0fake_codex.py" %*' + "\r\n", encoding="utf-8")
        else:
            (d / "codex").write_text(f'#!/bin/sh\nexec "{sys.executable}" -I -B "$(dirname "$0")/fake_codex.py" "$@"\n', encoding="utf-8")
            (d / "codex").chmod(0o755)
        (d / "mode").write_text(mode, encoding="utf-8")
        (d / "version").write_text(version, encoding="utf-8")
        return d

    def make_root(self, cap=None, planted="match", canary="pass"):
        """A temp ROOT holding copies of the wrapper (so the real state/ledger/canary file are never touched) and a fake canary.
        planted: "match" (last-pass.json names CODEX_VERSION), "stale", "garbage", or None (no file)."""
        root = Path(tempfile.mkdtemp(prefix="bswroot-"))
        self.addCleanup(shutil.rmtree, root, True)
        (root / "clients").mkdir()
        (root / "scripts").mkdir()
        (root / "bench").mkdir()
        for name in ("openai_review.py", "openai_mcp.py"):
            shutil.copy(HERE / name, root / "clients" / name)
        (root / "scripts" / "shadow_canary.py").write_text(FAKE_CANARY_PY, encoding="utf-8")
        (root / "bench" / "models.json").write_text(json.dumps(self.models_json), encoding="utf-8")
        (root / "broadsword.toml").write_text('state_dir = "state"\n' + ("" if cap is None else f"gpt_daily_cap_usd = {cap}\n"), encoding="utf-8")
        (root / "canary_mode").write_text(canary, encoding="utf-8")
        (root / "canary_version").write_text(CODEX_VERSION, encoding="utf-8")
        if planted is not None:
            (root / "state" / "canary").mkdir(parents=True)
            body = {"match": json.dumps({"codex_version": CODEX_VERSION}), "stale": json.dumps({"codex_version": "codex-cli 0.0.1-old"}), "garbage": "{not json"}[planted]
            (root / "state" / "canary" / "last-pass.json").write_text(body, encoding="utf-8")
        return root

    def run_script(self, prompt="the prompt", env=None, base=None, codex="fail", cwd=None, root=None, cap=None, planted="match", canary="pass",
                   version=CODEX_VERSION):
        """The wrapper runs from a temp ROOT copy (self.root). codex=<mode>: a fake codex first on PATH ("fail" by default: it answers
        --version and then fails, so the API answers). codex=None: no codex anywhere (PATH is an empty folder, APPDATA an empty folder)."""
        e = {k: v for k, v in os.environ.items() if k != "OPENAI_API_KEY" and not k.startswith("OPENAI_")}
        e["OPENAI_API_KEY"] = FAKE_KEY
        e["OPENAI_BASE_URL"] = base or self.url
        empty = Path(tempfile.mkdtemp(prefix="nocodex-"))
        self.addCleanup(shutil.rmtree, empty, True)
        e["APPDATA"] = str(empty)
        if codex is None:
            e["PATH"] = str(empty)
        else:
            self.codex_dir = self.make_codex_dir(codex, version)
            e["PATH"] = str(self.codex_dir)
        e.update(env or {})
        self.root = root or self.make_root(cap=cap, planted=planted, canary=canary)
        p = subprocess.run([sys.executable, "-I", "-B", str(self.root / "clients" / "openai_review.py"), *ARGS], input=prompt, capture_output=True, text=True, env=e, timeout=60, cwd=cwd)
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

    def test_model_reported_false_when_no_event_names_the_model(self):
        for mode in ("ok", "modelnoise"):
            with self.subTest(mode=mode):
                j = json.loads(self.run_script(codex=mode).stdout)
                self.assertEqual(j["backend"], "codex", j)
                self.assertIs(j["model_reported"], False)
                self.assertEqual(j["model_used"], [])
                self.assertEqual(list(j["modelUsage"]), ["gpt-5.6-sol"])

    def test_model_named_by_a_codex_event_is_returned_and_keys_the_usage(self):
        j = json.loads(self.run_script(codex="modelnamed").stdout)
        self.assertEqual(j["backend"], "codex", j)
        self.assertIs(j["model_reported"], True)
        self.assertEqual(j["model_used"], ["gpt-answered-by"])
        self.assertEqual(list(j["modelUsage"]), ["gpt-answered-by"], "keyed by the model that answered, not the one asked for")

    def test_two_different_reported_models_are_both_listed(self):
        j = json.loads(self.run_script(codex="modeltwo").stdout)
        self.assertIs(j["model_reported"], True)
        self.assertEqual(j["model_used"], ["model-a", "model-b"])
        self.assertEqual(list(j["modelUsage"]), ["gpt-5.6-sol"])

    def test_codex_exit_with_no_output_names_the_model(self):
        j = json.loads(self.run_script(codex="silentfail").stdout)
        self.assertEqual(j["backend"], "api")
        self.assertIn("codex exited 1 with no output (model may be unavailable: gpt-5.6-sol)", j["codex_error"])
        j = json.loads(self.run_script(codex="fail").stdout)
        self.assertIn("code 1: unrelated MCP connection error", j["codex_error"], "with stderr the old message stays")

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
        p = self.run_script(codex=None, env={"PATH": os.pathsep.join([".", "", "sub", str(cur / "nope")])}, cwd=str(cur))
        j = json.loads(p.stdout)
        self.assertTrue(j["is_error"], j)
        self.assertTrue(j["codex_error"].startswith("codex_unavailable"), j)
        self.assertEqual(Fake.seen, [], "no codex found: an error, not an API call")
        self.assertFalse((cur / "seen.json").exists())

    def test_codex_missing_is_codex_unavailable_and_nothing_is_called(self):
        p = self.run_script(codex=None)
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1, p.stderr)
        self.assertTrue(j["is_error"])
        self.assertEqual(j["backend"], "codex")
        self.assertTrue(j["codex_error"].startswith("codex_unavailable"), j)
        self.assertEqual(Fake.seen, [])
        self.assertFalse((self.root / "scripts" / "canary-ran").exists(), "no codex: no canary run")

    # ---- gate: the canary must have passed for the installed codex version ----

    def test_version_mismatch_runs_the_offline_canary_then_codex(self):
        for planted in ("stale", None, "garbage"):
            with self.subTest(planted=planted):
                p = self.run_script("PROMPT TEXT", codex="ok", planted=planted)
                j = json.loads(p.stdout)
                self.assertEqual(p.returncode, 0, p.stdout)
                self.assertEqual(j["backend"], "codex", j)
                self.assertFalse(j["is_error"], j)
                ran = self.root / "scripts" / "canary-ran"
                self.assertTrue(ran.is_file(), "the canary ran")
                self.assertEqual(ran.read_text(encoding="utf-8"), "--offline-only", "offline leg only: no model call from the automatic path")
                started = self.codex_dir / "started"
                self.assertTrue(started.is_file(), "codex was started after the canary passed")
                self.assertLessEqual(ran.stat().st_mtime_ns, started.stat().st_mtime_ns, "canary first, then codex")
                last = json.loads((self.root / "state" / "canary" / "last-pass.json").read_text(encoding="utf-8"))
                self.assertEqual(last["codex_version"], CODEX_VERSION)

    def test_a_failing_canary_refuses_and_codex_never_starts(self):
        p = self.run_script(codex="ok", planted="stale", canary="fail")
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertTrue(j["is_error"], j)
        self.assertEqual(j["backend"], "codex")
        self.assertTrue(j["codex_error"].startswith("canary_failed"), j)
        self.assertIn("leg A: FAIL", j["codex_error"], "the first line of the canary's output")
        self.assertNotIn("second line", j["codex_error"])
        self.assertTrue((self.root / "scripts" / "canary-ran").is_file(), "the canary did run")
        self.assertFalse((self.codex_dir / "started").exists(), "the fake codex was never started")
        self.assertEqual(Fake.seen, [], "and the API fallback was not used either")

    def test_a_canary_that_exits_0_without_recording_a_pass_still_refuses(self):
        p = self.run_script(codex="ok", planted=None, canary="pass-unrecorded")
        j = json.loads(p.stdout)
        self.assertTrue(j["is_error"] and j["codex_error"].startswith("canary_failed"), j)
        self.assertFalse((self.codex_dir / "started").exists())

    def test_a_missing_canary_script_refuses(self):
        root = self.make_root(planted="stale")
        (root / "scripts" / "shadow_canary.py").unlink()
        j = json.loads(self.run_script(codex="ok", root=root).stdout)
        self.assertTrue(j["is_error"] and j["codex_error"].startswith("canary_failed"), j)
        self.assertFalse((self.codex_dir / "started").exists())

    def test_matching_version_skips_the_canary(self):
        p = self.run_script(codex="ok", planted="match")
        j = json.loads(p.stdout)
        self.assertEqual(j["backend"], "codex", j)
        self.assertFalse((self.root / "scripts" / "canary-ran").exists(), "a recorded pass for this version: no canary run")
        self.assertTrue((self.codex_dir / "started").is_file())

    def test_codex_version_failing_is_codex_unavailable_with_no_canary(self):
        p = self.run_script(codex="ok", version="FAIL", planted="stale")
        j = json.loads(p.stdout)
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertTrue(j["is_error"], j)
        self.assertTrue(j["codex_error"].startswith("codex_unavailable"), j)
        self.assertFalse((self.root / "scripts" / "canary-ran").exists(), "no canary when codex itself is unavailable")
        self.assertFalse((self.codex_dir / "started").exists())
        self.assertEqual(Fake.seen, [])

    # ---- gate: the daily GPT spend cap and its ledger ----

    @staticmethod
    def today():
        import datetime
        return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")

    def plant_ledger(self, root, usd):
        d = root / "state" / "gpt-spend"
        d.mkdir(parents=True, exist_ok=True)
        (d / f"{self.today()}.tsv").write_text("utc\tcaller\tmodel\tinput_tokens\toutput_tokens\test_usd\tplan_or_credits\n"
                                               f"2026-01-01T00:00:00Z\tplanted\tgpt-5.6-sol\t1\t1\t{usd}\tcodex-plan\n", encoding="utf-8")

    def ledger_lines(self, root):
        return (root / "state" / "gpt-spend" / f"{self.today()}.tsv").read_text(encoding="utf-8").strip().split("\n")

    def test_cap_reached_refuses_before_any_call_and_writes_the_note_once(self):
        root = self.make_root()
        self.plant_ledger(root, 60.0)  # the built-in cap is 60
        for _ in range(2):
            j = json.loads(self.run_script(codex="ok", root=root).stdout)
            self.assertTrue(j["is_error"], j)
            self.assertEqual(j["codex_error"], "daily_cap_reached: 60.0000 of 60")
            self.assertFalse((self.codex_dir / "started").exists(), "no codex process")
            self.assertEqual(Fake.seen, [], "no API call")
        self.assertFalse((root / "scripts" / "canary-ran").exists())
        notes = list((root / "state" / "gpt-spend").glob("CAP-REACHED-*.txt"))
        self.assertEqual([n.name for n in notes], [f"CAP-REACHED-{self.today()}.txt"])
        self.assertEqual(len(self.ledger_lines(root)), 2, "a refused call adds no ledger row")

    def test_just_under_the_cap_still_calls(self):
        root = self.make_root()
        self.plant_ledger(root, 59.99)
        j = json.loads(self.run_script(codex="ok", root=root).stdout)
        self.assertEqual(j["backend"], "codex", j)

    def test_the_cap_comes_from_the_toml_and_zero_means_no_calls(self):
        for cap in ("0", "0.0"):
            with self.subTest(cap=cap):
                j = json.loads(self.run_script(codex="ok", cap=cap).stdout)
                self.assertTrue(j["is_error"], j)
                self.assertEqual(j["codex_error"], "daily_cap_reached: 0.0000 of 0")
                self.assertFalse((self.codex_dir / "started").exists())
                self.assertEqual(Fake.seen, [])
        root = self.make_root(cap="5")
        self.plant_ledger(root, 5.0)
        j = json.loads(self.run_script(codex="ok", root=root).stdout)
        self.assertEqual(j["codex_error"], "daily_cap_reached: 5.0000 of 5", "a lower cap in the toml wins over the built-in 60")

    def test_a_bad_cap_in_the_toml_is_an_error_and_no_call(self):
        p = self.run_script(codex="ok", cap='"lots"')
        j = json.loads(p.stdout)
        self.assertTrue(j["is_error"], j)
        self.assertFalse((self.codex_dir / "started").exists())

    def test_the_ledger_records_each_call(self):
        root = self.make_root()
        j = json.loads(self.run_script(codex="ok", root=root, env={"OPENAI_REVIEW_CALLER": "shadow"}).stdout)
        self.assertEqual(j["backend"], "codex", j)
        j2 = json.loads(self.run_script(codex="ok", root=root).stdout)
        self.assertEqual(j2["backend"], "codex")
        lines = self.ledger_lines(root)
        self.assertEqual(lines[0].split("\t"), ["utc", "caller", "model", "input_tokens", "output_tokens", "est_usd", "plan_or_credits", "call_type"])
        self.assertEqual(len(lines), 3, "one header and one row per call")
        first = lines[1].split("\t")
        self.assertEqual(first[1:5], ["shadow", "gpt-5.6-sol", "50", "7"])
        self.assertAlmostEqual(float(first[5]), (50 * 30 + 7 * 120) / 1e6, places=6, msg="no price listed anywhere: the built-in high price, never 0")
        self.assertEqual(first[6], "codex-plan")
        self.assertEqual(lines[2].split("\t")[1], "unlabelled")

    OLD_HEADER = "utc\tcaller\tmodel\tinput_tokens\toutput_tokens\test_usd\tplan_or_credits\n"

    def plant_raw(self, root, text):
        d = root / "state" / "gpt-spend"
        d.mkdir(parents=True, exist_ok=True)
        (d / f"{self.today()}.tsv").write_text(text, encoding="utf-8")

    def test_a_new_row_has_a_call_type_from_the_environment_and_unknown_when_unset_or_blank(self):
        root = self.make_root()
        for env in ({"OPENAI_REVIEW_CALL_TYPE": "gate"}, {}, {"OPENAI_REVIEW_CALL_TYPE": "  "}, {"OPENAI_REVIEW_CALL_TYPE": "a\tb\nc"}):
            self.run_script(codex="ok", root=root, env=env)
        rows = [l.split("\t") for l in self.ledger_lines(root)[1:]]
        self.assertEqual([len(r) for r in rows], [8, 8, 8, 8])
        self.assertEqual([r[7] for r in rows], ["gate", "unknown", "unknown", "a b c"])

    def test_an_old_seven_column_file_gives_the_same_cap_sum_and_is_not_rewritten(self):
        root = self.make_root()
        old = self.OLD_HEADER + "2026-01-01T00:00:00Z\tplanted\tgpt-5.6-sol\t1\t1\t59.99\tcodex-plan\n"
        self.plant_raw(root, old)
        j = json.loads(self.run_script(codex="ok", root=root).stdout)
        self.assertEqual(j["backend"], "codex", j)
        text = (root / "state" / "gpt-spend" / f"{self.today()}.tsv").read_text(encoding="utf-8")
        self.assertTrue(text.startswith(old), "the old header and row are untouched; the new row is only appended")
        root2 = self.make_root()
        self.plant_raw(root2, self.OLD_HEADER + "2026-01-01T00:00:00Z\tplanted\tgpt-5.6-sol\t1\t1\t60.0\tcodex-plan\n")
        j = json.loads(self.run_script(codex="ok", root=root2).stdout)
        self.assertEqual(j["codex_error"], "daily_cap_reached: 60.0000 of 60")

    def test_a_mixed_file_sums_old_and_new_rows(self):
        root = self.make_root()
        self.plant_raw(root, self.OLD_HEADER + "2026-01-01T00:00:00Z\told\tgpt-5.6-sol\t1\t1\t52.5\tcodex-plan\n"
                       "2026-01-01T00:00:01Z\tnew\tgpt-5.6-sol\t1\t1\t7.5\tapi-credits\tshadow\n")
        j = json.loads(self.run_script(codex="ok", root=root).stdout)
        self.assertEqual(j["codex_error"], "daily_cap_reached: 60.0000 of 60")
        self.assertFalse((self.codex_dir / "started").exists())
        self.assertEqual(Fake.seen, [])

    def test_a_malformed_row_still_fails_closed_in_old_new_and_mixed_files(self):
        for name, body in (("old", "2026-01-01T00:00:00Z\tp\tm\t1\t1\tnot-a-number\tcodex-plan\n"),
                           ("new", "2026-01-01T00:00:00Z\tp\tm\t1\t1\tnot-a-number\tcodex-plan\tgate\n"),
                           ("short", "2026-01-01T00:00:00Z\tp\tm\n"),
                           ("mixed", "2026-01-01T00:00:00Z\tp\tm\t1\t1\t0.5\tcodex-plan\n2026-01-01T00:00:00Z\tp\tm\t1\t1\t\tcodex-plan\tgate\n")):
            with self.subTest(name=name):
                Fake.seen = []
                root = self.make_root()
                self.plant_raw(root, self.OLD_HEADER + body)
                j = json.loads(self.run_script(codex="ok", root=root).stdout)
                self.assertTrue(j["is_error"], j)
                self.assertIn("unreadable row", j["result"] + j.get("codex_error", ""))
                self.assertFalse((self.codex_dir / "started").exists(), "no codex process")
                self.assertEqual(Fake.seen, [], "no API call")

    def test_an_unpriced_model_counts_at_the_highest_listed_price(self):
        self.addCleanup(setattr, self, "models_json", MODELS_JSON)
        self.models_json = {"models": {"cheap": {"price_usd_per_mtok": {"input": 1, "output": 2}},
                                       "dear": {"price_usd_per_mtok": {"input": 5, "output": 10}}, "gpt-5.6-sol": {}}}
        root = self.make_root()
        self.run_script(codex="ok", root=root)
        row = self.ledger_lines(root)[1].split("\t")
        self.assertAlmostEqual(float(row[5]), (50 * 5 + 7 * 10) / 1e6, places=6)
        self.models_json = {"models": {"cheap": {"price_usd_per_mtok": {"input": 1, "output": 2}},
                                       "gpt-5.6-sol": {"price_usd_per_mtok": {"input": 3, "output": 4}}}}
        root = self.make_root()
        self.run_script(codex="ok", root=root)
        row = self.ledger_lines(root)[1].split("\t")
        self.assertAlmostEqual(float(row[5]), (50 * 3 + 7 * 4) / 1e6, places=6, msg="a listed model counts at its own price")

    def test_an_api_fallback_call_is_recorded_as_credits(self):
        root = self.make_root()
        j = json.loads(self.run_script(codex="fail", root=root).stdout)  # codex answers --version, then fails: the API answers
        self.assertEqual(j["backend"], "api", j)
        lines = self.ledger_lines(root)
        self.assertEqual(len(lines), 3, "the failed codex attempt and the API call are each a row")
        self.assertEqual(lines[1].split("\t")[6], "codex-plan-failed")
        row = lines[2].split("\t")
        self.assertEqual((row[3], row[4], row[6]), ("100", "20", "api-credits"))

    DIRECT_DRIVER = (
        "import argparse, importlib.util, json, os, sys, time\n"
        "from pathlib import Path\n"
        "root = Path(sys.argv[1])\n"
        "spec = importlib.util.spec_from_file_location('orv', root / 'clients' / 'openai_review.py')\n"
        "m = importlib.util.module_from_spec(spec)\n"
        "spec.loader.exec_module(m)\n"
        "a = argparse.Namespace(model='gpt-5.6-sol', system_prompt=None)\n"
        "if len(sys.argv) > 3:\n"
        "    def boom(*x): raise OSError('disk full')\n"
        "    m.record_spend = boom\n"
        "try:\n"
        "    if sys.argv[2] == 'review_codex':\n"
        "        r = m.review_codex(a, 'p', dict(os.environ), time.monotonic() + 60, 60.0, 1.0)\n"
        "    else:\n"
        "        r = m.review_api(a, 'p', dict(os.environ), 60.0, 60.0)\n"
        "    print(json.dumps({'ok': r['result'][:20]}))\n"
        "except Exception as e:\n"
        "    print(json.dumps({'err': type(e).__name__, 'msg': str(e)}))\n"
    )

    def direct(self, root, fn="review_codex", codex="ok", break_ledger=False):
        """Call review_codex() / review_api() directly (as bench/reviewer/run_codex.py does), bypassing review()."""
        e = {k: v for k, v in os.environ.items() if not k.startswith("OPENAI_")}
        e["OPENAI_API_KEY"], e["OPENAI_BASE_URL"] = FAKE_KEY, self.url
        empty = Path(tempfile.mkdtemp(prefix="nocodex-"))
        self.addCleanup(shutil.rmtree, empty, True)
        e["APPDATA"] = str(empty)
        self.codex_dir = self.make_codex_dir(codex)
        e["PATH"] = str(self.codex_dir)
        p = subprocess.run([sys.executable, "-I", "-B", "-c", self.DIRECT_DRIVER, str(root), fn] + (["x"] if break_ledger else []), capture_output=True, text=True, env=e, timeout=60)
        self.assertEqual(p.returncode, 0, p.stderr)
        return json.loads(p.stdout)

    def test_review_codex_called_directly_is_capped_and_ledgered(self):
        root = self.make_root()
        out = self.direct(root)
        self.assertIn("ok", out, out)
        lines = self.ledger_lines(root)
        self.assertEqual(len(lines), 2, "one header and exactly one row for one direct call")
        self.assertEqual(lines[1].split("\t")[6], "codex-plan")
        root = self.make_root(cap="0")
        out = self.direct(root)
        self.assertEqual((out["err"], out["msg"]), ("Refused", "daily_cap_reached: 0.0000 of 0"))
        self.assertFalse((self.codex_dir / "started").exists(), "cap reached: codex never starts")
        self.assertFalse((root / "state" / "gpt-spend" / f"{self.today()}.tsv").exists())

    def test_review_api_called_directly_is_capped_and_ledgered(self):
        root = self.make_root(cap="0")
        out = self.direct(root, "review_api")
        self.assertEqual(out["err"], "Refused", out)
        self.assertEqual(Fake.seen, [])
        root = self.make_root()
        self.assertIn("ok", self.direct(root, "review_api"))
        self.assertEqual(self.ledger_lines(root)[1].split("\t")[6], "api-credits")

    def test_a_ledger_that_cannot_be_written_closes_the_gate_for_later_calls(self):
        root = self.make_root()
        out = self.direct(root, break_ledger=True)
        self.assertIn("ok", out, "the answer is kept")
        self.assertTrue((root / "state" / "gpt-spend" / f"LEDGER-FAILED-{self.today()}.txt").is_file())
        (self.codex_dir / "started").unlink()
        out = self.direct(root)
        self.assertEqual(out["err"], "Refused", out)
        self.assertTrue(out["msg"].startswith("ledger_unwritable"), out)
        self.assertFalse((self.codex_dir / "started").exists())

    def test_a_result_with_several_models_is_summed_each_at_its_own_price(self):
        root = self.make_root()
        driver = (
            "import importlib.util, sys\nfrom pathlib import Path\nroot = Path(sys.argv[1])\n"
            "spec = importlib.util.spec_from_file_location('orv', root / 'clients' / 'openai_review.py')\n"
            "m = importlib.util.module_from_spec(spec)\nspec.loader.exec_module(m)\n"
            "m.record_spend(root, {}, 'a', {'modelUsage': {'a': {'inputTokens': 10, 'outputTokens': 1}, 'b': {'inputTokens': 5, 'outputTokens': 2}}}, 'api')\n")
        subprocess.run([sys.executable, "-I", "-B", "-c", driver, str(root)], check=True, timeout=60)
        row = self.ledger_lines(root)[1].split("\t")
        self.assertEqual((row[3], row[4]), ("15", "3"))
        self.assertAlmostEqual(float(row[5]), (15 * 30 + 3 * 120) / 1e6, places=6)

    def test_concurrent_appends_under_the_lock_lose_no_rows(self):
        root = self.make_root()
        driver = (
            "import importlib.util, sys\n"
            "from pathlib import Path\n"
            "root = Path(sys.argv[1])\n"
            "spec = importlib.util.spec_from_file_location('orv', root / 'clients' / 'openai_review.py')\n"
            "m = importlib.util.module_from_spec(spec)\n"
            "spec.loader.exec_module(m)\n"
            "for i in range(40):\n"
            "    m.record_spend(root, {'OPENAI_REVIEW_CALLER': sys.argv[2]}, 'gpt-5.6-sol', {'modelUsage': {'x': {'inputTokens': 1, 'outputTokens': 1}}}, 'codex')\n"
        )
        procs = [subprocess.Popen([sys.executable, "-I", "-B", "-c", driver, str(root), name], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for name in ("A", "B", "C")]
        for pr in procs:
            _, err = pr.communicate(timeout=60)
            self.assertEqual(pr.returncode, 0, err)
        lines = self.ledger_lines(root)
        self.assertEqual(len(lines), 1 + 120, "one header (written once) and every row of the three writers")
        self.assertEqual(sum(1 for x in lines if x.startswith("utc\t")), 1)
        for who in "ABC":
            self.assertEqual(sum(1 for x in lines[1:] if x.split("\t")[1] == who), 40)
        self.assertTrue(all(len(x.split("\t")) == 8 for x in lines), "no row is torn")

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
