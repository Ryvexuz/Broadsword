"""Coding benchmark runner (one-shot track): one model call per (task, model, effort), then the hidden tests.

Usage (from anywhere):
  BENCH_CWD=<folder outside the project> [BENCH_CLAUDE=<path to claude cli>] \
    python -I -B run.py <out-dir> <provider> <model> <effort|default> [task ...] [openai options]

  provider  claude  the headless Claude Code CLI, run like bench/reviewer/run.py: cwd = BENCH_CWD, no tools,
                    no MCP, prompt on stdin, JSON output
            openai  one Chat Completions request (see openai_provider.py); key from OPENAI_API_KEY or the root .env.
                    Options: --price-in/--price-out (USD per 1M tokens; cost_usd is null without both), --max-tokens,
                    --timeout, --retries, --backoff, --base-url, --root. Costs real money: only for a model Lazlo named.
  --attempts N      (1 to 3, default 1) up to N tries per task: a task that does not fully pass is asked again with the
                    spec plus the names of the failing hidden tests (names only; "did not compile" / "no single code
                    block" when there are none). Attempt 1 is the one-shot result (pass@1). Round k asks only the tasks
                    that have not passed, and each round is graded as one batch. Not the same as the openai --retries
                    (HTTP retries on 429/5xx). With N = 1 the result JSON is exactly the one-shot format.
  python -I -B run.py regrade <out-dir>   grades the saved .rs files again (no model call), after a run whose
                    grading failed (its result JSONs say graded=false); updates the JSONs
  task      folder names under tasks/ (01-duration ...); none given = all six

BENCH_CWD must be OUTSIDE the project and have no CLAUDE.md, AGENTS.md or .git in it or any parent, so no
project instructions or memory reach the model. BENCH_CLAUDE may be a .py file (run with the current Python):
that is how the smoke test uses a fake CLI. The model sees only the task's spec.md: never the tests.

Per task it writes <out-dir>/<task>__<provider>__<model>__<effort>.{raw.json,answer.txt,rs,json}.
The result .json holds: provider, model, effort, task, tokens, cost_usd, wall_s (the model call),
harness_wall_s, compiles, passed, total, graded, call_failed, format_failure, first_error, model_used.
graded is false when the model call was made but grading failed; passed/compiles are then not real.
With --attempts N > 1 the JSON also holds (the fields above describe the LAST attempt): attempts_made, max_attempts,
pass_at (number of the first attempt that fully passed, null = no pass), cost_usd_total (sum over the attempts' known
costs, null if none known), cost_unknown_attempts, wall_s_total, and attempts (one object per attempt with the same
fields plus failed_tests). Attempt files: <tag>.* holds the latest, <tag>__aK.* the K-th (K >= 2).
"""
import importlib.util, json, os, pathlib, re, subprocess, sys, time, tomllib

HERE = pathlib.Path(__file__).resolve().parent
CFG = tomllib.loads((HERE / "config.toml").read_text(encoding="utf-8"))
_spec = importlib.util.spec_from_file_location("coder_harness", HERE / "harness.py")
H = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(H)
_spec = importlib.util.spec_from_file_location("coder_openai_provider", HERE / "openai_provider.py")
OP = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(OP)
CLAUDE = os.environ.get("BENCH_CLAUDE", "claude")
WORKDIR = os.environ.get("BENCH_CWD")
OPENAI_OPTS = []  # the --price-in ... options of the command line, set in main()

SYSTEM = """You are an expert Rust programmer. You are given a specification for one Rust library file.
Write that file. Follow the specification exactly: the public items, their signatures, the behaviour, the error cases and the constraints.
Answer with the complete source file in ONE fenced code block marked rust, and no other code block. Text outside the block is ignored.
Do not write a main function and do not write tests."""


def provider_claude(prompt, model, effort):
    prog = [sys.executable, "-I", "-B", CLAUDE] if CLAUDE.endswith(".py") else [CLAUDE]
    args = prog + ["-p", "--system-prompt", SYSTEM, "--model", model, "--tools", "",
                   "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"]
    if effort != "default":
        args += ["--effort", effort]
    try:
        p = subprocess.run(args, cwd=WORKDIR, input=prompt.encode("utf-8"), capture_output=True,
                           timeout=CFG["model_timeout_seconds"])
    except subprocess.TimeoutExpired:
        # a call past model_timeout_seconds is a failed call for this task; the run goes on with the next one
        err = f"model call timed out after {CFG['model_timeout_seconds']}s"
        return {"answer": "", "raw": json.dumps({"is_error": True, "error": err}).encode("utf-8"),
                "tokens": {"input": None, "output": None, "cache_read": None, "cache_write": None},
                "cost_usd": None, "exit": None, "is_error": True, "error": err, "model_used": []}
    try:
        r = json.loads(p.stdout)
    except Exception:
        r = {"result": "", "is_error": True, "error": p.stderr.decode(errors="replace")[-500:]}
    u = r.get("usage") or {}
    tokens = {"input": u.get("input_tokens"), "output": u.get("output_tokens"),
              "cache_read": u.get("cache_read_input_tokens"), "cache_write": u.get("cache_creation_input_tokens")}
    return {"answer": r.get("result") or "", "raw": p.stdout, "tokens": tokens, "cost_usd": r.get("total_cost_usd"),
            "exit": p.returncode, "is_error": r.get("is_error"), "error": r.get("error"),
            "model_used": list((r.get("modelUsage") or {}).keys())}


def provider_openai(prompt, model, effort):
    return OP.call(SYSTEM, prompt, model, effort)


PROVIDERS = {"claude": provider_claude, "openai": provider_openai}

FENCE = re.compile(r"^```[^\n]*\n(.*?)^```[ \t]*$", re.M | re.S)


def extract_code(answer):
    """Returns (code, problem). Exactly one fenced block is required."""
    blocks = FENCE.findall(answer)
    if len(blocks) == 1 and blocks[0].strip():
        return blocks[0], None
    return None, f"{len(blocks)} code blocks in the answer (exactly one is required)"


def check_workdir():
    if not WORKDIR:
        sys.exit("set BENCH_CWD to a folder outside the project")
    d = pathlib.Path(WORKDIR).resolve()
    for a in [d, *d.parents]:
        for marker in ("CLAUDE.md", "AGENTS.md", ".git"):
            if (a / marker).exists():
                sys.exit(f"BENCH_CWD {d} is inside a project ({a / marker} exists); pick a folder outside it")
    d.mkdir(parents=True, exist_ok=True)


def regrade(out):
    """Grades the saved .rs files of a results folder again, without calling a model. Updates the result JSONs
    (compiles, passed, total, harness_wall_s, first_error, graded=true); the model data stays as it was."""
    items = []
    for f in sorted(out.glob("*.json")):
        if f.name.endswith(".raw.json"):
            continue
        rec = json.loads(f.read_text(encoding="utf-8"))
        rs = f.with_suffix(".rs")
        if rs.is_file():
            items.append((f, rec, rs, H.resolve_task(rec["task"])))
    if not items:
        sys.exit(f"no result with a saved .rs file in {out}")
    results = H.grade_batch([(rs, t) for _, _, rs, t in items])
    for (f, rec, _, _), h in zip(items, results):
        rec.update(compiles=h["compiles"], passed=h["passed"], total=h["total"], harness_wall_s=h["wall_s"],
                   first_error=h["first_error"], graded=True)
        if rec.get("attempts"):  # an --attempts run: the saved .rs is the latest attempt, so its log entry is updated too
            rec["attempts"][-1].update(compiles=h["compiles"], passed=h["passed"], total=h["total"],
                                       harness_wall_s=h["wall_s"], first_error=h["first_error"], graded=True,
                                       failed_tests=h.get("failed_tests", []))
        f.write_text(json.dumps(rec, indent=1), encoding="utf-8")
        print(f"{f.stem}: compiles={rec['compiles']} passed={rec['passed']}/{rec['total']}", flush=True)


MAX_ATTEMPTS = 3  # the largest --attempts accepted


def take_attempts(argv):
    """Pulls `--attempts N` (or --attempts=N) out of argv. Returns (remaining argv, N); N is 1 when it is not given."""
    rest, n, i = [], 1, 0
    while i < len(argv):
        a = argv[i]
        if a == "--attempts" or a.startswith("--attempts="):
            v = a.split("=", 1)[1] if "=" in a else (argv[i + 1] if i + 1 < len(argv) else "")
            if "=" not in a:
                i += 1
            if not v.isdigit() or not 1 <= int(v) <= MAX_ATTEMPTS:
                sys.exit(f"--attempts must be a whole number from 1 to {MAX_ATTEMPTS}")
            n = int(v)
        else:
            rest.append(a)
        i += 1
    return rest, n


def fully_passed(rec):
    return bool(rec["graded"] and rec["compiles"] and rec["total"] and rec["passed"] == rec["total"]
                and not rec["call_failed"] and not rec["format_failure"])


def retry_prompt(base, rec):
    """The prompt for the next attempt: the spec again plus WHAT failed, never why. Only failing test names are fed
    back (no compiler text, no assertion output, no hidden test code)."""
    if rec["call_failed"]:
        return base  # nothing to say about a call that gave no answer
    if rec["format_failure"]:
        note = "Your previous answer did not contain exactly one fenced rust code block."
    elif not rec["compiles"]:
        note = "Your previous solution did not compile."
    else:
        note = "Your previous solution failed these hidden tests (names only): " + ", ".join(rec.get("_failed") or []) + "."
    return f"{base}\n\n{note} Write the complete corrected Rust source file now, in one ```rust block."


def main():
    global OPENAI_OPTS
    if len(sys.argv) >= 2 and sys.argv[1] == "regrade":
        if len(sys.argv) != 3:
            sys.exit("usage: run.py regrade <out-dir>")
        return regrade(pathlib.Path(sys.argv[2]))
    sys.argv[1:], attempts = take_attempts(sys.argv[1:])
    sys.argv[1:], OPENAI_OPTS = OP.split_options(sys.argv[1:])
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    out = pathlib.Path(sys.argv[1]); provider, model, effort = sys.argv[2:5]
    if provider not in PROVIDERS:
        sys.exit(f"provider must be one of {sorted(PROVIDERS)}")
    if OPENAI_OPTS and provider != "openai":
        sys.exit("--price-in/--price-out/--max-tokens/--timeout/--retries/--backoff/--base-url/--root are for provider openai only")
    check_workdir()
    if provider == "openai":
        OP.prepare(model, effort, OPENAI_OPTS, CFG["model_timeout_seconds"])  # settings and key checked before any task
    all_tasks = [d for d in sorted((HERE / "tasks").iterdir()) if d.is_dir()]
    want = sys.argv[5:]
    tasks = [d for d in all_tasks if not want or d.name in want]
    unknown = set(want) - {d.name for d in all_tasks}
    if unknown:
        sys.exit(f"unknown tasks: {sorted(unknown)}")
    out.mkdir(parents=True, exist_ok=True)
    # per task: the base prompt, the attempt log, the latest record. Round 1 asks every task; round k asks only the
    # tasks that have not passed yet. Each round is graded as ONE Broadsword job (see harness.py).
    state = {}
    for t in tasks:
        spec = (t / "spec.md").read_text(encoding="utf-8")
        state[t.name] = {"dir": t, "base": f"{spec}\n\nWrite the complete Rust source file now, in one ```rust block.",
                         "prompt": None, "log": [], "rec": None, "stop": False,
                         "total": len(re.findall(r"#\[test\]", (t / "tests.rs").read_text(encoding="utf-8")))}
        state[t.name]["prompt"] = state[t.name]["base"]
    for rnd in range(1, attempts + 1):
        to_grade = []
        for st in state.values():
            if st["stop"]:
                continue
            t = st["dir"]
            tag = f"{t.name}__{provider}__{model}__{effort}"
            atag = tag if rnd == 1 else f"{tag}__a{rnd}"
            t0 = time.time()
            res = PROVIDERS[provider](st["prompt"], model, effort)
            dt = time.time() - t0
            code, problem = extract_code(res["answer"])
            call_failed = bool(res.get("is_error") and not res["answer"])
            if call_failed:
                problem = f"model call failed: {res.get('error')}"
            rec = {"provider": provider, "model": model, "effort": effort, "task": t.name, "tokens": res["tokens"],
                   "cost_usd": res["cost_usd"], "wall_s": round(dt, 1), "exit": res["exit"], "harness_wall_s": None,
                   "compiles": False, "passed": 0, "total": st["total"], "graded": True, "call_failed": call_failed, "format_failure": problem is not None and not call_failed,
                   "first_error": problem, "model_used": res.get("model_used", [])}
            # attempt 1 keeps the file names of a one-shot run; later attempts add __aN, and <tag>.* always holds the latest
            for name in {atag, tag}:
                (out / f"{name}.raw.json").write_bytes(res["raw"])
                (out / f"{name}.answer.txt").write_text(res["answer"], encoding="utf-8")
            stale = out / f"{tag}.rs"
            if code is not None:
                for name in {atag, tag}:
                    (out / f"{name}.rs").write_text(code, encoding="utf-8")
                to_grade.append((rec, out / f"{tag}.rs", t))
            elif stale.exists():
                stale.unlink()  # the latest attempt has no code; an older solution must not be taken for it
            st["rec"] = rec
            print(f"{atag}: model call {rec['wall_s']}s cost={rec['cost_usd']} call_failed={rec['call_failed']} format_failure={rec['format_failure']}", flush=True)
        # one Broadsword job grades every extracted solution of this round
        if to_grade:
            try:
                results = H.grade_batch([(rs, t) for _, rs, t in to_grade])
            except SystemExit as e:
                # busy lock, refused job, no result file: the paid model calls already happened, so keep their data.
                # Each task still gets a result JSON (graded=false + the error); `run.py regrade <out-dir>` grades later.
                err = f"grading failed: {e.code}"
                print(err, file=sys.stderr, flush=True)
                for rec, _, _ in to_grade:
                    rec.update(graded=False, first_error=err)
            else:
                for (rec, _, _), h in zip(to_grade, results):
                    rec.update(compiles=h["compiles"], passed=h["passed"], total=h["total"], harness_wall_s=h["wall_s"],
                               first_error=h["first_error"], graded=True, _failed=h.get("failed_tests", []))
        for st in state.values():
            rec = st["rec"]
            if st["stop"] or rec is None or len(st["log"]) >= rnd:
                continue
            entry = {k: v for k, v in rec.items() if k not in ("provider", "model", "effort", "task", "_failed")}
            entry["failed_tests"] = rec.get("_failed", [])
            st["log"].append(entry)
            if fully_passed(rec) or rec["graded"] is False:
                st["stop"] = True  # passed, or cannot be judged (ungraded): no point asking again
            else:
                st["prompt"] = retry_prompt(st["base"], rec)
    for st in state.values():
        rec = st["rec"]
        tag = f"{st['dir'].name}__{provider}__{model}__{effort}"
        log = st["log"]
        if attempts > 1:
            # cost to completion: the sum over every attempt made (an unknown cost is counted, never taken as 0)
            costs = [a["cost_usd"] for a in log]
            known = [c for c in costs if c is not None]
            rec.update(attempts_made=len(log), max_attempts=attempts,
                       pass_at=next((i + 1 for i, a in enumerate(log) if fully_passed(dict(a))), None),
                       cost_usd_total=sum(known) if known else None, cost_unknown_attempts=len(costs) - len(known),
                       wall_s_total=round(sum(a["wall_s"] for a in log), 1), attempts=log)
        rec.pop("_failed", None)
        (out / f"{tag}.json").write_text(json.dumps(rec, indent=1), encoding="utf-8")
        print(f"{tag}: compiles={rec['compiles']} passed={rec['passed']}/{rec['total']}", flush=True)


if __name__ == "__main__":
    main()
