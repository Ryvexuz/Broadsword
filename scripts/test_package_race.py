"""Release-folder gate tests. Run with python -I -B scripts/test_package_race.py -v."""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import os
import pathlib
import random
import stat
import struct
import tempfile
import unittest
from unittest import mock

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("package_race", HERE / "package_race.py")
package_race = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package_race)

REQUIRED = (
    "race.exe", "crash-sim.exe", "cars/car_basic.bsn", "tracks/oval.track",
    "tracks/chicane.track", "demo/oval_lap.input", "README.txt",
)


def make_pe(imports: list[str], delay_imports: list[str] = [], extra: bytes = b"") -> bytes:
    """One PE32+ section: ordinary descriptors, delay descriptors, names, extra."""
    body = bytearray((len(imports) + 1) * 20 + (len(delay_imports) + 1) * 32)
    delay_start = (len(imports) + 1) * 20
    for names, start, stride, field in (
            (imports, 0, 20, 12), (delay_imports, delay_start, 32, 4)):
        for i, name in enumerate(names):
            struct.pack_into("<I", body, start + i * stride + field, 0x1000 + len(body))
            body.extend(name.encode("ascii") + b"\0")
    body.extend(extra)
    header = bytearray(0x200)
    header[:2] = b"MZ"
    struct.pack_into("<I", header, 0x3C, 0x40)
    header[0x40:0x44] = b"PE\0\0"
    struct.pack_into("<HHIIIHH", header, 0x44, 0x8664, 1, 0, 0, 0, 240, 0)
    opt = 0x58
    struct.pack_into("<H", header, opt, 0x20B)
    struct.pack_into("<I", header, opt + 108, 16)
    struct.pack_into("<II", header, opt + 112 + 8, 0x1000, (len(imports) + 1) * 20)
    struct.pack_into("<II", header, opt + 112 + 13 * 8,
                     0x1000 + delay_start, (len(delay_imports) + 1) * 32)
    section = opt + 240
    header[section:section + 8] = b".rdata\0\0"
    struct.pack_into("<IIII", header, section + 8, len(body), 0x1000, len(body), 0x200)
    return bytes(header + body)


class PackageRaceTest(unittest.TestCase):
    def good_folder(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        folder = pathlib.Path(tmp.name) / "release"
        folder.mkdir()
        for rel in REQUIRED:
            path = folder / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"release\n")
        (folder / "race.exe").write_bytes(make_pe([
            "KERNEL32.dll", "user32.dll", "api-ms-win-crt-runtime-l1-1-0.dll"]))
        (folder / "crash-sim.exe").write_bytes(make_pe(["kernel32.dll", "VCRUNTIME140.dll"]))
        (folder / "vcruntime140.dll").write_bytes(make_pe(["kernel32.dll"]))
        return folder

    def run_check(self, folder):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = package_race.main(["check", "--folder", str(folder)])
        self.assertEqual(err.getvalue(), "")
        return code, out.getvalue().splitlines()

    def test_good_folder_exact_output(self):
        folder = self.good_folder()
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = package_race.main(["check", "--folder", str(folder)])
        file_names = sorted((*REQUIRED, "vcruntime140.dll"))
        expected = [f"file {rel} {os.stat(folder / rel).st_size}" for rel in file_names]
        expected.extend([
            "import crash-sim.exe kernel32.dll allowed",
            "import crash-sim.exe vcruntime140.dll beside",
            "import race.exe api-ms-win-crt-runtime-l1-1-0.dll allowed",
            "import race.exe kernel32.dll allowed",
            "import race.exe user32.dll allowed",
            "import vcruntime140.dll kernel32.dll allowed",
            "release-check: ok files=8 dlls=6",
        ])
        self.assertEqual(code, 0)
        self.assertEqual(out.getvalue(), "\n".join(expected) + "\n")
        # Additional executables and DLLs participate; nested files are scanned.
        (folder / "Z.EXE").write_bytes(make_pe(["NTDLL.DLL"]))
        note = b"file:///C:/source/main.rs\0"
        (folder / "demo/notes.txt").write_bytes(note)
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertIn("import Z.EXE ntdll.dll allowed", lines)
        self.assertIn(f"leak demo/notes.txt offset={note.index(b'C:/')} text=C:/source/main.rs", lines)

    def test_foreign_dll_without_copy_fails(self):
        folder = self.good_folder()
        (folder / "vcruntime140.dll").unlink()
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertIn("import crash-sim.exe vcruntime140.dll MISSING", lines)
        self.assertEqual(lines[-1], "release-check: FAIL problems=1")

    def test_foreign_dll_beside_passes(self):
        folder = self.good_folder()
        (folder / "crash-sim.exe").write_bytes(make_pe(["mylib.dll"]))
        (folder / "mylib.dll").write_bytes(make_pe(["kernel32.dll"]))
        code, lines = self.run_check(folder)
        self.assertEqual(code, 0)
        self.assertIn("import crash-sim.exe mylib.dll beside", lines)
        self.assertIn("import mylib.dll kernel32.dll allowed", lines)

    def assert_path_leak(self, path):
        folder = self.good_folder()
        image = make_pe(["kernel32.dll"], extra=b"\0panic at " + path + b"\0")
        (folder / "race.exe").write_bytes(image)
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertIn(f"leak race.exe offset={image.index(path)} text={path.decode('ascii')}", lines)
        self.assertEqual(lines[-1], "release-check: FAIL problems=1")

    def test_drive_path_forward_slash_fails(self):
        self.assert_path_leak(b"C:/Users/x/src/main.rs")
        # The first hit alone is used, and the next printable run is scanned.
        data = b"xC:/no\0 C:/yes D:/later\0D:/next\0"
        self.assertEqual(list(package_race.scan_leaks(data)),
                         [(data.index(b"C:/yes"), "C:/yes D:/later"),
                          (data.index(b"D:/next"), "D:/next")])
        huge = b"C:/" + b"x" * 1000000
        self.assertEqual(list(package_race.scan_leaks(huge)), [(0, huge[:120].decode())])

    def test_drive_path_backslash_fails(self):
        self.assert_path_leak(b"C:\\Users\\x\\src\\main.rs")

    def test_missing_file_fails(self):
        folder = self.good_folder()
        (folder / "tracks/chicane.track").unlink()
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertIn("missing tracks/chicane.track", lines)
        self.assertEqual(lines[-1], "release-check: FAIL problems=1")
        (folder / "README.txt").write_bytes(b"")
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertEqual(lines[-3:], ["missing tracks/chicane.track", "empty README.txt",
                                      "release-check: FAIL problems=2"])

    def test_pdb_path_is_not_a_leak(self):
        folder = self.good_folder()
        (folder / "race.exe").write_bytes(make_pe(
            ["kernel32.dll"], extra=b"\0C:\\build\\race.pdb\0D:/x/crash_sim.PDB\0"))
        code, lines = self.run_check(folder)
        self.assertEqual(code, 0)
        self.assertFalse(any(line.startswith("leak ") for line in lines))
        # Validate the exemption against the full string, before the output cut.
        self.assertEqual(list(package_race.scan_leaks(b"C:/" + b"x" * 1000000 + b".PDB")), [])

    def test_url_is_not_a_leak(self):
        folder = self.good_folder()
        (folder / "race.exe").write_bytes(make_pe(
            ["kernel32.dll"], extra=b"\0https://example.com/a\0"))
        code, lines = self.run_check(folder)
        self.assertEqual(code, 0)
        self.assertFalse(any(line.startswith("leak ") for line in lines))
        self.assertEqual(list(package_race.scan_leaks(b"AC:/no 9D:\\no\0")), [])

    def test_allow_list_case_and_prefix(self):
        folder = self.good_folder()
        names = ["GDI32.DLL", "api-ms-win-core-synch-l1-2-0.dll",
                 "Api-MS-Win-crt-math-l1-1-0.dll", "api-ms-wi.dll"]
        (folder / "race.exe").write_bytes(make_pe(names + ["gdi32.dll"], ["Gdi32.DLL"]))
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        for name in names[:3]:
            self.assertIn(f"import race.exe {name.lower()} allowed", lines)
        self.assertEqual(lines.count("import race.exe gdi32.dll allowed"), 1)
        self.assertIn("import race.exe api-ms-wi.dll MISSING", lines)
        self.assertEqual(lines[-1], "release-check: FAIL problems=1")
        self.assertEqual(package_race.ALLOWED_DLLS, frozenset({
            "kernel32.dll", "ntdll.dll", "user32.dll", "gdi32.dll", "ws2_32.dll",
            "advapi32.dll", "bcryptprimitives.dll", "ucrtbase.dll", "shell32.dll",
            "ole32.dll", "msvcrt.dll",
        }))

    def test_delay_import_is_checked(self):
        folder = self.good_folder()
        image = make_pe(["KERNEL32.dll"], ["foo.dll"])
        self.assertEqual(package_race.read_imports(image), ["KERNEL32.dll", "foo.dll"])
        (folder / "race.exe").write_bytes(image)
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertIn("import race.exe foo.dll MISSING", lines)
        self.assertEqual(lines[-1], "release-check: FAIL problems=1")

    def test_malformed_pe_is_named(self):
        def refused(data, name):
            with self.assertRaises(package_race.PeError) as caught:
                package_race.read_imports(data)
            self.assertEqual(caught.exception.name, name)

        refused(b"MZ", "Truncated")
        refused(b"XX" + bytes(200), "NotPe")
        pe32 = bytearray(make_pe([]))
        struct.pack_into("<H", pe32, 0x58, 0x10B)
        refused(bytes(pe32), "UnsupportedMagic")
        folder = self.good_folder()
        (folder / "race.exe").write_bytes(b"MZ")
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertIn("pe race.exe Truncated", lines)
        self.assertEqual(lines[-1], "release-check: FAIL problems=1")
        bad_rva = bytearray(make_pe(["x.dll"]))
        struct.pack_into("<I", bad_rva, 0x200 + 12, 0x90000000)
        refused(bytes(bad_rva), "RvaOutsideSections")
        for bad in ("", "bad name.dll", "x" * 256, "bad\x7fname"):
            with self.subTest(bad=bad):
                refused(make_pe([bad]), "BadName")
        for delay in (False, True):
            with self.subTest(delay=delay):
                names = ["x.dll"] * 4096
                image = make_pe([] if delay else names, names if delay else [])
                self.assertEqual(package_race.read_imports(image), names)
                names.append("x.dll")
                refused(make_pe([] if delay else names, names if delay else []),
                        "TooManyImports")
        no_dirs = bytearray(make_pe(["ignored.dll"]))
        struct.pack_into("<I", no_dirs, 0x58 + 108, 1)
        self.assertEqual(package_race.read_imports(bytes(no_dirs)), [])
        zero_rva = bytearray(make_pe(["ignored.dll"]))
        struct.pack_into("<I", zero_rva, 0x58 + 112 + 8, 0)
        self.assertEqual(package_race.read_imports(bytes(zero_rva)), [])
        rng = random.Random(7)
        allowed_errors = {"NotPe", "Truncated", "UnsupportedMagic", "RvaOutsideSections",
                          "BadName", "TooManyImports"}
        for i in range(2000):
            length = rng.randrange(601)
            data = bytes(rng.randrange(256) for _ in range(length))
            if i % 2:
                image = bytearray(make_pe(["kernel32.dll"], ["delay.dll"]))
                start = rng.randrange(len(image) + 1)
                end = min(start + length, len(image))
                image[start:end] = data[:end - start]
                data = bytes(image)
            with self.subTest(sample=i):
                try:
                    result = package_race.read_imports(data)
                except package_race.PeError as error:
                    self.assertIn(error.name, allowed_errors)
                else:
                    self.assertIsInstance(result, list)
                    self.assertTrue(all(isinstance(name, str) for name in result))

    def test_data_folder_is_refused(self):
        folder = self.good_folder()
        (folder / "data").mkdir()
        code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertEqual(lines[-2:], ["unexpected data", "release-check: FAIL problems=1"])
        # Inject metadata/errors, never create real links or junctions.
        real_opened = package_race._opened

        @contextlib.contextmanager
        def obstruct(path, expected, directory=False, parent_fd=None):
            name = os.path.basename(path)
            if name == "README.txt":
                raise package_race._LinkFound()
            if name == "car_basic.bsn":
                raise PermissionError("denied")
            with real_opened(path, expected, directory, parent_fd) as fd:
                yield fd

        with mock.patch.object(package_race, "_opened", obstruct):
            code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertEqual(lines[-5:], [
            "missing README.txt", "unexpected data",
            "link README.txt", "unreadable cars/car_basic.bsn",
            "release-check: FAIL problems=4",
        ])
        for info in (
                type("Info", (), {"st_mode": stat.S_IFLNK, "st_file_attributes": 0})(),
                type("Info", (), {"st_mode": stat.S_IFDIR, "st_file_attributes": 0x400})()):
            self.assertTrue(package_race._is_link(info))
        # An identity change at the lowest-level open fails before any read.
        path = folder / "README.txt"
        expected = os.stat(path)
        changed = mock.Mock(st_dev=expected.st_dev, st_ino=expected.st_ino + 1,
                            st_mode=expected.st_mode, st_file_attributes=0)
        stat_method = "lstat" if os.name == "nt" else "fstat"
        with mock.patch.object(package_race.os, stat_method, return_value=changed):
            with self.assertRaises(OSError):
                with package_race._opened(str(path), expected):
                    self.fail("replacement entry was accepted")
        replacement_link = mock.Mock(st_mode=stat.S_IFLNK, st_file_attributes=0x400)
        with mock.patch.object(package_race.os, stat_method, return_value=replacement_link):
            with self.assertRaises(package_race._LinkFound):
                with package_race._opened(str(path), expected):
                    self.fail("replacement link was accepted")

    def test_check_writes_nothing(self):
        folder = self.good_folder()

        def snapshot(root):
            entries = [root, *root.rglob("*")]
            return sorted((p.relative_to(root).as_posix(), p.stat().st_size,
                           p.stat().st_mtime_ns) for p in entries)

        before = snapshot(folder.parent)
        self.assertEqual(self.run_check(folder)[0], 0)
        self.assertEqual(snapshot(folder.parent), before)
        real_fdopen = package_race.os.fdopen
        pending_failure = True

        @contextlib.contextmanager
        def interrupted(fd, mode):
            nonlocal pending_failure
            with real_fdopen(fd, mode) as stream:
                if pending_failure:
                    pending_failure = False
                    stream.read(1)
                    broken = mock.Mock()
                    broken.read.side_effect = OSError("interrupted read")
                    yield broken
                else:
                    yield stream

        # A failure after part of the first file is read is reported, and
        # neither success nor failure changes the release or its parent.
        with mock.patch.object(package_race.os, "fdopen", interrupted):
            code, lines = self.run_check(folder)
        self.assertEqual(code, 1)
        self.assertIn("unreadable README.txt", lines)
        self.assertEqual(lines[-1], "release-check: FAIL problems=1")
        self.assertEqual(snapshot(folder.parent), before)
        (folder / "race.exe").write_bytes(b"MZ")
        before = snapshot(folder.parent)
        self.assertEqual(self.run_check(folder)[0], 1)
        self.assertEqual(snapshot(folder.parent), before)

    def test_usage_and_build_exit_2(self):
        folder = self.good_folder()
        missing_dir = str(folder / "absent")
        cases = [
            [], ["check"], ["check", "--folder"], ["check", "--folder", missing_dir],
            ["frob"], ["build", "--out", "x"], ["check", "--unknown", str(folder)],
            ["check", "--folder", str(folder), "--folder", str(folder)],
            ["check", "--folder", str(folder / "README.txt")],
            ["check", "--folder", ""], ["check", "--folder", "bad\0path"],
            ["check", "--folder", "bad\r\npath"],
        ]
        for args in cases:
            with self.subTest(args=args):
                out, err = io.StringIO(), io.StringIO()
                with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                    code = package_race.main(args)
                self.assertEqual(code, 2)
                self.assertEqual(out.getvalue(), "")
                if args and args[0] == "build":
                    self.assertEqual(err.getvalue(),
                                     "package_race: build is not implemented yet (n20-26b-2)\n")
                else:
                    self.assertEqual(len(err.getvalue().splitlines()), 1)
                    self.assertTrue(err.getvalue().startswith(("usage: ", "package_race: ")))


if __name__ == "__main__":
    unittest.main()
