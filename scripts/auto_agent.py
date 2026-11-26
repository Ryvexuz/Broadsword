#!/usr/bin/env python3
"""Launch the two overdue recurring agents, without interactive permissions."""

from __future__ import annotations

import argparse
from contextlib import contextmanager
from datetime import datetime, timezone
import errno
import json
import math
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import threading
import time
import tomllib


JOBS = ("process_audit", "batch_review")
SETTING_KEYS = (
    "auto_agent_timeout_minutes",
    "auto_agent_min_gap_minutes",
    "auto_agent_max_starts_per_day",
)
# The heartbeat freshness window is part of the tick's input contract.
HEARTBEAT_MAX_AGE_MINUTES = 30
# poll/WaitForSingleObject use bounded millisecond counters. This is an OS
# conversion ceiling per wait, NOT a limit on the configured overall duration.
MAX_WAIT_SECONDS = min(int(threading.TIMEOUT_MAX), ((1 << 31) - 1) // 1000)
COMMON_TOOLS = (
    "Read", "Grep", "Glob",
    "Bash(git log *)",
    "Bash(git show *)",
    "Bash(git diff *)",
    "Bash(git -C * log *)",
    "Bash(git -C * show *)",
    "Bash(build/bin/broadsword.exe queue list*)",
    "Bash(build/bin/broadsword.exe queue show *)",
    "Bash(build/bin/broadsword.exe task list*)",
    "Bash(build/bin/broadsword.exe task show *)",
    "Bash(build/bin/broadsword.exe task check *)",
    "Bash(build/bin/broadsword.exe task add *)",
    "Bash(python -I scratch/slot_check.py *)",
    "Bash(python -I state/staging/exe_staleness.py *)",
    "Bash(python -I state/staging/reviewer-patterns/stats2.py*)",
    "Bash(powershell -NoProfile -File scratch/resource-check.ps1*)",
    "Write(state/backlog/*.md)",
)
EXTRA_TOOLS = {
    "process_audit": ("Write(state/staging/process-audit/audit-*.md)",),
    "batch_review": (
        "Write(state/staging/batch-review/batch-*.md)",
        "Bash(gh run list *)",
        "Bash(gh run view *)",
        "Bash(build/bin/broadsword.exe run crash-sim -- cargo test *)",
    ),
}


def say(message: str, *, error: bool = False) -> None:
    try:
        print(message, file=sys.stderr if error else sys.stdout)
    except BrokenPipeError:
        # Losing the tick's console must not interrupt cleanup or release locks.
        pass


def one_line(message: str) -> str:
    """Escape controls, including hostile filenames, before progress logging."""
    return json.dumps(message, ensure_ascii=True)[1:-1]


def utc_now() -> datetime:
    return datetime.now(timezone.utc)


def iso_time(value: datetime) -> str:
    return value.isoformat(timespec="microseconds").replace("+00:00", "Z")


def parse_time(value: object) -> datetime:
    if not isinstance(value, str) or not re.fullmatch(
        r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?Z", value
    ):
        raise ValueError("invalid ISO UTC timestamp")
    return datetime.fromisoformat(value.replace("Z", "+00:00"))


def age_microseconds(now: datetime, then: datetime) -> int:
    delta = now - then
    return (delta.days * 86400 + delta.seconds) * 1_000_000 + delta.microseconds


def validated_root(root: Path) -> Path:
    root = Path(root).resolve(strict=True)
    if not root.is_dir() or any(
        ord(char) < 32 or char in "\u0085\u2028\u2029" for char in str(root)
    ):
        raise ValueError("invalid root directory")
    return root


def local_path(root: Path, relative: str) -> Path:
    """Refuse existing links/reparse points in script-owned runtime paths."""
    path = root / relative
    for parent in (path, *path.parents):
        if parent == root:
            break
        try:
            info = parent.lstat()
        except FileNotFoundError:
            continue
        if parent.is_symlink() or getattr(info, "st_file_attributes", 0) & 0x400:
            raise ValueError(f"linked runtime path: {parent}")
    if not path.resolve().is_relative_to(root):
        raise ValueError(f"runtime path escapes root: {path}")
    return path


def load_settings(root: Path) -> tuple[int, int, int]:
    with local_path(root, "broadsword.toml").open("rb") as stream:
        settings = tomllib.load(stream)
    for key in SETTING_KEYS:
        if key not in settings:
            raise ValueError(f"missing required setting {key}")
        if type(settings[key]) is not int or settings[key] < 1:
            raise ValueError(f"{key} must be an integer >= 1")
    return tuple(settings[key] for key in SETTING_KEYS)


@contextmanager
def file_lock(path: Path, *, create: bool = True):
    """Lock byte zero; an absent read-only probe never creates a file."""
    try:
        stream = path.open("a+b" if create else "r+b")
    except FileNotFoundError:
        if create:
            raise
        yield None
        return
    locked = False
    try:
        stream.seek(0)
        try:
            if os.name == "nt":
                import msvcrt
                msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            locked = True
        except OSError as error:
            if error.errno not in (errno.EACCES, errno.EAGAIN, errno.EDEADLK):
                raise
        yield stream if locked else False
    finally:
        try:
            if locked:
                stream.seek(0)
                if os.name == "nt":
                    import msvcrt
                    msvcrt.locking(stream.fileno(), msvcrt.LK_UNLCK, 1)
                else:
                    import fcntl
                    fcntl.flock(stream.fileno(), fcntl.LOCK_UN)
        finally:
            stream.close()


def job_lock_path(root: Path, job: str) -> Path:
    if job not in JOBS:
        raise ValueError(f"invalid job: {job}")
    return local_path(root, f"state/auto-agent-{job}.lock")


def tools_for(job: str) -> tuple[str, ...]:
    if job not in JOBS:
        raise ValueError(f"invalid job: {job}")
    return COMMON_TOOLS + EXTRA_TOOLS[job]


def claude_argv(executable: str, job: str) -> list[str]:
    return [
        executable, "-p", "--model", "opus", "--permission-mode", "dontAsk",
        "--permission-prompts", "none", "--setting-sources", "project",
        "--strict-mcp-config", "--no-session-persistence", "--output-format",
        "json", "--allowedTools", *tools_for(job),
    ]


def find_claude() -> str | None:
    found = shutil.which("claude")
    if found:
        return found
    fallback = Path.home() / ".local" / "bin" / "claude.exe"
    return str(fallback) if fallback.is_file() else None


def brief_relative(job: str) -> str:
    return f"state/staging/{job.replace('_', '-')}/BRIEF.md"


def prompt_for(root: Path, job: str) -> str:
    brief = local_path(root, brief_relative(job)).read_text(encoding="utf-8")
    patterns = ", ".join(tool for tool in tools_for(job) if tool.startswith("Bash("))
    return (
        "You were started by the Broadsword tick loop (scripts/auto_agent.py) "
        f"because state/heartbeats.json marks {job} overdue. Root: {root}. "
        "You run headless: nobody answers questions, and any tool outside your "
        "allowed list is denied. Use exactly these command forms (forward slashes): "
        f"{patterns}. Follow the brief below; finish by writing your output file.\n\n"
        f"{brief}"
    )


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def reject_constant(value):
    raise ValueError(f"invalid JSON constant: {value}")


def read_heartbeats(root: Path, now: datetime) -> dict:
    with local_path(root, "state/heartbeats.json").open(encoding="utf-8") as stream:
        data = json.load(
            stream, object_pairs_hook=unique_object, parse_constant=reject_constant
        )
    if not isinstance(data, dict) or not isinstance(data.get("jobs"), dict):
        raise ValueError("invalid heartbeat object/jobs")
    age = age_microseconds(now, parse_time(data.get("written_utc")))
    if age < 0 or age > HEARTBEAT_MAX_AGE_MINUTES * 60 * 1_000_000:
        raise ValueError("written_utc outside freshness window")
    for job in JOBS:
        if job in data["jobs"] and not isinstance(data["jobs"][job], dict):
            raise ValueError(f"invalid heartbeat job: {job}")
    return data["jobs"]


def start_command(root: Path, job: str) -> list[str]:
    return [
        sys.executable, "-I", str(Path(__file__).resolve()),
        "--root", str(root), "--run", job,
    ]


def start_detached(root: Path, job: str) -> int:
    options = (
        {"creationflags": subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP}
        if os.name == "nt" else {"start_new_session": True}
    )
    proc = subprocess.Popen(
        start_command(root, job), cwd=root, stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True,
        **options,
    )
    return proc.pid


def gate_reason(root: Path, job: str, now: datetime, gap: int, cap: int, ledger) -> str | None:
    last = local_path(root, f"state/auto-agent-{job}.last")
    try:
        last_time = parse_time(last.read_bytes().decode("utf-8").removesuffix("\n"))
    except FileNotFoundError:
        pass
    else:
        if last_time > now:
            raise ValueError("future last-start timestamp")
        if age_microseconds(now, last_time) < gap * 60 * 1_000_000:
            return f"minimum gap ({gap} min)"
    count = 0
    if ledger is not None:
        ledger.seek(0)
        for raw in ledger:
            fields = raw.decode("utf-8").removesuffix("\n").split("\t")
            if len(fields) != 2 or fields[1] not in JOBS:
                raise ValueError("invalid auto-agent-starts.tsv row")
            when = parse_time(fields[0])
            if when > now:
                raise ValueError("future auto-agent-starts.tsv timestamp")
            if fields[1] == job and when.date() == now.date():
                count += 1
    if count >= cap:
        return f"daily cap ({cap})"
    return None


def check(root: Path, dry_run: bool = False) -> int:
    try:
        root = validated_root(root)
        _, gap, cap = load_settings(root)
    except (OSError, ValueError) as error:
        say(f"auto_agent: {error}", error=True)
        return 2
    now = utc_now()
    try:
        jobs = read_heartbeats(root, now)
    except (OSError, ValueError, RecursionError) as error:
        say(
            f"auto_agent: heartbeats unreadable or stale "
            f"({one_line(str(error))}); nothing started"
        )
        return 0
    for job in JOBS:
        if jobs.get(job, {}).get("overdue") is not True:
            continue
        try:
            # The ledger is the shared resource, not a job label. Serialize
            # reservations so two simultaneous ticks cannot both pass the gap.
            ledger_path = local_path(root, "state/auto-agent-starts.tsv")
            with file_lock(ledger_path, create=not dry_run) as ledger:
                if ledger is False:
                    say(f"auto_agent: skipped {job}: starts ledger locked")
                    continue
                with file_lock(job_lock_path(root, job), create=not dry_run) as lock:
                    if lock is False:
                        say(f"auto_agent: skipped {job}: already running")
                        continue
                reason = gate_reason(root, job, now, gap, cap, ledger)
                if reason:
                    say(f"auto_agent: skipped {job}: {reason}")
                    continue
                if dry_run:
                    command = subprocess.list2cmdline(start_command(root, job))
                    say(f"auto_agent: dry-run: {command}")
                    executable = find_claude() or "claude"
                    command = subprocess.list2cmdline(claude_argv(executable, job))
                    say(f"auto_agent: dry-run claude: {command}")
                    continue
                # Reserve before spawning; a failed launch still consumes its
                # reservation, rather than creating a paid-run retry loop.
                stamp = iso_time(now)
                local_path(root, f"state/auto-agent-{job}.last").write_text(
                    stamp + "\n", encoding="utf-8", newline="\n"
                )
                ledger.seek(0, os.SEEK_END)
                ledger.write(f"{stamp}\t{job}\n".encode("utf-8"))
                ledger.flush()
                pid = start_detached(root, job)
                say(f"auto_agent: started {job} run pid {pid}")
        except (OSError, ValueError, OverflowError) as error:
            say(f"auto_agent: skipped {job}: {one_line(str(error))}; nothing started")
    return 0


def snapshot(root: Path) -> dict[str, tuple[int, int]]:
    folder = local_path(root, "state/backlog")
    result = {}
    for path in folder.glob("*.md"):
        path = local_path(root, str(path.relative_to(root)))
        info = path.stat()
        result[path.name] = (info.st_size, info.st_mtime_ns)
    return result


def changed_briefs(root: Path, before: dict[str, tuple[int, int]]) -> list[str]:
    after = snapshot(root)
    return sorted(name for name, info in before.items() if after.get(name) != info)


def progress(root: Path, message: str) -> None:
    path = local_path(root, "state/pc-progress.md")
    with path.open("a", encoding="utf-8", newline="\n") as stream:
        stream.write(f"{iso_time(utc_now())} {one_line(message)} (tick-loop)\n")


def kill_tree(proc: subprocess.Popen, timeout_seconds: int) -> None:
    if os.name == "nt":
        result = subprocess.run(
            ["taskkill", "/T", "/F", "/PID", str(proc.pid)],
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            timeout=min(timeout_seconds, MAX_WAIT_SECONDS), check=False,
        )
        if result.returncode != 0:
            raise OSError(f"taskkill failed ({result.returncode}): {result.stderr!r}")
    else:
        os.killpg(proc.pid, signal.SIGKILL)
    proc.wait(timeout=min(timeout_seconds, MAX_WAIT_SECONDS))


def timeout_nanoseconds(seconds: int | float) -> int:
    """Exact conversion; positive sub-nanosecond test budgets expire at once."""
    numerator, denominator = seconds.as_integer_ratio()
    return numerator * 1_000_000_000 // denominator


def feed_stdin(pipe, data: bytes, done: threading.Event, errors: list) -> None:
    # Windows communicate() can block writing stdin before it checks a timeout.
    # Only this daemon thread touches the pipe; the main thread always retains
    # the ability to kill the tree, even if a descendant keeps stdin open.
    try:
        pipe.write(data)
    except (OSError, ValueError) as error:
        errors.append(error)
    finally:
        try:
            pipe.close()
        except (OSError, ValueError) as error:
            errors.append(error)
        done.set()


def communicate(proc: subprocess.Popen, written: threading.Event, seconds: int | float) -> None:
    # All deadline arithmetic is integer nanoseconds, including huge settings.
    deadline = time.monotonic_ns() + timeout_nanoseconds(seconds)
    while True:
        remaining = deadline - time.monotonic_ns()
        if remaining <= 0:
            raise subprocess.TimeoutExpired(proc.args, seconds)
        wait = min(remaining, MAX_WAIT_SECONDS * 1_000_000_000) / 1_000_000_000
        try:
            proc.wait(timeout=wait)
        except subprocess.TimeoutExpired:
            if time.monotonic_ns() >= deadline:
                raise
            continue
        remaining = deadline - time.monotonic_ns()
        if written.is_set():
            return
        if remaining > 0 and written.wait(
            min(remaining, MAX_WAIT_SECONDS * 1_000_000_000) / 1_000_000_000
        ):
            return


def run(
    root: Path, job: str, *, _claude_for_test: list[str] | None = None,
    _timeout_seconds_for_test: float | None = None,
) -> int:
    """All runs acquire the OS lock before the private Claude spawn site."""
    try:
        root = validated_root(root)
        minutes, _, _ = load_settings(root)
        lock_path = job_lock_path(root, job)
        seconds = minutes * 60
        if _timeout_seconds_for_test is not None:
            value = _timeout_seconds_for_test
            if (
                type(value) not in (int, float)
                or (type(value) is float and not math.isfinite(value))
                or not 0 < value <= seconds
            ):
                raise ValueError("invalid test timeout (must only shorten configured timeout)")
            seconds = value
        if _claude_for_test is not None and (
            type(_claude_for_test) is not list or not _claude_for_test or any(
                not isinstance(arg, str) or not arg or "\0" in arg
                for arg in _claude_for_test
            )
        ):
            raise ValueError("invalid test Claude argv prefix")
        lock_path.parent.mkdir(parents=True, exist_ok=True)
        with file_lock(lock_path) as lock:
            if lock is False:
                say(f"auto_agent: {job} already running")
                return 0
            return _run_locked(root, job, minutes, seconds, _claude_for_test)
    except (OSError, ValueError, OverflowError) as error:
        say(f"auto_agent: {one_line(str(error))}", error=True)
        return 2


def _run_locked(root: Path, job: str, minutes: int, seconds: int | float, prefix) -> int:
    before = {}
    captured = False
    started = time.monotonic_ns()
    code = 2
    detail = ""
    output = "none"
    proc = None
    writer = None
    written = threading.Event()
    write_errors = []
    try:
        before = snapshot(root)
        captured = True
        progress(
            root, f"AUTO-AGENT start {job}: claude -p opus, brief {brief_relative(job)}, "
            f"timeout {minutes} min, pid {os.getpid()}",
        )
        try:
            prompt = prompt_for(root, job)
        except FileNotFoundError as error:
            raise ValueError("brief missing") from error
        executable = prefix[0] if prefix is not None else find_claude()
        if executable is None:
            raise ValueError("claude not found")
        argv = claude_argv(executable, job)
        if prefix is not None:
            argv = [*prefix, *argv[1:]]
        folder = local_path(root, "scratch/auto-agent")
        folder.mkdir(parents=True, exist_ok=True)
        name = f"{job}-{utc_now().strftime('%Y%m%d-%H%M%S')}.json"
        output = f"scratch/auto-agent/{name}"
        out_path = local_path(root, output)
        err_path = local_path(root, str(out_path.with_suffix(".err").relative_to(root)))
        options = (
            {"creationflags": subprocess.CREATE_NEW_PROCESS_GROUP}
            if os.name == "nt" else {"start_new_session": True}
        )
        with out_path.open("xb") as stdout, err_path.open("xb") as stderr:
            proc = subprocess.Popen(
                argv, cwd=root, stdin=subprocess.PIPE, stdout=stdout, stderr=stderr,
                close_fds=True, **options,
            )
            writer = threading.Thread(
                target=feed_stdin,
                args=(proc.stdin, prompt.encode("utf-8"), written, write_errors),
                daemon=True,
            )
            writer.start()
            try:
                communicate(proc, written, seconds)
                code = proc.returncode
            except subprocess.TimeoutExpired:
                kill_tree(proc, minutes * 60)
                code = 124
                detail = ", TIMEOUT killed"
    except (OSError, ValueError, OverflowError, RuntimeError, subprocess.SubprocessError) as error:
        detail += f", refusal: {error}"
        say(f"auto_agent: {one_line(str(error))}", error=True)
        code = 2
    finally:
        # Even a write/communication failure must not leave a paid child alive.
        if proc is not None:
            if proc.poll() is None:
                try:
                    kill_tree(proc, minutes * 60)
                except (OSError, subprocess.SubprocessError) as error:
                    detail += f", cleanup failed: {error}"
                    say(f"auto_agent: {one_line(str(error))}", error=True)
                    code = 2
            if writer is not None and writer.ident is not None:
                writer.join(timeout=min(minutes * 60, MAX_WAIT_SECONDS))
                if writer.is_alive():
                    detail += ", stdin writer did not stop after tree kill"
                    code = 2
                if write_errors and code != 124:
                    detail += f", stdin failed: {write_errors[0]}"
                    code = 2
            elif proc.stdin is not None:
                try:
                    proc.stdin.close()
                except OSError as error:
                    detail += f", stdin close failed: {error}"
                    say(f"auto_agent: {one_line(str(error))}", error=True)
                    code = 2
        try:
            changed = (
                ", ".join(changed_briefs(root, before)) if captured else "snapshot unavailable"
            )
        except (OSError, ValueError) as error:
            changed = f"snapshot failed: {error}"
            code = 2
        elapsed = (time.monotonic_ns() - started) / 60_000_000_000
        try:
            progress(
                root, f"AUTO-AGENT end {job}: exit {code} after {elapsed:.3f} min"
                f"{detail}; output {output}; changed existing briefs: {changed or 'none'}",
            )
        except (OSError, ValueError) as error:
            say(f"auto_agent: end log failed: {one_line(str(error))}", error=True)
            code = 2
    return code


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True, type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true")
    mode.add_argument("--run", choices=JOBS)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    if args.dry_run and not args.check:
        parser.error("--dry-run requires --check")
    return check(args.root, args.dry_run) if args.check else run(args.root, args.run)


if __name__ == "__main__":
    sys.exit(main())
