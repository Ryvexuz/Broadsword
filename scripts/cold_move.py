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
of any class needs a new yes and a new file (this is stricter than renaming only after a
successful run: a run that fails part-way has used its approval too). A rename failure refuses
the run, nothing moved. A file whose name holds ".used-" is never accepted as an approval. Only
classes that are both named by --class and approved by the file are moved; do_move checks this
again for every item.

Never moved, whatever the flags: a path with a git-tracked file; anything inside --state-dir
(state/ records); a worktree the board does not list as finished (running, queued, unknown, or
the board cannot be read), or that git lists as locked; an item that is or holds a link
(symlink, NTFS junction or any other reparse point, found by scan_tree, which never follows
one), listed as "skipped: contains a link"; a folder holding a secret file (.env*); anything
whose target already exists. For the copy classes (build-cache, evidence, reference) every
candidate also goes through repo_guard: one holding a .git entry (file or folder) at any depth,
one that is, holds or sits inside a worktree from `git worktree list` (other than root itself),
and one that overlaps --worktrees-dir or --state-dir is skipped; the guard runs at planning and
again right before the copy. Files and folders are copied, the size and SHA-256 of every file
are verified, and only then is the source removed.

A retired worktree on the same drive as the target is moved with `git worktree move`, then
`git worktree repair`. Across drives (where `git worktree move` fails on Windows) it is copied,
every file (the worktree's .git file included) is verified by size and SHA-256, then
`git worktree repair <new path>` runs from root, `git worktree list` must show the new path and
no longer the old one, and only then is the source removed. If a step fails, the source is left
untouched, the copy this run made is removed (after a repair, git's record of the worktree,
<admin>/gitdir in the main repo, is first put back from the bytes saved before the repair; if
that fails the copy is kept so git never points at nothing), and the item is reported FAILED
with the step named. Right before
either kind of move the board, git's lock flag, the clean status and the target are re-checked.

Removal (delete_tree, the one place this script deletes a tree): the source must still match,
path for path and by size, mtime and file id, the snapshot taken before its copy, and hold no
link. Then it is renamed, in one atomic same-folder rename, to a quarantine name
<name>.cold-quarantine-<pid>-<id>, the quarantine is scanned again against the same snapshot, and
only the snapshot's files and folders are removed from it, deepest first. "Source untouched"
holds up to the remove step. If removal fails after the rename, what is left stays under the
quarantine name, never half-live at the source path, and the item is reported
"FAILED ...: at step remove: leftover kept at <quarantine path>"; the verified copy and, for a
worktree, git's record (which already points at the copy) are never touched. A partial copy is
removed the same way, and only the temp name this run created.

Windows: handle-relative (ctypes, no extra dependency). scan_tree, copy_item, manifest and the
removal open every entry as itself (FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS for
the item, NtCreateFile with FILE_OPEN_REPARSE_POINT relative to the handle of its already
verified parent folder for everything under it), check on that handle (GetFileInformationByHandle)
that it is no reparse point and, for the removal, that its file id, size and mtime are the
snapshot's, and then act on that handle: list the folder from it, read the file from it, delete
it through it (SetFileInformationByHandle, FileDispositionInfoEx with POSIX semantics and
FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE, so no chmod; a folder must be empty). The item
itself is opened by name, and its handle's final path must equal the item's path. Closed by
this, on Windows: following a folder swapped for a junction while a tree is scanned, copied,
hashed or removed (the handle in use is the original folder's, and an entry swapped before it is
opened opens as the junction itself and is refused); removing an entry swapped between its check
and its delete (the check and the delete are on one handle, and Windows refuses to rename a
folder while a handle to something inside it is open); removing a file replaced after the scan
(file id); clearing read-only anywhere. What stays: the item's own path is resolved by name once
(a swap of a folder above the item is caught by the final-path check, not prevented); the
quarantine rename acts on whatever is at the item's name at that moment (a swapped-in link is
renamed, never followed, and the re-scan then refuses it); the copy's destination, this run's own
fresh temp folder, is written by path with exclusive creates (a swap there can only add new files
elsewhere, never overwrite or delete); on a file system without POSIX delete (FAT, exFAT) the
read-only attribute is cleared through the same handle before the delete.
Elsewhere (not Windows): the same steps run path-based (_scan_tree_path, _copy_item_path,
_delete_snapshot_path): each folder is lstat'ed again right before it is listed, and each entry
and every folder above it right before each chmod, unlink and rmdir; a swap in the moment
between such a check and the call after it is not caught there.
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
import stat
import struct
import subprocess
import sys
import time
import uuid
from pathlib import Path

WINDOWS = os.name == "nt"
if WINDOWS:
    import ctypes
    from ctypes import wintypes

# Test hook. It may only make the guard stricter (lesson 11): True forces every worktree move
# through the verified copy path (copy, verify, repair, list, then remove), never the reverse.
# Tests use it because a second drive is not available. Nothing else sets it.
FORCE_WORKTREE_COPY = False
FILE_ATTRIBUTE_REPARSE_POINT = 0x400  # junctions, symlinks and every other reparse point
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


class LinkFound(Exception):
    """scan_tree met a symlink, junction or other reparse point; `where` is its relative path."""

    def __init__(self, where):
        super().__init__(f"contains a link ({where})")
        self.where = where


class TreeChanged(Exception):
    """The tree no longer matches the snapshot it was verified against; nothing was deleted."""


class RemoveFailed(Exception):
    """delete_tree failed after the quarantine rename; what is left is kept at `leftover`."""

    def __init__(self, leftover, cause):
        super().__init__(f"leftover kept at {leftover} ({cause})")
        self.leftover = leftover
        self.cause = cause


class PathMoved(OSError):
    """A path opened by name resolves somewhere else (a folder above it was swapped for a link)."""


# ---------------------------------------------------------------- tree walking and removal
def is_link_stat(st):
    """True for a symlink, an NTFS junction or any other reparse point (an lstat result)."""
    return stat.S_ISLNK(st.st_mode) or bool(
        getattr(st, "st_file_attributes", 0) & FILE_ATTRIBUTE_REPARSE_POINT
    )


def scan_tree(top):
    """Snapshot top without ever following a link: (dirs, files).

    dirs maps each relative posix path of a folder to its file id; files maps each relative
    posix path of a file to (size, mtime_ns, file id). A file top gives files {".": ...}.
    Raises LinkFound when top or anything under it is a link (symlink, junction, other reparse
    point), OSError when a folder cannot be read or an entry is neither a plain file nor a
    folder (fail closed, never skipped). On Windows the walk is handle-relative
    (_scan_tree_win); elsewhere it is path-based (_scan_tree_path).
    """
    return _scan_tree_win(top) if WINDOWS else _scan_tree_path(top)


def _scan_tree_path(top):
    """The non-Windows scan (path-based; also callable on Windows, which tests do).

    Each folder's (st_dev, st_ino) is recorded from an lstat when it is first seen, and the
    folder is lstat'ed again immediately before its os.scandir: a link or reparse point, or a
    different (st_dev, st_ino) (the folder was swapped while queued), raises LinkFound, so a
    junction swapped in between queueing and listing is never listed through. A swap between
    that lstat and the os.scandir right after it is not caught (path-based residual race).
    """
    st = os.lstat(top)
    if is_link_stat(st):
        raise LinkFound(".")
    if stat.S_ISREG(st.st_mode):
        return {}, {".": (st.st_size, st.st_mtime_ns, (st.st_dev, st.st_ino))}
    if not stat.S_ISDIR(st.st_mode):
        raise OSError(f"{top} is neither a plain file nor a folder")
    dirs, files = {}, {}
    stack = [(Path(top), "", (st.st_dev, st.st_ino))]
    while stack:
        folder, rel, ident = stack.pop()
        now = os.lstat(folder)  # immediately before the listing
        if (is_link_stat(now) or not stat.S_ISDIR(now.st_mode)
                or (now.st_dev, now.st_ino) != ident):
            raise LinkFound(rel or ".")
        with os.scandir(folder) as it:
            entries = sorted(it, key=lambda e: e.name)
        for e in entries:
            r = f"{rel}/{e.name}" if rel else e.name
            est = e.stat(follow_symlinks=False)
            if is_link_stat(est) or e.is_junction():
                raise LinkFound(r)
            if stat.S_ISDIR(est.st_mode):
                seen = os.lstat(e.path)  # DirEntry has no st_ino on Windows; lstat records it
                if is_link_stat(seen) or not stat.S_ISDIR(seen.st_mode):
                    raise LinkFound(r)
                dirs[r] = (seen.st_dev, seen.st_ino)
                stack.append((Path(e.path), r, (seen.st_dev, seen.st_ino)))
            elif stat.S_ISREG(est.st_mode):
                files[r] = (est.st_size, est.st_mtime_ns, (est.st_dev, est.st_ino))
            else:
                raise OSError(f"{r} is neither a plain file nor a folder")
    return dirs, files


def link_reason(path):
    """'contains a link (<where>)' when path is or holds a link, else None. OSError propagates."""
    try:
        scan_tree(path)
    except LinkFound as e:
        return str(e)
    return None


def _make_writable(path, st):
    """Clear the read-only attribute of one entry. Called only from _delete_snapshot_path (the
    non-Windows removal; the Windows removal never chmods), on an entry of
    the tree being removed that was just lstat'ed as a plain file or folder (never a link)."""
    if not st.st_mode & stat.S_IWRITE:
        os.chmod(path, stat.S_IMODE(st.st_mode) | stat.S_IWRITE)


def _plain_lstat(path, want_dir):
    st = os.lstat(path)
    if is_link_stat(st):
        raise LinkFound(str(path))
    if not (stat.S_ISDIR(st.st_mode) if want_dir else stat.S_ISREG(st.st_mode)):
        raise TreeChanged(f"{path} changed kind; not deleted")
    return st


def _plain_entry(q, rel, want_dir):
    """lstat q/rel and every folder above it up to and including q; each must be plain (no link,
    no reparse point). A folder swapped for a junction after the scan raises LinkFound here, so
    no chmod or unlink is ever sent through it. Returns the entry's own lstat result."""
    if rel is None:  # the quarantine root itself (its parent folder is not checked)
        return _plain_lstat(q, want_dir)
    p = q
    _plain_lstat(p, True)
    for part in rel.split("/")[:-1]:
        p = p / part
        _plain_lstat(p, True)
    return _plain_lstat(q / rel, want_dir)


def delete_tree(top, expected):
    """The one place this script deletes a file or tree.

    expected is a scan_tree snapshot taken before the copy was made and verified.
      1. top is scanned again (never following a link). A link (LinkFound) or any difference
         from expected in path, size, mtime or file id (TreeChanged): nothing is renamed or
         deleted.
      2. top is renamed (one atomic same-folder rename) to a fresh quarantine name. An OSError
         here (for example a file held open) propagates: nothing was deleted, top is as it was.
      3. The quarantine is scanned again against expected, then only the snapshot's files and
         folders are removed from it, deepest first.
         - Windows (_delete_snapshot_win): every entry is opened as itself, relative to the
           handle of its verified parent folder, its handle must show no reparse point and the
           snapshot's file id, size and mtime, and it is deleted through that handle (POSIX
           delete, read-only ignored: no chmod). Nothing is done by path.
         - Elsewhere (_delete_snapshot_path): before each entry, it and every folder above it up
           to the quarantine root are lstat'ed and must be plain (_plain_entry); read-only is
           cleared on the entry, then it is removed by path.
    Any failure in step 3 raises RemoveFailed(leftover=quarantine path): what is left stays
    there under that name, never half-live at top.
    """
    top = Path(top)
    if scan_tree(top) != expected:
        raise TreeChanged("the tree changed after it was verified; nothing deleted")
    q = top.parent / f"{top.name}.cold-quarantine-{os.getpid()}-{uuid.uuid4().hex}"
    if exists_any(q):  # a uuid name: cannot happen, but never rename onto something
        raise TreeChanged(f"quarantine name {q.name} already exists; nothing deleted")
    os.rename(top, q)
    try:
        now = scan_tree(q)
        if now != expected:
            raise TreeChanged("the tree changed during the quarantine rename")
        if WINDOWS:
            _delete_snapshot_win(q, expected)
        else:
            _delete_snapshot_path(q, now)
    except (LinkFound, TreeChanged, OSError) as e:
        raise RemoveFailed(q, e)


def _delete_snapshot_path(q, snap):
    """The non-Windows removal: path-based, with _plain_entry right before each call. A swap
    between that check and the chmod/unlink/rmdir right after it is not caught (residual race of
    path-based deletion; closed on Windows by _delete_snapshot_win)."""
    dirs, files = snap
    if "." in files:
        _make_writable(q, _plain_entry(q, None, False))
        os.unlink(q)
        return
    for rel in sorted(files):
        _make_writable(q / rel, _plain_entry(q, rel, False))
        os.unlink(q / rel)
    for rel in sorted(dirs, key=lambda r: r.count("/"), reverse=True) + [None]:
        _make_writable(q / rel if rel else q, _plain_entry(q, rel, True))
        os.rmdir(q / rel if rel else q)  # only ever an empty folder: never recursive


# ---------------------------------------------------------------- Windows, handle-relative
# Every entry is opened as itself (FILE_FLAG_OPEN_REPARSE_POINT / FILE_OPEN_REPARSE_POINT: a
# link is never followed), children relative to the handle of their already verified parent
# (NtCreateFile with RootDirectory), so a path swapped after a check can never redirect the
# handle actually used. Reads, listing and deletion all act on that handle.
_W_READ_DATA = 0x0001  # FILE_READ_DATA, = FILE_LIST_DIRECTORY on a folder
_W_READ_ATTRIBUTES = 0x0080
_W_WRITE_ATTRIBUTES = 0x0100
_W_DELETE = 0x00010000
_W_SYNCHRONIZE = 0x00100000
_W_ACCESS_READ = _W_SYNCHRONIZE | _W_READ_ATTRIBUTES | _W_READ_DATA
_W_ACCESS_DELETE = _W_ACCESS_READ | _W_DELETE | _W_WRITE_ATTRIBUTES
_W_SHARE_ALL = 0x7
_W_ATTR_READONLY = 0x1
_W_ATTR_DIRECTORY = 0x10
_W_ATTR_NORMAL = 0x80
_W_EPOCH_100NS = 116444736000000000

if WINDOWS:
    _k32 = ctypes.WinDLL("kernel32", use_last_error=True)
    _nt = ctypes.WinDLL("ntdll")

    class _UnicodeString(ctypes.Structure):
        _fields_ = [("Length", wintypes.USHORT), ("MaximumLength", wintypes.USHORT),
                    ("Buffer", ctypes.c_void_p)]

    class _ObjectAttributes(ctypes.Structure):
        _fields_ = [("Length", wintypes.ULONG), ("RootDirectory", wintypes.HANDLE),
                    ("ObjectName", ctypes.POINTER(_UnicodeString)), ("Attributes", wintypes.ULONG),
                    ("SecurityDescriptor", ctypes.c_void_p),
                    ("SecurityQualityOfService", ctypes.c_void_p)]

    class _IoStatusBlock(ctypes.Structure):
        _fields_ = [("Status", ctypes.c_void_p), ("Information", ctypes.c_size_t)]

    class _ByHandleInfo(ctypes.Structure):
        _fields_ = [("attrs", wintypes.DWORD), ("ctime", wintypes.FILETIME),
                    ("atime", wintypes.FILETIME), ("mtime", wintypes.FILETIME),
                    ("volume", wintypes.DWORD), ("size_hi", wintypes.DWORD),
                    ("size_lo", wintypes.DWORD), ("links", wintypes.DWORD),
                    ("index_hi", wintypes.DWORD), ("index_lo", wintypes.DWORD)]

    _k32.CreateFileW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p,
                                 wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
    _k32.CreateFileW.restype = wintypes.HANDLE
    _k32.CloseHandle.argtypes = [wintypes.HANDLE]
    _k32.CloseHandle.restype = wintypes.BOOL
    _k32.GetFileInformationByHandle.argtypes = [wintypes.HANDLE, ctypes.POINTER(_ByHandleInfo)]
    _k32.GetFileInformationByHandle.restype = wintypes.BOOL
    _k32.GetFileInformationByHandleEx.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                                  wintypes.DWORD]
    _k32.GetFileInformationByHandleEx.restype = wintypes.BOOL
    _k32.SetFileInformationByHandle.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p,
                                                wintypes.DWORD]
    _k32.SetFileInformationByHandle.restype = wintypes.BOOL
    _k32.ReadFile.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD,
                              ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
    _k32.ReadFile.restype = wintypes.BOOL
    _k32.GetFinalPathNameByHandleW.argtypes = [wintypes.HANDLE, wintypes.LPWSTR, wintypes.DWORD,
                                               wintypes.DWORD]
    _k32.GetFinalPathNameByHandleW.restype = wintypes.DWORD
    _nt.NtCreateFile.argtypes = [ctypes.POINTER(wintypes.HANDLE), wintypes.ULONG,
                                 ctypes.POINTER(_ObjectAttributes),
                                 ctypes.POINTER(_IoStatusBlock), ctypes.c_void_p, wintypes.ULONG,
                                 wintypes.ULONG, wintypes.ULONG, wintypes.ULONG, ctypes.c_void_p,
                                 wintypes.ULONG]
    _nt.NtCreateFile.restype = ctypes.c_long
    _nt.RtlNtStatusToDosError.argtypes = [wintypes.ULONG]
    _nt.RtlNtStatusToDosError.restype = wintypes.ULONG
    _W_INVALID = wintypes.HANDLE(-1).value


def _w_error(what, err=None):
    err = ctypes.get_last_error() if err is None else err
    e = ctypes.WinError(err)
    return OSError(e.errno, f"{what}: {e.strerror}", None, err)


def _w_close(h):
    _k32.CloseHandle(h)


class _WInfo:
    """What one handle refers to, from GetFileInformationByHandle."""

    def __init__(self, raw):
        self.attrs = raw.attrs
        self.is_dir = bool(raw.attrs & _W_ATTR_DIRECTORY)
        self.is_link = bool(raw.attrs & FILE_ATTRIBUTE_REPARSE_POINT)
        self.size = (raw.size_hi << 32) | raw.size_lo
        ft = (raw.mtime.dwHighDateTime << 32) | raw.mtime.dwLowDateTime
        self.mtime_ns = (ft - _W_EPOCH_100NS) * 100
        self.fid = (raw.volume, (raw.index_hi << 32) | raw.index_lo)


def _w_info(h, what):
    raw = _ByHandleInfo()
    if not _k32.GetFileInformationByHandle(h, ctypes.byref(raw)):
        raise _w_error(f"cannot read the attributes of {what}")
    return _WInfo(raw)


def _w_final_path(h):
    buf = ctypes.create_unicode_buffer(32768)
    n = _k32.GetFinalPathNameByHandleW(h, buf, len(buf), 0)
    if n == 0 or n >= len(buf):
        raise _w_error("cannot read the final path of a handle")
    p = buf.value
    if p.startswith("\\\\?\\UNC\\"):
        return "\\\\" + p[8:]
    return p[4:] if p.startswith("\\\\?\\") else p


def _w_open_root(path, access):
    """Open path itself (a link opens as the link, never followed). Returns (handle, info).

    A link raises LinkFound("."). The handle's final path must equal path: if a folder above it
    resolves elsewhere (was swapped for a link), PathMoved is raised and nothing is done.
    """
    full = os.path.abspath(str(path))
    long = ("\\\\?\\UNC\\" + full[2:]) if full.startswith("\\\\") else ("\\\\?\\" + full)
    flags = 0x02000000 | 0x00200000  # FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    h = _k32.CreateFileW(long, access, _W_SHARE_ALL, None, 3, flags, None)  # 3 = OPEN_EXISTING
    if h is None or h == _W_INVALID:
        raise _w_error(f"cannot open {path}")
    try:
        info = _w_info(h, path)
        if info.is_link:
            raise LinkFound(".")
        final = _w_final_path(h)
        if os.path.normcase(final) != os.path.normcase(full):
            raise PathMoved(f"{path} resolves to {final}: a folder above it is a link; nothing done")
    except BaseException:
        _w_close(h)
        raise
    return h, info


def _w_open_child(parent, name, access, what):
    """Open the entry `name` of the folder handle `parent` as itself (FILE_OPEN_REPARSE_POINT),
    relative to that handle (NtCreateFile RootDirectory), never by a full path."""
    if not name or name in (".", "..") or "\\" in name or "/" in name:
        raise TreeChanged(f"bad entry name {name!r} under {what}")
    raw = name.encode("utf-16-le")
    buf = ctypes.create_string_buffer(raw + b"\0\0")
    us = _UnicodeString(len(raw), len(raw) + 2, ctypes.cast(buf, ctypes.c_void_p))
    oa = _ObjectAttributes(ctypes.sizeof(_ObjectAttributes), parent, ctypes.pointer(us),
                           0x40, None, None)  # OBJ_CASE_INSENSITIVE, as Win32 names are
    iosb = _IoStatusBlock()
    h = wintypes.HANDLE()
    # FILE_OPEN (1); FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT | FILE_OPEN_FOR_BACKUP_INTENT
    status = _nt.NtCreateFile(ctypes.byref(h), access, ctypes.byref(oa), ctypes.byref(iosb), None,
                              0, _W_SHARE_ALL, 1, 0x00200000 | 0x20 | 0x4000, None, 0)
    if status < 0:
        raise _w_error(f"cannot open {what}", _nt.RtlNtStatusToDosError(status & 0xFFFFFFFF))
    return h.value


def _w_listdir(h, what):
    """Names in the folder handle h (FileFullDirectoryInfo on the handle, never a path)."""
    names = []
    buf = ctypes.create_string_buffer(65536)
    cls = 15  # FileFullDirectoryRestartInfo, then FileFullDirectoryInfo (14)
    while True:
        if not _k32.GetFileInformationByHandleEx(h, cls, buf, len(buf)):
            err = ctypes.get_last_error()
            if err == 18:  # ERROR_NO_MORE_FILES
                return names
            raise _w_error(f"cannot list {what}", err)
        cls = 14
        raw = buf.raw
        off = 0
        while True:
            nxt, = struct.unpack_from("<I", raw, off)
            nlen, = struct.unpack_from("<I", raw, off + 60)
            name = raw[off + 68: off + 68 + nlen].decode("utf-16-le")
            if name not in (".", ".."):
                names.append(name)
            if nxt == 0:
                break
            off += nxt


def _w_read_chunks(h, what):
    buf = ctypes.create_string_buffer(1 << 20)
    got = wintypes.DWORD()
    while True:
        if not _k32.ReadFile(h, buf, len(buf), ctypes.byref(got), None):
            raise _w_error(f"cannot read {what}")
        if got.value == 0:
            return
        yield buf.raw[:got.value]


def _w_delete(h, what):
    """Delete the entry the handle h refers to, through the handle: POSIX semantics (the name
    goes when the handle closes) and FILE_DISPOSITION_FLAG_IGNORE_READONLY_ATTRIBUTE (no chmod).
    A folder must be empty (never recursive). On a file system without that call (FAT, exFAT)
    the read-only attribute is cleared on the same handle and the plain disposition is set."""
    flags = ctypes.c_ulong(0x1 | 0x2 | 0x10)  # DELETE | POSIX_SEMANTICS | IGNORE_READONLY_ATTRIBUTE
    if _k32.SetFileInformationByHandle(h, 21, ctypes.byref(flags), 4):  # FileDispositionInfoEx
        return
    err = ctypes.get_last_error()
    if err not in (1, 50, 87):  # not "unsupported" on this file system: a real refusal
        raise _w_error(f"cannot delete {what}", err)
    info = _w_info(h, what)
    if info.attrs & _W_ATTR_READONLY:
        attrs = (info.attrs & ~_W_ATTR_READONLY) or _W_ATTR_NORMAL
        basic = struct.pack("<qqqqI4x", 0, 0, 0, 0, attrs)  # FILE_BASIC_INFO, times unchanged
        bbuf = ctypes.create_string_buffer(basic, len(basic))
        if not _k32.SetFileInformationByHandle(h, 0, bbuf, len(basic)):  # FileBasicInfo
            raise _w_error(f"cannot clear read-only on {what}")
    yes = ctypes.c_ubyte(1)
    if not _k32.SetFileInformationByHandle(h, 4, ctypes.byref(yes), 1):  # FileDispositionInfo
        raise _w_error(f"cannot delete {what}")


def _scan_tree_win(top):
    h, info = _w_open_root(top, _W_ACCESS_READ)
    try:
        if not info.is_dir:
            return {}, {".": (info.size, info.mtime_ns, info.fid)}
        dirs, files = {}, {}
        _w_scan_dir(h, Path(top), "", dirs, files)
        return dirs, files
    finally:
        _w_close(h)


def _w_scan_dir(h, top, rel, dirs, files):
    for name in sorted(_w_listdir(h, top / rel if rel else top)):
        r = f"{rel}/{name}" if rel else name
        ch = _w_open_child(h, name, _W_ACCESS_READ, top / r)
        try:
            info = _w_info(ch, top / r)
            if info.is_link:
                raise LinkFound(r)
            if info.is_dir:
                dirs[r] = info.fid
                _w_scan_dir(ch, top, r, dirs, files)
            else:
                files[r] = (info.size, info.mtime_ns, info.fid)
        finally:
            _w_close(ch)


def _w_check(info, expected, what):
    if info.is_link:
        raise LinkFound(str(what))
    if (info.size, info.mtime_ns, info.fid) != expected:
        raise TreeChanged(f"{what} is not the verified file (size, mtime or file id); not deleted")


def _delete_snapshot_win(q, expected):
    """Delete the quarantine q through handles only (see delete_tree, step 3)."""
    dirs, files = expected
    h, info = _w_open_root(q, _W_ACCESS_DELETE)
    try:
        if "." in files:
            _w_check(info, files["."], q)
            _w_delete(h, q)
            return
        if not info.is_dir:
            raise TreeChanged(f"{q} is no longer a folder; not deleted")
        _w_delete_dir(h, q, "", dirs, files)
        _w_delete(h, q)
    finally:
        _w_close(h)


def _w_delete_dir(h, q, rel, dirs, files):
    for name in sorted(_w_listdir(h, q / rel if rel else q)):
        r = f"{rel}/{name}" if rel else name
        if r not in files and r not in dirs:
            raise TreeChanged(f"{q / r} appeared after the verify; not deleted")
        ch = _w_open_child(h, name, _W_ACCESS_DELETE, q / r)
        try:
            info = _w_info(ch, q / r)
            if info.is_link:
                raise LinkFound(r)
            if info.is_dir:
                if dirs.get(r) != info.fid:
                    raise TreeChanged(f"{q / r} is not the verified folder (file id); not deleted")
                _w_delete_dir(ch, q, r, dirs, files)
            else:
                _w_check(info, files.get(r), q / r)
            _w_delete(ch, q / r)
        finally:
            _w_close(ch)


def _w_copy_file(h, out, info, what):
    """Copy the file behind the verified handle h to out (a new file, exclusive create in this
    run's own temp copy), then set its mtime and, if the source is read-only, its read-only
    attribute (on the copy only)."""
    with open(out, "xb") as f:
        for chunk in _w_read_chunks(h, what):
            f.write(chunk)
    os.utime(out, ns=(info.mtime_ns, info.mtime_ns))
    if info.attrs & _W_ATTR_READONLY:
        os.chmod(out, stat.S_IREAD)


def _copy_item_win(src, dst):
    h, info = _w_open_root(src, _W_ACCESS_READ)
    try:
        dst.parent.mkdir(parents=True, exist_ok=True)
        if not info.is_dir:
            _w_copy_file(h, dst, info, src)
            return
        os.mkdir(dst)  # fails if dst exists: never copies into something this run did not make
        _w_copy_dir(h, Path(src), dst, "")
    finally:
        _w_close(h)


def _w_copy_dir(h, src, dst, rel):
    for name in sorted(_w_listdir(h, src / rel if rel else src)):
        r = f"{rel}/{name}" if rel else name
        ch = _w_open_child(h, name, _W_ACCESS_READ, src / r)
        try:
            info = _w_info(ch, src / r)
            if info.is_link:
                raise LinkFound(r)
            if info.is_dir:
                os.mkdir(dst / r)
                _w_copy_dir(ch, src, dst, r)
            else:
                _w_copy_file(ch, dst / r, info, src / r)
        finally:
            _w_close(ch)


def _manifest_win(path):
    h, info = _w_open_root(path, _W_ACCESS_READ)
    try:
        if not info.is_dir:
            return {".": (info.size, _w_sha256(h, path))}
        out = {}
        _w_manifest_dir(h, Path(path), "", out)
        return out
    finally:
        _w_close(h)


def _w_sha256(h, what):
    d = hashlib.sha256()
    for chunk in _w_read_chunks(h, what):
        d.update(chunk)
    return d.hexdigest()


def _w_manifest_dir(h, top, rel, out):
    for name in sorted(_w_listdir(h, top / rel if rel else top)):
        r = f"{rel}/{name}" if rel else name
        ch = _w_open_child(h, name, _W_ACCESS_READ, top / r)
        try:
            info = _w_info(ch, top / r)
            if info.is_link:
                raise LinkFound(r)
            if info.is_dir:
                out[r] = None
                _w_manifest_dir(ch, top, r, out)
            else:
                out[r] = (info.size, _w_sha256(ch, top / r))
        finally:
            _w_close(ch)


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
    """Return a reason string if path is or holds a link or a secret file, else None.

    Links (symlinks, junctions, other reparse points) are found by scan_tree, which never
    follows one; the reason then reads "contains a link (<where>)".
    """
    try:
        dirs, files = scan_tree(path)
    except LinkFound as e:
        return str(e)
    if "." in files:
        return "is a secret file" if path.name.lower().startswith(".env") else None
    for rel in sorted(set(dirs) | set(files)):
        n = rel.rsplit("/", 1)[-1]
        if n.lower().startswith(".env"):
            return f"holds a secret file ({n})"
    return None


def size_and_newest(path):
    """Total size and newest mtime (seconds) of path, from scan_tree (links are never followed)."""
    _dirs, files = scan_tree(path)
    total = sum(v[0] for v in files.values())
    newest = max((v[1] for v in files.values()), default=0) / 1e9
    if newest == 0.0:
        newest = os.lstat(path).st_mtime
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


def worktree_records(root):
    """{resolved path: True when locked} for every worktree `git worktree list --porcelain`
    names. Fails closed (Refusal) when git fails."""
    p = run_git(root, "worktree", "list", "--porcelain")
    if p.returncode != 0:
        raise Refusal(f"git worktree list failed: {p.stderr.strip()}")
    out = {}
    cur = None
    for line in p.stdout.splitlines():
        if line.startswith("worktree "):
            cur = Path(line[len("worktree "):]).resolve()
            out[cur] = False
        elif cur is not None and (line == "locked" or line.startswith("locked ")):
            out[cur] = True
    return out


def registered_worktrees(root):
    """Resolved paths of every worktree `git worktree list --porcelain` names. Fails closed."""
    return set(worktree_records(root))


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
    why = link_reason(src)  # first: resolve() and the walk below must never go through a link
    if why is not None:
        return why
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
        # Links first: no later walk (repo_guard, git, size) runs over a tree holding one.
        skip = tree_has_symlink_or_secret(src)
        if skip is None:
            skip = repo_guard(guard, src)
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
    except LinkFound as e:  # a link that appeared between the checks above
        skip = str(e)
    except OSError as e:
        skip = f"unreadable ({e})"
    return Item(cls, src, dst, size, skip, guard)


def plan_worktrees(args, root, target, state, records):
    base = rel_under(root, args.worktrees_dir, "--worktrees-dir")
    if not base.is_dir():
        return []
    board = read_board(args)
    items = []
    for proj in sorted(base.iterdir()):
        if is_link_stat(os.lstat(proj)):
            items.append(Item("worktree", proj, target_for(target, "worktree", root, proj), 0,
                              "contains a link (.)"))
            continue
        if not proj.is_dir():
            continue
        for wt in sorted(proj.iterdir()):
            dst = target_for(target, "worktree", root, wt)
            size = 0
            skip = None
            status = board.get(wt.name)
            if is_link_stat(os.lstat(wt)):
                skip = "contains a link (.)"
            elif not wt.is_dir():
                skip = "not a plain folder"
            elif status is None:
                skip = "not on the board (treated as active)"
            elif status not in FINISHED:
                skip = f"active worktree (board status {status})"
            elif wt.resolve() not in records:
                skip = "not registered with git"
            elif records[wt.resolve()]:
                skip = "locked in git (git worktree lock)"
            elif is_inside(wt.resolve(), state):
                skip = "inside the state folder"
            else:
                try:
                    skip = worktree_tree_reason(wt)
                    if skip is None and (dst.exists() or dst.is_symlink()):
                        skip = "target already exists"
                    if skip is None:
                        size, _n = size_and_newest(wt)
                except LinkFound as e:
                    skip = str(e)
                except OSError as e:
                    skip = f"unreadable ({e})"
            items.append(Item("worktree", wt, dst, size, skip))
    return items


def worktree_tree_reason(wt):
    """Why the worktree folder must not move (link, secret, dirty), else None. Links are checked
    first, so git never runs over a tree that holds one. OSError propagates (fail closed)."""
    why = tree_has_symlink_or_secret(wt)
    if why is not None:
        return why
    st = run_git(wt, "status", "--porcelain")
    if st.returncode != 0:
        return f"git status failed: {st.stderr.strip()}"
    if st.stdout.strip():
        return "dirty (uncommitted changes)"
    return None


def build_plan(args, root, target, state):
    records = worktree_records(root)
    wdir = rel_under(root, args.worktrees_dir, "--worktrees-dir") if args.worktrees_dir else None
    guard = RepoGuard(root, state, wdir, set(records))
    items = []
    items += plan_children(args, root, target, state, "build-cache", args.cache_dir, True, guard)
    items += plan_children(args, root, target, state, "evidence", args.evidence_dir, False, guard)
    items += plan_children(args, root, target, state, "reference", args.reference_dir, False, guard)
    if args.worktrees_dir:
        items += plan_worktrees(args, root, target, state, records)
    return items


# ---------------------------------------------------------------- moving
def manifest(path):
    """{relative path: (size, sha256) or None for a folder} of a file or tree.

    A link anywhere raises LinkFound instead of being followed. On Windows every file is hashed
    from its own handle, opened relative to its verified parent folder (_manifest_win).
    """
    if WINDOWS:
        return _manifest_win(path)
    dirs, files = scan_tree(path)
    if "." in files:
        return {".": (files["."][0], sha256_file(path))}
    out = {d: None for d in dirs}
    for rel, v in files.items():
        out[rel] = (v[0], sha256_file(Path(path) / rel))
    return out


def copy_item(src, dst):
    """Copy a file or tree to dst, which must not exist, never through a link.

    Windows (_copy_item_win): every folder and file is opened as itself relative to the handle
    of its verified parent, and each file is read from its own handle, so a folder swapped for a
    junction during the copy is never read through. Only the destination (this run's own fresh
    temp copy) is written by path, with exclusive creates.
    Elsewhere (_copy_item_path): path-based, see there.
    """
    if WINDOWS:
        _copy_item_win(src, dst)
    else:
        _copy_item_path(src, dst)


def _copy_item_path(src, dst):
    """The non-Windows copy: the tree is scanned first (LinkFound if it holds a link: nothing is
    copied), then only the scanned folders and files are copied, each lstat'ed as a plain entry
    right before. Residual race: a folder swapped for a junction between that lstat and the copy
    of a file inside it is not caught here; the verify step re-scans the source and fails the
    item on it (closed on Windows by _copy_item_win)."""
    dirs, files = scan_tree(src)
    dst.parent.mkdir(parents=True, exist_ok=True)
    if "." in files:
        _plain_lstat(src, False)
        shutil.copy2(src, dst, follow_symlinks=False)
        return
    os.mkdir(dst)  # fails if dst exists: never copies into something this run did not make
    for rel in sorted(dirs, key=lambda r: r.count("/")):
        _plain_lstat(src / rel, True)
        os.mkdir(dst / rel)
    for rel in sorted(files):
        _plain_lstat(src / rel, False)
        shutil.copy2(src / rel, dst / rel, follow_symlinks=False)


def verify_copy(src, dst):
    """Raise MoveFailed unless every entry, size and SHA-256 of dst equals src (no links)."""
    try:
        a, b = manifest(src), manifest(dst)
    except LinkFound as e:
        raise MoveFailed(f"a link appeared during the copy: {e}")
    if a != b:
        bad = sorted(k for k in set(a) | set(b) if a.get(k) != b.get(k))
        raise MoveFailed(f"copy does not match the source (size or SHA-256), first difference: {bad[0]}")


def snapshot_source(src):
    """scan_tree(src) for the copy; a link or an unreadable folder fails the item, nothing copied."""
    try:
        return scan_tree(src)
    except LinkFound as e:
        raise MoveFailed(f"step check: {e} (found right before the copy); nothing copied")
    except OSError as e:
        raise MoveFailed(f"step check: source unreadable ({e}); nothing copied")


def new_temp_name(dst):
    tmp = dst.parent / f"{dst.name}.cold-tmp-{os.getpid()}-{uuid.uuid4().hex}"
    if exists_any(tmp):  # a uuid name: cannot happen, but then it is not ours to remove
        raise MoveFailed(f"temp path {tmp.name} already exists; nothing copied")
    return tmp


def remove_source(src, snap, what):
    """Remove the verified source through delete_tree only. Failure is reported, never forced;
    the verified copy (and git's record of a worktree) is never touched here."""
    try:
        delete_tree(src, snap)
    except RemoveFailed as e:
        raise MoveFailed(f"at step remove: {e}; {what}, untouched")
    except LinkFound as e:
        raise MoveFailed(f"at step remove: source holds a link now ({e}); nothing deleted, "
                         f"source kept at {src}; {what}")
    except TreeChanged as e:
        raise MoveFailed(f"at step remove: {e}; source kept at {src}; {what}")
    except OSError as e:
        raise MoveFailed(f"at step remove: could not quarantine the source ({e}); nothing "
                         f"deleted, source kept at {src}; {what}")


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
    # Links first, so repo_guard's walk never runs over a tree holding one. This snapshot is
    # what delete_tree later requires the source to still match.
    snap = snapshot_source(item.src)
    if item.guard is not None:
        try:
            why = repo_guard(item.guard, item.src)
        except OSError as e:
            raise MoveFailed(f"repo guard could not read the source ({e}); nothing copied")
        if why is not None:
            raise MoveFailed(f"{why} (found right before the copy); nothing copied")
    try:
        item.dst.parent.mkdir(parents=True, exist_ok=True)
        tmp = new_temp_name(item.dst)
    except OSError as e:
        raise MoveFailed(f"step copy: cannot prepare the target folder ({e}); nothing copied")
    step = "copy"
    try:
        copy_item(item.src, tmp)
        step = "verify"
        verify_copy(item.src, tmp)
        step = "rename into place"
        if exists_any(item.dst):
            raise MoveFailed("target appeared during the copy; temp copy dropped, target untouched")
        os.rename(tmp, item.dst)  # fails if dst exists on Windows; the check above covers POSIX
    except (MoveFailed, OSError, LinkFound, TreeChanged) as e:
        note = drop_own_copy(tmp)
        raise MoveFailed(f"step {step} failed: {e}; source kept; {note}")
    # Verified and in place. Only now is the source removed.
    remove_source(item.src, snap, f"copy verified at {item.dst}")


def drop_own_copy(path):
    """Remove a copy this run made (a fresh temp name, or the target it renamed that temp to).

    Goes through delete_tree, so it never follows or deletes through a link. Returns a note for
    the report; a copy that cannot be removed is left and named, never forced.
    """
    if not exists_any(path):
        return "no partial copy was left"
    try:
        delete_tree(path, scan_tree(path))
        return f"partial copy {path.name} removed"
    except RemoveFailed as e:
        return f"partial copy not fully removed: {e}"
    except (LinkFound, TreeChanged, OSError) as e:
        return f"partial copy left at {path} ({e})"


def same_volume(src, dst):
    """True when src and the existing part of dst's path are on one volume (same drive and st_dev).

    FORCE_WORKTREE_COPY (test hook) makes this False, which only ever selects the stricter,
    verified copy path. Any error also selects the copy path.
    """
    if FORCE_WORKTREE_COPY:
        return False
    probe = dst.parent
    while not probe.exists() and probe != probe.parent:
        probe = probe.parent
    try:
        return (src.drive.lower() == probe.drive.lower()
                and os.stat(src).st_dev == os.stat(probe).st_dev)
    except OSError:
        return False


def recheck_worktree(args, root, item):
    """Right before a worktree moves: target absent, board finished, not locked, registered,
    no link, no secret, clean. Planning is not enough. Raises MoveFailed (source kept)."""
    if exists_any(item.dst):
        raise MoveFailed("target appeared after planning; source kept, nothing moved")
    try:
        status = read_board(args).get(item.src.name)
    except Refusal as e:
        raise MoveFailed(f"board unreadable at move time (source kept): {e}")
    if status not in FINISHED:
        raise MoveFailed(f"board status is now {status or 'missing'}, not finished; source kept")
    try:
        records = worktree_records(root)
        if item.src.resolve() not in records:
            raise MoveFailed("git no longer lists this worktree; source kept, nothing moved")
        if records[item.src.resolve()]:
            raise MoveFailed("worktree is locked in git now; source kept, nothing moved")
        why = worktree_tree_reason(item.src)
    except (Refusal, OSError) as e:
        raise MoveFailed(f"re-check failed at move time ({e}); source kept, nothing moved")
    if why is not None:
        raise MoveFailed(f"{why} (found right before the move); source kept, nothing moved")


def move_worktree(args, root, item):
    recheck_worktree(args, root, item)
    if not same_volume(item.src, item.dst):
        copy_worktree(root, item)
        return
    item.dst.parent.mkdir(parents=True, exist_ok=True)
    p = run_git(root, "worktree", "move", str(item.src), str(item.dst))
    if p.returncode != 0:
        raise MoveFailed(f"git worktree move failed (source kept): {p.stderr.strip()}")
    r = run_git(root, "worktree", "repair")
    if r.returncode != 0:
        raise MoveFailed(f"moved, but git worktree repair failed: {r.stderr.strip()}")


def verify_gitfile(src, copy):
    """The worktree's .git file: a plain file in both, the same bytes, and a gitdir: line."""
    a, b = src / ".git", copy / ".git"
    for p in (a, b):
        st = os.lstat(p)
        if is_link_stat(st) or not stat.S_ISREG(st.st_mode):
            raise MoveFailed(f"{p} is not a plain .git file")
    ta, tb = a.read_bytes(), b.read_bytes()
    if ta != tb or not ta.startswith(b"gitdir:"):
        raise MoveFailed("the copied .git file does not match the source's gitdir: file")


def admin_gitdir_file(src):
    """<admin>/gitdir: the file in the main repo's .git/worktrees/<name> where git records the
    worktree's path. Found from the worktree's own .git file ("gitdir: <admin>")."""
    line = (src / ".git").read_text(encoding="utf-8").strip()
    if not line.startswith("gitdir:"):
        raise MoveFailed("the worktree's .git file has no gitdir: line")
    admin = Path(line[len("gitdir:"):].strip())
    if not admin.is_absolute():
        admin = src / admin
    f = admin.resolve() / "gitdir"
    st = os.lstat(f)
    if is_link_stat(st) or not stat.S_ISREG(st.st_mode):
        raise MoveFailed(f"{f} is not a plain file")
    return f


def restore_admin_record(f, saved):
    """Put back the saved bytes of <admin>/gitdir (written to a fresh temp name next to it, then
    os.replace). Returns None when done, else the error text. Writes nothing in the source; the
    only file it may unlink is its own temp file, created exclusively ("xb") just before."""
    tmp = f.with_name(f"{f.name}.cold-restore-{uuid.uuid4().hex}")
    try:
        with open(tmp, "xb") as h:
            h.write(saved)
        os.replace(tmp, f)
    except OSError as e:
        try:
            if tmp.exists():
                os.unlink(tmp)
        except OSError:
            pass
        return str(e)
    return None


def copy_worktree(root, item):
    """Cross-drive worktree move: copy, verify, repair, list, and only then remove the source.

    On any failure before the removal: the source is untouched (nothing in it is written), the
    copy this run made is removed, and once a repair was started git's record (<admin>/gitdir)
    is first put back from the bytes saved before it (if that fails the copy is kept).
    """
    src, dst = item.src, item.dst
    snap = snapshot_source(src)
    if ".git" not in snap[1]:
        raise MoveFailed("step check: no .git file in the worktree; nothing copied")
    try:
        # git's own record of where this worktree lives (<admin>/gitdir). Its bytes are saved
        # so a failure after the repair can put them back without writing into the source
        # (`git worktree repair <old path>` would rewrite the source's .git file).
        admin_gitdir = admin_gitdir_file(src)
        saved_record = admin_gitdir.read_bytes()
    except (MoveFailed, OSError) as e:
        raise MoveFailed(f"step check failed: cannot read git's record of the worktree ({e}); "
                         "nothing copied")
    try:
        dst.parent.mkdir(parents=True, exist_ok=True)
        tmp = new_temp_name(dst)
    except OSError as e:
        raise MoveFailed(f"step copy: cannot prepare the target folder ({e}); nothing copied")
    made = tmp  # the one path this run created and may remove
    repaired = False  # True once `git worktree repair <new path>` has been started
    step = "copy"
    try:
        copy_item(src, tmp)
        step = "verify"
        verify_copy(src, tmp)
        verify_gitfile(src, tmp)
        step = "rename into place"
        if exists_any(dst):
            raise MoveFailed("target appeared during the copy; target untouched")
        os.rename(tmp, dst)
        made = dst
        step = "repair"
        repaired = True  # set before the call: a failed repair may still have changed the record
        r = run_git(root, "worktree", "repair", str(dst))
        if r.returncode != 0:
            raise MoveFailed(f"git worktree repair exited {r.returncode}: {r.stderr.strip()}")
        step = "list"
        records = worktree_records(root)
        if dst.resolve() not in records:
            raise MoveFailed("git worktree list does not show the new path")
        if src.resolve() in records:
            raise MoveFailed("git worktree list still shows the old path")
    except (MoveFailed, OSError, LinkFound, TreeChanged, Refusal) as e:
        if repaired:
            bad = restore_admin_record(admin_gitdir, saved_record)
            if bad is not None:
                # git may still point at the copy: keep it, so no git record points at nothing.
                raise MoveFailed(
                    f"step {step} failed: {e}; source kept untouched; putting back git's record "
                    f"{admin_gitdir} also failed ({bad}): run `git worktree repair {src}` by "
                    f"hand, then remove the copy at {made}"
                )
            note = "git pointed back at the source; " + drop_own_copy(made)
        else:
            note = drop_own_copy(made)
        raise MoveFailed(f"step {step} failed: {e}; source kept untouched; {note}")
    # Copied, verified, repaired and listed. Only now is the source removed.
    remove_source(src, snap, f"the worktree is now at {dst} (copied, verified, repaired, listed)")


def do_move(args, root, item, approved):
    """Move one item. The class must be approved by the approval file (checked here as well as
    when the run starts, so no caller can move an unapproved class)."""
    if item.cls not in approved or item.cls not in args.classes:
        raise MoveFailed(f"class {item.cls} is not approved for this run; nothing moved")
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
    if ".used-" in f.name.lower():
        raise Refusal(
            f"refusing --yes: {rel} is a used-up approval (its name holds .used-); "
            f"each run needs a new one. {REMINDER}"
        )
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
    return f, frozenset(approved)


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
    approval, approved = None, frozenset()
    if args.yes:
        approval, approved = check_approval(root, state, args.approval_file, args.classes)
    free = free_gb(root)
    if free >= args.trigger_gb and not args.force_list:
        raise Refusal(
            f"not running: free space {free:.1f} GB is not under the trigger {args.trigger_gb:g} GB "
            "(--force-list shows the list as a dry run)"
        )
    items = build_plan(args, root, target, state)
    out.write(f"free space {free:.1f} GB, trigger {args.trigger_gb:g} GB\n")
    for it in items:
        status = f"skipped: {it.skip}" if it.skip else "move"
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
        if it.cls not in args.classes or it.cls not in approved:
            out.write(f"not moved (class {it.cls} not named by --class and approved): {it.src}\n")
            continue
        try:
            do_move(args, root, it, approved)
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
