"""Gate-prompt replay: the queue's low-risk gating call, re-run on saved queue prompts, for two system-prompt arms.

Usage (from anywhere; paths are resolved from this file):
  BENCH_CWD=<folder outside the project> [BENCH_CLAUDE=<path to claude cli>] \
    python -I -B replay.py <arm> <run> <prompt-dir> <out-dir> <entry-id> [<entry-id> ...]
  python -I -B replay.py --show        print length and sha256 of each arm's system prompt, then exit (no model call)

Arms:
  A  the gate prompt the queue sends today for `--risk low` (SYSTEM_PROMPT_GATE in reviewer.rs): prompt_head!, gate_block!,
     prompt_tail!, joined by blank lines as `concat!` does there. Read from reviewer.rs with the canary's allow-listed parser
     (scripts/shadow_canary.py prompt_macro), never with a regex of our own.
  B  arm A with the lines of replay-rules-2026-11-20.txt appended to the end of the gate block (one newline between).

The call is the queue's ClaudeCli call: `<cli> -p --system-prompt <arm> --model haiku --tools "" --strict-mcp-config
--max-turns 1 --output-format json`, prompt on stdin, cwd BENCH_CWD (outside the project, so no CLAUDE.md loads), and an
environment without any variable whose name looks like a secret. Every reply's modelUsage must name a haiku model or the
run stops. Raw replies go to <out-dir>/<entry>.<arm>.r<run>.json; an existing reply file is reused, never re-bought.
Every prompt file (<prompt-dir>/<entry>.prompt.txt) is checked to exist and be non-empty before the first paid call.
"""
import hashlib, json, os, pathlib, re, subprocess, sys, time

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent.parent
REVIEWER_RS = ROOT / "crates" / "broadsword" / "src" / "reviewer.rs"
RULES = HERE / "replay-rules-2026-11-20.txt"
MODEL = "haiku"
SECRET_NAME = re.compile(r"KEY|TOKEN|SECRET|PASSWORD|PASS|PAT|CREDENTIAL|AUTH", re.I)

sys.path.insert(0, str(ROOT / "scripts"))
import shadow_canary as canary  # noqa: E402  (the allow-listed reviewer.rs parser)


def macro(name: str) -> str:
    text = REVIEWER_RS.read_bytes().decode("utf-8").replace("\r\n", "\n")
    return canary.prompt_macro(text, canary.code_states(text, REVIEWER_RS), name, REVIEWER_RS)


def arms() -> dict:
    head, block, tail = macro("prompt_head"), macro("gate_block"), macro("prompt_tail")
    # The shadow prompt must be head + tail, as the canary reads it; if not, the layout below is not reviewer.rs's.
    if canary.queue_system_prompt(REVIEWER_RS) != head + "\n\n" + tail:
        sys.exit("reviewer.rs SYSTEM_PROMPT is not prompt_head!() + blank line + prompt_tail!(): update replay.py first")
    rules = RULES.read_text(encoding="utf-8").rstrip("\n")
    if not rules.strip():
        sys.exit(f"{RULES} is empty")
    a = head + "\n\n" + block + "\n\n" + tail
    b = head + "\n\n" + block + "\n" + rules + "\n\n" + tail
    if rules in block:
        print("note: the rules are already in gate_block!, so arm A already holds them and arm B holds them twice")
    return {"A": a, "B": b}


def scrubbed_env() -> dict:
    return {k: v for k, v in os.environ.items() if not SECRET_NAME.search(k)}


def main() -> int:
    if sys.argv[1:] == ["--show"]:
        for k, v in arms().items():
            print(f"arm {k}: {len(v)} characters, sha256 {hashlib.sha256(v.encode('utf-8')).hexdigest()}")
        return 0
    if len(sys.argv) < 6:
        sys.exit(__doc__)
    arm, run, pdir, odir, ids = sys.argv[1], int(sys.argv[2]), pathlib.Path(sys.argv[3]), pathlib.Path(sys.argv[4]), sys.argv[5:]
    workdir = os.environ.get("BENCH_CWD")
    if not workdir:
        sys.exit("set BENCH_CWD to a folder outside the project")
    wd = pathlib.Path(workdir).resolve()
    if wd == ROOT or ROOT in wd.parents:
        sys.exit("BENCH_CWD must be outside the project")
    system = arms().get(arm)
    if system is None:
        sys.exit(f"unknown arm {arm!r}: A or B")
    prompts = {}
    for i in ids:
        p = pdir / f"{i}.prompt.txt"
        if not p.is_file() or p.stat().st_size == 0:
            sys.exit(f"missing or empty prompt file {p}: nothing was called")
        prompts[i] = p.read_bytes()
    cli = os.environ.get("BENCH_CLAUDE", "claude")
    odir.mkdir(parents=True, exist_ok=True)
    wd.mkdir(parents=True, exist_ok=True)
    total = 0.0
    for i in ids:
        f = odir / f"{i}.{arm}.r{run}.json"
        fresh = not f.exists()
        if fresh:
            t0 = time.time()
            p = subprocess.run([cli, "-p", "--system-prompt", system, "--model", MODEL, "--tools", "", "--strict-mcp-config",
                                "--max-turns", "1", "--output-format", "json"],
                               input=prompts[i], capture_output=True, cwd=wd, env=scrubbed_env(), timeout=900)
            f.write_bytes(p.stdout)
            print(f"{i}: exit {p.returncode}, {time.time() - t0:.0f} s", flush=True)
        j = json.loads(f.read_text(encoding="utf-8"))
        used = list((j.get("modelUsage") or {}).keys())
        if not used or any("haiku" not in m for m in used):
            sys.exit(f"{f}: answered by {used}, not a {MODEL} model (lesson 32); stopping")
        c = float(j.get("total_cost_usd") or 0.0)
        total += c if fresh else 0.0
        print(f"{i} arm {arm} run {run}: {used} ${c:.4f}{'' if fresh else ' (reused, not re-bought)'}", flush=True)
    print(f"TOTAL new spend arm {arm} run {run}: ${total:.4f}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
