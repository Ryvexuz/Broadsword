"""Tests for scripts/tomlwait.py: every hold and wait interval is clipped to the window [--start, --end].

Each test runs the script as a subprocess (python -I) over a fixture folder of task records.
"""
import calendar
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, 'tomlwait.py')
SCRATCH = os.path.join(HERE, '..', 'scratch')
WIN_START = '2026-11-20T09:00:00Z'
WIN_END = '2026-11-20T10:00:00Z'


def epoch(text):
    return calendar.timegm(time.strptime(text, '%Y-%m-%dT%H:%M:%SZ'))


# Each record: created, started, ended (ISO or None), status, claims. Task 'late' ends after the window end.
RECORDS = [
    ('early', dict(status='done', created='2026-11-20T09:10:00Z', started='2026-11-20T09:20:00Z',
                   ended='2026-11-20T09:50:00Z', claims='broadsword.toml')),
    ('late', dict(status='done', created='2026-11-20T09:15:00Z', started='2026-11-20T09:25:00Z',
                  ended='2026-11-20T10:30:00Z', claims='broadsword.toml')),
    ('abandoned', dict(status='abandoned', created='2026-11-20T09:45:00Z', started=None,
                       ended='2026-11-20T10:40:00Z', claims='config/queue.toml')),
    ('open', dict(status='open', created='2026-11-20T09:30:00Z', started=None, ended=None,
                  claims='config/board.toml\\nsrc/lib.rs')),
]


def write_tasks(folder, records):
    os.makedirs(folder, exist_ok=True)
    for name, r in records:
        lines = ['id = T-' + name, 'project = broadsword', 'status = ' + r['status'],
                 'created = %d' % epoch(r['created']), 'claims = ' + r['claims']]
        if r['started']:
            lines.append('started = %d' % epoch(r['started']))
        if r['ended']:
            lines.append('ended = %d' % epoch(r['ended']))
        with open(os.path.join(folder, name + '.task'), 'w', encoding='utf8') as fh:
            fh.write('\n'.join(lines) + '\n')


def results(out):
    """The script output with each row's raw `ended` column removed. The raw ended time is the record's own
    value and is printed as read; every measure (wait, hold, overlap, the summary lines) must match."""
    return '\n'.join(re.sub(r' \d\d-\d\d \d\d:\d\d:\d\d(?= wait=)', '', line) for line in out.splitlines())


def run_script(tasks_dir):
    proc = subprocess.run([sys.executable, '-I', SCRIPT, '--start', WIN_START, '--end', WIN_END,
                           '--tasks-dir', tasks_dir], capture_output=True, text=True, timeout=120)
    if proc.returncode != 0:
        raise AssertionError('tomlwait exited %d: %s' % (proc.returncode, proc.stderr))
    return proc.stdout


class ClipToWindow(unittest.TestCase):
    def setUp(self):
        os.makedirs(SCRATCH, exist_ok=True)
        self.root = tempfile.mkdtemp(prefix='tomlwait-test-', dir=os.path.abspath(SCRATCH))

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def test_task_ending_after_window_end_equals_run_with_end_at_window_end(self):
        # Run A: the records as they are; 'late' ends at 10:30, after --end (10:00).
        # Run B: the same records, but 'late' ends at exactly --end. Clipping means A must print B's output.
        as_is = os.path.join(self.root, 'as_is')
        at_end = os.path.join(self.root, 'at_end')
        write_tasks(as_is, RECORDS)
        clipped = [(n, dict(r, ended=WIN_END) if n == 'late' else r) for n, r in RECORDS]
        write_tasks(at_end, clipped)
        out_as_is = results(run_script(as_is))
        out_at_end = results(run_script(at_end))
        print('\n--- run with late ended 10:30 (results) ---\n' + out_as_is)
        print('--- run with late ended at --end 10:00 (results) ---\n' + out_at_end)
        self.assertEqual(out_as_is, out_at_end)

    def test_abandoned_unstarted_task_ending_after_window_end_waits_only_to_window_end(self):
        # 'abandoned' never started and ends at 10:40. Its wait must stop at --end (10:00): 15 minutes.
        as_is = os.path.join(self.root, 'as_is')
        write_tasks(as_is, RECORDS)
        out = run_script(as_is)
        line = [l for l in out.splitlines() if l.startswith('T-abandoned ')]
        print('\n' + '\n'.join(line))
        self.assertEqual(len(line), 1, out)
        self.assertIn('wait=15.0', line[0])
        self.assertIn('overlap=', line[0])


if __name__ == '__main__':
    unittest.main()
