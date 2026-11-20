"""Check .github/workflows/ci.yml against the CI contract in docs/nodes/0.1.md.

  python -I -B scripts/check_ci_workflow.py [--root DIR] [--workflow FILE] [--doc FILE]

Std-only; no YAML library. It reads only the few lines it needs from the workflow:
the keys under `on:`, `runs-on:`, every `run:` and `uses:` line, and any `secrets.` mention.

The contract is the `- ci-<key>: <value>` lines in the doc (ci-triggers, ci-runner, ci-build, ci-test, ci-checkout).
Exit 0 and `ci workflow: OK` when the workflow matches. Exit 1 with one `ci workflow: FAILED, ...` line per problem.
Exit 2 when a file is missing or the doc lacks a contract line. No network.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import pathlib
import re

CONTRACT_KEYS = ("triggers", "runner", "build", "test", "checkout")
SHA = re.compile(r"^[0-9a-f]{40}$")


def strip_comment(line):
    """The line without a trailing ` # comment` (a '#' starting the line or following a space)."""
    return re.sub(r"(^|\s)#.*$", "", line).rstrip()


def read_contract(text):
    """{key: value} from the `- ci-<key>: <value>` lines of the doc."""
    out = {}
    for line in text.splitlines():
        m = re.match(r"^\s*-\s+ci-(\w+):\s*(.+?)\s*$", line)
        if m:
            out[m.group(1)] = m.group(2)
    return out


def indent_of(line):
    return len(line) - len(line.lstrip(" "))


def read_triggers(lines):
    """(triggers, limited): the keys directly under a top-level `on:` and whether any of them restricts branches or tags."""
    triggers, limited = [], []
    for i, raw in enumerate(lines):
        if re.match(r"^on:\s*$", strip_comment(raw)):
            child_indent = None
            current = None
            for raw2 in lines[i + 1:]:
                line = strip_comment(raw2)
                if not line.strip():
                    continue
                ind = indent_of(line)
                if ind == 0:
                    break
                if child_indent is None:
                    child_indent = ind
                if ind == child_indent:
                    m = re.match(r"^\s*([\w-]+):", line)
                    if m:
                        current = m.group(1)
                        triggers.append(current)
                elif ind > child_indent and re.match(r"^\s*(branches|branches-ignore|tags|tags-ignore|paths|paths-ignore)\s*:", line):
                    limited.append(current)
            return triggers, limited
        m = re.match(r"^on:\s*(.+)$", strip_comment(raw))
        if m:  # inline forms: `on: push` or `on: [push, pull_request]`
            return [t.strip() for t in m.group(1).strip("[]").split(",") if t.strip()], []
    return [], []


def check(workflow_text, contract):
    """The list of problems (empty = OK)."""
    problems = []
    lines = workflow_text.splitlines()

    triggers, limited = read_triggers(lines)
    want = [t.strip() for t in contract["triggers"].split(",") if t.strip()]
    if sorted(triggers) != sorted(want):
        problems.append(f"triggers are [{', '.join(triggers)}] but docs/nodes/0.1.md says [{', '.join(want)}]")
    if limited:
        problems.append(f"trigger(s) {', '.join(sorted(set(limited)))} limit branches, tags or paths; CI must run on every push and PR")

    runners = [m.group(1).strip().strip("'\"") for m in (re.match(r"^\s*-?\s*runs-on:\s*(.+)$", strip_comment(l)) for l in lines) if m]
    if not runners:
        problems.append("no runs-on line")
    for r in runners:
        if r != contract["runner"]:
            problems.append(f"runs-on is {r} but docs/nodes/0.1.md says {contract['runner']}")

    runs = []
    for l in lines:
        m = re.match(r"^\s*-?\s*run:\s*(.*)$", strip_comment(l))
        if m:
            runs.append(m.group(1).strip().strip("'\""))
    if contract["build"] not in runs:
        problems.append(f"no run line equal to the documented build command: {contract['build']}")
    if contract["test"] not in runs:
        problems.append(f"no run line equal to the documented test command: {contract['test']}")
    for r in runs:
        if r not in (contract["build"], contract["test"]):
            problems.append(f"unexpected run line (only the documented build and test commands are allowed): {r}")

    for l in lines:
        m = re.match(r"^\s*-?\s*uses:\s*(.+)$", strip_comment(l))
        if m:
            ref = m.group(1).strip().strip("'\"")
            name, _, rev = ref.partition("@")
            if name != "actions/checkout":
                problems.append(f"uses {ref}: only actions/checkout is allowed")
            elif not SHA.match(rev):
                problems.append(f"uses {ref}: actions/checkout must be pinned to a full 40-character commit SHA")
    if not any(re.match(r"^\s*-?\s*uses:\s*actions/checkout@", l) for l in lines):
        problems.append("no actions/checkout step")

    for l in lines:
        if "secrets." in strip_comment(l):
            problems.append(f"workflow mentions secrets: {l.strip()}")
    return problems


def main():
    here = pathlib.Path(__file__).resolve().parent.parent
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=str(here))
    ap.add_argument("--workflow", default=None)
    ap.add_argument("--doc", default=None)
    a = ap.parse_args()
    root = pathlib.Path(a.root).resolve()
    workflow = pathlib.Path(a.workflow) if a.workflow else root / ".github" / "workflows" / "ci.yml"
    doc = pathlib.Path(a.doc) if a.doc else root / "docs" / "nodes" / "0.1.md"
    for f in (workflow, doc):
        if not f.is_file():
            print(f"ci workflow: FAILED, {f} not found")
            return 2
    contract = read_contract(doc.read_text(encoding="utf-8"))
    missing = [k for k in CONTRACT_KEYS if k not in contract]
    if missing:
        print(f"ci workflow: FAILED, {doc} has no contract line for: {', '.join('ci-' + k for k in missing)}")
        return 2
    problems = check(workflow.read_text(encoding="utf-8"), contract)
    if problems:
        for p in problems:
            print(f"ci workflow: FAILED, {p}")
        return 1
    print("ci workflow: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
