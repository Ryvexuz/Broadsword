"""Read-only report of running tasks whose newest coder result awaits verification."""

import argparse
import pathlib
import re
import sys
import time
import tomllib


OPEN_QUEUE_STATUSES = {"queued", "running", "blocked", "stale", "approved"}
MINUTE_NS = 60 * 1_000_000_000


class ScanError(ValueError):
    """Invalid input prevents a complete report."""


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise ScanError(message)


def positive_integer(value):
    if not re.fullmatch(r"[0-9]+", value):
        raise argparse.ArgumentTypeError("min-age-min must be a positive integer")
    try:
        number = int(value)
    except ValueError as error:
        raise argparse.ArgumentTypeError("min-age-min is too large") from error
    if number <= 0:
        raise argparse.ArgumentTypeError("min-age-min must be a positive integer")
    return number


def records(folder, suffix, required=False):
    try:
        children = sorted(folder.iterdir())
    except FileNotFoundError:
        if required:
            raise ScanError(f"tasks folder does not exist: {folder}")
        return []
    result = []
    for path in children:
        if path.suffix == suffix:
            fields = {}
            for key, value in re.findall(
                r"(?m)^(\w+) = (.*)$", path.read_text(encoding="utf-8")
            ):
                if key in fields:
                    raise ScanError(f"duplicate field {key} in {path}")
                fields[key] = value
            result.append(fields)
    return result


def setting_path(root, settings, key):
    value = settings.get(key)
    if not isinstance(value, str) or not value or any(ord(c) < 32 for c in value):
        raise ScanError(f"missing or invalid {key} in broadsword.toml")
    # Recognise Windows absolute paths even when inspecting fixtures on another OS.
    windows = pathlib.PureWindowsPath(value)
    if windows.drive or windows.root or pathlib.Path(value).is_absolute():
        raise ScanError(f"{key} must be relative to root")
    path = (root / value.replace("\\", "/")).resolve()
    if not path.is_relative_to(root):
        raise ScanError(f"{key} escapes root")
    return path


def scan(root, min_age, now_ns):
    root = pathlib.Path(root).resolve()
    settings = tomllib.loads((root / "broadsword.toml").read_text(encoding="utf-8"))
    state = setting_path(root, settings, "state_dir")
    scratch = setting_path(root, settings, "scratch_dir")
    tasks = records(state / "tasks", ".task", required=True)
    entries = records(state / "queue", ".entry")
    waiting = {e.get("task") for e in entries if e.get("status") in OPEN_QUEUE_STATUSES}
    try:
        files = list(scratch.iterdir())
    except FileNotFoundError:
        files = []
    running = [task for task in tasks if task.get("status") == "running"]
    lines = []
    for task in sorted(running, key=lambda task: task.get("id", "")):
        task_id, project = task.get("id", ""), task.get("project", "")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", task_id):
            raise ScanError("running task has missing or invalid id")
        if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*", project):
            raise ScanError(f"running task {task_id} has missing or invalid project")
        suffix = re.escape(task_id) + r"(?:[-.][A-Za-z][\w.-]*)?"
        coders = [p for p in files if re.fullmatch("codex-" + suffix + r"\.txt", p.name)]
        if not coders:
            continue
        newest = max(coders, key=lambda p: (p.stat().st_mtime_ns, p.name))
        modified = newest.stat().st_mtime_ns
        age = now_ns - modified
        # Stream the output rather than loading potentially large coder logs.
        with newest.open(encoding="utf-8") as output:
            has_result = False
            for line in output:
                if re.search(r'"type"\s*:\s*"result"', line):
                    has_result = True
        verified = any(
            p.stat().st_mtime_ns >= modified
            for p in files
            if re.fullmatch("verify-" + suffix + r"\.md", p.name)
        )
        if has_result and age >= min_age * MINUTE_NS and not verified and task_id not in waiting:
            lines.append(f"CODED, NOT VERIFIED: {task_id} {project} {age // MINUTE_NS} min")
    lines.append(
        f"coded_wait: {len(lines)} of {len(running)} running task(s) "
        f"coded and not verified (min age {min_age} min)"
    )
    return lines


def main(argv=None, stdout=None, stderr=None, now_ns=None):
    stdout = sys.stdout if stdout is None else stdout
    stderr = sys.stderr if stderr is None else stderr
    parser = Parser(description=__doc__)
    parser.add_argument("root")
    parser.add_argument("--min-age-min", required=True, type=positive_integer)
    try:
        args = parser.parse_args(argv)
        lines = scan(args.root, args.min_age_min, time.time_ns() if now_ns is None else now_ns)
    except (OSError, UnicodeError, ValueError) as error:
        reason = " ".join(str(error).splitlines())
        print(f"coded_wait: {reason}", file=stderr)
        return 2
    print("\n".join(lines), file=stdout)
    return 0


if __name__ == "__main__":
    sys.exit(main())
