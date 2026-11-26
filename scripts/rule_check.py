#!/usr/bin/env python3
"""Usage: python -I scripts/rule_check.py [--root DIR] [--rules FILE]

Read-only standing-rule checks; only --changed FILE writes state.
--list lists the registry without running checks; --quiet hides unchecked rules.
"""
import argparse
import datetime as dt
import glob
import io
import json
import math
import os
from pathlib import Path, PureWindowsPath
import re
import shutil
import subprocess
import sys
import threading
import time
import tomllib


FIELDS = {
    "none": ({"note"}, set()),
    "file_age": ({"paths", "max_hours"}, set()),
    "git_clean": ({"repos"}, set()),
    "no_abs_paths": ({"pathspecs", "pattern", "allow"}, set()),
    "tracked_absent": ({"names"}, set()),
    "file_contains": ({"path", "text"}, set()),
    "setting_read": ({"keys", "src"}, set()),
    "heartbeat": ({"jobs", "max_json_minutes"}, set()),
    "open_tasks_have_briefs": (set(), {"exe"}),
    "disk_floor": (set(), set()),
    "cmd": ({"argv"}, {"ok_exit", "timeout_s"}),
}
LISTS = {"paths", "repos", "pathspecs", "allow", "names", "keys", "jobs", "argv"}
OWNERS = {"orchestrator", "tick", "lazlo", "reviewer", "housekeeper", "planner", "session-end"}


def string(value):
    return (isinstance(value, str) and bool(value) and "\0" not in value
            and not any(c in value for c in "\r\n\v\f\x1c\x1d\x1e\x85\u2028\u2029"))


def relative(value):
    win = PureWindowsPath(value)
    return (string(value) and not Path(value).is_absolute() and not win.drive
            and not win.root and ".." not in win.parts)


def positive(value):
    return (type(value) in (int, float) and math.isfinite(value) and value > 0)


def validate_check(check):
    if not isinstance(check, dict) or check.get("kind") not in FIELDS:
        raise ValueError("unknown kind or missing kind")
    required, optional = FIELDS[check["kind"]]
    if required - check.keys():
        raise ValueError(f"missing check key: {sorted(required - check.keys())}")
    if check.keys() - required - optional - {"kind"}:
        raise ValueError("unexpected check key")
    for key, value in check.items():
        if key in LISTS:
            if not isinstance(value, list) or (key != "allow" and not value):
                raise ValueError(f"{key} must be a nonempty list")
            if not all(string(item) for item in value):
                raise ValueError(f"{key} must contain one-line strings")
        elif key in {"max_hours", "max_json_minutes", "timeout_s"}:
            if not positive(value):
                raise ValueError(f"{key} must be finite and positive")
        elif key == "ok_exit":
            if not isinstance(value, list) or not value or any(type(v) is not int for v in value):
                raise ValueError("ok_exit must be a nonempty list of integers")
        elif not string(value):
            raise ValueError(f"{key} must be a nonempty one-line string")
    for key in ("paths", "repos"):
        if key in check and not all(relative(v) for v in check[key]):
            raise ValueError(f"{key} must be root-relative")
    for key in ("path", "src", "exe"):
        if key in check and not relative(check[key]):
            raise ValueError(f"{key} must be root-relative")
    if "pattern" in check:
        re.compile(check["pattern"])
    if "allow" in check and any(not re.fullmatch(r".+:[1-9][0-9]*", v) for v in check["allow"]):
        raise ValueError("allow entries must be path:line with a positive line number")
    if "keys" in check and not all(re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", k) for k in check["keys"]):
        raise ValueError("invalid setting key")
    if "names" in check and any("/" in n or "\\" in n or ":" in n for n in check["names"]):
        raise ValueError("names must be basenames")
    if "jobs" in check and "*" in check["jobs"] and check["jobs"] != ["*"]:
        raise ValueError("wildcard jobs must be ['*']")


def load_rules(path):
    """Load and validate the entire registry before any check can run."""
    with Path(path).open("rb") as stream:
        data = tomllib.load(stream)
    if set(data) != {"rule"} or not isinstance(data["rule"], list) or not data["rule"]:
        raise ValueError("expected nonempty [[rule]] tables")
    seen = set()
    for rule in data["rule"]:
        expected = {"id", "source", "text", "owner", "check"}
        if set(rule) != expected:
            raise ValueError(f"rule keys: missing {sorted(expected - rule.keys())}; unexpected {sorted(rule.keys() - expected)}")
        for key in expected - {"check"}:
            if not string(rule[key]):
                raise ValueError(f"invalid {key}")
        if not re.fullmatch(r"[a-z0-9-]+", rule["id"]):
            raise ValueError(f"invalid id: {rule['id']}")
        if rule["owner"] not in OWNERS:
            raise ValueError(f"invalid owner: {rule['owner']}")
        if rule["id"] in seen:
            raise ValueError(f"duplicate id: {rule['id']}")
        seen.add(rule["id"])
        validate_check(rule["check"])
    return data["rule"]


def child(argv, root, timeout=60, tail=None):
    """Bound waiting for both the process and inherited pipes by one deadline."""
    process = subprocess.Popen(argv, cwd=root, stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                               shell=False)
    outputs = [bytearray(), bytearray()]
    errors = []
    stopped = threading.Event()

    def drain(pipe, output):
        try:
            if tail is None:
                while chunk := pipe.read1(io.DEFAULT_BUFFER_SIZE):
                    if stopped.is_set():
                        break
                    output.extend(chunk)
            else:
                # cmd only needs the first 160 characters of the last nonempty
                # line; drain arbitrary output without retaining all of it.
                with io.TextIOWrapper(pipe, encoding="utf-8", errors="replace") as text:
                    prefix, last, nonempty = "", "", False
                    while chunk := text.read(io.DEFAULT_BUFFER_SIZE):
                        if stopped.is_set():
                            break
                        for char in chunk:
                            if char in "\n\r\v\f\x1c\x1d\x1e\x85\u2028\u2029":
                                if nonempty:
                                    last = prefix
                                prefix, nonempty = "", False
                            else:
                                nonempty |= not char.isspace()
                                if len(prefix) < tail:
                                    prefix += char
                    output.extend((prefix if nonempty else last).encode("utf-8"))
        except Exception as exc:
            errors.append(exc)
        finally:
            pipe.close()

    readers = [threading.Thread(target=drain, args=(pipe, output), daemon=True)
               for pipe, output in zip((process.stdout, process.stderr), outputs)]
    deadline = time.monotonic() + timeout
    for reader in readers:
        reader.start()
    try:
        process.wait(timeout=max(0, deadline - time.monotonic()))
        for reader in readers:
            reader.join(timeout=max(0, deadline - time.monotonic()))
        if any(reader.is_alive() for reader in readers):
            raise subprocess.TimeoutExpired(argv, timeout)
    except subprocess.TimeoutExpired:
        stopped.set()
        if process.poll() is None:
            process.kill()
            process.wait()
        # Do not close a pipe locked by a reader or wait for a grandchild's EOF.
        raise
    if errors:
        raise errors[0]
    return subprocess.CompletedProcess(argv, process.returncode, *(bytes(v) for v in outputs))


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def git(root, *args):
    result = child(["git", "--no-optional-locks", "-C", str(root), *args], root)
    if result.returncode:
        raise RuntimeError(result.stderr.decode("utf-8", "replace").strip())
    return result.stdout.decode("utf-8", "surrogateescape")


def missing_briefs(root, output):
    """Parse task-list TSV and return ids whose brief is absent."""
    missing = []
    for line in output.splitlines():
        if not line.strip():
            continue
        task_id = line.split("\t")[0]
        if not re.fullmatch(r"[a-zA-Z0-9_-]+", task_id):
            raise ValueError(f"invalid task id: {task_id!r}")
        if not (root / "state" / "backlog" / f"{task_id}.md").is_file():
            missing.append(task_id)
    return missing


def evaluate(root, check):
    kind = check["kind"]
    issues = []
    now = dt.datetime.now(dt.timezone.utc)
    if kind == "none":
        return "unchecked", check["note"]
    if kind == "file_age":
        matches = [Path(p) for pat in check["paths"]
                   for p in glob.glob(str(root / pat), recursive=True) if Path(p).is_file()]
        if not matches:
            issues.append(f"no file matches {check['paths']}")
        else:
            newest = max(matches, key=lambda p: p.stat().st_mtime)
            age = (now.timestamp() - newest.stat().st_mtime) / 3600
            if age > check["max_hours"]:
                issues.append(f"{newest.relative_to(root)} is {age:.1f} h old (max {check['max_hours']})")
    elif kind == "git_clean":
        for repo in check["repos"]:
            path = root / repo
            if not path.is_dir():
                issues.append(f"{repo}: missing dir")
                continue
            output = git(path, "status", "--porcelain", "--untracked-files=no")
            if output:
                issues.append(f"{repo}: {output.splitlines()[0][3:]}")
    elif kind in {"no_abs_paths", "tracked_absent"}:
        specs = check.get("pathspecs", [])
        files = [p for p in git(root, "ls-files", "-z", "--", *specs).split("\0") if p]
        if kind == "tracked_absent":
            issues.extend(p for p in files if Path(p).name in check["names"])
        else:
            pattern = re.compile(check["pattern"])
            hits = []
            for path in files:
                try:
                    content = (root / path).read_bytes()
                    if b"\0" in content:
                        continue
                    text = content.decode("utf-8")
                except (OSError, UnicodeError):
                    continue
                hits.extend(f"{path}:{i}" for i, line in enumerate(text.splitlines(), 1)
                            if pattern.search(line) and f"{path}:{i}" not in check["allow"])
            if hits:
                issues.append(f"{len(hits)} hits: {', '.join(hits[:5])}")
    elif kind == "file_contains":
        path = root / check["path"]
        if not path.is_file():
            issues.append(f"missing file: {check['path']}")
        elif check["text"] not in path.read_text(encoding="utf-8"):
            issues.append(f"{check['path']} lacks {check['text']!r}")
    elif kind == "setting_read":
        config = (root / "broadsword.toml").read_text(encoding="utf-8")
        sources = [p.read_text(encoding="utf-8") for p in (root / check["src"]).rglob("*.rs")]
        for key in check["keys"]:
            if not re.search(rf"(?m)^\s*{re.escape(key)}\s*=", config):
                issues.append(f"{key}: not in broadsword.toml")
            if not any(re.search(rf"\b{re.escape(key)}\b", text) for text in sources):
                issues.append(f"{key}: read by no .rs file under {check['src']}")
    elif kind == "heartbeat":
        data = json.loads((root / "state/heartbeats.json").read_text(encoding="utf-8"),
                          object_pairs_hook=unique_object)
        stamp = data["written_utc"]
        if not isinstance(stamp, str) or not stamp.endswith("Z"):
            raise ValueError("written_utc must be a UTC timestamp ending Z")
        written = dt.datetime.fromisoformat(stamp[:-1] + "+00:00")
        age = (now - written).total_seconds() / 60
        if age > check["max_json_minutes"]:
            issues.append(f"heartbeats.json is {age:.1f} min old (max {check['max_json_minutes']})")
        jobs = data["jobs"]
        if not isinstance(jobs, dict) or (check["jobs"] == ["*"] and not jobs):
            raise ValueError("jobs must be a nonempty object")
        for name in (jobs if check["jobs"] == ["*"] else check["jobs"]):
            if name not in jobs:
                issues.append(f"{name}: absent")
                continue
            job = jobs[name]
            result = job.get("result", "not yet run")
            if type(job["overdue"]) is not bool or not isinstance(result, str):
                raise ValueError(f"invalid heartbeat job: {name}")
            if job["overdue"]:
                issues.append(f"{name}: {result[:120]}")
    elif kind == "open_tasks_have_briefs":
        exe = root / check.get("exe", "build/bin/broadsword.exe")
        if not exe.is_file():
            issues.append(f"executable missing: {exe}")
        else:
            result = child([str(exe), "task", "list", "--status", "open"], root)
            if result.returncode:
                raise RuntimeError(f"task list exit {result.returncode}: {result.stderr.decode('utf-8', 'replace')}")
            missing = missing_briefs(root, result.stdout.decode("utf-8"))
            if missing:
                issues.append(f"{len(missing)} missing briefs: {', '.join(missing[:10])}")
    elif kind == "disk_floor":
        with (root / "broadsword.toml").open("rb") as stream:
            floor = tomllib.load(stream)["disk_floor_gb"]
        if not positive(floor):
            raise ValueError("disk_floor_gb must be finite and positive")
        free = shutil.disk_usage(root).free / 1e9
        detail = f"free {free:.3f} GB, disk_floor_gb {floor}"
        return ("ok" if free >= floor else "broken"), detail
    elif kind == "cmd":
        replacements = {"{root}": str(root), "{python}": sys.executable,
                        "{date_minus_7d}": (now - dt.timedelta(days=7)).strftime("%Y-%m-%d")}
        argv = check["argv"][:]
        for index, arg in enumerate(argv):
            for key, value in replacements.items():
                arg = arg.replace(key, value)
            argv[index] = arg
        if check["argv"][0] == "{python}":
            scripts = [a for a in argv[1:] if not a.startswith("-")]
            if scripts and not (root / scripts[0]).is_file():
                return "broken", f"check script missing: {scripts[0]}"
        timeout = check.get("timeout_s", 60)
        try:
            result = child(argv, root, timeout, tail=160)
        except subprocess.TimeoutExpired:
            return "broken", f"timed out after {timeout} s"
        if result.returncode not in check.get("ok_exit", [0]):
            last = (result.stdout or result.stderr).decode("utf-8", "replace")
            issues.append(f"exit {result.returncode}: {last[:160]}")
    return ("broken", "; ".join(issues)) if issues else ("ok", "")


def run_check(root, check):
    """Single fail-closed entry point, including for imported callers."""
    try:
        validate_check(check)
        status, detail = evaluate(Path(root).resolve(), check)
        return status, " ".join(detail.splitlines())
    except Exception as exc:
        return "broken", f"check crashed: {type(exc).__name__}: {' '.join(str(exc).splitlines())}"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parent.parent)
    parser.add_argument("--rules", type=Path)
    parser.add_argument("--quiet", action="store_true")
    parser.add_argument("--list", action="store_true")
    parser.add_argument("--changed", type=Path)
    args = parser.parse_args(argv)
    rules_path = args.rules or args.root / "docs/RULES.toml"
    try:
        rules = load_rules(rules_path)
    except Exception as exc:
        print(f"rule_check: {rules_path}: {exc}")
        return 1
    if args.list:
        for rule in rules:
            print(f"{rule['id']}\t{rule['owner']}\t{rule['check']['kind']}")
        return 0
    previous = set()
    try:
        if args.changed and args.changed.exists():
            saved = json.loads(args.changed.read_text(encoding="utf-8"))
            if not isinstance(saved, list) or not all(string(v) for v in saved):
                raise ValueError("changed state must be a list of ids")
            previous = set(saved)
    except Exception as exc:
        print(f"rule_check: {args.changed}: {exc}")
        return 1
    counts = dict(ok=0, broken=0, unchecked=0)
    broken = set()
    for rule in rules:
        status, detail = run_check(args.root, rule["check"])
        counts[status] += 1
        ident, owner = rule["id"], rule["owner"]
        if status == "broken":
            broken.add(ident)
            if ident not in previous:
                print(f"RULE BROKEN {ident}: {detail} (owner {owner})")
        elif status == "unchecked" and not args.quiet:
            print(f"RULE UNCHECKED {ident}: {rule['text'][:80]} (owner {owner})")
    state_error = False
    if args.changed:
        for ident in sorted(previous - broken):
            print(f"RULE FIXED {ident}")
        try:
            temporary = args.changed.with_name(args.changed.name + ".tmp")
            temporary.write_text(json.dumps(sorted(broken)) + "\n", encoding="utf-8")
            os.replace(temporary, args.changed)
        except Exception as exc:
            print(f"rule_check: {args.changed}: {exc}")
            state_error = True
    print(f"rules: {len(rules)} total, {counts['ok']} ok, {counts['broken']} broken, {counts['unchecked']} unchecked")
    return 1 if state_error else (2 if broken else 0)


if __name__ == "__main__":
    sys.exit(main())
