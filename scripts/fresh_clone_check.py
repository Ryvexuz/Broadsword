"""Prove that a fresh clone builds and tests in one command (roadmap node 0.1).

  python -I scripts/fresh_clone_check.py [--repo DIR] [--root DIR] [--timeout SECONDS]

1. `git clone --no-hardlinks --branch main` of the local repo (default: this checkout) into
   scratch/fresh-clone-<utc>/ under the root (never anywhere else).
2. Reads the clone's documented one command: the first line in README.md, else AGENTS.md, that contains
   the words 'One command:' followed by the command in backticks (case-insensitive). It says which file it came from. A repo that documents no
   such command FAILS the check (exit 2): a command nobody wrote down is not a one-command build.
3. Runs it from the clone's root with the clone's OWN target folder (CARGO_TARGET_DIR=<clone>/target), so it never
   takes the shared build lock. Output is saved to scratch/fresh-clone-<utc>.txt and printed.
   BOOTSTRAP: when that command runs the installed `<install_dir>/broadsword.exe` and the clone has none (it is built, not
   committed), the check first does what docs/MOVING.md step 4 says, inside the clone, and prints each step: the one
   documented plain-cargo exception `cargo build --offline` (CARGO_TARGET_DIR=<clone>/<build_dir>), copy
   build/debug/broadsword.exe to scratch/bs-boot.exe, `scratch/bs-boot.exe build broadsword`, `build/debug/broadsword.exe install`;
   then provisions projects/crash-sim with `git clone` of a LOCAL repo (--crash-sim, default the root's or its main checkout's
   projects/crash-sim; a URL is refused: no network). The script refuses to bootstrap unless the clone's docs/MOVING.md contains
   those exact commands. A failed step is a FAIL naming the step; the one command is then not run.
4. Prints the commit tested and PASS (command exit 0) or FAIL. Deletes the clone, but only if its resolved path is
   a fresh-clone-* folder strictly inside the resolved scratch folder.

Exit 0 PASS, 1 FAIL (command failed or timed out), 2 refused or could not run (nothing was judged).

NEVER run this inside a Broadsword job (a nested run deadlocked builds for 15 minutes, PROGRESS.md). Broadsword
sets no job marker variable. What job.rs does set in every job child is CARGO_TARGET_DIR = the shared build folder
(build_dir in broadsword.toml, joined to the root). So the script refuses when CARGO_TARGET_DIR resolves to the
build_dir of the --root, --repo or this checkout, or of the main checkout
the git common dir of any of them points to (a job in a worktree gets the main root's folder, not the worktree's); a shell that sets CARGO_TARGET_DIR to some other folder is not refused.
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


def main_checkout_of(path):
    """The main checkout of the repo `path` belongs to (the parent of `git rev-parse --git-common-dir`), or None.
    A job started in a worktree gets the MAIN root's build folder, so a worktree must be mapped back to it."""
    try:
        r = subprocess.run(["git", "-C", str(path), "rev-parse", "--path-format=absolute", "--git-common-dir"],
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, timeout=60)
    except (OSError, subprocess.SubprocessError):
        return None
    out = r.stdout.decode("utf-8", "replace").strip()
    if r.returncode or not out:
        return None
    common = pathlib.Path(out)
    return common.parent if common.name == ".git" else None


def in_job(env, roots):
    """The build folder a job gave this process (CARGO_TARGET_DIR), if it is the shared build folder of any of `roots`."""
    target = env.get("CARGO_TARGET_DIR")
    if not target:
        return None
    for r in list(roots) + [m for m in map(main_checkout_of, roots) if m]:
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


EXE = ".exe"  # docs/MOVING.md names the Windows binaries; the bootstrap follows that document literally
MOVING_CARGO = "cargo build --offline"
MOVING_COPY_FROM, MOVING_COPY_TO = "build/debug/broadsword.exe", "scratch/bs-boot.exe"
MOVING_BOOT_BUILD = "scratch/bs-boot.exe build broadsword"
MOVING_INSTALL = "build/debug/broadsword.exe install"


def clone_config(clone):
    """The clone's broadsword.toml keys the bootstrap needs (required keys: no defaults). Refused if unreadable or missing."""
    try:
        cfg = tomllib.loads((pathlib.Path(clone) / "broadsword.toml").read_text(encoding="utf-8"))
        return {k: cfg[k] for k in ("build_dir", "install_dir", "projects_dir")}
    except (OSError, KeyError, ValueError, TypeError) as e:
        raise Refused(f"cannot bootstrap: the clone's broadsword.toml is unreadable or lacks build_dir/install_dir/projects_dir ({e!r})")


def needs_bootstrap(clone, command, cfg):
    """True when the documented command runs the installed broadsword binary of the clone and that file does not exist yet."""
    first = command.split()[0] if command.split() else ""
    if os.path.isabs(first):
        return False
    target = pathlib.Path(clone) / first
    installed = pathlib.Path(clone) / cfg["install_dir"] / ("broadsword" + EXE)
    return norm(target) == norm(installed) and not target.exists()


def bootstrap_steps(clone, cfg):
    """[(label, ('run', argv, env_overrides) or ('copy', src, dst))]: docs/MOVING.md step 4, checked against the clone's own copy
    of that file, which must contain every command quoted here (otherwise Refused: the doc and the script have drifted)."""
    clone = pathlib.Path(clone)
    try:
        doc = (clone / "docs" / "MOVING.md").read_text(encoding="utf-8")
    except OSError as e:
        raise Refused(f"cannot bootstrap: docs/MOVING.md is not readable in the clone ({e})")
    for lit in (MOVING_CARGO, MOVING_COPY_FROM, MOVING_COPY_TO, MOVING_BOOT_BUILD, MOVING_INSTALL):
        if f"`{lit}`" not in doc:
            raise Refused(f"cannot bootstrap: docs/MOVING.md does not contain `{lit}`; refusing to run a command the document does not give")
    build = clone / cfg["build_dir"]
    boot = clone / MOVING_COPY_TO
    built = build / "debug" / ("broadsword" + EXE)
    return [
        (f"{MOVING_CARGO}  (the one plain-cargo bootstrap exception in docs/MOVING.md)", ("run", MOVING_CARGO.split(), {"CARGO_TARGET_DIR": str(build)})),
        (f"copy {MOVING_COPY_FROM} to {MOVING_COPY_TO}", ("copy", built, boot)),
        (MOVING_BOOT_BUILD, ("run", [str(boot), "build", "broadsword"], {})),
        (MOVING_INSTALL, ("run", [str(built), "install"], {})),
    ]


def default_crash_sim(root):
    """The local crash-sim repo: <root>/<projects_dir>/crash-sim, else the main checkout's one (a worktree has none)."""
    roots = [pathlib.Path(root)] + [m for m in [main_checkout_of(root)] if m]
    for r in roots:
        try:
            pd = tomllib.loads((r / "broadsword.toml").read_text(encoding="utf-8"))["projects_dir"]
        except (OSError, KeyError, ValueError, TypeError):
            continue
        if (r / pd / "crash-sim" / ".git").exists():
            return r / pd / "crash-sim"
    return None


def provision_crash_sim(clone, cfg, source):
    """`git clone` of a LOCAL crash-sim repo into <clone>/<projects_dir>/crash-sim (no network: the source must be a folder)."""
    if source is None or not pathlib.Path(source).is_dir() or not (pathlib.Path(source) / ".git").exists():
        raise Refused(f"cannot provision projects/crash-sim: {source!r} is not a local Git repo folder (give --crash-sim DIR; a URL is never cloned)")
    dest = pathlib.Path(clone) / cfg["projects_dir"] / "crash-sim"
    dest.parent.mkdir(parents=True, exist_ok=True)
    c = run(["git", "clone", "--no-hardlinks", "--branch", "main", "--", str(pathlib.Path(source).resolve()), str(dest)])
    return c.returncode, c.stdout.decode("utf-8", "replace")


def do_bootstrap(clone, cfg, crash_sim, env, timeout, outfile):
    """Run the bootstrap, appending every step and its output to `outfile`. Returns None on success, else the failing step."""
    def note(text):
        with open(outfile, "ab") as f:
            f.write(text.encode("utf-8"))
    for label, step in bootstrap_steps(clone, cfg):
        note(f"== bootstrap step: {label}\n")
        if step[0] == "copy":
            try:
                shutil.copyfile(step[1], step[2])
            except OSError as e:
                note(f"copy failed: {e}\n")
                return label
            continue
        senv = dict(env)
        senv.update(step[2])
        rc, why = run_command(subprocess.list2cmdline(step[1]) if os.name == "nt" else shlex.join(step[1]), clone, senv, timeout, outfile, append=True)
        note(why)
        if rc != 0:
            note(f"step exit: {rc}\n")
            return label
    label = "provision projects/crash-sim: git clone of the local crash-sim repo"
    note(f"== bootstrap step: {label}\n")
    rc, out = provision_crash_sim(clone, cfg, crash_sim)
    note(out)
    if rc:
        note(f"step exit: {rc}\n")
        return label
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


def run_command(command, cwd, env, timeout, outfile, append=False):
    """Run the documented command with its output going straight to `outfile` (no pipe, so a grandchild that outlives the
    command can never block a read). Returns (exit code or 'timeout' or 'not started', error text)."""
    # Windows takes the command line as written (so `python scripts\x.py` survives); elsewhere split it like a shell
    argv = command if os.name == "nt" else shlex.split(command)
    kw = {"start_new_session": True} if os.name != "nt" else {}
    with open(outfile, "ab" if append else "wb") as f:
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
    ap.add_argument("--crash-sim", default=None, help="local crash-sim repo to clone into the fresh clone when it must be bootstrapped (default: <root>/projects/crash-sim, else the main checkout's)")
    ap.add_argument("--timeout", type=int, default=3600, help="seconds for each bootstrap step and for the one command (default 3600)")
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
        boot_failed = None
        boot_ran = False
        try:
            cfg = clone_config(clone) if (clone / "broadsword.toml").exists() else None
            if cfg and needs_bootstrap(clone, command, cfg):
                boot_ran = True
                crash_sim = pathlib.Path(args.crash_sim).resolve() if args.crash_sim else default_crash_sim(args.root)
                print(f"bootstrap: {command.split()[0]} is not committed, so it is built first as docs/MOVING.md step 4 says "
                      f"(crash-sim source: {crash_sim})")
                outfile.write_bytes(b"")
                boot_failed = do_bootstrap(clone, cfg, crash_sim, child_env, args.timeout, outfile)
        except Refused as e:
            print(f"FAIL (nothing judged): {e}")
            return 2
        if boot_failed:
            out = outfile.read_text(encoding="utf-8", errors="replace")
            print(out, end="" if out.endswith("\n") or not out else "\n")
            print(f"FAIL: bootstrap step failed: {boot_failed}; the one command was not run. Output saved to {outfile}")
            return 1
        child_env["CARGO_TARGET_DIR"] = str(clone / "target")
        rc, note = run_command(command, clone, child_env, args.timeout, outfile, append=boot_ran)
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
