"""Coding benchmark runner (one-shot track): one model call per (task, model, effort), then the hidden tests.

Usage (from anywhere):
  BENCH_CWD=<folder outside the project> [BENCH_CLAUDE=<path to claude cli>] \
    python -I -B run.py <out-dir> <provider> <model> <effort|default> [task ...]

  provider  claude  the headless Claude Code CLI, run like bench/reviewer/run.py: cwd = BENCH_CWD, no tools,
                    no MCP, prompt on stdin, JSON output
            openai  not built yet (raises)
  task      folder names under tasks/ (01-duration ...); none given = all six

BENCH_CWD must be OUTSIDE the project and have no CLAUDE.md, AGENTS.md or .git in it or any parent, so no
project instructions or memory reach the model. BENCH_CLAUDE may be a .py file (run with the current Python):
that is how the smoke test uses a fake CLI. The model sees only the task's spec.md: never the tests.

Per task it writes <out-dir>/<task>__<provider>__<model>__<effort>.{raw.json,answer.txt,rs,json}.
The result .json holds: provider, model, effort, task, tokens, cost_usd, wall_s (the model call),
harness_wall_s, compiles, passed, total, format_failure, first_error.
"""
import importlib.util, json, os, pathlib, re, subprocess, sys, time, tomllib

HERE = pathlib.Path(__file__).resolve().parent
CFG = tomllib.loads((HERE / "config.toml").read_text(encoding="utf-8"))
_spec = importlib.util.spec_from_file_location("coder_harness", HERE / "harness.py")
H = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(H)
CLAUDE = os.environ.get("BENCH_CLAUDE", "claude")
WORKDIR = os.environ.get("BENCH_CWD")

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
    p = subprocess.run(args, cwd=WORKDIR, input=prompt.encode("utf-8"), capture_output=True,
                       timeout=CFG["model_timeout_seconds"])
    try:
        r = json.loads(p.stdout)
    except Exception:
        r = {"result": "", "is_error": True, "error": p.stderr.decode(errors="replace")[-500:]}
    u = r.get("usage") or {}
    tokens = {"input": u.get("input_tokens"), "output": u.get("output_tokens"),
              "cache_read": u.get("cache_read_input_tokens"), "cache_write": u.get("cache_creation_input_tokens")}
    return {"answer": r.get("result") or "", "raw": p.stdout, "tokens": tokens, "cost_usd": r.get("total_cost_usd"),
            "exit": p.returncode, "is_error": r.get("is_error"), "error": r.get("error")}


def provider_openai(prompt, model, effort):
    # bench/reviewer/run_openai.py does not exist yet. When it does, reuse its request code here and
    # compute cost only from prices passed in explicitly.
    raise NotImplementedError("openai provider: not built yet")


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


def main():
    if len(sys.argv) < 5:
        sys.exit(__doc__)
    out = pathlib.Path(sys.argv[1]); provider, model, effort = sys.argv[2:5]
    if provider not in PROVIDERS:
        sys.exit(f"provider must be one of {sorted(PROVIDERS)}")
    check_workdir()
    all_tasks = [d for d in sorted((HERE / "tasks").iterdir()) if d.is_dir()]
    want = sys.argv[5:]
    tasks = [d for d in all_tasks if not want or d.name in want]
    unknown = set(want) - {d.name for d in all_tasks}
    if unknown:
        sys.exit(f"unknown tasks: {sorted(unknown)}")
    out.mkdir(parents=True, exist_ok=True)
    recs, to_grade = [], []
    for t in tasks:
        spec = (t / "spec.md").read_text(encoding="utf-8")
        total = len(re.findall(r"#\[test\]", (t / "tests.rs").read_text(encoding="utf-8")))
        prompt = f"{spec}\n\nWrite the complete Rust source file now, in one ```rust block."
        tag = f"{t.name}__{provider}__{model}__{effort}"
        t0 = time.time()
        res = PROVIDERS[provider](prompt, model, effort)
        dt = time.time() - t0
        (out / f"{tag}.raw.json").write_bytes(res["raw"])
        (out / f"{tag}.answer.txt").write_text(res["answer"], encoding="utf-8")
        code, problem = extract_code(res["answer"])
        if res.get("is_error") and not res["answer"]:
            problem = f"model call failed: {res.get('error')}"
        rec = {"provider": provider, "model": model, "effort": effort, "task": t.name, "tokens": res["tokens"],
               "cost_usd": res["cost_usd"], "wall_s": round(dt, 1), "exit": res["exit"], "harness_wall_s": None,
               "compiles": False, "passed": 0, "total": total, "format_failure": problem is not None,
               "first_error": problem}
        recs.append((tag, rec))
        if code is not None:
            rs = out / f"{tag}.rs"
            rs.write_text(code, encoding="utf-8")
            to_grade.append((rec, rs, t))
        print(f"{tag}: model call {rec['wall_s']}s cost={rec['cost_usd']} format_failure={rec['format_failure']}", flush=True)
    # one Broadsword job grades every extracted solution (see harness.py)
    if to_grade:
        results = H.grade_batch([(rs, t) for _, rs, t in to_grade])
        for (rec, _, _), h in zip(to_grade, results):
            rec.update(compiles=h["compiles"], passed=h["passed"], total=h["total"], harness_wall_s=h["wall_s"],
                       first_error=h["first_error"])
    for tag, rec in recs:
        (out / f"{tag}.json").write_text(json.dumps(rec, indent=1), encoding="utf-8")
        print(f"{tag}: compiles={rec['compiles']} passed={rec['passed']}/{rec['total']}", flush=True)


if __name__ == "__main__":
    main()
