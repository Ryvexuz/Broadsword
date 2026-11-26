"""Shared, read-only readiness rule; callers supply board TSV rows.

``brief`` is text, or None for a missing brief. ``statuses`` maps task IDs to
status strings; ``held`` contains (project, claim) pairs from running rows.
Board claims must already be normalized, as in slot_check.
"""
import pathlib
import re
import shlex
import subprocess
import sys
import tomllib


TASK = re.compile(r"(?<![\w-])t-\d{8}-\d{6}(?:-\d+)?(?![\w-])")
PREREQUISITES = re.compile(r"^\s*(?:-\s*)?(?:Prerequisites|Depends on)\s*:\s*(.*)$", re.I)
MODEL = re.compile(r"^\s*(?:-\s*)?Model\s*:\s*(.*)$", re.I)
CLAIMS = re.compile(r"^\s*(?:-\s*)?Claims\s*:\s*(.*)$", re.I)


def claims_overlap(a, b):
    """Copy of slot_check.claims_overlap: board overlaps_with, Windows case."""
    if a == "*" or b == "*":
        return True
    a, _, ak = a.lower().partition("#")
    b, _, bk = b.lower().partition("#")
    if a == b:
        return not ak or not bk or ak == bk
    return (not ak and b.startswith(a + "/")) or (not bk and a.startswith(b + "/"))


def model_route(brief):
    """Default to GPT high; preserve non-GPT model text for the caller."""
    for line in brief.splitlines():
        match = MODEL.fullmatch(line.replace("**", ""))
        if match:
            value = match[1].strip()
            if value.lower().startswith("gpt"):
                return "gpt", "low" if re.search(r"\blow\b", value, re.I) else "high"
            return "other", value
    return "gpt", "high"


def _validate_row(row):
    if (len(row) < 6 or any(not isinstance(x, str) for x in row)
            or not TASK.fullmatch(row[0]) or not row[3]
            or any(c in x for x in row for c in "\r\n\0")):
        raise ValueError("readiness: invalid board row")
    if any(not claim.strip() for claim in row[5].split(", ")):
        raise ValueError("readiness: empty board claim")


def check(row, brief, statuses, held, lint_ok):
    """Return (ok, first failing reason), with no I/O or mutation."""
    if brief is None:
        return False, "no brief"
    _validate_row(row)
    lines = [match[1].strip() for line in brief.splitlines()
             if (match := PREREQUISITES.fullmatch(line.replace("**", "")))]
    if not lines:
        return False, "no Prerequisites line"
    for line in lines:
        if re.search(r"\b(?:blocked|waits\s+on|Lazlo)\b", line, re.I):
            return False, "prerequisites blocked"
        ids = TASK.findall(line)
        if not ids and not re.match(r"none\b", line, re.I):
            return False, "prerequisites must name task ids or start with none"
        for task_id in ids:
            if task_id == row[0]:
                continue
            status = statuses.get(task_id, "unknown")
            if status != "done":
                return False, f"prerequisite {task_id} is {status}"
    for claim in row[5].split(", "):
        for project, other in held:
            if row[3] == project and claims_overlap(claim.strip(), other):
                return False, f"claim {claim.strip()} overlaps {other}"
    if not lint_ok:
        return False, "brief_lint not ok"
    # Board membership is exact, not overlap: a folder, wildcard, or whole
    # file must not silently stand in for a different brief claim.
    board_claims = {claim.strip() for claim in row[5].split(", ")}
    missing = []
    for line in brief.splitlines():
        match = CLAIMS.fullmatch(line.replace("**", ""))
        if not match:
            continue
        # Remove annotations before splitting, including nested commas.
        depth = 0
        claims_text = []
        for char in match[1]:
            if char == "(":
                depth += 1
            elif char == ")" and depth:
                depth -= 1
            elif not depth:
                claims_text.append(char)
        for item in "".join(claims_text).split(","):
            claim = item.strip().strip("`").strip()
            if claim and claim not in board_claims and claim not in missing:
                missing.append(claim)
    if missing:
        return False, "brief claims not on the board: " + ", ".join(missing)
    return True, "ok"


def _lint_args(root):
    try:
        with (root / "broadsword.toml").open("rb") as stream:
            settings = tomllib.load(stream)
        value = settings["ready_lint_args"]
        if not isinstance(value, str) or not value.strip() or any(c in value for c in "\r\n\0"):
            raise ValueError("expected nonempty, single-line arguments")
        args = shlex.split(value)
        if not args:
            raise ValueError("expected lint arguments")
        return args
    except (OSError, KeyError, ValueError) as error:
        raise SystemExit(f"readiness: ready_lint_args: {error}") from error


def startable(root, running_rows, open_rows, statuses):
    """Read briefs and batch lint once; exit loudly on configuration/I/O errors.

    Lint exit 1 permits individual exact ``<path>: ok`` results; every other
    nonzero exit rejects the entire batch. Missing or ambiguous results reject
    that brief. No caller is wired by this module.
    """
    root = pathlib.Path(root).resolve()
    args = _lint_args(root)
    running_rows, open_rows = list(running_rows), list(open_rows)
    for row in running_rows + open_rows:
        _validate_row(row)
    held = [(row[3], claim.strip()) for row in running_rows
            for claim in row[5].split(", ")]
    briefs = []
    for row in open_rows:
        path = root / "state" / "backlog" / f"{row[0]}.md"
        try:
            text = path.read_text(encoding="utf-8")
        except FileNotFoundError:
            text = None
        except (OSError, UnicodeError) as error:
            raise SystemExit(f"readiness: cannot read brief {path}: {error}") from error
        briefs.append((row, path, text))
    candidates = [path for _, path, text in briefs if text is not None]
    results = {}
    failure = None
    if candidates:
        try:
            proc = subprocess.run(
                [sys.executable, "-I", str(root / "scripts" / "brief_lint.py"),
                 *args, "--root", str(root), *map(str, candidates)],
                cwd=root, capture_output=True, text=True, encoding="utf-8",
            )
            if proc.returncode not in (0, 1):
                detail = "\n".join(part.strip() for part in (proc.stderr, proc.stdout) if part.strip())
                failure = f"brief_lint exited {proc.returncode}: {detail}"
            else:
                for line in proc.stdout.splitlines():
                    path, sep, verdict = line.rpartition(": ")
                    if sep:
                        results.setdefault(path, []).append(verdict)
        except (OSError, UnicodeError) as error:
            failure = f"brief_lint failed: {error}"
    answer = []
    for row, path, text in briefs:
        ok, reason = check(row, text, statuses, held,
                           failure is None and results.get(str(path)) == ["ok"])
        if reason == "brief_lint not ok" and failure:
            reason = failure
        answer.append((row, ok, reason))
    return answer
