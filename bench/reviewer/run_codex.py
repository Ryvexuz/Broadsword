"""Reviewer benchmark runner, Codex half: one fixed diff-only review per (case, model) through the Codex CLI (ChatGPT plan).

Usage (from bench/reviewer/, after ./make_cases.sh has regenerated the diffs):
  python -I -B run_codex.py <out-dir> <model> default [case-id ...]

Same arguments, prompts and result files as run_openai.py (<case>__<model>__<effort>.json and .raw.json, read by score.py), with
"backend": "codex" and cost_usd null (the ChatGPT plan has no per-call price).

SECURITY: the reviewed diff is untrusted text, and Codex reads any file it is offered a tool for. This runner never writes codex flags:
it calls clients/openai_review.py's review_codex(), so the command is codex_command() (one-entry model catalog, NO tools offered;
docs/SHADOW_SANDBOX.md), the child environment is codex_env() (API key and every copy of a .env value dropped), and an answer whose
event stream shows any tool item or unknown event is an error. There is no API fallback: a codex failure is recorded as is.

Effort: only `default` is accepted. The no-tools catalog lists no reasoning levels, so codex would ignore or refuse a level, and a
result labelled "high" that was not run at "high" would be wrong.

Every run uses your ChatGPT plan's quota: ask Lazlo first.
"""
import argparse, importlib.util, json, os, pathlib, re, sys, time

HERE = pathlib.Path(__file__).resolve().parent
sys.dont_write_bytecode = True


def _load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


run_mod = _load("bench_reviewer_run", HERE / "run.py")
review = _load("openai_review", HERE.parent.parent / "clients" / "openai_review.py")
oc = review.oc
ro = _load("bench_reviewer_run_openai", HERE / "run_openai.py")

def parser():
    p = argparse.ArgumentParser(description="Codex half of the reviewer benchmark (ChatGPT plan, no tools offered).")
    p.add_argument("out_dir", help="folder for the result files (created if missing)")
    p.add_argument("model", help="model name, sent as-is")
    p.add_argument("effort", help="must be 'default' (see the module doc)")
    p.add_argument("cases", nargs="*", help="case ids to run (default: all in cases.json)")
    p.add_argument("--timeout", type=float, default=600.0, help="seconds codex may take per case (default: %(default)s)")
    for flag in ("--base-url", "--max-tokens", "--retries", "--backoff", "--price-in", "--price-out", "--service-tier", "--root"):
        p.add_argument(flag, help="accepted for compatibility with run_openai.py; not used by codex (an error if given)")
    return p


def main(argv=None, environ=None):
    environ = os.environ if environ is None else environ
    args = parser().parse_args(argv)
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except (AttributeError, ValueError):
            pass
    given = [f for f, a in (("--base-url", args.base_url), ("--max-tokens", args.max_tokens), ("--retries", args.retries),
                            ("--backoff", args.backoff), ("--price-in", args.price_in), ("--price-out", args.price_out),
                            ("--service-tier", args.service_tier), ("--root", args.root)) if a is not None]
    if given:
        sys.exit(f"codex does not use {', '.join(given)}; leave them out")
    if not args.model.strip():
        sys.exit("model must not be empty")
    if args.effort != "default":
        sys.exit("codex runs only with effort 'default' (the no-tools catalog lists no reasoning levels)")
    if args.timeout <= 0:
        sys.exit("--timeout must be > 0")
    if review.find_codex(environ) is None:
        sys.exit("codex executable not found on PATH or under APPDATA/npm")
    cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
    known = {c["id"] for c in cases}
    unknown = [c for c in args.cases if c not in known]
    if unknown:
        sys.exit(f"unknown case id(s): {', '.join(unknown)}. Known: {', '.join(sorted(known))}")
    want = set(args.cases)
    todo = [c for c in cases if not want or c["id"] in want]
    diffs = {}
    for c in todo:  # every input is checked before the first call
        try:
            diffs[c["id"]] = (HERE / c["diff"]).read_text(encoding="utf-8")
        except OSError as e:
            sys.exit(f"case {c['id']}: cannot read its diff {c['diff']} ({type(e).__name__}); run make_cases.sh first")
    out = pathlib.Path(args.out_dir).resolve()
    out.mkdir(parents=True, exist_ok=True)
    cargs = argparse.Namespace(model=args.model, system_prompt=run_mod.SYSTEM)
    failures = 0
    for c in todo:
        prompt = ro.user_prompt(c, diffs[c["id"]])
        tag = f"{c['id']}__{ro.safe(args.model)}__default"
        t0 = time.time()
        raw, error = None, None
        deadline = time.monotonic() + args.timeout
        try:
            raw = review.review_codex(cargs, prompt, environ, deadline, args.timeout, 1.0)
        except oc.Fail as e:
            error = str(e)
        except Exception as e:
            error = f"unexpected {type(e).__name__}: {oc.redact(e)}"[:500]
        dt = time.time() - t0
        usage = ro.normalize_usage((raw or {}).get("usage"))
        rec = {"case": c["id"], "model": args.model, "effort": "default", "wall_s": round(dt, 1),
               "exit": 1 if error else 0, "cost_usd": None, "usage": usage,
               "result": (raw or {}).get("result") or "", "is_error": bool(error), "model_used": [],
               "backend": "codex", "codex_items": (raw or {}).get("codex_items", [])}
        if error:
            rec["error"] = oc.redact(error)
        (out / f"{tag}.raw.json").write_text(oc.redact(json.dumps(raw if raw is not None else {"error": rec.get("error")}, indent=1)), encoding="utf-8")
        (out / f"{tag}.json").write_text(oc.redact(json.dumps(rec, indent=1)), encoding="utf-8")
        print(f"{tag}: exit={rec['exit']} {dt:.0f}s cost=None" + (f" ERROR: {rec['error']}" if error else ""), flush=True)
        failures += bool(error)
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
