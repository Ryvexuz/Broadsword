"""Toml-claim wait and hold measure for docs/CONFIG-SPLIT.md (sections 1 and 8).

Usage (from the project root; the task records live outside Git):
    python -I scripts/tomlwait.py [tasks_dir [window_start_epoch [now_epoch]]]
Defaults: tasks_dir = state/tasks (relative to the current folder: the
main checkout three levels above a worktree, first; the worktree's own state/tasks is a stale copy), window 2026-11-20T09:00:00Z to
2026-11-21T04:40:00Z (the section 1 baseline).

Takes every task whose `claims` names broadsword.toml or a config/*.toml file and
that was created inside the window. Per task, in minutes:
  wait    = created -> started (or ended if abandoned unstarted, else now)
  hold    = started -> ended (or now, if still running); 0 if never started
  overlap = part of the wait during which ANOTHER listed task held the claim
Summary measures (named in docs/CONFIG-SPLIT.md section 8):
  measure_a = number of tasks listed
  measure_b = sum of overlap over all tasks (minutes waited behind a holder)
  measure_c = minutes during which at least one task held the claim while at
              least one other listed task was waiting (as a time union)
"""
import glob
import os
import re
import sys
import datetime

WIN_START = 1795165200  # 2026-11-20T09:00:00Z
NOW = 1795236000        # 2026-11-21T04:40:00Z


def find_tasks_dir(arg):
    if arg:
        return arg
    for cand in ('../../../state/tasks', 'state/tasks'):
        if os.path.isdir(cand):
            return cand
    sys.exit('tomlwait: no state/tasks folder found; pass the tasks dir as the first argument')


def is_toml_claim(c):
    c = c.replace(chr(92), '/')
    return c == 'broadsword.toml' or (c.startswith('config/') and c.endswith('.toml'))


def load(tasks_dir, win_start, now):
    rows = []
    for f in sorted(glob.glob(os.path.join(tasks_dir, '*.task'))):
        d = {}
        with open(f, encoding='utf8') as fh:
            for l in fh:
                if ' = ' in l:
                    k, v = l.rstrip('\n').split(' = ', 1)
                    d[k] = v
        cl = re.split(re.escape(chr(92) + 'n') + '|' + chr(10), d.get('claims', ''))
        if any(is_toml_claim(c) for c in cl) and win_start <= int(d.get('created', '0')) <= now:
            d['_c'] = int(d['created'])
            d['_s'] = int(d['started']) if d.get('started') else None
            d['_e'] = int(d['ended']) if d.get('ended') else None
            rows.append(d)
    rows.sort(key=lambda d: d['_c'])
    return rows


def hold_iv(d, now):
    return (d['_s'], d['_e'] or now) if d['_s'] else None


def wait_iv(d, now):
    return (d['_c'], d['_s'] or d['_e'] or now)


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
    a = sys.argv[1:]
    tasks_dir = find_tasks_dir(a[0] if len(a) > 0 else None)
    win_start = int(a[1]) if len(a) > 1 else WIN_START
    now = int(a[2]) if len(a) > 2 else NOW
    rows = load(tasks_dir, win_start, now)
    if not rows:
        sys.exit('tomlwait: no toml-claiming task in the window (empty input is an error)')
    fmt = lambda t: datetime.datetime.fromtimestamp(int(t), datetime.timezone.utc).strftime('%m-%d %H:%M:%S')
    win_min = (now - win_start) / 60
    print('window', fmt(win_start), 'to', fmt(now), 'minutes %.1f' % win_min)
    print('id | status | created | started | ended | wait_min | hold_min | overlap_min')
    tot_hold = tot_over = 0.0
    contended = []
    for d in rows:
        w = wait_iv(d, now)
        h = hold_iv(d, now)
        ov = []
        for o in rows:
            if o is d:
                continue
            oh = hold_iv(o, now)
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
