"""Report running claims with no recent commit or worktree edit; never mutate state."""
import argparse
from datetime import datetime, timezone
from decimal import Decimal, InvalidOperation
import os
from pathlib import Path, PureWindowsPath
import subprocess
import time
import tomllib


HEADER = "id\tproject\towner\tidle_hours\tlast_activity_utc\tclaims\tnote"
# Unix seconds representable by the UTC output format (years 1970 through 9999).
MAX_TIMESTAMP = 253402300799


def timestamp(value):
    try:
        result = Decimal(str(value))
    except InvalidOperation:
        raise ValueError("timestamp must be finite Unix seconds") from None
    if not result.is_finite() or not 0 <= result <= MAX_TIMESTAMP:
        raise ValueError("timestamp must be Unix seconds in years 1970 through 9999")
    return result


def cell(value):
    """Keep untrusted record fields and diagnostics within one TSV cell."""
    return str(value).replace("\\", "\\\\").replace("\t", "\\t").replace(
        "\r", "\\r").replace("\n", "\\n")


def git(path, *args):
    env = dict(os.environ, GIT_OPTIONAL_LOCKS="0")
    try:
        proc = subprocess.run(
            ["git", "-C", str(path), *args], env=env, capture_output=True)
    except OSError as exc:
        return None, f"git: {exc}"
    if proc.returncode:
        lines = proc.stderr.decode("utf-8", "replace").splitlines()
        return None, "git: " + (lines[0] if lines else f"exit {proc.returncode}")
    return proc.stdout, ""


def inside(root, value, name, git_path=False):
    """Accept relative board paths, including Windows separators, within root."""
    forbidden = "\0" if git_path else "\0\r\n"
    if not isinstance(value, str) or not value or any(c in value for c in forbidden):
        raise ValueError(f"{name}: expected a nonempty relative path")
    win = PureWindowsPath(value)
    path = Path(value if git_path else value.replace("\\", "/"))
    windows_absolute = (not git_path or os.name == "nt") and (win.drive or win.root)
    if windows_absolute or path.is_absolute() or ".." in path.parts:
        raise ValueError(f"{name}: path must stay inside root")
    result = root / path
    if not result.resolve().is_relative_to(root.resolve()):
        raise ValueError(f"{name}: path must stay inside root")
    return result


def settings(root):
    try:
        data = tomllib.loads((root / "broadsword.toml").read_text(encoding="utf-8"))
    except (OSError, UnicodeError, tomllib.TOMLDecodeError) as exc:
        raise ValueError(f"broadsword.toml: {exc}") from exc
    hours = data.get("stale_claim_hours")
    if type(hours) is not int or hours < 1:
        raise ValueError("stale_claim_hours: required whole number >= 1")
    for key in ("state_dir", "projects_dir"):
        inside(root, data.get(key), key)
    return data


def read_task(path):
    fields = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        key, sep, value = line.partition("=")
        if not sep or not key.strip() or key.strip() in fields:
            raise ValueError("malformed or duplicate record field")
        fields[key.strip()] = value.strip()
    if fields.get("status") != "running":
        return None
    value = fields.get("started", "")
    if not value.isascii() or not value.isdecimal():
        raise ValueError("started: required integer Unix seconds")
    fields["started"] = timestamp(value)
    return fields


def last_activity(root, cfg, task):
    last = task["started"]
    notes = []
    branch = task.get("branch")
    if branch:
        project = task.get("project", "")
        try:
            repo = root if project == "broadsword" else inside(
                inside(root, cfg["projects_dir"], "projects_dir"), project, "project")
            out, error = git(repo, "log", "-1", "--format=%ct", "--end-of-options", branch, "--")
            if error:
                notes.append(error)
            else:
                last = max(last, timestamp(out.decode("ascii").strip()))
        except (ValueError, UnicodeError) as exc:
            notes.append(f"git: {exc}")
    if task.get("worktree"):
        try:
            worktree = inside(root, task["worktree"], "worktree")
            if worktree.is_dir():
                # -z avoids quoted paths and makes spaces, tabs, and renames unambiguous.
                out, error = git(worktree, "status", "--porcelain", "-z", "--untracked-files=all")
                if error:
                    notes.append(error)
                else:
                    worktree_last = last
                    entries = iter(out.split(b"\0"))
                    for entry in entries:
                        if not entry:
                            continue
                        if len(entry) < 4 or entry[2:3] != b" ":
                            raise ValueError("malformed porcelain status")
                        if b"R" in entry[:2] or b"C" in entry[:2]:
                            next(entries)  # The second path is the rename/copy source.
                        name = os.fsdecode(entry[3:])
                        path = inside(worktree, name, "status path", git_path=True)
                        try:
                            mtime = Decimal(path.lstat().st_mtime_ns) / 1000000000
                        except FileNotFoundError:
                            continue  # Deleted paths have no mtime.
                        worktree_last = max(worktree_last, timestamp(mtime))
                    last = worktree_last
        except (OSError, ValueError, StopIteration) as exc:
            notes.append(f"git: {exc}")
    return last, "; ".join(notes)


def report(root, cfg, now):
    rows, skipped, running = [], [], 0
    for path in sorted(inside(root, cfg["state_dir"], "state_dir").joinpath("tasks").glob("*.task")):
        try:
            task = read_task(path)
        except (OSError, UnicodeError, ValueError) as exc:
            skipped.append(f"skipped {cell(path)}: {cell(exc)}")
            continue
        if task is None:
            continue
        running += 1
        last, note = last_activity(root, cfg, task)
        idle = now - last
        if idle >= cfg["stale_claim_hours"] * 3600:
            utc = datetime.fromtimestamp(int(last), timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
            claims = ",".join(task.get("claims", "").split("\\n"))
            rows.append((idle, [
                task.get("id", path.stem), task.get("project", ""), task.get("owner", ""),
                f"{idle / 3600:.1f}", utc, claims, note]))
    rows.sort(key=lambda row: row[0], reverse=True)
    return [HEADER, *skipped, *("\t".join(map(cell, row)) for _, row in rows),
            f"stale claims: {len(rows)} of {running} running tasks idle >= "
            f"{cfg['stale_claim_hours']} h; {len(skipped)} records skipped"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root")
    parser.add_argument("--now")
    args = parser.parse_args()
    try:
        if args.root is None:
            out, error = git(Path.cwd(), "rev-parse", "--path-format=absolute", "--git-common-dir")
            if error:
                raise ValueError(error)
            root = Path(os.fsdecode(out).strip()).parent
        else:
            root = Path(args.root).resolve()
        cfg = settings(root)
        now = timestamp(args.now if args.now is not None else time.time())
        print("\n".join(report(root, cfg, now)))
    except (OSError, ValueError) as exc:
        parser.error(str(exc))


if __name__ == "__main__":
    main()
