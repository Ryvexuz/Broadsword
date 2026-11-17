"""Reviewer benchmark runner: one fixed diff-only review per (case, model, effort), headless Claude Code.

Usage (from bench/reviewer/, after ./make_cases.sh has regenerated the diffs):
  BENCH_CWD=<folder outside the project> [BENCH_CLAUDE=<path to claude cli>] \
    python -I -B run.py <out-dir> <model> <effort|default> [case-id ...]

BENCH_CWD must be OUTSIDE the project, so no CLAUDE.md, AGENTS.md or project memory loads. No tools and no MCP.
The prompt goes on stdin, because large diffs exceed Windows' 32k-character command-line limit.
"""
import json, os, pathlib, subprocess, sys, time

HERE = pathlib.Path(__file__).resolve().parent
CLAUDE = os.environ.get("BENCH_CLAUDE", "claude")
WORKDIR = os.environ.get("BENCH_CWD")

SYSTEM = """You are an independent code reviewer. You did not write this code and you do not fix it.
You are given a task description, its done check, and a diff. Decide whether the change really delivers what the task and done check require.
Look for: bugs that break the done check or the stated rules, data loss, security problems, races, faked or silent success, and wrong documentation.
Do not report style issues. Be specific: name the file and line (from the diff) and the concrete scenario that fails.

Answer in exactly this format and nothing else:
VERDICT: PASS or FAIL
BLOCKING: <file:line> - <one sentence: what fails, in which scenario>
(one BLOCKING line per blocking problem; write "BLOCKING: none" if there are none)
MINOR: <file:line> - <one sentence>
(one MINOR line per minor problem; write "MINOR: none" if there are none)"""


def main():
    if not WORKDIR:
        sys.exit("set BENCH_CWD to a folder outside the project")
    out = pathlib.Path(sys.argv[1]); model = sys.argv[2]; effort = sys.argv[3]
    want = set(sys.argv[4:])
    cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
    out.mkdir(parents=True, exist_ok=True)
    pathlib.Path(WORKDIR).mkdir(parents=True, exist_ok=True)
    for c in cases:
        if want and c["id"] not in want:
            continue
        diff = (HERE / c["diff"]).read_text(encoding="utf-8")
        prompt = f"{c['brief']}\n\nDIFF:\n```diff\n{diff}\n```\n\nReview it now, in the required format."
        args = [CLAUDE, "-p", "--system-prompt", SYSTEM, "--model", model, "--tools", "",
                "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"]
        if effort != "default":
            args += ["--effort", effort]
        t0 = time.time()
        p = subprocess.run(args, cwd=WORKDIR, input=prompt.encode("utf-8"), capture_output=True, timeout=900)
        dt = time.time() - t0
        tag = f"{c['id']}__{model}__{effort}"
        (out / f"{tag}.raw.json").write_bytes(p.stdout)
        try:
            r = json.loads(p.stdout)
        except Exception:
            r = {"result": "", "error": p.stderr.decode(errors="replace")[-500:]}
        rec = {"case": c["id"], "model": model, "effort": effort, "wall_s": round(dt, 1), "exit": p.returncode,
               "cost_usd": r.get("total_cost_usd"), "usage": r.get("usage"), "result": r.get("result", ""),
               "is_error": r.get("is_error"), "model_used": list((r.get("modelUsage") or {}).keys())}
        (out / f"{tag}.json").write_text(json.dumps(rec, indent=1), encoding="utf-8")
        print(f"{tag}: exit={p.returncode} {dt:.0f}s cost={rec['cost_usd']}", flush=True)


if __name__ == "__main__":
    main()
