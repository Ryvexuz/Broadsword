"""A one-shot OpenAI reviewer that speaks the Claude CLI's headless contract, for the merge queue's SHADOW review.

The queue runs it as:  python -I -B clients/openai_review.py -p --system-prompt <s> --model <m> --tools "" --strict-mcp-config
--max-turns 1 --output-format json   with the review prompt on stdin. It prints ONE JSON object like `claude --output-format json`:
`result` (the answer text), `is_error`, `total_cost_usd` (null unless prices are configured), `usage` and `modelUsage` (keyed by
the model the API says answered). Exit code 0 or 1. The key is never printed.

Codex is offered NO tool at all (codex_command: CODEX_NO_TOOLS_ARGS plus a no-tools model catalog; docs/SHADOW_SANDBOX.md,
proven by scripts/shadow_canary.py); an answer whose event stream shows any tool item, unknown event or unparseable line is
discarded ("codex_items" lists what was seen).

Path order: the Codex CLI first (ChatGPT plan, `codex exec ... --json -o <file> -`, prompt on stdin, fresh temp folder); if it is missing
or fails, the API below answers and the output records "backend": "api" plus "codex_error". A codex answer has "backend": "codex",
"model_reported" (true only when a codex event names the answering model, see codex_model_used; on codex-cli 0.170.0 none does,
so it is false) and "model_used" (the names reported, [] when none) and a null cost. It needs USERPROFILE/HOME/APPDATA/PATH for its sign-in;
the queue's scrub drops only secret-named variables, so they survive.

The key comes from the OPENAI_API_KEY environment variable, else the main root's `.env` (the loader in openai_mcp.py).
Optional environment: OPENAI_BASE_URL (tests point it at a local fake server), OPENAI_PRICE_INPUT_PER_MTOK and
OPENAI_PRICE_OUTPUT_PER_MTOK (USD per million tokens; both needed for a cost), OPENAI_REVIEW_TIMEOUT (seconds, ONE deadline for the whole call,
inside the queue's shadow limit) and OPENAI_REVIEW_CODEX_SHARE (0 < share <= 1, the part of it codex may use; the API fallback gets the
rest). Invalid values are an error naming the variable. Until the queue passes them, absent ones use 240 and 0.6 and the result's "notes"
(and "codex_error" on a fallback) say so.

Two gates sit at the lowest level every GPT call passes through: the cap and ledger wrap review_codex() and review_api() themselves
(see `metered`), so review() and direct callers such as bench/reviewer/run_codex.py are both covered, and one call is counted once:

1. Daily spend cap (Lazlo 2026-11-19: GPT reviews may draw on the OpenAI overflow credits, at most $20 a day). Every call that gets
   past the gates is appended as one row to <state_dir>/gpt-spend/<UTC date>.tsv (utc, caller, model, input_tokens, output_tokens,
   est_usd, plan_or_credits, call_type) under an OS lock. Older files have 7 columns (no call_type); they are never rewritten and
   still sum correctly, and scripts/usage_sum.py reads their rows as call_type "unknown". `call_type` comes from OPENAI_REVIEW_CALL_TYPE
   (gate, shadow, full or bench; "unknown" if unset or blank). Before a call (a codex attempt and an API fallback are each a call), if today's est_usd sum is >= the cap, no model is called: the result
   is is_error with codex_error "daily_cap_reached: <sum> of <cap>" (callers record SKIPPED), and CAP-REACHED-<date>.txt is written
   once. THE ESTIMATE IS CONSERVATIVE: tokens x list price from bench/models.json, and calls the ChatGPT plan answered are counted
   at list price too, so the sum can only over-state what the credits were charged. A model with no listed price counts at the
   highest listed price (never 0). Calls running at the same moment can overshoot the cap by what they cost: the check is made
   before a call, not held across it. Cap 0 means no GPT call at all. `caller` comes from OPENAI_REVIEW_CALLER ("unlabelled" if absent).
2. Codex canary (Lazlo 2026-11-19: re-run the shadow canary after any Codex update). Before any codex process other than
   `codex --version` is started, the current version is compared with <root>/<state_dir>/canary/last-pass.json (<root> is the main
   Broadsword root, also from a worktree: there is one record; written by scripts/shadow_canary.py on a full PASS). Missing, unreadable or different: scripts/shadow_canary.py --offline-only runs as a child
   (no model call) with the codex child environment. It must exit 0 AND leave a last-pass.json naming this version, else no codex is
   started and the result is is_error with codex_error "canary_failed: <reason>". `codex --version` failing, or no codex found, is "codex_unavailable".
   Neither falls back to the API: the queue records the shadow as ERROR and goes on.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import datetime
import importlib.util
import json
import os
import subprocess
import tempfile
import time
import tomllib
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
# must be offered NO tool at all. `--sandbox read-only` alone lets its shell read any file by absolute path, and even with every
# tool feature disabled codex 0.170.0 still offers the code-mode `exec` tool, whose nested `apply_patch` is a file-content oracle
# (it checks a patch against any file before the sandbox refuses the write), and the multi-agent tools. Those come from the
# MODEL'S CATALOG ENTRY (tool_mode "code_mode_only", apply_patch_tool_type, multi_agent_version), not from features, so
# codex_command() also hands codex a catalog entry of our own for the requested model (no_tools_catalog), and
# `tools.experimental_request_user_input.enabled=false` removes the last one. Result, captured from the request codex sends
# (local fake Responses server, scripts/shadow_canary.py leg A): `"tools": []` and no `additional_tools` item.
# `--strict-config` makes codex refuse to start if a later version does not know any key or feature named here, so none of them
# can silently stop applying. The feature switches below are kept as a second layer.
CODEX_NO_TOOLS_ARGS = (
    "--sandbox", "read-only",
    "--ephemeral",
    "--skip-git-repo-check",
    "--ignore-user-config",  # no user MCP servers (the Node REPL), plugins, notify hooks or project trust; sign-in still works
    "--ignore-rules",  # no execpolicy .rules files
    "--strict-config",
    "-c", "tools.experimental_request_user_input.enabled=false",
    "--disable", "shell_tool",
    "--disable", "unified_exec",
    "--disable", "view_image",
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

# The instructions codex puts first in the request in place of its own agent prompt (which describes a shell and a workspace).
NO_TOOLS_INSTRUCTIONS = (
    "You answer one request with text only. You have no tools: you cannot read files, run commands, browse, or start other "
    "agents. Answer only from the text you are given."
)


def no_tools_catalog(model: str) -> dict:
    """The model catalog codex is given (model_catalog_json): ONE entry, for exactly the requested model, with no tools:
    tool_mode "direct" (no code-mode `exec`), shell_type "disabled", and no apply_patch_tool_type, multi_agent_version, web
    search or experimental tools. The other fields are the ones codex 0.170.0 needs to parse an entry."""
    return {"models": [{
        "slug": model,
        "display_name": model,
        "tool_mode": "direct",
        "shell_type": "disabled",
        "experimental_supported_tools": [],
        "supports_search_tool": False,
        "include_skills_usage_instructions": False,
        "include_plugin_usage_instructions": False,
        "include_apps_usage_instructions": False,
        "supported_reasoning_levels": [],
        "visibility": "list",
        "supported_in_api": True,
        "priority": 0,
        "support_verbosity": False,
        "truncation_policy": {"mode": "tokens", "limit": 10000},
        "base_instructions": NO_TOOLS_INSTRUCTIONS,
    }]}


def codex_command(exe: str, model: str, work: Path, answer: Path) -> list:
    """The full codex command line, used by the wrapper and the canary alike. Writes the no-tools catalog into `work`."""
    catalog = Path(work) / "no-tools-catalog.json"
    catalog.write_text(json.dumps(no_tools_catalog(model)), encoding="utf-8")
    path = str(catalog)
    if any(c in path for c in ("'", "\n", "\r")):  # it goes into a TOML literal string
        raise oc.Fail("the temp folder path cannot be passed to codex (it holds a quote or a line break)")
    return [exe, "exec", "-m", model, *CODEX_NO_TOOLS_ARGS, "-c", f"model_catalog_json='{path}'", "--json", "-o", str(answer), "-"]


# The --json stream of a tool-free answer holds only these event types and these item types. Anything else (a command, a file
# change, an MCP or sub-agent call, a web search, an unknown event or item, a line that is not one JSON object) trips the call:
# the answer is discarded and the wrapper falls back. With no tool offered this should never fire; it is there for a codex update
# that brings a tool back. It cannot be the protection on its own: calls made inside a code-mode `exec` never show up as items.
CODEX_EVENTS = frozenset({"thread.started", "turn.started", "turn.completed", "turn.failed", "error", "item.started", "item.updated", "item.completed"})
TOOL_FREE_ITEMS = frozenset({
    "agent_message",
    "reasoning",
    "error",  # codex's own notices, such as "Falling back from WebSockets to HTTPS transport"
})


def codex_events(stdout: str) -> list:
    """The parsed events, one per line. Lines are split on "\\n" ONLY: codex does not escape U+2028, U+2029 or U+0085 inside
    its JSON, and str.splitlines() would cut a line there. A non-empty line that is not one JSON object becomes
    {"type": "unparseable"}."""
    events = []
    for line in (stdout or "").split("\n"):
        line = line.rstrip("\r")
        if not line.strip():
            continue
        try:
            ev = json.loads(line)
        except ValueError:
            ev = None
        events.append(ev if isinstance(ev, dict) else {"type": "unparseable"})
    return events


def codex_item_types(stdout: str) -> list:
    """Sorted labels for the event stream: each item's type (an MCP call as `mcp_tool_call:<server>/<tool>`), `event:<type>`
    for an unknown event, `unparseable` for a bad line, `item:invalid` for an item event without an item object."""
    seen = set()
    for ev in codex_events(stdout):
        kind = str(ev.get("type"))
        if kind == "unparseable":
            seen.add("unparseable")
        elif kind not in CODEX_EVENTS:
            seen.add(f"event:{kind}")
        elif kind.startswith("item."):
            item = ev.get("item")
            if not isinstance(item, dict):
                seen.add("item:invalid")
                continue
            label = str(item.get("type"))
            if label == "mcp_tool_call":
                label = f"{label}:{item.get('server')}/{item.get('tool')}"
            seen.add(label)
    return sorted(seen)


# Events that may carry the answering model as a top-level string "model". Checked on codex-cli 0.170.0 (a real codex run against a
# local fake Responses server whose response.created and response.completed both named a model): thread.started holds only
# thread_id, turn.started nothing, item.completed the item, turn.completed only usage. NO event names the model, so today
# model_reported is false. This reads the field if a later codex adds it; the tests use a fake codex that emits it.
MODEL_EVENTS = frozenset({"thread.started", "turn.started", "turn.completed"})


def codex_model_used(stdout: str) -> list:
    """Sorted distinct model names that codex events report (a non-empty string "model" on a MODEL_EVENTS event); [] when none."""
    seen = set()
    for ev in codex_events(stdout):
        m = ev.get("model")
        if ev.get("type") in MODEL_EVENTS and isinstance(m, str) and m.strip():
            seen.add(m.strip())
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
        raise Refused("codex_unavailable: codex executable not found")
    text_in = (f"=== SYSTEM INSTRUCTIONS ===\n{args.system_prompt}\n=== END SYSTEM INSTRUCTIONS ===\n\n" if args.system_prompt else "") + prompt
    root = oc.find_main_root(HERE)
    child = codex_env(environ, root)
    canary_gate(root, exe, child, deadline)  # raises Refused BEFORE any codex process other than `codex --version` starts
    budget = min(timeout * share, deadline - time.monotonic())
    with tempfile.TemporaryDirectory(prefix="bsw-codex-") as work:
        answer = Path(work) / "answer.txt"
        cmd = codex_command(exe, args.model, Path(work), answer)
        code, stdout, stderr = run_with_deadline(cmd, text_in, work, child, budget)
        items = codex_item_types(stdout)
        tools = [t for t in items if t not in TOOL_FREE_ITEMS]
        if tools:
            raise oc.Fail(f"codex used a tool or printed an unexpected event ({', '.join(tools)}); its answer is discarded")
        if code != 0:
            if not (stderr or "").strip() and not (stdout or "").strip():
                raise oc.Fail(f"codex exited {code} with no output (model may be unavailable: {args.model})")
            raise oc.Fail(f"codex exited with code {code}: {(stderr or '').strip()[-200:]}")
        text = answer.read_text(encoding="utf-8", errors="replace") if answer.is_file() else ""
    if not text.strip():
        raise oc.Fail("codex answer is empty")
    usage = None
    for ev in codex_events(stdout):
        if ev.get("type") == "turn.completed" and isinstance(ev.get("usage"), dict):
            usage = ev["usage"]
    if not usage:  # never report zero usage as real data
        raise oc.Fail("codex output has no parseable turn.completed event with usage")
    tin, tout = int(usage.get("input_tokens") or 0), int(usage.get("output_tokens") or 0)
    used = codex_model_used(stdout)
    keyed = used[0] if len(used) == 1 else args.model  # several different names: keep the requested key, model_used lists them all
    return {
        "type": "result",
        "is_error": False,
        "result": oc.redact(text),
        "total_cost_usd": None,
        "usage": dict(usage),
        "modelUsage": {keyed: {"inputTokens": tin, "outputTokens": tout}},
        "model_reported": bool(used),
        "model_used": used,
        "backend": "codex",
        "codex_items": items,
    }


# ---- gate 1: the daily GPT spend cap ----------------------------------------------------------------------------------------

# TEMPORARY: Lazlo's cap of $20 a day (2026-11-19), in this ONE named place until broadsword.toml has the key `gpt_daily_cap_usd`
# (follow-up once the toml is free to edit). If the toml already has that key it wins over this constant. 0 = no GPT calls.
# Raised to 60 on 2026-11-20 (Lazlo 19:19Z: "it has 13% usage for the week left feel free to keep usingit unti i tell you",
# "if they are actually helpingthat is"). The estimate counts unpriced models at UNPRICED_USD_PER_MTOK, so it runs high.
GPT_DAILY_CAP_USD = 60.0
# USD per million tokens, used only when bench/models.json lists NO price for any model (it holds none today). Deliberately high.
UNPRICED_USD_PER_MTOK = {"input": 30.0, "output": 120.0}
LEDGER_COLUMNS = ("utc", "caller", "model", "input_tokens", "output_tokens", "est_usd", "plan_or_credits", "call_type")
CANARY_SHARE = 0.5  # the offline canary may use at most this part of what is left of the deadline, so codex keeps the rest
LEDGER_LOCK_WAIT = 10.0  # seconds to wait for the ledger's OS lock


class Refused(Exception):
    """A gate that stops the call before any model is reached. Never falls back to the API; review() returns it as an error result."""

    def __init__(self, backend_error: str):
        super().__init__(backend_error)
        self.backend_error = backend_error


def config_of(root: Path) -> dict:
    try:
        return tomllib.loads((root / oc.CONFIG_FILE).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise oc.Fail(f"cannot read {oc.CONFIG_FILE}: {type(e).__name__}")


def state_dir_of(root: Path) -> str:
    v = config_of(root).get("state_dir")
    if not isinstance(v, str) or not v:
        raise oc.Fail(f"{oc.CONFIG_FILE} has no `state_dir`")
    return v


def daily_cap(root: Path) -> float:
    cfg = config_of(root)
    if "gpt_daily_cap_usd" not in cfg:
        return GPT_DAILY_CAP_USD
    v = cfg["gpt_daily_cap_usd"]
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not (v == v and 0 <= v < float("inf")):
        raise oc.Fail(f"{oc.CONFIG_FILE} `gpt_daily_cap_usd` must be a number >= 0")
    return float(v)


def model_prices(root: Path, model: str) -> tuple:
    """(input, output) USD per million tokens. The model's own `price_usd_per_mtok` ({"input": x, "output": y}) in bench/models.json;
    a model without one counts at the HIGHEST input and output price listed for any model, and with no price listed anywhere at
    UNPRICED_USD_PER_MTOK. Never 0, so an unpriced model cannot be used to dodge the cap."""
    listed = {}
    try:
        models = json.loads((root / "bench" / "models.json").read_text(encoding="utf-8")).get("models") or {}
        for name, entry in models.items():
            p = entry.get("price_usd_per_mtok") if isinstance(entry, dict) else None
            if isinstance(p, dict):
                i, o = p.get("input"), p.get("output")
                if all(isinstance(x, (int, float)) and not isinstance(x, bool) and x > 0 for x in (i, o)):
                    listed[name] = (float(i), float(o))
    except (OSError, ValueError, AttributeError):
        pass
    if model in listed:
        return listed[model]
    if listed:
        return max(p[0] for p in listed.values()), max(p[1] for p in listed.values())
    return UNPRICED_USD_PER_MTOK["input"], UNPRICED_USD_PER_MTOK["output"]


class ledger_lock:
    """An OS lock on a file (released when the process dies, so a crash never leaves it stuck)."""

    def __init__(self, path: Path):
        self.path = path

    def __enter__(self):
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.f = open(self.path, "a+b")
        end = time.monotonic() + LEDGER_LOCK_WAIT
        while True:
            try:
                if os.name == "nt":
                    import msvcrt
                    self.f.seek(0)
                    msvcrt.locking(self.f.fileno(), msvcrt.LK_NBLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(self.f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except OSError:
                if time.monotonic() >= end:
                    self.f.close()
                    raise oc.Fail("the GPT spend ledger is locked by another process")
                time.sleep(0.05)

    def __exit__(self, *exc):
        try:
            if os.name == "nt":
                import msvcrt
                self.f.seek(0)
                msvcrt.locking(self.f.fileno(), msvcrt.LK_UNLCK, 1)
        finally:
            self.f.close()


def utc_today() -> str:
    return datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")


def spend_dir(root: Path) -> Path:
    return root / state_dir_of(root) / "gpt-spend"


def spent_today(ledger: Path) -> float:
    """Sum of est_usd in today's ledger. A row that cannot be read is an error (fail closed), never skipped."""
    if not ledger.is_file():
        return 0.0
    total = 0.0
    for n, line in enumerate(ledger.read_text(encoding="utf-8").split("\n"), 1):
        if not line.strip() or line.startswith("utc\t"):
            continue
        cols = line.split("\t")
        try:
            total += float(cols[5])
        except (IndexError, ValueError):
            raise oc.Fail(f"the GPT spend ledger {ledger.name} has an unreadable row (line {n})")
    return total


def cap_gate(root: Path) -> None:
    """Raises Refused when today's estimated spend has reached the cap; writes CAP-REACHED-<date>.txt once."""
    cap = daily_cap(root)
    day = utc_today()
    folder = spend_dir(root)
    if (folder / f"LEDGER-FAILED-{day}.txt").is_file():  # a paid call went unrecorded today: the sum cannot be trusted, so fail closed
        raise Refused(f"ledger_unwritable: a GPT call today could not be recorded (see LEDGER-FAILED-{day}.txt)")
    with ledger_lock(folder / f"{day}.lock"):
        total = spent_today(folder / f"{day}.tsv")
    if total < cap:
        return
    note = folder / f"CAP-REACHED-{day}.txt"
    try:
        with open(note, "x", encoding="utf-8") as f:
            f.write(f"GPT daily cap reached on {day} (UTC): estimated {total:.4f} USD of {cap:g} USD. No GPT call is made until the next UTC day "
                    f"or until gpt_daily_cap_usd is raised. Orchestrator: tell Lazlo.\n")
    except FileExistsError:
        pass
    except OSError:
        pass  # the refusal below still holds
    raise Refused(f"daily_cap_reached: {total:.4f} of {cap:g}")


def record_spend(root: Path, environ, model: str, result: dict, backend: str) -> None:
    """Append one ledger row for a call that got past the gates. Tokens come from the result (0 when the call failed)."""
    tin = tout = 0
    est = 0.0
    mu = result.get("modelUsage") if isinstance(result.get("modelUsage"), dict) else {}
    for name, v in mu.items():  # every model the result reports, each at its own price
        if isinstance(v, dict):
            i, o = int(v.get("inputTokens") or 0), int(v.get("outputTokens") or 0)
            pin, pout = model_prices(root, str(name))
            tin, tout, est = tin + i, tout + o, est + (i * pin + o * pout) / 1e6
    caller = (environ.get("OPENAI_REVIEW_CALLER") or "unlabelled").replace("\t", " ").replace("\n", " ")
    call_type = " ".join((environ.get("OPENAI_REVIEW_CALL_TYPE") or "").split()) or "unknown"  # one line, one cell, never empty
    label = ("codex-plan" if backend == "codex" else "api-credits") + ("" if not result.get("is_error") else "-failed")
    row = "\t".join([datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), caller, model, str(tin), str(tout), f"{est:.6f}", label, call_type])
    folder = spend_dir(root)
    day = utc_today()
    with ledger_lock(folder / f"{day}.lock"):
        ledger = folder / f"{day}.tsv"
        new = not ledger.is_file() or ledger.stat().st_size == 0
        with open(ledger, "a", encoding="utf-8", newline="\n") as f:
            if new:
                f.write("\t".join(LEDGER_COLUMNS) + "\n")
            f.write(row + "\n")
            f.flush()
            os.fsync(f.fileno())


def metered(backend: str, fn):
    """Wrap a function that makes ONE paid GPT call (args, prompt, environ, ...) in the cap check and the ledger, so every caller is covered.
    Refused (a gate stopped it before any model was reached) is passed on with no ledger row. A call that ran and failed gets a row with
    0 tokens (the real usage is unknown); a call that returns gets its reported usage. If the row cannot be written the answer is kept,
    a note says so, and LEDGER-FAILED-<date>.txt makes cap_gate refuse every later call that day (fail closed, no unrecorded spending)."""
    def inner(args, prompt, environ, *a, **k):
        root = oc.find_main_root(HERE)
        cap_gate(root)
        try:
            result = fn(args, prompt, environ, *a, **k)
        except Refused:
            raise
        except Exception:
            ledger_row(root, environ, args.model, {"is_error": True}, backend)
            raise
        note = ledger_row(root, environ, args.model, result, backend)
        if note:
            result.setdefault("notes", []).append(note)
        return result
    inner.__name__ = fn.__name__
    inner.__doc__ = fn.__doc__
    return inner


def ledger_row(root: Path, environ, model: str, result: dict, backend: str) -> str:
    """record_spend, but never raises: returns "" or a note. On a failed write, plants the fail-closed marker."""
    try:
        record_spend(root, environ, model, result, backend)
        return ""
    except Exception as e:
        try:
            folder = spend_dir(root)
            folder.mkdir(parents=True, exist_ok=True)
            (folder / f"LEDGER-FAILED-{utc_today()}.txt").write_text(f"A GPT call could not be written to the ledger ({type(e).__name__}). No GPT call is made until this file is removed.\n", encoding="utf-8")
        except Exception:
            pass
        return f"spend ledger NOT written: {oc.redact(type(e).__name__)}"


review_codex = metered("codex", review_codex)


# ---- gate 2: the canary must have passed for this codex version -------------------------------------------------------------


def canary_state_file(root: Path) -> Path:
    """The ONE canary record: <root>/<state_dir>/canary/last-pass.json, where `root` is the main Broadsword root (oc.find_main_root,
    the folder holding broadsword.toml that every other gate here uses). Never the checkout this file runs from: a worktree's own
    state/ folder is not read, even if one exists there, so every checkout sees the same record that shadow_canary.py writes."""
    return root / state_dir_of(root) / "canary" / "last-pass.json"


def read_last_pass(path: Path):
    """The codex_version recorded by the last canary PASS, or None if the file is missing, unreadable or malformed."""
    try:
        v = json.loads(path.read_text(encoding="utf-8")).get("codex_version")
    except (OSError, ValueError, AttributeError):
        return None
    return v if isinstance(v, str) and v else None


def codex_version(exe: str, child: dict, budget: float) -> str:
    try:
        with tempfile.TemporaryDirectory(prefix="bsw-codexv-") as work:
            code, out, _ = run_with_deadline([exe, "--version"], "", work, child, budget)
    except oc.Fail as e:
        raise Refused(f"codex_unavailable: {e}")
    version = (out or "").strip()
    if code != 0 or not version:
        raise Refused(f"codex_unavailable: codex --version exited {code}" + ("" if version else " with no output"))
    return version


def canary_gate(root: Path, exe: str, child: dict, deadline: float) -> None:
    """Raises Refused unless the canary has passed for the installed codex version (running it offline when it has not)."""
    version = codex_version(exe, child, min(20.0, max(deadline - time.monotonic(), 0.1)))
    state = canary_state_file(root)
    if read_last_pass(state) == version:
        return
    script = HERE.parent / "scripts" / "shadow_canary.py"
    if not script.is_file():
        raise Refused("canary_failed: scripts/shadow_canary.py not found")
    try:
        with tempfile.TemporaryDirectory(prefix="bsw-canary-") as work:
            code, out, err = run_with_deadline([sys.executable, "-I", "-B", str(script), "--offline-only"], "", work, child, (deadline - time.monotonic()) * CANARY_SHARE)
    except oc.Fail as e:
        raise Refused(f"canary_failed: {oc.redact(str(e))}"[:300])
    if code != 0:
        lines = [x.strip() for x in (out or "").split("\n") if x.strip()] or [x.strip() for x in (err or "").split("\n") if x.strip()]
        raise Refused(f"canary_failed: {oc.redact(lines[0] if lines else 'no output')} (exit {code})"[:300])
    if read_last_pass(state) != version:
        raise Refused("canary_failed: the canary exited 0 but did not record a pass for this codex version")


def review(args, prompt: str, environ) -> dict:
    timeout, share, notes = settings_of(environ)
    deadline = time.monotonic() + timeout
    root = oc.find_main_root(HERE)
    try:
        return answer_call(args, prompt, environ, timeout, share, notes, deadline)  # the cap and ledger are inside review_codex / review_api
    except Refused as e:
        return refused_result(e)


def refused_result(e: Refused) -> dict:
    result = error_result(e.backend_error)
    result["backend"] = "codex"
    result["codex_error"] = oc.redact(e.backend_error)[:400]
    return result


def answer_call(args, prompt: str, environ, timeout: float, share: float, notes: list, deadline: float) -> dict:
    try:
        result = review_codex(args, prompt, environ, deadline, timeout, share)
        if notes:
            result["notes"] = result.get("notes", []) + notes
        return result
    except Refused:
        raise
    except Exception as e:  # any codex failure falls back, and is recorded
        codex_error = oc.redact(f"{type(e).__name__}: {e}" if not isinstance(e, oc.Fail) else str(e))[:400]
    try:
        result = review_api(args, prompt, environ, deadline - time.monotonic(), timeout)
    except Refused:
        raise
    except oc.Fail as e:
        result = error_result(str(e))
    except Exception as e:
        result = error_result(f"unexpected {type(e).__name__}: {e}")
    if result["is_error"]:  # the final error must show the codex failure too, not only the API's
        result["result"] = f"codex: {codex_error}; api: {result['result']}"
    result["backend"] = "api"
    result["codex_error"] = codex_error + ("; " + "; ".join(notes) if notes else "")
    if notes:
        result["notes"] = result.get("notes", []) + notes
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


review_api = metered("api", review_api)


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
