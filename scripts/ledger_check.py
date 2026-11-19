"""Check the RETRO.md improvement ledger: every process change cites a before and an after baseline, and WORSE means revert.

  python -I scripts/ledger_check.py --since <YYYY-MM-DD> --max-pending-days <n> [--min-samples <n>] [--noise <percent>]

--min-samples and --noise default to DEFAULT_MIN_SAMPLES and DEFAULT_NOISE below: one place, in this records-checking script
(outside the Rust crate, so not in broadsword.toml), set to the values docs/BASELINES.md and RETRO use when they quote a comparison.

For each ledger row added on or after --since (the date of the newest commit that introduced the row's `| Since | Change |`
text, so a row removed and re-added is dated by the re-add; a row not yet committed counts as added today), one of these must be written in the row, usually in Status:
  before: <name> after: <name>                     both baselines recorded with scripts/baseline.py
  before: <name> after: pending 2026-11-25         the date the after side will be measured
optionally followed by `metrics: a,b` to compare only the metrics the change should move. Rows whose Since cell
starts with `planned` are not changes yet and are skipped.

A row fails when: it names no baselines; a named baseline file is missing; it is pending with no date, past its date,
or older than --max-pending-days; or the comparison (scripts/baseline.py, same --min-samples and --noise) is WORSE
and the row's Status does not START with `reverted` or `fixed` (or `hindered ... reverted`) or say `reverted in <sha>`;
"not reverted yet" or "will be fixed" do not count.

A row whose Change names `max_agents` or a scale-up is judged on throughput and failures only (RETRO lesson 35): its WORSE
lock wait, test time and other metrics are listed as warnings and do not fail it.

Read-only: never edits RETRO.md. Output: one line per row, then `ledger: ok` or `ledger: N problems`.
Exit 0 when ok, 1 on any problem or bad input (missing RETRO.md, changed table header, git failure), 2 on usage.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import datetime as dt
import importlib.util
import pathlib
import re
import subprocess

HERE = pathlib.Path(__file__).resolve().parent
SECTION = "## Improvement ledger"
HEADER = ["Since", "Change", "Should improve", "Measured by", "Watch for harm", "Status"]
NAME = r"[A-Za-z0-9][A-Za-z0-9._-]*"
BEFORE_RE = re.compile(rf"\bbefore:\s*`?({NAME})`?")
AFTER_RE = re.compile(rf"\bafter:\s*`?({NAME})`?")
PENDING_DATE_RE = re.compile(r"\bafter:\s*`?pending`?[^|\d]{0,30}(\d{4}-\d{2}-\d{2})")
GIT_TIMEOUT = 60  # seconds per git call, as in dev_metrics
METRICS_RE = re.compile(r"\bmetrics:\s*`?([a-z0-9_]+(?:\s*,\s*[a-z0-9_]+)*)`?")
# The one place the comparison limits live. This is a records-checking script outside the Rust crate, so they are not in
# broadsword.toml (the settings rule is for the crate's limits); they are the values docs/BASELINES.md and RETRO use when
# they quote a comparison (a verdict needs 5 merged tasks a side, a change within 10% is NOISE). Change them here only.
DEFAULT_MIN_SAMPLES = 5
DEFAULT_NOISE = 10.0
# A WORSE row counts as reverted or fixed only on a clear marker in Status: it STARTS with `reverted` or `fixed`, or
# `hindered ... reverted|fixed`, or says `reverted in <sha>` / `fixed in <sha>`. A marker with a negation or future word
# before it in its clause ("not reverted yet", "will be fixed", "to be reverted if ...") does not count.
START_MARK_RE = re.compile(r"^\s*(?:(reverted|fixed)\b|hindered\b[^;]*?\b(reverted|fixed)\b)", re.IGNORECASE)
SHA_MARK_RE = re.compile(r"\b(reverted|fixed) in `?[0-9a-f]{7,40}\b", re.IGNORECASE)
NEGATED_RE = re.compile(r"\b(not|never|no|will|would|should|to be|yet|until|unless|if|once|when|after)\b|n't", re.IGNORECASE)


def marker(status):
    """The word `reverted` or `fixed` if Status carries a clear, non-negated, non-future marker, else None."""
    for rx in (START_MARK_RE, SHA_MARK_RE):
        m = rx.search(status)
        if not m:
            continue
        idx = max(i for i in range(1, (m.lastindex or 0) + 1) if m.group(i))
        # A marker that opens Status has nothing before it; otherwise look at the clause before the marker word.
        clause = "" if m.start(idx) <= len(status) - len(status.lstrip()) else re.split(r"[;.]", status[: m.start(idx)])[-1]
        if not NEGATED_RE.search(clause):
            return m.group(idx).lower()
    return None
# RETRO lesson 35 (Lazlo, 2026-11-19): a scale-up is judged on throughput and failures only. Lock wait, test time and every
# other WORSE metric are warnings printed with the row, not a veto. RAM and swap are not in a baseline, so they are not checked here.
SCALE_RE = re.compile(r"\bmax_agents\b|\bscale-?ups?\b", re.IGNORECASE)
SCALE_METRICS = ["merged_per_hour", "main_check_failed", "reverted_or_reopened"]


def load_baseline():
    path = HERE / "baseline.py"
    if not path.is_file():
        raise SystemExit(f"ledger_check: {path} not found (nb-06 baseline.py is needed)")
    spec = importlib.util.spec_from_file_location("_ledger_baseline", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def cells(line):
    s = line.strip()
    if not (s.startswith("|") and s.endswith("|")):
        return None
    return [c.strip() for c in s[1:-1].split("|")]


def ledger_rows(text, path):
    """[(line number, raw line, cells)] of the ledger table. A changed header is an error naming the columns."""
    lines = text.splitlines()
    try:
        start = next(i for i, l in enumerate(lines) if l.strip() == SECTION)
    except StopIteration:
        raise ValueError(f"{path}: no `{SECTION}` section") from None
    i = start + 1
    while i < len(lines) and not lines[i].lstrip().startswith("|"):
        if lines[i].startswith("## "):
            raise ValueError(f"{path}: the `{SECTION}` section has no table")
        i += 1
    if i >= len(lines) or cells(lines[i]) != HEADER:
        found = cells(lines[i]) if i < len(lines) else None
        raise ValueError(f"{path}: ledger header is {found}, expected exactly {HEADER}; update ledger_check.py with the table")
    sep = cells(lines[i + 1]) if i + 1 < len(lines) else None
    if not sep or not all(re.fullmatch(r":?-{3,}:?", c) for c in sep):
        raise ValueError(f"{path}: line {i + 2}: expected the table's `| --- |` separator")
    rows = []
    for j in range(i + 2, len(lines)):
        c = cells(lines[j])
        if c is None:
            break
        rows.append((j + 1, lines[j], c))
    return rows


def git_out(root, args, timeout):
    try:
        return subprocess.run(["git", "-C", str(root), *args], capture_output=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise ValueError(f"git {args[0]} in {root}: {e}") from None


def count_in(root, rev, needle, timeout):
    """How many times the row prefix occurs in RETRO.md at rev; 0 if the file or revision does not exist."""
    res = git_out(root, ["show", f"{rev}:RETRO.md"], timeout)
    return res.stdout.decode("utf-8", "replace").count(needle) if res.returncode == 0 else 0


def added_date(root, raw, timeout):
    """UTC date of the newest commit that introduced this row's `| Since | Change |` prefix (more occurrences than its
    parent had), so a row removed and later re-added is dated by the re-add. Today if HEAD does not hold it (not committed)."""
    s = raw.strip()
    cut = [m.start() for m in re.finditer(r"\|", s)]
    needle = s[: cut[2] + 1] if len(cut) > 2 else s
    res = git_out(root, ["log", "--format=%at %H", "-S", needle, "--", "RETRO.md"], timeout)
    if res.returncode != 0:
        raise ValueError(f"git log -S in {root}: {res.stderr.decode('utf-8', 'replace').strip()[:300]}")
    today = dt.datetime.now(dt.timezone.utc).date()
    if count_in(root, "HEAD", needle, timeout) == 0:
        return today
    for line in res.stdout.decode("utf-8", "replace").splitlines():  # newest first
        at, _, sha = line.partition(" ")
        if at.isdigit() and count_in(root, sha, needle, timeout) > count_in(root, f"{sha}^", needle, timeout):
            return dt.datetime.fromtimestamp(int(at), dt.timezone.utc).date()
    return today


def check_row(bl, root, cs, added, today, args):
    """(ok, message) for one row."""
    since_cell, status = cs[0], cs[5]
    text = " ".join(cs)
    if since_cell.lower().startswith("planned"):
        return True, "skipped (planned, not a change yet)"
    b = BEFORE_RE.search(text)
    a = AFTER_RE.search(text)
    if not b and not a:
        return False, "names no baselines: write `before: <name> after: <name>` or `after: pending YYYY-MM-DD`"
    if not b:
        return False, "names an after but no `before: <name>` baseline"
    if not a:
        return False, f"names before `{b.group(1)}` but no `after: <name>` or `after: pending YYYY-MM-DD`"
    bdir = root / "state" / "baselines"
    if not (bdir / f"{b.group(1)}.json").is_file():
        return False, f"before baseline missing: {bdir / (b.group(1) + '.json')}"
    if a.group(1) == "pending":
        age = (today - added).days
        d = PENDING_DATE_RE.search(text)
        if not d:
            return False, "after is pending with no date: write `after: pending YYYY-MM-DD`"
        try:
            due = dt.date.fromisoformat(d.group(1))
        except ValueError:
            return False, f"after: pending date {d.group(1)} is not a real date"
        if due < today:
            return False, f"after was due {due} and is still pending"
        if age > args.max_pending_days:
            return False, f"pending for {age} days (row added {added}), more than --max-pending-days {args.max_pending_days}"
        return True, f"pending, to be measured by {due} (row added {added})"
    try:
        before = bl.load_baseline(root, b.group(1))
        after = bl.load_baseline(root, a.group(1))
        m = METRICS_RE.search(text)
        metrics = [x.strip() for x in m.group(1).split(",")] if m else None
        rows, overall = bl.compare(before, after, args.min_samples, args.noise, metrics)
        warn = ""
        if SCALE_RE.search(cs[1]) and metrics is None:
            overall = bl.compare(before, after, args.min_samples, args.noise, SCALE_METRICS)[1]
            names = [r[0] for r in rows if r[4] == "WORSE" and r[0] not in SCALE_METRICS]
            warn = f" (scale-up, judged on {', '.join(SCALE_METRICS)}; warnings: {', '.join(names) if names else 'none'})"
    except bl.InputError as e:
        return False, str(e)
    what = f"{b.group(1)} -> {a.group(1)}: {overall}{warn}"
    if overall == "WORSE":
        mark = marker(status)
        if mark:
            return True, f"{what}, marked {mark}"
        return False, (f"{what} and not marked `reverted` or `fixed` (Status must start with it, or say `reverted in <sha>`; "
                       f"'not yet' and 'will be' do not count): revert it or fix it (project rule)")
    return True, what


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--since", required=True, type=dt.date.fromisoformat, help="check rows added on or after this UTC date")
    ap.add_argument("--max-pending-days", required=True, type=int, help="a pending row older than this fails")
    ap.add_argument("--min-samples", type=int, default=DEFAULT_MIN_SAMPLES, help=f"passed to baseline.py compare (default {DEFAULT_MIN_SAMPLES})")
    ap.add_argument("--noise", type=float, default=DEFAULT_NOISE, help=f"passed to baseline.py compare, percent (default {DEFAULT_NOISE:g})")
    args = ap.parse_args(argv)
    if args.max_pending_days < 0 or args.min_samples < 1 or args.noise < 0:
        ap.error("--max-pending-days must be 0 or more, --min-samples 1 or more, --noise 0 or more")
    bl = load_baseline()
    try:
        root = bl.main_root()
    except bl.InputError as e:
        print(f"ledger_check: {e}", file=sys.stderr)
        return 1
    retro = root / "RETRO.md"
    today = dt.datetime.now(dt.timezone.utc).date()
    try:
        rows = ledger_rows(retro.read_text(encoding="utf-8"), retro)
        dated = [(n, raw, cs, added_date(root, raw, GIT_TIMEOUT)) for n, raw, cs in rows]
    except (OSError, UnicodeError, ValueError) as e:
        print(f"ledger_check: {e}", file=sys.stderr)
        return 1
    problems = checked = 0
    for n, raw, cs, added in dated:
        label = f"RETRO.md:{n} [{cs[0] if cs else '?'}] {(cs[1] if len(cs) > 1 else raw)[:60]}"
        if len(cs) != len(HEADER):
            problems += 1
            print(f"{label}: PROBLEM - {len(cs)} cells, expected {len(HEADER)}")
            continue
        if added < args.since:
            continue
        checked += 1
        ok, msg = check_row(bl, root, cs, added, today, args)
        problems += not ok
        print(f"{label}: {'ok' if ok else 'PROBLEM'} - {msg}")
    print(f"checked {checked} of {len(dated)} row(s) added on or after {args.since}")
    print("ledger: ok" if problems == 0 else f"ledger: {problems} problem{'s' if problems != 1 else ''}")
    return 0 if problems == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
