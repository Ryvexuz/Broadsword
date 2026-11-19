"""Record a named process baseline for a time window, and compare two baselines: BETTER, WORSE or NOISE.

  python -I scripts/baseline.py record <name> --since <ISO time> [--until <ISO time>]
  python -I scripts/baseline.py compare <before> <after> --min-samples <n> --noise <percent> [--metrics a,b]
  python -I scripts/baseline.py list

Project rule: every process change needs before/after evidence (throughput, quality, cost); a change the evidence does
not show to be better is switched back. `record` measures one window from Broadsword's own records and writes
state/baselines/<name>.json (outside Git). It never overwrites an existing baseline: a "before" is evidence.

Sources (read-only), all in the main checkout:
  state/tasks/*.task         created, started, ended, status, note
  state/queue/*.entry        each queue attempt: timestamped step log, verdict, cost, tokens, coder, reviewer
  state/queue/*.review       each review's header (parsed with scripts/review_costs.py)
  state/jobs/*.rec           job kind, start and end
  state/agent-runs.tsv       subagent runs the orchestrator logs (role, model, task, tokens, seconds)
  git                        main's commit, Revert commits on main inside the window
Helpers (fields, num, steps, covered, header) are imported by path from scripts/dev_metrics.py and scripts/review_costs.py.
dev_metrics has no function for coder use or coder tokens (its `util` is computed inside its printing function), so
those are baseline's own, with the same formula (seconds / window).

Window membership: a task counts if it merged (ended) inside [since, until); a queue entry, review or job counts if it
was created or started inside the window; a task's running time counts where it overlaps the window.

Exit codes: 0 done, 1 bad input (missing or unreadable source, bad time, unknown baseline), 2 usage.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import math
import os
import pathlib
import re
import statistics
import subprocess
import time
import uuid

HERE = pathlib.Path(__file__).resolve().parent
SCHEMA = 1
# Metrics added or renamed after the first schema-1 baselines were recorded. Old baselines (evidence, never rewritten)
# lack them; validation does not require them and compare shows "-" for them. Every other METRICS key is required.
LATER_METRICS = ("coder_use_upper_bound", "coder_tokens_unmerged")
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,80}$")

# name -> (group, direction, unit, description). direction: "up" = higher is better, "down" = lower is better,
# None = shown for context only, never given a verdict.
METRICS = {
    "merged_tasks": ("throughput", None, "tasks", "tasks merged in the window (queue or approved direct merge)"),
    "merged_per_hour": ("throughput", "up", "/h", "merged tasks per hour of the window"),
    "lead_median_s": ("throughput", "down", "s", "median task start -> merged"),
    "lead_p90_s": ("throughput", "down", "s", "p90 task start -> merged"),
    "idle_share": ("pipeline", "down", "share", "share of the window with no task running (dev_metrics 'idle')"),
    "coder_use_upper_bound": ("pipeline", None, "share", "UPPER BOUND of coder use: seconds of coder runs whose task overlaps the window, each run's seconds capped at its task's overlap with the window (runs have no start time), / window; over 100% = parallel; context only"),
    "review_rounds_per_merged": ("quality", "down", "rounds", "reviews per merged task, failed ones included"),
    "first_pass_fail_rate": ("quality", "down", "share", "merged tasks whose first review was FAIL"),
    "reverted_or_reopened": ("quality", "down", "count", "Revert commits on main + tasks merged more than once"),
    "main_check_failed": ("quality", "down", "count", "entries that merged but failed the check on main"),
    "direct_merges": ("quality", None, "count", "tasks merged outside the queue (approved direct merges)"),
    "review_usd_per_merged": ("cost", "down", "USD", "all review cost in the window (retries included) / merged"),
    "review_tokens_per_merged": ("cost", "down", "tokens", "all review tokens in the window / merged"),
    "coder_tokens_per_merged": ("cost", "down", "tokens", "coder tokens of all tasks finished in the window (done, failed or abandoned) / merged tasks"),
    "coder_tokens_unmerged": ("cost", None, "tokens", "of those coder tokens, how many belong to tasks that did not merge (waste kept in the cost)"),
    "coder_runs_recorded": ("cost", None, "runs", "coder runs of tasks that ended in the window"),
    "entry_median_s": ("queue", "down", "s", "median queue entry created -> ended (merge time per attempt)"),
    "check_median_s": ("queue", "down", "s", "median `check` step per entry"),
    "review_median_s": ("queue", "down", "s", "median `review` step per entry"),
    "recheck_median_s": ("queue", "down", "s", "median `main_check` step (re-check on main) per entry"),
    "test_job_median_s": ("queue", "down", "s", "median check/test job run time (state/jobs, kind check or test)"),
    "build_lock_wait_s": ("queue", "down", "s", "build-lock wait inside jobs: not recorded by Broadsword yet"),
}


class InputError(Exception):
    """Bad input: exit 1 with the message, which names the file or value."""


def load_script(name):
    """Import scripts/<name>.py by path (no copy of its logic). Its import-time root check exits loudly; that is an error."""
    path = HERE / f"{name}.py"
    if not path.is_file():
        raise InputError(f"{path}: not found (baseline.py imports it)")
    spec = importlib.util.spec_from_file_location(f"_baseline_{name}", path)
    mod = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(mod)
    except SystemExit as e:
        raise InputError(f"{path}: {e}") from None
    return mod


def main_root():
    """The main checkout (holds broadsword.toml and state/), even from a task worktree. Same rule as dev_metrics."""
    try:
        res = subprocess.run(["git", "-C", str(HERE), "rev-parse", "--path-format=absolute", "--git-common-dir"],
                             capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise InputError(f"cannot run git to find the main checkout ({e})") from None
    common = res.stdout.strip()
    root = pathlib.Path(common).parent if common else None
    if root is None or not (root / "broadsword.toml").is_file() or not (root / "state").is_dir():
        raise InputError(f"no Broadsword main checkout with broadsword.toml and state/ found (git said {common!r})")
    return root


def parse_time(text, flag):
    """ISO 8601 with a zone (2026-11-18T09:00:00Z) -> epoch seconds. A time without a zone is refused, not guessed."""
    try:
        t = dt.datetime.fromisoformat(text.strip())
    except ValueError:
        raise InputError(f"{flag} {text!r}: not an ISO 8601 time such as 2026-11-18T09:00:00Z") from None
    if t.tzinfo is None:
        raise InputError(f"{flag} {text!r}: give a time zone (Z or +01:00); a bare time would be read in the wrong zone")
    return int(t.timestamp())


def iso(epoch):
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(epoch))


def read_fields(dm, path):
    try:
        return dm.fields(path)
    except (OSError, UnicodeError) as e:
        raise InputError(f"{path}: unreadable ({e})") from None


def need_dir(path):
    if not path.is_dir():
        raise InputError(f"{path}: missing (a record source for the baseline)")
    return path


def git(root, *args, timeout=60):
    try:
        res = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise InputError(f"git {' '.join(args)} in {root}: {e}") from None
    if res.returncode != 0:
        raise InputError(f"git {' '.join(args)} in {root}: {res.stderr.strip()[:300]}")
    return res.stdout


def branch_exists(root, branch):
    """True if refs/heads/<branch> exists, False if it does not. Any other git failure, including a broken ref that git
    skips with a warning, is an input error (never a silent fallback)."""
    try:
        res = subprocess.run(["git", "-C", str(root), "for-each-ref", "--format=%(refname)", f"refs/heads/{branch}"],
                             capture_output=True, text=True, timeout=60)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise InputError(f"git for-each-ref in {root}: {e}") from None
    if res.returncode != 0:
        raise InputError(f"git for-each-ref refs/heads/{branch} in {root}: exit {res.returncode}: {res.stderr.strip()[:300]}")
    if res.stdout.strip() == f"refs/heads/{branch}":
        return True
    if res.stderr.strip():
        raise InputError(f"git for-each-ref refs/heads/{branch} in {root}: {res.stderr.strip()[:300]}")
    return False


def p90(values):
    s = sorted(values)
    return s[max(0, math.ceil(0.9 * len(s)) - 1)]


def med(values):
    return statistics.median(values) if values else None


def ratio(a, b):
    return a / b if b else None


def strict_num(text, where):
    """A whole number from a record cell. Empty or absent is None (unknown); present but malformed is an input error naming `where`."""
    if text is None or not str(text).strip():
        return None
    try:
        return int(str(text).strip())
    except ValueError:
        raise InputError(f"{where}: {str(text).strip()[:40]!r} is not a whole number") from None


def finite(value, where):
    """The one number gate: a float that is finite, else an input error naming `where`. float() alone accepts 'nan' and 'inf'."""
    try:
        v = float(value)
    except (TypeError, ValueError):
        raise InputError(f"{where}: {str(value).strip()[:40]!r} is not a number") from None
    if not math.isfinite(v):
        raise InputError(f"{where}: {str(value).strip()[:40]!r} is not a finite number")
    return v


def check_finite(values, per_model):
    """Final gate before a baseline is written: no metric or per-model figure may be NaN or infinite."""
    for k, v in values.items():
        if v is not None:
            finite(v, f"metric {k}")
    for name, m in per_model.items():
        for f in ("runs", "usd", "tokens", "seconds"):
            finite(m[f], f"per_model {name} {f}")


def measure(root, since, until):
    """Every metric for [since, until), plus samples, per-model rows and notes. Raises InputError on a bad source."""
    dm = load_script("dev_metrics")
    rc = load_script("review_costs")
    if pathlib.Path(dm.ROOT).resolve() != root.resolve():
        raise InputError(f"dev_metrics found the root {dm.ROOT}, baseline found {root}: refusing to mix two checkouts")
    notes = []
    state = root / "state"
    tasks = [read_fields(dm, p) for p in sorted(need_dir(state / "tasks").glob("*.task"))]
    qdir = need_dir(state / "queue")
    entries = [read_fields(dm, p) for p in sorted(qdir.glob("*.entry"))]
    entries = [e for e in entries if e.get("test_hook") != "true"]
    jobs_dir = need_dir(state / "jobs")
    runs_file = state / "agent-runs.tsv"
    if not runs_file.is_file():
        raise InputError(f"{runs_file}: missing (coder runs, tokens and seconds come from it)")

    def inside(t):
        return t is not None and since <= t < until

    # Reviews: every .review file whose entry is not a test hook, keyed by task.
    reviews = []
    for p in sorted(qdir.glob("*.review")):
        try:
            h = rc.header(p.read_text(encoding="utf-8", errors="replace"))
        except OSError as e:
            raise InputError(f"{p}: unreadable ({e})") from None
        if h.get("test_hook") == "true":
            continue
        h["_file"] = p.name
        reviews.append(h)
    for e in entries:
        rf = e.get("review_file")
        if rf and not (root / rf).is_file():
            raise InputError(f"{root / rf}: missing, but entry {e.get('id')} names it as its review file")
    entry_by_id = {e.get("id"): e for e in entries}

    def review_time(h):
        e = entry_by_id.get(h.get("entry"))
        where = f"{qdir / h['_file']}: attempt"
        return (dm.num(e.get("created")) if e else None), strict_num(h.get("attempt"), where) or 1

    by_task = {}
    for e in entries:
        by_task.setdefault(e.get("task"), []).append(e)

    merged = []
    direct = 0
    reopened = 0
    for t in tasks:
        if t.get("status") != "done" or not inside(dm.num(t.get("ended"))):
            continue
        ok = [e for e in by_task.get(t.get("id"), []) if e.get("status") in ("merged", "merged_check_failed")]
        if ok:
            merged.append(t)
            reopened += len(ok) > 1
        elif t.get("note", "").startswith("merged"):
            merged.append(t)
            direct += 1
    n = len(merged)
    window = until - since
    leads = [dm.num(t["ended"]) - dm.num(t["started"]) for t in merged if dm.num(t.get("started"))]
    if len(leads) < n:
        notes.append(f"{n - len(leads)} merged task(s) have no start time and are left out of lead times")

    spans = []
    for t in tasks:
        s = dm.num(t.get("started"))
        if s:
            spans.append((s, dm.num(t.get("ended")) or until))
    busy = dm.covered(spans, since, until)

    # Quality: reviews of the merged tasks. Every review of a merged task counts as a round. The first pass is the review
    # whose queue entry was created first (attempt number breaks ties); a review with no entry or no created time cannot be
    # placed in time, so it is left out of the first-pass and cost figures and counted in a note.
    merged_ids = {t["id"] for t in merged}
    rounds = 0
    first_fail = 0
    reviewed = 0
    no_time = 0
    for tid in merged_ids:
        all_rs = [r for r in reviews if r.get("task") == tid]
        rounds += len(all_rs)
        rs = sorted((r for r in all_rs if review_time(r)[0] is not None), key=lambda r: (review_time(r)[0], review_time(r)[1]))
        no_time += len(all_rs) - len(rs)
        if rs:
            reviewed += 1
            first_fail += rs[0].get("verdict", "").startswith("FAIL")
    main_branch = "main"
    if branch_exists(root, main_branch):
        main_sha = git(root, "rev-parse", f"refs/heads/{main_branch}").strip()
    else:
        main_sha = git(root, "rev-parse", "HEAD").strip()
        notes.append("no branch `main`; recorded HEAD instead")
        main_branch = "HEAD"
    subjects = git(root, "log", main_branch, "--first-parent", f"--since=@{since}", f"--until=@{until}", "--format=%s").splitlines()
    reverts = sum(1 for s in subjects if s.startswith("Revert"))

    # Cost: all reviews whose entry was created in the window (retries and rejected attempts included).
    win_reviews = [r for r in reviews if inside(review_time(r)[0])]
    if no_time:
        notes.append(f"{no_time} review(s) of merged tasks have no queue entry or no created time: counted in "
                     "review_rounds_per_merged, left out of first_pass_fail_rate and review cost and tokens")
    usd = 0.0
    rtok = 0
    no_cost = no_tokens = 0
    per_model = {}
    for r in win_reviews:
        # A review with no cost or token field is not counted as zero: it is counted apart and reported.
        c = None
        if r.get("cost_usd", "").strip():
            c = finite(r["cost_usd"], f"{qdir / r['_file']}: cost_usd")
        else:
            no_cost += 1
        tin = strict_num(r.get("input tokens"), f"{qdir / r['_file']}: input tokens")
        tout = strict_num(r.get("output tokens"), f"{qdir / r['_file']}: output tokens")
        if tin is None or tout is None:
            no_tokens += 1
        tok = (tin or 0) + (tout or 0)
        usd += c or 0.0
        rtok += tok
        m = per_model.setdefault(f"reviewer:{r.get('model', '?')}", {"runs": 0, "usd": 0.0, "tokens": 0, "seconds": 0})
        m["runs"] += 1
        m["usd"] += c or 0.0
        m["tokens"] += tok
    if no_cost:
        notes.append(f"{no_cost} of {len(win_reviews)} reviews have no cost recorded: review USD is a lower bound")
    if no_tokens:
        notes.append(f"{no_tokens} of {len(win_reviews)} reviews have no token counts recorded: review tokens are a lower bound")

    # Coder runs. Two rules, one per use:
    #  - coder_use_upper_bound and per_model (activity in the window): a row counts if its task's running time overlaps
    #    the window, or, with no known task, by its date. agent-runs.tsv has no start time, so a run's real overlap with
    #    the window cannot be known; its seconds are capped at its task's overlap with the window (or at the window
    #    length when it has no known task). That is an upper bound, not a measurement. dev_metrics has no function for
    #    this (its `util` is computed inside the printing function `models()`).
    #  - coder_tokens_per_merged (cost): only rows of tasks that ENDED inside the window (finished work, merged, failed
    #    or abandoned; every run counts, retries included, as rejected review attempts are counted too). The tokens of
    #    tasks that did not merge are also reported apart (coder_tokens_unmerged). Tasks still running at `until` and rows with no known
    #    task are left out and counted in the notes, so unfinished work never inflates the cost.
    task_span = {t.get("id"): (dm.num(t.get("started")), dm.num(t.get("ended")) or until) for t in tasks}
    task_ended = {t.get("id"): dm.num(t.get("ended")) for t in tasks}
    first_day, last_day = time.strftime("%Y-%m-%d", time.gmtime(since)), time.strftime("%Y-%m-%d", time.gmtime(until - 1))
    try:
        lines = runs_file.read_text(encoding="utf-8").splitlines()
    except (OSError, UnicodeError) as e:
        raise InputError(f"{runs_file}: unreadable ({e})") from None
    head = lines[0].split("\t") if lines else []
    for col in ("date_utc", "role", "model", "task", "tokens", "seconds"):
        if col not in head:
            raise InputError(f"{runs_file}: header has no `{col}` column (found {head})")
    coder_secs = coder_tok = coder_runs = unfinished_runs = coder_tok_unmerged = 0
    no_tok_runs = no_sec_runs = no_span_runs = 0
    for lineno, line in enumerate(lines[1:], 2):
        if not line.strip():
            continue
        v = dict(zip(head, line.split("\t")))
        span = task_span.get(v.get("task"))
        known = bool(span and span[0])
        if known:
            hit = span[0] < until and span[1] > since
            overlap = min(span[1], until) - max(span[0], since)
        else:
            hit = first_day <= v.get("date_utc", "") <= last_day
            overlap = window
        finished = inside(task_ended.get(v.get("task")))
        # Cost to completion counts every run of a task that ended in the window, whatever the run's date; activity
        # (coder use, per model) counts only runs that are in the window by task span or, with no span, by date.
        if not hit and not finished:
            continue
        secs = strict_num(v.get("seconds"), f"{runs_file} line {lineno}: seconds")
        tok = strict_num(v.get("tokens"), f"{runs_file} line {lineno}: tokens")
        if v.get("role") == "coder":
            no_tok_runs += tok is None
            no_sec_runs += secs is None
        secs, tok = secs or 0, tok or 0
        secs = min(secs, overlap)
        coder = v.get("role") == "coder"
        if hit:
            if coder and not finished:
                unfinished_runs += 1
            if coder and not known:
                no_span_runs += 1
            m = per_model.setdefault(f"{v.get('role', '?')}:{v.get('model', '?')}", {"runs": 0, "usd": 0.0, "tokens": 0, "seconds": 0})
            m["runs"] += 1
            m["tokens"] += tok
            m["seconds"] += secs
            if coder:
                coder_secs += secs
        if coder and finished:
            coder_runs += 1
            coder_tok += tok
            if v.get("task") not in merged_ids:
                coder_tok_unmerged += tok
    if unfinished_runs:
        notes.append(f"{unfinished_runs} coder run(s) in the window belong to tasks not finished in it (or of no known task): "
                     "counted in coder_use_upper_bound, left out of coder tokens")
    if no_span_runs:
        notes.append(f"{no_span_runs} runs have no task span: counted in full, capped at the window (coder_use_upper_bound)")
    if no_tok_runs:
        notes.append(f"{no_tok_runs} coder runs have no token count: coder tokens are a lower bound")
    if no_sec_runs:
        notes.append(f"{no_sec_runs} coder runs have no seconds: coder use is a lower bound")
    for r in win_reviews:
        secs = dm.steps(entry_by_id[r["entry"]]).get("review") if r.get("entry") in entry_by_id else None
        per_model[f"reviewer:{r.get('model', '?')}"]["seconds"] += secs or 0

    # Queue steps per entry created in the window.
    win_entries = [e for e in entries if inside(dm.num(e.get("created")))]
    step = {"check": [], "review": [], "main_check": []}
    durs = []
    for e in win_entries:
        st = dm.steps(e)
        for k in step:
            if k in st:
                step[k].append(st[k])
        a, b = dm.num(e.get("created")), dm.num(e.get("ended"))
        if a and b:
            durs.append(b - a)
    test_jobs = []
    for p in sorted(jobs_dir.glob("*.rec")):
        r = read_fields(dm, p)
        a, b = dm.num(r.get("started")), dm.num(r.get("ended"))
        if r.get("kind") in ("check", "test") and inside(a) and b:
            test_jobs.append(b - a)
    notes.append("build-lock wait inside a job is not recorded by Broadsword yet (dev_metrics says the same)")

    values = {
        "merged_tasks": n,
        "merged_per_hour": n / (window / 3600),
        "lead_median_s": med(leads),
        "lead_p90_s": p90(leads) if leads else None,
        "idle_share": 1 - busy / window,
        "coder_use_upper_bound": coder_secs / window,
        "review_rounds_per_merged": ratio(rounds, n),
        "first_pass_fail_rate": ratio(first_fail, reviewed),
        "reverted_or_reopened": reverts + reopened,
        "main_check_failed": sum(1 for e in win_entries if e.get("status") == "merged_check_failed"),
        "direct_merges": direct,
        "review_usd_per_merged": ratio(usd, n),
        "review_tokens_per_merged": ratio(rtok, n),
        "coder_tokens_per_merged": ratio(coder_tok, n) if coder_runs else None,
        "coder_tokens_unmerged": coder_tok_unmerged if coder_runs else None,
        "coder_runs_recorded": coder_runs,
        "entry_median_s": med(durs),
        "check_median_s": med(step["check"]),
        "review_median_s": med(step["review"]),
        "recheck_median_s": med(step["main_check"]),
        "test_job_median_s": med(test_jobs),
        "build_lock_wait_s": None,
    }
    if not coder_runs:
        notes.append("no coder runs of tasks that ended in this window in state/agent-runs.tsv: coder tokens not known")
    samples = {"merged_tasks": n, "queue_entries": len(win_entries), "reviews": len(win_reviews),
               "test_jobs": len(test_jobs), "coder_runs": coder_runs,
               "reviews_without_cost": no_cost, "reviews_without_tokens": no_tokens}
    return values, samples, per_model, main_sha, notes


def fmt(v, unit=""):
    if v is None:
        return "-"
    if unit == "share":
        return f"{v:.1%}"
    if unit == "s":
        return f"{v:.0f}s"
    if unit == "USD":
        return f"{v:.4f}"
    if isinstance(v, float):
        return f"{v:.2f}"
    return str(v)


def baselines_dir(root):
    return root / "state" / "baselines"


def validate_baseline(data, path):
    """Every key compare and list read (every metric except LATER_METRICS, the sample count, the window, the commit and toml hash, per_model) must be there with the right type, else bad input naming the file."""
    def bad(why):
        raise InputError(f"{path}: not a valid schema-{SCHEMA} baseline ({why})")
    if not isinstance(data, dict) or data.get("schema") != SCHEMA:
        bad("schema")
    for key, typ in (("name", str), ("main_commit", str), ("broadsword_toml_sha256", str), ("window", dict),
                     ("metrics", dict), ("samples", dict), ("per_model", dict), ("notes", list)):
        if not isinstance(data.get(key), typ):
            bad(f"`{key}` missing or not a {typ.__name__}")
    for key in ("since", "until"):
        if not isinstance(data["window"].get(key), str):
            bad(f"`window.{key}` missing")
    mt = data["samples"].get("merged_tasks")
    if isinstance(mt, bool) or not isinstance(mt, int):
        bad("`samples.merged_tasks` missing or not a whole number")
    missing = [m for m in METRICS if m not in LATER_METRICS and m not in data["metrics"]]
    if missing:
        bad(f"`metrics` lacks {', '.join(missing)}")
    for k, v in data["metrics"].items():
        if v is not None and (isinstance(v, bool) or not isinstance(v, (int, float))):
            bad(f"metric `{k}` is not a number or null")
        if isinstance(v, float) and not math.isfinite(v):
            bad(f"metric `{k}` is not a finite number")
    for k, v in data["per_model"].items():
        if not isinstance(v, dict) or not all(isinstance(v.get(f), (int, float)) for f in ("runs", "usd", "tokens")):
            bad(f"per_model `{k}` is malformed")


def _no_constant(text):
    raise ValueError(f"{text} is not a finite number")


def load_baseline(root, name):
    if not NAME_RE.match(name):
        raise InputError(f"baseline name {name!r}: use letters, digits, '.', '_' and '-' only")
    path = baselines_dir(root) / f"{name}.json"
    try:
        data = json.loads(path.read_text(encoding="utf-8"), parse_constant=_no_constant)
    except FileNotFoundError:
        raise InputError(f"{path}: no such baseline (record it with: baseline.py record {name} --since ...)") from None
    except (OSError, UnicodeError, ValueError) as e:
        raise InputError(f"{path}: unreadable ({e})") from None
    validate_baseline(data, path)
    return data


def cmd_record(args):
    if not NAME_RE.match(args.name):
        raise InputError(f"baseline name {args.name!r}: use letters, digits, '.', '_' and '-' only")
    since = parse_time(args.since, "--since")
    until = parse_time(args.until, "--until") if args.until else int(time.time())
    if until <= since:
        raise InputError(f"--until {iso(until)} is not after --since {iso(since)}")
    root = main_root()
    out = baselines_dir(root) / f"{args.name}.json"
    if out.exists():
        raise InputError(f"{out}: already exists; a baseline is evidence and is never overwritten, pick a new name")
    toml = root / "broadsword.toml"
    try:
        toml_hash = hashlib.sha256(toml.read_bytes()).hexdigest()
    except OSError as e:
        raise InputError(f"{toml}: unreadable ({e})") from None
    values, samples, per_model, main_sha, notes = measure(root, since, until)
    check_finite(values, per_model)
    data = {
        "schema": SCHEMA, "name": args.name, "recorded_at": iso(int(time.time())),
        "window": {"since": iso(since), "until": iso(until), "hours": round((until - since) / 3600, 3)},
        "main_commit": main_sha, "broadsword_toml_sha256": toml_hash,
        "metrics": values, "samples": samples, "per_model": per_model, "notes": notes,
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    # Never-overwrite must be atomic: write a uniquely named temp file, then os.link it to the final name. link fails if
    # the name exists, so of two concurrent records exactly one wins and the loser exits 1 with the winner untouched.
    tmp = out.with_name(f".{out.name}.{os.getpid()}.{uuid.uuid4().hex}.tmp")
    try:
        tmp.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        try:
            os.link(tmp, out)
        except FileExistsError:
            raise InputError(f"{out}: already exists; a baseline is evidence and is never overwritten, pick a new name") from None
        except OSError as e:
            raise InputError(f"{out}: cannot create ({e})") from None
    finally:
        try:
            tmp.unlink()
        except OSError:
            pass
    print(f"baseline {args.name}: {data['window']['since']} to {data['window']['until']} ({data['window']['hours']} h)")
    print(f"main {main_sha[:10]}, broadsword.toml sha256 {toml_hash[:12]}")
    for k, (group, _, unit, desc) in METRICS.items():
        print(f"  {group:<10} {k:<26} {fmt(values[k], unit):>12}  {desc}")
    print(f"  per model: " + ("; ".join(f"{k} {v['runs']} run(s), {v['tokens']} tokens, ${v['usd']:.4f}" for k, v in sorted(per_model.items())) or "none"))
    for nline in notes:
        print(f"note: {nline}")
    print(f"written: {out}")
    return 0


def compare(before, after, min_samples, noise, metrics=None):
    """Rows (metric, before, after, change %, verdict, reason) and the overall verdict. Pure: no I/O.
    Overall: WORSE if any compared metric is WORSE (watch-for-harm rule), else BETTER if any is BETTER, else NOISE."""
    names = list(METRICS) if metrics is None else metrics
    unknown = [m for m in names if m not in METRICS]
    if unknown:
        raise InputError(f"unknown metric(s) {', '.join(unknown)}; known: {', '.join(METRICS)}")
    nb = before["samples"].get("merged_tasks", 0)
    na = after["samples"].get("merged_tasks", 0)
    few = nb < min_samples or na < min_samples
    rows = []
    for m in names:
        _, direction, unit, _ = METRICS[m]
        b, a = before["metrics"].get(m), after["metrics"].get(m)
        change = None
        if b is not None and a is not None:
            change = 0.0 if a == b else (math.inf if b == 0 else (a - b) / abs(b) * 100)
            if b == 0 and a < 0:
                change = -math.inf
        if direction is None:
            verdict, reason = "NOISE", "context only, no direction"
        elif b is None or a is None:
            verdict, reason = "NOISE", "not recorded on both sides"
        elif few:
            verdict, reason = "NOISE", f"fewer than {min_samples} merged tasks (before {nb}, after {na})"
        elif abs(change) <= noise:
            verdict, reason = "NOISE", f"within {noise:g}%"
        else:
            good = (change > 0) == (direction == "up")
            verdict, reason = ("BETTER" if good else "WORSE"), ""
        rows.append((m, b, a, change, verdict, reason, unit))
    verdicts = [r[4] for r in rows if METRICS[r[0]][1] is not None]
    overall = "WORSE" if "WORSE" in verdicts else "BETTER" if "BETTER" in verdicts else "NOISE"
    return rows, overall


def cmd_compare(args):
    root = main_root()
    before, after = load_baseline(root, args.before), load_baseline(root, args.after)
    metrics = [m.strip() for m in args.metrics.split(",") if m.strip()] if args.metrics else None
    rows, overall = compare(before, after, args.min_samples, args.noise, metrics)
    for label, d in (("before", before), ("after", after)):
        w = d["window"]
        print(f"{label:<6} {d['name']}: {w['since']} to {w['until']}, main {d['main_commit'][:10]}, "
              f"toml {d['broadsword_toml_sha256'][:12]}, {d['samples'].get('merged_tasks', 0)} merged task(s)")
    if before["broadsword_toml_sha256"] != after["broadsword_toml_sha256"]:
        print("note: broadsword.toml differs between the two (expected when the change is a setting)")
    print(f"{'metric':<26} {'before':>12} {'after':>12} {'change':>9}  verdict")
    for m, b, a, change, verdict, reason, unit in rows:
        ch = "-" if change is None else ("inf" if math.isinf(change) else f"{change:+.1f}%")
        print(f"{m:<26} {fmt(b, unit):>12} {fmt(a, unit):>12} {ch:>9}  {verdict}{'  (' + reason + ')' if reason else ''}")
    models = sorted(set(before.get("per_model", {})) | set(after.get("per_model", {})))
    if models:
        print("per model (context only): runs, tokens per run, USD per run, before -> after")
    for k in models:
        cells = []
        for d in (before, after):
            v = d.get("per_model", {}).get(k)
            cells.append(f"{v['runs']} run(s), {v['tokens'] // v['runs'] if v['runs'] else 0} tok, ${v['usd'] / v['runs'] if v['runs'] else 0:.4f}" if v else "-")
        print(f"  {k:<24} {cells[0]}  ->  {cells[1]}")
    counts = {v: sum(1 for r in rows if r[4] == v) for v in ("BETTER", "WORSE", "NOISE")}
    print(f"overall: {overall} ({counts['BETTER']} better, {counts['WORSE']} worse, {counts['NOISE']} noise; "
          f"--min-samples {args.min_samples}, --noise {args.noise:g}%)")
    return 0


def cmd_list(args):
    root = main_root()
    d = baselines_dir(root)
    names = sorted(p.stem for p in d.glob("*.json")) if d.is_dir() else []
    for name in names:
        b = load_baseline(root, name)
        print(f"{name}\t{b['window']['since']}\t{b['window']['until']}\t{b['samples'].get('merged_tasks', 0)} merged")
    if not names:
        print(f"no baselines in {d}")
    return 0


def positive_int(text):
    v = int(text)
    if v < 1:
        raise argparse.ArgumentTypeError("must be 1 or more")
    return v


def percent(text):
    v = float(text)
    if not (0 <= v < 1000):
        raise argparse.ArgumentTypeError("must be a percent from 0 to below 1000")
    return v


def parser():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("record", help="measure a window and write state/baselines/<name>.json")
    r.add_argument("name")
    r.add_argument("--since", required=True, help="window start, ISO 8601 with a zone")
    r.add_argument("--until", help="window end, ISO 8601 with a zone (now if left out)")
    c = sub.add_parser("compare", help="compare two baselines")
    c.add_argument("before")
    c.add_argument("after")
    c.add_argument("--min-samples", required=True, type=positive_int, help="merged tasks needed on each side")
    c.add_argument("--noise", required=True, type=percent, help="changes within this percent are NOISE")
    c.add_argument("--metrics", help="comma list: compare only these metrics (the ones the change should move)")
    sub.add_parser("list", help="list recorded baselines")
    return ap


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        return {"record": cmd_record, "compare": cmd_compare, "list": cmd_list}[args.cmd](args)
    except InputError as e:
        print(f"baseline: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
