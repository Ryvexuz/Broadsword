"""Reviewer benchmark runner, OpenAI half: one fixed diff-only review per (case, model, effort) via Chat Completions.

Usage (from bench/reviewer/, after ./make_cases.sh has regenerated the diffs):
  python -I -B run_openai.py <out-dir> <model> <effort|default> [case-id ...]

It uses the same SYSTEM prompt and user prompt as run.py (imported, not copied), so results are comparable, and
writes the files score.py reads: <case>__<model>__<effort>.json and <case>__<model>__<effort>.raw.json.

It never runs without a model name on the command line, and every run costs real money: ask Lazlo first.
The API key comes from OPENAI_API_KEY or the root .env (via clients/openai_mcp.py) and is never printed or written.
"""
import argparse, importlib.util, json, os, pathlib, re, socket, sys, time, urllib.error, urllib.parse, urllib.request

HERE = pathlib.Path(__file__).resolve().parent
sys.dont_write_bytecode = True


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


run_mod = _load("bench_reviewer_run", HERE / "run.py")
oc = _load("openai_mcp", HERE.parent.parent / "clients" / "openai_mcp.py")


MODELS_FILE = HERE.parent / "models.json"  # bench/models.json: per-model API kind, token cap and tiers
MODEL_KEYS = {"api", "max_output_tokens", "service_tiers", "note"}
API_KINDS = ("chat", "responses")
TIERS = ("default", "flex", "fast")


def load_models(path=None):
    """bench/models.json as {model: spec}. Every field is required and checked; nothing is defaulted."""
    path = pathlib.Path(path or MODELS_FILE)
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        models = data["models"]
        assert isinstance(models, dict)
    except (OSError, ValueError, KeyError, AssertionError, TypeError) as e:
        sys.exit(f"cannot read the model table {path}: {type(e).__name__}")
    for name, spec in models.items():
        ok = (isinstance(spec, dict) and {"api", "max_output_tokens", "service_tiers"} <= set(spec) and set(spec) <= MODEL_KEYS
              and spec["api"] in API_KINDS and isinstance(spec["max_output_tokens"], int) and not isinstance(spec["max_output_tokens"], bool)
              and spec["max_output_tokens"] > 0 and isinstance(spec["service_tiers"], list) and spec["service_tiers"]
              and all(t in TIERS for t in spec["service_tiers"]))
        if not ok:
            sys.exit(f"{path}: the entry for {name!r} is invalid (needs api in {API_KINDS}, max_output_tokens > 0, service_tiers from {TIERS})")
    return models


def model_spec(models, model):
    if model not in models:
        sys.exit(f"model {model!r} is not in models.json; add it there (api kind, max_output_tokens, service_tiers). Known: {', '.join(sorted(models))}")
    return models[model]


def user_prompt(case, diff):
    # Identical to run.py's prompt.
    return f"{case['brief']}\n\nDIFF:\n```diff\n{diff}\n```\n\nReview it now, in the required format."


def build_payload(model, effort, prompt, max_tokens, api="chat", tier="default"):
    if api == "responses":
        payload = {"model": model, "instructions": run_mod.SYSTEM, "input": prompt, "max_output_tokens": max_tokens}
        if effort != "default":
            payload["reasoning"] = {"effort": effort}
    else:
        payload = {"model": model,
                   "messages": [{"role": "system", "content": run_mod.SYSTEM}, {"role": "user", "content": prompt}],
                   "max_completion_tokens": max_tokens}
        if effort != "default":
            payload["reasoning_effort"] = effort
    if tier != "default":
        payload["service_tier"] = tier
    return payload


def responses_call(base_url, key, payload, timeout, backoff, retries):
    """POST {base}/responses (the Responses API). Same error handling as openai_mcp.chat_completion: 429/5xx retried, redirects
    refused, messages redacted. Raises oc.Fail."""
    url = base_url.rstrip("/") + "/responses"
    data = json.dumps(payload).encode("utf-8")
    for attempt in range(retries + 1):
        req = urllib.request.Request(url, data=data, method="POST", headers={
            "Content-Type": "application/json", "Authorization": "Bearer " + key, "User-Agent": oc.CLIENT_NAME})
        try:
            with oc._OPENER.open(req, timeout=timeout) as resp:
                raw = resp.read()
        except urllib.error.HTTPError as e:
            try:
                body = e.read()
            except (OSError, ValueError):
                body = b""
            st = e.code
            if st == 429 or st >= 500:
                if attempt < retries:
                    wait = oc.retry_delay(e.headers, attempt, backoff)
                    oc.err(f"HTTP {st} from the API; retrying in {wait:g} s ({attempt + 1}/{retries})")
                    time.sleep(wait)
                    continue
                raise oc.Fail(f"HTTP {st}: still failing after {retries} retries. API message: {oc.api_message(body)}")
            if 300 <= st < 400:
                raise oc.Fail(f"HTTP {st}: the API answered with a redirect, which is not followed. Check --base-url.")
            if st == 401:
                raise oc.Fail("HTTP 401: the API rejected the key. Check OPENAI_API_KEY (or the line in .env).")
            raise oc.Fail(f"HTTP {st}: the API refused the request. API message: {oc.api_message(body)}")
        except (TimeoutError, socket.timeout):
            raise oc.Fail(f"the API did not answer within {timeout:g} s (--timeout)")
        except urllib.error.URLError as e:
            if isinstance(e.reason, (TimeoutError, socket.timeout)):
                raise oc.Fail(f"the API did not answer within {timeout:g} s (--timeout)")
            raise oc.Fail(f"cannot reach {oc.redact(url)}: {oc.redact(e.reason)}")
        except OSError as e:
            raise oc.Fail(f"network error talking to the API: {oc.redact(e)}")
        try:
            result = json.loads(raw.decode("utf-8", errors="replace"))
        except ValueError:
            raise oc.Fail("the API returned something that is not valid JSON")
        if not isinstance(result, dict) or not isinstance(result.get("output"), list):
            raise oc.Fail("the API response has no `output` list; it is not a Responses API response")
        return result
    raise oc.Fail("unreachable")


def parse_responses(raw):
    """(text, error) from a Responses API body: the output_text parts of every message item, joined."""
    status = raw.get("status")
    usage = json.dumps(raw.get("usage") or {}, sort_keys=True)
    parts, refusal = [], None
    for item in raw["output"]:
        if not isinstance(item, dict) or item.get("type") != "message":
            continue
        for c in item.get("content") or []:
            if isinstance(c, dict) and c.get("type") == "output_text" and isinstance(c.get("text"), str):
                parts.append(c["text"])
            elif isinstance(c, dict) and c.get("type") == "refusal":
                refusal = c.get("refusal")
    text = "".join(parts)
    if status != "completed":
        det = raw.get("incomplete_details")
        why = det.get("reason") if isinstance(det, dict) else None
        e = f"incomplete answer: status={status!r}, reason={why!r}, usage={usage}"
        if why == "max_output_tokens":
            e += " (the reasoning probably used up max_output_tokens; raise --max-tokens)"
        return "", e
    if not text.strip():
        return "", (f"the model refused: {refusal}" if refusal else f"empty answer: status='completed', usage={usage}")
    return text, None


def normalize_usage(u):
    """Keep the API's fields and add the input_tokens/output_tokens names score.py reads."""
    if not isinstance(u, dict):
        return None
    u = dict(u)
    u["input_tokens"] = u.get("input_tokens") or u.get("prompt_tokens") or 0
    u["output_tokens"] = u.get("output_tokens") or u.get("completion_tokens") or 0
    return u


def cost_usd(usage, price_in, price_out):
    if price_in is None or price_out is None or usage is None:
        return None
    return round(usage["input_tokens"] * price_in / 1e6 + usage["output_tokens"] * price_out / 1e6, 6)


def find_key(root):
    """Environment first, then <root>/.env (same as openai_mcp.py). Warns, without values, when both exist and differ."""
    env_key = (os.environ.get("OPENAI_API_KEY") or "").strip()
    env_file = root / ".env"
    if env_key and env_file.is_file():
        try:
            file_key = oc.parse_env_text(env_file.read_text(encoding="utf-8", errors="replace")).get("OPENAI_API_KEY", "").strip()
        except OSError:
            file_key = ""
        if file_key and file_key != env_key:
            oc.err("warning: OPENAI_API_KEY in the environment differs from the one in .env; using the environment one")
    return oc.load_api_key(root)


def parser():
    p = argparse.ArgumentParser(
        description="OpenAI half of the reviewer benchmark. Costs real money: only run it for a model Lazlo named.")
    p.add_argument("out_dir", help="folder for the result files (created if missing)")
    p.add_argument("model", help="model name, sent as-is")
    p.add_argument("effort", help="reasoning_effort value to send (e.g. low, medium, high), or 'default' to omit it")
    p.add_argument("cases", nargs="*", help="case ids to run (default: all in cases.json)")
    p.add_argument("--base-url", default=oc.DEFAULT_BASE_URL, help="API base URL (default: %(default)s)")
    p.add_argument("--root", help="folder holding .env (default: the main checkout of this repo)")
    p.add_argument("--timeout", type=float, default=600.0, help="seconds to wait per request (default: %(default)s)")
    p.add_argument("--max-tokens", type=int, default=16000,
                   help="max output tokens per request, reasoning included; lowered to the model's max_output_tokens in models.json (default: %(default)s)")
    p.add_argument("--service-tier", default="default", choices=TIERS,
                   help="service tier; must be one the model accepts in models.json (default: %(default)s = none sent)")
    p.add_argument("--retries", type=int, default=2, help="retries on 429 and 5xx (default: %(default)s)")
    p.add_argument("--backoff", type=float, default=2.0, help="base retry wait in seconds, doubling (default: %(default)s)")
    p.add_argument("--price-in", type=float, help="USD per 1M input tokens; with --price-out, enables cost_usd (default: none)")
    p.add_argument("--price-out", type=float, help="USD per 1M output tokens (default: none)")
    return p


def safe(s):
    return re.sub(r"[^A-Za-z0-9._-]", "_", s)


def main(argv=None):
    args = parser().parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    if not args.model.strip() or not args.effort.strip():
        sys.exit("model and effort must not be empty")
    if args.max_tokens < 1 or args.timeout <= 0 or args.retries < 0:
        sys.exit("--max-tokens must be >= 1, --timeout > 0, --retries >= 0")
    if (args.price_in is None) != (args.price_out is None):
        sys.exit("give both --price-in and --price-out, or neither")
    u = urllib.parse.urlparse(args.base_url)
    if u.scheme not in ("http", "https") or not u.hostname:
        sys.exit("--base-url must be an http(s) URL")
    if u.scheme == "http" and u.hostname not in ("127.0.0.1", "localhost", "::1"):
        sys.exit("refusing to send the API key over plain http to a non-local host")
    spec = model_spec(load_models(), args.model)
    if args.service_tier not in spec["service_tiers"]:
        sys.exit(f"model {args.model} does not accept service tier {args.service_tier!r} (models.json lists: {', '.join(spec['service_tiers'])})")
    max_tokens = min(args.max_tokens, spec["max_output_tokens"])
    if max_tokens < args.max_tokens:
        print(f"note: --max-tokens {args.max_tokens} lowered to {max_tokens}, the cap for {args.model} in models.json", file=sys.stderr)
    cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
    known = {c["id"] for c in cases}
    unknown = [c for c in args.cases if c not in known]
    if unknown:
        sys.exit(f"unknown case id(s): {', '.join(unknown)}. Known: {', '.join(sorted(known))}")
    root = pathlib.Path(args.root) if args.root else oc.find_main_root(HERE)
    try:
        key = find_key(root)
    except oc.Fail as e:
        sys.exit(f"error: {e}")
    if args.price_in is None:
        print("note: no --price-in/--price-out given, so cost_usd is null (prices are never guessed)", file=sys.stderr)
    want = set(args.cases)
    todo = [c for c in cases if not want or c["id"] in want]
    diffs = {}
    for c in todo:  # every input is checked before the first paid call
        try:
            diffs[c["id"]] = (HERE / c["diff"]).read_text(encoding="utf-8")
        except OSError as e:
            sys.exit(f"case {c['id']}: cannot read its diff {c['diff']} ({type(e).__name__}); run make_cases.sh first")
    out = pathlib.Path(args.out_dir).resolve()
    out.mkdir(parents=True, exist_ok=True)
    failures = 0
    for c in todo:
        payload = build_payload(args.model, args.effort, user_prompt(c, diffs[c["id"]]), max_tokens, spec["api"], args.service_tier)
        tag = f"{c['id']}__{safe(args.model)}__{safe(args.effort)}"
        t0 = time.time()
        raw, error = None, None
        try:
            if spec["api"] == "responses":
                raw = responses_call(args.base_url, key, payload, args.timeout, args.backoff, args.retries)
            else:
                raw = oc.chat_completion(args.base_url, key, payload, args.timeout, args.backoff, args.retries)
        except oc.Fail as e:
            error = str(e)
        except Exception as e:
            error = f"unexpected {type(e).__name__}: {oc.redact(e)}"[:500]
        dt = time.time() - t0
        usage = normalize_usage((raw or {}).get("usage"))
        text = ""
        if raw is not None and spec["api"] == "responses":
            text, error = parse_responses(raw)
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
        rec_error = bool(error) or raw is None
        rec = {"case": c["id"], "model": args.model, "effort": args.effort, "wall_s": round(dt, 1),
               "exit": 1 if rec_error else 0, "cost_usd": cost_usd(usage, args.price_in, args.price_out),
               "usage": usage, "result": text, "is_error": rec_error,
               "backend": "openai-" + spec["api"], "max_tokens_sent": max_tokens,
               "model_used": [raw["model"]] if raw and isinstance(raw.get("model"), str) else []}
        if error:
            rec["error"] = oc.redact(error)
        (out / f"{tag}.raw.json").write_text(oc.redact(json.dumps(raw if raw is not None else {"error": rec.get("error")}, indent=1)), encoding="utf-8")
        (out / f"{tag}.json").write_text(oc.redact(json.dumps(rec, indent=1)), encoding="utf-8")
        print(f"{tag}: exit={rec['exit']} {dt:.0f}s cost={rec['cost_usd']}" + (f" ERROR: {rec['error']}" if error else ""), flush=True)
        failures += rec_error
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
