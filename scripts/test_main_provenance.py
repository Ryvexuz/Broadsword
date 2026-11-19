#!/usr/bin/env python3
"""Tests for scripts/main_provenance.py. Run: python -I -B scripts/test_main_provenance.py -v

Every test builds a throw-away Broadsword-shaped root plus one project repo under the main checkout's scratch folder
(scratch_dir from broadsword.toml, never the system temp folder) and removes it afterwards. The script under test runs as
a real subprocess, so exit codes and printed lines are what a user sees. MAIN_PROVENANCE_SCRIPT overrides the script path
(used to prove the tests fail against a deliberately broken copy).
"""
import os
import shutil
import subprocess
import sys
import tomllib
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.environ.get("MAIN_PROVENANCE_SCRIPT") or os.path.join(HERE, "main_provenance.py")


def main_checkout_scratch():
    """<main checkout>/<scratch_dir>: the main checkout is the parent of Git's common dir, so a worktree finds it too."""
    here_root = os.path.dirname(HERE)
    out = subprocess.run(["git", "-C", here_root, "rev-parse", "--path-format=absolute", "--git-common-dir"],
                         capture_output=True, text=True, check=True).stdout.strip()
    main_root = os.path.dirname(out)
    with open(os.path.join(main_root, "broadsword.toml"), "rb") as f:
        cfg = tomllib.load(f)
    d = os.path.join(main_root, cfg["scratch_dir"])
    if not os.path.isdir(d):
        raise RuntimeError(f"scratch folder {d} does not exist")
    return d


def git(repo, *args):
    p = subprocess.run(["git", "-C", repo, "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false",
                        *args], capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(f"git {args} failed: {p.stderr}")
    return p.stdout.strip()


def commit(repo, files, msg):
    """files: {path: text or None (delete)}; returns the full sha."""
    for path, text in files.items():
        full = os.path.join(repo, path)
        if text is None:
            git(repo, "rm", "-q", "--", path)
            continue
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8", newline="\n") as f:
            f.write(text)
        git(repo, "add", "--", path)
    git(repo, "commit", "-q", "-m", msg)
    return git(repo, "rev-parse", "HEAD")


def entry_text(**kv):
    base = dict(id="q-x", project="broadsword", status="merged", main_branch="main", base_head="", main_sha="")
    base.update(kv)
    return "".join(f"{k} = {v}\n" for k, v in base.items())


class Scratch(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.dir = tempfile.mkdtemp(prefix="mp-test-", dir=main_checkout_scratch())
        self.root = os.path.join(self.dir, "root")
        os.makedirs(os.path.join(self.root, "state", "queue"))
        os.makedirs(os.path.join(self.root, "docs"))
        os.makedirs(os.path.join(self.root, "projects"))
        with open(os.path.join(self.root, "broadsword.toml"), "w") as f:
            f.write('state_dir = "state"\nprojects_dir = "projects"\n')
        git(self.root, "init", "-q", "-b", "main")
        self.c0 = commit(self.root, {"src/a.rs": "a\n"}, "base")
        git(self.root, "tag", "base")
        self.allow = os.path.join(self.root, "docs", "direct-merges.txt")
        self.write_allow("")

    def tearDown(self):
        def onerr(func, path, exc):  # git object files are read-only on Windows
            os.chmod(path, 0o700)
            func(path)
        shutil.rmtree(self.dir, onerror=onerr)
        self.assertFalse(os.path.exists(self.dir))

    def write_allow(self, text):
        with open(self.allow, "w", encoding="utf-8") as f:
            f.write(text)

    def entry(self, name, **kv):
        with open(os.path.join(self.root, "state", "queue", name + ".entry"), "w", encoding="utf-8") as f:
            f.write(entry_text(id=name, **kv))

    def run_script(self, *args, since="base"):
        argv = [sys.executable, "-I", "-B", SCRIPT, "--root", self.root, *([f"--since={since}"] if since else []), *args]
        p = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8")
        return p.returncode, p.stdout, p.stderr

    def four(self):
        """One queued, one listed, one records-only and one unaccounted commit after `base`."""
        self.queued = commit(self.root, {"src/q.rs": "q\n"}, "queued change")
        self.entry("q-1", base_head=self.c0, main_sha=self.queued)
        self.listed = commit(self.root, {"src/l.rs": "l\n"}, "listed change")
        self.write_allow(f"broadsword | {self.listed[:12]} | Lazlo | testing | test source line 1\n")
        self.records = commit(self.root, {"NOTES.md": "n\n", "evidence/e.txt": "e\n", "docs/d.txt": "d\n"}, "records only")
        self.bad = commit(self.root, {"src/bad.rs": "bad\n"}, "sneaked in")


class FourKinds(Scratch):
    def test_one_of_each_exit_1_and_only_the_unaccounted_one_flagged(self):
        self.four()
        code, out, err = self.run_script()
        self.assertEqual(code, 1, out + err)
        lines = out.splitlines()
        unacc = [l for l in lines if l.startswith("UNACCOUNTED")]
        self.assertEqual(len(unacc), 1, out)
        self.assertIn(self.bad[:12], unacc[0])
        self.assertIn("sneaked in", unacc[0])
        self.assertIn("src/bad.rs", unacc[0])
        listed = [l for l in lines if l.startswith("LISTED")]
        self.assertEqual(len(listed), 1)
        self.assertIn(self.listed[:12], listed[0])
        self.assertIn("Lazlo", listed[0])
        self.assertIn("testing", listed[0])
        self.assertIn("test source line 1", listed[0])
        rec = [l for l in lines if l.startswith("RECORDS")]
        self.assertEqual(len(rec), 1)
        self.assertIn(self.records[:12], rec[0])
        self.assertIn("queue 1, listed 1, records 1, UNACCOUNTED 1", out)
        self.assertIn("not failures", out)  # the output says records-only commits are not failures
        for sha in (self.queued, self.listed, self.records):
            self.assertNotIn("UNACCOUNTED broadsword " + sha[:12], out)

    def test_exit_0_when_nothing_is_unaccounted(self):
        self.four()
        self.write_allow(self.read_allow() + f"broadsword | {self.bad[:10]} | Lazlo | testing | src 2\n")
        code, out, err = self.run_script()
        self.assertEqual(code, 0, out + err)
        self.assertIn("result: 0 unaccounted", out)

    def read_allow(self):
        with open(self.allow, encoding="utf-8") as f:
            return f.read()


class Rules(Scratch):
    def test_whole_branch_range_of_one_queue_entry_is_accounted(self):
        c1 = commit(self.root, {"src/1.rs": "1\n"}, "task commit 1")
        c2 = commit(self.root, {"src/2.rs": "2\n"}, "task commit 2")
        self.entry("q-2", base_head=self.c0, main_sha=c2)
        code, out, err = self.run_script()
        self.assertEqual(code, 0, out + err)
        self.assertIn("queue 2", out)

    def test_rejected_entry_does_not_account_for_a_commit(self):
        c1 = commit(self.root, {"src/1.rs": "1\n"}, "was rejected")
        self.entry("q-3", status="rejected", base_head=self.c0, main_sha=c1)
        code, out, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("UNACCOUNTED broadsword " + c1[:12], out)

    def test_merged_check_failed_entry_still_counts_as_landed(self):
        c1 = commit(self.root, {"src/1.rs": "1\n"}, "landed, check failed after")
        self.entry("q-4", status="merged_check_failed", base_head=self.c0, main_sha=c1)
        code, out, err = self.run_script()
        self.assertEqual(code, 0, out + err)

    def test_mixed_commit_with_one_code_file_is_not_records(self):
        c = commit(self.root, {"NOTES.md": "n\n", "src/sneak.rs": "s\n"}, "notes plus code")
        code, out, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("UNACCOUNTED broadsword " + c[:12], out)

    def test_nested_md_is_not_records_but_root_md_is(self):
        c = commit(self.root, {"src/README.md": "r\n"}, "nested md")
        code, out, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("UNACCOUNTED broadsword " + c[:12], out)

    def test_moving_code_into_docs_is_not_hidden_as_records(self):
        c = commit(self.root, {"src/a.rs": None, "docs/a.rs": "a\n"}, "move code into docs")
        code, out, _ = self.run_script()
        self.assertEqual(code, 1, out)
        self.assertIn("UNACCOUNTED broadsword " + c[:12], out)
        self.assertIn("src/a.rs", out)

    def test_commit_before_since_is_not_checked(self):
        commit(self.root, {"src/early.rs": "e\n"}, "early")
        git(self.root, "tag", "later")
        code, out, err = self.run_script(since="later")
        self.assertEqual(code, 0, out + err)
        self.assertIn("0 commits", out)

    def test_project_repo_main_branch_master_and_missing_since_checks_all_history(self):
        proj = os.path.join(self.root, "projects", "proj")
        os.makedirs(proj)
        git(proj, "init", "-q", "-b", "master")
        first = commit(proj, {"src/x.rs": "x\n"}, "proj first")
        second = commit(proj, {"src/y.rs": "y\n"}, "proj second")
        self.entry("q-5", project="proj", main_branch="master", base_head=first, main_sha=second)
        code, out, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("== proj (master)", out)
        self.assertIn("does not exist here", out)
        self.assertIn("UNACCOUNTED proj " + first[:12], out)  # the first commit predates every queue entry
        self.assertNotIn("UNACCOUNTED proj " + second[:12], out)

    def test_allow_list_entry_is_per_repo(self):
        c = commit(self.root, {"src/z.rs": "z\n"}, "listed for another repo")
        proj = os.path.join(self.root, "projects", "proj")
        os.makedirs(proj)
        git(proj, "init", "-q", "-b", "main")
        commit(proj, {"README.md": "r\n"}, "proj records")
        self.write_allow(f"proj | {c[:12]} | Lazlo | testing | src\n")
        code, out, _ = self.run_script()
        self.assertEqual(code, 1)
        self.assertIn("UNACCOUNTED broadsword " + c[:12], out)
        self.assertIn("WARNING allow-list line 1", out)  # that hash is not a commit in proj

    def test_allow_listed_hash_on_another_branch_warns(self):
        git(self.root, "checkout", "-q", "-b", "side", "base")
        off = commit(self.root, {"src/side.rs": "s\n"}, "side branch only")
        git(self.root, "checkout", "-q", "main")
        self.write_allow(f"broadsword | {off[:12]} | Lazlo | testing | src\n")
        code, out, _ = self.run_script()
        self.assertEqual(code, 0, out)
        self.assertIn("WARNING allow-list line 1", out)
        self.assertIn("is not a commit on broadsword main", out)

    def test_read_only_nothing_in_the_repo_changes(self):
        self.four()
        def snap():
            s = {}
            for d, _, fs in os.walk(self.root):
                for n in fs:
                    p = os.path.join(d, n)
                    st = os.stat(p)
                    s[p] = (st.st_size, st.st_mtime_ns)
            return s
        before, refs = snap(), git(self.root, "for-each-ref")
        self.run_script()
        self.assertEqual(snap(), before)
        self.assertEqual(git(self.root, "for-each-ref"), refs)


class BadInput(Scratch):
    def assert_2(self, *args, since="base", expect=""):
        code, out, err = self.run_script(*args, since=since)
        self.assertEqual(code, 2, out + err)
        self.assertIn(expect, err)

    def test_unknown_since(self):
        self.assert_2(since="no-such-tag", expect="not a commit or tag")

    def test_option_like_since(self):
        self.assert_2(since="--all", expect="option-like")

    def test_missing_since_argument(self):
        code, _, _ = self.run_script(since=None)
        self.assertEqual(code, 2)

    def test_since_not_an_ancestor(self):
        git(self.root, "checkout", "-q", "-b", "other", "base")
        other = commit(self.root, {"src/o.rs": "o\n"}, "other branch")
        git(self.root, "tag", "othertag")
        git(self.root, "checkout", "-q", "main")
        commit(self.root, {"src/m.rs": "m\n"}, "on main")
        self.assert_2(since="othertag", expect="not an ancestor")

    def test_allow_list_line_with_missing_field(self):
        self.write_allow("broadsword | abcdef1 | Lazlo | no source\n")
        self.assert_2(expect="all five non-empty")

    def test_allow_list_bad_hash(self):
        self.write_allow("broadsword | xyz | Lazlo | why | src\n")
        self.assert_2(expect="not a hex commit hash")

    def test_placeholder_approvers_are_refused_naming_the_line(self):
        for who in ("NOT RECORDED", "not recorded (orchestrator; nothing found)", "?", "TBD", "Unknown", "none", "None.", "n/a", "-", "unapproved"):
            with self.subTest(who=who):
                self.write_allow(f"# comment\nbroadsword | abcdef1 | {who} | why | src\n")
                code, out, err = self.run_script()
                self.assertEqual(code, 2, who + out + err)
                self.assertIn("direct-merges.txt:2", err)
                self.assertIn("placeholder", err)

    def test_real_approvers_are_accepted(self):
        c = commit(self.root, {"src/z.rs": "z\n"}, "approved")
        for who in ("Lazlo", "Nonesuch Person", "Unknownson", "general rule: Lazlo's \"take the file\""):
            with self.subTest(who=who):
                self.write_allow(f"broadsword | {c[:12]} | {who} | why | src\n")
                code, out, err = self.run_script()
                self.assertEqual(code, 0, out + err)

    def test_git_failure_message_keeps_gits_stderr(self):
        commit(self.root, {"src/t.rs": "t" + chr(10)}, "tree about to go missing")
        tree = git(self.root, "rev-parse", "HEAD^{tree}")
        obj = os.path.join(self.root, ".git", "objects", tree[:2], tree[2:])
        os.chmod(obj, 0o700)
        os.remove(obj)  # `git show` of that commit now fails with its own explanation
        code, out, err = self.run_script()
        self.assertEqual(code, 2, out + err)
        self.assertRegex(err, r"failed in .*\(exit \d+\): \S")
        self.assertIn(tree[:6], err)  # git's own stderr names the missing object

    def test_allow_list_unknown_repo(self):
        self.write_allow("nosuch | abcdef1 | Lazlo | why | src\n")
        self.assert_2(expect="is not one of")

    def test_allow_list_missing_file(self):
        os.remove(self.allow)
        self.assert_2(expect="allow-list")

    def test_malformed_entry(self):
        with open(os.path.join(self.root, "state", "queue", "q-9.entry"), "w") as f:
            f.write("garbage without equals\n")
        self.assert_2(expect="key = value")

    def test_merged_entry_without_main_sha(self):
        self.entry("q-8", main_sha="")
        self.assert_2(expect="main_sha")

    def test_merged_entry_whose_commit_is_not_in_the_repo(self):
        self.entry("q-7", main_sha="1234567890abcdef1234567890abcdef12345678")
        self.assert_2(expect="not in the broadsword repo")

    def test_missing_queue_folder(self):
        shutil.rmtree(os.path.join(self.root, "state", "queue"))
        self.assert_2(expect="queue folder not found")


if __name__ == "__main__":
    unittest.main()
