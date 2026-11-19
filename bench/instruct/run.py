"""Instruction-following benchmark runner: one model call per (case, model, effort). No new provider code:
the Claude CLI call and the OpenAI call are imported from bench/coder/run.py (which reuses bench/reviewer).

Usage:
  BENCH_CWD=<folder outside the project> [BENCH_CLAUDE=<path to claude cli>] \
    python -I -B bench/instruct/run.py <out-dir> <provider> <model> <effort|default> [case-id ...] [openai options]

  provider  claude | openai, exactly as in bench/coder/run.py (same BENCH_CWD rules, same openai options:
            --price-in/--price-out/--max-tokens/--timeout/--retries/--backoff/--base-url/--root).
            Every real run costs money; OpenAI also needs the key: ask Lazlo first.
Per case it writes <out-dir>/<case>__<provider>__<model>__<effort>.{raw.json,answer.txt,json}. Then: python -I -B score.py <out-dir>
A failed call (refusal, timeout, CLI error) is recorded with call_failed true (is_error, a non-zero exit or an error subtype, whatever text came back); score.py excludes it.
The Claude and OpenAI calls are imported from bench/coder/run.py on purpose: one provider path for every bench.
"""
import importlib.util, json, pathlib, re, sys, time, tomllib

HERE = pathlib.Path(__file__).resolve().parent
CFG = tomllib.loads((HERE / "config.toml").read_text(encoding="utf-8"))
_spec = importlib.util.spec_from_file_location("coder_run_for_instruct", HERE.parent / "coder" / "run.py")
CR = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(CR)

# the system prompt is deliberately bare: the rules are in each case's prompt
SYSTEM = "You are a helpful assistant. Follow the user's instructions exactly."
CR.SYSTEM = SYSTEM  # provider_claude reads this module global; the openai call takes it as an argument
CR.CFG["model_timeout_seconds"] = CFG["model_timeout_seconds"]


def safe(s):
    return re.sub(r"[^A-Za-z0-9._-]", "_", s)


def call_failed(res):
    """The ONE place that decides a call failed (so it is an ERROR in score.py, never a FAIL), whatever text came back:
    is_error true, a non-zero exit code, or a CLI result whose subtype starts with 'error'. A timeout has is_error true."""
    if res.get("is_error"):
        return True
    if res.get("exit") not in (0, None):
        return True
    try:
        raw = json.loads(res["raw"])
    except Exception:
        return False  # openai raw is saved as JSON too; an unreadable raw is not by itself a failed call
    return isinstance(raw, dict) and str(raw.get("subtype", "")).startswith("error")


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    argv, openai_opts = CR.OP.split_options(argv)
    if len(argv) < 4:
        sys.exit(__doc__)
    out = pathlib.Path(argv[0]); provider, model, effort = argv[1:4]
    if provider not in CR.PROVIDERS:
        sys.exit(f"provider must be one of {sorted(CR.PROVIDERS)}")
    if openai_opts and provider != "openai":
        sys.exit("the openai options are for provider openai only")
    CR.check_workdir()
    if provider == "openai":
        CR.OP.prepare(model, effort, openai_opts, CFG["model_timeout_seconds"])
    cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
    want = argv[4:]
    unknown = set(want) - {c["id"] for c in cases}
    if unknown:
        sys.exit(f"unknown cases: {sorted(unknown)}")
    out.mkdir(parents=True, exist_ok=True)
    for c in cases:
        if want and c["id"] not in want:
            continue
        tag = f"{c['id']}__{provider}__{safe(model)}__{safe(effort)}"
        t0 = time.time()
        res = CR.PROVIDERS[provider](c["prompt"], model, effort)
        dt = time.time() - t0
        (out / f"{tag}.raw.json").write_bytes(res["raw"])
        (out / f"{tag}.answer.txt").write_text(res["answer"], encoding="utf-8")
        failed = call_failed(res)
        rec = {"provider": provider, "model": model, "effort": effort, "case": c["id"], "answer": res["answer"],
               "tokens": res["tokens"], "cost_usd": res["cost_usd"], "wall_s": round(dt, 1), "exit": res["exit"],
               "is_error": res.get("is_error"), "call_failed": failed, "error": res.get("error"),
               "model_used": res.get("model_used", [])}
        (out / f"{tag}.json").write_text(json.dumps(rec, indent=1), encoding="utf-8")
        print(f"{tag}: {rec['wall_s']}s cost={rec['cost_usd']} call_failed={failed}", flush=True)


if __name__ == "__main__":
    main()
