"""Prove that a fresh clone builds and tests in one command (roadmap node 0.1).

  python -I scripts/fresh_clone_check.py [--repo DIR] [--root DIR] [--timeout SECONDS]

1. `git clone --no-hardlinks --branch main` of the local repo (default: this checkout) into
   scratch/fresh-clone-<utc>/ under the root (never anywhere else).
2. Reads the clone's documented one command: the first line in README.md, else AGENTS.md, that contains
   the words 'One command:' followed by the command in backticks (case-insensitive). It says which file it came from. A repo that documents no
   such command FAILS the check (exit 2): a command nobody wrote down is not a one-command build.
3. Runs it from the clone's root with the clone's OWN target folder (CARGO_TARGET_DIR=<clone>/target), so it never
   takes the shared build lock. Output is saved to scratch/fresh-clone-<utc>.txt and printed.
4. Prints the commit tested and PASS (command exit 0) or FAIL. Deletes the clone, but only if its resolved path is
   a fresh-clone-* folder strictly inside the resolved scratch folder.

Exit 0 PASS, 1 FAIL (command failed or timed out), 2 refused or could not run (nothing was judged).

NEVER run this inside a Broadsword job (a nested run deadlocked builds for 15 minutes, PROGRESS.md). Broadsword
sets no job marker variable. What job.rs does set in every job child is CARGO_TARGET_DIR = the shared build folder
(build_dir in broadsword.toml, joined to the root). So the script refuses when CARGO_TARGET_DIR resolves to the
build_dir of the --root, --repo or this checkout; a shell that sets CARGO_TARGET_DIR to some other folder is not refused.
The orchestrator runs it directly.
No network (the clone is local), no model calls.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import datetime
import os
import pathlib
import re
import shlex
import shutil
import stat
import subprocess
import tomllib

DOC_FILES = ("README.md", "AGENTS.md")
COMMAND_RE = re.compile(r"one[ -]command\s*:\s*`([^`]+)`", re.IGNORECASE)
CLONE_PREFIX = "fresh-clone-"


class Refused(Exception):
    """Nothing was run or judged."""


def norm(path):
    return os.path.normcase(str(pathlib.Path(path).resolve()))


def build_dir_of(root):
    """The shared build folder broadsword.toml names for `root` (required key: no default), or None if unreadable."""
    try:
        cfg = tomllib.loads((pathlib.Path(root) / "broadsword.toml").read_text(encoding="utf-8"))
        return pathlib.Path(root) / cfg["build_dir"]
    except (OSError, KeyError, ValueError, TypeError):
        return None


def in_job(env, roots):
    """The build folder a job gave this process (CARGO_TARGET_DIR), if it is the shared build folder of any of `roots`."""
    target = env.get("CARGO_TARGET_DIR")
    if not target:
        return None
    for r in roots:
        b = build_dir_of(r)
        if b is not None and norm(b) == norm(target):
            return b
    return None


def documented_command(clone):
    """(file name, command) from the first doc that has one, or None."""
    for name in DOC_FILES:
        p = pathlib.Path(clone) / name
        if p.is_file():
            m = COMMAND_RE.search(p.read_text(encoding="utf-8", errors="replace"))
            if m:
                return name, m.group(1).strip()
    return None


def safe_delete(path, scratch):
    """Delete `path` only if it is a fresh-clone-* folder strictly inside `scratch` once both are resolved."""
    p, s = pathlib.Path(path).resolve(), pathlib.Path(scratch).resolve()
    if p.parent != s or not p.name.startswith(CLONE_PREFIX) or not p.is_dir():
        raise Refused(f"not deleting {p}: it is not a {CLONE_PREFIX}* folder directly inside {s}")

    def onerror(func, target, _exc):  # Git marks object files read-only; Windows refuses to delete those
        os.chmod(target, stat.S_IWRITE)
        func(target)

    shutil.rmtree(p, onerror=onerror)


def run(argv, **kw):
    return subprocess.run(argv, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kw)


def kill_tree(proc):
    """Kill `proc` and everything it started (a timeout must not leave a grandchild holding the clone)."""
    if os.name == "nt":
        subprocess.run(["taskkill", "/T", "/F", "/PID", str(proc.pid)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    else:
        import signal
        try:
            os.killpg(proc.pid, signal.SIGKILL)
        except OSError:
            pass
    try:
        proc.kill()
    except OSError:
        pass


def run_command(command, cwd, env, timeout, outfile):
    """Run the documented command with its output going straight to `outfile` (no pipe, so a grandchild that outlives the
    command can never block a read). Returns (exit code or 'timeout' or 'not started', error text)."""
    # Windows takes the command line as written (so `python scripts\x.py` survives); elsewhere split it like a shell
    argv = command if os.name == "nt" else shlex.split(command)
    kw = {"start_new_session": True} if os.name != "nt" else {}
    with open(outfile, "wb") as f:
        try:
            proc = subprocess.Popen(argv, cwd=cwd, env=env, stdout=f, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL, **kw)
        except OSError as e:
            return "not started", f"could not start: {e}\n"
        try:
            rc = proc.wait(timeout=timeout)
            if os.name != "nt":  # its process group may still hold helpers; on Windows an exited pid has no tree (and may be reused)
                kill_tree(proc)
            return rc, ""
        except subprocess.TimeoutExpired:
            kill_tree(proc)
            try:
                proc.wait(timeout=30)
            except subprocess.TimeoutExpired:
                pass
            return "timeout", f"timed out after {timeout} s; the command and its child processes were killed\n"


def main(argv=None, env=None):
    env = dict(os.environ) if env is None else env
    here_root = pathlib.Path(__file__).resolve().parent.parent
    ap = argparse.ArgumentParser(prog="fresh_clone_check.py", description="Clone main fresh and run its one command.")
    ap.add_argument("--repo", default=str(here_root), help="repository to clone (default: this checkout)")
    ap.add_argument("--root", default=str(here_root), help="Broadsword root whose scratch/ gets the clone")
    ap.add_argument("--timeout", type=int, default=3600, help="seconds for the one command (default 3600)")
    args = ap.parse_args(argv)
    args.repo, args.root = str(pathlib.Path(args.repo).resolve()), str(pathlib.Path(args.root).resolve())  # the command runs elsewhere: no relative paths
    shared = in_job(env, [args.root, args.repo, here_root])
    if shared:
        print(f"fresh_clone_check: refusing to run: CARGO_TARGET_DIR is {shared}, the shared build folder a Broadsword job "
              "gives its child, so this looks like a job (nested runs deadlock the build lock). Run it directly, not through "
              "broadsword run/test, or unset CARGO_TARGET_DIR if this is a plain shell.")
        return 2
    scratch = pathlib.Path(args.root) / "scratch"
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    clone = scratch / f"{CLONE_PREFIX}{stamp}"
    outfile = scratch / f"{CLONE_PREFIX}{stamp}.txt"
    scratch.mkdir(parents=True, exist_ok=True)
    if clone.exists():
        print(f"fresh_clone_check: {clone} already exists; try again in a second")
        return 2
    code = 2
    try:
        c = run(["git", "clone", "--no-hardlinks", "--branch", "main", "--", str(args.repo), str(clone)])
        if c.returncode:
            print("fresh_clone_check: git clone failed:\n" + c.stdout.decode("utf-8", "replace"))
            return 2
        sha = run(["git", "-C", str(clone), "rev-parse", "HEAD"]).stdout.decode().strip()
        print(f"commit tested: {sha}\nclone: {clone}")
        found = documented_command(clone)
        if not found:
            print("FAIL: no documented one command: add a line 'One command:' plus the command in backticks to " + " or ".join(DOC_FILES))
            return 2
        doc, command = found
        print(f"one command (from {doc}): {command}")
        child_env = dict(env)
        child_env["CARGO_TARGET_DIR"] = str(clone / "target")
        rc, note = run_command(command, clone, child_env, args.timeout, outfile)
        out = outfile.read_text(encoding="utf-8", errors="replace") + note
        outfile.write_text(out, encoding="utf-8")
        print(out, end="" if out.endswith("\n") or not out else "\n")
        code = 0 if rc == 0 else 1
        print(f"exit: {rc}\n{'PASS' if code == 0 else 'FAIL'}: commit {sha[:12]}, output saved to {outfile}")
        return code
    finally:
        if clone.exists():
            try:
                safe_delete(clone, scratch)
            except (Refused, OSError) as e:  # never delete on doubt; a locked file must not hide the verdict already printed
                print(f"fresh_clone_check: clone NOT deleted: {e}")


if __name__ == "__main__":
    sys.exit(main())
