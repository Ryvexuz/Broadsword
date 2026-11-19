"""Tests for score_hard.py, with hand-written review texts. Run from bench/reviewer/:
  python -B -m unittest test_score_hard
"""
import json, pathlib, re, shutil, subprocess, sys, tempfile, unittest

import score_hard

HERE = pathlib.Path(__file__).resolve().parent
CASE = score_hard.load_case()

PERFECT = """VERDICT: FAIL
BLOCKING: src/config.rs:63 - max_attempts uses the exclusive range 1..10, so a config with max_attempts = 10 is refused although 10 is the documented upper bound.
BLOCKING: src/store.rs:142 - put checks path.exists() and then writes with fs::write: a TOCTOU race, two threads enqueueing the same id both succeed and one overwrites the other.
BLOCKING: src/paths.rs:27 - only the first component is checked for `..`, so an id like a/../../x escapes the queue folder (path traversal).
BLOCKING: src/config.rs:14 - Config derives Debug and Queue::open logs it with {:?}, so the api_token is written to the log in plain text.
BLOCKING: src/store.rs:182 - ack does `let _ = self.save(...)` and returns Ok, so a failed write is silently swallowed and the job stays claimed.
BLOCKING: src/store.rs:78 - payload.len() as u16 truncates the length in the header, so a payload over 65535 bytes is stored but cannot be read back.
BLOCKING: src/config.rs:83 - lease() uses Duration::from_millis on lease_secs, the wrong unit: a 30 second lease becomes 30 milliseconds.
BLOCKING: src/store.rs:162 - the lease guard is inverted: now > lease_until refuses expired leases and lets a live lease be stolen.
BLOCKING: src/store.rs:157 - claim acquires the lock and every early return with ? (NotFound, AlreadyDone) skips release, so the store stays locked forever.
BLOCKING: src/retry.rs:76 - backoff_stays_within_cap loops over 10..cfg.max_attempts, an empty range with max_attempts 8, so the test can never fail.
BLOCKING: src/config.rs:39 - the doc comment says unknown keys are ignored but the code returns UnknownKey, so the documentation contradicts the behaviour.
BLOCKING: src/retry.rs:23 - wrapping_shl wraps for attempt >= 64, so the backoff factor drops back to 1 and the delay resets to base_delay_ms.
MINOR: none"""

EMPTY = "VERDICT: PASS\nBLOCKING: none\nMINOR: none"

EVERYTHING = PERFECT.replace("MINOR: none", "") + """
BLOCKING: src/lib.rs:46 - peek calls first.unwrap(), which can panic.
BLOCKING: src/config.rs:33 - parse_u32 uses `as u32`, an unchecked cast that can truncate.
BLOCKING: src/log.rs:40 - the logger ignores the error from writeln with let _, so log failures are silently swallowed.
MINOR: none"""

VAGUE = """VERDICT: FAIL
BLOCKING: src/store.rs - error handling looks wrong in a few places.
BLOCKING: src/config.rs - validation could be stricter and the code is risky.
BLOCKING: src/retry.rs - the retry logic should be reviewed.
BLOCKING: src/paths.rs - path handling seems unsafe.
MINOR: src/lib.rs - naming could be better."""

MINOR_ONLY = """VERDICT: PASS
BLOCKING: none
MINOR: src/config.rs:63 - max_attempts = 10 is refused because the range 1..10 is exclusive.
MINOR: src/config.rs:83 - lease() uses from_millis on lease_secs, the wrong unit."""


class ScoreHard(unittest.TestCase):
    def test_case_shape(self):
        self.assertEqual(len(CASE["bugs"]), 12)
        self.assertEqual(len(CASE["decoys"]), 3)
        self.assertEqual(len({b["kind"] for b in CASE["bugs"]}), 12)
        for b in CASE["bugs"] + CASE["decoys"]:
            re.compile(b["catch"])
            self.assertTrue(b["file"].startswith("src/") and len(b["lines"]) == 2)

    def test_every_catch_in_cases_json_compiles(self):
        # the old score.py compiles each case's top-level `catch` with re.compile;
        # Python 3.11+ refuses a global flag that is not at the start
        cases = json.loads((HERE / "cases.json").read_text(encoding="utf-8"))
        for c in cases:
            if "catch" in c:
                re.compile(c["catch"])
        hard = [c for c in cases if c["id"] == "hard1"][0]
        self.assertTrue(hard["catch"].startswith("(?i)"))
        self.assertEqual(hard["catch"].count("(?i)"), 1)

    def test_perfect_review(self):
        s = score_hard.score_text(PERFECT, CASE)
        self.assertEqual(len(s["caught"]), 12, s)
        self.assertEqual((s["minor_only"], s["missed"], s["decoy_fps"], s["other"]), ([], [], [], []))

    def test_empty_review(self):
        s = score_hard.score_text(EMPTY, CASE)
        self.assertEqual((len(s["caught"]), len(s["missed"]), s["decoy_fps"], s["other"]), (0, 12, [], []))

    def test_flagging_everything_catches_all_but_pays_in_false_alarms(self):
        s = score_hard.score_text(EVERYTHING, CASE)
        self.assertEqual(len(s["caught"]), 12)
        self.assertEqual(sorted(s["decoy_fps"]), ["d1", "d2", "d3"])
        self.assertEqual(s["other"], [])

    def test_vague_wording_catches_nothing(self):
        s = score_hard.score_text(VAGUE, CASE)
        self.assertEqual(s["caught"], [])
        self.assertEqual(len(s["missed"]), 12)
        self.assertEqual(s["decoy_fps"], [])
        self.assertEqual(len(s["other"]), 4)

    def test_minor_only(self):
        s = score_hard.score_text(MINOR_ONLY, CASE)
        self.assertEqual(s["caught"], [])
        self.assertEqual(sorted(s["minor_only"]), ["b01", "b07"])
        self.assertEqual(len(s["missed"]), 10)

    def test_a_decoy_mentioned_only_as_minor_still_counts_as_a_false_alarm(self):
        s = score_hard.score_text("VERDICT: PASS\nBLOCKING: none\nMINOR: src/lib.rs:46 - first.unwrap() may panic.", CASE)
        self.assertEqual(s["decoy_fps"], ["d1"])

    def test_score_dir_reads_run_py_result_files(self):
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            rec = dict(case="hard1", model="opus", effort="high", wall_s=42.5, exit=0, cost_usd=0.31, result=PERFECT, is_error=False)
            (d / "hard1__opus__high.json").write_text(json.dumps(rec), encoding="utf-8")
            (d / "hard1__opus__high.raw.json").write_text("{}", encoding="utf-8")
            (d / "a19ee1a__opus__high.json").write_text(json.dumps(dict(rec, case="a19ee1a")), encoding="utf-8")
            _, rows = score_hard.score_dir(d)
        self.assertEqual(len(rows), 1)
        self.assertEqual((rows[0]["model"], rows[0]["cost"], rows[0]["wall"], len(rows[0]["caught"])), ("opus", 0.31, 42.5, 12))

    def test_score_hard_reads_every_key_run_py_writes(self):
        """Runs run.py's own main() with only the claude CLI call replaced by a canned reply, then scores the file it wrote."""
        import importlib.util, os, types
        from unittest import mock
        spec = importlib.util.spec_from_file_location("bench_run", HERE / "run.py")
        run = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(run)
        canned = json.dumps({"result": PERFECT, "total_cost_usd": 0.25, "usage": {}, "is_error": False,
                             "modelUsage": {"m": {}}}).encode("utf-8")
        fake = types.SimpleNamespace(stdout=canned, stderr=b"", returncode=0)
        diff = self.built_diff()
        with tempfile.TemporaryDirectory() as d:
            d = pathlib.Path(d)
            (d / "cases").mkdir()
            (d / "cases" / "hard1.diff").write_text(diff, encoding="utf-8")
            (d / "cases.json").write_text((HERE / "cases.json").read_text(encoding="utf-8"), encoding="utf-8")
            out = d / "out"
            with mock.patch.object(run, "HERE", d), mock.patch.object(run, "WORKDIR", str(d / "work")),                     mock.patch.object(run.subprocess, "run", return_value=fake),                     mock.patch.object(sys, "argv", ["run.py", str(out), "opus", "high", "hard1"]):
                run.main()
            rec = json.loads((out / "hard1__opus__high.json").read_text(encoding="utf-8"))
            for key in ("case", "model", "effort", "wall_s", "exit", "cost_usd", "result"):
                self.assertIn(key, rec)
            _, rows = score_hard.score_dir(out)
        self.assertEqual(len(rows), 1)
        r = rows[0]
        self.assertEqual((r["model"], r["effort"], r["cost"], r["err"], len(r["caught"])), ("opus", "high", 0.25, False, 12))
        self.assertEqual(r["wall"], rec["wall_s"])

    def test_run_py_sends_only_the_brief_and_the_diff(self):
        src = (HERE / "run.py").read_text(encoding="utf-8")
        self.assertEqual(set(re.findall(r"\bc\[['\"](\w+)['\"]\]", src)), {"id", "diff", "brief"})

    def built_diff(self):
        """Builds the diff with hard/make_diff.sh into a temp file (cases/ is git-ignored, so a fresh checkout has no
        hard1.diff). If a cases/hard1.diff exists it must be byte-identical to the fresh one."""
        bash = shutil.which("bash")
        self.assertIsNotNone(bash, "bash is needed to run hard/make_diff.sh")
        with tempfile.TemporaryDirectory() as d:
            out = pathlib.Path(d) / "hard1.diff"
            r = subprocess.run([bash, str(HERE / "hard" / "make_diff.sh"), str(out)], capture_output=True, text=True)
            self.assertEqual(r.returncode, 0, r.stderr)
            fresh = out.read_bytes()
        existing = HERE / CASE["diff"]
        if existing.exists():
            self.assertEqual(existing.read_bytes(), fresh, "cases/hard1.diff is stale: rerun hard/make_diff.sh")
        return fresh.decode("utf-8")

    def test_brief_and_diff_do_not_hint(self):
        diff = self.built_diff()
        words = r"\b(bugs?|buggy|planted|decoys?|fixme|todo|xxx|hack|vulnerab\w*|insecure|broken)\b"
        self.assertIsNone(re.search(words, diff, re.I))
        self.assertIsNone(re.search(words, CASE["brief"], re.I))
        self.assertNotIn("hard/", CASE["brief"])


if __name__ == "__main__":
    unittest.main()
