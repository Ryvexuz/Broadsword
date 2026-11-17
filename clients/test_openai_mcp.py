"""Tests for clients/openai_mcp.py. No network: a fake OpenAI server runs on 127.0.0.1 in a thread.

Run:  python -I -B -m unittest discover -s clients -p "test_openai_mcp.py"

The tests talk to the REAL Broadsword MCP server (the installed binary in <main root>/build/bin), but in a throwaway
root under <main root>/scratch/ that has its own tiny git cargo project. The fake key below is not a real key.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import contextlib
import importlib.util
import json
import os
import shutil
import subprocess
import sys
import threading
import unittest
import uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

HERE = Path(__file__).resolve().parent
CLIENT = HERE / "openai_mcp.py"

_spec = importlib.util.spec_from_file_location("openai_mcp", CLIENT)
oc = importlib.util.module_from_spec(_spec)
sys.modules["openai_mcp"] = oc
_spec.loader.exec_module(oc)

FAKE_KEY = "sk-test-SECRET123"
MAIN_ROOT = oc.find_main_root(HERE)
MAIN_EXE = oc.default_server_path(MAIN_ROOT)


@contextlib.contextmanager
def scratch_tmp():
    """A temporary folder under <main root>/scratch (never %TEMP%), removed afterwards."""
    d = MAIN_ROOT / "scratch" / f"m3-unit-{uuid.uuid4().hex[:8]}"
    d.mkdir(parents=True)
    try:
        yield str(d)
    finally:
        rm_tree(d)


def rm_tree(path: Path) -> None:
    def onerror(func, p, _exc):
        os.chmod(p, 0o700)
        func(p)

    shutil.rmtree(path, onerror=onerror)


# ---- fake OpenAI ------------------------------------------------------------------------------


def final(text, usage=(10, 5)):
    return {"status": 200, "body": {"model": "fake-model", "choices": [{"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": text}}],
                                     "usage": {"prompt_tokens": usage[0], "completion_tokens": usage[1], "total_tokens": usage[0] + usage[1]}}}


def tool_calls(*calls, usage=(20, 7)):
    tcs = [{"id": f"call_{i}", "type": "function", "function": {"name": n, "arguments": json.dumps(a)}} for i, (n, a) in enumerate(calls)]
    return {"status": 200, "body": {"model": "fake-model", "choices": [{"index": 0, "finish_reason": "tool_calls", "message": {"role": "assistant", "content": None, "tool_calls": tcs}}],
                                     "usage": {"prompt_tokens": usage[0], "completion_tokens": usage[1], "total_tokens": usage[0] + usage[1]}}}


def http_error(status, message, headers=None):
    return {"status": status, "headers": headers or {}, "body": {"error": {"message": message, "type": "x"}}}


class FakeOpenAI:
    def __init__(self, script):
        self.script, self.requests, self.auth = list(script), [], []
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                n = int(self.headers.get("Content-Length", 0))
                outer.requests.append({"path": self.path, "json": json.loads(self.rfile.read(n))})
                outer.auth.append(self.headers.get("Authorization"))
                step = outer.script[min(len(outer.requests) - 1, len(outer.script) - 1)]
                data = step["body"].encode() if isinstance(step["body"], str) else json.dumps(step["body"]).encode()
                self.send_response(step["status"])
                for k, v in step.get("headers", {}).items():
                    self.send_header(k, v)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *a):
                pass

        self.httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self.httpd.server_port}/v1"
        self.thread = threading.Thread(target=self.httpd.serve_forever, daemon=True)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *a):
        self.httpd.shutdown()
        self.httpd.server_close()

    def tool_messages(self, request_index):
        return [m for m in self.requests[request_index]["json"]["messages"] if m.get("role") == "tool"]


# ---- throwaway Broadsword root ----------------------------------------------------------------


class RootCase(unittest.TestCase):
    """A throwaway root under <main root>/scratch with its own copy of the server and a tiny git cargo project."""

    @classmethod
    def setUpClass(cls):
        if not MAIN_EXE.is_file():
            raise unittest.SkipTest(f"{MAIN_EXE} not installed: run `broadsword install` first")
        cls.root = MAIN_ROOT / "scratch" / f"m3-test-{uuid.uuid4().hex[:8]}"
        (cls.root / "build" / "bin").mkdir(parents=True)
        shutil.copy(HERE.parent / "broadsword.toml", cls.root / "broadsword.toml")
        shutil.copy(MAIN_EXE, cls.root / "build" / "bin" / MAIN_EXE.name)
        proj = cls.root / "projects" / "tiny"
        (proj / "src").mkdir(parents=True)
        (proj / "Cargo.toml").write_text('[package]\nname = "tiny"\nversion = "0.1.0"\nedition = "2021"\n\n[workspace]\n', encoding="utf-8")
        (proj / "src" / "lib.rs").write_text("pub fn add(a: i32, b: i32) -> i32 { a + b }\n\n#[cfg(test)]\nmod t {\n    #[test]\n    fn adds() { assert_eq!(super::add(1, 2), 3); }\n}\n", encoding="utf-8")
        git = ["git", "-c", "user.name=t", "-c", "user.email=t@example.com"]
        for cmd in (["init", "-q"], ["add", "."], ["commit", "-q", "-m", "tiny"]):
            subprocess.run([*git, *cmd], cwd=proj, check=True, capture_output=True)
        cls.exe = cls.root / "build" / "bin" / MAIN_EXE.name

    @classmethod
    def tearDownClass(cls):
        if getattr(cls, "root", None) and cls.root.exists():
            rm_tree(cls.root)

    def mcp_log(self):
        p = self.root / "scratch" / "logs" / "mcp.log"
        return p.read_text(encoding="utf-8", errors="replace") if p.exists() else ""

    def cli(self, base_url, *extra, key=FAKE_KEY, dotenv=None, prompt="do it", env_extra=None):
        env = {k: v for k, v in os.environ.items() if k != "OPENAI_API_KEY"}
        if key is not None:
            env["OPENAI_API_KEY"] = key
        env.update(env_extra or {})
        env_file = self.root / ".env"
        env_file.unlink(missing_ok=True)
        if dotenv is not None:
            env_file.write_text(dotenv, encoding="utf-8")
        try:
            return subprocess.run(
                [sys.executable, "-I", str(CLIENT), "--root", str(self.root), "--base-url", base_url, "--backoff", "0", "--prompt", prompt, *extra],
                capture_output=True, text=True, encoding="utf-8", env=env, timeout=300,
            )
        finally:
            env_file.unlink(missing_ok=True)

    def calls_in_log(self, since=0):
        return [l for l in self.mcp_log().splitlines()[since:] if "tools/call" in l]


class EndToEnd(RootCase):
    def test_status_then_check_are_forwarded_and_final_answer_printed(self):
        before = len(self.mcp_log().splitlines())
        script = [tool_calls(("status", {}), ("check", {"project": "tiny"})), final("All good: check passed.", usage=(30, 4))]
        with FakeOpenAI(script) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("[tool call] status", r.stdout)
        self.assertIn("[tool call] check", r.stdout)
        self.assertIn("exit code: 0", r.stdout)
        self.assertIn("All good: check passed.", r.stdout)
        self.assertIn("turns=2 tool_calls=2", r.stdout)
        self.assertIn("prompt_tokens=50 completion_tokens=11 total_tokens=61", r.stdout)
        log = self.mcp_log().splitlines()[before:]
        self.assertTrue(any("tools/call status -> exit 0" in l for l in log), log)
        self.assertTrue(any("tools/call check -> exit 0" in l for l in log), log)
        # the second request carries both tool results, and the real key was what the API received
        msgs = fake.tool_messages(1)
        self.assertEqual(len(msgs), 2)
        self.assertIn("exit code: 0", msgs[1]["content"])
        self.assertEqual(fake.auth[0], "Bearer " + FAKE_KEY)
        self.assertTrue(fake.requests[0]["path"].endswith("/chat/completions"))
        self.assertEqual(fake.requests[0]["json"]["model"], "gpt-4.1-mini")
        self.assertEqual([m["role"] for m in fake.requests[0]["json"]["messages"]], ["system", "user"])
        self.assertTrue(self.mcp_log().rstrip().endswith("stopped"), "the server must have been closed and have said so")

    def test_model_and_tools_offered(self):
        with FakeOpenAI([final("hi")]) as fake:
            r = self.cli(fake.url, "--model", "gpt-test-1")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        req = fake.requests[0]["json"]
        self.assertEqual(req["model"], "gpt-test-1")
        names = sorted(t["function"]["name"] for t in req["tools"])
        self.assertEqual(names, ["build", "check", "jobs", "projects", "status", "test", "tools"])
        self.assertNotIn("watch", names)
        self.assertIn("model=gpt-test-1", r.stdout)

    def test_disallowed_tool_is_refused_locally_and_never_reaches_the_server(self):
        before = len(self.mcp_log().splitlines())
        script = [tool_calls(("run", {"project": "tiny", "command": ["cargo", "--version"]}), ("watch", {}), ("install", {})), final("done")]
        with FakeOpenAI(script) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.calls_in_log(before), [], "no tools/call may have reached the server")
        msgs = fake.tool_messages(1)
        self.assertEqual(len(msgs), 3)
        for m in msgs:
            self.assertIn("not allowed", m["content"])
        self.assertIn("(refused 3)", r.stdout)

    def test_allow_list_can_widen_but_watch_stays_out(self):
        script = [tool_calls(("watch", {}), ("status", {})), final("ok")]
        with FakeOpenAI(script) as fake:
            r = self.cli(fake.url, "--allow", "status, run, watch")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        names = sorted(t["function"]["name"] for t in fake.requests[0]["json"]["tools"])
        self.assertEqual(names, ["run", "status"])
        self.assertIn("watch", r.stderr)
        self.assertIn("mcp_callable", r.stderr)
        self.assertIn("not allowed", fake.tool_messages(1)[0]["content"])

    def test_unknown_name_in_allow_exits_2_before_any_api_call(self):
        with FakeOpenAI([final("x")]) as fake:
            r = self.cli(fake.url, "--allow", "status,nosuch")
        self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
        self.assertIn("nosuch", r.stderr)
        self.assertEqual(fake.requests, [])

    def test_key_and_secret_env_vars_never_reach_the_server_or_its_tools(self):
        # the reviewer's case: the model asks `run` to echo the key
        cmds = [("run", {"project": "tiny", "command": ["echo", "%OPENAI_API_KEY%"]}), ("run", {"project": "tiny", "command": ["echo", "%MY_SERVICE_TOKEN%"]})]
        with FakeOpenAI([tool_calls(*cmds), final("done")]) as fake:
            os.environ["MY_SERVICE_TOKEN"] = "tok-SECRETTOKEN999"
            try:
                log = self.root / "scratch" / "m3-env-log.json"
                r = self.cli(fake.url, "--allow", "run", "--json-log", str(log))
            finally:
                os.environ.pop("MY_SERVICE_TOKEN", None)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        secrets = [FAKE_KEY.encode(), b"SECRETTOKEN999"]
        blob = json.dumps(fake.requests).encode() + (r.stdout + r.stderr).encode()
        for sec in secrets:
            self.assertNotIn(sec, blob)
        # the tool really ran, and saw the variables as unset (cmd echoes the unexpanded text)
        self.assertIn("%OPENAI_API_KEY%", fake.tool_messages(1)[0]["content"])
        self.assertIn("%MY_SERVICE_TOKEN%", fake.tool_messages(1)[1]["content"])
        hits = []
        for f in self.root.rglob("*"):
            if f.is_file() and f.stat().st_size < 50_000_000:
                data = f.read_bytes()
                hits += [str(f) for sec in secrets if sec in data]
        self.assertEqual(hits, [], "the secrets must not be in any file under the root")

    def test_variables_holding_the_key_under_any_name_are_not_passed_on(self):
        cmds = [("run", {"project": "tiny", "command": ["echo", f"%{n}%"]}) for n in ("OPENAI_KEY", "MYVAR")]
        with FakeOpenAI([tool_calls(*cmds), final("done")]) as fake:
            r = self.cli(fake.url, "--allow", "run", env_extra={"OPENAI_KEY": FAKE_KEY, "MYVAR": "prefix-" + FAKE_KEY})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("%OPENAI_KEY%", fake.tool_messages(1)[0]["content"])
        self.assertIn("%MYVAR%", fake.tool_messages(1)[1]["content"])
        hits = [str(f) for f in self.root.rglob("*") if f.is_file() and b"SECRET123" in f.read_bytes()]
        self.assertEqual(hits, [], "the key must not be in any file under the root")

    def test_tool_result_text_is_redacted_before_the_model_sees_it(self):
        with FakeOpenAI([tool_calls(("run", {"project": "tiny", "command": ["echo", "sk-leaked123456"]})), final("x")]) as fake:
            r = self.cli(fake.url, "--allow", "run")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("sk-leaked123456", fake.tool_messages(1)[0]["content"])
        self.assertIn("[REDACTED]", fake.tool_messages(1)[0]["content"])

    def test_redirect_is_not_followed_and_is_an_error(self):
        with FakeOpenAI([final("must not be reached")]) as target:
            with FakeOpenAI([{"status": 302, "headers": {"Location": target.url + "/chat/completions"}, "body": {}}]) as first:
                r = self.cli(first.url)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("redirect", r.stderr)
        self.assertEqual(target.requests, [])
        self.assertEqual(target.auth, [])
        self.assertNotIn("Traceback", r.stderr)

    def test_non_string_arguments_are_a_tool_error_not_a_traceback(self):
        for bad in ({"x": 1}, [1, 2], 5):
            resp = tool_calls(("status", {}))
            resp["body"]["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = bad
            with FakeOpenAI([resp, final("fine")]) as fake:
                r = self.cli(fake.url)
            self.assertEqual(r.returncode, 0, (bad, r.stdout + r.stderr))
            self.assertNotIn("Traceback", r.stderr)
            c = fake.tool_messages(1)[0]["content"]
            self.assertTrue("tool error" in c or "unknown argument" in c, c)

    def test_assistant_messages_are_sent_back_with_only_known_fields(self):
        resp = tool_calls(("status", {}))
        resp["body"]["choices"][0]["message"].update({"refusal": None, "annotations": [], "audio": None})
        resp["body"]["choices"][0]["message"]["tool_calls"][0]["index"] = 0
        with FakeOpenAI([resp, final("ok")]) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        am = [m for m in fake.requests[1]["json"]["messages"] if m["role"] == "assistant"][0]
        self.assertEqual(sorted(am), ["content", "role", "tool_calls"])
        self.assertEqual(sorted(am["tool_calls"][0]), ["function", "id", "type"])
        self.assertEqual(sorted(am["tool_calls"][0]["function"]), ["arguments", "name"])

    def test_malformed_tool_arguments_become_a_tool_error(self):
        bad = tool_calls(("status", {}))
        bad["body"]["choices"][0]["message"]["tool_calls"][0]["function"]["arguments"] = "{not json"
        with FakeOpenAI([bad, final("recovered")]) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("not a valid JSON object", fake.tool_messages(1)[0]["content"])

    def test_max_turns_stops_the_loop(self):
        with FakeOpenAI([tool_calls(("status", {}))]) as fake:  # the last scripted step repeats forever
            r = self.cli(fake.url, "--max-turns", "2")
        self.assertEqual(r.returncode, 3, r.stdout + r.stderr)
        self.assertEqual(len(fake.requests), 2)
        self.assertIn("--max-turns 2", r.stderr)
        self.assertIn("turns=2", r.stdout)

    def test_long_tool_result_is_cut_to_its_tail_and_says_so(self):
        with FakeOpenAI([tool_calls(("tools", {})), final("ok")]) as fake:
            r = self.cli(fake.url, "--max-tool-chars", "80")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        content = fake.tool_messages(1)[0]["content"]
        self.assertTrue(content.startswith("[tool result truncated: only the last 80 of "), content[:100])
        self.assertTrue(content.endswith("exit code: 0"))
        self.assertLessEqual(len(content.split("\n", 1)[1]), 80)

    def test_key_from_dotenv_with_space_and_quotes_is_used_and_never_leaked(self):
        # the model echoes the key, a 401-style body would too: nothing printed or logged may contain it
        script = [tool_calls(("status", {})), final(f"my key is {FAKE_KEY}")]
        log = self.root / "scratch" / "m3-log.json"
        with FakeOpenAI(script) as fake:
            r = self.cli(fake.url, "--json-log", str(log), key=None, dotenv=f'# c\nOTHER=1\nOPENAI_API_KEY= "{FAKE_KEY}" \n')
            self.assertEqual(fake.auth[0], "Bearer " + FAKE_KEY)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("SECRET123", r.stdout + r.stderr)
        self.assertIn("[REDACTED]", r.stdout)
        text = log.read_text(encoding="utf-8")
        self.assertNotIn("SECRET123", text)
        doc = json.loads(text)
        self.assertEqual(doc["turns"], 2)
        self.assertEqual(doc["usage"]["total_tokens"], 27 + 15)
        self.assertEqual(doc["events"][1]["name"], "status")
        self.assertEqual(doc["events"][1]["exit_code"], 0)
        self.assertIn("messages", doc)

    def test_json_log_outside_scratch_or_evidence_is_refused(self):
        with FakeOpenAI([final("x")]) as fake:
            r = self.cli(fake.url, "--json-log", str(MAIN_ROOT / "m3-outside-log.json"))
            self.assertEqual(r.returncode, 2, r.stdout + r.stderr)
            self.assertIn("scratch", r.stderr)
            r = self.cli(fake.url, "--json-log", str(self.root / "projects" / "tiny" / "x.json"))
            self.assertEqual(r.returncode, 2)
            r = self.cli(fake.url, "--json-log", str(self.root / "scratch" / ".." / "x.json"))
            self.assertEqual(r.returncode, 2)
            self.assertEqual(fake.requests, [])

    def test_http_401_is_a_clear_error_with_no_key_and_no_retry(self):
        with FakeOpenAI([http_error(401, f"Incorrect API key provided: {FAKE_KEY}.")]) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 1)
        self.assertIn("HTTP 401", r.stderr)
        self.assertNotIn("Traceback", r.stderr)
        self.assertNotIn("SECRET123", r.stdout + r.stderr)
        self.assertEqual(len(fake.requests), 1)
        self.assertTrue(self.mcp_log().rstrip().endswith("stopped"), "server closed on the error path too")

    def test_http_403(self):
        with FakeOpenAI([http_error(403, "Project does not have access to model x")]) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 1)
        self.assertIn("HTTP 403", r.stderr)
        self.assertIn("does not have access", r.stderr)
        self.assertEqual(len(fake.requests), 1)

    def test_http_429_is_retried_and_can_succeed(self):
        script = [http_error(429, "slow down", {"Retry-After": "0"}), http_error(429, "slow down", {"Retry-After": "0"}), final("finally")]
        with FakeOpenAI(script) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(len(fake.requests), 3)
        self.assertIn("finally", r.stdout)
        self.assertIn("retrying", r.stderr)

    def test_http_429_gives_up_after_two_retries(self):
        with FakeOpenAI([http_error(429, "quota", {"Retry-After": "0"})]) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 1)
        self.assertEqual(len(fake.requests), 3)
        self.assertIn("HTTP 429", r.stderr)
        self.assertNotIn("Traceback", r.stderr)

    def test_http_500_is_retried_then_reported(self):
        with FakeOpenAI([http_error(500, "boom")]) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 1)
        self.assertEqual(len(fake.requests), 3)
        self.assertIn("HTTP 500", r.stderr)
        self.assertNotIn("Traceback", r.stderr)

    def test_bad_model_name_shows_the_api_message(self):
        with FakeOpenAI([http_error(404, "The model `nope-1` does not exist or you do not have access to it.")]) as fake:
            r = self.cli(fake.url, "--model", "nope-1")
        self.assertEqual(r.returncode, 1)
        self.assertIn("HTTP 404", r.stderr)
        self.assertIn("`nope-1` does not exist", r.stderr)
        self.assertIn("--model", r.stderr)

    def test_malformed_json_from_the_api(self):
        with FakeOpenAI([{"status": 200, "body": "not json at all"}]) as fake:
            r = self.cli(fake.url)
        self.assertEqual(r.returncode, 1)
        self.assertNotIn("Traceback", r.stderr)
        self.assertIn("not valid JSON", r.stderr)

    def test_missing_key_says_how_to_add_it(self):
        with FakeOpenAI([final("x")]) as fake:
            r = self.cli(fake.url, key=None)
        self.assertEqual(r.returncode, 1)
        self.assertIn("OPENAI_API_KEY", r.stderr)
        self.assertIn(".env", r.stderr)
        self.assertEqual(fake.requests, [])
        self.assertNotIn("Traceback", r.stderr)

    def test_api_timeout_is_a_clear_error(self):
        # a server that never answers: accept and sleep
        import socket
        s = socket.socket()
        s.bind(("127.0.0.1", 0))
        s.listen(5)
        try:
            r = self.cli(f"http://127.0.0.1:{s.getsockname()[1]}/v1", "--timeout", "1")
        finally:
            s.close()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("--timeout", r.stderr)
        self.assertNotIn("Traceback", r.stderr)

    def test_plain_http_to_a_remote_host_is_refused(self):
        r = self.cli("http://example.com/v1")
        self.assertEqual(r.returncode, 2)
        self.assertIn("https", r.stderr)


class ServerLifecycle(RootCase):
    def test_server_process_is_gone_after_close(self):
        s = oc.McpServer(self.exe, self.root, 30)
        tools = s.start()
        self.assertIn("status", [t["name"] for t in tools])
        self.assertIsNone(s.proc.poll())
        s.close()
        self.assertIsNotNone(s.proc.poll())
        s.close()  # twice is fine

    def test_close_after_a_failed_start_and_kill_path(self):
        s = oc.McpServer(self.exe, self.root, 30)
        s.close()
        self.assertIsNotNone(s.proc.poll())

    def test_missing_server_binary_is_a_clear_error(self):
        with self.assertRaises(oc.Fail) as cm:
            oc.McpServer(self.root / "nope.exe", self.root, 5)
        self.assertIn("cannot start the Broadsword server", str(cm.exception))

    def test_default_tools_come_from_read_only_hints_and_watch_is_never_offered(self):
        with oc.McpServer(self.exe, self.root, 30) as s:
            tools = s.start()
        self.assertEqual(sorted(oc.default_allow(tools)), ["build", "check", "jobs", "projects", "status", "test", "tools"])
        for n in ("run", "recover", "abandon", "cleanup", "resume", "install", "watch"):
            self.assertNotIn(n, oc.default_allow(tools) if n != "watch" else [t["name"] for t in oc.select_tools(tools, None)[0]])
        offered, warnings = oc.select_tools(tools, ["watch", "status"])
        self.assertEqual([t["name"] for t in offered], ["status"])
        self.assertTrue(any("watch" in w for w in warnings))

    def test_every_real_tool_converts_to_an_openai_function(self):
        with oc.McpServer(self.exe, self.root, 30) as s:
            tools = s.start()
        for t in tools:
            f = oc.to_openai_tool(t)
            self.assertEqual(f["type"], "function")
            self.assertEqual(f["function"]["name"], t["name"])
            self.assertEqual(f["function"]["parameters"]["type"], "object")
            blob = json.dumps(f)
            for bad in ("minItems", "maxItems", "$schema"):
                self.assertNotIn(bad, blob)
            self.assertNotIn('"required": []', blob)
        run = next(oc.to_openai_tool(t) for t in tools if t["name"] == "run")
        self.assertEqual(run["function"]["parameters"]["required"], ["project", "command"])
        self.assertIn("at least 1 item", run["function"]["parameters"]["properties"]["command"]["description"])


class Units(unittest.TestCase):
    def test_schema_conversion(self):
        src = {"$schema": "x", "type": "object", "properties": {"a": {"type": "array", "items": {"type": "string"}, "minItems": 1, "description": "list"}, "b": {"type": "string"}}, "required": [], "additionalProperties": False}
        got = oc.convert_schema(src)
        self.assertEqual(got, {"type": "object", "properties": {"a": {"type": "array", "items": {"type": "string"}, "description": "list (at least 1 item.)"}, "b": {"type": "string"}}, "additionalProperties": False})
        self.assertIn("$schema", src, "the input is not modified")
        tool = {"name": "t", "description": "d", "inputSchema": {"type": "object", "properties": {}, "required": []}}
        self.assertEqual(oc.to_openai_tool(tool), {"type": "function", "function": {"name": "t", "description": "d", "parameters": {"type": "object", "properties": {}}}})

    def test_env_parsing(self):
        self.assertEqual(oc.parse_env_text('A= 1 \nB="two"\n\'C\'=x\n# n\nexport D=\'d d\'\nE=\n  F =  "q" \n'), {"A": "1", "B": "two", "'C'": "x", "D": "d d", "E": "", "F": "q"})

    def test_key_loading_env_wins_over_dotenv(self):
        with scratch_tmp() as d:
            root = Path(d)
            (root / ".env").write_text("OPENAI_API_KEY = 'sk-from-file-1'\n", encoding="utf-8")
            self.assertEqual(oc.load_api_key(root, {"OPENAI_API_KEY": " sk-from-env-2 "}), "sk-from-env-2")
            self.assertEqual(oc.load_api_key(root, {}), "sk-from-file-1")
            self.assertEqual(oc.load_api_key(root, {"OPENAI_API_KEY": "  "}), "sk-from-file-1")

    def test_missing_key_is_a_clear_error(self):
        with scratch_tmp() as d:
            with self.assertRaises(oc.Fail) as cm:
                oc.load_api_key(Path(d), {})
            self.assertIn("OPENAI_API_KEY", str(cm.exception))
            self.assertIn(".env", str(cm.exception))
            (Path(d) / ".env").write_text("OTHER=1\n", encoding="utf-8")
            with self.assertRaises(oc.Fail):
                oc.load_api_key(Path(d), {})
            with self.assertRaises(oc.Fail):
                oc.load_api_key(Path(d), {"OPENAI_API_KEY": "sk-bad key"})

    def test_redact(self):
        oc._SECRETS.add("plainsecretvalue")
        self.assertEqual(oc.redact("a sk-proj-abc123_X-9 b plainsecretvalue c"), "a [REDACTED] b [REDACTED] c")
        self.assertEqual(oc.redact("Incorrect API key: sk-test********1234."), "Incorrect API key: [REDACTED].")
        self.assertEqual(oc.redact("no secrets here"), "no secrets here")

    def test_api_message_never_returns_a_raw_body(self):
        self.assertEqual(oc.api_message(b'{"error":{"message":"bad sk-abcdef123"}}'), "bad [REDACTED]")
        self.assertNotIn("sk-", oc.api_message(b"<html>sk-abcdef123</html>"))

    def test_truncate_and_preview(self):
        t = "".join(f"line{i}\n" for i in range(100))
        c = oc.truncate_tail(t, 50)
        self.assertTrue(c.startswith("[tool result truncated: only the last 50 of 690 characters are shown]\n"))
        self.assertTrue(c.endswith("line99\n"))
        self.assertEqual(oc.truncate_tail("short", 50), "short")
        p = oc.preview(t)
        self.assertIn("line0", p)
        self.assertIn("line99", p)
        self.assertIn("90 lines omitted", p)
        self.assertNotIn("line50", p)

    def test_child_env_drops_by_value_and_by_key_suffix(self):
        oc._SECRETS.add("valuesecret-xyz")
        env = {"MYVAR": "a valuesecret-xyz b", "OPENAI_KEY": "q", "PATH": "p"}
        self.assertEqual(oc.child_env(env), {"PATH": "p"})

    def test_child_env_drops_key_token_and_secret_variables(self):
        env = {"OPENAI_API_KEY": "a", "openai_api_key": "b", "ANTHROPIC_API_KEY": "c", "GH_TOKEN": "d", "x_secret": "e", "PATH": "p", "HOME": "h", "KEYBOARD": "k"}
        self.assertEqual(oc.child_env(env), {"PATH": "p", "HOME": "h", "KEYBOARD": "k"})

    def test_retry_delay_respects_retry_after(self):
        self.assertEqual(oc.retry_delay({"Retry-After": "7"}, 0, 1.0), 7.0)
        self.assertEqual(oc.retry_delay({"Retry-After": "9999"}, 0, 1.0), 60.0)
        self.assertEqual(oc.retry_delay({}, 1, 1.0), 2.0)
        self.assertEqual(oc.retry_delay({"Retry-After": "Wed, 21 Oct"}, 0, 1.0), 1.0)

    def test_main_root_follows_a_worktree_git_file(self):
        with scratch_tmp() as d:
            main, wt = Path(d) / "main", Path(d) / "main" / ".claude" / "worktrees" / "a"
            (main / ".git" / "worktrees" / "a").mkdir(parents=True)
            wt.mkdir(parents=True)
            (main / "broadsword.toml").write_text("root = \".\"\n", encoding="utf-8")
            (wt / "broadsword.toml").write_text("root = \".\"\n", encoding="utf-8")
            (wt / ".git").write_text(f"gitdir: {main / '.git' / 'worktrees' / 'a'}\n", encoding="utf-8")
            (wt / "sub").mkdir()
            self.assertEqual(oc.find_main_root(wt / "sub"), main.resolve())
            self.assertEqual(oc.find_main_root(main), main.resolve())

    def test_server_lines_that_are_not_responses_are_skipped(self):
        # a fake "server": a python one-liner that prints noise, a notification and a wrong-id response before the real one
        code = (
            "import sys, json\n"
            "for line in sys.stdin:\n"
            "    m = json.loads(line)\n"
            "    if 'id' not in m: continue\n"
            "    sys.stderr.write('x' * 100000 + '\\n'); sys.stderr.flush()\n"
            "    print('garbage not json'); print('{\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\"}')\n"
            "    print(json.dumps({'jsonrpc':'2.0','id':999,'result':{}}))\n"
            "    print(json.dumps({'jsonrpc':'2.0','id':m['id'],'result':{'tools':[{'name':'zzz'}]}}), flush=True)\n"
        )
        with scratch_tmp() as d:
            script = Path(d) / "fake.py"
            script.write_text(code, encoding="utf-8")
            bat = Path(d) / ("fake.cmd" if os.name == "nt" else "fake.sh")
            bat.write_text((f'@"{sys.executable}" "{script}"\r\n' if os.name == "nt" else f'#!/bin/sh\nexec "{sys.executable}" "{script}"\n'), encoding="utf-8")
            bat.chmod(0o755)
            with oc.McpServer(bat, Path(d), 10) as s:
                self.assertEqual([t["name"] for t in s.start()], ["zzz"])  # also proves 100 KB of stderr per request does not block it


if __name__ == "__main__":
    unittest.main()
