"""Tests for openai_provider.py and run.py's openai and codex branches. No real network, no real codex: a fake server on
127.0.0.1 in a thread, and a fake `codex` (a small Python script) first on a PATH given to the code under test.

Run (from the repo root):
  env -u OPENAI_API_KEY python -I -B -m unittest discover -s bench/coder -p "test_openai_provider.py" -v

The key is fake and every call passes --root to a throwaway folder, so the real root .env is never read.
The end-to-end tests run run.py with answers that have no code block, so nothing is graded (no cargo is started).
"""
import sys

sys.dont_write_bytecode = True

import contextlib, importlib.util, io, json, os, pathlib, stat, subprocess, tempfile, threading, types, unittest, unittest.mock, shutil
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
        self.script, self.requests, self.auth, self.paths = list(script), [], [], []
        outer = self

        class H(BaseHTTPRequestHandler):
            def do_POST(self):
                n = int(self.headers.get("Content-Length", 0))
                outer.requests.append(json.loads(self.rfile.read(n)))
                outer.auth.append(self.headers.get("Authorization"))
                outer.paths.append(self.path)
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


def resp_body(text=CODE, status="completed", model="gpt-resp-2026-01-01", details=None):
    return {"model": model, "status": status, "incomplete_details": details,
            "output": [{"type": "reasoning", "summary": []},
                       {"type": "message", "content": [{"type": "output_text", "text": text}]}],
            "usage": {"input_tokens": 900, "output_tokens": 300, "input_tokens_details": {"cached_tokens": 32}}}


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
        catalog = open(argv[i + 1].split("=", 1)[1].strip("'\""), encoding="utf-8").read()
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
events.append({"type": "turn.completed", "usage": {"input_tokens": 1234, "output_tokens": 56, "cached_input_tokens": 10}})
open(out, "w", encoding="utf-8").write(os.environ.get("FAKE_CODEX_TEXT", ""))
for e in events:
    print(json.dumps(e))
'''


def make_fake_codex(folder):
    """A fake codex executable in `folder`; returns the folder (to put first on PATH)."""
    folder.mkdir(parents=True, exist_ok=True)
    (folder / "fake_codex.py").write_text(FAKE_CODEX, encoding="utf-8")
    if os.name == "nt":
        (folder / "codex.cmd").write_text(f'@"{sys.executable}" "%~dp0fake_codex.py" %*\r\n', encoding="utf-8")
    else:
        w = folder / "codex"
        w.write_text(f'#!/bin/sh\nexec "{sys.executable}" "$(dirname "$0")/fake_codex.py" "$@"\n', encoding="utf-8")
        w.chmod(w.stat().st_mode | stat.S_IEXEC)
    return folder


@contextlib.contextmanager
def codex_gates(tmp):
    """review_codex() sits behind the GPT spend cap and the canary gate, which read <root>/broadsword.toml, a canary record for the
    codex version and a ledger folder. Point them at a throwaway root so a test never reads or writes the real state/ (the real
    ledger may already be at the daily cap). Same seam as bench/reviewer/test_run_codex.py."""
    root = tmp / "gate-root"
    (root / "state" / "canary").mkdir(parents=True, exist_ok=True)
    (root / "broadsword.toml").write_text('state_dir = "state"\n', encoding="utf-8")
    # the canary record is read from <main root>/<state_dir>/canary/last-pass.json (canary_state_file), the root find_main_root returns;
    # it names the fake codex's version, so the gate passes without running scripts/shadow_canary.py
    (root / "state" / "canary" / "last-pass.json").write_text(json.dumps({"codex_version": "codex-cli 9.9.9-test"}), encoding="utf-8")
    (tmp / "clients").mkdir(exist_ok=True)
    review = op._review()
    old_here, old_root = review.HERE, review.oc.find_main_root
    review.HERE = tmp / "clients"
    review.oc.find_main_root = lambda start: root
    try:
        yield root
    finally:
        review.HERE, review.oc.find_main_root = old_here, old_root


MODELS = {"models": {"gpt-fake": {"api": "chat", "max_output_tokens": 20000, "service_tiers": ["default", "fast"]},
                     "gpt-resp": {"api": "responses", "max_output_tokens": 50000, "service_tiers": ["default"]}}}


class Provider(unittest.TestCase):
    def setUp(self):
        self.tmp = pathlib.Path(tempfile.mkdtemp(prefix="bench-coder-openai-test-"))
        (self.tmp / "root").mkdir()
        (self.tmp / "models.json").write_text(json.dumps(MODELS), encoding="utf-8")
        self.old_models, op._ro().MODELS_FILE = op._ro().MODELS_FILE, self.tmp / "models.json"
        self.old_key = os.environ.pop("OPENAI_API_KEY", None)
        os.environ["OPENAI_API_KEY"] = FAKE_KEY
        op._ro().oc._SECRETS.clear()
        op._state.pop("cfg", None)

    def tearDown(self):
        op._ro().MODELS_FILE = self.old_models
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

    # ---- models.json, Responses API, caps, tiers (the same rules as bench/reviewer/run_openai.py) ----
    def test_responses_model_uses_responses_endpoint_and_shape(self):
        f = Fake([(200, resp_body())])
        self.addCleanup(f.close)
        r = self.go(f, "gpt-resp", "high")
        self.assertEqual(f.paths, ["/v1/responses"])
        req = f.requests[0]
        self.assertEqual((req["model"], req["instructions"], req["input"]), ("gpt-resp", SYSTEM, "USER PROMPT"))
        self.assertEqual(req["max_output_tokens"], 16000)
        self.assertEqual(req["reasoning"], {"effort": "high"})
        for k in ("messages", "max_completion_tokens", "reasoning_effort"):
            self.assertNotIn(k, req)
        self.assertEqual((r["answer"], r["exit"], r["is_error"], r["backend"]), (CODE, 0, False, "openai-responses"))
        self.assertEqual(r["model_used"], ["gpt-resp-2026-01-01"])
        self.assertTrue(r["model_reported"])
        self.assertEqual(r["tokens"], {"input": 900, "output": 300, "cache_read": 32, "cache_write": None})

    def test_responses_default_effort_omits_reasoning(self):
        f = Fake([(200, resp_body())])
        self.addCleanup(f.close)
        self.go(f, "gpt-resp", "default")
        self.assertEqual(f.paths, ["/v1/responses"])
        self.assertNotIn("reasoning", f.requests[0])

    def test_responses_incomplete_and_chat_shaped_bodies_are_errors(self):
        f = Fake([(200, resp_body(text="", status="incomplete", details={"reason": "max_output_tokens"}))])
        self.addCleanup(f.close)
        r = self.go(f, "gpt-resp")
        self.assertTrue(r["is_error"])
        self.assertEqual((r["exit"], r["answer"]), (1, ""))
        self.assertIn("max_output_tokens", r["error"])
        g = Fake([(200, body())])  # a Chat Completions body from the Responses endpoint
        self.addCleanup(g.close)
        r = self.go(g, "gpt-resp")
        self.assertTrue(r["is_error"])
        self.assertIn("Responses API", r["error"])

    def test_chat_result_names_its_backend_and_model(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        r = self.go(f)
        self.assertEqual((r["backend"], r["model_reported"], f.paths), ("openai-chat", True, ["/v1/chat/completions"]))

    def test_max_tokens_lowered_to_the_model_cap(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        se = io.StringIO()
        with contextlib.redirect_stderr(se):
            op.prepare("gpt-fake", "high", ["--base-url", f.url, "--root", str(self.tmp / "root"), "--max-tokens", "32000"], 600)
        op.call(SYSTEM, "p", "gpt-fake", "high")
        self.assertEqual(f.requests[0]["max_completion_tokens"], 20000)
        self.assertIn("lowered to 20000", se.getvalue())

    def test_model_not_in_the_table_is_refused_before_any_request(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as cm:
            op.prepare("gpt-unlisted", "high", ["--base-url", f.url, "--root", str(self.tmp / "root")], 600)
        self.assertIn("not in models.json", str(cm.exception))
        self.assertEqual(f.requests, [])

    def test_service_tier_sent_only_when_the_model_accepts_it(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        self.go(f, "gpt-fake", "high", "--service-tier", "fast")
        self.assertEqual(f.requests[0]["service_tier"], "fast")
        self.go(f, "gpt-fake", "high")
        self.assertNotIn("service_tier", f.requests[1])
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as cm:
            self.go(f, "gpt-fake", "high", "--service-tier", "flex")
        self.assertIn("does not accept service tier", str(cm.exception))
        self.assertEqual(len(f.requests), 2)

    # ---- the codex provider (fake codex, no network) ----
    def codex_env(self, **extra):
        log = self.tmp / "codex-log.json"
        env = {**os.environ, "PATH": str(make_fake_codex(self.tmp / "bin")) + os.pathsep + os.environ.get("PATH", ""),
               "FAKE_CODEX_LOG": str(log), "FAKE_CODEX_TEXT": CODE, **extra}
        return env, log

    def codex_go(self, model="gpt-codex-fake", effort="default", **extra):
        env, log = self.codex_env(**extra)
        with unittest.mock.patch.dict(os.environ, env, clear=True), codex_gates(self.tmp) as root:
            op.prepare_codex(model, effort, 120)
            r = op.call_codex(SYSTEM, "USER PROMPT", model, effort)
        self.gate_root = root
        return r, log

    def test_codex_model_named_in_an_event_is_recorded_as_reported(self):
        r, _ = self.codex_go(FAKE_CODEX_MODE="modelnamed")
        self.assertEqual((r["is_error"], r["model_used"], r["model_reported"]), (False, ["gpt-answered-by"], True))
        self.assertEqual(json.loads(r["raw"])["model_used"], ["gpt-answered-by"])

    def test_codex_gate_refusal_is_a_failed_call_that_never_claims_a_model(self):
        # The gate that can still refuse a codex call is the canary: the daily cap never refuses codex (plan calls are free).
        # No record for the fake codex's version and no scripts/shadow_canary.py beside the temp clients/ folder: canary_failed.
        env, log = self.codex_env()
        with unittest.mock.patch.dict(os.environ, env, clear=True), codex_gates(self.tmp) as root:
            (root / "state" / "canary" / "last-pass.json").unlink()
            op.prepare_codex("gpt-codex-fake", "default", 120)
            r = op.call_codex(SYSTEM, "USER PROMPT", "gpt-codex-fake", "default")
        self.assertTrue(r["is_error"])
        self.assertIn("canary_failed", r["error"])
        self.assertEqual((r["answer"], r["model_used"], r["model_reported"]), ("", [], False))
        self.assertFalse(log.exists())  # codex was never started (only `codex --version`, which the fake does not log)

    def test_codex_is_never_refused_for_the_daily_cap(self):
        env, log = self.codex_env()
        with unittest.mock.patch.dict(os.environ, env, clear=True), codex_gates(self.tmp) as root:
            (root / "broadsword.toml").write_text('state_dir = "state"\ngpt_daily_cap_usd = 0\n', encoding="utf-8")
            op.prepare_codex("gpt-codex-fake", "default", 120)
            r = op.call_codex(SYSTEM, "USER PROMPT", "gpt-codex-fake", "default")
        self.assertEqual((r["is_error"], r["answer"]), (False, CODE))
        self.assertTrue(log.exists())

    def test_codex_result_never_claims_a_model(self):
        r, log = self.codex_go()
        self.assertEqual((r["answer"], r["exit"], r["is_error"], r["error"], r["cost_usd"]), (CODE, 0, False, None, None))
        self.assertEqual((r["model_used"], r["model_reported"], r["backend"]), ([], False, "codex"))
        self.assertEqual(r["tokens"], {"input": 1234, "output": 56, "cache_read": 10, "cache_write": None})
        self.assertEqual(json.loads(r["raw"])["backend"], "codex")

    def test_codex_command_is_exactly_the_shared_one_and_gets_both_prompts(self):
        r, log = self.codex_go()
        rec = json.loads(log.read_text(encoding="utf-8"))
        want = op._review().codex_command("codex", "gpt-codex-fake", self.tmp, self.tmp / "answer.txt")
        norm = lambda c: [x for x in c if "no-tools-catalog" not in x and "answer.txt" not in x and "bsw-codex-" not in x]
        self.assertEqual(norm(rec["argv"]), norm(want[1:]))
        self.assertEqual(len(json.loads(rec["catalog"])["models"]), 1)
        self.assertIn(SYSTEM, rec["stdin"])
        self.assertIn("USER PROMPT", rec["stdin"])

    def test_codex_api_key_never_reaches_codex(self):
        r, log = self.codex_go(OPENAI_API_KEY=FAKE_KEY, COPY_OF_THE_KEY=FAKE_KEY)
        rec = json.loads(log.read_text(encoding="utf-8"))
        self.assertNotIn("OPENAI_API_KEY", [n.upper() for n in rec["env_names"]])
        self.assertNotIn(FAKE_KEY, rec["env_values"])

    def test_codex_tool_item_and_failure_are_failed_calls(self):
        r, _ = self.codex_go(FAKE_CODEX_MODE="tool")
        self.assertTrue(r["is_error"])
        self.assertEqual((r["answer"], r["exit"]), ("", 1))
        self.assertIn("command_execution", r["error"])
        r, _ = self.codex_go(FAKE_CODEX_MODE="fail")
        self.assertTrue(r["is_error"])
        self.assertIn("code 1", r["error"])
        self.assertFalse(r["model_reported"])

    def test_codex_key_in_the_answer_is_redacted_in_raw(self):
        r, _ = self.codex_go(FAKE_CODEX_TEXT=f"{CODE}\nleak {FAKE_KEY}", OPENAI_API_KEY=FAKE_KEY)
        self.assertNotIn(FAKE_KEY, r["raw"].decode())
        self.assertNotIn(FAKE_KEY, r["answer"])

    def test_codex_effort_and_missing_codex_refused_before_any_call(self):
        env, log = self.codex_env()
        with unittest.mock.patch.dict(os.environ, env, clear=True):
            with self.assertRaises(SystemExit) as cm:
                op.prepare_codex("gpt-codex-fake", "high", 120)
        self.assertIn("effort 'default'", str(cm.exception))
        self.assertFalse(log.exists())
        with unittest.mock.patch.dict(os.environ, {"PATH": str(self.tmp / "empty")}, clear=True):
            with self.assertRaises(SystemExit) as cm:
                op.prepare_codex("gpt-codex-fake", "default", 120)
        self.assertIn("not found", str(cm.exception))


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
        p = subprocess.run([sys.executable, "-I", "-B", str(HERE / "run.py"), str(out), "openai", "gpt-4o", "high",
                            "01-duration", "--base-url", fake.url, "--root", str(self.tmp / "root"), "--backoff", "0", *extra],
                           capture_output=True, text=True, timeout=120, env=self.env)
        return p, out

    def test_run_py_writes_the_result_fields_without_calling_the_real_api(self):
        f = Fake([(200, body("no code block here"))])
        self.addCleanup(f.close)
        p, out = self.run_py(f, "--price-in", "2", "--price-out", "10")
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads((out / "01-duration__openai__gpt-4o__high.json").read_text(encoding="utf-8"))
        for k in ("provider", "model", "effort", "task", "tokens", "cost_usd", "wall_s", "exit", "harness_wall_s", "compiles",
                  "passed", "total", "format_failure", "first_error"):
            self.assertIn(k, rec)
        self.assertEqual((rec["provider"], rec["model"], rec["task"], rec["exit"]), ("openai", "gpt-4o", "01-duration", 0))
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
        rec = json.loads((out / "01-duration__openai__gpt-4o__high.json").read_text(encoding="utf-8"))
        self.assertIsNone(rec["cost_usd"])
        self.assertEqual(rec["exit"], 1)
        self.assertTrue(rec["call_failed"])
        self.assertFalse(rec["format_failure"])
        self.assertIn("model call failed", rec["first_error"])
        self.assertIn("finish_reason='length'", rec["first_error"])

    def test_run_py_records_model_reported_and_backend_for_openai(self):
        f = Fake([(200, body("no code block here"))])
        self.addCleanup(f.close)
        p, out = self.run_py(f)
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads((out / "01-duration__openai__gpt-4o__high.json").read_text(encoding="utf-8"))
        self.assertEqual((rec["model_reported"], rec["backend"]), (True, "openai-chat"))

    def test_run_py_refuses_a_model_missing_from_models_json_before_any_request(self):
        f = Fake([(200, body())])
        self.addCleanup(f.close)
        out = self.tmp / "out"
        p = subprocess.run([sys.executable, "-I", "-B", str(HERE / "run.py"), str(out), "openai", "gpt-not-in-the-table", "high",
                            "01-duration", "--base-url", f.url, "--root", str(self.tmp / "root")],
                           capture_output=True, text=True, timeout=120, env=self.env)
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("not in models.json", p.stderr)
        self.assertEqual(f.requests, [])

    def run_codex(self, effort="default", *extra, text="no code block here", mode="ok"):
        """run.py main() in this process (a subprocess cannot be given the gate seam), with the fake codex first on PATH.
        Returns (an object with returncode and stderr, the out folder)."""
        bin_dir = make_fake_codex(self.tmp / "bin")
        env = dict(self.env, PATH=str(bin_dir) + os.pathsep + self.env.get("PATH", ""), FAKE_CODEX_LOG=str(self.tmp / "codex-log.json"),
                   FAKE_CODEX_TEXT=text, FAKE_CODEX_MODE=mode)
        out = self.tmp / "out"
        argv = ["run.py", str(out), "codex", "gpt-codex-fake", effort, "01-duration", *extra]
        err = io.StringIO()
        code = 0
        with unittest.mock.patch.dict(os.environ, env, clear=True), unittest.mock.patch.object(sys, "argv", argv),                 contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(err), codex_gates(self.tmp):
            spec = importlib.util.spec_from_file_location("run_codex_under_test", HERE / "run.py")
            run = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(run)
            run.OP._state["review"] = op._review()  # the module with the gate seam in place
            try:
                run.main()
            except SystemExit as e:
                code = e.code if isinstance(e.code, int) else 1
                if not isinstance(e.code, int):
                    err.write(str(e.code))
        return types.SimpleNamespace(returncode=code, stderr=err.getvalue()), out

    def test_run_py_codex_result_says_the_model_was_not_reported(self):
        p, out = self.run_codex()
        self.assertEqual(p.returncode, 0, p.stderr)
        rec = json.loads((out / "01-duration__codex__gpt-codex-fake__default.json").read_text(encoding="utf-8"))
        self.assertEqual((rec["provider"], rec["model"], rec["backend"]), ("codex", "gpt-codex-fake", "codex"))
        self.assertEqual((rec["model_used"], rec["model_reported"], rec["cost_usd"], rec["exit"]), ([], False, None, 0))
        self.assertEqual(rec["tokens"]["input"], 1234)
        self.assertTrue(rec["format_failure"])  # the fake answer has no code block, so nothing is graded
        s = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(out)], capture_output=True, text=True, timeout=60)
        self.assertEqual(s.returncode, 0, s.stderr)
        self.assertIn("codex:gpt-codex-fake [model not reported]", s.stdout)

    def test_score_py_marks_only_rows_that_say_model_reported_false(self):
        out = self.tmp / "scores"
        out.mkdir()
        for provider, extra in (("openai", {"model_reported": True}), ("claude", {})):
            rec = {"provider": provider, "model": "m", "effort": "default", "task": "01-duration", "tokens": {}, "cost_usd": None,
                   "wall_s": 1.0, "compiles": True, "passed": 1, "total": 1, "graded": True, "call_failed": False,
                   "format_failure": False, **extra}
            (out / f"01-duration__{provider}__m__default.json").write_text(json.dumps(rec), encoding="utf-8")
        s = subprocess.run([sys.executable, "-I", "-B", str(HERE / "score.py"), str(out)], capture_output=True, text=True, timeout=60)
        self.assertEqual(s.returncode, 0, s.stderr)
        self.assertNotIn("not reported", s.stdout)

    def test_run_py_codex_refuses_openai_options_and_other_efforts_before_codex_starts(self):
        p, out = self.run_codex("default", "--max-tokens", "100")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("provider openai only", p.stderr)
        p, out = self.run_codex("high")
        self.assertNotEqual(p.returncode, 0)
        self.assertIn("effort 'default'", p.stderr)
        self.assertFalse((self.tmp / "codex-log.json").exists())  # codex never started
        self.assertFalse(out.exists())

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
