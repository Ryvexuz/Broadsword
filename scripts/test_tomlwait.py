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
        lines = ['id = T-' + name, 'project = ' + r.get('project', 'broadsword'), 'status = ' + r['status'],
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


def rec(created, started, ended, claims, project='broadsword', status='done'):
    return dict(status=status, created=created, started=started, ended=ended, claims=claims, project=project)


class SameFileAndProject(unittest.TestCase):
    def setUp(self):
        os.makedirs(SCRATCH, exist_ok=True)
        self.root = tempfile.mkdtemp(prefix='tomlwait-test-', dir=os.path.abspath(SCRATCH))

    def tearDown(self):
        shutil.rmtree(self.root, ignore_errors=True)

    def row(self, records, name):
        folder = os.path.join(self.root, 'recs')
        write_tasks(folder, records)
        out = run_script(folder)
        print(chr(10) + out)
        rows = [l for l in out.splitlines() if l.startswith('T-' + name + ' ')]
        self.assertEqual(len(rows), 1, out)
        return rows[0], out

    def test_waiter_is_not_counted_behind_a_holder_of_a_different_config_file(self):
        # Waiter waits 09:10-09:40 on config/queue.toml; the holder holds only config/review.toml 09:15-09:30.
        records = [
            ('waiter', rec('2026-11-20T09:10:00Z', '2026-11-20T09:40:00Z', '2026-11-20T09:50:00Z', 'config/queue.toml')),
            ('holder', rec('2026-11-20T09:00:00Z', '2026-11-20T09:15:00Z', '2026-11-20T09:30:00Z', 'config/review.toml')),
        ]
        line, out = self.row(records, 'waiter')
        self.assertIn('overlap=0.0', line)
        self.assertIn('measure_b overlap_sum_min=0.0', out)

    def test_waiter_is_counted_behind_a_holder_of_the_same_config_file(self):
        # Same fixture, but the holder holds config/queue.toml: the 15 minutes 09:15-09:30 are counted.
        records = [
            ('waiter', rec('2026-11-20T09:10:00Z', '2026-11-20T09:40:00Z', '2026-11-20T09:50:00Z', 'config/queue.toml')),
            ('holder', rec('2026-11-20T09:00:00Z', '2026-11-20T09:15:00Z', '2026-11-20T09:30:00Z', 'config/queue.toml')),
        ]
        line, out = self.row(records, 'waiter')
        self.assertIn('overlap=15.0', line)
        self.assertIn('measure_b overlap_sum_min=15.0', out)

    def test_a_holder_matches_through_a_backslash_path_and_one_of_several_claims(self):
        records = [
            ('waiter', rec('2026-11-20T09:10:00Z', '2026-11-20T09:40:00Z', '2026-11-20T09:50:00Z', 'src/a.rs' + chr(92) + 'nconfig/queue.toml')),
            ('holder', rec('2026-11-20T09:00:00Z', '2026-11-20T09:15:00Z', '2026-11-20T09:30:00Z', 'config' + chr(92) + 'queue.toml')),
        ]
        line, _ = self.row(records, 'waiter')
        self.assertIn('overlap=15.0', line)

    def test_record_of_another_project_is_ignored(self):
        # 'other' belongs to crash-sim and claims config/queue.toml; it must not be listed or counted as a holder.
        records = [
            ('waiter', rec('2026-11-20T09:10:00Z', '2026-11-20T09:40:00Z', '2026-11-20T09:50:00Z', 'config/queue.toml')),
            ('other', rec('2026-11-20T09:00:00Z', '2026-11-20T09:15:00Z', '2026-11-20T09:30:00Z', 'config/queue.toml', project='crash-sim')),
        ]
        line, out = self.row(records, 'waiter')
        self.assertIn('overlap=0.0', line)
        self.assertNotIn('T-other', out)
        self.assertIn('measure_a tasks=1', out)


if __name__ == '__main__':
    unittest.main()
