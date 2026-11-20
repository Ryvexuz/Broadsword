"""OpenAI and Codex providers for bench/coder/run.py: one request (or one codex run) per (task, model, effort).

run.py loads this file by path (python -I drops the script folder from sys.path). The request, key and error logic is
NOT copied: it is imported from bench/reviewer/run_openai.py (which in turn uses clients/openai_mcp.py): key from
OPENAI_API_KEY, then the root .env; no redirects; bounded retries on 429/5xx; the key is redacted from everything written.
If run_openai.py is not in this checkout yet, the main checkout's copy is used (found with openai_mcp.find_main_root).

provider openai: the model must be a row of bench/models.json (a model not listed is an error, never a default). Its `api`
picks the endpoint: `chat` = Chat Completions, `responses` = the Responses API (the pro and codex models refuse Chat Completions).
--max-tokens is lowered to the row's max_output_tokens (with a note); --service-tier must be one the row lists.

provider codex: one run of the Codex CLI (ChatGPT plan quota, no per-call price, cost_usd null), through
clients/openai_review.py's review_codex(), so the command is codex_command() (NO tool offered, docs/SHADOW_SANDBOX.md), the child
environment drops the API key and every .env value, and a tool item in the event stream makes the answer an error. This file
writes no codex flag of its own. Only effort `default`; no API option is accepted; no API fallback. Codex does not report which
model answered, so such a result has model_used [] and model_reported false: the run proves no model name.

Options (given after the positional arguments of run.py, openai provider only):
  --price-in/--price-out USD per 1M tokens (cost_usd stays null without both), --max-tokens, --timeout, --retries,
  --backoff, --base-url, --root, --service-tier. Same names and defaults as run_openai.py; its parser does the parsing.
Every real run costs money or plan quota: only for a model Lazlo named. Tests use a fake server on 127.0.0.1 and a fake codex.
"""
import argparse, importlib.util, json, os, pathlib, sys, time

HERE = pathlib.Path(__file__).resolve().parent
sys.dont_write_bytecode = True
# every option run_openai's parser takes, and whether it takes a value
OPTIONS = {"--base-url": True, "--root": True, "--timeout": True, "--max-tokens": True, "--retries": True,
           "--backoff": True, "--price-in": True, "--price-out": True, "--service-tier": True}
_state = {}


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


def _ro():
    """run_openai.py as a module (cached)."""
    if "ro" not in _state:
        local = HERE.parent / "reviewer" / "run_openai.py"
        if not local.is_file():
            boot = _load("openai_mcp_boot", HERE.parent.parent / "clients" / "openai_mcp.py")
            try:
                main_root = boot.find_main_root(HERE)
            except boot.Fail as e:
                raise RuntimeError(f"run_openai.py not found and no main checkout: {e}")
            local = main_root / "bench" / "reviewer" / "run_openai.py"
            if not local.is_file():
                raise RuntimeError(f"run_openai.py not found at {local}")
        _state["ro"] = _load("bench_reviewer_run_openai", local)
    return _state["ro"]


def _review():
    """clients/openai_review.py as a module (cached): review_codex() and codex_command() live there."""
    if "review" not in _state:
        _state["review"] = _load("openai_review", HERE.parent.parent / "clients" / "openai_review.py")
    return _state["review"]


def split_options(argv):
    """Pull the OpenAI options out of run.py's argv. Returns (remaining argv, option list)."""
    rest, opts, i = [], [], 0
    while i < len(argv):
        a = argv[i]
        name = a.split("=", 1)[0]
        if name in OPTIONS:
            if "=" in a:
                opts.append(a)
            elif i + 1 < len(argv):
                opts += [a, argv[i + 1]]
                i += 1
            else:
                raise SystemExit(f"{a} needs a value")
        else:
            rest.append(a)
        i += 1
    return rest, opts


def prepare(model, effort, opts, timeout_default):
    """Validate everything and find the key BEFORE any task runs. Exits with a clear message on a bad setting."""
    ro = _ro()
    oc = ro.oc
    # the positional slots only satisfy run_openai's parser; they are not used
    if "--timeout" not in " ".join(opts):
        opts = opts + ["--timeout", str(timeout_default)]
    args = ro.parser().parse_args(["-", model, effort, *opts])
    if not model.strip() or not effort.strip():
        sys.exit("model and effort must not be empty")
    if args.max_tokens < 1 or args.timeout <= 0 or args.retries < 0:
        sys.exit("--max-tokens must be >= 1, --timeout > 0, --retries >= 0")
    if (args.price_in is None) != (args.price_out is None):
        sys.exit("give both --price-in and --price-out, or neither")
    from urllib.parse import urlparse
    u = urlparse(args.base_url)
    if u.scheme not in ("http", "https") or not u.hostname:
        sys.exit("--base-url must be an http(s) URL")
    if u.scheme == "http" and u.hostname not in ("127.0.0.1", "localhost", "::1"):
        sys.exit("refusing to send the API key over plain http to a non-local host")
    spec = ro.model_spec(ro.load_models(), model)  # bench/models.json; a model not listed exits here
    if args.service_tier not in spec["service_tiers"]:
        sys.exit(f"model {model} does not accept service tier {args.service_tier!r} (models.json lists: {', '.join(spec['service_tiers'])})")
    max_tokens = min(args.max_tokens, spec["max_output_tokens"])
    if max_tokens < args.max_tokens:
        print(f"note: --max-tokens {args.max_tokens} lowered to {max_tokens}, the cap for {model} in models.json", file=sys.stderr)
    root = pathlib.Path(args.root) if args.root else oc.find_main_root(HERE)
    try:
        key = ro.find_key(root)
    except oc.Fail as e:
        sys.exit(f"error: {e}")
    if args.price_in is None:
        print("note: no --price-in/--price-out given, so cost_usd is null (prices are never guessed)", file=sys.stderr)
    _state["cfg"] = (args, key, spec, max_tokens)


def build_payload(system, prompt, model, effort, max_tokens, api="chat", tier="default"):
    """Same payload shapes as run_openai.build_payload, with the coder prompts."""
    if api == "responses":
        p = {"model": model, "instructions": system, "input": prompt, "max_output_tokens": max_tokens}
        if effort != "default":
            p["reasoning"] = {"effort": effort}
    else:
        p = {"model": model, "messages": [{"role": "system", "content": system}, {"role": "user", "content": prompt}],
             "max_completion_tokens": max_tokens}
        if effort != "default":
            p["reasoning_effort"] = effort
    if tier != "default":
        p["service_tier"] = tier
    return p


def call(system, prompt, model, effort):
    """Returns the dict run.py expects: answer, raw (bytes), tokens, cost_usd, exit, is_error, error, model_used, model_reported, backend."""
    ro = _ro()
    oc = ro.oc
    args, key, spec, max_tokens = _state["cfg"]
    api = spec["api"]
    payload = build_payload(system, prompt, model, effort, max_tokens, api, args.service_tier)
    raw, error = None, None
    try:
        if api == "responses":
            raw = ro.responses_call(args.base_url, key, payload, args.timeout, args.backoff, args.retries)
        else:
            raw = oc.chat_completion(args.base_url, key, payload, args.timeout, args.backoff, args.retries)
    except oc.Fail as e:
        error = str(e)
    except Exception as e:
        error = f"unexpected {type(e).__name__}: {oc.redact(e)}"[:500]
    usage = ro.normalize_usage((raw or {}).get("usage"))
    text = ""
    if raw is not None and api == "responses":
        text, error = ro.parse_responses(raw)
    elif raw is not None:
        try:
            choice = raw["choices"][0]
            msg = choice["message"]
            text = msg.get("content") or ""
            finish = choice.get("finish_reason")
            refusal = msg.get("refusal")
        except (KeyError, IndexError, TypeError, AttributeError) as e:
            text = ""
            error = f"malformed response, no choices[0].message ({type(e).__name__})"
        else:
            if not isinstance(text, str):
                text = ""
                error = "malformed response: message content is not text"
            elif not text.strip() and refusal:
                text = ""
                error = f"the model refused: {refusal}"
            elif finish != "stop" or not text.strip():
                why = "empty answer" if not text.strip() else "incomplete answer"
                error = (f"{why}: finish_reason={finish!r}, usage={json.dumps((raw.get('usage') or {}), sort_keys=True)}"
                         + (" (the reasoning probably used up max_completion_tokens; raise --max-tokens)" if finish == "length" else ""))
    failed = bool(error) or raw is None
    if failed:
        text = ""  # an incomplete answer is an error, never graded; the raw file keeps whatever came back
    u = usage or {}
    details = u.get("input_tokens_details") if api == "responses" else u.get("prompt_tokens_details")
    cached = details.get("cached_tokens") if isinstance(details, dict) else None
    tokens = {"input": u.get("input_tokens"), "output": u.get("output_tokens"), "cache_read": cached, "cache_write": None}
    body = raw if raw is not None else {"error": oc.redact(error)}
    model_used = [raw["model"]] if raw and isinstance(raw.get("model"), str) else []
    return {"answer": text, "raw": oc.redact(json.dumps(body, indent=1)).encode("utf-8"), "tokens": tokens,
            "cost_usd": ro.cost_usd(usage, args.price_in, args.price_out), "exit": 1 if failed else 0,
            "is_error": failed, "error": oc.redact(error) if error else None,
            "model_used": model_used, "model_reported": bool(model_used), "backend": "openai-" + api}


def prepare_codex(model, effort, timeout):
    """Codex provider checks, BEFORE any task runs: model and effort, codex on PATH. Exits with a clear message."""
    review = _review()
    if not model.strip():
        sys.exit("model must not be empty")
    if effort != "default":
        sys.exit("codex runs only with effort 'default' (the no-tools catalog lists no reasoning levels)")
    if timeout <= 0:
        sys.exit("the model timeout must be > 0")
    if review.find_codex(os.environ) is None:
        sys.exit("codex executable not found on PATH or under APPDATA/npm")
    _state["codex"] = timeout


def call_codex(system, prompt, model, effort):
    """One codex run through review_codex() (no codex flag is built here). Same dict as call(); cost null; model_reported only when review_codex read a model name from the codex events."""
    review = _review()
    oc = review.oc
    timeout = _state["codex"]
    cargs = argparse.Namespace(model=model, system_prompt=system)
    raw, error = None, None
    try:
        raw = review.review_codex(cargs, prompt, os.environ, time.monotonic() + timeout, timeout, 1.0)
    except oc.Fail as e:
        error = str(e)
    except Exception as e:
        error = f"unexpected {type(e).__name__}: {oc.redact(e)}"[:500]
    text = (raw or {}).get("result") or ""
    if not isinstance(text, str) or not text.strip():
        text = ""
        error = error or "codex answer is empty"
    failed = bool(error)
    if failed:
        text = ""
    u = (raw or {}).get("usage")
    u = u if isinstance(u, dict) else {}
    cached = u.get("cached_input_tokens")
    tokens = {"input": u.get("input_tokens"), "output": u.get("output_tokens"),
              "cache_read": cached if isinstance(cached, int) else None, "cache_write": None}
    body = raw if raw is not None else {"error": oc.redact(error)}
    # model_used / model_reported come from review_codex (codex_model_used reads the codex events); modelUsage is keyed by the
    # model we ASKED for and proves nothing, so it is not read here. On codex-cli 0.170.0 no event names a model: reported False.
    used = (raw or {}).get("model_used")
    used = [m for m in used if isinstance(m, str) and m] if isinstance(used, list) else []
    reported = bool(used) and bool((raw or {}).get("model_reported")) and not failed
    return {"answer": text, "raw": oc.redact(json.dumps(body, indent=1)).encode("utf-8"), "tokens": tokens,
            "cost_usd": None, "exit": 1 if failed else 0, "is_error": failed,
            "error": oc.redact(error) if error else None, "model_used": used if reported else [], "model_reported": reported, "backend": "codex"}
