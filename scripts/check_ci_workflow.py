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
        if re.match(r"^on\s*:\s*$", strip_comment(raw)):
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
                    m = re.match(r"^\s*([\w-]+)\s*:", line)
                    if m:
                        current = m.group(1)
                        triggers.append(current)
                elif ind > child_indent and re.match(r"^\s*(branches|branches-ignore|tags|tags-ignore|paths|paths-ignore)\s*:", line):
                    limited.append(current)
            return triggers, limited
        m = re.match(r"^on\s*:\s*(.+)$", strip_comment(raw))
        if m:  # inline forms: `on: push` or `on: [push, pull_request]`
            return [t.strip() for t in m.group(1).strip("[]").split(",") if t.strip()], []
    return [], []


FLOW_START = re.compile(r"(^\s*(-\s+)?[{\[])|(:\s+[{\[])")  # a flow mapping or sequence as a line, list item or value
ANCHOR_ALIAS = re.compile(r"(^|[\s,\[{])[&*][^\s,\]}]+")
QUOTED_KEY = re.compile(r"^\s*(-\s+)?[\"'][^\"']*[\"']\s*:")
ON_INLINE_LIST = re.compile(r"^on\s*:\s*\[\s*[\w-]+(\s*,\s*[\w-]+)*\s*,?\s*\]$")


def shape_problems(lines):
    """Fail closed: the checker reads block-style YAML only, so any form it cannot read line by line is refused, not guessed at.
    Refused: document markers (multi-document files), anchors and aliases, flow-style `{...}` or `[...]` (the one exception is a flat
    `on: [push, pull_request]` list of plain names), quoted keys, explicit `?` keys, `!tags`, any line under `steps:` that is not a plain `key: value` line or a
    block-scalar body (`unrecognised form`), and more than one top-level `on:`. `key :` is read like `key:`."""
    problems = []
    on_keys = 0
    for n, raw in enumerate(lines, 1):
        line = re.sub(r"\$\{\{.*?\}\}", "", strip_comment(raw))  # a ${{ expression }} is not YAML structure
        if not line.strip():
            continue
        if re.match(r"^(---|\.\.\.)(\s|$)", line):
            problems.append(f"line {n}: YAML document marker (multi-document files are refused): {line.strip()}")
        if ANCHOR_ALIAS.search(line):
            problems.append(f"line {n}: YAML anchor or alias (refused; the checker cannot follow them): {line.strip()}")
        if re.match(r"^\s*(-\s+)?<<\s*:", line):
            problems.append(f"line {n}: YAML merge key (refused): {line.strip()}")
        if QUOTED_KEY.match(line):
            problems.append(f"line {n}: quoted YAML key (refused; the checker reads plain keys only): {line.strip()}")
        if re.match(r"^(on|\"on\"|'on'|true)\s*:", line):
            on_keys += 1
        if re.match(r"^on\s*:\s*[{\[]", line):
            if not ON_INLINE_LIST.match(line):
                problems.append(f"line {n}: flow-style `on:` other than a flat `on: [push, pull_request]` list (refused): {line.strip()}")
        elif FLOW_START.search(line):
            problems.append(f"line {n}: flow-style YAML `{{...}}` or `[...]` (refused; write block style): {line.strip()}")
    if on_keys > 1:
        problems.append(f"{on_keys} top-level `on:` keys (refused)")
    problems.extend(steps_problems(lines))
    return problems


EXPLICIT_KEY = re.compile(r"^\s*(-\s+)*\?(\s|$)")
TAG = re.compile(r"(^\s*(-\s+)*|:\s+)!\S*")
KEY_LINE = re.compile(r"^(\s*)((?:-\s+)*)[\w.-]+\s*:(\s|$)")
BLOCK_SCALAR = re.compile(r":\s*[|>][+-]?\d*[+-]?$")
DOUBLE_QUOTED = re.compile(r'^"(?:[^"\\]|\\.)*"\s*$')
SINGLE_QUOTED = re.compile(r"^'(?:[^']|'')*'\s*$")


def unclosed_quote(value):
    """True when `value` opens a double or single quoted scalar that does not close on the same line."""
    value = value.strip()
    if value[:1] == '"':
        return not DOUBLE_QUOTED.match(value)
    if value[:1] == "'":
        return not SINGLE_QUOTED.match(value)
    return False


def steps_problems(lines):
    """Every line under a `steps:` key must be a plain `key: value` line (optionally after `- `) or the body of a `|` / `>` block
    scalar; a quoted value must close on its own line and a key line indented deeper than a key that already has a value is a
    continuation of that value (both refused); explicit `? key` entries and `!tag`s are refused anywhere. Anything else is `unrecognised form`: the checker cannot say
    what it runs, so it fails instead of passing it."""
    problems = []
    steps_indent = None
    body_above = None  # indent a line must exceed to be the body of the block scalar just opened
    prev_has_value, prev_col = False, 0  # the previous key line carried a value, and its key's column
    for n, raw in enumerate(lines, 1):
        line = re.sub(r"\$\{\{.*?\}\}", "", strip_comment(raw))
        if not line.strip():
            continue
        ind = indent_of(line)
        if EXPLICIT_KEY.match(line) or TAG.search(line):
            problems.append(f"line {n}: unrecognised form (explicit `?` key or `!tag`; refused): {line.strip()}")
        if steps_indent is not None and ind <= steps_indent:
            steps_indent = None
            body_above = None
            prev_has_value = False
        if steps_indent is None:
            if re.match(r"^\s*(-\s+)*steps\s*:\s*$", line):
                steps_indent = ind
                body_above = None
            continue
        if body_above is not None and ind > body_above:
            continue  # inside a block scalar body
        body_above = None
        m = KEY_LINE.match(line)
        if not m:
            problems.append(f"line {n}: unrecognised form inside steps (not a plain `key: value` line): {line.strip()}")
            continue
        key_col = len(m.group(1)) + len(m.group(2))
        value = strip_comment(raw).split(":", 1)[1].strip()
        if prev_has_value and key_col > prev_col:
            problems.append(f"line {n}: unrecognised form inside steps (continuation of the previous line's value; refused): {line.strip()}")
        if unclosed_quote(value):
            problems.append(f"line {n}: unrecognised form inside steps (quoted value does not close on its own line; refused): {line.strip()}")
        if BLOCK_SCALAR.search(line):
            body_above = len(m.group(1)) + 2 * m.group(2).count("-")  # the key's own column, so any deeper line is the body
            if m.group(2):
                body_above = len(m.group(1)) + len(m.group(2)) - 1
            prev_has_value = False
        else:
            prev_has_value = bool(value)
        prev_col = key_col
    return problems


def check(workflow_text, contract):
    """The list of problems (empty = OK)."""
    lines = workflow_text.splitlines()
    problems = shape_problems(lines)

    triggers, limited = read_triggers(lines)
    want = [t.strip() for t in contract["triggers"].split(",") if t.strip()]
    if sorted(triggers) != sorted(want):
        problems.append(f"triggers are [{', '.join(triggers)}] but docs/nodes/0.1.md says [{', '.join(want)}]")
    if limited:
        problems.append(f"trigger(s) {', '.join(sorted(set(limited)))} limit branches, tags or paths; CI must run on every push and PR")

    runners = [m.group(1).strip().strip("'\"") for m in (re.match(r"^\s*-?\s*runs-on\s*:\s*(.+)$", strip_comment(l)) for l in lines) if m]
    if not runners:
        problems.append("no runs-on line")
    for r in runners:
        if r != contract["runner"]:
            problems.append(f"runs-on is {r} but docs/nodes/0.1.md says {contract['runner']}")

    runs = []
    for l in lines:
        m = re.match(r"^\s*-?\s*run\s*:\s*(.*)$", strip_comment(l))
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
        m = re.match(r"^\s*-?\s*uses\s*:\s*(.+)$", strip_comment(l))
        if m:
            ref = m.group(1).strip().strip("'\"")
            name, _, rev = ref.partition("@")
            if name != "actions/checkout":
                problems.append(f"uses {ref}: only actions/checkout is allowed")
            elif not SHA.match(rev):
                problems.append(f"uses {ref}: actions/checkout must be pinned to a full 40-character commit SHA")
    if not any(re.match(r"^\s*-?\s*uses\s*:\s*actions/checkout@", l) for l in lines):
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
