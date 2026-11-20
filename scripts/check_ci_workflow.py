"""Check that .github/workflows/ci.yml is exactly the approved CI workflow (roadmap node 0.1).

  python -I -B scripts/check_ci_workflow.py [--root DIR] [--workflow FILE] [--doc FILE]

Std-only, no network, and no YAML parsing. The approved workflow is the APPROVED text below. The check passes ONLY if
ci.yml is equal to it byte for byte, after CRLF line ends are turned into LF (nothing else is normalised: a BOM, a lone
CR, a tab, trailing spaces or a missing final newline all fail). Any other content fails, with a unified diff and the
line "ci.yml changed: review the diff, then update the approved copy in the same commit".

Why no parser: four review rounds each found another YAML form that a hand parser read wrongly, so a workflow that ran
other code still printed OK. Comparing against one reviewed copy fails closed by construction: whatever YAML can express,
a file that is not the approved copy is refused.

APPROVED itself is checked first by pin_problems(), so changing it stays deliberate: plain substring checks that it holds
the documented triggers (push and pull_request, with no filters), runner and two commands, that its only `uses:` is
actions/checkout at the pinned SHA, and that it has none of the keys that would weaken the job. The documented values
are the `- ci-<key>: <value>` lines in docs/nodes/0.1.md (ci-triggers, ci-runner, ci-build, ci-test, ci-checkout); they
must equal the constants below. This is a self-test of the pin, not a parser.

Exit 0 and `ci workflow: OK` when ci.yml is the approved copy. Exit 1 on a difference or a failed self-test.
Exit 2 when a file is missing or the doc lacks a contract line.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import difflib
import pathlib
import re

CONTRACT_KEYS = ("triggers", "runner", "build", "test", "checkout")

CHECKOUT_SHA = "11bd71901bbe5b1630ceea73d27597364c9af683"  # actions/checkout v4.2.2
TRIGGERS = "push, pull_request"
RUNNER = "windows-latest"
BUILD = "cargo build --locked -p broadsword"
TEST = "cargo test --locked -p broadsword"
CHECKOUT = f"actions/checkout@{CHECKOUT_SHA}"

CHANGED = "ci.yml changed: review the diff, then update the approved copy in the same commit"

# The one approved workflow. To change ci.yml, change this text in the same commit, and have the diff reviewed.
APPROVED = f"""# CI for the Broadsword crate (roadmap node 0.1). scripts/check_ci_workflow.py checks this file against docs/nodes/0.1.md.
# No secrets, no deploy steps; the only third-party action is actions/checkout, pinned to a full commit SHA.
name: ci

on:
  push:
  pull_request:

permissions:
  contents: read

jobs:
  build-test:
    runs-on: {RUNNER}
    steps:
      - uses: {CHECKOUT} # v4.2.2
      - name: Build
        run: {BUILD}
      - name: Test
        run: {TEST}
"""

# Substrings the approved copy must never contain: each would limit when CI runs, skip or soften the tests, change
# what the commands do, or reach secrets ("$" refuses every ${{ }} expression, so no context or secret can be read;
# the header comment's "No secrets," is allowed). A plain list, checked on the approved copy only.
FORBIDDEN = ("$", "secrets.", "secrets)", "branches", "tags", "paths", "types:", "if:", "continue-on-error", "shell:", "env:",
             "working-directory", "container:", "services:", "strategy:", "defaults:", "\t", "\r", "{", "}", "&", "*",
             "<<", "---", "...", " :", "?", "|", ">", "!", "\"", "'", "[", "]", "%")


def read_contract(text):
    """{key: value} from the `- ci-<key>: <value>` lines of the doc."""
    out = {}
    for line in text.splitlines():
        m = re.match(r"^\s*-\s+ci-(\w+):\s*(.+?)\s*$", line)
        if m:
            out[m.group(1)] = m.group(2)
    return out


def pin_problems(approved, contract):
    """Problems with the approved copy itself: the documented values, and exact substring checks on the pin."""
    p = []
    expected = {"triggers": TRIGGERS, "runner": RUNNER, "build": BUILD, "test": TEST, "checkout": CHECKOUT}
    for key, want in expected.items():
        if contract.get(key) != want:
            p.append(f"docs/nodes/0.1.md says ci-{key}: {contract.get(key)!r}, the approved copy has {want!r}")
    if not re.fullmatch(r"[0-9a-f]{40}", CHECKOUT_SHA):
        p.append("the pinned actions/checkout ref is not a full 40-character commit SHA")
    if "\non:\n  push:\n  pull_request:\n\n" not in approved or approved.count("\non") != 1:
        p.append("approved copy: triggers are not exactly `on:` / `push:` / `pull_request:` with no filters")
    if approved.count("runs-on:") != 1 or f"\n    runs-on: {RUNNER}\n" not in approved:
        p.append(f"approved copy: runs-on is not exactly {RUNNER}")
    if approved.count("run:") != 2:
        p.append("approved copy: has a `run:` other than the build and test commands")
    for cmd in (BUILD, TEST):
        if f"\n        run: {cmd}\n" not in approved:
            p.append(f"approved copy: no `run: {cmd}` line")
    if approved.count("uses:") != 1 or f"\n      - uses: {CHECKOUT} # " not in approved:
        p.append(f"approved copy: the only `uses:` must be {CHECKOUT}")
    for bad in FORBIDDEN:
        if bad in approved:
            p.append(f"approved copy: contains {bad!r}")
    if not approved.endswith("\n"):
        p.append("approved copy: does not end with a newline")
    return p


def check(workflow_bytes, approved=APPROVED):
    """[] when the workflow equals the approved copy after CRLF -> LF; otherwise the diff lines and the CHANGED line."""
    try:
        text = workflow_bytes.replace(b"\r\n", b"\n").decode("utf-8")
    except UnicodeDecodeError as e:
        return [f"ci.yml is not valid UTF-8 ({e})", CHANGED]
    if text == approved:
        return []
    diff = list(difflib.unified_diff(approved.splitlines(keepends=True), text.splitlines(keepends=True),
                                     "approved (scripts/check_ci_workflow.py APPROVED)", ".github/workflows/ci.yml"))
    out = [line.rstrip("\n") if line.endswith("\n") else line + "  (no newline at end)" for line in diff]
    return out + [CHANGED]


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
    pin = pin_problems(APPROVED, contract)
    if pin:
        for p in pin:
            print(f"ci workflow: FAILED, {p}")
        return 1
    diff = check(workflow.read_bytes())
    if diff:
        print("ci workflow: FAILED, .github/workflows/ci.yml is not the approved copy")
        for line in diff:
            print(line)
        return 1
    print("ci workflow: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
