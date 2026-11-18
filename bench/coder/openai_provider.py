"""OpenAI provider for bench/coder/run.py: one Chat Completions request per (task, model, effort).

run.py loads this file by path (python -I drops the script folder from sys.path). The request, key and error logic is
NOT copied: it is imported from bench/reviewer/run_openai.py (which in turn uses clients/openai_mcp.py): key from
OPENAI_API_KEY, then the root .env; no redirects; bounded retries on 429/5xx; the key is redacted from everything written.
If run_openai.py is not in this checkout yet, the main checkout's copy is used (found with openai_mcp.find_main_root).

Options (given after the positional arguments of run.py, openai provider only):
  --price-in/--price-out USD per 1M tokens (cost_usd stays null without both), --max-tokens, --timeout, --retries,
  --backoff, --base-url, --root. Same names and defaults as run_openai.py; its parser does the parsing.
Every real run costs money: only for a model Lazlo named. Tests use a fake server on 127.0.0.1.
"""
import importlib.util, json, pathlib, sys

HERE = pathlib.Path(__file__).resolve().parent
sys.dont_write_bytecode = True
# every option run_openai's parser takes, and whether it takes a value
OPTIONS = {"--base-url": True, "--root": True, "--timeout": True, "--max-tokens": True, "--retries": True,
           "--backoff": True, "--price-in": True, "--price-out": True}
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
    root = pathlib.Path(args.root) if args.root else oc.find_main_root(HERE)
    try:
        key = ro.find_key(root)
    except oc.Fail as e:
        sys.exit(f"error: {e}")
    if args.price_in is None:
        print("note: no --price-in/--price-out given, so cost_usd is null (prices are never guessed)", file=sys.stderr)
    _state["cfg"] = (args, key)


def build_payload(system, prompt, model, effort, max_tokens):
    """Same payload shape as run_openai.build_payload, with the coder prompts."""
    p = {"model": model, "messages": [{"role": "system", "content": system}, {"role": "user", "content": prompt}],
         "max_completion_tokens": max_tokens}
    if effort != "default":
        p["reasoning_effort"] = effort
    return p


def call(system, prompt, model, effort):
    """Returns the dict run.py expects: answer, raw (bytes), tokens, cost_usd, exit, is_error, error, model_used."""
    ro = _ro()
    oc = ro.oc
    args, key = _state["cfg"]
    payload = build_payload(system, prompt, model, effort, args.max_tokens)
    raw, error = None, None
    try:
        raw = oc.chat_completion(args.base_url, key, payload, args.timeout, args.backoff, args.retries)
    except oc.Fail as e:
        error = str(e)
    except Exception as e:
        error = f"unexpected {type(e).__name__}: {oc.redact(e)}"[:500]
    usage = ro.normalize_usage((raw or {}).get("usage"))
    text = ""
    if raw is not None:
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
    details = u.get("prompt_tokens_details") or {}
    tokens = {"input": u.get("input_tokens"), "output": u.get("output_tokens"),
              "cache_read": details.get("cached_tokens") if isinstance(details, dict) else None, "cache_write": None}
    body = raw if raw is not None else {"error": oc.redact(error)}
    return {"answer": text, "raw": oc.redact(json.dumps(body, indent=1)).encode("utf-8"), "tokens": tokens,
            "cost_usd": ro.cost_usd(usage, args.price_in, args.price_out), "exit": 1 if failed else 0,
            "is_error": failed, "error": oc.redact(error) if error else None,
            "model_used": [raw["model"]] if raw and isinstance(raw.get("model"), str) else []}
