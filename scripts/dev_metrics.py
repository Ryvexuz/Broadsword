"""Throughput and idle time of development, from Broadsword's own records, for the retro's speed section.

  python -I scripts/dev_metrics.py                   every task
  python -I scripts/dev_metrics.py --since 20261118  tasks created on or after that UTC date (YYYYMMDD)

Read-only. Sources: state/tasks/*.task (created, started, ended), state/queue/*.entry (each step's UTC timestamps,
attempts, verdicts), state/jobs/*.rec (job start and end), and the author time of each task's merged commits (when
the coder committed). Model use comes from every queue review (state/queue/*.entry:
model, review time, tokens, cost) and from state/agent-runs.tsv, where the orchestrator logs each subagent run (role,
model, task, tokens, seconds) from the agent's completion report. Not recorded anywhere yet, so not shown: build-lock
wait inside a job, and the orchestrator's own tokens.

Per task:
  lead       task created -> task ended (done, failed or abandoned)
  coding     task started -> last commit by the coder (author time of the newest merged commit)
  handoff    last coder commit -> first queue entry for the task: time the finished work waited on the orchestrator
  queue      first queue entry created -> last entry ended, with the check, review and main-check time inside it
Window (first task created -> last task ended):
  throughput  merged tasks per hour of the window
  busy        share of the window with at least one task running (agents had work). A task that exists but has
              not started, or was abandoned before starting, is waiting: that time counts as idle on purpose
  idle        share with no task running (nothing in progress: the pipeline was empty)
  jobs        share with at least one Broadsword job running (the machine was building or testing); a job left
              `running` by a crash counts until it is recovered, and such jobs are listed in the warnings
Models (same window, by date for agent runs):
  runs, active time, tokens and review cost per model and role; utilisation = active time / window
Cost to completion (per coder model, over the tasks in range): everything a task used until it was done, summed over
  every coder or fix run (state/agent-runs.tsv), every gating review and every shadow review of its queue entries.
  rounds = queue entries of the task; wall = task started -> ended. Median and worst over DONE tasks only. Tasks that are
  not done are counted as open (open or running) or not merged (abandoned) and never enter a median. An empty token field
  is unknown, never 0: a task whose sum has an unknown part is a lower bound (counted), a task with no known number
  is left out of that median. Printed as "coder $", "review $" and "total $": coder $ is summed from an optional cost_usd column of agent-runs.tsv (not recorded today, so
  it shows "-", unknown, and total $ is then not shown, never review $ alone or $0); Codex (shadow) tokens are plan
  allowance, shown as tokens, never as $0. Also per coder: first-pass PASS rate under the gating reviewer, BLOCKING lines
  per first review split by the adjudicated verdict in state/shadow-verdicts.tsv, and fix rounds (rounds - 1).
Reusable by other scripts: model_rows() (per model and role: runs, seconds, tokens, $; coder use = role "coder" seconds
  / window), read_runs(), task_costs().
"""
import calendar
import pathlib
import statistics
import subprocess
import sys
import time

def main_root():
    """The main checkout, where state/ and projects/ live, even when this script runs from a task worktree.
    Exits loudly if it can't be found: a report read from the wrong folder would be silently wrong."""
    here = pathlib.Path(__file__).resolve().parent
    try:
        common = subprocess.run(["git", "-C", str(here), "rev-parse", "--path-format=absolute", "--git-common-dir"], capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired) as e:
        raise SystemExit(f"dev_metrics: cannot run git to find the main checkout ({e})")
    root = pathlib.Path(common).parent if common else None
    if root is None or not (root / "broadsword.toml").is_file() or not (root / "state" / "tasks").is_dir():
        raise SystemExit(f"dev_metrics: no Broadsword main checkout with state/tasks found (git said {common!r})")
    return root


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


WARNINGS = []  # printed at the end: a missing number is shown as `-` and explained here, never silently


def commit_times(project, base, tip):
    """Author times of the commits in base..tip, i.e. when the coder committed (a rebase keeps author time)."""
    repo = ROOT if project == "broadsword" else ROOT / "projects" / project
    if not base or not tip:
        return []
    try:
        res = subprocess.run(["git", "-C", str(repo), "log", "--format=%at", f"{base}..{tip}"], capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as e:
        WARNINGS.append(f"git log failed in {repo.name} for {base[:10]}..{tip[:10]}: {e}")
        return []
    if res.returncode != 0:
        WARNINGS.append(f"git log failed in {repo.name} for {base[:10]}..{tip[:10]}: {res.stderr.strip()[:200]}")
        return []
    return sorted(int(t) for t in res.stdout.split() if t.isdigit())


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
        # The coder's commits: from the merged entry if there is one, else from the newest entry that recorded a tip
        # (a rejected or abandoned task still had coding time and handoff waits; that is the waste worth seeing).
        src = ok[-1] if ok else next((e for e in reversed(es) if e.get("base_head") and (e.get("checked_tip") or e.get("rebased_tip"))), None)
        tip = (src.get("main_sha") if ok else (src.get("checked_tip") or src.get("rebased_tip"))) if src else None
        times = commit_times(t.get("project"), src.get("base_head"), tip) if src else []
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
            jobs.append((a, now))  # still running, or interrupted and not yet recovered
            WARNINGS.append(f"job {p.stem} is recorded as running; it is counted up to now")
    busy = covered(task_spans, lo, hi)
    job_time = covered(jobs, lo, hi)
    print()
    print(f"window      {dur(window)} ({time.strftime('%Y-%m-%d %H:%M', time.gmtime(lo))} to {time.strftime('%H:%M', time.gmtime(hi))} UTC)")
    print(f"throughput  {merged} merged task(s), {merged / (window / 3600):.2f} per hour of the window")
    print(f"busy        {busy / window:5.1%}  at least one task running")
    print(f"idle        {1 - busy / window:5.1%}  no task running (pipeline empty)")
    print(f"jobs        {job_time / window:5.1%}  at least one Broadsword job running")
    models(entries, since, lo, hi, window)
    completion(tasks, entries, read_runs(None))
    print("not recorded: build-lock wait inside a job; the orchestrator's own tokens (see the app's usage card)")
    for w in WARNINGS:
        print(f"warning: {w}")
    return 0


def read_runs(since=None):
    """Rows of state/agent-runs.tsv as dicts (header names as keys), from `since` (YYYYMMDD) on. [] if there is no file.
    An empty field is an empty string: unknown, never 0 (use num())."""
    log = ROOT / "state" / "agent-runs.tsv"
    if not log.is_file():
        return []
    lines = log.read_text(encoding="utf-8").splitlines()
    head = lines[0].split("	") if lines else []
    out = []
    for line in lines[1:]:
        v = dict(zip(head, line.split("	")))
        if since and v.get("date_utc", "").replace("-", "") < since:
            continue
        out.append(v)
    return out


def model_rows(entries, since, lo, hi):
    """Per (model, role): [runs, active seconds, tokens, cost $]. Reusable: `active seconds / window` is the utilisation
    (coder use is the rows whose role is "coder"). Reviews come from the queue entries created in [lo, hi]; every
    subagent run in state/agent-runs.tsv from `since` on counts. An unknown token count or duration adds 0 here (the
    printed table has always done so); use read_runs() and num() to tell unknown from zero."""
    rows = {}

    def add(model, role, secs, tokens, cost):
        k = (model, role)
        r = rows.setdefault(k, [0, 0, 0, 0.0])
        r[0] += 1
        r[1] += secs or 0
        r[2] += tokens or 0
        r[3] += cost or 0.0

    for e in entries:
        c = num(e.get("created")) or 0
        if not e.get("reviewer_model") or not (lo <= c <= hi):
            continue
        if e.get("test_hook") == "true":
            continue
        secs = steps(e).get("review")
        tokens = (num(e.get("tokens_in")) or 0) + (num(e.get("tokens_out")) or 0)
        try:
            cost = float(e.get("cost_usd") or 0)
        except ValueError:
            cost = 0.0
        if e.get("verdict") or secs:  # a review that ran but never reached a verdict (timeout, stale) still used the model
            add(e.get("reviewer_model"), "reviewer", secs, tokens, cost)
    for v in read_runs(since):
        add(v.get("model", "?"), v.get("role", "?"), num(v.get("seconds")), num(v.get("tokens")), 0.0)
    return rows


def is_openai(model):
    return str(model or "").lower().startswith(("gpt", "o1", "o3", "o4", "codex"))


def blocking_lines(entry):
    """BLOCKING lines of a queue entry (the field holds them joined by a literal backslash-n)."""
    return [x for x in entry.get("blocking", "").split("\\n") if x.strip()]


def task_costs(tasks, entries_by_task, runs, verdicts):
    """One dict per task: coder model, state (done / open / not merged), rounds, wall seconds, tokens (Claude, OpenAI),
    review $, and the first review. Token fields are [known sum, unknown parts] pairs, never a silent 0."""
    runs_by_task = {}
    for v in runs:
        if v.get("role") in ("coder", "fix", "fixer"):
            runs_by_task.setdefault(v.get("task"), []).append(v)
    out = []
    for t in tasks:
        tid = t.get("id")
        es = sorted((e for e in entries_by_task.get(tid, []) if e.get("test_hook") != "true"), key=lambda e: num(e.get("created")) or 0)
        rs = runs_by_task.get(tid, [])
        coder_rs = [v for v in rs if v.get("role") == "coder"]
        model = (coder_rs[0].get("model") if coder_rs else None) or (es[0].get("coder") if es and es[0].get("coder") else "?")
        status = t.get("status")
        state = "done" if status == "done" else "open" if status in ("open", "running") else "not merged"
        started, ended, created = num(t.get("started")), num(t.get("ended")), num(t.get("created"))
        claude, openai = [0, 0], [0, 0]  # [known sum, unknown parts]

        def add(bucket, v):
            n = num(v)
            if n is None:
                bucket[1] += 1
            else:
                bucket[0] += n

        coder_usd = [0.0, 0]  # [known $, runs without a recorded $]: state/agent-runs.tsv has no price column today
        for v in rs:
            add(openai if is_openai(v.get("model")) else claude, v.get("tokens"))
            try:
                c = float(v.get("cost_usd"))  # optional column; absent or empty = unknown, never 0
            except (TypeError, ValueError):
                c = None
            if c is None or is_openai(v.get("model")):  # Codex runs are on the plan allowance: no $
                coder_usd[1] += 1
            else:
                coder_usd[0] += c
        usd, usd_unknown = 0.0, 0
        for e in es:
            if e.get("reviewer_model") and (e.get("verdict") or e.get("review_calls")):
                bucket = openai if is_openai(e.get("reviewer_model")) else claude
                rt = [num(e.get("tokens_in")), num(e.get("tokens_out"))]
                if None in rt:
                    bucket[1] += 1
                else:
                    bucket[0] += sum(rt)
                try:
                    usd += float(e["cost_usd"])
                except (KeyError, ValueError):
                    usd_unknown += 1  # counted, so the table can say the $ is a lower bound
            if e.get("shadow_model"):  # Codex on the plan allowance: tokens only, never $0
                add(openai, e.get("shadow_tokens"))
        first = next((e for e in es if e.get("verdict")), None)
        fid = str(first.get("id", "")).strip().lower() if first else None
        adj = [v for v in verdicts if first and str(v.get("entry", "")).strip().lower() == fid]  # case and spaces never hide an adjudication
        out.append({"id": tid, "model": model, "state": state, "rounds": len(es), "coder_runs": len(coder_rs),
                    "wall": (ended - (started or created)) if ended and (started or created) else None,
                    "claude": claude, "openai": openai, "usd": usd, "usd_unknown": usd_unknown, "coder_usd": coder_usd,
                    "first_reviewer": first.get("reviewer_model") if first else None,
                    "first_verdict": first.get("verdict") if first else None,
                    "first_lines": len(blocking_lines(first)) if first else 0,
                    "first_adjudicated": [v.get("verdict") for v in adj]})
    return out


def read_verdicts():
    f = ROOT / "state" / "shadow-verdicts.tsv"
    if not f.is_file():
        return []
    lines = f.read_text(encoding="utf-8").splitlines()
    head = lines[0].split("\t") if lines else []
    return [dict(zip(head, ln.split("\t"))) for ln in lines[1:]]


def stats(values):
    """(median, worst) of known numbers, (None, None) if none."""
    return (statistics.median(values), max(values)) if values else (None, None)


def completion(tasks, entries, runs):
    by_task = {}
    for e in entries:
        by_task.setdefault(e.get("task"), []).append(e)
    rows = task_costs(tasks, by_task, runs, read_verdicts())
    print()
    print("cost to completion per coder model (done tasks only; median / worst)")
    by_model = {}
    for r in rows:
        by_model.setdefault(r["model"], []).append(r)

    def fmt(v, f=lambda x: f"{x:.0f}"):
        return "-" if v is None else f(v)

    g = lambda x: f"{x:g}"
    for model, rs in sorted(by_model.items()):
        done = [r for r in rs if r["state"] == "done"]
        print(f"{model}: {len(done)} done, {sum(1 for r in rs if r['state'] == 'open')} open, "
              f"{sum(1 for r in rs if r['state'] == 'not merged')} not merged")
        if not done:
            continue
        m, w = stats([r["rounds"] for r in done if r["rounds"]])
        print(f"  rounds to merge     {fmt(m, g)} / {fmt(w)}  (done tasks with no queue entry, left out: {sum(1 for r in done if not r['rounds'])})")
        m, w = stats([r["wall"] for r in done if r["wall"] is not None])
        print(f"  wall time to merge  {fmt(m, dur)} / {fmt(w, dur)}")
        for key, label in (("claude", "Claude tokens"), ("openai", "OpenAI tokens (plan)")):
            known = [r[key][0] for r in done if r[key][0]]
            lower = sum(1 for r in done if r[key][1] and r[key][0])
            nothing = sum(1 for r in done if r[key][1] and not r[key][0])
            m, w = stats(known)
            print(f"  {label:<20}  {fmt(m)} / {fmt(w)}  (lower bounds, part unknown: {lower}; no number, left out: {nothing})")
        print(f"  coder runs recorded for {sum(1 for r in done if r['coder_runs'])} of {len(done)} done tasks (the rest: no coder tokens in the sums above)")
        # coder $ is known only for a task whose every coder run carries a price; otherwise it is unknown, never 0
        coder_known = [r for r in done if r["coder_runs"] and not r["coder_usd"][1]]
        m, w = stats([r["coder_usd"][0] for r in coder_known])
        print(f"  coder $             {fmt(m, lambda x: f'{x:.4f}')} / {fmt(w, lambda x: f'{x:.4f}')}  (known for {len(coder_known)} of {len(done)} done tasks; "
              f"unknown for {len(done) - len(coder_known)}: no price recorded in state/agent-runs.tsv, left out, never counted as $0)")
        m, w = stats([r["usd"] for r in done])
        unpriced = sum(r["usd_unknown"] for r in done)
        print(f"  review $            {fmt(m, lambda x: f'{x:.4f}')} / {fmt(w, lambda x: f'{x:.4f}')}  (gating reviews only; Codex shadow is plan allowance: tokens above, not $0)"
              + (f"; {unpriced} review(s) without a price in {sum(1 for r in done if r['usd_unknown'])} task(s): lower bounds" if unpriced else ""))
        totals = [r["coder_usd"][0] + r["usd"] for r in coder_known if not r["usd_unknown"]]
        m, w = stats(totals)
        print(f"  total $             {fmt(m, lambda x: f'{x:.4f}')} / {fmt(w, lambda x: f'{x:.4f}')}  (coder $ + review $, only for the {len(totals)} of {len(done)} done tasks where both are known; "
              + ("the others have an unknown coder $ or an unpriced review, so no total" if len(totals) < len(done) else "all known") + ")")
        # coders are compared under the same gating reviewer only: the first-pass numbers never mix reviewers
        firsts_all = [r for r in done if r["first_verdict"]]
        for reviewer in sorted({r["first_reviewer"] or "?" for r in firsts_all}):
            firsts = [r for r in firsts_all if (r["first_reviewer"] or "?") == reviewer]
            passed = sum(1 for r in firsts if r["first_verdict"].startswith("PASS"))
            print(f"  first-pass PASS under {reviewer}  {passed}/{len(firsts)}" + (f" = {passed / len(firsts):.0%}" if firsts else ""))
            lines = sum(r["first_lines"] for r in firsts)
            split = {}
            for r in firsts:
                for v in r["first_adjudicated"]:
                    split[v] = split.get(v, 0) + 1
            print(f"  BLOCKING, 1st review under {reviewer}  {lines} in {len(firsts)} reviews; adjudicated {dict(sorted(split.items())) or 'none'}, not adjudicated {max(lines - sum(split.values()), 0)}")
        m, w = stats([r["rounds"] - 1 for r in done if r["rounds"]])
        print(f"  fix rounds          {fmt(m, g)} / {fmt(w)}")


def models(entries, since, lo, hi, window):
    """Prints model_rows(): per model and role, runs, active seconds, tokens, cost, and active share of the window."""
    rows = model_rows(entries, since, lo, hi)
    print()
    print(f"{'model':<8} {'role':<12} {'runs':>4} {'active':>8} {'tokens':>10} {'cost $':>8} {'util':>6}")
    for (model, role), (n, secs, tokens, cost) in sorted(rows.items()):
        print(f"{model:<8} {role:<12} {n:>4} {dur(secs):>8} {tokens:>10} {cost:>8.4f} {secs / window:>6.1%}")
    if not (ROOT / "state" / "agent-runs.tsv").is_file():
        print("(no state/agent-runs.tsv: subagent runs are not logged yet)")


if __name__ == "__main__":
    sys.exit(main())
