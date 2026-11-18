"""Throughput and idle time of development, from Broadsword's own records, for the retro's speed section.

  python -I scripts/dev_metrics.py                   every task
  python -I scripts/dev_metrics.py --since 20261118  tasks created on or after that UTC date (YYYYMMDD)

Read-only. Sources: state/tasks/*.task (created, started, ended), state/queue/*.entry (each step's UTC timestamps,
attempts, verdicts), state/jobs/*.rec (job start and end), and the author time of each task's merged commits (when
the coder committed). Not recorded anywhere yet, so not shown: build-lock wait inside a job, and coder token use.

Per task:
  lead       task created -> task ended (done, failed or abandoned)
  coding     task started -> last commit by the coder (author time of the newest merged commit)
  handoff    last coder commit -> first queue entry for the task: time the finished work waited on the orchestrator
  queue      first queue entry created -> last entry ended, with the check, review and main-check time inside it
Window (first task created -> last task ended):
  throughput  merged tasks per hour of the window
  busy        share of the window with at least one task running (agents had work)
  idle        share with no task running (nothing in progress: the pipeline was empty)
  jobs        share with at least one Broadsword job running (the machine was building or testing)
"""
import calendar
import pathlib
import subprocess
import sys
import time

def main_root():
    """The main checkout, where state/ and projects/ live, even when this script runs from a task worktree."""
    here = pathlib.Path(__file__).resolve().parent
    try:
        common = subprocess.run(["git", "-C", str(here), "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True, text=True, timeout=30).stdout.strip()
        if common:
            return pathlib.Path(common).parent
    except (OSError, subprocess.TimeoutExpired):
        pass
    return here.parent


ROOT = main_root()


def fields(path):
    out = {}
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        k, sep, v = line.partition(" = ")
        if sep:
            out[k.strip()] = v
    return out


def num(v):
    try:
        return int(v)
    except (TypeError, ValueError):
        return None


def utc(stamp):
    """'2026-11-18 06:47:43' (UTC) -> epoch seconds."""
    return calendar.timegm(time.strptime(stamp, "%Y-%m-%d %H:%M:%S"))


def steps(entry):
    """Seconds spent in each queue step, from the entry's timestamped log."""
    marks = []
    for line in entry.get("log", "").split("\\n"):
        parts = line.split(" ", 3)
        if len(parts) == 4 and parts[2] == "UTC" and parts[3].endswith(": begin"):
            marks.append((parts[3][: -len(": begin")], utc(parts[0] + " " + parts[1])))
    end = num(entry.get("ended"))
    out = {}
    for i, (name, t) in enumerate(marks):
        nxt = marks[i + 1][1] if i + 1 < len(marks) else end
        if nxt is not None:
            out[name] = out.get(name, 0) + max(0, nxt - t)
    return out


def commit_times(project, base, tip):
    """Author times of the commits in base..tip, i.e. when the coder committed (a rebase keeps author time)."""
    repo = ROOT if project == "broadsword" else ROOT / "projects" / project
    if not base or not tip:
        return []
    try:
        out = subprocess.run(["git", "-C", str(repo), "log", "--format=%at", f"{base}..{tip}"], capture_output=True, text=True, timeout=30).stdout.split()
    except (OSError, subprocess.TimeoutExpired):
        return []
    return sorted(int(t) for t in out if t.isdigit())


def dur(s):
    if s is None:
        return "-"
    s = int(s)
    return f"{s // 3600}h{s % 3600 // 60:02d}m" if s >= 3600 else f"{s // 60}m{s % 60:02d}s"


def covered(intervals, lo, hi):
    """Seconds of [lo, hi] covered by at least one interval."""
    spans = sorted((max(a, lo), min(b, hi)) for a, b in intervals if b > lo and a < hi)
    total, cur_a, cur_b = 0, None, None
    for a, b in spans:
        if cur_b is None or a > cur_b:
            if cur_b is not None:
                total += cur_b - cur_a
            cur_a, cur_b = a, b
        else:
            cur_b = max(cur_b, b)
    if cur_b is not None:
        total += cur_b - cur_a
    return total


def main():
    since = sys.argv[sys.argv.index("--since") + 1] if "--since" in sys.argv else None
    start_of = calendar.timegm(time.strptime(since, "%Y%m%d")) if since else 0
    now = int(time.time())
    tasks = [fields(p) for p in sorted((ROOT / "state" / "tasks").glob("*.task"))]
    tasks = [t for t in tasks if (num(t.get("created")) or 0) >= start_of]
    entries = [fields(p) for p in sorted((ROOT / "state" / "queue").glob("*.entry"))]
    by_task = {}
    for e in entries:
        by_task.setdefault(e.get("task"), []).append(e)
    if not tasks:
        print("no tasks in range")
        return 0

    print(f"{'task':<22} {'project':<10} {'status':<9} {'lead':>7} {'coding':>7} {'handoff':>7} {'queue':>7} {'check':>7} {'review':>7} {'main':>7} {'tries':>5}  title")
    merged = 0
    task_spans = []
    for t in tasks:
        created, started, ended = num(t.get("created")), num(t.get("started")), num(t.get("ended"))
        if started:
            task_spans.append((started, ended or now))
        es = sorted(by_task.get(t.get("id"), []), key=lambda e: num(e.get("created")) or 0)
        ok = [e for e in es if e.get("status") in ("merged", "merged_check_failed")]
        merged += 1 if ok else 0
        times = commit_times(t.get("project"), ok[-1].get("base_head"), ok[-1].get("main_sha")) if ok else []
        last = times[-1] if times else None
        q0 = num(es[0].get("created")) if es else None
        q1 = max((num(e.get("ended")) or 0 for e in es), default=0) or None
        st = {}
        for e in es:
            for k, v in steps(e).items():
                st[k] = st.get(k, 0) + v
        coding = last - started if last and started else None
        # each queue entry waited from the coder's latest commit before it until it was queued
        waits = []
        for e in es:
            c = num(e.get("created"))
            before = [x for x in times if c and x <= c]
            if before:
                waits.append(c - before[-1])
        handoff = sum(waits) if waits else None
        print(
            f"{t.get('id', '?'):<22} {t.get('project', '?')[:10]:<10} {t.get('status', '?'):<9} "
            f"{dur(ended - created if ended and created else None):>7} {dur(coding):>7} {dur(handoff):>7} "
            f"{dur(q1 - q0 if q0 and q1 else None):>7} {dur(st.get('check')):>7} {dur(st.get('review')):>7} {dur(st.get('main_check')):>7} "
            f"{len(es):>5}  {t.get('title', '')[:50]}"
        )

    lo = min(num(t.get("created")) or now for t in tasks)
    hi = max([num(t.get("ended")) or now for t in tasks] + [lo + 1])
    window = hi - lo
    jobs = []
    for p in (ROOT / "state" / "jobs").glob("*.rec"):
        r = fields(p)
        a, b = num(r.get("started")), num(r.get("ended"))
        if a and b:
            jobs.append((a, b))
        elif a and r.get("status") == "running":
            jobs.append((a, now))  # still running (or interrupted and not yet recovered)
    busy = covered(task_spans, lo, hi)
    job_time = covered(jobs, lo, hi)
    print()
    print(f"window      {dur(window)} ({time.strftime('%Y-%m-%d %H:%M', time.gmtime(lo))} to {time.strftime('%H:%M', time.gmtime(hi))} UTC)")
    print(f"throughput  {merged} merged task(s), {merged / (window / 3600):.2f} per hour of the window")
    print(f"busy        {busy / window:5.1%}  at least one task running")
    print(f"idle        {1 - busy / window:5.1%}  no task running (pipeline empty)")
    print(f"jobs        {job_time / window:5.1%}  at least one Broadsword job running")
    print("not recorded: build-lock wait inside a job; coder tokens (take them from agent reports)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
