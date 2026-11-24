#!/usr/bin/env python3
"""Count tick-loop stalls and ask the existing restart safety boundary to act.

Usage: python -I scripts/stall_restart.py --root <dir> [--dry-run]
This step only reads files; restart-orch.ps1 owns all restart policy and writes.
"""

from __future__ import annotations

import argparse
from pathlib import Path
import subprocess
import sys
import tomllib


def count_stalls(lines) -> int:
    """Count stalls after the last non-empty, non-tick-loop line."""
    count = 0
    for raw in lines:
        line = raw.rstrip()
        if not line:
            continue
        if not line.endswith("(tick-loop)"):
            count = 0
        elif "ORCH STALL" in line:
            count += 1
    return count


def load_settings(root: Path) -> tuple[int, int]:
    path = root / "broadsword.toml"
    with path.open("rb") as stream:
        settings = tomllib.load(stream)
    keys = ("stall_restart_lines", "stall_restart_timeout_seconds")
    for key in keys:
        if key not in settings:
            raise ValueError(f"{path}: missing required setting {key}")
        if type(settings[key]) is not int or settings[key] < 1:
            raise ValueError(f"{path}: {key} must be an integer >= 1")
    return tuple(settings[key] for key in keys)


def run(root: Path, dry_run: bool = False, *, _script_for_test: Path | None = None) -> int:
    """Validate and count before the sole spawn site; override is for tests only."""
    try:
        progress = root / "state" / "pc-progress.md"
        script = root / "scripts" / "restart-orch.ps1"
        for path in (progress, script, root / "broadsword.toml"):
            if not path.is_file():
                print(f"stall_restart: missing file {path}", file=sys.stderr)
                return 2
        threshold, timeout = load_settings(root)
    except (OSError, ValueError) as error:
        print(f"stall_restart: {error}", file=sys.stderr)
        return 2
    try:
        with progress.open(encoding="utf-8", errors="replace") as stream:
            count = count_stalls(stream)
    except OSError as error:
        print(f"stall_restart: cannot read {progress}: {error}", file=sys.stderr)
        return 1
    if count < threshold:
        print(
            f"stall_restart: {count} ORCH STALL line(s) since the last "
            "orchestrator line; no restart"
        )
        return 0
    command = [
        "powershell", "-NoProfile", "-File",
        str(_script_for_test if _script_for_test is not None else script),
    ]
    if dry_run:
        print(f"stall_restart: dry-run: {subprocess.list2cmdline(command)}")
        return 0
    try:
        result = subprocess.run(
            command, capture_output=True, text=True, encoding="utf-8",
            errors="replace", timeout=timeout, check=False,
        )
    except subprocess.TimeoutExpired:
        print(
            f"stall_restart: restart-orch timed out "
            f"(stall_restart_timeout_seconds={timeout})",
            file=sys.stderr,
        )
        return 1
    except (OSError, ValueError, OverflowError) as error:
        print(f"stall_restart: restart-orch launch error: {error}", file=sys.stderr)
        return 1
    output = result.stderr or result.stdout
    first_line = output.splitlines()[0] if output else "(no output)"
    print(
        f"stall_restart: {count} ORCH STALL lines; "
        f"restart-orch exit {result.returncode}: {first_line}"
    )
    if result.returncode == 0:
        return 0
    return 3 if result.returncode == 2 else 1


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    return run(args.root, args.dry_run)


if __name__ == "__main__":
    sys.exit(main())
