"""List and run a roadmap node's done checks.

  python -I scripts/node_check.py <node> --list
  python -I scripts/node_check.py <node> --run <id>
  python -I scripts/node_check.py <node> --run --all
  (add --root DIR to use another Broadsword root; default is the folder above scripts/)

A node's plan, docs/nodes/<node>.md, writes each done check as a fenced block whose info string is `check <id>`:
the first line is the command, the next line is `PASS: <rule>`. Anything else in the block is ignored.

--list prints each check's id, command and PASS rule, and one `line N: ...` per malformed block (exit 1).
--run runs the command from the root with stdout and stderr merged, saves the output to
scratch/node-checks/<node>/<id>.txt, and prints the output, the exit code and the PASS rule. Only two rule forms
are judged here, and only these can fail a run (exit 1):
  PASS: last line == <text>    the last non-empty output line equals <text>
  PASS: contains <text>        <text> appears in the output
  PASS: exit != 0              the command must exit with a non-zero code (a refused-on-purpose check)
Every check also needs exit code 0, unless its rule is `exit != 0`: a non-zero exit, a timeout or a command that
cannot start is always a FAIL, whatever the output says (one place: verdict()). Any other rule is free text: with exit 0
it is printed with "judge by hand" and never counted as a pass or a failure.
--run --all refuses (exit 2, nothing run) while the plan has malformed blocks, so no check is silently skipped;
--run <id> still runs that one check and prints the problems as warnings.
Ids are unique ignoring case (output files share a case-insensitive folder) and are not Windows device names.

Refused (exit 2, nothing run):
  - inside a Broadsword job (BROADSWORD_JOB is set, or CARGO_TARGET_DIR is: jobs set it for every child), because
    a nested broadsword run deadlocks on the build lock;
  - any command that is not `build/bin/broadsword.exe ...`, `broadsword ...` or `python -I ...`;
  - shell syntax: only a trailing `> file` and `2>&1` are accepted (the output is captured anyway); `;`, `&`, `|`,
    `<`, backticks and `$` are refused.
No network, no model calls.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import os
import pathlib
import re
import shlex
import subprocess

FENCE_RE = re.compile(r"^(\s*)(`{3,})\s*(.*?)\s*$")
NODE_RE = re.compile(r"^\d+\.\d+$")
ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
BAD_CHARS = set(";&|<`$()")
RESERVED = {"con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
            "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"}  # Windows device names


class Refused(Exception):
    """The run was refused; nothing was executed."""


def parse(text):
    """Return (checks, problems). A check is {id, command, rule, line}; a problem is (line, message). Lines are 1-based."""
    checks, problems = [], []
    lines = text.splitlines()
    seen = {}
    i = 0
    while i < len(lines):
        m = FENCE_RE.match(lines[i])
        info = m.group(3).split() if m else []
        if not (m and info and info[0] == "check"):
            i += 1
            continue
        start = i + 1
        ticks = len(m.group(2))
        j = i + 1
        while j < len(lines):
            c = FENCE_RE.match(lines[j])
            if c and len(c.group(2)) >= ticks and not c.group(3):
                break
            j += 1
        body = lines[i + 1 : j]
        if j >= len(lines):
            problems.append((start, "check block is never closed"))
        elif len(info) != 2 or not ID_RE.match(info[1]):
            problems.append((start, f"info string must be 'check <id>' with one id, got {m.group(3)!r}"))
        elif not body or not body[0].strip():
            problems.append((start, f"check {info[1]}: the first line must be the command"))
        elif len(body) < 2 or not body[1].startswith("PASS:") or not body[1][5:].strip():
            problems.append((start, f"check {info[1]}: the line after the command must start 'PASS:' and give a rule"))
        elif info[1].split(".")[0].casefold() in RESERVED:
            problems.append((start, f"check id {info[1]} is a Windows device name"))
        elif info[1].casefold() in seen:
            problems.append((start, f"duplicate check id {info[1]} (ignoring case; first at line {seen[info[1].casefold()]})"))
        else:
            seen[info[1].casefold()] = start
            checks.append({"id": info[1], "command": body[0].strip(), "rule": body[1][5:].strip(), "line": start})
        i = j + 1
    return checks, problems


def in_job(env):
    return bool(env.get("BROADSWORD_JOB") or env.get("CARGO_TARGET_DIR"))


def build_argv(command, root):
    """The argv to run for `command`, or raise Refused."""
    try:
        toks = shlex.split(command, posix=False)  # keeps backslashes in Windows paths and quoted words together
    except ValueError as e:
        raise Refused(f"cannot read {command!r}: {e}")
    while toks and toks[-1] == "2>&1":
        toks.pop()
    if len(toks) >= 2 and toks[-2] == ">":
        toks = toks[:-2]
    while toks and toks[-1] == "2>&1":
        toks.pop()
    if any(t in (">", "2>&1") or t.startswith(">") for t in toks):
        raise Refused(f"only a trailing '> file' and '2>&1' are accepted: {command!r}")
    bad = sorted(set(" ".join(toks)) & BAD_CHARS)  # after the allowed redirects are removed, so 2>&1 is not an '&'
    if bad:
        raise Refused(f"shell syntax {''.join(bad)!r} is not allowed in {command!r}")
    toks = [t[1:-1] if len(t) > 1 and t[0] == t[-1] and t[0] in "\"'" else t for t in toks]
    if not toks:
        raise Refused("empty command")
    exe = pathlib.Path(root) / "build" / "bin" / "broadsword.exe"
    if toks[0] in ("build/bin/broadsword.exe", "broadsword"):
        return [str(exe) if exe.exists() else toks[0]] + toks[1:]
    if toks[0] == "python" and len(toks) >= 2 and toks[1] == "-I":
        return [sys.executable] + toks[1:]
    raise Refused(f"only build/bin/broadsword.exe, broadsword and 'python -I' commands run, not {toks[0]!r}")


def judge(rule, output):
    """True/False for the two mechanical rule forms, None for free text."""
    if rule.startswith("last line == "):
        want = rule[len("last line == "):]
        got = [l for l in output.splitlines() if l.strip()]
        return bool(got) and got[-1].strip() == want.strip()
    if rule.startswith("contains "):
        return rule[len("contains "):] in output
    return None


def verdict(rule, output, code):
    """The one place a check is judged: PASS (True), FAIL (False) or None (free text, judge by hand).
    `code` is the exit code, or a string (timeout, not started) when there is none."""
    if rule.strip() == "exit != 0":
        return isinstance(code, int) and code != 0
    if code != 0:
        return False
    return judge(rule, output)


def run_command(argv, cwd, timeout):
    """Run argv; return (output, code). On timeout the whole process tree is killed, not just the child."""
    kw = {"start_new_session": True} if os.name != "nt" else {}
    try:
        proc = subprocess.Popen(argv, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kw)
    except OSError as e:
        return f"could not start: {e}\n", "not started"
    try:
        out, _ = proc.communicate(timeout=timeout)
        return out.decode("utf-8", "replace"), proc.returncode
    except subprocess.TimeoutExpired:
        if os.name == "nt":
            subprocess.run(["taskkill", "/PID", str(proc.pid), "/T", "/F"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        else:
            import signal
            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except OSError:
                pass
        proc.kill()
        out, _ = proc.communicate()
        return out.decode("utf-8", "replace") + f"timed out after {timeout} s\n", "timeout"


def main(argv=None, env=None):
    env = os.environ if env is None else env
    ap = argparse.ArgumentParser(prog="node_check.py", description="List and run a node's done checks.")
    ap.add_argument("node")
    ap.add_argument("--root", default=str(pathlib.Path(__file__).resolve().parent.parent))
    ap.add_argument("--timeout", type=int, default=3600, help="seconds per command (default 3600)")
    g = ap.add_mutually_exclusive_group(required=True)
    g.add_argument("--list", action="store_true")
    g.add_argument("--run", nargs="?", const="", metavar="ID")
    ap.add_argument("--all", action="store_true")
    args = ap.parse_args(argv)
    if not NODE_RE.match(args.node):
        print(f"node_check: node must look like 1.2, got {args.node!r}")
        return 2
    root = pathlib.Path(args.root)
    doc = root / "docs" / "nodes" / f"{args.node}.md"
    if not doc.is_file():
        print(f"node_check: no plan at {doc}")
        return 2
    checks, problems = parse(doc.read_text(encoding="utf-8"))
    if args.list:
        for c in checks:
            print(f"{c['id']}\n  command: {c['command']}\n  PASS: {c['rule']}")
        for line, msg in problems:
            print(f"{doc.name} line {line}: {msg}")
        print(f"{len(checks)} checks, {len(problems)} problems")
        return 1 if problems else 0
    if bool(args.run) == bool(args.all):
        print("node_check: give --run <id> or --run --all")
        return 2
    if in_job(env):
        print("node_check: refusing to run inside a Broadsword job (nested runs deadlock the build lock)")
        return 2
    if args.all:
        if problems or not checks:
            for line, msg in problems:
                print(f"{doc.name} line {line}: {msg}")
            print(f"node_check: refusing --run --all: {len(problems)} malformed block(s), {len(checks)} usable check(s); fix the plan first")
            return 2
        chosen = checks
    else:
        for line, msg in problems:
            print(f"warning: {doc.name} line {line}: {msg}")
        chosen = [c for c in checks if c["id"] == args.run]
        if not chosen:
            print(f"node_check: no check {args.run!r} in {doc.name}")
            return 2
    plans = []
    for c in chosen:  # refuse everything before running anything
        try:
            plans.append((c, build_argv(c["command"], root)))
        except Refused as e:
            print(f"node_check: refused check {c['id']}: {e}")
            return 2
    outdir = root / "scratch" / "node-checks" / args.node
    outdir.mkdir(parents=True, exist_ok=True)
    failed = 0
    for c, argv_ in plans:
        print(f"== {c['id']}: {c['command']}")
        out, code = run_command(argv_, root, args.timeout)
        (outdir / f"{c['id']}.txt").write_text(out, encoding="utf-8")
        print(out, end="" if out.endswith("\n") or not out else "\n")
        v = verdict(c["rule"], out, code)
        word = {True: "PASS", False: "FAIL", None: "judge by hand"}[v]
        print(f"exit: {code}\nrule: {c['rule']}\nresult: {word} (output saved to scratch/node-checks/{args.node}/{c['id']}.txt)")
        failed += v is False
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
