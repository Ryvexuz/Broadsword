"""Read-only release-folder gate.

The Windows DLL allow-list is not final: changes require the orchestrator's
record in the evidence (STAGE7 3.4). Build support belongs to n20-26b-2.
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import os
import re
import stat
import struct

ALLOWED_DLLS = frozenset({
    "kernel32.dll", "ntdll.dll", "user32.dll", "gdi32.dll", "ws2_32.dll",
    "advapi32.dll", "bcryptprimitives.dll", "ucrtbase.dll", "shell32.dll",
    "ole32.dll", "msvcrt.dll",
})
API_MS_WIN_PREFIX = "api-ms-win-"
REQUIRED_FILES = (
    "race.exe", "crash-sim.exe", "cars/car_basic.bsn", "tracks/oval.track",
    "tracks/chicane.track", "demo/oval_lap.input", "README.txt",
)


class PeError(ValueError):
    def __init__(self, name):
        self.name = name
        super().__init__(name)


def read_imports(data: bytes) -> list[str]:
    """Read the PE32+ import and delay-import subset, with bounded reads."""
    def take(offset, size):
        if offset < 0 or offset + size > len(data):
            raise PeError("Truncated")
        return data[offset:offset + size]

    def u16(offset):
        return struct.unpack("<H", take(offset, 2))[0]

    def u32(offset):
        return struct.unpack("<I", take(offset, 4))[0]

    if data[:2] != b"MZ":
        raise PeError("NotPe")
    pe = u32(0x3C)
    if take(pe, 4) != b"PE\0\0":
        raise PeError("NotPe")
    coff = pe + 4
    take(coff, 20)
    count, opt_size = u16(coff + 2), u16(coff + 16)
    opt = coff + 20
    take(opt, opt_size)
    if opt_size < 2:
        raise PeError("Truncated")
    if u16(opt) != 0x20B:
        raise PeError("UnsupportedMagic")
    if opt_size < 112:
        raise PeError("Truncated")
    directories = u32(opt + 108)
    section_start = opt + opt_size
    take(section_start, count * 40)
    sections = []
    for i in range(count):
        pos = section_start + i * 40
        virtual_size, address, raw_size, raw = struct.unpack(
            "<IIII", take(pos + 8, 16))
        sections.append((address, max(virtual_size, raw_size), raw))

    def mapped(rva):
        for address, size, raw in sections:
            if address <= rva < address + size:
                return raw + rva - address
        raise PeError("RvaOutsideSections")

    def dll_name(rva):
        offset = mapped(rva)
        name = bytearray()
        for i in range(256):
            byte = take(offset + i, 1)[0]
            if byte == 0:
                if not name:
                    raise PeError("BadName")
                return name.decode("ascii")
            if not 0x21 <= byte <= 0x7E:
                raise PeError("BadName")
            name.append(byte)
        raise PeError("BadName")

    result = []
    for index, stride, name_offset in ((1, 20, 12), (13, 32, 4)):
        if index >= directories:
            continue
        entry = 112 + index * 8
        if entry + 8 > opt_size:
            raise PeError("Truncated")
        rva = u32(opt + entry)
        if not rva:
            continue
        # Read the terminator after exactly 4096 imports, but no more imports.
        for i in range(4097):
            descriptor = take(mapped(rva + i * stride), stride)
            if not any(descriptor):
                break
            if i == 4096:
                raise PeError("TooManyImports")
            name_rva = struct.unpack_from("<I", descriptor, name_offset)[0]
            result.append(dll_name(name_rva))
    return result


_STRINGS = re.compile(rb"[\x20-\x7e]+")
_DRIVE = re.compile(rb"(?<![A-Za-z0-9])[A-Za-z]:[/\\]")


def scan_leaks(data: bytes):
    """Yield first drive-path hit per printable run; exempt before truncating."""
    for run in _STRINGS.finditer(data):
        hit = _DRIVE.search(data, run.start(), run.end())
        if hit is not None and data[run.end() - 4:run.end()].lower() != b".pdb":
            yield hit.start(), data[hit.start():min(hit.start() + 120, run.end())].decode("ascii")


class _LinkFound(OSError):
    pass


def _is_link(info):
    return (stat.S_ISLNK(info.st_mode)
            or bool(getattr(info, "st_file_attributes", 0) & 0x400))


def _windows_fd(path):
    """Pin an entry without following reparse points or permitting replacement."""
    import ctypes
    import msvcrt
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    create = kernel.CreateFileW
    create.argtypes = (wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                       wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD,
                       wintypes.HANDLE)
    create.restype = wintypes.HANDLE
    close = kernel.CloseHandle
    close.argtypes = (wintypes.HANDLE,)
    close.restype = wintypes.BOOL
    # OPEN_REPARSE_POINT | BACKUP_SEMANTICS; share read/write, not delete.
    handle = create(path, 0x80000000, 3, None, 3, 0x02200000, None)
    if handle == wintypes.HANDLE(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        class AttributeTag(ctypes.Structure):
            _fields_ = (("attributes", wintypes.DWORD), ("tag", wintypes.DWORD))

        query = kernel.GetFileInformationByHandleEx
        query.argtypes = (wintypes.HANDLE, ctypes.c_int, wintypes.LPVOID, wintypes.DWORD)
        query.restype = wintypes.BOOL
        info = AttributeTag()
        if not query(handle, 9, ctypes.byref(info), ctypes.sizeof(info)):
            raise ctypes.WinError(ctypes.get_last_error())
        if info.attributes & 0x400:
            raise _LinkFound()
        fd = msvcrt.open_osfhandle(handle, os.O_RDONLY | os.O_BINARY)
    except BaseException:
        close(handle)
        raise
    return fd


@contextlib.contextmanager
def _opened(path, expected, directory=False, parent_fd=None):
    """Open without following a last-component link; verify before reading."""
    if os.name == "nt":
        fd = _windows_fd(path)
    else:
        flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
        if directory:
            flags |= os.O_DIRECTORY
        try:
            fd = os.open(path, flags, dir_fd=parent_fd)
        except OSError:
            # A replacement link must be labelled link, not just unreadable.
            if _is_link(os.stat(path, dir_fd=parent_fd, follow_symlinks=False)):
                raise _LinkFound()
            raise
    try:
        # Older Windows Python versions expose zero file IDs through fstat.
        # The no-delete handle pins the path, so lstat retains its real ID.
        actual = os.lstat(path) if os.name == "nt" else os.fstat(fd)
        if _is_link(actual):
            raise _LinkFound()
        right_type = stat.S_ISDIR(actual.st_mode) if directory else stat.S_ISREG(actual.st_mode)
        if ((actual.st_dev, actual.st_ino) != (expected.st_dev, expected.st_ino)
                or not right_type):
            raise OSError("entry changed")
        yield fd
    finally:
        os.close(fd)


def check_folder(folder):
    imports, leaks = [], []
    pe_errors, links, unreadable = [], set(), set()
    sizes = {}
    unexpected_data = False

    def visit(path, rel, expected, parent_fd=None):
        nonlocal unexpected_data
        if _is_link(expected):
            links.add(rel)
            return
        directory = stat.S_ISDIR(expected.st_mode)
        if not directory and not stat.S_ISREG(expected.st_mode):
            unreadable.add(rel)
            return
        if directory:
            if rel == "data":
                unexpected_data = True
        else:
            sizes[rel] = expected.st_size
        open_path = path if os.name == "nt" or parent_fd is None else os.path.basename(path)
        try:
            with _opened(open_path, expected, directory, parent_fd) as fd:
                if directory:
                    # POSIX traversal is handle-relative; Windows pins parents
                    # against deletion/renaming until descendants are finished.
                    with os.scandir(path if os.name == "nt" else fd) as entries:
                        children = sorted(entries, key=lambda entry: entry.name)
                    for entry in children:
                        child_rel = entry.name if rel == "." else rel + "/" + entry.name
                        try:
                            info = (os.lstat(os.path.join(path, entry.name)) if os.name == "nt"
                                    else os.stat(entry.name, dir_fd=fd, follow_symlinks=False))
                        except OSError:
                            unreadable.add(child_rel)
                            continue
                        visit(os.path.join(path, entry.name), child_rel, info, fd)
                else:
                    with os.fdopen(os.dup(fd), "rb") as stream:
                        data = stream.read()
                    leaks.extend((rel, offset, text) for offset, text in scan_leaks(data))
                    if "/" not in rel and rel.lower().endswith((".exe", ".dll")):
                        try:
                            names = sorted({name.lower() for name in read_imports(data)})
                            imports.extend((rel, name) for name in names)
                        except PeError as error:
                            pe_errors.append((rel, error.name))
        except _LinkFound:
            sizes.pop(rel, None)
            links.add(rel)
        except (OSError, RecursionError):
            unreadable.add(rel)

    try:
        visit(folder, ".", os.lstat(folder))
    except OSError:
        unreadable.add(".")
    lines = [f"file {rel} {size}" for rel, size in sorted(sizes.items())]
    top_files = {rel.lower() for rel in sizes if "/" not in rel}
    missing_imports = 0
    for rel, dll in sorted(imports):
        decision = ("allowed" if dll in ALLOWED_DLLS or dll.startswith(API_MS_WIN_PREFIX)
                    else "beside" if dll in top_files else "MISSING")
        missing_imports += decision == "MISSING"
        lines.append(f"import {rel} {dll} {decision}")
    lines.extend(f"leak {rel} offset={offset} text={text}"
                 for rel, offset, text in sorted(leaks))
    problems = [f"missing {rel}" for rel in REQUIRED_FILES if rel not in sizes]
    problems.extend(f"empty {rel}" for rel in REQUIRED_FILES if sizes.get(rel) == 0)
    problems.extend(f"pe {rel} {name}" for rel, name in sorted(pe_errors))
    if unexpected_data:
        problems.append("unexpected data")
    problems.extend(f"link {rel}" for rel in sorted(links))
    problems.extend(f"unreadable {rel}" for rel in sorted(unreadable))
    lines.extend(problems)
    count = len(problems) + len(leaks) + missing_imports
    lines.append(f"release-check: FAIL problems={count}" if count
                 else f"release-check: ok files={len(sizes)} dlls={len(imports)}")
    print("\n".join(lines))
    return 1 if count else 0


def main(argv) -> int:
    if argv and argv[0] == "build":
        print("package_race: build is not implemented yet (n20-26b-2)", file=sys.stderr)
        return 2
    if (len(argv) != 3 or argv[:2] != ["check", "--folder"]
            or not argv[2] or argv[2].startswith("-")
            or any(char in argv[2] for char in "\0\r\n")):
        print("usage: package_race.py check --folder <folder>", file=sys.stderr)
        return 2
    try:
        folder = os.path.abspath(argv[2])
        info = os.lstat(folder)
        if not stat.S_ISDIR(info.st_mode) and not _is_link(info):
            raise OSError("not a directory")
    except (OSError, ValueError):
        print("package_race: folder does not exist or is not a directory", file=sys.stderr)
        return 2
    try:
        return check_folder(folder)
    except (OSError, UnicodeError, ValueError):
        # An unavailable output stream cannot turn a refusal into success.
        try:
            print("package_race: cannot write check output", file=sys.stderr)
        except (OSError, UnicodeError, ValueError):
            pass
        return 1


if __name__ == "__main__":
    sys.stdout.reconfigure(newline="\n")
    sys.stderr.reconfigure(newline="\n")
    sys.exit(main(sys.argv[1:]))
