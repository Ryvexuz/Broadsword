"""Toml-claim wait and hold measure for docs/CONFIG-SPLIT.md (sections 1 and 8).

Usage (run from the worktree or the project root; the task records live outside Git):
    python -I scripts/tomlwait.py --start 2026-11-20T09:00:00Z --end 2026-11-21T04:40:00Z [--tasks-dir DIR]
--start and --end are both required UTC times in the form YYYY-MM-DDTHH:MM:SSZ. There are no defaults:
a missing or malformed argument exits 2 with a usage message. Any task record with a missing or malformed
claims line exits 2 and names the file.

Takes every task whose `claims` names broadsword.toml or a config/*.toml file and
that was created inside the window [--start, --end]. Per task, in minutes:
  wait    = created -> started (or ended if abandoned unstarted, else the window end)
  hold    = started -> ended (or the window end, if still running); 0 if never started
  Every wait and hold interval is clipped to [--start, --end] (clip_iv), so a task that ends after --end counts
  only up to --end.
  overlap = part of the wait during which ANOTHER listed task held the claim
Summary measures (named in docs/CONFIG-SPLIT.md section 8):
  measure_a = number of tasks listed
  measure_b = sum of overlap over all tasks (minutes waited behind a holder)
  measure_c = minutes during which at least one task held the claim while at
              least one other listed task was waiting (as a time union)
"""
import argparse
import datetime
import glob
import os
import re
import sys

ISO_FORMAT = '%Y-%m-%dT%H:%M:%SZ'
CLAIM_SPLIT = re.compile(re.escape(chr(92) + 'n') + '|' + chr(10))


def fail(msg):
    print('tomlwait: ' + msg, file=sys.stderr)
    sys.exit(2)


def parse_utc(text):
    try:
        t = datetime.datetime.strptime(text, ISO_FORMAT).replace(tzinfo=datetime.timezone.utc)
    except ValueError:
        raise argparse.ArgumentTypeError('expected UTC time YYYY-MM-DDTHH:MM:SSZ, got %r' % text)
    return int(t.timestamp())


def find_tasks_dir(arg):
    """Return the task records folder: the --tasks-dir value if one was given. Otherwise
    try ../../../state/tasks, then state/tasks, both relative to the current folder, and
    return the first that is a directory. Exits 2 if neither is one."""
    if arg:
        return arg
    for cand in ('../../../state/tasks', 'state/tasks'):
        if os.path.isdir(cand):
            return cand
    fail('no state/tasks folder found; pass --tasks-dir')


def is_toml_claim(c):
    c = c.replace(chr(92), '/')
    return c == 'broadsword.toml' or (c.startswith('config/') and c.endswith('.toml'))


def read_record(f):
    d = {}
    with open(f, encoding='utf8') as fh:
        for l in fh:
            if ' = ' in l:
                k, v = l.rstrip('\n').split(' = ', 1)
                d[k] = v
    return d


def claim_entries(f, d):
    """The claim entries of one record. The record must carry a claims line, and every
    entry (split on a literal backslash-n or a newline) must be non-empty with no
    leading or trailing spaces. Anything else is malformed: exit 2, naming the file."""
    if 'claims' not in d:
        fail('%s: no claims line' % f)
    raw = d['claims']
    entries = CLAIM_SPLIT.split(raw)
    if any(e.strip() == '' or e != e.strip() for e in entries):
        fail('%s: malformed claims line %r (empty value, empty entry or padded entry)' % (f, raw))
    return entries


def load(tasks_dir, win_start, now):
    rows = []
    for f in sorted(glob.glob(os.path.join(tasks_dir, '*.task'))):
        d = read_record(f)
        entries = claim_entries(f, d)
        if any(is_toml_claim(c) for c in entries) and win_start <= int(d.get('created', '0')) <= now:
            d['_c'] = int(d['created'])
            d['_s'] = int(d['started']) if d.get('started') else None
            d['_e'] = int(d['ended']) if d.get('ended') else None
            rows.append(d)
    rows.sort(key=lambda d: d['_c'])
    return rows


def clip_iv(lo, hi, win_start, win_end):
    """The one clip every hold and wait interval goes through: [lo, hi] cut to [win_start, win_end].
    An interval that lies wholly outside the window comes back empty (lo == hi)."""
    lo, hi = max(lo, win_start), min(hi, win_end)
    return (lo, max(lo, hi))


def hold_iv(d, win_start, now):
    return clip_iv(d['_s'], d['_e'] or now, win_start, now) if d['_s'] else None


def wait_iv(d, win_start, now):
    return clip_iv(d['_c'], d['_s'] or d['_e'] or now, win_start, now)


def inter(a, b):
    lo, hi = max(a[0], b[0]), min(a[1], b[1])
    return (lo, hi) if hi > lo else None


def union_len(ivs):
    ivs = sorted(ivs)
    tot, end = 0, None
    cur = None
    for lo, hi in ivs:
        if cur is None or lo > cur[1]:
            if cur:
                tot += cur[1] - cur[0]
            cur = [lo, hi]
        else:
            cur[1] = max(cur[1], hi)
    if cur:
        tot += cur[1] - cur[0]
    return tot


def main():
    ap = argparse.ArgumentParser(description='Toml-claim wait and hold measure (docs/CONFIG-SPLIT.md section 8).')
    ap.add_argument('--start', type=parse_utc, required=True, help='window start, UTC, YYYY-MM-DDTHH:MM:SSZ')
    ap.add_argument('--end', type=parse_utc, required=True, help='window end, UTC, YYYY-MM-DDTHH:MM:SSZ')
    ap.add_argument('--tasks-dir', default=None, help='task records folder (default: found as in find_tasks_dir)')
    args = ap.parse_args()
    win_start, now = args.start, args.end
    if now <= win_start:
        fail('--end must be later than --start')
    tasks_dir = find_tasks_dir(args.tasks_dir)
    rows = load(tasks_dir, win_start, now)
    if not rows:
        fail('no toml-claiming task in the window (empty input is an error)')
    fmt = lambda t: datetime.datetime.fromtimestamp(int(t), datetime.timezone.utc).strftime('%m-%d %H:%M:%S')
    win_min = (now - win_start) / 60
    print('window', fmt(win_start), 'to', fmt(now), 'minutes %.1f' % win_min)
    print('id | status | created | started | ended | wait_min | hold_min | overlap_min')
    tot_hold = tot_over = 0.0
    contended = []
    for d in rows:
        w = wait_iv(d, win_start, now)
        h = hold_iv(d, win_start, now)
        ov = []
        for o in rows:
            if o is d:
                continue
            oh = hold_iv(o, win_start, now)
            if oh:
                x = inter(w, oh)
                if x:
                    ov.append(x)
                    contended.append(x)
        d['_ov'] = union_len(ov) / 60
        d['_hold'] = (h[1] - h[0]) / 60 if h else 0.0
        tot_over += d['_ov']
        if d['_s'] and d['_e']:
            tot_hold += d['_hold']
        print(d['id'], d['status'], fmt(d['_c']), fmt(d['_s']) if d['_s'] else '-', fmt(d['_e']) if d['_e'] else '-',
              'wait=%.1f' % ((w[1] - w[0]) / 60), 'hold=%.1f' % d['_hold'], 'overlap=%.1f' % d['_ov'])
    print('tasks', len(rows), 'started', sum(1 for d in rows if d['_s']),
          'abandoned_unstarted', sum(1 for d in rows if not d['_s'] and d['status'] == 'abandoned'),
          'open_unstarted', sum(1 for d in rows if not d['_s'] and d['status'] == 'open'))
    print('total hold of started+ended tasks: %.1f min = %.1f%% of window' % (tot_hold, 100 * tot_hold / win_min))
    print('measure_a tasks=%d' % len(rows))
    print('measure_b overlap_sum_min=%.1f' % tot_over)
    print('measure_c contended_hold_min=%.1f hours=%.2f' % (union_len(contended) / 60, union_len(contended) / 3600))


main()
