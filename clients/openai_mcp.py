#!/usr/bin/env python3
"""Bridge an OpenAI model to Broadsword's MCP server (Stage 2, milestone 3).

Spawns `<main root>/build/bin/broadsword.exe mcp`, offers the allowed MCP tools to a model through the
Chat Completions API, runs the tool calls the model makes through MCP, and prints what happened.

Python 3.13, standard library only (Rust's std has no TLS, so the HTTPS part lives here).

The API key comes from OPENAI_API_KEY or from `<main root>/.env`. It is never printed, logged or written:
everything that leaves this program goes through `redact()`.

Run from anywhere inside the project:
    python -I -B clients/openai_mcp.py --prompt "Run check on crash-sim and report the result."

Exit codes: 0 final answer reached; 1 error (key, network, API, server); 2 bad command line;
3 stopped at --max-turns without a final answer; 130 interrupted.
"""

from __future__ import annotations

import argparse
import json
import os
import queue
import re
import socket
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

CONFIG_FILE = "broadsword.toml"
PROTOCOL_VERSION = "2025-11-25"
CLIENT_NAME = "broadsword-openai-client"
DEFAULT_MODEL = "gpt-4.1-mini"
DEFAULT_BASE_URL = "https://api.openai.com/v1"
EXTRA_DEFAULT_TOOLS = ("check", "build", "test")

SYSTEM_PROMPT = (
    "You are connected to Broadsword, a local development tool, through a set of tools. "
    "Use them to do what the user asks. Tools run one at a time and a build or check can take a while. "
    "Each tool result ends with its exit code. In your final answer, report what the tools actually returned "
    "(exit codes, pass and fail counts, job ids). Never claim success without a tool result that shows it."
)

KEY_PATTERN = re.compile(r"sk-[A-Za-z0-9_*\-]{3,}")

# Secrets to blank out of everything printed or written, besides anything that looks like `sk-...`.
_SECRETS: set[str] = set()


class Fail(Exception):
    """A problem to report in one clear line (exit code 1)."""

    def __init__(self, message: str, code: int = 1):
        super().__init__(message)
        self.code = code


# ---- secrets ----------------------------------------------------------------------------------


def redact(text: object) -> str:
    """Blank out the API key (exact value and any `sk-...` look-alike) in `text`."""
    s = text if isinstance(text, str) else str(text)
    for secret in sorted(_SECRETS, key=len, reverse=True):
        if secret:
            s = s.replace(secret, "[REDACTED]")
    return KEY_PATTERN.sub("[REDACTED]", s)


def out(text: str = "") -> None:
    print(redact(text), flush=True)


def err(text: str) -> None:
    print(redact(text), file=sys.stderr, flush=True)


def parse_env_text(text: str) -> dict[str, str]:
    """Simple `NAME=value` lines. `#` comments, blank lines and a leading `export ` are ignored; whitespace around the
    name and the value is trimmed and one pair of matching surrounding quotes is stripped."""
    values: dict[str, str] = {}
    for raw in text.lstrip("﻿").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        name, _, value = line.partition("=")
        name = name.strip()
        if name.startswith("export "):
            name = name[len("export "):].strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        if name:
            values[name] = value
    return values


def load_api_key(root: Path, environ: dict[str, str] | None = None) -> str:
    """The environment wins; otherwise `<root>/.env`. Raises Fail, never showing a value."""
    environ = os.environ if environ is None else environ
    key = (environ.get("OPENAI_API_KEY") or "").strip()
    if not key:
        env_file = root / ".env"
        if env_file.is_file():
            try:
                key = parse_env_text(env_file.read_text(encoding="utf-8", errors="replace")).get("OPENAI_API_KEY", "").strip()
            except OSError as e:
                raise Fail(f"cannot read {env_file}: {e.strerror or type(e).__name__}")
    if not key:
        raise Fail(
            "no OpenAI API key found. Set the OPENAI_API_KEY environment variable, or add a line "
            f"`OPENAI_API_KEY=<your key>` to {root / '.env'} (that file is git-ignored)."
        )
    if not key.isascii() or any(c.isspace() or ord(c) < 32 for c in key):
        raise Fail("the OpenAI API key has spaces, control characters or non-ASCII characters in it; check the value")
    _SECRETS.add(key)
    return key


# ---- project paths ----------------------------------------------------------------------------


def find_main_root(start: Path) -> Path:
    """Walk up from `start` to the folder with broadsword.toml; if that is a linked Git worktree (its `.git` is a file
    reading `gitdir: <main>/.git/worktrees/<name>`), the main checkout is three parents up from that gitdir."""
    start = start.resolve()
    folder = next((d for d in (start, *start.parents) if (d / CONFIG_FILE).is_file()), None)
    if folder is None:
        raise Fail(f"{CONFIG_FILE} not found in {start} or any parent folder (use --root)")
    dot_git = folder / ".git"
    if dot_git.is_file():
        try:
            for line in dot_git.read_text(encoding="utf-8", errors="replace").splitlines():
                if line.strip().startswith("gitdir:"):
                    gitdir = (folder / line.strip()[len("gitdir:"):].strip()).resolve()
                    main = gitdir.parent.parent.parent
                    if (main / CONFIG_FILE).is_file():
                        return main
        except OSError:
            pass
    return folder


def toml_string(root: Path, name: str, default: str) -> str:
    """One `name = "value"` setting from broadsword.toml (the file is flat; this is all we need)."""
    try:
        text = (root / CONFIG_FILE).read_text(encoding="utf-8", errors="replace")
    except OSError:
        return default
    m = re.search(rf'^\s*{re.escape(name)}\s*=\s*"([^"]*)"', text, re.MULTILINE)
    return m.group(1) if m else default


def default_server_path(root: Path) -> Path:
    install = root / toml_string(root, "install_dir", "build/bin")
    for exe in ("broadsword.exe", "broadsword"):
        if (install / exe).is_file():
            return install / exe
    return install / ("broadsword.exe" if os.name == "nt" else "broadsword")


def checked_log_path(root: Path, raw: str) -> Path:
    """`--json-log` may only write inside `<root>/<scratch_dir>/` or `<root>/evidence/`."""
    allowed = [(root / toml_string(root, "scratch_dir", "scratch")).resolve(), (root / "evidence").resolve()]
    path = Path(raw)
    path = (path if path.is_absolute() else Path.cwd() / path).resolve()
    for base in allowed:
        if path != base and base in path.parents:
            return path
    raise Fail(f"--json-log must be a file inside {allowed[0]} or {allowed[1]}; got {path}", 2)


# ---- tool choice and schema conversion --------------------------------------------------------


def is_callable(tool: dict) -> bool:
    return (((tool.get("_meta") or {}).get("broadsword")) or {}).get("mcp_callable", True) is not False


def default_allow(tools: list[dict]) -> list[str]:
    """Read-only tools (annotations.readOnlyHint) plus check, build and test; never anything else."""
    names = []
    for t in tools:
        if (t.get("annotations") or {}).get("readOnlyHint") is True or t["name"] in EXTRA_DEFAULT_TOOLS:
            names.append(t["name"])
    return names


def select_tools(tools: list[dict], allow: list[str] | None) -> tuple[list[dict], list[str]]:
    """(tools to offer, warnings). A tool is offered only if it is allowed AND callable over MCP."""
    warnings: list[str] = []
    by_name = {t["name"]: t for t in tools}
    wanted = default_allow(tools) if allow is None else allow
    offered = []
    for name in wanted:
        t = by_name.get(name)
        if t is None:
            warnings.append(f"--allow names `{name}`, which the server does not have: ignored")
        elif not is_callable(t):
            warnings.append(f"tool `{name}` cannot be called over MCP (_meta.broadsword.mcp_callable is false): not offered")
        elif t not in offered:
            offered.append(t)
    return offered, warnings


_DROPPED_KEYS = ("$schema", "$id", "$comment")
_LENGTH_KEYS = ("minItems", "maxItems")


def convert_schema(schema: object) -> object:
    """Adapt an MCP inputSchema for OpenAI function parameters. Only these changes, each because the API (or its strict
    sub-set) is known to be picky about them:
      - `$schema`, `$id`, `$comment` are removed;
      - an empty `required` list is removed;
      - `minItems` / `maxItems` are removed and written into the property's description instead
        (the server still enforces them and says so in its error).
    Everything else (type, properties, items, enum, description, additionalProperties) is passed through unchanged."""
    if isinstance(schema, list):
        return [convert_schema(x) for x in schema]
    if not isinstance(schema, dict):
        return schema
    result: dict = {}
    notes = []
    for k, v in schema.items():
        if k in _DROPPED_KEYS:
            continue
        if k in _LENGTH_KEYS:
            notes.append(f"{'at least' if k == 'minItems' else 'at most'} {v} item{'s' if v != 1 else ''}")
            continue
        if k == "required" and v == []:
            continue
        if k == "properties" and isinstance(v, dict):
            result[k] = {name: convert_schema(sub) for name, sub in v.items()}
        else:
            result[k] = convert_schema(v)
    if notes:
        text = result.get("description", "")
        result["description"] = (text + " " if text else "") + f"({'; '.join(notes)}.)"
    return result


def to_openai_tool(tool: dict) -> dict:
    params = convert_schema(tool.get("inputSchema") or {"type": "object", "properties": {}})
    if not isinstance(params, dict):
        params = {"type": "object", "properties": {}}
    params.setdefault("type", "object")
    params.setdefault("properties", {})
    return {"type": "function", "function": {"name": tool["name"], "description": tool.get("description", ""), "parameters": params}}


# ---- MCP client -------------------------------------------------------------------------------


def kill_tree(proc: subprocess.Popen) -> None:
    if proc.poll() is not None:
        return
    if os.name == "nt":
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], capture_output=True, check=False)
    try:
        proc.kill()
    except OSError:
        pass


SECRET_SUFFIXES = ("_API_KEY", "_KEY", "_TOKEN", "_SECRET")


def child_env(environ=None) -> dict:
    """A copy of the environment without secrets: OPENAI_API_KEY and anything named *_API_KEY, *_TOKEN, *_SECRET.
    Variables whose VALUE contains a loaded secret are dropped whatever their name. The server (and every tool it runs, including `run`) gets this, so the model cannot read the key from it."""
    environ = os.environ if environ is None else environ
    secrets = [x for x in _SECRETS if x]
    return {
        k: v for k, v in environ.items()
        if k.upper() != "OPENAI_API_KEY" and not k.upper().endswith(SECRET_SUFFIXES) and not any(x in v for x in secrets)
    }


class McpServer:
    """The server process and its JSON-RPC line protocol. Always `close()` it (or use `with`)."""

    def __init__(self, exe: Path, cwd: Path, call_timeout: float):
        self.call_timeout = call_timeout
        self.next_id = 1
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.stderr_tail: list[str] = []
        self.calls_sent = 0
        flags = 0x08000000 if os.name == "nt" else 0  # CREATE_NO_WINDOW
        try:
            self.proc = subprocess.Popen(
                [str(exe), "mcp"], cwd=str(cwd), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                creationflags=flags, env=child_env(),
            )
        except OSError as e:
            raise Fail(f"cannot start the Broadsword server {exe}: {e.strerror or e}. Run `broadsword install` first, or pass --server.")
        self._threads = [
            threading.Thread(target=self._read_stdout, daemon=True),
            threading.Thread(target=self._read_stderr, daemon=True),
        ]
        for t in self._threads:
            t.start()

    def _read_stdout(self) -> None:
        try:
            for raw in self.proc.stdout:  # type: ignore[union-attr]
                self.lines.put(raw.decode("utf-8", errors="replace").strip())
        finally:
            self.lines.put(None)

    def _read_stderr(self) -> None:
        # Drained for the whole life of the server so its stderr pipe never fills up; only the tail is kept.
        for raw in self.proc.stderr:  # type: ignore[union-attr]
            self.stderr_tail.append(raw.decode("utf-8", errors="replace").rstrip())
            del self.stderr_tail[:-50]

    def _send(self, message: dict) -> None:
        data = (json.dumps(message, separators=(",", ":")) + "\n").encode("utf-8")
        try:
            self.proc.stdin.write(data)  # type: ignore[union-attr]
            self.proc.stdin.flush()  # type: ignore[union-attr]
        except (OSError, ValueError):
            raise Fail(f"the Broadsword server closed its input.{self._stderr_note()}")

    def _stderr_note(self) -> str:
        tail = [x for x in self.stderr_tail[-5:] if x]
        return " Its last stderr lines: " + " | ".join(tail) if tail else ""

    def notify(self, method: str, params: dict | None = None) -> None:
        msg: dict = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            msg["params"] = params
        self._send(msg)

    def request(self, method: str, params: dict | None = None, timeout: float = 30.0) -> dict:
        """Send a request and wait for the response with the same id. Other lines are skipped. Returns `result`;
        a JSON-RPC error raises Fail."""
        rid = self.next_id
        self.next_id += 1
        msg: dict = {"jsonrpc": "2.0", "id": rid, "method": method}
        if params is not None:
            msg["params"] = params
        self._send(msg)
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise Fail(f"the Broadsword server did not answer `{method}` within {timeout:g} s")
            try:
                line = self.lines.get(timeout=min(left, 0.5))  # short polls so Ctrl+C is seen promptly
            except queue.Empty:
                continue
            if line is None:
                raise Fail(f"the Broadsword server exited during `{method}`.{self._stderr_note()}")
            if not line:
                continue
            try:
                resp = json.loads(line)
            except ValueError:
                continue  # not JSON: skip
            if not isinstance(resp, dict) or resp.get("id") != rid or ("result" not in resp and "error" not in resp):
                continue  # a notification, a server request or someone else's response: skip
            if "error" in resp:
                e = resp["error"] if isinstance(resp["error"], dict) else {}
                raise McpError(f"{method}: {e.get('message', 'error')} (code {e.get('code')})")
            result = resp.get("result")
            return result if isinstance(result, dict) else {}

    def start(self) -> list[dict]:
        self.request("initialize", {"protocolVersion": PROTOCOL_VERSION, "capabilities": {}, "clientInfo": {"name": CLIENT_NAME, "version": "0.1.0"}})
        self.notify("notifications/initialized")
        tools = self.request("tools/list").get("tools")
        if not isinstance(tools, list):
            raise Fail("tools/list returned no tools list")
        return [t for t in tools if isinstance(t, dict) and isinstance(t.get("name"), str)]

    def call_tool(self, name: str, arguments: dict) -> dict:
        self.calls_sent += 1
        return self.request("tools/call", {"name": name, "arguments": arguments}, timeout=self.call_timeout)

    def close(self) -> None:
        """Stop the server, whatever state it is in. Safe to call twice."""
        proc = self.proc
        try:
            if proc.stdin and not proc.stdin.closed:
                proc.stdin.close()
        except OSError:
            pass
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            kill_tree(proc)
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                pass
        for t in self._threads:
            t.join(timeout=2)
        for pipe in (proc.stdout, proc.stderr):
            try:
                if pipe:
                    pipe.close()
            except OSError:
                pass

    def __enter__(self) -> "McpServer":
        return self

    def __exit__(self, *exc) -> None:
        self.close()


class McpError(Fail):
    pass


def result_text(result: dict) -> str:
    parts = [c.get("text", "") for c in result.get("content") or [] if isinstance(c, dict) and c.get("type") == "text"]
    text = "\n".join(parts)
    if not text and isinstance(result.get("structuredContent"), dict):
        text = json.dumps(result["structuredContent"])
    return text


def exit_code_of(result: dict) -> int | None:
    sc = result.get("structuredContent")
    if isinstance(sc, dict) and isinstance(sc.get("exit_code"), int):
        return sc["exit_code"]
    return None


def truncate_tail(text: str, limit: int) -> str:
    """Keep the last `limit` characters and say so."""
    if limit <= 0 or len(text) <= limit:
        return text
    return f"[tool result truncated: only the last {limit} of {len(text)} characters are shown]\n" + text[-limit:]


def preview(text: str, head: int = 5, tail: int = 5) -> str:
    lines = text.splitlines()
    if len(lines) <= head + tail + 1:
        return "\n".join(lines)
    return "\n".join(lines[:head] + [f"... ({len(lines) - head - tail} lines omitted) ..."] + lines[-tail:])


# ---- OpenAI client ----------------------------------------------------------------------------


def api_message(body: bytes) -> str:
    """The `error.message` of an API error body, redacted. Never the raw body."""
    try:
        data = json.loads(body.decode("utf-8", errors="replace"))
        msg = data["error"]["message"]
        if isinstance(msg, str) and msg.strip():
            return redact(msg.strip())[:600]
    except (ValueError, KeyError, TypeError):
        pass
    return "(the error body was not JSON in the usual form, so it is not shown)"


def retry_delay(headers, attempt: int, base: float) -> float:
    after = headers.get("Retry-After") if headers is not None else None
    if after:
        try:
            return min(max(float(after), 0.0), 60.0)
        except ValueError:
            pass
    return base * (2 ** attempt)


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None  # never follow: the Authorization header must not travel to another URL


_OPENER = urllib.request.build_opener(_NoRedirect)


def chat_completion(base_url: str, key: str, payload: dict, timeout: float, backoff: float, retries: int = 2) -> dict:
    """POST {base}/chat/completions. 429 and 5xx are retried at most `retries` times. Raises Fail with a clear message."""
    url = base_url.rstrip("/") + "/chat/completions"
    data = json.dumps(payload).encode("utf-8")
    for attempt in range(retries + 1):
        req = urllib.request.Request(
            url, data=data, method="POST",
            headers={"Content-Type": "application/json", "Authorization": "Bearer " + key, "User-Agent": CLIENT_NAME},
        )
        try:
            with _OPENER.open(req, timeout=timeout) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as e:
            body = b""
            try:
                body = e.read()
            except (OSError, ValueError):
                pass
            status = e.code
            if status in (429, 500, 502, 503, 504) or status >= 500:
                if attempt < retries:
                    wait = retry_delay(e.headers, attempt, backoff)
                    err(f"HTTP {status} from the API; retrying in {wait:g} s ({attempt + 1}/{retries})")
                    time.sleep(wait)
                    continue
                if status == 429:
                    raise Fail(f"HTTP 429: rate limit or quota reached, still failing after {retries} retries. API message: {api_message(body)}")
                raise Fail(f"HTTP {status}: the API server failed, still failing after {retries} retries. API message: {api_message(body)}")
            if 300 <= status < 400:
                raise Fail(f"HTTP {status}: the API answered with a redirect, which is not followed (the key must not be sent elsewhere). Check --base-url.")
            if status == 401:
                raise Fail("HTTP 401: the API rejected the key. Check OPENAI_API_KEY (or the line in .env): it may be wrong, revoked or for another project.")
            if status == 403:
                raise Fail(f"HTTP 403: access denied (the key may lack permission for this model or region). API message: {api_message(body)}")
            if status in (400, 404):
                raise Fail(f"HTTP {status}: the API refused the request; if it is the model name, check --model. API message: {api_message(body)}")
            raise Fail(f"HTTP {status} from the API. API message: {api_message(body)}")
        except (TimeoutError, socket.timeout):
            raise Fail(f"the API did not answer within {timeout:g} s (--timeout)")
        except urllib.error.URLError as e:
            if isinstance(e.reason, (TimeoutError, socket.timeout)):
                raise Fail(f"the API did not answer within {timeout:g} s (--timeout)")
            raise Fail(f"cannot reach {redact(url)}: {redact(e.reason)}")
        except OSError as e:
            raise Fail(f"network error talking to the API: {redact(e)}")
        try:
            result = json.loads(raw.decode("utf-8", errors="replace"))
        except ValueError:
            raise Fail("the API returned something that is not valid JSON")
        if not isinstance(result, dict) or not isinstance(result.get("choices"), list) or not result["choices"]:
            raise Fail("the API response has no `choices`; it is not a Chat Completions response")
        msg = result["choices"][0].get("message") if isinstance(result["choices"][0], dict) else None
        if not isinstance(msg, dict):
            raise Fail("the API response has no message in its first choice")
        return result
    raise Fail("unreachable")  # pragma: no cover


# ---- the agent loop ---------------------------------------------------------------------------


def clean_assistant(message: dict) -> dict:
    """Only the fields the API accepts back: role, content and tool_calls (id, type, function name and arguments)."""
    clean: dict = {"role": "assistant", "content": message.get("content")}
    calls = []
    for c in message.get("tool_calls") or []:
        fn = c.get("function") if isinstance(c, dict) else None
        fn = fn if isinstance(fn, dict) else {}
        args = fn.get("arguments")
        calls.append({"id": c.get("id") if isinstance(c, dict) else None, "type": "function",
                      "function": {"name": fn.get("name") if isinstance(fn.get("name"), str) else "",
                                   "arguments": args if isinstance(args, str) else json.dumps(args if args is not None else {})}})
    if calls:
        clean["tool_calls"] = calls
    return clean


class Run:
    def __init__(self) -> None:
        self.turns = 0
        self.tool_calls = 0
        self.refused = 0
        self.usage = {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        self.events: list[dict] = []
        self.final: str | None = None
        self.stopped_at_max = False

    def add_usage(self, usage: object) -> dict:
        usage = usage if isinstance(usage, dict) else {}
        seen = {}
        for k in self.usage:
            v = usage.get(k)
            seen[k] = v if isinstance(v, int) else 0
            self.usage[k] += seen[k]
        return seen


def run_agent(args, key: str, server: McpServer, offered: list[dict], run: Run, messages: list[dict]) -> None:
    allowed = {t["name"] for t in offered}
    openai_tools = [to_openai_tool(t) for t in offered]
    for _ in range(args.max_turns):
        run.turns += 1
        payload: dict = {"model": args.model, "messages": messages}
        if openai_tools:
            payload["tools"] = openai_tools
        response = chat_completion(args.base_url, key, payload, args.timeout, args.backoff)
        used = run.add_usage(response.get("usage"))
        message = response["choices"][0]["message"]
        messages.append(clean_assistant(message))
        run.events.append({"type": "response", "turn": run.turns, "usage": used, "model": response.get("model"), "finish_reason": response["choices"][0].get("finish_reason")})
        calls = message.get("tool_calls") or []
        if not calls:
            run.final = message.get("content") or ""
            return
        if message.get("content"):
            out(f"[model] {message['content']}")
        for call in calls:
            run_tool_call(args, server, allowed, run, messages, call)
    run.stopped_at_max = True


def run_tool_call(args, server: McpServer, allowed: set[str], run: Run, messages: list[dict], call: dict) -> None:
    run.tool_calls += 1
    fn = call.get("function") if isinstance(call, dict) else None
    fn = fn if isinstance(fn, dict) else {}
    name = fn.get("name") if isinstance(fn.get("name"), str) else ""
    raw_args = fn.get("arguments")
    if raw_args is not None and not isinstance(raw_args, str):
        raw_args = json.dumps(raw_args)
    call_id = call.get("id") if isinstance(call, dict) else None
    event: dict = {"type": "tool_call", "turn": run.turns, "id": call_id, "name": name, "arguments": raw_args}
    out(f"[tool call] {name} {raw_args if raw_args else '{}'}")

    arguments = None
    problem = None
    if name not in allowed:
        problem = f"tool `{name}` is not allowed in this session and was not run. Allowed tools: {', '.join(sorted(allowed)) or 'none'}."
        run.refused += 1
        event["refused"] = True
    else:
        try:
            arguments = json.loads(raw_args) if raw_args else {}
            if not isinstance(arguments, dict):
                raise ValueError("not an object")
        except (ValueError, TypeError):
            problem = f"the arguments for `{name}` are not a valid JSON object; nothing was run. Send arguments as a JSON object."
            event["bad_arguments"] = True

    if problem is not None:
        text, code, is_error = "tool error: " + problem, None, True
        out(f"[tool result] REFUSED: {problem}")
    else:
        try:
            result = server.call_tool(name, arguments)
        except McpError as e:  # a protocol error for this call: tell the model, keep going
            result = {"content": [{"type": "text", "text": f"tool error: {e}"}], "isError": True}
        text, code, is_error = result_text(result), exit_code_of(result), bool(result.get("isError"))
        out(f"[tool result] exit code: {code if code is not None else 'n/a'}{' (error)' if is_error else ''}")
        shown = preview(text)
        if shown:
            out("\n".join("    " + line for line in shown.splitlines()))
    event.update({"result": text, "exit_code": code, "is_error": is_error})
    run.events.append(event)
    text = redact(text)
    event["result"] = text
    messages.append({"role": "tool", "tool_call_id": call_id, "content": truncate_tail(text, args.max_tool_chars)})


# ---- main -------------------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Let an OpenAI model use Broadsword's tools through its MCP server.")
    p.add_argument("--prompt", required=True, help="what to ask the model")
    p.add_argument("--model", default=DEFAULT_MODEL, help=f"OpenAI model name (default {DEFAULT_MODEL})")
    p.add_argument("--base-url", default=DEFAULT_BASE_URL, help=f"API base URL (default {DEFAULT_BASE_URL})")
    p.add_argument("--allow", help="comma-separated tool names to offer (default: the read-only tools plus check, build and test)")
    p.add_argument("--server", help="path of the broadsword binary to run as `<server> mcp` (default: <root>/<install_dir>/broadsword.exe)")
    p.add_argument("--root", help="project root (default: found by walking up from the current folder to broadsword.toml)")
    p.add_argument("--max-turns", type=int, default=8, help="most model calls before stopping (default 8)")
    p.add_argument("--max-tool-chars", type=int, default=20000, help="a tool result longer than this is cut to its tail before the model sees it (default 20000)")
    p.add_argument("--timeout", type=float, default=120.0, help="seconds to wait for one API response (default 120)")
    p.add_argument("--mcp-timeout", type=float, default=1900.0, help="seconds to wait for one tool call (the server kills calls at 1800 by default)")
    p.add_argument("--backoff", type=float, default=1.0, help="base seconds before a retry of HTTP 429/5xx when there is no Retry-After (doubles each time; default 1)")
    p.add_argument("--json-log", help="write the full transcript here (only inside the root's scratch/ or evidence/ folder)")
    return p


def write_log(path: Path, args, run: Run, messages: list[dict], offered: list[str], error: str | None) -> None:
    doc = {
        "model": args.model, "base_url": args.base_url, "offered_tools": offered, "prompt": args.prompt,
        "turns": run.turns, "tool_calls": run.tool_calls, "refused_tool_calls": run.refused, "usage": run.usage,
        "final": run.final, "stopped_at_max_turns": run.stopped_at_max, "error": error,
        "events": run.events, "messages": messages,
    }
    text = redact(json.dumps(doc, indent=2, ensure_ascii=False))
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text + "\n", encoding="utf-8")


def main(argv: list[str] | None = None) -> int:
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")  # type: ignore[union-attr]
        except (AttributeError, ValueError):
            pass
    args = build_parser().parse_args(argv)
    run, messages, offered_names = Run(), [], []
    log_path: Path | None = None
    error: str | None = None
    server: McpServer | None = None
    code = 0
    try:
        if args.max_turns < 1 or args.max_tool_chars < 1:
            raise Fail("--max-turns and --max-tool-chars must be at least 1", 2)
        root = find_main_root(Path(args.root) if args.root else Path.cwd())
        if args.json_log:
            log_path = checked_log_path(root, args.json_log)
        parsed = urllib.parse.urlparse(args.base_url)
        if parsed.scheme not in ("http", "https") or not parsed.hostname:
            raise Fail(f"--base-url must be an http(s) URL, got {redact(args.base_url)!r}", 2)
        if parsed.scheme == "http" and parsed.hostname not in ("127.0.0.1", "localhost", "::1"):
            raise Fail("refusing to send the API key over plain http to a non-local host; use https", 2)
        key = load_api_key(root)
        exe = Path(args.server) if args.server else default_server_path(root)
        server = McpServer(exe, root, args.mcp_timeout)
        tools = server.start()
        allow = [n.strip() for n in args.allow.split(",") if n.strip()] if args.allow is not None else None
        offered, warnings = select_tools(tools, allow)
        if allow is not None:
            unknown = [n for n in allow if n not in {t["name"] for t in tools}]
            if unknown:
                raise Fail(f"--allow names tools the server does not have: {', '.join(unknown)}. Known: {', '.join(t['name'] for t in tools)}", 2)
        for w in warnings:
            err("warning: " + w)
        offered_names = [t["name"] for t in offered]
        out(f"[setup] server {exe.name}, model {args.model}, tools offered: {', '.join(offered_names) or 'none'}")
        messages = [{"role": "system", "content": SYSTEM_PROMPT}, {"role": "user", "content": args.prompt}]
        run_agent(args, key, server, offered, run, messages)
    except Fail as e:
        error, code = str(e), e.code
        err(f"error: {error}")
        if server is not None and server.stderr_tail:
            err("server stderr (last lines): " + " | ".join(x for x in server.stderr_tail[-3:] if x))
    except Exception as e:  # nothing may end in a traceback
        error, code = f"unexpected {type(e).__name__}: {redact(e)}"[:500], 1
        err(f"error: {error}")
    except KeyboardInterrupt:
        error, code = "interrupted", 130
        err("interrupted")
    finally:
        if server is not None:
            server.close()
    if run.final is not None:
        out("")
        out("[final answer]")
        out(run.final)
    elif run.stopped_at_max:
        error = f"stopped at --max-turns {args.max_turns} without a final answer"
        err(error)
        code = code or 3
    if run.turns or code == 0:
        u = run.usage
        out(f"[summary] model={args.model} turns={run.turns} tool_calls={run.tool_calls} (refused {run.refused}) "
            f"prompt_tokens={u['prompt_tokens']} completion_tokens={u['completion_tokens']} total_tokens={u['total_tokens']}")
    if log_path is not None:
        try:
            write_log(log_path, args, run, messages, offered_names, error)
            out(f"[log] {log_path}")
        except OSError as e:
            err(f"error: cannot write the json log: {e.strerror or e}")
            code = code or 1
    return code


if __name__ == "__main__":
    sys.exit(main())
