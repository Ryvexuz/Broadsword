"""A one-shot OpenAI reviewer that speaks the Claude CLI's headless contract, for the merge queue's SHADOW review.

The queue runs it as:  python -I -B clients/openai_review.py -p --system-prompt <s> --model <m> --tools "" --strict-mcp-config
--max-turns 1 --output-format json   with the review prompt on stdin. It prints ONE JSON object like `claude --output-format json`:
`result` (the answer text), `is_error`, `total_cost_usd` (null unless prices are configured), `usage` and `modelUsage` (keyed by
the model the API says answered). Exit code 0 or 1. The key is never printed.

Codex runs with NO tool that can read a file or run a command (CODEX_NO_TOOLS_ARGS, docs/SHADOW_SANDBOX.md, proven by
scripts/shadow_canary.py); an answer whose event stream shows any tool use is discarded ("codex_items" lists what was seen).

Path order: the Codex CLI first (ChatGPT plan, `codex exec ... --json -o <file> -`, prompt on stdin, fresh temp folder); if it is missing
or fails, the API below answers and the output records "backend": "api" plus "codex_error". A codex answer has "backend": "codex",
"model_reported": false (codex does not name the model) and a null cost. It needs USERPROFILE/HOME/APPDATA/PATH for its sign-in;
the queue's scrub drops only secret-named variables, so they survive.

The key comes from the OPENAI_API_KEY environment variable, else the main root's `.env` (the loader in openai_mcp.py).
Optional environment: OPENAI_BASE_URL (tests point it at a local fake server), OPENAI_PRICE_INPUT_PER_MTOK and
OPENAI_PRICE_OUTPUT_PER_MTOK (USD per million tokens; both needed for a cost), OPENAI_REVIEW_TIMEOUT (seconds, ONE deadline for the whole call,
inside the queue's shadow limit) and OPENAI_REVIEW_CODEX_SHARE (0 < share <= 1, the part of it codex may use; the API fallback gets the
rest). Invalid values are an error naming the variable. Until the queue passes them, absent ones use 240 and 0.6 and the result's "notes"
(and "codex_error" on a fallback) say so.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import importlib.util
import json
import os
import subprocess
import tempfile
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("openai_mcp", HERE / "openai_mcp.py")
oc = importlib.util.module_from_spec(_spec)
sys.modules["openai_mcp"] = oc
_spec.loader.exec_module(oc)


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(description="Shadow reviewer: same flags as the Claude CLI, answered by an OpenAI model.")
    p.add_argument("-p", "--print", dest="print_mode", action="store_true")
    p.add_argument("--system-prompt", default="")
    p.add_argument("--model", required=True)
    p.add_argument("--tools", default="")
    p.add_argument("--strict-mcp-config", action="store_true")
    p.add_argument("--max-turns", default="1")
    p.add_argument("--output-format", default="json")
    return p


def price(environ, name):
    try:
        return float(environ.get(name, ""))
    except ValueError:
        return None


def error_result(message: str) -> dict:
    return {"type": "result", "is_error": True, "result": oc.redact(message), "total_cost_usd": None, "usage": {}, "modelUsage": {}}


def find_codex(environ):
    """The codex executable: found by walking the PATH entries of the environment given (absolute entries only; "." , empty and
    relative entries are skipped, so the current folder is never searched, which shutil.which may do on Windows), then the npm
    global folder under APPDATA (absolute only). None if absent."""
    names = ("codex.cmd", "codex.exe", "codex.bat", "codex") if os.name == "nt" else ("codex",)
    folders = []
    for raw in (environ.get("PATH") or "").split(os.pathsep):
        entry = raw.strip().strip('"')
        if entry and Path(entry).is_absolute():
            folders.append(Path(entry))
    appdata = environ.get("APPDATA")
    if appdata and Path(appdata).is_absolute():
        folders.append(Path(appdata) / "npm")
    for folder in folders:
        for name in names:
            cand = folder / name
            if cand.is_file():
                return str(cand)
    return None


def codex_env(environ, root: Path) -> dict:
    """The ONE place the codex child's environment is built. Codex signs in with ChatGPT, so it never needs the API key: drop
    OPENAI_API_KEY by name, and every variable whose VALUE equals a value in `<root>/.env` or the API key's own value (the same
    secret under another name). Empty values are not secrets and are kept. The API fallback uses the key in this process only.
    Raises oc.Fail if the .env cannot be read, so the child is never started with an unchecked environment."""
    secrets = set()
    env_file = root / ".env"
    if env_file.is_file():
        try:
            secrets.update(v for v in oc.parse_env_text(env_file.read_text(encoding="utf-8", errors="replace")).values() if v)
        except OSError as e:
            raise oc.Fail(f"cannot read the .env to scrub the codex environment: {type(e).__name__}")
    key = (environ.get("OPENAI_API_KEY") or "").strip()
    if key:
        secrets.add(key)
    return {k: v for k, v in environ.items() if k.upper() != "OPENAI_API_KEY" and v.strip() not in secrets}


# TEMPORARY (task t-20261119-175203): until the queue passes OPENAI_REVIEW_TIMEOUT and OPENAI_REVIEW_CODEX_SHARE, an ABSENT variable
# falls back to these and the result says so. A PRESENT but invalid value is always an error. Remove once the queue passes both.
LEGACY_TIMEOUT = 240.0
LEGACY_CODEX_SHARE = 0.6


def settings_of(environ):
    """(overall deadline seconds, codex share of it, notes). OPENAI_REVIEW_TIMEOUT is the ONE deadline for the whole call (codex plus
    the API fallback); the queue only kills this wrapper at shadow_reviewer_timeout_seconds, so the value it passes must leave room to
    print the JSON. OPENAI_REVIEW_CODEX_SHARE (0 < share <= 1) is the part of it codex may use. A present but invalid value raises
    oc.Fail naming the variable; an absent one uses the legacy value and adds a note naming the variable."""
    notes = []

    def number(name, legacy, ok, rule):
        raw = environ.get(name)
        if raw is None:
            notes.append(f"{name} not set: using the legacy {legacy:g}")
            return legacy
        try:
            v = float(raw)
        except ValueError:
            raise oc.Fail(f"{name} is invalid ({raw!r}): {rule}")
        if not (v == v and ok(v)):
            raise oc.Fail(f"{name} is invalid ({raw!r}): {rule}")
        return v

    timeout = number("OPENAI_REVIEW_TIMEOUT", LEGACY_TIMEOUT, lambda v: 0 < v < float("inf"), "must be a number of seconds > 0")
    share = number("OPENAI_REVIEW_CODEX_SHARE", LEGACY_CODEX_SHARE, lambda v: 0 < v <= 1, "must be a number with 0 < share <= 1")
    return timeout, share, notes


# The API fallback needs at least this many seconds; with less it fails with a clear error instead of starting a doomed call.
MIN_API_SECONDS = 5.0

# THE ONE PLACE codex's tool access is set (audit A6, docs/SHADOW_SANDBOX.md). The reviewed diff is untrusted text, so the model
# must not be able to read any file: `--sandbox read-only` alone still lets its shell read every file by absolute path (canary
# 2026-11-19). These flags take away every tool that can read a file or run a command, and `--strict-config` makes codex refuse to
# start if a later version stops knowing any of them (it errors on an unknown -c key or feature name), so they can never
# silently stop applying. What remains (codex 0.170.0, `codex debug prompt-input`): the code-mode `exec` (a V8 isolate with no
# file system or network) whose only nested tools are apply_patch (writes only, refused by the read-only sandbox) and MCP
# resource readers (no MCP server is configured), plus wait / request_user_input and the multi-agent tools (sub-agents get this
# same configuration).
CODEX_NO_TOOLS_ARGS = (
    "--sandbox", "read-only",
    "--ephemeral",
    "--skip-git-repo-check",
    "--ignore-user-config",  # no user MCP servers (the Node REPL), plugins, notify hooks or project trust; sign-in still works
    "--ignore-rules",  # no execpolicy .rules files
    "--strict-config",
    "--disable", "shell_tool",
    "--disable", "unified_exec",
    "--disable", "view_image",  # reads image files
    "--disable", "apps",
    "--disable", "plugins",
    "--disable", "remote_plugin",
    "--disable", "browser_use",
    "--disable", "browser_use_external",
    "--disable", "in_app_browser",
    "--disable", "computer_use",
    "--disable", "image_generation",
    "--disable", "multi_agent",
    "--disable", "multi_agent_v2",
    "--disable", "sleep_tool",
    "--disable", "goals",
    "--disable", "tool_suggest",
    "--disable", "skill_search",
    "--disable", "skill_mcp_dependency_install",
    "--disable", "hooks",
    "--disable", "memories",
    "--disable", "workspace_dependencies",
    "-c", 'web_search="disabled"',
    "-c", "project_doc_max_bytes=0",  # no AGENTS.md from the working folder or its parents
    "-c", 'approval_policy="never"',  # nothing is ever approved
)

# The --json items a tool-free answer may contain. An MCP call is labelled `mcp_tool_call:<server>/<tool>`; the two allowed ones
# are codex's built-in resource LISTINGS, which answer `[]` because no MCP server is configured (the model calls them when asked
# to read a file; canary diagnostic 2026-11-19). Any other item (a command, a file change, read_mcp_resource, any other MCP or
# sub-agent call, a web search...) means the model reached a tool: the answer is discarded and the call fails (a tripwire for a
# codex update that brings a tool back).
TOOL_FREE_ITEMS = frozenset({
    "agent_message",
    "reasoning",
    "error",  # codex's own notices, such as "Falling back from WebSockets to HTTPS transport"
    "mcp_tool_call:codex/list_mcp_resources",
    "mcp_tool_call:codex/list_mcp_resource_templates",
})


def codex_item_types(stdout: str) -> list:
    """Every item in codex's --json event stream, as sorted labels: the item type, or `mcp_tool_call:<server>/<tool>`."""
    seen = set()
    for line in (stdout or "").splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if isinstance(ev, dict) and str(ev.get("type", "")).startswith("item.") and isinstance(ev.get("item"), dict):
            item = ev["item"]
            kind = str(item.get("type"))
            if kind == "mcp_tool_call":
                kind = f"{kind}:{item.get('server')}/{item.get('tool')}"
            seen.add(kind)
    return sorted(seen)


def kill_tree(p) -> None:
    """Kill the process and everything it started. On Windows codex is a codex.cmd, so killing only the pid leaves the node
    grandchild alive, still holding the output pipes."""
    try:
        if os.name == "nt":
            subprocess.run(["taskkill", "/T", "/F", "/PID", str(p.pid)], capture_output=True, timeout=20)
        else:
            os.killpg(p.pid, 9)  # started with start_new_session=True, so the pid is the group id
    except Exception:
        pass
    try:
        p.kill()
    except Exception:
        pass


def run_with_deadline(cmd, text_in: str, work: str, environ, budget: float):
    """Run cmd with a hard time limit that really holds: returns (returncode, stdout, stderr), or raises oc.Fail on timeout/start error."""
    kw = {} if os.name == "nt" else {"start_new_session": True}
    try:
        p = subprocess.Popen(cmd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", errors="replace", cwd=work, env=dict(environ), **kw)
    except OSError as e:
        raise oc.Fail(f"codex could not start: {type(e).__name__}")
    try:
        out, err = p.communicate(input=text_in, timeout=max(budget, 0.1))
    except subprocess.TimeoutExpired:
        kill_tree(p)
        try:
            p.communicate(timeout=5)
        except Exception:  # still blocked on a pipe: give up on its output
            for f in (p.stdin, p.stdout, p.stderr):
                try:
                    if f:
                        f.close()
                except Exception:
                    pass
        raise oc.Fail(f"codex timed out after {budget:.0f} s")
    return p.returncode, out, err


def review_codex(args, prompt: str, environ, deadline: float, timeout: float, share: float) -> dict:
    """Answer through the Codex CLI (the ChatGPT plan). Raises oc.Fail with a short reason when it cannot."""
    exe = find_codex(environ)
    if exe is None:
        raise oc.Fail("codex executable not found")
    text_in = (f"=== SYSTEM INSTRUCTIONS ===\n{args.system_prompt}\n=== END SYSTEM INSTRUCTIONS ===\n\n" if args.system_prompt else "") + prompt
    budget = min(timeout * share, deadline - time.monotonic())
    with tempfile.TemporaryDirectory(prefix="bsw-codex-") as work:
        answer = Path(work) / "answer.txt"
        cmd = [exe, "exec", "-m", args.model, *CODEX_NO_TOOLS_ARGS, "--json", "-o", str(answer), "-"]
        child = codex_env(environ, oc.find_main_root(HERE))
        code, stdout, stderr = run_with_deadline(cmd, text_in, work, child, budget)
        items = codex_item_types(stdout)
        tools = [t for t in items if t not in TOOL_FREE_ITEMS]
        if tools:
            raise oc.Fail(f"codex used a tool ({', '.join(tools)}); its answer is discarded")
        if code != 0:
            raise oc.Fail(f"codex exited with code {code}: {(stderr or '').strip()[-200:]}")
        text = answer.read_text(encoding="utf-8", errors="replace") if answer.is_file() else ""
    if not text.strip():
        raise oc.Fail("codex answer is empty")
    usage = None
    for line in (stdout or "").splitlines():
        try:
            ev = json.loads(line)
        except ValueError:
            continue
        if isinstance(ev, dict) and ev.get("type") == "turn.completed" and isinstance(ev.get("usage"), dict):
            usage = ev["usage"]
    if not usage:  # never report zero usage as real data
        raise oc.Fail("codex output has no parseable turn.completed event with usage")
    tin, tout = int(usage.get("input_tokens") or 0), int(usage.get("output_tokens") or 0)
    return {
        "type": "result",
        "is_error": False,
        "result": oc.redact(text),
        "total_cost_usd": None,
        "usage": dict(usage),
        "modelUsage": {args.model: {"inputTokens": tin, "outputTokens": tout}},
        "model_reported": False,
        "backend": "codex",
        "codex_items": items,
    }


def review(args, prompt: str, environ) -> dict:
    timeout, share, notes = settings_of(environ)
    deadline = time.monotonic() + timeout
    try:
        result = review_codex(args, prompt, environ, deadline, timeout, share)
        if notes:
            result["notes"] = notes
        return result
    except Exception as e:  # any codex failure falls back, and is recorded
        codex_error = oc.redact(f"{type(e).__name__}: {e}" if not isinstance(e, oc.Fail) else str(e))[:400]
    try:
        result = review_api(args, prompt, environ, deadline - time.monotonic(), timeout)
    except oc.Fail as e:
        result = error_result(str(e))
    except Exception as e:
        result = error_result(f"unexpected {type(e).__name__}: {e}")
    if result["is_error"]:  # the final error must show the codex failure too, not only the API's
        result["result"] = f"codex: {codex_error}; api: {result['result']}"
    result["backend"] = "api"
    result["codex_error"] = codex_error + ("; " + "; ".join(notes) if notes else "")
    if notes:
        result["notes"] = notes
    return result


def review_api(args, prompt: str, environ, remaining: float, timeout_total: float) -> dict:
    """The API fallback, given what is left of the one overall deadline. No retries: a retry could run past the deadline."""
    if remaining < MIN_API_SECONDS:
        raise oc.Fail(f"only {max(remaining, 0):.1f} s of the {timeout_total:.0f} s deadline remain, under the {MIN_API_SECONDS:.0f} s the API needs")
    root = oc.find_main_root(HERE)
    key = oc.load_api_key(root, environ)
    base = environ.get("OPENAI_BASE_URL") or oc.DEFAULT_BASE_URL
    timeout = remaining
    messages = []
    if args.system_prompt:
        messages.append({"role": "system", "content": args.system_prompt})
    messages.append({"role": "user", "content": prompt})
    resp = oc.chat_completion(base, key, {"model": args.model, "messages": messages}, timeout, 1.0, retries=0)
    choice = resp["choices"][0]
    text = choice["message"].get("content")
    text = text if isinstance(text, str) else ""
    reason = choice.get("finish_reason")
    usage = resp.get("usage") if isinstance(resp.get("usage"), dict) else {}
    tin, tout = int(usage.get("prompt_tokens") or 0), int(usage.get("completion_tokens") or 0)
    pin, pout = price(environ, "OPENAI_PRICE_INPUT_PER_MTOK"), price(environ, "OPENAI_PRICE_OUTPUT_PER_MTOK")
    cost = round((tin * pin + tout * pout) / 1e6, 6) if pin is not None and pout is not None else None
    model = resp.get("model") if isinstance(resp.get("model"), str) and resp.get("model") else args.model
    result = {
        "type": "result",
        "is_error": False,
        "result": oc.redact(text),
        "total_cost_usd": cost,
        "usage": {"input_tokens": tin, "output_tokens": tout},
        "modelUsage": {model: {"inputTokens": tin, "outputTokens": tout}},
    }
    if reason != "stop":
        result["is_error"] = True
        result["result"] = oc.redact(f"the model did not finish cleanly: finish_reason = {reason!r}" + (f"; partial answer: {text[:300]}" if text.strip() else ""))
    elif not text.strip():
        result["is_error"] = True
        result["result"] = "the model's answer is empty"
    return result


def main(argv=None, stdin=None, environ=None) -> int:
    environ = os.environ if environ is None else environ
    try:
        args = build_parser().parse_args(argv)
        prompt = (sys.stdin if stdin is None else stdin).read()
        result = review(args, prompt, environ)
    except oc.Fail as e:
        result = error_result(str(e))
    except SystemExit:
        raise
    except Exception as e:  # never a traceback that could hold a key
        result = error_result(f"unexpected {type(e).__name__}: {e}")
    sys.stdout.write(json.dumps(result) + "\n")
    sys.stdout.flush()
    return 1 if result["is_error"] else 0


if __name__ == "__main__":
    sys.exit(main())
