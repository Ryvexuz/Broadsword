#!/usr/bin/env python3
"""Run every session-end gate with one command (read-only; it never fixes anything).

Gates, in order, one line each: `<name>: ok | FAIL | SKIPPED  <reason>`
  1. shadow_report      scripts/shadow_report.py --open      open shadow-only findings fail
  2. ledger_check       scripts/ledger_check.py              the improvement ledger (nb-07)
  3. main_provenance    scripts/main_provenance.py           commits on main that skipped the queue (nb-10)
  4. sync_playbook      scripts/sync_playbook.py --check     the playbook copy is current
  5. records_updated    PROGRESS.md and LAB_LOG.md changed (committed or uncommitted) since --since
  6. stale_tasks        no `running` task whose worktree had no commit and no job for longer than --stale-hours
A child script that is missing is SKIPPED (not installed). shadow_report.py without an `--open` flag counts as not installed
too. A child that crashes, times out or cannot be started is FAIL. Last line: `session-end: ok` or `session-end: N failing`.
Exit 0 when nothing fails (SKIPPED does not fail), 1 otherwise, 2 for bad arguments.

Needs from the caller: --since (session start, UTC: YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS][Z]), --timeout-seconds (per child),
--stale-hours, and --max-pending-days (ledger_check requires it). main_provenance's `--since` is the last commit on main
before --since. Children run with the Broadsword root as working directory. Paths come from broadsword.toml via
--root; the only files read are PROGRESS.md, LAB_LOG.md, state/tasks/*.task and state/jobs/*.rec, and git is used read-only.
"""
import argparse
import datetime as dt
import os
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
OK, FAIL, SKIPPED = "ok", "FAIL", "SKIPPED"


def parse_since(text):
    """UTC epoch seconds for a date or datetime string."""
    s = text.strip()
    if s.endswith(("Z", "z")):
        s = s[:-1]
    try:
        d = dt.datetime.fromisoformat(s)
    except ValueError:
        raise argparse.ArgumentTypeError(f"not a date or UTC datetime: {text!r}") from None
    return int(d.replace(tzinfo=dt.timezone.utc).timestamp())


def positive(kind):
    def conv(text):
        try:
            v = kind(text)
        except ValueError:
            raise argparse.ArgumentTypeError(f"not a number: {text!r}") from None
        if v <= 0:
            raise argparse.ArgumentTypeError("must be above 0")
        return v
    return conv


def tail(text, n=6):
    lines = [l.rstrip() for l in text.splitlines() if l.strip()]
    return lines[-n:]


def run_child(cmd, cwd, timeout):
    """(exit code | None on timeout/start failure, output text, note). Output goes to a temp file so a stuck grandchild
    holding a pipe cannot block us after the kill."""
    with tempfile.TemporaryFile() as out:
        try:
            p = subprocess.Popen(cmd, cwd=cwd, stdout=out, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL)
        except OSError as e:
            return None, "", f"could not start: {e}"
        try:
            code = p.wait(timeout=timeout)
            note = ""
        except subprocess.TimeoutExpired:
            if os.name == "nt":
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)], capture_output=True)
            p.kill()
            try:
                p.wait(timeout=10)
            except subprocess.TimeoutExpired:
                pass
            code, note = None, f"timed out after {timeout:g}s"
        out.seek(0)
        return code, out.read().decode("utf-8", "replace"), note


def gate_script(root, name, extra, timeout, need_flag=None):
    path = os.path.join(root, "scripts", name)
    if not os.path.isfile(path):
        return SKIPPED, "not installed (scripts/%s missing)" % name, []
    if need_flag:
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                if need_flag not in f.read():
                    return SKIPPED, f"not installed (scripts/{name} has no {need_flag})", []
        except OSError as e:
            return FAIL, f"cannot read scripts/{name}: {e}", []
    code, out, note = run_child([sys.executable, "-I", "-B", path, *extra], root, timeout)
    if code == 0:
        return OK, "", []
    if code is None:
        return FAIL, note, tail(out)
    return FAIL, f"exit {code}", tail(out)


def git(root, *args):
    env = dict(os.environ, GIT_OPTIONAL_LOCKS="0")
    p = subprocess.run(["git", "-C", root, *args], capture_output=True, text=True, encoding="utf-8",
                       errors="replace", env=env)
    return p.returncode, p.stdout.strip(), p.stderr.strip()


def gate_records(root, since):
    missing, stale = [], []
    for name in ("PROGRESS.md", "LAB_LOG.md"):
        if not os.path.isfile(os.path.join(root, name)):
            missing.append(name)
            continue
        code, status, err = git(root, "status", "--porcelain", "--", name)
        if code != 0:
            return FAIL, f"git status failed: {err[:200]}", []
        if status:  # uncommitted edit counts as changed
            continue
        code, ct, err = git(root, "log", "-1", "--format=%ct", "--", name)
        if code != 0:
            return FAIL, f"git log failed: {err[:200]}", []
        if not ct.isdigit() or int(ct) < since:
            stale.append(name)
    if missing:
        return FAIL, "missing: " + ", ".join(missing), []
    if stale:
        return FAIL, "not changed since the session start: " + ", ".join(stale), []
    return OK, "", []


def read_kv(path):
    rec = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                k, sep, v = line.rstrip("\n").partition(" = ")
                if sep:
                    rec[k.strip()] = v
                elif line.rstrip("\n").endswith(" =") :
                    rec[line.rstrip("\n")[:-2].strip()] = ""
    except OSError:
        return None
    return rec


def as_int(v):
    try:
        return int(v)
    except (TypeError, ValueError):
        return 0


def gate_stale(root, stale_hours, now):
    tdir = os.path.join(root, "state", "tasks")
    jdir = os.path.join(root, "state", "jobs")
    if not os.path.isdir(tdir):
        return SKIPPED, "no state/tasks folder", []
    last_job = {}
    if os.path.isdir(jdir):
        for n in os.listdir(jdir):
            if not n.endswith(".rec"):
                continue
            r = read_kv(os.path.join(jdir, n))
            if r and r.get("task"):
                t = max(as_int(r.get("started")), as_int(r.get("ended")))
                last_job[r["task"]] = max(last_job.get(r["task"], 0), t)
    stale, errors = [], []
    for n in sorted(os.listdir(tdir)):
        if not n.endswith(".task"):
            continue
        r = read_kv(os.path.join(tdir, n))
        if r is None:
            errors.append(f"{n}: unreadable")
            continue
        if r.get("status") != "running":
            continue
        tid = r.get("id") or n[:-5]
        last = max(as_int(r.get("started")), as_int(r.get("created")), last_job.get(tid, 0))
        wt = r.get("worktree", "")
        if wt and not r.get("worktree_removed"):
            wpath = os.path.join(root, wt)
            if os.path.isdir(wpath):
                code, head, _ = git(wpath, "rev-parse", "HEAD")
                if code == 0 and head != r.get("base"):  # a commit exists beyond the base
                    code, ct, _ = git(wpath, "log", "-1", "--format=%ct", "HEAD")
                    if code == 0:
                        last = max(last, as_int(ct))
        age_h = (now - last) / 3600.0
        if last == 0 or age_h > stale_hours:
            stale.append(f"{tid} ({r.get('title', '')[:60]}): no commit or job for {age_h:.1f}h")
    if errors:
        return FAIL, "; ".join(errors), []
    if stale:
        return FAIL, f"{len(stale)} running task(s) idle over {stale_hours:g}h", stale
    return OK, "", []


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--since", required=True, type=parse_since, help="session start, UTC (YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS]Z)")
    ap.add_argument("--timeout-seconds", required=True, type=positive(float), help="each child script gets at most this long")
    ap.add_argument("--stale-hours", required=True, type=positive(float), help="a running task idle longer than this fails")
    ap.add_argument("--max-pending-days", required=True, type=positive(int), help="passed to ledger_check.py")
    ap.add_argument("--root", default=os.path.dirname(HERE), help="the Broadsword root; default: this script's project")
    args = ap.parse_args(argv)
    root = os.path.abspath(args.root)
    since_date = dt.datetime.fromtimestamp(args.since, dt.timezone.utc).date().isoformat()
    t = args.timeout_seconds

    results = [("shadow_report", *gate_script(root, "shadow_report.py", ["--root", root, "--open"], t, "--open"))]
    results.append(("ledger_check", *gate_script(root, "ledger_check.py",
                    ["--since", since_date, "--max-pending-days", str(args.max_pending_days)], t)))
    code, base, err = git(root, "rev-list", "-1", f"--before={args.since}", "main")
    if code == 0 and base:
        results.append(("main_provenance", *gate_script(root, "main_provenance.py",
                        ["--since", base, "--root", root], t)))
    else:
        results.append(("main_provenance", FAIL, "no commit on main before --since" if code == 0 else f"git failed: {err[:200]}", []))
    results.append(("sync_playbook", *gate_script(root, "sync_playbook.py", ["--check"], t)))
    results.append(("records_updated", *gate_records(root, args.since)))
    results.append(("stale_tasks", *gate_stale(root, args.stale_hours, time.time())))

    for name, status, reason, detail in results:
        print(f"{name}: {status}" + (f"  {reason}" if reason else ""))
        for d in detail:
            print(f"    {d}")
    failing = sum(1 for r in results if r[1] == FAIL)
    print("session-end: ok" if not failing else f"session-end: {failing} failing")
    return 1 if failing else 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SystemExit:
        raise
    except Exception as e:  # never a traceback-only exit: report it as a failure
        print(f"session-end: 1 failing  (internal error: {e})")
        sys.exit(1)
