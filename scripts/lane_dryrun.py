"""Check every lane's offline Cargo metadata in a retained scratch copy.

Python 3.11+; standard library only. No builds, checkpoints or board writes.
Real .rs files accompany Cargo manifests so Cargo's implicit target discovery
still works; no placeholder targets or edited manifests are used.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import datetime
import json
import math
import os
from pathlib import Path, PureWindowsPath
import shutil
import stat
import subprocess
import threading
import tomllib


class BadInput(Exception):
    """The proposed inputs cannot safely be checked."""


def one_line(value):
    """Keep untrusted diagnostics on one labelled output line."""
    return "".join(c if c.isprintable() else f"\\u{ord(c):04x}" for c in str(value))


def plain_path(value, label):
    if not isinstance(value, str) or not value or any(not c.isprintable() for c in value):
        raise BadInput(f"{label}: expected a non-empty path without control characters")
    return Path(value)


def relative_path(value, label):
    plain_path(value, label)
    # Recognize Windows paths even when this script runs on Unix.
    win = PureWindowsPath(value)
    parts = value.replace("\\", "/").split("/")
    if (win.drive or win.root or ":" in value or ".." in parts
            or not any(p not in ("", ".") for p in parts)):
        raise BadInput(f"{label}: expected a path strictly inside root: {value!r}")
    return Path(value)


def no_links(path):
    """Refuse symlinks and Windows junctions, including in parent components."""
    path = Path(os.path.abspath(path))
    for part in reversed((path, *path.parents)):
        info = part.lstat()
        if stat.S_ISLNK(info.st_mode) or getattr(info, "st_file_attributes", 0) & getattr(
            stat, "FILE_ATTRIBUTE_REPARSE_POINT", 0
        ):
            raise BadInput(f"links are not supported: {part}")
    return path


def existing(value, label, directory=False):
    path = plain_path(value, label)
    if not path.exists():
        raise BadInput(f"{label}: missing path {path}")
    path = no_links(path)
    if not (path.is_dir() if directory else path.is_file()):
        raise BadInput(f"{label}: expected {'folder' if directory else 'file'}: {path}")
    return path


def read_plain(path):
    """Read an ordinary file, checking the opened identity rather than just its name."""
    no_links(path)
    before = path.stat()
    if not stat.S_ISREG(before.st_mode):
        raise BadInput(f"expected an ordinary file: {path}")
    flags = os.O_RDONLY | getattr(os, "O_BINARY", 0) | getattr(os, "O_NOFOLLOW", 0)
    fd = os.open(path, flags)
    with os.fdopen(fd, "rb") as source:
        opened = os.fstat(source.fileno())
        if not stat.S_ISREG(opened.st_mode) or not os.path.samestat(before, opened):
            raise BadInput(f"file changed while opening: {path}")
        no_links(path)
        return source.read()


def config(path):
    try:
        return tomllib.loads(read_plain(path).decode("utf-8"))
    except (UnicodeError, ValueError, RecursionError) as exc:
        raise BadInput(f"invalid TOML {path}: {exc}") from exc


def positive_integer(cfg, key):
    value = cfg.get(key)
    if type(value) is not int or value <= 0:
        raise BadInput(f"broadsword.toml: {key} must be a positive integer")
    return value


def settings(root, candidate):
    cfg = config(existing(str(root / "broadsword.toml"), "root config"))
    proposed = config(candidate) if candidate else {}
    projects = relative_path(proposed.get("projects_dir", cfg.get("projects_dir")), "projects_dir")
    scratch = relative_path(cfg.get("scratch_dir"), "scratch_dir")
    build = relative_path(cfg.get("build_dir"), "build_dir")
    # Reuse the existing command deadline and captured-output setting: no hidden limits.
    timeout = positive_integer(cfg, "done_check_timeout_seconds")
    try:
        valid_timeout = math.isfinite(float(timeout)) and timeout <= threading.TIMEOUT_MAX
    except OverflowError:
        valid_timeout = False
    if not valid_timeout:
        raise BadInput("done_check_timeout_seconds exceeds the platform wait range")
    cap = positive_integer(cfg, "mcp_output_max_kb") * 1024
    if cap >= sys.maxsize:
        raise BadInput("mcp_output_max_kb exceeds the platform read range")
    for left, right in ((projects, scratch), (projects, build), (scratch, build)):
        if left.is_relative_to(right) or right.is_relative_to(left):
            raise BadInput(f"managed folders overlap: {left} and {right}")
    original_projects = relative_path(cfg.get("projects_dir"), "root projects_dir")
    excluded = {root / scratch, root / build, root / projects, root / original_projects}
    for key in ("state_dir", "worktrees_dir", "install_dir"):
        if key in cfg:
            excluded.add(root / relative_path(cfg[key], key))
    return projects, scratch, build, timeout, cap, excluded


def lane_name(name):
    if not name or any(c.isspace() or not c.isprintable() or c in "\\/:" for c in name):
        raise BadInput(f"invalid lane name: {name!r}")
    return name


def discover(root, projects, added):
    """Mirror job::list_projects: self plus manifest-bearing immediate children."""
    lanes = [("broadsword", root, Path("."))]
    directory = root / projects
    if directory.exists():
        no_links(directory)
        if not directory.is_dir():
            raise BadInput(f"projects_dir is not a folder: {directory}")
        for child in sorted(directory.iterdir()):
            if child.name != "broadsword" and (child / "Cargo.toml").is_file():
                no_links(child)
                lanes.append((lane_name(child.name), child, projects / child.name))
    if added:
        name = lane_name(added.name)
        if any(n.casefold() == name.casefold() for n, _, _ in lanes):
            raise BadInput(f"duplicate lane name: {name}")
        if not (added / "Cargo.toml").is_file():
            raise BadInput(f"new project has no Cargo.toml: {added}")
        lanes.append((name, added, projects / name))
    return lanes


def check_manifest_paths(data, manifest, destination, scratch):
    """Cargo must not resolve a copied path back into an original checkout."""
    def check(value):
        plain_path(value, f"{manifest}: Cargo path")
        win = PureWindowsPath(value)
        if win.drive or win.root or ":" in value:
            raise BadInput(f"{manifest}: absolute Cargo path {value!r}")
        path = Path(os.path.abspath(destination.parent / value))
        if not path.is_relative_to(scratch):
            raise BadInput(f"{manifest}: Cargo path escapes scratch: {value!r}")

    def dependency_paths(table):
        if not isinstance(table, dict):
            return
        for key in ("dependencies", "dev-dependencies", "build-dependencies"):
            dependencies = table.get(key, {})
            if isinstance(dependencies, dict):
                for dependency in dependencies.values():
                    if isinstance(dependency, dict) and "path" in dependency:
                        check(dependency["path"])

    dependency_paths(data)
    targets = data.get("target", {})
    if isinstance(targets, dict):
        for target in targets.values():
            dependency_paths(target)
    for key in ("patch", "replace"):
        tables = data.get(key, {})
        if isinstance(tables, dict):
            groups = tables.values() if key == "patch" else [tables]
            for group in groups:
                if isinstance(group, dict):
                    for dependency in group.values():
                        if isinstance(dependency, dict) and "path" in dependency:
                            check(dependency["path"])
    for key in ("lib", "bin", "bench", "example", "test"):
        entries = data.get(key, [])
        if isinstance(entries, dict):
            entries = [entries]
        if isinstance(entries, list):
            for entry in entries:
                if isinstance(entry, dict) and "path" in entry:
                    check(entry["path"])
    workspace = data.get("workspace", {})
    if not isinstance(workspace, dict):
        raise BadInput(f"{manifest}: workspace must be a table")
    dependency_paths(workspace)
    for key in ("members", "default-members", "exclude"):
        values = workspace.get(key, [])
        if not isinstance(values, list):
            raise BadInput(f"{manifest}: workspace.{key} must be a list")
        for value in values:
            check(value)
    for table in (data, workspace):
        package = table.get("package", {})
        if not isinstance(package, dict):
            raise BadInput(f"{manifest}: package must be a table")
        for key in ("build", "readme", "license-file", "workspace"):
            value = package.get(key)
            if isinstance(value, str):
                check(value)


def copy_lane(source, destination, scratch, excluded):
    """Allowlist actual source/manifests; never traverse managed output or links."""
    def walk(current, relative):
        no_links(current)
        for entry in sorted(current.iterdir()):
            if (entry.name.casefold() in ("target", ".git") or entry in excluded
                    or scratch.is_relative_to(entry)):
                continue
            no_links(entry)
            target = destination / relative / entry.name
            if entry.is_dir():
                walk(entry, relative / entry.name)
            elif entry.name in ("Cargo.toml", "Cargo.lock") or entry.suffix == ".rs":
                data = read_plain(entry)
                if entry.name == "Cargo.toml":
                    check_manifest_paths(tomllib.loads(data.decode("utf-8")), entry, target, scratch)
                target.parent.mkdir(parents=True, exist_ok=True)
                no_links(target.parent)
                with target.open("xb") as output:
                    output.write(data)

    destination.mkdir(parents=True, exist_ok=True)
    no_links(destination)
    walk(source, Path("."))


def json_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def json_constant(value):
    raise ValueError(f"non-finite JSON number: {value}")


def json_float(value):
    number = float(value)
    if not math.isfinite(number):
        raise ValueError(f"non-finite JSON number: {value}")
    return number


def metadata(directory, log_dir, timeout, cap, build):
    """The single Cargo entry point; files avoid descendant-held pipe deadlocks."""
    log_dir.mkdir()
    env = os.environ.copy()
    env["CARGO_TARGET_DIR"] = str(build)
    stdout_path, stderr_path = log_dir / "stdout", log_dir / "stderr"
    # Explicit PATH resolution also finds the test's cargo.cmd on Windows;
    # CreateProcess alone does not search PATHEXT.
    cargo = shutil.which("cargo")
    if cargo is None:
        return "FAIL", "cargo not found on PATH"
    timed_out = False
    with stdout_path.open("xb") as stdout, stderr_path.open("xb") as stderr:
        try:
            child = subprocess.Popen(
                [cargo, "metadata", "--no-deps", "--format-version", "1", "--offline"],
                cwd=directory, env=env, stdout=stdout, stderr=stderr,
            )
        except OSError as exc:
            return "FAIL", str(exc)
        try:
            code = child.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            child.kill()
            child.wait(timeout=timeout)
            timed_out = True
    with stdout_path.open("rb") as source:
        out = source.read(min(os.fstat(source.fileno()).st_size, cap + 1))
    with stderr_path.open("rb") as source:
        err = source.read(min(os.fstat(source.fileno()).st_size, cap + 1))
    first_error = err.decode("utf-8", "replace").splitlines()
    detail = first_error[0] if first_error else ""
    if timed_out:
        return "FAIL", f"{detail} (cargo metadata timed out)" if detail else "cargo metadata timed out"
    if code:
        return "FAIL", detail or f"cargo metadata exited {code} (no stderr)"
    if len(out) > cap or len(err) > cap:
        return "FAIL", detail or "cargo metadata output exceeds mcp_output_max_kb"
    try:
        result = json.loads(out, object_pairs_hook=json_object,
                            parse_constant=json_constant, parse_float=json_float)
    except (ValueError, UnicodeError, RecursionError) as exc:
        return "FAIL", detail or f"invalid metadata JSON: {exc}"
    if not isinstance(result, dict) or not isinstance(result.get("packages"), list):
        return "FAIL", detail or "metadata packages must be a list"
    if not result["packages"]:
        return "EMPTY", "packages=[]"
    return "OK", "non-empty packages"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", required=True)
    parser.add_argument("--add-project")
    parser.add_argument("--toml")
    parser.add_argument("--empty-ok", action="store_true",
                        help="do not block on an empty workspace")
    args = parser.parse_args(argv)
    try:
        # Validate ALL supplied paths before creating even the scratch parent.
        root = existing(args.root, "--root", directory=True)
        added = existing(args.add_project, "--add-project", directory=True) if args.add_project is not None else None
        candidate = existing(args.toml, "--toml") if args.toml is not None else None
        projects, scratch_rel, build_rel, timeout, cap, excluded = settings(root, candidate)
        lanes = discover(root, projects, added)
        parent = root / scratch_rel
        if parent.exists():
            no_links(parent)
        parent.mkdir(parents=True, exist_ok=True)
        no_links(parent)
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%d-%H%M%S")
        scratch = parent / f"lane-dryrun-{stamp}"
        # Exclusive creation: two invocations in the same second never overwrite.
        scratch.mkdir()
    except (BadInput, OSError, ValueError) as exc:
        print(f"lane-dryrun: bad input: {one_line(exc)}", file=sys.stderr)
        return 2

    print(f"lane-dryrun: scratch {one_line(scratch)}")
    copied = {}
    # Finish copying every lane before metadata: sibling path dependencies may cross lanes.
    for name, source, relative in lanes:
        try:
            copy_lane(source, scratch / relative, scratch, excluded)
        except (BadInput, OSError, ValueError, RecursionError) as exc:
            copied[name] = ("FAIL", str(exc))
    blocked = []
    for name, _, relative in lanes:
        try:
            status, detail = copied.get(name) or metadata(
                scratch / relative, scratch / f".metadata-{name}", timeout, cap, scratch / build_rel
            )
        except (OSError, ValueError, subprocess.SubprocessError) as exc:
            status, detail = "FAIL", str(exc)
        print(f"lane {one_line(name)} {status} {one_line(detail)}")
        # EMPTY may become OK by default once task t-20261122-043622 merges.
        if status == "FAIL" or (status == "EMPTY" and not args.empty_ok):
            blocked.append(name)
    if blocked:
        print("lane-dryrun: BLOCKED " + " ".join(map(one_line, blocked)))
        return 1
    print("lane-dryrun: ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
