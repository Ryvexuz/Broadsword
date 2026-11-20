"""List, and with --yes move, cold data from the internal drive to the external drive.

Usage (every flag is required unless marked optional; nothing has a default):
    python -I scripts/cold_move.py --root <dir> --target <external root> --trigger-gb <n>
        --state-dir <rel> --cold-days <n>
        [--cache-dir <rel> ...] [--evidence-dir <rel> ...] [--reference-dir <rel> ...]
        [--worktrees-dir <rel>] (--board-file <tsv> | --broadsword <exe>)
        [--force-list] [--yes --class <name> ... --approval-file <rel>]

These become required settings in broadsword.toml in a later task (which claims that file);
until then they are flags.

Classes (a class is only considered when its flag is given):
    build-cache   each immediate child of a --cache-dir whose newest file is older than --cold-days
    evidence      each immediate child of an --evidence-dir
    reference     each immediate child of a --reference-dir
    worktree      each <worktrees-dir>/<project>/<task id> whose task the board lists as
                  done, abandoned or failed, which is clean and registered with git

The default run is a dry run: it prints each item with class, size and target path and moves
nothing. It refuses to run unless free space of --root is under --trigger-gb (or --force-list,
for a dry run only). --yes moves only the classes named by --class, and refuses unless the
approval file (inside --state-dir) exists and is exactly one line

    APPROVED <today's UTC date, YYYY-MM-DD> <class>[,<class>...]

naming every class the run asks for. The orchestrator writes it only after Lazlo's yes in chat.
An approval is used once: when a --yes run reaches the move step it renames the file to
<name>.used-<UTC timestamp>-<id> (never deletes it) before moving anything, so the next --yes run
of any class needs a new yes and a new file. A rename failure refuses the run, nothing moved.

Never moved, whatever the flags: a path with a git-tracked file; anything inside --state-dir
(state/ records); a worktree the board does not list as finished (running, queued, unknown, or
the board cannot be read); a symlink or a folder holding one; a folder holding a secret file
(.env*); anything whose target already exists. For the copy classes (build-cache, evidence,
reference) every candidate also goes through repo_guard: one holding a .git entry (file or
folder) at any depth, one that is, holds or sits inside a worktree from `git worktree list`
(other than root itself), and one that overlaps --worktrees-dir or --state-dir is skipped; the
guard runs at planning and again right before the copy. Files and folders are copied, the size
and SHA-256 of every file are verified, and only then is the source removed. A retired worktree
is moved with `git worktree move`, then `git worktree repair`; right before the move the board
is read again and the target re-checked absent.
With --yes the run takes an exclusive lock file (cold-move.lock in --state-dir) and refuses while it
exists; a killed run leaves it behind and it is removed by hand. Each copy goes to a fresh temp
name next to the target, is verified, then renamed into place; an error removes only that temp
name, never a target this run did not create.
Exit codes: 0 ok (skipped items are listed, not errors), 1 some item failed to move, 2 usage error
or refusal.
"""
import argparse
import hashlib
import os
import shutil
import subprocess
import sys
import time
import uuid
from pathlib import Path

GIT_TIMEOUT = 120
BOARD_TIMEOUT = 60
CLASSES = ("build-cache", "evidence", "reference", "worktree")
FINISHED = ("done", "abandoned", "failed")
APPROVAL_PREFIX = "APPROVED"
LOCK_NAME = "cold-move.lock"
REMINDER = (
    "The first real run (--yes) needs Lazlo's yes in chat for every class. "
    "The orchestrator writes the approval file (APPROVED <UTC date> <classes>) only after that "
    "yes; each --yes run uses it up."
)


class Refusal(Exception):
    """A named reason the whole run must not go ahead (exit 2)."""


class MoveFailed(Exception):
    """One item could not be moved; its source is kept."""


# ---------------------------------------------------------------- helpers
def free_gb(path):
    return shutil.disk_usage(path).free / 1e9


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def run_git(root, *args):
    try:
        p = subprocess.run(
            ["git", "-C", str(root), *args],
            capture_output=True, text=True, timeout=GIT_TIMEOUT,
        )
    except (OSError, subprocess.TimeoutExpired) as e:
        raise Refusal(f"git could not run: {e}")
    return p


def rel_under(root, rel, what):
    """A relative path flag, validated, joined under root. No drive, no absolute, no '..'."""
    if not rel or "\n" in rel or "\r" in rel or "\0" in rel:
        raise Refusal(f"{what}: empty or has a line break")
    p = Path(rel)
    if p.is_absolute() or p.anchor or p.drive or ".." in p.parts:
        raise Refusal(f"{what}: must be a relative path inside root, got {rel!r}")
    full = (root / p).resolve()
    if full != root and root not in full.parents:
        raise Refusal(f"{what}: {rel!r} resolves outside root")
    return full


def is_inside(child, parent):
    return child == parent or parent in child.parents


def tree_has_symlink_or_secret(path):
    """Return a reason string if the folder holds a symlink or secret file, else None."""
    if path.is_symlink():
        return "is a symlink"
    if path.is_file():
        return "is a secret file" if path.name.lower().startswith(".env") else None
    for dirpath, dirnames, filenames in os.walk(path):
        for n in dirnames + filenames:
            q = Path(dirpath) / n
            if q.is_symlink():
                return f"holds a symlink ({q.name})"
            if n.lower().startswith(".env"):
                return f"holds a secret file ({n})"
    return None


def size_and_newest(path):
    total = 0
    newest = 0.0
    if path.is_file():
        st = path.stat()
        return st.st_size, st.st_mtime
    for dirpath, _d, files in os.walk(path):
        for n in files:
            st = (Path(dirpath) / n).stat()
            total += st.st_size
            newest = max(newest, st.st_mtime)
    if newest == 0.0:
        newest = path.stat().st_mtime
    return total, newest


def tracked_in(root, path):
    """True when git tracks any file at or under path. Fails closed: a git error refuses the run."""
    rel = path.relative_to(root).as_posix()
    p = run_git(root, "ls-files", "--", rel)
    if p.returncode != 0:
        raise Refusal(f"git ls-files failed for {rel}: {p.stderr.strip()}")
    return bool(p.stdout.strip())


# ---------------------------------------------------------------- board
def read_board(args):
    """Return {task id: status}. Raises Refusal when the board cannot be read (fail closed)."""
    if args.board_file:
        try:
            text = Path(args.board_file).read_text(encoding="utf-8")
        except OSError as e:
            raise Refusal(f"board file unreadable: {e}")
    else:
        try:
            p = subprocess.run(
                [args.broadsword, "task", "list"], cwd=args.root,
                capture_output=True, text=True, timeout=BOARD_TIMEOUT,
            )
        except (OSError, subprocess.TimeoutExpired) as e:
            raise Refusal(f"board unreadable: {e}")
        if p.returncode != 0:
            raise Refusal(f"board unreadable: broadsword task list exited {p.returncode}")
        text = p.stdout
    board = {}
    for line in text.splitlines():
        cols = line.split("\t")
        if len(cols) >= 2 and cols[0].strip():
            board[cols[0].strip()] = cols[1].strip()
    if not board:
        raise Refusal("board is empty or unreadable (no task rows)")
    return board


# ---------------------------------------------------------------- listing
class Item:
    def __init__(self, cls, src, dst, size, skip=None, guard=None):
        self.cls, self.src, self.dst, self.size, self.skip = cls, src, dst, size, skip
        self.guard = guard  # RepoGuard for copy classes: re-run right before the copy


def registered_worktrees(root):
    """Resolved paths of every worktree `git worktree list --porcelain` names. Fails closed."""
    p = run_git(root, "worktree", "list", "--porcelain")
    if p.returncode != 0:
        raise Refusal(f"git worktree list failed: {p.stderr.strip()}")
    out = set()
    for line in p.stdout.splitlines():
        if line.startswith("worktree "):
            out.add(Path(line[len("worktree "):]).resolve())
    return out


class RepoGuard:
    """What a copy-class candidate must never be, hold or sit inside."""

    def __init__(self, root, state, worktrees_dir, worktrees):
        self.root = root
        self.state = state
        self.worktrees_dir = worktrees_dir  # resolved path, or None when --worktrees-dir is not given
        self.worktrees = set(worktrees)  # resolved paths from git worktree list


def _raise(e):
    raise e


def repo_guard(guard, src):
    """Return a reason string when src must not be copied and removed, else None.

    The one place every build-cache, evidence and reference candidate passes through, at planning
    and again right before its copy. Refuses a candidate that:
      - overlaps the state folder or --worktrees-dir (is, holds or sits inside it);
      - is, holds or sits inside a worktree that `git worktree list` names (root itself is the
        main worktree and every candidate sits inside it, so for root only "is or holds" counts);
      - is named .git or holds a .git entry (file or folder) at any depth: a nested clone or a
        worktree whose files the root repo's `git ls-files` cannot see.
    An unreadable folder raises OSError (fail closed), never silently skips the walk.
    """
    real = src.resolve()
    if is_inside(real, guard.state) or is_inside(guard.state, real):
        return "overlaps the state folder"
    if guard.worktrees_dir is not None and (
        is_inside(real, guard.worktrees_dir) or is_inside(guard.worktrees_dir, real)
    ):
        return "overlaps the worktrees folder"
    for wt in guard.worktrees:
        if is_inside(wt, real):
            return f"is or holds a git worktree ({wt.name})"
        if wt != guard.root and is_inside(real, wt):
            return f"sits inside a git worktree ({wt.name})"
    if src.name.lower() == ".git":
        return "is a .git entry"
    if src.is_dir() and not src.is_symlink():
        for dirpath, dirnames, filenames in os.walk(src, onerror=_raise):
            if any(n.lower() == ".git" for n in dirnames + filenames):
                where = Path(dirpath).relative_to(src).as_posix()
                return f"holds a .git entry (nested repo or worktree at {where})"
    return None


def target_for(target, cls, root, src):
    return target / cls / src.relative_to(root)


def plan_children(args, root, target, state, cls, dirs, cold_only, guard):
    items = []
    for rel in dirs:
        base = rel_under(root, rel, f"--{cls}-dir")
        if is_inside(base, state) or is_inside(state, base):
            raise Refusal(f"--{cls}-dir {rel!r} overlaps the state folder: state/ records are never moved")
        if not base.is_dir():
            continue
        for child in sorted(base.iterdir()):
            items.append(classify(args, root, target, state, cls, child, cold_only, guard))
    return items


def classify(args, root, target, state, cls, src, cold_only, guard):
    dst = target_for(target, cls, root, src)
    skip = None
    size = 0
    try:
        skip = repo_guard(guard, src)
        if skip is None:
            skip = tree_has_symlink_or_secret(src)
        if skip is None and is_inside(src.resolve(), state):
            skip = "inside the state folder"
        if skip is None and tracked_in(root, src):
            skip = "git-tracked"
        if skip is None:
            size, newest = size_and_newest(src)
            if cold_only and newest > time.time() - args.cold_days * 86400:
                skip = "not cold yet"
        if skip is None and (dst.exists() or dst.is_symlink()):
            skip = "target already exists"
    except OSError as e:
        skip = f"unreadable ({e})"
    return Item(cls, src, dst, size, skip, guard)


def plan_worktrees(args, root, target, state, registered):
    base = rel_under(root, args.worktrees_dir, "--worktrees-dir")
    if not base.is_dir():
        return []
    board = read_board(args)
    items = []
    for proj in sorted(base.iterdir()):
        if not proj.is_dir() or proj.is_symlink():
            continue
        for wt in sorted(proj.iterdir()):
            dst = target_for(target, "worktree", root, wt)
            size = 0
            skip = None
            status = board.get(wt.name)
            if wt.is_symlink() or not wt.is_dir():
                skip = "not a plain folder"
            elif status is None:
                skip = "not on the board (treated as active)"
            elif status not in FINISHED:
                skip = f"active worktree (board status {status})"
            elif wt.resolve() not in registered:
                skip = "not registered with git"
            elif is_inside(wt.resolve(), state):
                skip = "inside the state folder"
            else:
                st = run_git(wt, "status", "--porcelain")
                if st.returncode != 0:
                    skip = f"git status failed: {st.stderr.strip()}"
                elif st.stdout.strip():
                    skip = "dirty (uncommitted changes)"
                elif (why := tree_has_symlink_or_secret(wt)) is not None:
                    skip = why
                elif dst.exists() or dst.is_symlink():
                    skip = "target already exists"
                else:
                    try:
                        size, _n = size_and_newest(wt)
                    except OSError as e:
                        skip = f"unreadable ({e})"
            items.append(Item("worktree", wt, dst, size, skip))
    return items


def build_plan(args, root, target, state):
    registered = registered_worktrees(root)
    wdir = rel_under(root, args.worktrees_dir, "--worktrees-dir") if args.worktrees_dir else None
    guard = RepoGuard(root, state, wdir, registered)
    items = []
    items += plan_children(args, root, target, state, "build-cache", args.cache_dir, True, guard)
    items += plan_children(args, root, target, state, "evidence", args.evidence_dir, False, guard)
    items += plan_children(args, root, target, state, "reference", args.reference_dir, False, guard)
    if args.worktrees_dir:
        items += plan_worktrees(args, root, target, state, registered)
    return items


# ---------------------------------------------------------------- moving
def manifest(path):
    """{relative path: (size, sha256) or None for a folder} of a file or tree."""
    if path.is_file():
        return {".": (path.stat().st_size, sha256_file(path))}
    out = {}
    for dirpath, dirnames, files in os.walk(path):
        for n in dirnames:
            out[(Path(dirpath) / n).relative_to(path).as_posix()] = None
        for n in files:
            q = Path(dirpath) / n
            out[q.relative_to(path).as_posix()] = (q.stat().st_size, sha256_file(q))
    return out


def copy_item(src, dst):
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.is_file():
        shutil.copy2(src, dst)
    else:
        shutil.copytree(src, dst, symlinks=False)


def verify_copy(src, dst):
    """Raise MoveFailed unless every entry, size and SHA-256 of dst equals src."""
    a, b = manifest(src), manifest(dst)
    if a != b:
        bad = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
        raise MoveFailed(f"copy does not match the source (size or SHA-256), first difference: {bad[0]}")


def exists_any(p):
    return p.exists() or p.is_symlink()


def move_copy(item):
    """Copy to a fresh temp name next to dst, verify, rename to dst, then remove the source.

    Only the temp path (a unique name this run checked was absent) is ever removed on error;
    dst is never deleted, because this run never created it.
    """
    if not exists_any(item.src):
        raise MoveFailed("source is gone; nothing copied")
    if exists_any(item.dst):
        raise MoveFailed("target appeared after planning; nothing copied, nothing removed")
    if item.guard is not None:
        try:
            why = repo_guard(item.guard, item.src)
        except OSError as e:
            raise MoveFailed(f"repo guard could not read the source ({e}); nothing copied")
        if why is not None:
            raise MoveFailed(f"{why} (found right before the copy); nothing copied")
    tmp = item.dst.parent / f"{item.dst.name}.cold-tmp-{os.getpid()}-{uuid.uuid4().hex}"
    try:
        item.dst.parent.mkdir(parents=True, exist_ok=True)
        if exists_any(tmp):  # a uuid name: cannot happen, but then it is not ours to remove
            raise MoveFailed(f"temp path {tmp.name} already exists; nothing copied")
        try:
            copy_item(item.src, tmp)
            verify_copy(item.src, tmp)
            if exists_any(item.dst):
                raise MoveFailed("target appeared during the copy; temp copy dropped, target untouched")
            os.rename(tmp, item.dst)  # fails if dst exists on Windows; the check above covers POSIX
        except MoveFailed:
            remove_partial(tmp)
            raise
        except OSError as e:
            remove_partial(tmp)
            raise MoveFailed(f"copy failed: {e}")
    except OSError as e:
        raise MoveFailed(f"copy failed: {e}")
    # Verified and in place. Only now is the source removed.
    try:
        if item.src.is_file():
            item.src.unlink()
        else:
            shutil.rmtree(item.src)
    except OSError as e:
        raise MoveFailed(f"copy verified but the source could not be removed: {e}")


def remove_partial(dst):
    """Remove only what this run created at the target."""
    try:
        if dst.is_symlink() or dst.is_file():
            dst.unlink()
        elif dst.is_dir():
            shutil.rmtree(dst)
    except OSError:
        pass


def move_worktree(args, root, item):
    """Re-check the board and the target right before `git worktree move`; planning is not enough."""
    if exists_any(item.dst):
        raise MoveFailed("target appeared after planning; source kept, nothing moved")
    try:
        status = read_board(args).get(item.src.name)
    except Refusal as e:
        raise MoveFailed(f"board unreadable at move time (source kept): {e}")
    if status not in FINISHED:
        raise MoveFailed(f"board status is now {status or 'missing'}, not finished; source kept")
    item.dst.parent.mkdir(parents=True, exist_ok=True)
    p = run_git(root, "worktree", "move", str(item.src), str(item.dst))
    if p.returncode != 0:
        raise MoveFailed(f"git worktree move failed (source kept): {p.stderr.strip()}")
    r = run_git(root, "worktree", "repair")
    if r.returncode != 0:
        raise MoveFailed(f"moved, but git worktree repair failed: {r.stderr.strip()}")


def do_move(args, root, item):
    if item.cls == "worktree":
        move_worktree(args, root, item)
    else:
        move_copy(item)


# ---------------------------------------------------------------- run
def utc_today():
    return time.strftime("%Y-%m-%d", time.gmtime())


def check_approval(root, state, rel, classes):
    """Return the approval file path if it approves today (UTC) every class in classes, else refuse.

    The file is exactly one line: APPROVED <YYYY-MM-DD> <class>[,<class>...].
    """
    f = rel_under(root, rel, "--approval-file")
    if not is_inside(f, state) or f == state:
        raise Refusal("--approval-file must be inside the state folder")
    if f.is_symlink() or not f.is_file():
        raise Refusal(f"refusing --yes: approval file {rel} does not exist. {REMINDER}")
    lines = [l for l in f.read_text(encoding="utf-8", errors="replace").splitlines() if l.strip()]
    form = f"{APPROVAL_PREFIX} <YYYY-MM-DD> <class>[,<class>...]"
    if len(lines) != 1:
        raise Refusal(f"refusing --yes: {rel} must be exactly one line, {form}. {REMINDER}")
    parts = lines[0].split()
    if len(parts) != 3 or parts[0] != APPROVAL_PREFIX:
        raise Refusal(f"refusing --yes: {rel} must read {form}. {REMINDER}")
    if parts[1] != utc_today():
        raise Refusal(
            f"refusing --yes: {rel} is dated {parts[1]!r}, not today ({utc_today()} UTC). {REMINDER}"
        )
    approved = parts[2].split(",")
    for c in approved:
        if c not in CLASSES:
            raise Refusal(f"refusing --yes: {rel} names unknown class {c!r}. {REMINDER}")
    missing = [c for c in classes if c not in approved]
    if missing:
        raise Refusal(
            f"refusing --yes: {rel} does not approve class {', '.join(missing)} "
            f"(it approves {', '.join(approved)}). {REMINDER}"
        )
    return f


def consume_approval(f):
    """Rename the approval file to a fresh .used-<UTC timestamp>-<id> name. Never deletes it.

    Raises Refusal when the rename fails, so no move happens on an approval that was not used up.
    """
    stamp = time.strftime("%Y%m%dT%H%M%SZ", time.gmtime())
    used = f.with_name(f"{f.name}.used-{stamp}-{uuid.uuid4().hex[:8]}")
    if exists_any(used):  # a fresh uuid name: cannot happen, but never replace a file
        raise Refusal(f"cannot use up the approval file: {used.name} already exists; nothing moved")
    try:
        os.rename(f, used)
    except OSError as e:
        raise Refusal(f"cannot use up the approval file ({e}); nothing moved")
    return used


def parse(argv):
    ap = argparse.ArgumentParser(prog="cold_move.py", description=__doc__.split("\n")[0])
    ap.add_argument("--root", required=True)
    ap.add_argument("--target", required=True, help="target root on the external drive")
    ap.add_argument("--trigger-gb", required=True, type=float)
    ap.add_argument("--state-dir", required=True)
    ap.add_argument("--cold-days", required=True, type=float)
    ap.add_argument("--cache-dir", action="append", default=[])
    ap.add_argument("--evidence-dir", action="append", default=[])
    ap.add_argument("--reference-dir", action="append", default=[])
    ap.add_argument("--worktrees-dir")
    ap.add_argument("--board-file")
    ap.add_argument("--broadsword")
    ap.add_argument("--force-list", action="store_true")
    ap.add_argument("--yes", action="store_true")
    ap.add_argument("--class", dest="classes", action="append", default=[])
    ap.add_argument("--approval-file")
    return ap.parse_args(argv)


def validate(args):
    if args.trigger_gb != args.trigger_gb or args.trigger_gb < 0 or args.trigger_gb == float("inf"):
        raise Refusal("--trigger-gb must be a finite number >= 0")
    if args.cold_days != args.cold_days or args.cold_days < 0 or args.cold_days == float("inf"):
        raise Refusal("--cold-days must be a finite number >= 0")
    for c in args.classes:
        if c not in CLASSES:
            raise Refusal(f"unknown --class {c!r}; classes are {', '.join(CLASSES)}")
    if args.yes and args.force_list:
        raise Refusal("--force-list is for a dry run only; it cannot be combined with --yes")
    if args.yes and not args.classes:
        raise Refusal("--yes needs at least one --class: only the named classes are moved")
    if args.yes and not args.approval_file:
        raise Refusal(f"--yes needs --approval-file. {REMINDER}")
    if args.worktrees_dir and bool(args.board_file) == bool(args.broadsword):
        raise Refusal("worktree class needs exactly one of --board-file or --broadsword")


def acquire_lock(state):
    """Exclusive-create the run lock in the state folder. Refuse when it exists (never removed here)."""
    if not state.is_dir():
        raise Refusal("--state-dir is not an existing folder; cannot take the run lock")
    lock = state / LOCK_NAME
    try:
        fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    except FileExistsError:
        raise Refusal(
            f"run lock {lock.name} is held: another run is moving, or one was killed. "
            "Check, then remove the file by hand."
        )
    except OSError as e:
        raise Refusal(f"cannot take the run lock: {e}")
    with os.fdopen(fd, "w") as f:
        f.write(f"pid {os.getpid()}\n")
    return lock


def run(args, out):
    validate(args)
    lock = None
    try:
        root = Path(args.root).resolve()
        if args.yes and root.is_dir():
            lock = acquire_lock(rel_under(root, args.state_dir, "--state-dir"))
        return run_locked(args, out)
    finally:
        if lock is not None:
            try:
                lock.unlink()
            except OSError:
                pass


def run_locked(args, out):
    root = Path(args.root).resolve()
    if not root.is_dir():
        raise Refusal(f"--root {args.root!r} is not a folder")
    target = Path(args.target).resolve()
    if is_inside(target, root) or is_inside(root, target):
        raise Refusal("--target must be a different folder, not inside root or holding it")
    if not target.is_dir():
        raise Refusal(f"--target {args.target!r} is not an existing folder (is the drive attached?)")
    state = rel_under(root, args.state_dir, "--state-dir")
    approval = None
    if args.yes:
        approval = check_approval(root, state, args.approval_file, args.classes)
    free = free_gb(root)
    if free >= args.trigger_gb and not args.force_list:
        raise Refusal(
            f"not running: free space {free:.1f} GB is not under the trigger {args.trigger_gb:g} GB "
            "(--force-list shows the list as a dry run)"
        )
    items = build_plan(args, root, target, state)
    out.write(f"free space {free:.1f} GB, trigger {args.trigger_gb:g} GB\n")
    for it in items:
        status = f"SKIP ({it.skip})" if it.skip else "move"
        out.write(f"{it.cls}\t{it.size}\t{it.src}\t->\t{it.dst}\t{status}\n")
    todo = [i for i in items if not i.skip]
    if not args.yes:
        out.write(f"dry run: {len(todo)} item(s) would move, nothing moved. {REMINDER}\n")
        return 0
    # The approval is used up before the first move, so it can never cover a second run.
    used = consume_approval(approval)
    out.write(f"approval used up: {approval.name} -> {used.name}\n")
    moved = failed = 0
    for it in todo:
        if it.cls not in args.classes:
            out.write(f"not moved (class {it.cls} not named by --class): {it.src}\n")
            continue
        try:
            do_move(args, root, it)
            moved += 1
            out.write(f"moved {it.src} -> {it.dst}\n")
        except MoveFailed as e:
            failed += 1
            out.write(f"FAILED {it.src}: {e}\n")
    out.write(f"done: {moved} moved, {failed} failed\n")
    return 1 if failed else 0


def main(argv=None):
    try:
        args = parse(sys.argv[1:] if argv is None else argv)
        return run(args, sys.stdout)
    except Refusal as e:
        print(f"refused: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
