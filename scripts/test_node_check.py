"""Tests for scripts/node_check.py. Each test builds a fake Broadsword root under the project's scratch folder and
removes it afterwards, so the real docs/ and scratch/ are never touched.

  python -I -B scripts/test_node_check.py -v

BS_NODE_CHECK_SCRIPT=<path> runs the same tests against another copy of the script (used to prove the tests can fail).
"""
import sys

sys.dont_write_bytecode = True

import importlib.util
import io
import os
import pathlib
import shutil
import tempfile
import unittest
from contextlib import redirect_stdout

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = pathlib.Path(os.environ.get("BS_NODE_CHECK_SCRIPT") or HERE / "node_check.py")
SCRATCH = HERE.parent / "scratch"
F = "`" * 3


def load(path):
    spec = importlib.util.spec_from_file_location("node_check_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


nc = load(SCRIPT)


def block(cid, cmd, rule):
    return f"{F}check {cid}\n{cmd}\nPASS: {rule}\n{F}\n"


class Parse(unittest.TestCase):
    def test_reads_checks_with_line_numbers_and_ignores_other_fences(self):
        text = (
            "# Node 1.2\n\n" + F + "sh\necho not a check\n" + F + "\n\n"
            + block("c1", "python -I a.py", "last line == ok")
            + "text\n"
            + block("c2", "broadsword test crash-sim", "contains 164 passed")
        )
        checks, problems = nc.parse(text)
        self.assertEqual(problems, [])
        self.assertEqual([c["id"] for c in checks], ["c1", "c2"])
        self.assertEqual(checks[0]["command"], "python -I a.py")
        self.assertEqual(checks[0]["rule"], "last line == ok")
        self.assertEqual((checks[0]["line"], checks[1]["line"]), (7, 12))

    def test_extra_lines_in_a_block_are_ignored(self):
        checks, problems = nc.parse(f"{F}check c1\ncmd\nPASS: free text\nnotes here\n{F}\n")
        self.assertEqual((len(checks), problems), (1, []))

    def test_malformed_blocks_are_flagged_with_their_line(self):
        cases = {
            "no id": (f"x\n{F}check\ncmd\nPASS: r\n{F}\n", 2),
            "two ids": (f"x\n{F}check a b\ncmd\nPASS: r\n{F}\n", 2),
            "empty block": (f"\n\n{F}check a\n{F}\n", 3),
            "no PASS line": (f"{F}check a\ncmd\nnotes\n{F}\n", 1),
            "empty rule": (f"{F}check a\ncmd\nPASS:   \n{F}\n", 1),
            "command only": (f"{F}check a\ncmd\n{F}\n", 1),
            "never closed": (f"a\n{F}check a\ncmd\nPASS: r\n", 2),
        }
        for name, (text, line) in cases.items():
            with self.subTest(name):
                checks, problems = nc.parse(text)
                self.assertEqual(checks, [])
                self.assertEqual([p[0] for p in problems], [line], problems)

    def test_duplicate_id_is_flagged_and_first_one_kept(self):
        checks, problems = nc.parse(block("a", "x", "r") + block("a", "y", "r"))
        self.assertEqual([c["command"] for c in checks], ["x"])
        self.assertEqual(len(problems), 1)
        self.assertEqual(problems[0][0], 5)

    def test_malformed_block_does_not_hide_the_next_good_one(self):
        checks, problems = nc.parse(f"{F}check a\n{F}\n" + block("b", "x", "r"))
        self.assertEqual([c["id"] for c in checks], ["b"])
        self.assertEqual(len(problems), 1)


class Judge(unittest.TestCase):
    def test_last_line_rule(self):
        self.assertIs(nc.judge("last line == ok", "a\nok\n\n"), True)
        self.assertIs(nc.judge("last line == ok", "ok\nbad\n"), False)
        self.assertIs(nc.judge("last line == ok", ""), False)

    def test_contains_rule(self):
        self.assertIs(nc.judge("contains 164 passed", "x 164 passed y"), True)
        self.assertIs(nc.judge("contains 164 passed", "163 passed"), False)

    def test_free_text_is_not_judged(self):
        self.assertIsNone(nc.judge("the picture looks right", "anything"))
        self.assertIsNone(nc.judge("last line equals ok", "ok"))


class Refusals(unittest.TestCase):
    def refused(self, cmd):
        with self.assertRaises(nc.Refused, msg=cmd):
            nc.build_argv(cmd, SCRATCH / "nowhere")

    def test_other_programs_are_refused(self):
        for cmd in ("cargo test", "rm -rf scratch", "python script.py", "python3 -I x.py", "powershell -c x", "git status"):
            self.refused(cmd)

    def test_shell_syntax_is_refused(self):
        for cmd in ("python -I a.py; rm x", "python -I a.py && b", "python -I a.py | tail", "python -I a.py < in",
                    "python -I a.py `x`", "python -I a.py $HOME", "broadsword test x > a > b", "python -I a.py >> log"):
            self.refused(cmd)

    def test_allowed_commands_and_trailing_redirects(self):
        root = SCRATCH / "nowhere"
        self.assertEqual(nc.build_argv("python -I a.py x", root)[1:], ["-I", "a.py", "x"])
        self.assertEqual(nc.build_argv("broadsword test crash-sim > scratch/o.txt 2>&1", root)[1:], ["test", "crash-sim"])
        self.assertEqual(nc.build_argv("build/bin/broadsword.exe check p 2>&1", root)[1:], ["check", "p"])

    def test_job_detection(self):
        self.assertTrue(nc.in_job({"BROADSWORD_JOB": "20261120-1"}))
        self.assertTrue(nc.in_job({"CARGO_TARGET_DIR": "build"}))
        self.assertFalse(nc.in_job({"PATH": "x"}))


class Cli(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(exist_ok=True)
        self.root = pathlib.Path(tempfile.mkdtemp(prefix="test-node-check-", dir=SCRATCH))
        (self.root / "docs" / "nodes").mkdir(parents=True)
        (self.root / "emit.py").write_text("import sys\nprint('line one')\nprint(sys.argv[1])\n", encoding="utf-8")
        (self.root / "docs" / "nodes" / "1.2.md").write_text(
            "# 1.2\n"
            + block("pass-last", "python -I emit.py ok", "last line == ok")
            + block("fail-last", "python -I emit.py nope", "last line == ok")
            + block("has", "python -I emit.py x", "contains line one")
            + block("hand", "python -I emit.py x", "it looks fine")
            + block("redir", "python -I emit.py ok > scratch/ignored.txt 2>&1", "last line == ok"),
            encoding="utf-8",
        )

    def tearDown(self):
        def onerror(func, p, _exc):
            os.chmod(p, 0o700)
            func(p)
        shutil.rmtree(self.root, onerror=onerror)

    def cli(self, *args, env=None):
        buf = io.StringIO()
        with redirect_stdout(buf):
            code = nc.main(["1.2", "--root", str(self.root), *args], env={} if env is None else env)
        return code, buf.getvalue()

    def test_list_prints_every_check(self):
        code, out = self.cli("--list")
        self.assertEqual(code, 0)
        for s in ("pass-last", "command: python -I emit.py ok", "PASS: last line == ok", "5 checks, 0 problems"):
            self.assertIn(s, out)

    def test_list_flags_a_malformed_block_and_exits_1(self):
        p = self.root / "docs" / "nodes" / "1.2.md"
        p.write_text(p.read_text(encoding="utf-8") + f"\n{F}check broken\ncmd\n{F}\n", encoding="utf-8")
        code, out = self.cli("--list")
        self.assertEqual(code, 1)
        self.assertRegex(out, r"1\.2\.md line \d+: check broken")
        self.assertIn("1 problems", out)

    def test_run_one_saves_output_and_judges_it(self):
        code, out = self.cli("--run", "pass-last")
        self.assertEqual(code, 0, out)
        self.assertIn("result: PASS", out)
        saved = (self.root / "scratch" / "node-checks" / "1.2" / "pass-last.txt").read_text(encoding="utf-8")
        self.assertEqual(saved.split(), ["line", "one", "ok"])

    def test_failed_mechanical_rule_exits_1(self):
        code, out = self.cli("--run", "fail-last")
        self.assertEqual(code, 1)
        self.assertIn("result: FAIL", out)

    def test_free_text_rule_is_left_to_the_human(self):
        code, out = self.cli("--run", "hand")
        self.assertEqual(code, 0)
        self.assertIn("judge by hand", out)
        self.assertIn("rule: it looks fine", out)

    def test_run_all_runs_every_check_and_fails_if_any_fails(self):
        code, out = self.cli("--run", "--all")
        self.assertEqual(code, 1)
        self.assertEqual(sum(l.startswith("== ") for l in out.splitlines()), 5)
        self.assertEqual((out.count("result: PASS"), out.count("result: FAIL")), (3, 1))
        self.assertTrue((self.root / "scratch" / "node-checks" / "1.2" / "redir.txt").is_file())

    def test_refused_inside_a_job_and_nothing_runs(self):
        for env in ({"BROADSWORD_JOB": "x"}, {"CARGO_TARGET_DIR": "build"}):
            code, out = self.cli("--run", "--all", env=env)
            self.assertEqual(code, 2)
            self.assertIn("inside a Broadsword job", out)
        self.assertFalse((self.root / "scratch" / "node-checks").exists())

    def test_one_refused_command_stops_the_whole_run_before_any_runs(self):
        p = self.root / "docs" / "nodes" / "1.2.md"
        p.write_text(p.read_text(encoding="utf-8") + block("evil", "cargo test", "contains ok"), encoding="utf-8")
        code, out = self.cli("--run", "--all")
        self.assertEqual(code, 2)
        self.assertIn("refused check evil", out)
        self.assertFalse((self.root / "scratch" / "node-checks").exists())

    def test_bad_arguments_exit_2(self):
        self.assertEqual(self.cli("--run", "nope")[0], 2)
        self.assertEqual(self.cli("--run")[0], 2)
        self.assertEqual(self.cli("--run", "x", "--all")[0], 2)
        buf = io.StringIO()
        with redirect_stdout(buf):
            self.assertEqual(nc.main(["../x", "--root", str(self.root), "--list"], env={}), 2)
            self.assertEqual(nc.main(["9.9", "--root", str(self.root), "--list"], env={}), 2)


if __name__ == "__main__":
    unittest.main()
