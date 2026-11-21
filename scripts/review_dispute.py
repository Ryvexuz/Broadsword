"""Review dispute path (SPEC_CHECKLIST.md, "Disputing a review finding"): prove a disputed finding with a runnable test, then ask a neutral model.

Run:  python -I -B scripts/review_dispute.py [--root <Broadsword root>] --model <tie-break model> [--exclude <model> ...]
          <queue entry id> <finding text> <test name>

What it does, in this order (each step fails closed with exit 2 and writes nothing unless noted):
 1. Reads <root>/state/queue/<entry>.entry (read only). The finding text, whitespace-normalized, must occur in the entry's `blocking`,
    `shadow_blocking` or `minor` field or in its review / shadow review file, so only a finding a reviewer really raised can be disputed.
 2. Takes the diff `git diff <base_head>...<checked_tip>` of the entry (never cut: over reviewer_diff_max_kb it refuses). The test name
    must occur on an ADDED line of that diff: the test is part of the change itself, not a file outside it.
 3. Runs the test through `broadsword run <project> --task <task> -- ...` (so the disk guard, shared build folder and job log apply).
    Test name forms: `<cargo test filter>` (`cargo test --offline <filter>`), or `py:<scripts/test_x.py>::<test name>`
    (`python -I -B -m unittest discover -s <dir> -p <file> -k <name> -v`). The run counts only when it exits 0 AND reports at least one
    test run and passed (`test result: ok. N passed` with N >= 1, or `Ran N tests` with N >= 1 and `OK`): a filter that matches nothing
    never passes. A failing test is recorded (ruling `not_asked`) and no model is called.
 3b. If the run passed: asks the tie-break model for a RULING through clients/openai_review.py (the existing review client: same
    daily cap, ledger and canary; its answer must come from the requested model). The prompt holds ONLY the finding, the diff and the
    test output (the test output cut to its last reviewer_check_tail_kb KB, everything redacted FIRST). It never holds brief text, the
    gating review or any argument. The model answers a line `RULING: real` (the finding is a real defect) or `RULING: false` or
    `RULING: unsure`; anything else is an error. This script never edits the brief, the review prompt or the queue entry: the
    orchestrator acts on the ruling.
 4. Appends one row to <root>/state/review-disputes.tsv under an OS lock (header: entry, finding_sha, finding, test, test_result,
    tiebreak_model, effort, ruling, evidence, date; finding_sha is the first 12 hex digits of SHA-256 of the normalized finding).

Neutral model: the tie-break model's family (haiku, sonnet, opus, fable or gpt: any name containing one) must differ from the family of the
entry's coder, of the entry's gating reviewer (`reviewer_model`, and `gate_model_used` when present), of the orchestrator (opus) and of
every --exclude. An unknown family anywhere is refused. The review client reaches OpenAI models only, so a gpt gating reviewer leaves no
neutral model: the script refuses and says so.

LIMIT (stated, not hidden): the review client has no reasoning-effort flag, so the `medium` effort asked for in the task is NOT applied;
the `effort` column records `client-default (clients/openai_review.py has no effort flag; medium NOT applied)`.

Limits come from broadsword.toml (required keys: reviewer_diff_max_kb, reviewer_check_tail_kb, reviewer_brief_max_kb, reviewer_timeout_seconds,
shadow_reviewer_timeout_seconds, redact_min_chars, install_dir, state_dir, projects_dir): a missing key is an error, never a default.
Standard library only.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import hashlib
import importlib.util
import json
import os
import re
import subprocess
import time
import tomllib
from pathlib import Path

HERE = Path(__file__).resolve().parent

_spec = importlib.util.spec_from_file_location("shadow_spotcheck", HERE / "shadow_spotcheck.py")
sc = importlib.util.module_from_spec(_spec)
sys.modules["shadow_spotcheck"] = sc
_spec.loader.exec_module(sc)  # reuse the reviewed OS lock and model-family rules (known limits: see its docstring)

DISPUTE_FILE = Path("state") / "review-disputes.tsv"
HEADER = "entry\tfinding_sha\tfinding\ttest\ttest_result\ttiebreak_model\teffort\truling\tevidence\tdate"
RULINGS = ("real", "false", "unsure", "not_asked", "error")
EFFORT_NOTE = "client-default (clients/openai_review.py has no effort flag; medium NOT applied)"
CLIENT = Path("clients") / "openai_review.py"
REQUIRED_KEYS = (
    "reviewer_diff_max_kb", "reviewer_check_tail_kb", "reviewer_brief_max_kb", "reviewer_timeout_seconds",
    "shadow_reviewer_timeout_seconds", "redact_min_chars", "install_dir", "state_dir", "projects_dir",
)
ENTRY_RE = re.compile(r"^q-\d{8}-\d{6}(-[A-Za-z0-9]+)?$")
CARGO_NAME_RE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_:.]*$")
PY_PATH_RE = re.compile(r"^[A-Za-z0-9_][A-Za-z0-9_./-]*\.py$")
SECRET_NAME_RE = re.compile(r"(_KEY|_TOKEN|_SECRET|_PAT|_PASSWORD|_PASS)$|_CREDENTIAL|_AUTH", re.I)

SYSTEM_PROMPT = (
    "You are a neutral referee. A code reviewer raised a finding against a change. The change's author says the finding is false and "
    "added a test to the change to prove it. You get ONLY the finding, the diff of the change and the output of that test. Decide from "
    "those alone whether the test really proves the finding false (it exercises the behaviour the finding is about, and its passing "
    "output shows the finding does not hold) or whether the finding still stands. Do not guess at anything you were not shown.\n"
    "Answer with a short reason, then as the LAST line exactly one of:\n"
    "RULING: real\nRULING: false\nRULING: unsure\n"
    "`real` = the finding is a real defect (the test does not disprove it); `false` = the test proves the finding does not hold; "
    "`unsure` = the evidence given cannot settle it."
)


class Refused(ValueError):
    pass


def norm(text: str) -> str:
    return " ".join(text.replace("\\n", " ").split())


def load_config(root: Path) -> dict:
    try:
        cfg = tomllib.loads((root / "broadsword.toml").read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise Refused(f"cannot read broadsword.toml under {root}: {e}")
    missing = [k for k in REQUIRED_KEYS if k not in cfg]
    if missing:
        raise Refused(f"broadsword.toml lacks the required key(s): {', '.join(missing)}")
    for k in REQUIRED_KEYS:
        v = cfg[k]
        if k in ("install_dir", "state_dir", "projects_dir"):
            if not isinstance(v, str) or not v.strip():
                raise Refused(f"broadsword.toml: {k} must be a non-empty string")
        elif isinstance(v, bool) or not isinstance(v, int) or v <= 0:
            raise Refused(f"broadsword.toml: {k} must be a whole number > 0")
    return cfg


def read_entry(root: Path, cfg: dict, entry: str) -> dict:
    if not ENTRY_RE.match(entry):
        raise Refused(f"queue entry id {entry!r} is not of the form q-YYYYMMDD-HHMMSS")
    path = root / cfg["state_dir"] / "queue" / f"{entry}.entry"
    if not path.is_file():
        raise Refused(f"no queue entry file {path}")
    out = {}
    for line in path.read_text(encoding="utf-8-sig", errors="replace").splitlines():
        if " = " in line:
            k, v = line.split(" = ", 1)
            out[k.strip()] = v
        elif line.endswith(" ="):
            out[line[:-2].strip()] = ""
    for k in ("project", "task", "coder", "base_head"):
        if not out.get(k):
            raise Refused(f"queue entry {entry} has no `{k}`")
    if not (out.get("checked_tip") or out.get("rebased_tip")):
        raise Refused(f"queue entry {entry} has no checked_tip or rebased_tip: nothing to take a diff of")
    return out


def finding_is_recorded(root: Path, cfg: dict, e: dict, finding: str) -> bool:
    hay = [e.get(k, "") for k in ("blocking", "shadow_blocking", "minor")]
    for k in ("review_file", "shadow_file"):
        rel = e.get(k, "").strip()
        if rel and not Path(rel).is_absolute() and ".." not in Path(rel).parts:
            p = root / rel
            if p.is_file():
                hay.append(p.read_text(encoding="utf-8", errors="replace"))
    return norm(finding) in norm("\n".join(hay))


def repo_of(root: Path, cfg: dict, project: str) -> Path:
    if not re.match(r"^[A-Za-z0-9_-]+$", project):
        raise Refused(f"project name {project!r} is not a plain name")
    return root if project == "broadsword" else root / cfg["projects_dir"] / project


def git_diff(repo: Path, base: str, tip: str, max_kb: int) -> str:
    for r in (base, tip):
        if not re.match(r"^[0-9a-f]{7,64}$", r):
            raise Refused(f"{r!r} is not a commit id")
    p = subprocess.run(["git", "-C", str(repo), "diff", f"{base}...{tip}"], capture_output=True, timeout=120)
    if p.returncode != 0:
        raise Refused(f"git diff {base}...{tip} failed: {p.stderr.decode('utf-8', 'replace').strip()[:200]}")
    if len(p.stdout) > max_kb * 1024:
        raise Refused(f"the diff is {len(p.stdout) // 1024} KB, over reviewer_diff_max_kb = {max_kb}: it is never cut, so no model is asked")
    text = p.stdout.decode("utf-8", "replace")
    if not text.strip():
        raise Refused("the diff is empty")
    return text


def added_lines(diff: str) -> str:
    return "\n".join(l[1:] for l in diff.splitlines() if l.startswith("+") and not l.startswith("+++"))


def test_command(test: str) -> tuple:
    """(argv after `--`, the name that must occur on an added diff line). Raises Refused on a bad name."""
    if test.startswith("py:"):
        path, sep, name = test[3:].partition("::")
        if not sep or not name or not PY_PATH_RE.match(path) or ".." in Path(path).parts or not re.match(r"^[A-Za-z0-9_.]+$", name):
            raise Refused(f"test {test!r}: the py form is py:<relative path>.py::<test name>")
        p = Path(path)
        return ["python", "-I", "-B", "-m", "unittest", "discover", "-s", p.parent.as_posix() or ".", "-p", p.name, "-k", name, "-v"], name
    if not CARGO_NAME_RE.match(test):
        raise Refused(f"test {test!r} is not a plain test name (letters, digits, _, : and . only; no leading dash)")
    return ["cargo", "test", "--offline", test], test.rsplit("::", 1)[-1]


def test_passed(out: str, code: int) -> tuple:
    """(passed, why): the run passed only if it exited 0 and reports at least one test run and passed."""
    if code != 0:
        return False, f"exit code {code}"
    cargo = [int(n) for n in re.findall(r"test result: ok\. (\d+) passed", out)]
    if cargo and sum(cargo) >= 1:
        return True, f"{sum(cargo)} passed"
    m = re.search(r"^Ran (\d+) tests? in ", out, re.M)
    if m and int(m.group(1)) >= 1 and re.search(r"^OK\b", out, re.M):
        return True, f"{m.group(1)} unittest(s) OK"
    return False, "exit 0 but no test reported as run and passed (the name may match nothing)"


def run_test(root: Path, cfg: dict, project: str, task: str, test: str) -> tuple:
    """(exit code, combined output). The default runner: `broadsword run <project> --task <task> -- <command>`."""
    argv, _ = test_command(test)
    exe = root / cfg["install_dir"] / ("broadsword.exe" if os.name == "nt" else "broadsword")
    if not exe.is_file():
        raise Refused(f"the installed binary {exe} was not found (run `broadsword install`)")
    p = subprocess.run([str(exe), "run", project, "--task", task, "--", *argv], capture_output=True, cwd=root, timeout=cfg["reviewer_timeout_seconds"])
    return p.returncode, p.stdout.decode("utf-8", "replace") + p.stderr.decode("utf-8", "replace")


def secret_values(root: Path, cfg: dict, environ) -> list:
    """Every .env value and every secret-named environment value of at least redact_min_chars characters."""
    vals = set()
    env_file = root / ".env"
    if env_file.is_file():
        for line in env_file.read_text(encoding="utf-8", errors="replace").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                vals.add(line.split("=", 1)[1].strip().strip("\"'"))
    for k, v in environ.items():
        if SECRET_NAME_RE.search(k):
            vals.add(v)
    return sorted((v for v in vals if len(v) >= cfg["redact_min_chars"]), key=len, reverse=True)


def redact(text: str, secrets: list) -> str:
    for s in secrets:
        text = text.replace(s, "[REDACTED]")
    return text


def tail_kb(text: str, kb: int) -> str:
    raw = text.encode("utf-8")
    if len(raw) <= kb * 1024:
        return text
    return f"[test output cut to its last {kb} KB]\n" + raw[-kb * 1024:].decode("utf-8", "replace")


def build_prompt(finding: str, diff: str, test_name: str, test_output: str) -> str:
    return (
        "=== FINDING (raised by a reviewer) ===\n" + finding + "\n=== END FINDING ===\n\n"
        "=== DIFF OF THE CHANGE ===\n" + diff + "\n=== END DIFF ===\n\n"
        f"=== OUTPUT OF THE TEST `{test_name}` (it passed) ===\n" + test_output + "\n=== END TEST OUTPUT ===\n"
    )


def neutral_check(model: str, entry: dict, excludes) -> str:
    """The tie-break model's family; raises Refused when it is not neutral. The review client reaches OpenAI (gpt) models only."""
    fam = sc.family(model)
    if not fam:
        raise Refused(f"tie-break model {model!r} has an unknown model family (known: {', '.join(sc.FAMILIES)}): refused")
    banned = {"coder": entry["coder"], "orchestrator": "orchestrator"}
    gate_names = [entry.get("reviewer_model", "").strip(), entry.get("gate_model_used", "").strip()]
    if not any(gate_names):
        raise Refused(f"queue entry {entry.get('id', '?')} names no gating reviewer (reviewer_model): cannot check independence")
    for i, g in enumerate(n for n in gate_names if n):
        banned[f"gating reviewer {i}"] = g
    for i, x in enumerate(excludes):
        banned[f"--exclude {x}"] = x
    for who, name in banned.items():
        f = sc.family(name)
        if not f:
            raise Refused(f"{who} {name!r} has an unknown model family: cannot check independence")
        if f == fam:
            raise Refused(f"tie-break model {model!r} (family {fam}) is in the family of the {who} ({name!r}): pick a neutral model")
    if fam != "gpt":
        raise Refused(f"tie-break model {model!r} is not an OpenAI model: the review client (clients/openai_review.py) reaches gpt models only")
    return fam


def call_client(root: Path, cfg: dict, model: str, prompt: str, environ, script: Path = None) -> dict:
    """One call through the review client; returns its parsed JSON object. Raises Refused on anything that is not a clean answer."""
    script = script or (root / CLIENT)
    if not script.is_file():
        raise Refused(f"the review client {script} was not found")
    env = dict(environ)
    env["OPENAI_REVIEW_CALLER"] = "review_dispute"
    env["OPENAI_REVIEW_TIMEOUT"] = str(max(cfg["shadow_reviewer_timeout_seconds"] - 30, 10))  # the client's ONE deadline, short of our kill
    argv = [sys.executable, "-I", "-B", str(script), "-p", "--system-prompt", SYSTEM_PROMPT, "--model", model, "--tools", "", "--strict-mcp-config", "--max-turns", "1", "--output-format", "json"]
    try:
        p = subprocess.run(argv, input=prompt.encode("utf-8"), capture_output=True, env=env, cwd=root, timeout=cfg["shadow_reviewer_timeout_seconds"])
    except subprocess.TimeoutExpired:
        raise Refused("the review client ran past shadow_reviewer_timeout_seconds and was killed")
    try:
        data = json.loads(p.stdout.decode("utf-8", "replace").strip().splitlines()[-1])
    except (IndexError, ValueError):
        raise Refused(f"the review client printed no JSON (exit {p.returncode})")
    if not isinstance(data, dict):
        raise Refused("the review client's output is not a JSON object")
    return data


def parse_ruling(data: dict, model: str) -> tuple:
    """(ruling, answer text). Raises Refused unless the client answered cleanly, as the requested model, with one RULING line."""
    if data.get("is_error") is not False:
        raise Refused(f"the review client reported an error: {norm(str(data.get('result', '')))[:300]}")
    used = data.get("modelUsage")
    names = list(used) if isinstance(used, dict) else []
    want = model.strip().lower()
    if not names or any(not str(n).lower().startswith(want) for n in names):
        raise Refused(f"the answer's modelUsage ({', '.join(map(str, names)) or 'empty'}) does not name the requested model {model!r}")
    text = data.get("result")
    if not isinstance(text, str) or not text.strip():
        raise Refused("the answer is empty")
    rulings = re.findall(r"^RULING: (real|false|unsure)\s*$", text, re.M)
    if len(rulings) != 1 or text.strip().splitlines()[-1].strip() != f"RULING: {rulings[0]}":
        raise Refused("the answer does not end with exactly one `RULING: real|false|unsure` line")
    return rulings[0], text


def append_row(root: Path, cfg: dict, cols: list) -> str:
    clean = [" ".join(str(c).split()) for c in cols]
    if len(clean) != 10 or not clean[0] or not clean[2] or clean[7] not in RULINGS:
        raise Refused("internal: malformed dispute row")
    out = "\t".join(clean)
    path = root / DISPUTE_FILE
    with sc._Lock(root / (str(DISPUTE_FILE) + ".lock")):
        new = not path.is_file() or path.stat().st_size == 0
        if not new:
            first = path.read_text(encoding="utf-8").split("\n", 1)[0]
            if first != HEADER:
                raise Refused(f"{path.name}: header is not {HEADER!r}")
        with open(path, "a", encoding="utf-8", newline="") as f:
            f.write((HEADER + "\n" if new else "") + out + "\n")
    return out


def dispute(root: Path, entry_id: str, finding: str, test: str, model: str, excludes=(), environ=None, runner=None, caller=None) -> tuple:
    """Run the whole path. Returns (ruling, row). `runner(root, cfg, project, task, test)` and `caller(root, cfg, model, prompt, environ)`
    are the two seams the tests replace; the defaults run the real test and the real client."""
    environ = os.environ if environ is None else environ
    runner = runner or run_test
    caller = caller or call_client
    root = Path(root)
    cfg = load_config(root)
    finding = " ".join(finding.split())
    if not finding:
        raise Refused("the finding text is empty")
    if len(finding.encode("utf-8")) > cfg["reviewer_brief_max_kb"] * 1024:
        raise Refused(f"the finding text is over reviewer_brief_max_kb = {cfg['reviewer_brief_max_kb']} KB")
    e = read_entry(root, cfg, entry_id)
    e["id"] = entry_id
    neutral_check(model, e, excludes)
    if not finding_is_recorded(root, cfg, e, finding):
        raise Refused(f"the finding text does not occur in the blocking, shadow_blocking or minor lines (or review files) of {entry_id}")
    argv, must_be_added = test_command(test)
    diff = git_diff(repo_of(root, cfg, e["project"]), e["base_head"], e.get("checked_tip") or e["rebased_tip"], cfg["reviewer_diff_max_kb"])
    if must_be_added not in added_lines(diff):
        raise Refused(f"the test name {must_be_added!r} is not on an added line of the change's diff: the proof must be part of the change itself")
    code, output = runner(root, cfg, e["project"], e["task"], test)
    secrets = secret_values(root, cfg, environ)
    output = redact(output, secrets)
    passed, why = test_passed(output, code)
    sha = hashlib.sha256(norm(finding).encode("utf-8")).hexdigest()[:12]
    today = time.strftime("%Y-%m-%d", time.gmtime())
    if not passed:
        row = append_row(root, cfg, [entry_id, sha, finding, test, f"failed: {why}", model, EFFORT_NOTE, "not_asked", "the test did not pass, so no model was asked", today])
        return "not_asked", row
    prompt = build_prompt(redact(finding, secrets), redact(diff, secrets), test, tail_kb(output, cfg["reviewer_check_tail_kb"]))
    try:
        ruling, text = parse_ruling(caller(root, cfg, model, prompt, environ), model)
    except Refused as err:
        row = append_row(root, cfg, [entry_id, sha, finding, test, f"passed: {why}", model, EFFORT_NOTE, "error", redact(str(err), secrets)[:400], today])
        raise Refused(f"{err} (recorded as an error row)") from None
    row = append_row(root, cfg, [entry_id, sha, finding, test, f"passed: {why}", model, EFFORT_NOTE, ruling, redact(norm(text), secrets)[:400], today])
    return ruling, row


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=".", help="the Broadsword root (holds broadsword.toml and state/)")
    ap.add_argument("--model", required=True, help="the neutral tie-break model (an OpenAI model name)")
    ap.add_argument("--exclude", action="append", default=[], metavar="MODEL", help="also refuse this model's family")
    ap.add_argument("entry")
    ap.add_argument("finding")
    ap.add_argument("test")
    args = ap.parse_args(argv)
    try:
        ruling, row = dispute(Path(args.root), args.entry, args.finding, args.test, args.model, args.exclude)
    except (Refused, sc.Malformed) as e:
        print(f"refused: {e}", file=sys.stderr)
        return 2
    except (subprocess.TimeoutExpired, OSError, ValueError) as e:
        print(f"refused: {type(e).__name__}: {e}", file=sys.stderr)
        return 2
    print(f"ruling: {ruling}")
    print(f"recorded: {row}")
    return 0 if ruling in ("real", "false", "unsure") else 1


if __name__ == "__main__":
    sys.exit(main())
