"""Tests for scripts/backup_status.py. Scratch repos and a scratch bare remote live under the
project's scratch folder (never the system temp folder) and are removed afterwards.

  python -I -B scripts/test_backup_status.py -v
"""
import sys

sys.dont_write_bytecode = True

import contextlib
import importlib.util
import io
import json
import os
import pathlib
import re
import shutil
import subprocess
import time
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = pathlib.Path(os.environ.get("BACKUP_STATUS_UNDER_TEST", str(HERE / "backup_status.py")))


def scratch_dir():
    toml = (HERE.parent / "broadsword.toml").read_text(encoding="utf-8")
    name = re.search(r'^scratch_dir\s*=\s*"([^"]+)"', toml, re.M).group(1)
    for anc in HERE.parents:
        if (anc / name).is_dir() and (anc / "worktrees").is_dir():   # the main checkout, not a worktree
            return anc / name
    raise RuntimeError("no %s folder found above %s" % (name, HERE))


spec = importlib.util.spec_from_file_location("backup_status_under_test", SCRIPT)
bs = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bs)


def rmtree(path):
    def onerror(func, p, _exc):   # git object files are read-only on Windows
        os.chmod(p, 0o700)
        func(p)
    shutil.rmtree(path, onerror=onerror) if os.path.exists(path) else None


def g(repo, *args):
    r = subprocess.run(["git", "-C", str(repo), "-c", "user.name=t", "-c", "user.email=t@t",
                        "-c", "commit.gpgsign=false", "-c", "tag.gpgsign=false", *args],
                       capture_output=True, text=True)
    assert r.returncode == 0, (args, r.stderr)
    return r.stdout.strip()


def commit(repo, name, date=None):
    (pathlib.Path(repo) / name).write_text(name)
    g(repo, "add", name)
    if date:
        os.environ["GIT_COMMITTER_DATE"] = date
        os.environ["GIT_AUTHOR_DATE"] = date
    try:
        g(repo, "commit", "-m", name)
    finally:
        os.environ.pop("GIT_COMMITTER_DATE", None)
        os.environ.pop("GIT_AUTHOR_DATE", None)


class Base(unittest.TestCase):
    def setUp(self):
        self.base = scratch_dir() / ("backup_status_test_%d_%s" % (os.getpid(), self.id().split(".")[-1]))
        rmtree(self.base)
        self.root = self.base / "root"
        self.bare = self.base / "remote.git"
        self.root.mkdir(parents=True)
        subprocess.run(["git", "init", "--bare", "-q", str(self.bare)], check=True)
        subprocess.run(["git", "init", "-q", "-b", "main", str(self.root)], check=True)
        g(self.root, "remote", "add", "origin", str(self.bare))
        commit(self.root, "a.txt")
        g(self.root, "push", "-q", "origin", "main")
        g(self.root, "fetch", "-q", "origin")   # local scratch bare repo: fills tracking refs
        self.map = self.base / "map.json"
        self.write_map([{"name": "root", "path": ".", "branch": "main",
                         "remote": "origin", "remote_branch": "main"}])

    def tearDown(self):
        rmtree(self.base)

    def write_map(self, repos):
        self.map.write_text(json.dumps({"repos": repos}), encoding="utf-8")

    def run_cli(self, *extra):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = bs.main(["--root", str(self.root), "--map", str(self.map), *extra])
        return code, out.getvalue()

    def remote_refs(self):
        return subprocess.run(["git", "-C", str(self.bare), "for-each-ref", "--format=%(refname)"],
                              capture_output=True, text=True).stdout.split()


class Status(Base):
    def test_clean(self):
        code, out = self.run_cli()
        self.assertEqual(code, 0, out)
        self.assertIn("ahead 0", out)
        self.assertTrue(out.rstrip().endswith("backup: ok"), out)

    def test_ahead_count_and_age(self):
        old = "%d +0000" % (time.time() - 3 * 86400)
        commit(self.root, "b.txt", old)
        commit(self.root, "c.txt")
        code, out = self.run_cli()
        self.assertEqual(code, 1, out)
        self.assertIn("ahead 2", out)
        self.assertIn("oldest unpushed 3d", out)
        self.assertIn("origin/main updated", out)
        self.assertTrue(out.rstrip().endswith("backup: 1 repos behind"), out)

    def test_ref_age_comes_from_reflog_not_packed_refs(self):
        old = "%d +0000" % (time.time() - 5 * 86400)
        sha = g(self.root, "rev-parse", "origin/main")
        g(self.root, "update-ref", "-d", "refs/remotes/origin/main")   # drops the ref and its reflog
        os.environ["GIT_COMMITTER_DATE"] = old
        try:
            g(self.root, "update-ref", "--create-reflog", "-m", "old fetch", "refs/remotes/origin/main", sha)
        finally:
            os.environ.pop("GIT_COMMITTER_DATE", None)
        g(self.root, "branch", "gone")
        g(self.root, "pack-refs", "--all")
        g(self.root, "branch", "-D", "gone")   # rewrites packed-refs now
        code, out = self.run_cli()
        self.assertIn("origin/main updated 5d ago", out)

    def test_no_reflog_says_age_unknown(self):
        g(self.root, "reflog", "expire", "--expire=all", "--all")
        os.remove(self.root / ".git" / "logs" / "refs" / "remotes" / "origin" / "main")
        code, out = self.run_cli()
        self.assertIn("origin/main age unknown", out)

    def test_missing_tag(self):
        commit(self.root, "b.txt")
        g(self.root, "push", "-q", "origin", "main")
        g(self.root, "fetch", "-q", "origin")
        g(self.root, "tag", "v1")
        code, out = self.run_cli()
        self.assertEqual(code, 0, out)  # tag on an already-pushed commit: documented limitation
        self.assertTrue(out.rstrip().endswith("backup: ok (tags: offline check only)"), out)
        commit(self.root, "c.txt")
        g(self.root, "tag", "v2")
        code, out = self.run_cli()
        self.assertEqual(code, 1, out)
        self.assertIn("tags not on remote 1 (v2)", out)

    def test_missing_tracking_ref_is_behind(self):
        self.write_map([{"name": "root", "path": ".", "branch": "main",
                         "remote": "origin", "remote_branch": "nope"}])
        code, out = self.run_cli()
        self.assertEqual(code, 1, out)
        self.assertIn("cannot verify", out)

    def test_map_required_keys(self):
        self.write_map([{"name": "root", "path": ".", "branch": "main", "remote": "origin"}])
        code, out = self.run_cli()
        self.assertEqual(code, 2)
        self.assertIn("remote_branch", out)

    def test_null_tracking_path_is_same_as_absent(self):
        self.write_map([{"name": "root", "path": ".", "branch": "main", "remote": "origin",
                         "remote_branch": "main", "tracking_path": None}])
        code, out = self.run_cli()
        self.assertEqual(code, 0, out)
        self.assertNotIn("Traceback", out)

    def test_bad_tracking_path_type_exit_2(self):
        self.write_map([{"name": "root", "path": ".", "branch": "main", "remote": "origin",
                         "remote_branch": "main", "tracking_path": 5}])
        code, out = self.run_cli()
        self.assertEqual(code, 2, out)
        self.assertIn("tracking_path", out)

    def test_url_credentials_never_printed(self):
        commit(self.root, "b.txt")
        g(self.root, "remote", "set-url", "origin", "https://user:FAKETOKEN123@example.invalid/r.git")
        code, out = self.run_cli("--push")   # no --yes: prints the plan only, no network
        self.assertEqual(code, 2, out)
        self.assertIn("will push", out)
        self.assertIn("https://example.invalid/r.git", out)
        self.assertNotIn("FAKETOKEN123", out)
        self.assertNotIn("user:", out)
        self.assertEqual(bs.scrub("fatal: unable to access 'https://u:tok@h.example/x.git/'"),
                         "fatal: unable to access 'https://h.example/x.git/'")
        self.assertEqual(bs.scrub("git@github.com:me/r.git"), "git@github.com:me/r.git")

    def test_read_only_by_default(self):
        commit(self.root, "b.txt")
        before = self.remote_refs()
        self.run_cli()
        self.assertEqual(before, self.remote_refs())
        self.assertEqual(g(self.root, "rev-parse", "origin/main"), g(self.root, "rev-parse", "main~1"))

    def test_other_tracking_repo(self):
        sub = self.root / "sub"
        subprocess.run(["git", "init", "-q", "-b", "main", str(sub)], check=True)
        commit(sub, "s.txt")
        self.write_map([{"name": "sub", "path": "sub", "branch": "main", "remote": "origin",
                         "remote_branch": "sub-main", "tracking_path": "."}])
        # sub's commit goes to the remote under another name and is tracked from the root repo
        g(sub, "push", "-q", str(self.bare), "main:refs/heads/sub-main")
        g(sub, "fetch", "-q", str(self.bare), "refs/heads/sub-main")
        g(self.root, "fetch", "-q", "origin")
        code, out = self.run_cli()
        self.assertEqual(code, 0, out)
        commit(sub, "t.txt")
        code, out = self.run_cli()
        self.assertEqual(code, 1, out)
        self.assertIn("ahead 1", out)


class Push(Base):
    def test_nested_repo_push_then_recheck_ok(self):
        sub = self.root / "sub"
        subprocess.run(["git", "init", "-q", "-b", "main", str(sub)], check=True)
        commit(sub, "s.txt")
        g(sub, "push", "-q", str(self.bare), "main:refs/heads/sub-main")   # earlier, by hand
        g(self.root, "fetch", "-q", "origin")    # root tracks origin/sub-main; root lacks sub's objects
        commit(sub, "u.txt")
        self.write_map([{"name": "sub", "path": "sub", "branch": "main", "remote": "origin",
                         "remote_branch": "sub-main", "tracking_path": "."}])
        # the root repo (tracking repo) never has sub's commits
        code, out = self.run_cli()
        self.assertEqual(code, 1, out)
        code, out = self.run_cli("--push", "--yes")
        self.assertEqual(code, 0, out)
        self.assertTrue(out.rstrip().endswith("backup: ok"), out)
        self.assertEqual(g(self.bare, "rev-parse", "sub-main"), g(sub, "rev-parse", "main"))
        code, out = self.run_cli()   # a fresh, separate status run agrees
        self.assertEqual(code, 0, out)
        self.assertIn("origin/sub-main pushed (recorded)", out)
        self.assertNotIn("origin/sub-main updated", out)
        commit(sub, "t.txt")
        code, out = self.run_cli()
        self.assertEqual(code, 1, out)
        self.assertIn("ahead 1", out)

    def test_non_object_map_entry_exit_2(self):
        self.map.write_text(json.dumps({"repos": ["root"]}), encoding="utf-8")
        code, out = self.run_cli()
        self.assertEqual(code, 2, out)
        self.assertIn("config error", out)

    def test_push_without_yes_refuses(self):
        commit(self.root, "b.txt")
        g(self.root, "tag", "v9")
        before = self.remote_refs()
        code, out = self.run_cli("--push")
        self.assertEqual(code, 2, out)
        self.assertIn("will push", out)
        self.assertIn("Add --yes", out)
        self.assertEqual(before, self.remote_refs())

    def test_yes_alone_refused(self):
        code, out = self.run_cli("--yes")
        self.assertEqual(code, 2)

    def test_push_yes_pushes_branch_and_tags(self):
        commit(self.root, "b.txt")
        g(self.root, "tag", "v1")
        code, out = self.run_cli("--push", "--yes")
        self.assertEqual(code, 0, out)
        self.assertTrue(out.rstrip().endswith("backup: ok"), out)
        self.assertIn("refs/tags/v1", self.remote_refs())
        self.assertEqual(g(self.bare, "rev-parse", "main"), g(self.root, "rev-parse", "main"))

    def test_first_time_branch_push(self):
        self.write_map([{"name": "root", "path": ".", "branch": "main",
                         "remote": "origin", "remote_branch": "brand-new"}])
        code, out = self.run_cli()
        self.assertEqual(code, 1, out)
        self.assertIn("cannot verify", out)
        code, out = self.run_cli("--push")
        self.assertEqual(code, 2, out)
        self.assertIn("will push root", out)
        self.assertNotIn("refs/heads/brand-new", self.remote_refs())
        code, out = self.run_cli("--push", "--yes")
        self.assertEqual(code, 0, out)
        self.assertEqual(g(self.bare, "rev-parse", "brand-new"), g(self.root, "rev-parse", "main"))
        code, out = self.run_cli()
        self.assertEqual(code, 0, out)

    def test_first_time_push_skipped_when_local_branch_missing(self):
        self.write_map([{"name": "root", "path": ".", "branch": "nobranch",
                         "remote": "origin", "remote_branch": "brand-new"}])
        code, out = self.run_cli("--push", "--yes")
        self.assertEqual(code, 1, out)
        self.assertNotIn("refs/heads/brand-new", self.remote_refs())

    def test_guard_refuses_force_exit_2(self):
        commit(self.root, "b.txt")
        orig = bs.push_plan
        bs.push_plan = lambda root, entry, res: ("origin", ["+refs/heads/main:refs/heads/main"])
        try:
            code, out = self.run_cli("--push", "--yes")
        finally:
            bs.push_plan = orig
        self.assertEqual(code, 2, out)
        self.assertIn("cannot plan push", out)
        self.assertEqual(g(self.bare, "rev-parse", "main"), g(self.root, "rev-parse", "main~1"))

    def test_guard_survives_python_O(self):
        code = """
import importlib.util, sys
s = importlib.util.spec_from_file_location('m', sys.argv[1])
m = importlib.util.module_from_spec(s)
s.loader.exec_module(m)
spec = 'refs/heads/a:refs/heads/a'
bad = [['git', 'push', 'u', spec, '--force'], ['git', 'push', 'u', '-f', spec],
       ['git', 'push', 'u', '--delete', 'x'], ['git', 'push', 'u', ':refs/heads/a'],
       ['git', 'push', 'u', '+' + spec]]
n = 0
for c in bad:
    try:
        m.check_plain_push(c)
    except m.PushRefused:
        n += 1
m.check_plain_push(['git', 'push', 'u', spec, 'refs/tags/v1+b:refs/tags/v1+b'])
print('refused', n, 'debug', __debug__)
"""
        r = subprocess.run([sys.executable, "-O", "-I", "-B", "-c", code, str(SCRIPT)],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.strip(), "refused 5 debug False")

    def test_push_never_forces_or_deletes(self):
        # remote main has a commit the local branch lacks: a plain push must be rejected, not forced
        other = self.base / "other"
        subprocess.run(["git", "clone", "-q", "-b", "main", str(self.bare), str(other)], check=True)
        commit(other, "x.txt")
        g(other, "push", "-q", "origin", "HEAD:main")
        remote_main = g(self.bare, "rev-parse", "main")
        commit(self.root, "b.txt")
        g(self.root, "push", "-q", "origin", "HEAD:refs/heads/keepme")
        code, out = self.run_cli("--push", "--yes")
        self.assertEqual(code, 1, out)
        self.assertIn("push FAILED", out)
        self.assertEqual(remote_main, g(self.bare, "rev-parse", "main"))
        self.assertIn("refs/heads/keepme", self.remote_refs())


if __name__ == "__main__":
    unittest.main()
