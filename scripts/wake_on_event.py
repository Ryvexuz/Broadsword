"""Block until queue status/step or task status changes; never modify state."""

import argparse
from pathlib import Path
import sys
import time


NANOSECONDS = 1_000_000_000


def positive_seconds(text):
    """Accept decimal whole seconds, without float conversion or overflow."""
    if not text or any(char not in "0123456789" for char in text):
        raise argparse.ArgumentTypeError("expected a whole number >= 1")
    try:
        value = int(text)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("invalid whole number of seconds") from exc
    if value < 1:
        raise argparse.ArgumentTypeError("expected a whole number >= 1")
    return value


def snapshot(root):
    """Only selected fields count; unreadable records are absent this time."""
    result = {}
    for kind, folder, suffix, fields in (
        ("queue", "queue", ".entry", ("status", "step")),
        ("task", "tasks", ".task", ("status",)),
    ):
        # Directory failures are not an empty snapshot: report them to main.
        for path in (root / "state" / folder).iterdir():
            if path.suffix != suffix:
                continue
            try:
                with path.open(encoding="utf-8", errors="replace") as record:
                    values = {}
                    for line in record:
                        key, separator, value = line.partition("=")
                        if separator and key.strip() in fields:
                            values[key.strip()] = value.strip()
            except OSError:
                continue
            result[(path.name, kind, path.stem)] = tuple(
                values.get(field) or "-" for field in fields
            )
    return result


def single_line(text):
    """Keep hostile filenames/values from injecting output lines."""
    return "".join(
        char if char.isprintable() else ascii(char)[1:-1] for char in text
    )


def changes(before, after):
    lines = []
    for key in sorted(before.keys() | after.keys()):
        if key in before and key in after and before[key] == after[key]:
            continue
        _, kind, identifier = key
        absent = ("-", "-") if kind == "queue" else ("-",)
        old = "/".join(before.get(key, absent))
        new = "/".join(after.get(key, absent))
        lines.append(single_line(f"EVENT {kind} {identifier} {old} -> {new}"))
    return lines


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--timeout-seconds", type=positive_seconds, default=1500)
    parser.add_argument("--poll-seconds", type=positive_seconds, default=15)
    args = parser.parse_args(argv)
    for folder in ("queue", "tasks"):
        path = args.root / "state" / folder
        try:
            exists = path.is_dir()
        except (OSError, ValueError):
            exists = False
        if not exists:
            parser.error(f"missing folder: {path}")
    try:
        before = snapshot(args.root)
        deadline = time.monotonic_ns() + args.timeout_seconds * NANOSECONDS
        next_poll = time.monotonic_ns() + args.poll_seconds * NANOSECONDS
        while True:
            now = time.monotonic_ns()
            remaining = min(deadline, next_poll) - now
            if remaining > 0:
                # One-second sleep slices avoid platform sleep overflow even
                # for arbitrarily large integer CLI durations.
                time.sleep(min(remaining, NANOSECONDS) / NANOSECONDS)
                continue
            lines = changes(before, snapshot(args.root))
            if lines:
                for line in lines:
                    print(line)
                print(f"woke: {len(lines)} change(s)")
                return 0
            if time.monotonic_ns() >= deadline:
                print(f"TIMEOUT after {args.timeout_seconds} s, no change")
                return 3
            next_poll = time.monotonic_ns() + args.poll_seconds * NANOSECONDS
    except (OSError, ValueError) as exc:
        print(f"wake_on_event: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
