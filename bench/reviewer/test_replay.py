"""replay.py without any model call: the arms it builds, and its refusals before the first paid call.

Run: python -I -B -m unittest discover -s bench/reviewer -p "test_replay.py" -v
"""
import os, pathlib, subprocess, sys, tempfile, unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import replay  # noqa: E402


def run(args, env_extra):
    env = dict(os.environ, **env_extra)
    env.pop("BENCH_CLAUDE", None)
    env["BENCH_CLAUDE"] = env_extra.get("BENCH_CLAUDE", "no-such-claude-program-for-test")
    return subprocess.run([sys.executable, "-I", "-B", str(HERE / "replay.py"), *args], capture_output=True, text=True, env=env)


class Arms(unittest.TestCase):
    def test_arm_a_is_the_gate_layout_and_b_adds_the_rules_inside_the_block(self):
        a = replay.arms()
        head, block, tail = replay.macro("prompt_head"), replay.macro("gate_block"), replay.macro("prompt_tail")
        self.assertEqual(a["A"], head + "\n\n" + block + "\n\n" + tail)
        rules = replay.RULES.read_text(encoding="utf-8").rstrip("\n")
        self.assertEqual(a["B"], a["A"].replace(block + "\n\n", block + "\n" + rules + "\n\n", 1))
        self.assertTrue(a["B"].index(rules) < a["B"].index("Answer in exactly this format"))
        self.assertNotIn(rules, a["A"])

    def test_show_prints_both_arms_and_calls_nothing(self):
        p = run(["--show"], {})
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("arm A:", p.stdout)
        self.assertIn("arm B:", p.stdout)


class Refusals(unittest.TestCase):
    def test_missing_prompt_file_stops_before_any_call(self):
        with tempfile.TemporaryDirectory() as t:
            out = pathlib.Path(t) / "out"
            p = run(["A", "1", t, str(out), "q-does-not-exist"], {"BENCH_CWD": t})
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("missing or empty prompt file", p.stderr)
            self.assertFalse(out.exists(), "nothing may be written before the inputs are checked")

    def test_empty_prompt_file_stops_before_any_call(self):
        with tempfile.TemporaryDirectory() as t:
            (pathlib.Path(t) / "q-x.prompt.txt").write_bytes(b"")
            p = run(["A", "1", t, str(pathlib.Path(t) / "out"), "q-x"], {"BENCH_CWD": t})
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("missing or empty prompt file", p.stderr)

    def test_cwd_inside_the_project_is_refused(self):
        with tempfile.TemporaryDirectory() as t:
            p = run(["A", "1", t, str(pathlib.Path(t) / "out"), "q-x"], {"BENCH_CWD": str(HERE)})
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("outside the project", p.stderr)

    def test_unknown_arm_is_refused(self):
        with tempfile.TemporaryDirectory() as t:
            p = run(["C", "1", t, str(pathlib.Path(t) / "out"), "q-x"], {"BENCH_CWD": t})
            self.assertNotEqual(p.returncode, 0)
            self.assertIn("unknown arm", p.stderr)


if __name__ == "__main__":
    unittest.main()
