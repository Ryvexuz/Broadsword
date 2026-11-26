"""Read-only brief preflight and board-claim coverage, before the GPT coder starts."""

import importlib.util
import os
import re
import subprocess
import sys
from pathlib import Path

LINT_FLAGS = [
    "--max-lines", "400", "--max-bullets", "80", "--max-files", "80",
    "--max-done-lines", "10", "--big-file-lines", "1000", "--gpt-preflight",
    "--size-pct", "90", "--cap-margin-pct", "5",
]
LINT_TIMEOUT_SECONDS = 120


def lint_problems(root, project_root, brief_path, max_kb):
    """Run the root's unmodified lint; only exit zero admits the brief."""
    try:
        root = Path(root).resolve()
        script = root / "scripts" / "brief_lint.py"
        if not script.is_file():
            return [f"brief_lint not found: {script}"]
        if type(max_kb) is not int or max_kb <= 0:
            return ["brief_lint could not run: max_kb must be a positive integer"]
        command = [
            sys.executable, "-I", "-B", str(script), *LINT_FLAGS,
            "--brief-max-kb", str(max_kb), "--root", str(Path(project_root).resolve()),
            str(Path(brief_path).resolve()),
        ]
        result = subprocess.run(
            command, cwd=root, capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=LINT_TIMEOUT_SECONDS,
        )
    except subprocess.TimeoutExpired:
        return [f"brief_lint could not run: timeout after {LINT_TIMEOUT_SECONDS} seconds"]
    except (OSError, ValueError, RuntimeError) as error:
        return [f"brief_lint could not run: {type(error).__name__}: {error}"]
    if result.returncode == 0:
        return []
    if result.returncode == 1:
        problems = [
            line.lstrip()[:300]
            for line in (result.stdout + "\n" + result.stderr).splitlines()
            if line.startswith("  FAIL ")
        ]
        return problems[:8] or ["brief_lint failed with no FAIL line"]
    return [f"brief_lint could not run: exit {result.returncode}"]


def brief_claims(root, text):
    """Use the lint's Claims blocks and path grammar, ignoring parenthetical notes."""
    script = Path(root) / "scripts" / "brief_lint.py"
    spec = importlib.util.spec_from_file_location("brief_lint_for_start_gate", script)
    lint = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(lint)
    lines = text.splitlines()
    found = []
    seen = set()
    for block in lint.claim_blocks(lines, lint.split_sections(lines)):
        while True:
            clean = re.sub(r"\([^()]*\)", "", block)
            if clean == block:
                break
            block = clean
        for token in re.split(r"[\s,;\[\]`'\"]+", block):
            token = token.rstrip(".:").replace("\\", "/")
            path, sep, key = token.partition("#")
            if not lint.PATH_RE.fullmatch(path):
                continue
            if sep and not re.fullmatch(r"[a-z0-9_]+", key):
                continue
            if token not in seen:
                seen.add(token)
                found.append(token)
    return found


def _covers(board, claim):
    board = board.replace("\\", "/")
    if os.name == "nt":
        board, claim = board.casefold(), claim.casefold()
    if board == "*":
        return True
    if "#" in board:
        return board == claim
    path = claim.partition("#")[0].rstrip("/")
    board = board.rstrip("/")
    return path == board or path.startswith(board + "/")


def claim_problems(root, text, board_claims):
    """Every brief claim needs a covering board claim; no claims is a refusal."""
    try:
        claims = brief_claims(root, text)
    except (OSError, ImportError, SyntaxError, ValueError) as error:
        return [f"claims: could not load brief_lint: {type(error).__name__}"]
    if not claims:
        return ["claims: the brief's Claims line names no path"]
    outside = [c for c in claims if not any(_covers(b, c) for b in board_claims)]
    return ["claims: not on the board claims: " + ", ".join(outside)] if outside else []


def check(root, project, brief_path, board_claims, max_kb):
    """Return lint and coverage failures without changing the brief or starting codex."""
    try:
        text = Path(brief_path).read_text(encoding="utf-8", errors="replace")
    except (OSError, ValueError) as error:
        return [f"cannot read the brief {brief_path}: {type(error).__name__}"]
    if (not isinstance(project, str) or project in (".", "..")
            or not re.fullmatch(r"[A-Za-z0-9_.-]+", project)):
        return ["brief_lint could not run: invalid project name"]
    root = Path(root)
    project_root = root if project == "broadsword" else root / "projects" / project
    return lint_problems(root, project_root, brief_path, max_kb) + claim_problems(root, text, board_claims)
