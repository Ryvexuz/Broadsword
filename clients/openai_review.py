"""A one-shot OpenAI reviewer that speaks the Claude CLI's headless contract, for the merge queue's SHADOW review.

The queue runs it as:  python -I -B clients/openai_review.py -p --system-prompt <s> --model <m> --tools "" --strict-mcp-config
--max-turns 1 --output-format json   with the review prompt on stdin. It prints ONE JSON object like `claude --output-format json`:
`result` (the answer text), `is_error`, `total_cost_usd` (null unless prices are configured), `usage` and `modelUsage` (keyed by
the model the API says answered). Exit code 0 or 1. The key is never printed.

Path order: the Codex CLI first (ChatGPT plan, `codex exec ... --json -o <file> -`, prompt on stdin, fresh temp folder); if it is missing
or fails, the API below answers and the output records "backend": "api" plus "codex_error". A codex answer has "backend": "codex",
"model_reported": false (codex does not name the model) and a null cost. It needs USERPROFILE/HOME/APPDATA/PATH for its sign-in;
the queue's scrub drops only secret-named variables, so they survive.

The key comes from the OPENAI_API_KEY environment variable, else the main root's `.env` (the loader in openai_mcp.py).
Optional environment: OPENAI_BASE_URL (tests point it at a local fake server), OPENAI_PRICE_INPUT_PER_MTOK and
OPENAI_PRICE_OUTPUT_PER_MTOK (USD per million tokens; both needed for a cost), OPENAI_REVIEW_TIMEOUT (seconds, ONE deadline for the whole call,
default 240, inside the queue's 300 s shadow limit; codex gets at most 60% of it, the API fallback the rest).
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import importlib.util
import json
import os
import shutil
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
    """The codex executable: on PATH (the environment given), else the npm global folder under APPDATA. None if absent."""
    found = shutil.which("codex", path=environ.get("PATH"))
    if found:
        return found
    appdata = environ.get("APPDATA")
    if appdata:
        for name in ("codex.cmd", "codex.exe", "codex"):
            cand = Path(appdata) / "npm" / name
            if cand.is_file():
                return str(cand)
    return None


def timeout_of(environ) -> float:
    """The ONE deadline for the whole call (codex plus the API fallback). The default sits inside the queue's
    shadow_reviewer_timeout_seconds (300 in broadsword.toml): the queue passes no timeout to this wrapper, it only kills it at that
    limit, so the wrapper must finish and print its JSON before then."""
    try:
        return float(environ.get("OPENAI_REVIEW_TIMEOUT", "240"))
    except ValueError:
        return 240.0


# Codex may use at most this share of the overall deadline, so a hung codex still leaves the API fallback real time to answer.
CODEX_SHARE = 0.6
# The API fallback needs at least this many seconds; with less it fails with a clear error instead of starting a doomed call.
MIN_API_SECONDS = 5.0


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


def review_codex(args, prompt: str, environ, deadline: float) -> dict:
    """Answer through the Codex CLI (the ChatGPT plan). Raises oc.Fail with a short reason when it cannot."""
    exe = find_codex(environ)
    if exe is None:
        raise oc.Fail("codex executable not found")
    text_in = (f"=== SYSTEM INSTRUCTIONS ===\n{args.system_prompt}\n=== END SYSTEM INSTRUCTIONS ===\n\n" if args.system_prompt else "") + prompt
    budget = min(timeout_of(environ) * CODEX_SHARE, deadline - time.monotonic())
    with tempfile.TemporaryDirectory(prefix="bsw-codex-") as work:
        answer = Path(work) / "answer.txt"
        cmd = [exe, "exec", "-m", args.model, "--sandbox", "read-only", "--ephemeral", "--skip-git-repo-check", "--json", "-o", str(answer), "-"]
        code, stdout, stderr = run_with_deadline(cmd, text_in, work, environ, budget)
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
    }


def review(args, prompt: str, environ) -> dict:
    deadline = time.monotonic() + timeout_of(environ)
    try:
        return review_codex(args, prompt, environ, deadline)
    except Exception as e:  # any codex failure falls back, and is recorded
        codex_error = oc.redact(f"{type(e).__name__}: {e}" if not isinstance(e, oc.Fail) else str(e))[:400]
    try:
        result = review_api(args, prompt, environ, deadline - time.monotonic())
    except oc.Fail as e:
        result = error_result(str(e))
    except Exception as e:
        result = error_result(f"unexpected {type(e).__name__}: {e}")
    result["backend"] = "api"
    result["codex_error"] = codex_error
    return result


def review_api(args, prompt: str, environ, remaining: float) -> dict:
    """The API fallback, given what is left of the one overall deadline. No retries: a retry could run past the deadline."""
    if remaining < MIN_API_SECONDS:
        raise oc.Fail(f"only {max(remaining, 0):.1f} s of the {timeout_of(environ):.0f} s deadline remain, under the {MIN_API_SECONDS:.0f} s the API needs")
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
