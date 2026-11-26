"""Read Actions results without dispatching or modifying a run."""

import argparse
import csv
from datetime import datetime, timezone
import json
from pathlib import Path
import re
import subprocess


class VerifyError(ValueError):
    """Invalid command output or API data."""


def field(value, name):
    if not isinstance(value, str) or not value or any(
        ord(char) < 32 or ord(char) == 127 or char in "\u0085\u2028\u2029"
        for char in value
    ):
        raise VerifyError(f"invalid {name}")
    return value


def stamp(value):
    field(value, "timestamp")
    try:
        result = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as exc:
        raise VerifyError("invalid timestamp") from exc
    if result.tzinfo is None:
        raise VerifyError("timestamp missing timezone")
    return result


def milliseconds(value):
    if type(value) is not int or value < 0:
        raise VerifyError("invalid milliseconds")
    return value


def minutes(value):
    # Integer arithmetic keeps huge JSON integers and tiny durations finite.
    tenths, remainder = divmod(value, 6000)
    if remainder > 3000 or (remainder == 3000 and tenths % 2):
        tenths += 1
    return f"{tenths // 10}.{tenths % 10}"


def elapsed(start, end):
    delta = end - start
    if delta.days < 0:
        raise VerifyError("negative elapsed time")
    return (delta.days * 86400 + delta.seconds) * 1000 + delta.microseconds // 1000


def invoke(run, argv, cwd):
    result = run(argv, cwd=cwd, capture_output=True, text=True, timeout=60)
    if result.returncode:
        lines = (result.stderr or "").splitlines()
        raise VerifyError(lines[0] if lines else "command returned non-zero")
    return result.stdout


def api_json(run, argv, cwd):
    try:
        return json.loads(invoke(run, argv, cwd))
    except (ValueError, RecursionError) as exc:
        if isinstance(exc, VerifyError):
            raise
        raise VerifyError("bad JSON") from exc


def measure(args, run, sha):
    runs = api_json(run, [
        "gh", "run", "list", "--commit", sha, "--workflow", args.workflow,
        "--limit", "20", "--json",
        "databaseId,status,conclusion,createdAt,updatedAt,headBranch,event",
    ], args.repo_dir)
    if not isinstance(runs, list):
        raise VerifyError("invalid run list")
    if not runs:
        return 5, f"gh-verify: {sha[:10]} {args.workflow} no run", [
            "-", "no run", "-", "unknown", "unknown", "unknown",
        ]
    picked = max(runs, key=lambda item: stamp(item["createdAt"]))
    run_id = picked["databaseId"]
    if type(run_id) is not int or run_id <= 0:
        raise VerifyError("invalid run id")
    status = field(picked["status"], "status")
    conclusion = field(picked["conclusion"] or "-", "conclusion")
    branch = field(picked["headBranch"], "branch")
    created = stamp(picked["createdAt"])
    endpoint = f"repos/{{owner}}/{{repo}}/actions/runs/{run_id}"
    detail = api_json(run, ["gh", "api", endpoint], args.repo_dir)
    started = detail.get("run_started_at")
    queue = wall = "unknown"
    if started is not None:
        started = stamp(started)
        queue = minutes(elapsed(created, started))
        wall = minutes(elapsed(started, stamp(detail["updated_at"])))
    billed = "unknown"
    code = 4
    if status == "completed":
        code = 0 if conclusion == "success" else 3
        timing = api_json(run, ["gh", "api", endpoint + "/timing"], args.repo_dir)
        if "run_duration_ms" in timing:
            wall = minutes(milliseconds(timing["run_duration_ms"]))
        billable = timing.get("billable")
        if billable is not None and not isinstance(billable, dict):
            raise VerifyError("invalid billable")
        if billable:
            billed = minutes(sum(
                milliseconds(entry["total_ms"]) for entry in billable.values()
            ))
    line = (
        f"gh-verify: {sha[:10]} {args.workflow} {status} {conclusion} "
        f"run={run_id} branch={branch} queue_min={queue} wall_min={wall} "
        f"billable_min={billed}"
    )
    return code, line, [str(run_id), status, conclusion, queue, wall, billed]


def main(argv=None, run=subprocess.run):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo-dir", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--workflow", default="ci")
    parser.add_argument("--log")
    args = parser.parse_args(argv)
    try:
        field(args.sha, "commit")
        field(args.workflow, "workflow")
        field(args.repo_dir, "repo directory")
        if args.sha.startswith("-"):
            raise VerifyError("invalid commit")
        try:
            sha = invoke(run, [
                "git", "-C", args.repo_dir, "rev-parse", "--verify",
                "--end-of-options", args.sha + "^{commit}",
            ], args.repo_dir).strip()
        except VerifyError:
            print(f"gh-verify: unknown commit {args.sha}")
            return 1
        if not re.fullmatch(r"[0-9a-fA-F]{40}|[0-9a-fA-F]{64}", sha):
            raise VerifyError("invalid resolved commit")
        code, line, row = measure(args, run, sha)
        if args.log:
            name = field(Path(args.repo_dir).resolve().name, "repo name")
            with open(args.log, "a+", encoding="utf-8", newline="") as log:
                log.seek(0, 2)
                writer = csv.writer(log, delimiter="\t", lineterminator="\n")
                if log.tell() == 0:
                    writer.writerow([
                        "utc", "repo_dir_name", "sha10", "run", "status",
                        "conclusion", "queue_min", "wall_min", "billable_min",
                    ])
                writer.writerow([
                    datetime.now(timezone.utc).isoformat(), name, sha[:10], *row,
                ])
        print(line)
        return code
    except (ValueError, OSError, subprocess.SubprocessError, KeyError,
            TypeError, AttributeError, OverflowError) as exc:
        message = str(exc).splitlines()
        print(f"gh-verify: gh failed: {message[0] if message else 'invalid response'}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
