"""Re-apply one adjudicated revert: python -I scripts/reapply.py <root>.

Run from the orchestrator's serialized tick (or by hand with the tick idle).
The TSV is locked against another reapply run, not against independent queue
writers. Keep the lane idle outside this script until qadd has taken over.
A refused row or a safely undone qadd failure does not block the next row.
"""

from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime
from enum import Enum, auto
import errno
import io
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import threading
import time
import tomllib


TASK_ID = re.compile(r"t-[A-Za-z0-9_-]+")
ENTRY_ID = re.compile(r"(q-[0-9]{8}-[0-9]{6})(?:-([1-9][0-9]*))?")
PROJECT = re.compile(r"[A-Za-z0-9_.-]+")
OID = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})")
DEVICE = re.compile(r"(?i)(?:CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|(?:COM|LPT)[1-9¹²³])")
OPEN = {"queued", "running", "approved", "interrupted"}
FINISHED_OR_INACTIVE = {
    "blocked", "cancelled", "check_failed", "conflict", "merged",
    "merged_check_failed", "perf_failed", "rejected", "stale",
}
PICK_STATE = ("CHERRY_PICK_HEAD", "sequencer")
OTHER_STATE = ("MERGE_HEAD", "REVERT_HEAD", "rebase-apply", "rebase-merge", "BISECT_START")
INPUT_ERRORS = (OSError, ValueError)


def say(message, *, file=None):
    # A closed log consumer must not interrupt Git cleanup.
    stream = sys.stdout if file is None else file
    try:
        print(message.encode(stream.encoding or "utf-8", "backslashreplace").decode(
            stream.encoding or "utf-8"), file=stream, flush=True)
    except OSError:
        pass


def line_text(text):
    return "".join(c if c.isprintable() else ascii(c)[1:-1] for c in str(text))


def relative(value, name, *, dot=False):
    """Use Windows path rules even when examining a row on another OS."""
    if dot and value == ".":
        return Path()
    if not isinstance(value, str) or not value.isprintable():
        raise ValueError(f"{name}: expected a printable relative path")
    parts = value.replace("\\", "/").split("/")
    if any(not p or p in (".", "..") or p.endswith((".", " "))
           or re.search(r'[<>:"|?*]', p) or DEVICE.fullmatch(p.split(".")[0].rstrip())
           for p in parts):
        raise ValueError(f"{name}: expected a plain path inside root")
    return Path(*parts)


def inside(root, path, *, kind=None):
    """Check each existing component without following links or Windows junctions."""
    here = root
    info = root.lstat()
    for part in ("", *path.relative_to(root).parts):
        if part:
            if part == ".." or not stat.S_ISDIR(info.st_mode):
                raise ValueError(f"not a directory inside root: {here}")
            here = here / part
            info = here.lstat()
        if stat.S_ISLNK(info.st_mode) or (
            getattr(info, "st_file_attributes", 0) & getattr(stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0)
        ):
            raise ValueError(f"link or reparse point: {here}")
    if kind and not kind(info.st_mode):
        raise ValueError(f"unexpected file type: {path}")
    return info


@contextmanager
def open_input(root, path):
    before = inside(root, path, kind=stat.S_ISREG)
    # O_NONBLOCK keeps a substituted FIFO from blocking open on POSIX.
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NONBLOCK", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    with os.fdopen(os.open(path, flags), "rb") as stream:
        opened = os.fstat(stream.fileno())
        after = inside(root, path, kind=stat.S_ISREG)
        if (not stat.S_ISREG(opened.st_mode) or not os.path.samestat(before, opened)
                or not os.path.samestat(opened, after)):
            raise ValueError(f"input changed while opening: {path}")
        yield stream


def try_lock_tsv(stream):
    """Keep a nonblocking OS lock on the existing input until its handle closes."""
    try:
        if os.name == "nt":
            import msvcrt

            msvcrt.locking(stream.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl

            fcntl.flock(stream.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError as exc:
        if exc.errno in (errno.EACCES, errno.EAGAIN):
            return False
        raise
    return True


@dataclass(frozen=True)
class Settings:
    root: Path
    state: Path
    projects: Path
    worktrees: Path
    scratch: Path
    seconds: int
    max_bytes: int

    @classmethod
    def load(cls, root):
        config = root / "broadsword.toml"
        with open_input(root, config) as stream:
            data = tomllib.load(stream)
        if relative(data.get("root"), "root", dot=True) != Path():
            raise ValueError("root must name the directory holding broadsword.toml and qadd.ps1")
        paths = [root / relative(data.get(key), key) for key in (
            "state_dir", "projects_dir", "worktrees_dir", "scratch_dir",
        )]
        # Use the existing local tool-call budgets; Windows waits take DWORD milliseconds.
        seconds = data.get("mcp_call_timeout_seconds")
        kb = data.get("mcp_output_max_kb")
        if type(seconds) is not int or not 0 < seconds * 1000 < (1 << 32) - 1:
            raise ValueError("mcp_call_timeout_seconds: expected a positive supported wait")
        if type(kb) is not int or not 0 < kb <= (sys.maxsize - 1) // 1024:
            raise ValueError("mcp_output_max_kb: expected a positive supported size")
        return cls(root, *paths, seconds, kb * 1024)


def bounded_lines(stream, limit):
    """Yield physical lines; an oversized line is one bad row, not several rows."""
    first = True
    while raw := stream.readline(limit + 1):
        oversized = len(raw) > limit
        if first:
            raw = raw.removeprefix(b"\xef\xbb\xbf")
            first = False
        significant = raw.lstrip()
        comment = significant.startswith(b"#")
        blank = not significant
        if oversized:
            while raw and not raw.endswith(b"\n"):
                raw = stream.readline(limit + 1)
                if blank and (significant := raw.lstrip()):
                    comment = significant.startswith(b"#")
                    blank = False
            yield b"" if blank or comment else None
        else:
            yield raw.removesuffix(b"\r\n").removesuffix(b"\n")


@dataclass(frozen=True)
class Row:
    task: str
    entry: str
    verdict: Path
    risk: str
    coder: str
    brief: Path

    @classmethod
    def parse(cls, cfg, raw):
        if raw is None:
            raise ValueError("line exceeds mcp_output_max_kb")
        fields = raw.decode("utf-8").split("\t")
        if len(fields) != 6:
            raise ValueError("expected six tab-separated fields")
        task, entry, verdict, risk, coder, brief = fields
        if not TASK_ID.fullmatch(task):
            raise ValueError("invalid task id")
        entry_order(entry)
        if risk not in ("low", "high"):
            raise ValueError("risk must be low or high")
        if not coder.isprintable() or coder != coder.strip() or coder.startswith("-") or '"' in coder:
            raise ValueError("invalid coder name")
        return cls(
            task, entry, cfg.root / relative(verdict, "adjudication file"), risk, coder,
            cfg.root / relative(brief, "brief") if brief else cfg.scratch / "briefs" / f"{task}.md",
        )


def entry_order(entry_id):
    match = ENTRY_ID.fullmatch(entry_id)
    if not match:
        raise ValueError("invalid queue entry id")
    stamp, suffix = match.groups()
    try:
        datetime.strptime(stamp, "q-%Y%m%d-%H%M%S")
    except ValueError as exc:
        raise ValueError("invalid queue entry timestamp") from exc
    # Queue ids are UTC timestamps with an incrementing collision suffix.
    suffix = suffix or ""
    if suffix == "1":
        raise ValueError("invalid queue entry suffix")
    return stamp, len(suffix), suffix


def read_entry(cfg, path):
    with open_input(cfg.root, path) as stream:
        data = stream.read(cfg.max_bytes + 1)
    if len(data) > cfg.max_bytes:
        raise ValueError(f"oversized queue entry {path.name}")
    fields = {}
    for raw in io.BytesIO(data):
        text = raw.removesuffix(b"\r\n").removesuffix(b"\n").decode("utf-8")
        key, sep, value = text.partition(" = ")
        if not sep and text.endswith(" ="):
            key, sep = text[:-2], " ="
        if not sep or not re.fullmatch(r"[a-z_][a-z_0-9]*", key) or key in fields:
            raise ValueError(f"malformed or duplicate queue field in {path.name}")
        fields[key] = value
    if (fields.get("id", path.stem) != path.stem
            or not TASK_ID.fullmatch(fields.get("task", ""))
            or not PROJECT.fullmatch(fields.get("project", ""))
            or fields.get("status") not in OPEN | FINISHED_OR_INACTIVE):
        raise ValueError(f"invalid queue identity or status in {path.name}")
    relative(fields["project"], "project")
    return fields


def lane_held(cfg, queue, project):
    """Presence is enough; a broken link, directory or unreadable path also holds."""
    key = project.lower()
    try:
        inside(cfg.root, queue, kind=stat.S_ISDIR)
        for name in {key, key.replace(".", "_")}:
            try:
                (queue / f"hold-{name}").lstat()
            except FileNotFoundError:
                continue
            return True
    except INPUT_ERRORS:
        return True
    return False


def open_or_newer(cfg, queue, row, project):
    """Read the durable statuses (an interrupted runner is also stored as running)."""
    inside(cfg.root, queue, kind=stat.S_ISDIR)
    busy = False
    target = entry_order(row.entry)
    lane = project.lower()
    with os.scandir(queue) as entries:
        for item in entries:
            path = queue / item.name
            if path.suffix.lower() != ".entry":
                continue
            order = entry_order(path.stem)
            entry = read_entry(cfg, path)
            if entry["task"] == row.task and order > target:
                return busy, True
            busy |= entry["project"].lower() == lane and entry["status"] in OPEN
    return busy, False


def eligible(cfg, row, *, expected=None):
    queue = cfg.state / "queue"
    try:
        inside(cfg.root, queue, kind=stat.S_ISDIR)
        entry = read_entry(cfg, queue / f"{row.entry}.entry")
        if (entry["task"] != row.task or entry["status"] != "merged_check_failed"
                or not OID.fullmatch(entry.get("revert_sha", ""))
                or (expected is not None and entry != expected)):
            say(f"REAPPLY SKIP: {row.task} not the recorded reverted round")
            return None
        project = entry["project"]
        busy, newer = open_or_newer(cfg, queue, row, project)
        if newer:
            return None
    except INPUT_ERRORS as exc:
        say(f"REAPPLY SKIP: {row.task} cannot establish queue state: {line_text(exc)}")
        return None
    try:
        with open_input(cfg.root, row.verdict) as stream:
            has_verdict = bool(stream.read(1))
    except INPUT_ERRORS:
        has_verdict = False
    if not has_verdict:
        say(f"REAPPLY SKIP: {row.task} no adjudication file {line_text(row.verdict)}")
        return None
    if busy or lane_held(cfg, queue, project):
        say(f"REAPPLY SKIP: {row.task} lane held or busy")
        return None
    return entry


class CommandError(ValueError):
    def __init__(self, message, code=None, *, incomplete=False):
        super().__init__(message)
        self.code = code
        self.incomplete = incomplete


def capture(cfg, argv, cwd, *, git=False):
    """Keep bounded bytes and wait neither indefinitely for a child nor for pipe EOF."""
    env = {key: val for key, val in os.environ.items() if not key.upper().startswith("GIT_")}
    env.update(GIT_OPTIONAL_LOCKS="0", GIT_NO_LAZY_FETCH="1", GIT_TERMINAL_PROMPT="0")
    proc = subprocess.Popen(
        argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL, bufsize=0,
    )
    output = bytearray()
    errors = []

    def drain():
        try:
            with proc.stdout as pipe:
                while block := pipe.read(io.DEFAULT_BUFFER_SIZE):
                    remaining = cfg.max_bytes - len(output)
                    output.extend(block[:remaining])
                    if len(block) > remaining and not errors:
                        errors.append(CommandError("output exceeds mcp_output_max_kb"))
        except INPUT_ERRORS as exc:
            errors.append(exc)

    deadline = time.monotonic() + cfg.seconds
    reader = threading.Thread(target=drain, daemon=True)
    try:
        reader.start()
        reader.join(cfg.seconds)
        if reader.is_alive():
            first = f"{errors[0]}; " if errors else ""
            raise CommandError(first + "output did not close before mcp_call_timeout_seconds",
                               proc.poll(), incomplete=True)
        code = proc.wait(timeout=max(0, deadline - time.monotonic()))
        if errors:
            raise CommandError(str(errors[0]), code)
    except subprocess.TimeoutExpired as exc:
        raise CommandError("process exceeded mcp_call_timeout_seconds", incomplete=True) from exc
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=cfg.seconds)
    return code, output.decode("utf-8", errors="strict" if git else "replace")


def git(cfg, repo, why, *args, ok=(0,), overrides=(), git_dir=None):
    inside(cfg.root, repo, kind=stat.S_ISDIR)
    if git_dir is None:
        inside(cfg.root, repo / ".git", kind=stat.S_ISDIR)
    else:
        inside(cfg.root, git_dir, kind=stat.S_ISDIR)
    location = [] if git_dir is None else ["--git-dir", str(git_dir), "--work-tree", str(repo)]
    argv = ["git", "-C", str(repo), *location, "--no-replace-objects", "-c", f"core.hooksPath={os.devnull}",
            "-c", "core.fsmonitor=false", "-c", "submodule.recurse=false",
            "-c", "rerere.enabled=false", "-c", "commit.gpgSign=false",
            "-c", "maintenance.auto=false", "-c", "gc.auto=0", "-c", "core.logAllRefUpdates=true",
            "-c", "core.quotePath=true", "-c", "merge.autoStash=false", *overrides, *args]
    say(f"GIT: {line_text(subprocess.list2cmdline(argv))}: {why}")
    try:
        code, out = capture(cfg, argv, repo, git=True)
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        raise CommandError(f"git {args[0]}: {exc}") from exc
    if code not in ok:
        raise CommandError(f"git {args[0]} exited {code}", code)
    return out


def oid(value, name):
    if not OID.fullmatch(value):
        raise ValueError(f"{name}: expected a full commit hash")
    return value


def has_state(git_dir, names):
    for name in names:
        try:
            (git_dir / name).lstat()
        except FileNotFoundError:
            continue
        return True
    return False


def verify_top(cfg, repo):
    output = git(cfg, repo, "verify this is the intended checkout", "rev-parse", "--show-toplevel")
    top = Path(output.rstrip("\r\n"))
    if not top.is_absolute() or Path(os.path.abspath(top)) != repo:
        raise ValueError("Git top level does not match the checkout path")


def git_pointer(cfg, path, prefix=""):
    with open_input(cfg.root, path) as stream:
        raw = stream.read(cfg.max_bytes + 1)
    if len(raw) > cfg.max_bytes:
        raise ValueError(f"oversized Git pointer: {path}")
    value = raw.removesuffix(b"\r\n").removesuffix(b"\n").decode("utf-8")
    if not value.startswith(prefix) or not value[len(prefix):].isprintable():
        raise ValueError(f"invalid Git pointer: {path}")
    target = Path(value[len(prefix):])
    if target.anchor and not target.is_absolute():
        raise ValueError(f"partially rooted Git pointer: {path}")
    return Path(os.path.abspath(path.parent / target))


def linked_git_dir(cfg, repo, worktree):
    main_git = repo / ".git"
    inside(cfg.root, main_git, kind=stat.S_ISDIR)
    git_file = worktree / ".git"
    meta = git_pointer(cfg, git_file, "gitdir: ")
    if (meta.parent != main_git / "worktrees"
            or git_pointer(cfg, meta / "commondir") != main_git
            or git_pointer(cfg, meta / "gitdir") != git_file):
        raise ValueError("task worktree is not linked to the project's main checkout")
    return meta


def safe_filters(work):
    keys = work("find configured external Git filters and merge drivers", "config",
                "--null", "--name-only", "--get-regexp", r"^(filter\.|merge\..*\.driver$)",
                ok=(0, 1))
    drivers = set()
    for key in keys.removesuffix("\0").split("\0") if keys else ():
        if not re.fullmatch(r"filter\.[A-Za-z0-9_.-]+\.[a-z]+", key):
            raise ValueError("configured merge driver or unsupported filter; not safe to replay unattended")
        drivers.add(key.rsplit(".", 1)[0])
    overrides = []
    for driver in sorted(drivers):
        for setting in ("clean=", "smudge=", "process=", "required=false"):
            overrides.extend(("-c", f"{driver}.{setting}"))
    return overrides


def require_idle(work, git_dir, branch):
    actual = work("verify the exact task ref", "symbolic-ref", "HEAD").strip()
    if actual != f"refs/heads/{branch}" or has_state(git_dir, PICK_STATE + OTHER_STATE):
        raise ValueError("wrong task branch or an existing Git operation")
    if work("protect local files, including ignored ones", "status", "--porcelain",
            "--untracked-files=all", "--ignored=matching", "--ignore-submodules=none"):
        raise ValueError("task worktree is not clean (including ignored files)")


def require_own_pick(cfg, git_dir, main, commits):
    if has_state(git_dir, OTHER_STATE):
        raise ValueError("another Git operation is present; not aborting")
    for name, expected in (("sequencer/head", (main,)), ("CHERRY_PICK_HEAD", commits)):
        try:
            with open_input(cfg.root, git_dir / name) as stream:
                raw = stream.read(cfg.max_bytes + 1)
        except FileNotFoundError:
            if name == "sequencer/head" and has_state(git_dir, ("sequencer",)):
                raise ValueError("cannot establish the sequencer's starting HEAD") from None
            continue
        if len(raw) > cfg.max_bytes or raw.decode("ascii").rstrip("\r\n") not in expected:
            raise ValueError("pending cherry-pick does not match this run; not aborting")


class Outcome(Enum):
    SKIPPED = auto()
    FAILED = auto()  # Reported failure; safe to try a later row.
    STOP = auto()    # Git operation failed or queue handoff may have happened.
    QUEUED = auto()


def finish_unqueued(cfg, row, entry, work, git_dir, branch, main, replay_head, outcome):
    """Restore only our replay, and only when it was not handed to the queue."""
    try:
        # qadd adds the entry *before* starting the driver, which can fail separately.
        _, newer = open_or_newer(cfg, cfg.state / "queue", row, entry["project"])
        if newer:
            if outcome is Outcome.SKIPPED:
                return outcome
            say(f"reapply: {row.task}: newer queue entry exists; retaining replay", file=sys.stderr)
            return Outcome.STOP
        if outcome is Outcome.STOP:
            # A timed-out child can leave descendants alive (including queue add).
            say(f"reapply: {row.task}: qadd outcome uncertain; retaining replay for inspection",
                file=sys.stderr)
            return outcome
        require_idle(work, git_dir, branch)
        if work("confirm HEAD still names only this run's replay", "rev-parse", "HEAD").strip() != replay_head:
            raise ValueError("task HEAD changed after replay; refusing to rewind")
        work("return the task to the replay base, keeping local work and reflog history",
             "reset", "--keep", main)
        require_idle(work, git_dir, branch)
        if work("verify the task is ready for a fresh attempt", "rev-parse", "HEAD").strip() != main:
            raise ValueError("task HEAD differs from the replay base")
    except (OSError, ValueError, KeyboardInterrupt) as exc:
        say(f"reapply: {row.task}: cleanup failed: {line_text(exc)}", file=sys.stderr)
        if outcome is Outcome.SKIPPED:
            say(f"REAPPLY CONFLICT: {row.task} ")
        return Outcome.STOP
    return outcome


def replay(cfg, row, entry):
    project = entry["project"]
    repo = cfg.root if project.lower() == "broadsword" else cfg.projects / project
    worktree = cfg.worktrees / project / row.task
    overrides = ()
    merging = False
    picking = False
    try:
        branch = f"task/{row.task}"
        if entry.get("branch", branch) != branch:
            raise ValueError("queue entry branch does not match the task")
        trunk = entry.get("main_branch") or "main"
        if trunk not in ("main", "master"):
            raise ValueError("invalid recorded main branch")
        base = oid(entry.get("train_up") or entry.get("pre_merge_sha", ""), "commit range base")
        tip = oid(entry.get("main_sha", ""), "main_sha")
        revert = oid(entry["revert_sha"], "revert_sha")
        git_dir = linked_git_dir(cfg, repo, worktree)
        verify_top(cfg, repo)

        def work(why, *args, ok=(0,)):
            if linked_git_dir(cfg, repo, worktree) != git_dir:
                raise ValueError("task worktree metadata changed")
            return git(cfg, worktree, why, *args, ok=ok, overrides=overrides, git_dir=git_dir)

        overrides = safe_filters(work)
        main = oid(git(cfg, repo, "pin the current main tip, not the old failed tip",
                       "rev-parse", "--verify", f"refs/heads/{trunk}^{{commit}}").strip(), "main tip")
        for before, after in ((base, tip), (tip, revert), (revert, main)):
            git(cfg, repo, "check the recorded range and revert are in main's history",
                "merge-base", "--is-ancestor", before, after)
        commits = git(cfg, repo, "list only this task's commits in replay order",
                      "rev-list", "--reverse", f"{base}..{tip}").splitlines()
        if not commits or tip == revert:
            raise ValueError("empty commit range or no distinct revert")
        for commit in commits:
            oid(commit, "rev-list commit")
        if not eligible(cfg, row, expected=entry):
            return Outcome.SKIPPED
        require_idle(work, git_dir, branch)
        try:
            work("refuse to overwrite a divergent or already replayed task tip",
                 "merge-base", "--is-ancestor", "HEAD", main)
        except CommandError as exc:
            if exc.code != 1:
                raise
            say(f"REAPPLY SKIP: {row.task} task tip is not an ancestor of main; inspect its commits")
            return Outcome.SKIPPED
        merging = True
        work("fast-forward the task to main without replacing local files",
             "merge", "--ff-only", "--no-overwrite-ignore", main)
        if not eligible(cfg, row, expected=entry):
            return Outcome.SKIPPED
        require_idle(work, git_dir, branch)
        if work("verify the cherry-pick will start exactly at the fast-forwarded tip",
                "rev-parse", "HEAD").strip() != main:
            raise ValueError("task HEAD is not at the pinned main tip")
        picking = True
        work("replay the reverted patches as new commits, without skipping any",
             "cherry-pick", *commits)
        replay_head = oid(work("record the tip created by this replay", "rev-parse", "HEAD").strip(),
                          "replay tip")
        require_idle(work, git_dir, branch)
        count = work("verify the replay has one new commit per original commit",
                     "rev-list", "--count", f"{main}..{replay_head}").strip()
        if count != str(len(commits)):
            raise ValueError("replay did not produce the expected number of commits")
    except (OSError, ValueError, KeyboardInterrupt) as exc:
        say(f"reapply: {row.task}: {line_text(exc)}", file=sys.stderr)
        files = ""
        if picking:
            try:
                files = ",".join(work("collect unmerged paths before aborting", "diff",
                                      "--no-ext-diff", "--name-only", "--diff-filter=U").splitlines())
            except INPUT_ERRORS as diff_error:
                say(f"reapply: cannot list conflicts: {line_text(diff_error)}", file=sys.stderr)
            try:
                if linked_git_dir(cfg, repo, worktree) != git_dir:
                    raise ValueError("task worktree metadata changed")
                if has_state(git_dir, PICK_STATE):
                    require_own_pick(cfg, git_dir, main, commits)
                    work("abort only this run's cherry-pick; retain the fast-forward",
                         "cherry-pick", "--abort")
                require_idle(work, git_dir, branch)
                if work("verify the original replay base survived the abort", "rev-parse", "HEAD").strip() != main:
                    raise ValueError("task HEAD differs from the replay base; inspect it manually")
            except INPUT_ERRORS as abort_error:
                say(f"reapply: cleanup failed: {line_text(abort_error)}", file=sys.stderr)
        say(f"REAPPLY CONFLICT: {row.task} {line_text(files)}")
        return Outcome.STOP if merging or isinstance(exc, KeyboardInterrupt) else Outcome.FAILED
    outcome = enqueue(cfg, row, entry, main, len(commits))
    if outcome is Outcome.QUEUED:
        return outcome
    return finish_unqueued(cfg, row, entry, work, git_dir, branch, main, replay_head, outcome)


def enqueue(cfg, row, entry, main, count):
    """The only qadd spawn site; recheck lane, newer-entry and verdict guards."""
    script = cfg.root / "scripts" / "qadd.ps1"
    starting = False
    try:
        if not eligible(cfg, row, expected=entry):
            return Outcome.SKIPPED
        inside(cfg.root, script, kind=stat.S_ISREG)
        starting = True
        code, output = capture(cfg, [
            "powershell", "-NoProfile", "-File", str(script), row.task, row.risk,
            row.coder, str(row.brief), "-Adjudicated", str(row.verdict),
        ], cfg.root)
    except (OSError, ValueError, subprocess.TimeoutExpired, KeyboardInterrupt) as exc:
        code = exc.code if isinstance(exc, CommandError) else None
        say(f"REAPPLY QADD FAILED: {row.task} exit {code if code is not None else 'unavailable'} "
            f"({line_text(exc) or type(exc).__name__})")
        incomplete = isinstance(exc, (KeyboardInterrupt, subprocess.TimeoutExpired)) or (
            isinstance(exc, CommandError) and exc.incomplete
        )
        return Outcome.STOP if starting and incomplete else Outcome.FAILED
    if code or not output.strip():
        detail = "" if code else " (no output)"
        say(f"REAPPLY QADD FAILED: {row.task} exit {code}{detail}")
        return Outcome.FAILED
    say(f"REAPPLY QUEUED: {row.task} onto {main[:10]} ({count} commits; verdict {line_text(row.verdict)})")
    say(line_text(output.strip().splitlines()[-1]))
    return Outcome.QUEUED


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if len(args) != 1 or not args[0]:
        say("usage: python -I scripts/reapply.py <root>", file=sys.stderr)
        return 1
    status = 0
    try:
        supplied = Path(args[0])
        if not args[0].isprintable() or (supplied.anchor and not supplied.is_absolute()):
            raise ValueError("root must be a printable, fully rooted or relative directory path")
        try:
            root = supplied.resolve(strict=True)
        except RuntimeError as exc:
            raise ValueError(f"cannot resolve root: {exc}") from exc
        cfg = Settings.load(root)
        path = cfg.state / "reapply.tsv"
        with open_input(root, path) as stream:
            if not try_lock_tsv(stream):
                say("REAPPLY SKIP: another reapply run is using reapply.tsv")
                return 0
            for number, raw in enumerate(bounded_lines(stream, cfg.max_bytes), 1):
                if raw is not None and (not raw.strip() or raw.lstrip().startswith(b"#")):
                    continue
                try:
                    row = Row.parse(cfg, raw)
                except ValueError as exc:
                    say(f"REAPPLY BAD ROW: {number} {line_text(exc)}")
                    continue
                entry = eligible(cfg, row)
                if entry is not None:
                    outcome = replay(cfg, row, entry)
                    if outcome is Outcome.STOP:
                        return 2
                    if outcome is Outcome.QUEUED:
                        return status
                    if outcome is Outcome.FAILED:
                        status = 2
    except INPUT_ERRORS as exc:
        say(f"reapply: cannot read root or reapply.tsv: {line_text(exc)}", file=sys.stderr)
        return 1
    return status


if __name__ == "__main__":
    sys.exit(main())
