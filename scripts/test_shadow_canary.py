"""Tests for scripts/shadow_canary.py: the last-pass.json record it writes, and how it reads the queue's SYSTEM_PROMPT from reviewer.rs.
No codex, no model: leg A / leg B and `codex --version` are replaced, and the checkout is a temporary folder with its own broadsword.toml.

Run:  python -I -B scripts/test_shadow_canary.py
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import ast
import importlib.util
import io
import json
import shutil
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("shadow_canary_under_test", HERE / "shadow_canary.py")
canary = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(canary)

TOML = 'scratch_dir = "scratch"\nstate_dir = "state"\nshadow_reviewer_model = "gpt-test"\nshadow_reviewer_timeout_seconds = 30\n'
REVIEWER_RS = 'pub const SYSTEM_PROMPT: &str = "be a reviewer";\n'


class LastPassTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="canary-test-"))
        self.addCleanup(shutil.rmtree, self.root, True)
        (self.root / "broadsword.toml").write_text(TOML, encoding="utf-8")
        (self.root / "reviewer.rs").write_text(REVIEWER_RS, encoding="utf-8")
        self.last = self.root / "state" / "canary" / "last-pass.json"

    def main(self, argv, a=0, b=0, version="codex-cli 9.9.9", checkout=None):
        with mock.patch.object(canary, "CHECKOUT", checkout or self.root), mock.patch.object(canary, "REVIEWER_RS", self.root / "reviewer.rs"), \
                mock.patch.object(canary, "leg_offline", return_value=a), mock.patch.object(canary, "leg_real", return_value=b), \
                mock.patch.object(canary, "codex_version", return_value=version), redirect_stdout(io.StringIO()):
            return canary.main(argv)

    def test_offline_pass_writes_the_version_atomically(self):
        self.assertEqual(self.main(["--offline-only"]), canary.REFUSED)
        rec = json.loads(self.last.read_text(encoding="utf-8"))
        self.assertEqual(rec["codex_version"], "codex-cli 9.9.9")
        self.assertEqual(rec["legs"], ["A"])
        self.assertIsNone(rec["model"])
        self.assertRegex(rec["utc"], r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ$")
        self.assertEqual([p.name for p in self.last.parent.iterdir()], ["last-pass.json"], "no temp file is left behind")

    def test_full_pass_records_both_legs_and_the_model(self):
        self.assertEqual(self.main([]), canary.REFUSED)
        rec = json.loads(self.last.read_text(encoding="utf-8"))
        self.assertEqual((rec["legs"], rec["model"]), (["A", "B"], "gpt-test"))

    def test_a_fail_writes_nothing(self):
        for label, a, b, argv in (("leg A open", canary.LEAKED, 0, ["--offline-only"]), ("leg A inconclusive", canary.INCONCLUSIVE, 0, []),
                                  ("leg B leaked", 0, canary.LEAKED, []), ("leg B inconclusive", 0, canary.INCONCLUSIVE, [])):
            with self.subTest(label):
                self.assertNotEqual(self.main(argv, a=a, b=b), canary.REFUSED)
                self.assertFalse(self.last.exists())

    def test_a_fail_leaves_an_older_pass_untouched(self):
        self.last.parent.mkdir(parents=True)
        self.last.write_text('{"codex_version": "old"}\n', encoding="utf-8")
        self.main([], a=canary.LEAKED)
        self.assertEqual(json.loads(self.last.read_text(encoding="utf-8")), {"codex_version": "old"})

    def test_a_pass_with_no_codex_version_is_not_recorded(self):
        self.assertEqual(self.main(["--offline-only"], version=None), canary.INCONCLUSIVE)
        self.assertFalse(self.last.exists())

    def test_from_a_worktree_the_pass_is_recorded_at_the_root(self):
        """A checkout in ROOT/worktrees/broadsword/wt (a `.git` file pointing at ROOT/.git/worktrees/wt) records in ROOT/state, the
        one record openai_review.py reads, and writes nothing in the worktree's own state/."""
        gitdir = self.root / ".git" / "worktrees" / "wt"
        gitdir.mkdir(parents=True)
        wt = self.root / "worktrees" / "broadsword" / "wt"
        wt.mkdir(parents=True)
        (wt / "broadsword.toml").write_text(TOML, encoding="utf-8")
        (wt / ".git").write_text(f"gitdir: {gitdir}\n", encoding="utf-8")
        self.assertEqual(self.main(["--offline-only"], checkout=wt), canary.REFUSED)
        self.assertEqual(json.loads(self.last.read_text(encoding="utf-8"))["codex_version"], "codex-cli 9.9.9")
        self.assertFalse((wt / "state").exists(), "no per-worktree record")

    def test_codex_version_never_raises(self):
        with mock.patch.object(canary, "run", side_effect=RuntimeError("boom")), mock.patch.object(canary.rv, "find_codex", return_value="codex"):
            self.assertIsNone(canary.codex_version())
        with mock.patch.object(canary, "run", return_value=(None, "", "")), mock.patch.object(canary.rv, "find_codex", return_value="codex"):
            self.assertIsNone(canary.codex_version(), "a timeout is no version")


REPO = HERE.parent

# A fixture in the form reviewer.rs uses (CRLF line ends, prompt macros, a literal piece, escapes, a trailing comma). HEAD and TAIL are
# the texts rustc makes of the two macro literals, written out by hand, so the parse is pinned to what concat! joins.
FIXTURE = r'''macro_rules! prompt_head {
    () => {
        "line one
says \"hi\" \\ back\tslash\n\'s"
    };
}
macro_rules! prompt_tail {
    () => {
        "Answer:
VERDICT: PASS"
    };
}
pub const SYSTEM_PROMPT: &str = concat!(prompt_head!(), "\n\n", prompt_tail!(),);
pub const SYSTEM_PROMPT_GATE: &str = concat!(prompt_head!(), "\n\nGATE");
'''.replace("\n", "\r\n")
HEAD = 'line one\nsays "hi" \\ back\tslash\n\'s'
TAIL = "Answer:\nVERDICT: PASS"

MACRO_H = 'macro_rules! h {\n    () => {\n        "head"\n    };\n}\n'
# Every form the parser must refuse (PromptParseError), never read as empty or partial.
UNKNOWN_FORMS = {
    "undefined macro": MACRO_H + 'pub const SYSTEM_PROMPT: &str = concat!(h!(), other!());\n',
    "macro with an argument": MACRO_H + 'pub const SYSTEM_PROMPT: &str = concat!(h!(x));\n',
    "macro defined twice": MACRO_H + MACRO_H + 'pub const SYSTEM_PROMPT: &str = concat!(h!());\n',
    "macro body not one literal": 'macro_rules! h {\n    () => {\n        concat!("a", "b")\n    };\n}\npub const SYSTEM_PROMPT: &str = concat!(h!());\n',
    "macro body two literals": 'macro_rules! h {\n    () => {\n        "a" "b"\n    };\n}\npub const SYSTEM_PROMPT: &str = concat!(h!());\n',
    "macro with a matcher": 'macro_rules! h {\n    ($x:expr) => {\n        "a"\n    };\n}\npub const SYSTEM_PROMPT: &str = concat!(h!());\n',
    "nested concat": MACRO_H + 'pub const SYSTEM_PROMPT: &str = concat!(h!(), concat!("a"));\n',
    "env! piece": 'pub const SYSTEM_PROMPT: &str = concat!(env!("HOME"));\n',
    "const name piece": 'pub const SYSTEM_PROMPT: &str = concat!(OTHER, "a");\n',
    "comment in concat": MACRO_H + 'pub const SYSTEM_PROMPT: &str = concat!(h!(), // x\n "a");\n',
    "raw string": 'pub const SYSTEM_PROMPT: &str = r"raw";\n',
    "unicode escape": r'pub const SYSTEM_PROMPT: &str = "a\u{41}";' + "\n",
    "hex escape": r'pub const SYSTEM_PROMPT: &str = "a\x41";' + "\n",
    "line-continuation escape": 'pub const SYSTEM_PROMPT: &str = "a\\\n   b";\n',
    "a function call": 'pub const SYSTEM_PROMPT: &str = prompt();\n',
    "no semicolon": 'pub const SYSTEM_PROMPT: &str = "a"\n',
    "unterminated literal": 'pub const SYSTEM_PROMPT: &str = "a;\n',
    "unterminated concat": 'pub const SYSTEM_PROMPT: &str = concat!("a",',
    "empty concat": 'pub const SYSTEM_PROMPT: &str = concat!();\n',
    "empty literal": 'pub const SYSTEM_PROMPT: &str = "";\n',
    "two definitions": 'pub const SYSTEM_PROMPT: &str = "a";\npub const SYSTEM_PROMPT: &str = "b";\n',
    "no definition": 'pub const SYSTEM_PROMPT_GATE: &str = "a";\n',
    "bare CR": 'pub const SYSTEM_PROMPT: &str = "a\rb";\n',
}


def bench_system_prompt() -> str:
    """SYSTEM in bench/reviewer/run.py, read as a literal (never imported): reviewer.rs documents SYSTEM_PROMPT as the prompt that
    benchmark measured, so it is an independent copy of the text rustc builds."""
    tree = ast.parse((REPO / "bench" / "reviewer" / "run.py").read_text(encoding="utf-8"))
    for node in tree.body:
        if isinstance(node, ast.Assign) and [getattr(t, "id", None) for t in node.targets] == ["SYSTEM"]:
            return ast.literal_eval(node.value)
    raise AssertionError("SYSTEM not found in bench/reviewer/run.py")


class PromptParseTests(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp(prefix="canary-prompt-"))
        self.addCleanup(shutil.rmtree, self.dir, True)

    def parse(self, text: str) -> str:
        path = self.dir / "reviewer.rs"
        path.write_bytes(text.encode("utf-8"))
        return canary.queue_system_prompt(path)

    def test_the_real_reviewer_rs_gives_the_benchmarked_prompt(self):
        prompt = canary.queue_system_prompt(REPO / "crates" / "broadsword" / "src" / "reviewer.rs")
        self.assertEqual(prompt, bench_system_prompt(), "reviewer.rs SYSTEM_PROMPT (concat! of prompt_head! and prompt_tail!), as parsed")
        self.assertTrue(prompt.startswith("You are an independent code reviewer."))
        self.assertIn('write "BLOCKING: none"', prompt, "escaped quotes are unescaped")
        self.assertNotIn("Rules for BLOCKING lines", prompt, "SYSTEM_PROMPT, not SYSTEM_PROMPT_GATE")
        self.assertNotIn("\r", prompt, "CRLF in the source reads as LF")

    def test_a_fixture_parses_to_the_pieces_rustc_joins(self):
        self.assertIn("\r\n", FIXTURE)
        self.assertEqual(self.parse(FIXTURE), HEAD + "\n\n" + TAIL)

    def test_the_plain_literal_form_still_parses(self):
        self.assertEqual(self.parse(REVIEWER_RS), "be a reviewer")

    def test_unknown_forms_are_refused_not_read_as_empty(self):
        for label, text in UNKNOWN_FORMS.items():
            with self.subTest(label):
                with self.assertRaises(canary.PromptParseError):
                    self.parse(text)

    def test_an_unknown_form_makes_the_canary_inconclusive_and_records_nothing(self):
        root = Path(tempfile.mkdtemp(prefix="canary-test-"))
        self.addCleanup(shutil.rmtree, root, True)
        (root / "broadsword.toml").write_text(TOML, encoding="utf-8")
        (root / "reviewer.rs").write_text(UNKNOWN_FORMS["undefined macro"], encoding="utf-8")
        legs = mock.Mock(return_value=canary.REFUSED)
        with mock.patch.object(canary, "CHECKOUT", root), mock.patch.object(canary, "REVIEWER_RS", root / "reviewer.rs"), \
                mock.patch.object(canary, "leg_offline", legs), mock.patch.object(canary, "leg_real", legs), \
                mock.patch.object(canary, "codex_version", return_value="codex-cli 9.9.9"), redirect_stdout(io.StringIO()) as out:
            for argv in (["--offline-only"], [], ["--show-prompt"]):
                with self.subTest(argv=argv):
                    self.assertEqual(canary.main(argv), canary.INCONCLUSIVE)
        self.assertIn("PromptParseError", out.getvalue())
        legs.assert_not_called()
        self.assertFalse((root / "state").exists(), "nothing recorded")

    def test_show_prompt_prints_the_real_prompt_and_records_nothing(self):
        with mock.patch.object(canary, "leg_offline") as a, mock.patch.object(canary, "leg_real") as b, \
                mock.patch.object(canary, "record_pass") as rec, redirect_stdout(io.StringIO()) as out:
            self.assertEqual(canary.main(["--show-prompt"]), 0)
        expected = bench_system_prompt()
        self.assertIn(f"{len(expected)} characters; first 80: {expected[:80]!r}", out.getvalue())
        a.assert_not_called()
        b.assert_not_called()
        rec.assert_not_called()


if __name__ == "__main__":
    unittest.main()
