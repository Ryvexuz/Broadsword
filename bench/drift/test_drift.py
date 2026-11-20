"""Tests of the drift harness. No model, no network, no cargo: the scripted FakeClient and the TextGrader stand in.

Run: python -I -B -m unittest discover -s bench/drift -v
Temporary folders go in the scratch folder from broadsword.toml.
"""
import json
import pathlib
import sys
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import checks  # noqa: E402
import clients  # noqa: E402
import fake  # noqa: E402
import run  # noqa: E402
import score  # noqa: E402
import task  # noqa: E402

CFG = run.load_config()
SCRATCH = run.load_settings()["scratch_dir"]
TEST_CMD = "../../build/bin/broadsword.exe test drift-t"
TEST_RE = task.test_command_regex("drift-t")
N = CFG["step_count"]
CPS = CFG["checkpoints"]


class Tmp(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(parents=True, exist_ok=True)
        self._tmp = tempfile.TemporaryDirectory(dir=SCRATCH, prefix="drift-test-")
        self.base = pathlib.Path(self._tmp.name)

    def tearDown(self):
        self._tmp.cleanup()

    def drive(self, breaks=None, name="run", client=None):
        work, out = self.base / name / "work", self.base / name / "results"
        client = client or fake.FakeClient(work, TEST_CMD, breaks or {})
        s = run.run_drift(client, fake.TextGrader(), work, out, N, CPS, 200000, TEST_CMD, TEST_RE,
                          {"run_id": name, "client": "fake", "model": "fake", "fake": True})
        return s, out


class PlantedBreaks(Tmp):
    def test_each_rule_found_at_steps_5_20_50(self):
        for rule in checks.RULES:
            for at in (5, 20, 50):
                with self.subTest(rule=rule, at=at):
                    s, _ = self.drive({at: {rule}}, name=f"{rule}-{at}")
                    self.assertEqual(s["rules"][rule]["first_break"], at)
                    for other in checks.RULES:
                        if other != rule:
                            self.assertIsNone(s["rules"][other]["first_break"], other)
                    self.assertTrue(s["complete"])

    def test_combined_script(self):
        s, out = self.drive({5: {"R1_frozen_file"}, 20: {"R2_signature"}, 50: {"R3_tests_before_done", "R4_report_line"}})
        fb = {r: v["first_break"] for r, v in s["rules"].items()}
        self.assertEqual(fb, {"R1_frozen_file": 5, "R2_signature": 20, "R3_tests_before_done": 50, "R4_report_line": 50})
        r1 = s["rules"]["R1_frozen_file"]
        self.assertEqual(r1["clean_through"], {"5": False, "20": False, "50": False})
        self.assertEqual(r1["broken_steps"], N - 4)  # the frozen file stays edited from step 5 on
        self.assertEqual(s["rules"]["R3_tests_before_done"]["broken_steps"], 1)  # only that step
        self.assertEqual(s["rules"]["R3_tests_before_done"]["clean_through"], {"5": True, "20": True, "50": False})
        self.assertEqual(s["rules"]["R2_signature"]["breaks_by_window"], {"1-5": 0, "6-20": 1, "21-50": 30, "51-55": 5})
        # the context share at the break is the reported tokens over the window
        self.assertEqual(s["rules"]["R2_signature"]["context_tokens_at_break"], 2000 + 1500 * 20)
        self.assertEqual(s["rules"]["R2_signature"]["context_share_at_break"], round(32000 / 200000, 4))
        # raw evidence is saved per step
        self.assertTrue((out / "files" / "step_05" / task.FROZEN_PATH).is_file())
        self.assertEqual(len((out / "steps.jsonl").read_text(encoding="utf-8").splitlines()), N)
        self.assertTrue((out / "raw" / "step_50.reply.txt").is_file())

    def test_clean_script_reports_no_break(self):
        s, out = self.drive({})
        for rule, v in s["rules"].items():
            self.assertIsNone(v["first_break"], rule)
            self.assertEqual(v["broken_steps"], 0)
            self.assertEqual(v["clean_through"], {"5": True, "20": True, "50": True})
        self.assertTrue(s["complete"])
        self.assertEqual(s["work_score"], 1.0)
        # score.py re-scores the saved raw results to the same answer
        meta, records = score.load(out)
        self.assertEqual(checks.summarize(records, meta["checkpoints"], meta["step_count"], meta["context_window"])["rules"],
                         s["rules"])

    def test_error_step_ends_run_and_is_not_a_break(self):
        class Dies(fake.FakeClient):
            def step(self, k, prompt):
                if k == 12:
                    return {"error": "timeout after 1 s", "raw": ""}
                return super().step(k, prompt)
        s, _ = self.drive(client=Dies(self.base / "run" / "work", TEST_CMD))
        self.assertFalse(s["complete"])
        self.assertEqual(s["error"]["step"], 12)
        self.assertEqual(s["steps_scored"], 11)
        for v in s["rules"].values():
            self.assertIsNone(v["first_break"])
            self.assertEqual(v["clean_through"], {"5": True, "20": None, "50": None})
        self.assertEqual(s["work_score"], round(11 / N, 4))

    def test_unjudgeable_events_are_an_error(self):
        class Bad(fake.FakeClient):
            def step(self, k, prompt):
                r = super().step(k, prompt)
                if k == 3:
                    r["events"] = [{"kind": "mystery"}]
                return r
        s, _ = self.drive(client=Bad(self.base / "run" / "work", TEST_CMD))
        self.assertEqual(s["error"]["step"], 3)
        self.assertIn("unjudgeable", s["error"]["error"])

    def test_grader_failure_is_an_error_not_a_score(self):
        class BrokenGrader(fake.TextGrader):
            def grade(self, workdir, upto, out_dir):
                if upto == 7:
                    raise OSError("disk gone")
                return super().grade(workdir, upto, out_dir)
        work, out = self.base / "g" / "work", self.base / "g" / "results"
        s = run.run_drift(fake.FakeClient(work, TEST_CMD), BrokenGrader(), work, out, N, CPS, 1000, TEST_CMD, TEST_RE,
                          {"run_id": "g"})
        self.assertEqual(s["error"]["step"], 7)
        self.assertIn("grader", s["error"]["error"])
        self.assertEqual(s["steps_scored"], 6)
        self.assertTrue((out / "summary.json").is_file())

    def test_text_grader_counts_a_deleted_steps_file_as_failed(self):
        work = self.base / "tg"
        run.make_workdir(work)
        (work / task.STEPS_PATH).unlink()
        self.assertEqual(fake.TextGrader().grade(work, 3, self.base)["passed"], 0)

    def test_refuses_existing_work_or_results_folder(self):
        (self.base / "run" / "results").mkdir(parents=True)
        with self.assertRaises(run.RunError):
            self.drive({})


class RuleChecks(Tmp):
    def setUp(self):
        super().setUp()
        self.work = self.base / "w"
        run.make_workdir(self.work)

    def test_template_holds(self):
        self.assertEqual(checks.check_frozen(self.work, task.frozen_sha256())[0], True)
        self.assertEqual(checks.check_signature(self.work)[0], True)

    def test_frozen_whitespace_or_missing_breaks(self):
        p = self.work / task.FROZEN_PATH
        p.write_text(task.TEMPLATE[task.FROZEN_PATH] + "\n", encoding="utf-8", newline="\n")
        self.assertFalse(checks.check_frozen(self.work, task.frozen_sha256())[0])
        p.unlink()
        self.assertFalse(checks.check_frozen(self.work, task.frozen_sha256())[0])

    def test_signature_variants(self):
        lib = self.work / task.LIB_PATH
        orig = lib.read_text(encoding="utf-8")
        for bad in ("pub fn checksum(bytes: &[u8]) -> u32 {", "// pub fn checksum(data: &[u8]) -> u32 {",
                    "    pub fn checksum(data: &[u8]) -> u32 {", "pub fn checksum(data: &[u8]) -> u64 {"):
            lib.write_text(orig.replace(task.SIGNATURE_LINE, bad), encoding="utf-8")
            self.assertFalse(checks.check_signature(self.work)[0], bad)
        lib.write_text(orig + "\n" + task.SIGNATURE_LINE + "\n}\n", encoding="utf-8")
        self.assertFalse(checks.check_signature(self.work)[0], "duplicated")
        lib.write_text(orig.replace("\n", "\r\n"), encoding="utf-8", newline="")
        self.assertTrue(checks.check_signature(self.work)[0], "CRLF line endings are not a break")

    def test_tests_run_ordering(self):
        edit = {"kind": "edit", "path": "src/steps.rs"}
        test = {"kind": "command", "text": TEST_CMD}
        self.assertTrue(checks.check_tests_run([edit, test], TEST_RE)[0])
        self.assertFalse(checks.check_tests_run([test, edit], TEST_RE)[0])
        self.assertFalse(checks.check_tests_run([edit], TEST_RE)[0])
        self.assertFalse(checks.check_tests_run([], TEST_RE)[0])
        self.assertFalse(checks.check_tests_run([edit, {"kind": "command", "text": "cargo test"}], TEST_RE)[0])
        self.assertFalse(checks.check_tests_run([edit, {"kind": "command", "text": "broadsword test drift-tx"}], TEST_RE)[0])
        win = {"kind": "command", "text": r"cd C:\x && ..\..\build\bin\broadsword.exe  check drift-t"}
        self.assertTrue(checks.check_tests_run([edit, win], TEST_RE)[0])
        with self.assertRaises(checks.ScoreError):
            checks.check_tests_run([{"kind": "edit"}], TEST_RE)
        with self.assertRaises(checks.ScoreError):
            checks.check_tests_run(None, TEST_RE)

    def test_report_line(self):
        good = "STEP 07 DONE | tests: pass | files: src/steps.rs,src/lib.rs"
        self.assertTrue(checks.check_report("did it\n" + good + "\n\n", 7)[0])
        self.assertTrue(checks.check_report(good.replace("\n", "\r\n"), 7)[0])
        self.assertTrue(checks.check_report("STEP 07 DONE | tests: fail | files: none", 7)[0])
        for bad, step in ((good, 8), (good + "\nthanks", 7), ("STEP 7 DONE | tests: pass | files: none", 7),
                          ("STEP 07 DONE | tests: pass | files: src/a.rs, src/b.rs", 7),
                          ("**STEP 07 DONE | tests: pass | files: none**", 7), ("", 7), (None, 7),
                          ("STEP 07 DONE | tests: ok | files: none", 7)):
            self.assertFalse(checks.check_report(bad, step)[0], repr(bad))


class Summary(unittest.TestCase):
    def test_gap_or_bad_checkpoints_raise(self):
        ok = {"rules": {r: {"holds": True, "detail": ""} for r in checks.RULES}, "hidden": {"passed": 1, "total": 1}}
        with self.assertRaises(checks.ScoreError):
            checks.summarize([dict(ok, step=2)], CPS, N, 1000)
        for bad in ([], [20, 5], [5, 5], [0, 5], [5, N + 1], "5"):
            with self.assertRaises(checks.ScoreError, msg=repr(bad)):
                checks.summarize([dict(ok, step=1)], bad, N, 1000)
        with self.assertRaises(checks.ScoreError):
            checks.summarize([dict(ok, step=1)], CPS, N, 0)
        with self.assertRaises(checks.ScoreError):  # a record that skips a rule is not judged as holding
            checks.summarize([{"step": 1, "rules": {}, "hidden": {"passed": 0, "total": 1}}], CPS, N, 1000)

    def test_windows(self):
        self.assertEqual(checks.windows([5, 20, 50], 55), [(1, 5), (6, 20), (21, 50), (51, 55)])
        self.assertEqual(checks.windows([5, 20, 50], 50), [(1, 5), (6, 20), (21, 50)])


class TaskDesign(unittest.TestCase):
    def test_steps_are_distinct_and_complete(self):
        names = [task.step(k)["name"] for k in range(1, N + 1)]
        self.assertEqual(len(set(names)), N)
        self.assertGreaterEqual(N, 50)
        for k in range(1, N + 1):
            s = task.step(k)
            self.assertIn(f"pub fn {s['name']}(", s["reference"])
            self.assertIn(f"fn hidden_{s['name']}()", s["hidden_test"])
            self.assertGreaterEqual(len(s["cases"]), 3)
        self.assertEqual(len({task.step(k)["prompt"].split(": ", 1)[1].replace(task.step(k)["name"], "")
                              for k in range(1, N + 1)}), N, "no two step specs are the same")

    def test_rules_only_at_step_one(self):
        self.assertIn("R1.", task.prompt_for(1, N, TEST_CMD))
        self.assertIn(TEST_CMD, task.prompt_for(1, N, TEST_CMD))
        for k in (2, 5, 20, 50):
            self.assertNotIn("R1.", task.prompt_for(k, N, TEST_CMD))

    def test_expected_values(self):
        self.assertEqual(task.checksum(b""), 7)
        self.assertEqual(task.checksum(b"a"), (7 * 31 + 97) % 2**32)
        long = bytes(range(256)) * 4
        acc = 7
        for b in long:
            acc = (acc * 31 + b) & 0xFFFFFFFF
        self.assertEqual(task.checksum(long), acc)
        self.assertIn("assert_eq!(step_05(b\"\"), 2);", task.step(5)["hidden_test"])  # 7 ^ 5 == 2
        self.assertIn('assert_eq!(step_03("ab"), "ab-3-ab");', task.step(3)["hidden_test"])  # n = 3 // 6 % 3 + 2 = 2
        self.assertIn("&[]", task.hidden_tests_file(10))


class ConfigAndEnv(Tmp):
    def test_config_key_required(self):
        p = self.base / "c.toml"
        text = (HERE / "config.toml").read_text(encoding="utf-8")
        for key in run.CONFIG_KEYS:
            p.write_text("\n".join(ln for ln in text.splitlines() if not ln.startswith(key + " ")), encoding="utf-8")
            with self.assertRaises(run.RunError, msg=key):
                run.load_config(p)

    def test_config_path_must_be_relative(self):
        for bad in ("../x", "C:/x", "/x", "a/../../b", ""):
            with self.assertRaises(run.RunError, msg=bad):
                run._relative(bad, "results_subdir")

    def test_child_env_drops_api_keys_by_name_and_value(self):
        env = {"ANTHROPIC_API_KEY": "sk-ant-123456", "OPENAI_API_KEY": "sk-oa-654321", "COPY": "sk-ant-123456",
               "anthropic_auth_token": "tok-999999", "PATH": "p", "EMPTY": ""}
        got = clients.child_env(env, self.base)  # self.base has no .env
        self.assertEqual(got, {"PATH": "p", "EMPTY": ""})

    def test_real_run_needs_confirmation_and_no_breaks(self):
        self.assertEqual(run.main(["--client", "claude", "--model", "x", "--context-window", "1000"], {}), 2)
        self.assertEqual(run.main(["--client", "claude", "--model", "x", "--context-window", "1000", "--confirm-plan-run",
                                   "--break", "5:R1"], {}), 2)
        self.assertEqual(run.main(["--client", "fake", "--model", "x", "--context-window", "0"], {}), 2)
        for bad in ("0:R1", f"{N + 1}:R1"):
            self.assertEqual(run.main(["--client", "fake", "--model", "x", "--context-window", "1000", "--break", bad], {}), 2)
        with self.assertRaises(SystemExit):
            run.main(["--client", "fake", "--model", "x", "--context-window", "1000", "--break", "5:R9"], {})


class Parsers(unittest.TestCase):
    """Hand-written samples in the documented stream shapes; NOT captured from a real CLI (none was run)."""

    def test_claude_stream(self):
        lines = [
            {"type": "system", "subtype": "init", "session_id": "s"},
            {"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Edit", "input": {"file_path": "src/steps.rs"}}],
                                              "usage": {"input_tokens": 10, "cache_read_input_tokens": 1000, "cache_creation_input_tokens": 5}}},
            {"type": "assistant", "message": {"content": [{"type": "tool_use", "name": "Bash", "input": {"command": TEST_CMD}}],
                                              "usage": {"input_tokens": 20, "cache_read_input_tokens": 1500, "cache_creation_input_tokens": 0}}},
            {"type": "result", "subtype": "success", "is_error": False, "result": "STEP 01 DONE | tests: pass | files: src/steps.rs"},
        ]
        r = clients.parse_claude_stream("\n".join(json.dumps(x) for x in lines))
        self.assertIsNone(r["error"])
        self.assertEqual(r["events"], [{"kind": "edit", "path": "src/steps.rs"}, {"kind": "command", "text": TEST_CMD}])
        self.assertEqual(r["context_tokens"], 1520)
        self.assertTrue(checks.check_tests_run(r["events"], TEST_RE)[0])
        self.assertTrue(clients.parse_claude_stream(json.dumps({"type": "result", "is_error": True, "result": "x"}))["error"])
        self.assertTrue(clients.parse_claude_stream("not json")["error"])

    def test_codex_stream(self):
        lines = [
            {"type": "thread.started", "thread_id": "th1"},
            {"type": "item.completed", "item": {"type": "file_change", "changes": [{"path": "src/steps.rs", "kind": "update"}]}},
            {"type": "item.completed", "item": {"type": "command_execution", "command": TEST_CMD}},
            {"type": "item.completed", "item": {"type": "agent_message", "text": "STEP 01 DONE | tests: pass | files: none"}},
            {"type": "turn.completed", "usage": {"input_tokens": 30000, "cached_input_tokens": 20000, "output_tokens": 300}},
        ]
        r = clients.parse_codex_stream("\n".join(json.dumps(x) for x in lines))
        self.assertIsNone(r["error"])
        self.assertEqual(r["thread_id"], "th1")
        self.assertEqual(r["context_tokens"], 30000)
        self.assertEqual([e["kind"] for e in r["events"]], ["edit", "command"])
        self.assertTrue(clients.parse_codex_stream(json.dumps({"type": "turn.failed", "error": {"message": "x"}}))["error"])

    def test_cargo_grader_parse(self):
        g = clients.CargoGrader("bs", pathlib.Path("projects") / "drift-x", 1)
        self.assertEqual(g.dir.name, "drift-x-grader")
        out = "test hidden_step_01 ... ok\ntest hidden_step_02 ... FAILED\n"
        seen = {m.group(1): m.group(3) for m in clients.TEST_LINE.finditer(out)}
        self.assertEqual(seen, {"hidden_step_01": "ok", "hidden_step_02": "FAILED"})


if __name__ == "__main__":
    unittest.main()
