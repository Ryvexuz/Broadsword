"""Drift benchmark runner: drives one model through every step of the task in ONE conversation, and after each step checks
the work folder and the step's transcript against the step-0 rules, grades the hidden tests so far, and records the
context size the provider reported.

Usage:
  python -I -B bench/drift/run.py --client fake --model <label> --context-window <tokens> [--break <step>:<rule> ...]
  python -I -B bench/drift/run.py --client claude|codex --model <model> --context-window <tokens> --confirm-plan-run

  fake    the scripted client (fake.py) and the text grader; no model, no cargo. Writes to <scratch_dir>/<dry_run_subdir>/.
  claude  Claude Code CLI on the Claude plan; codex: Codex CLI on the ChatGPT plan. Both are PROVISIONAL (clients.py) and
          use plan quota, so --confirm-plan-run is required. The work crate is <projects_dir>/drift-<run id>, graded by
          `broadsword test` of <projects_dir>/drift-<run id>-grader. Raw results go to <state_dir>/<results_subdir>/<run id>/.
  --context-window  the model's context window in tokens (look it up; never guessed), used for the context share.
  --break   fake only: plant a break of a rule (R1_frozen_file, R2_signature, R3_tests_before_done, R4_report_line, or
            R1..R4) at a step. May be repeated.
Then: python -I -B bench/drift/score.py <results dir>
"""
import argparse
import datetime
import json
import os
import pathlib
import re
import secrets
import sys
import tomllib

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import checks  # noqa: E402
import task  # noqa: E402

ROOT = HERE.parent.parent
CONFIG_KEYS = ("step_count", "checkpoints", "model_timeout_seconds", "grade_timeout_seconds", "results_subdir",
               "dry_run_subdir")
SETTINGS_KEYS = ("state_dir", "scratch_dir", "projects_dir", "install_dir")


class RunError(Exception):
    pass


def load_config(path=HERE / "config.toml"):
    """bench/drift/config.toml. Every key is required; there is no default anywhere in code."""
    cfg = tomllib.loads(pathlib.Path(path).read_text(encoding="utf-8"))
    missing = [k for k in CONFIG_KEYS if k not in cfg]
    if missing:
        raise RunError(f"{path}: missing required keys {missing}")
    checks.validate_checkpoints(cfg["checkpoints"], cfg["step_count"])
    for k in ("model_timeout_seconds", "grade_timeout_seconds"):
        v = cfg[k]
        if not isinstance(v, (int, float)) or isinstance(v, bool) or not (0 < v < float("inf")):
            raise RunError(f"{path}: {k} must be a number of seconds > 0, got {v!r}")
    for k in ("results_subdir", "dry_run_subdir"):
        _relative(cfg[k], k)
    return cfg


def _relative(value, name):
    """A configured path must be relative and stay inside its base: no drive, no leading slash, no `..`."""
    if not isinstance(value, str) or not value.strip():
        raise RunError(f"{name} must be a non-empty relative path")
    p = value.replace("\\", "/")
    if p.startswith("/") or re.match(r"^[A-Za-z]:", p) or ".." in p.split("/"):
        raise RunError(f"{name} must be a relative path inside its base folder, got {value!r}")
    return pathlib.Path(p)


def load_settings(root=ROOT):
    """The paths this bench needs from broadsword.toml (required keys)."""
    s = tomllib.loads((pathlib.Path(root) / "broadsword.toml").read_text(encoding="utf-8"))
    missing = [k for k in SETTINGS_KEYS if k not in s]
    if missing:
        raise RunError(f"broadsword.toml: missing required keys {missing}")
    return {k: pathlib.Path(root) / _relative(s[k], k) for k in SETTINGS_KEYS}


def snapshot(workdir):
    """{relative path: sha256} of every file in the work folder except build output and VCS folders."""
    import hashlib
    out = {}
    base = pathlib.Path(workdir)
    for p in sorted(base.rglob("*")):
        rel = p.relative_to(base).as_posix()
        if rel.split("/")[0] in ("target", ".git") or not p.is_file():
            continue
        out[rel] = hashlib.sha256(p.read_bytes()).hexdigest()
    return out


def make_workdir(workdir):
    workdir = pathlib.Path(workdir)
    if workdir.exists():
        raise RunError(f"work folder already exists, refusing to reuse it: {workdir}")
    for rel, text in task.TEMPLATE.items():
        p = workdir / rel
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(text, encoding="utf-8", newline="\n")


def _write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8", newline="\n")


def run_drift(client, grader, workdir, out_dir, step_count, checkpoints, context_window, test_command, test_re, meta):
    """Drive `client` through steps 1..step_count. Writes, in out_dir: meta.json, steps.jsonl (one record per step, flushed
    as it goes), raw/step_NN.{prompt,raw,reply}.txt, files/step_NN/<changed files> and summary.json. Returns the summary.
    The work folder must not exist yet. A failed model call is recorded as an error step and ends the run."""
    checks.validate_checkpoints(checkpoints, step_count)
    out_dir = pathlib.Path(out_dir)
    if out_dir.exists():
        raise RunError(f"results folder already exists, refusing to overwrite it: {out_dir}")
    out_dir.mkdir(parents=True)
    make_workdir(workdir)
    frozen_sha = task.frozen_sha256()
    meta = dict(meta, step_count=step_count, checkpoints=checkpoints, context_window=context_window,
                test_command=test_command, frozen_sha256=frozen_sha, rules=list(checks.RULES),
                grader=getattr(grader, "name", type(grader).__name__))
    _write(out_dir / "meta.json", json.dumps(meta, indent=2))
    before = snapshot(workdir)
    records = []
    with open(out_dir / "steps.jsonl", "w", encoding="utf-8", newline="\n") as log:
        for k in range(1, step_count + 1):
            prompt = task.prompt_for(k, step_count, test_command)
            _write(out_dir / "raw" / f"step_{k:02d}.prompt.txt", prompt)
            try:
                res = client.step(k, prompt)
            except Exception as e:  # a client crash is a failed call, recorded, never a silent stop
                res = {"error": f"client raised {type(e).__name__}: {e}", "raw": ""}
            _write(out_dir / "raw" / f"step_{k:02d}.raw.txt", str(res.get("raw") or ""))
            _write(out_dir / "raw" / f"step_{k:02d}.reply.txt", str(res.get("reply") or ""))
            if res.get("error"):
                rec = {"step": k, "error": str(res["error"])}
            else:
                after = snapshot(workdir)
                changed = sorted(p for p in set(before) | set(after) if before.get(p) != after.get(p))
                for rel in changed:
                    if rel in after:
                        dst = out_dir / "files" / f"step_{k:02d}" / rel
                        dst.parent.mkdir(parents=True, exist_ok=True)
                        dst.write_bytes((pathlib.Path(workdir) / rel).read_bytes())
                before = after
                try:
                    rules = checks.check_step(workdir, k, res.get("events"), res.get("reply"), frozen_sha, test_re)
                except checks.ScoreError as e:
                    rec = {"step": k, "error": f"unjudgeable transcript: {e}"}
                else:
                    try:
                        hidden = grader.grade(workdir, k, out_dir)
                    except Exception as e:  # the grader's own failure: recorded, the run ends
                        hidden = {"passed": None, "total": k, "failed": [], "error": f"{type(e).__name__}: {e}"}
                    if hidden.get("error") or hidden.get("passed") is None:
                        # the grader could not grade (its own failure, not the model's): an ERROR step, never a score
                        rec = {"step": k, "error": f"grader: {hidden.get('error') or 'no result'}", "rules": rules}
                    else:
                        rec = {"step": k, "rules": rules, "hidden": hidden, "changed_files": changed,
                               "events": res.get("events"), "context_tokens": res.get("context_tokens"),
                               "context_source": res.get("context_source")}
            records.append(rec)
            log.write(json.dumps(rec) + "\n")
            log.flush()
            if rec.get("error"):
                break
    summary = checks.summarize(records, checkpoints, step_count, context_window)
    summary["meta"] = meta
    _write(out_dir / "summary.json", json.dumps(summary, indent=2))
    return summary


RULE_ALIASES = {f"R{i + 1}": r for i, r in enumerate(checks.RULES)}


def parse_break(text):
    m = re.fullmatch(r"(\d+):(\S+)", text or "")
    if not m:
        raise argparse.ArgumentTypeError(f"--break must be <step>:<rule>, got {text!r}")
    rule = RULE_ALIASES.get(m.group(2), m.group(2))
    if rule not in checks.RULES:
        raise argparse.ArgumentTypeError(f"unknown rule {m.group(2)!r}; one of {list(checks.RULES)} or R1..R4")
    return int(m.group(1)), rule


def main(argv=None, environ=None):
    environ = os.environ if environ is None else environ
    ap = argparse.ArgumentParser(description="Long-task drift benchmark runner.")
    ap.add_argument("--client", required=True, choices=["fake", "claude", "codex"])
    ap.add_argument("--model", required=True)
    ap.add_argument("--context-window", required=True, type=int)
    ap.add_argument("--break", dest="breaks", action="append", default=[], type=parse_break)
    ap.add_argument("--confirm-plan-run", action="store_true")
    a = ap.parse_args(argv)
    try:
        cfg = load_config()
        st = load_settings()
        if a.context_window < 1:
            raise RunError("--context-window must be >= 1")
        if a.client != "fake" and a.breaks:
            raise RunError("--break is for the fake client only")
        for s, r in a.breaks:
            if not 1 <= s <= cfg["step_count"]:
                raise RunError(f"--break step {s} is outside 1..{cfg['step_count']}")
        if a.client != "fake" and not a.confirm_plan_run:
            raise RunError("a real run uses plan quota: add --confirm-plan-run (and ask Lazlo first)")
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S")
        run_id = f"{stamp}-{a.client}-{re.sub(r'[^A-Za-z0-9.-]', '_', a.model)}-{secrets.token_hex(2)}"
        meta = {"run_id": run_id, "client": a.client, "model": a.model, "fake": a.client == "fake",
                "started_utc": stamp, "planted_breaks": [[s, r] for s, r in a.breaks]}
        if a.client == "fake":
            import fake
            base = st["scratch_dir"] / _relative(cfg["dry_run_subdir"], "dry_run_subdir") / run_id
            workdir, out_dir = base / "work", base / "results"
            test_command = "broadsword test drift-dry"
            breaks = {}
            for s, r in a.breaks:
                breaks.setdefault(s, set()).add(r)
            client = fake.FakeClient(workdir, test_command, breaks)
            grader = fake.TextGrader()
            test_re = task.test_command_regex("drift-dry")
        else:
            import clients
            project = f"drift-{run_id}"
            workdir = st["projects_dir"] / project
            out_dir = st["state_dir"] / _relative(cfg["results_subdir"], "results_subdir") / run_id
            exe = st["install_dir"] / ("broadsword.exe" if os.name == "nt" else "broadsword")
            if not exe.is_file():
                raise RunError(f"the installed broadsword binary is missing: {exe}")
            test_command = f"{os.path.relpath(exe, workdir).replace(os.sep, '/')} test {project}"
            test_re = task.test_command_regex(project)
            cls = clients.ClaudeCliClient if a.client == "claude" else clients.CodexCliClient
            client = cls(a.model, workdir, ROOT, test_command, cfg["model_timeout_seconds"], environ)
            grader = clients.CargoGrader(exe, workdir, cfg["grade_timeout_seconds"])
            grader.setup(task)
        summary = run_drift(client, grader, workdir, out_dir, cfg["step_count"], cfg["checkpoints"], a.context_window,
                            test_command, test_re, meta)
    except (RunError, checks.ScoreError, OSError, RuntimeError) as e:
        print(f"drift run: error: {e}", file=sys.stderr)
        return 2
    print(f"results: {out_dir}")
    print(f"steps scored: {summary['steps_scored']}/{summary['step_count']}  complete: {summary['complete']}  "
          f"work score: {summary['work_score']}")
    for rule, r in summary["rules"].items():
        print(f"{rule}: first break {r['first_break'] if r['first_break'] else 'never'}")
    return 0 if summary["complete"] else 1


if __name__ == "__main__":
    sys.exit(main())
