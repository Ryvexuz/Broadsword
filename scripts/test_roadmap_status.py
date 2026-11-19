"""Tests for scripts/roadmap_status.py. Each test builds a tiny git repo in a scratch folder with a mini
CAPABILITY_ROADMAP.md and ROADMAP.md, runs the script on it, and removes the folder.

  python -I -B scripts/test_roadmap_status.py -v
"""
import sys

sys.dont_write_bytecode = True

import os
import pathlib
import re
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = HERE / "roadmap_status.py"
REAL_ROOT = HERE.parent

CAP = """# Cap

## Part 2: x

### Tier 0: Bedrock

| Node | What it is | Done when |
| --- | --- | --- |
| 0.1 Build thing | Stuff | It builds |
| 0.2 Math | Stuff | Tests pass |

### Tier 2: Core engines

| Node | Builds on | Done when |
| --- | --- | --- |
| 2.1 Raster engine (tiled layers) | 0.2 | Big image works |
| 2.2 Brush \\[wrap first: X\\] | 2.1 | Lag is low |

## Part 3: Coverage map

## Part 4: Broadsword alignment and order

### Mapping to your 12 stages

| Broadsword stage | Nodes |
| --- | --- |
| 1. Foundation | Tier 0; 2.1 |
| 4. Graphics | 2.1\u20132.2 |

Two suggested adjustments to your stage order:
"""

ROWS = {
    "0.1": ("Build thing", "1", "not started", "-", "It builds"),
    "0.2": ("Math", "1", "not started", "-", "Tests pass"),
    "2.1": ("Raster engine", "1, 4", "not started", "-", "Big image works"),
    "2.2": ("Brush", "4", "not started", "-", "Lag is low"),
}


def roadmap(rows, decisions=True):
    out = ["# Roadmap", "", "| Node | Name | Stage | Status | Evidence | Done when |", "| --- | --- | --- | --- | --- | --- |"]
    for n, r in rows.items():
        out.append("| " + n + " | " + " | ".join(r) + " |")
    if decisions:
        out += ["", "## Open decisions", ""]
    return "\n".join(out) + "\n"


def scratch_base():
    text = (REAL_ROOT / "broadsword.toml").read_text(encoding="utf-8")
    m = re.search(r'^scratch_dir\s*=\s*"([^"]+)"', text, re.M)
    if not m:
        raise RuntimeError("broadsword.toml has no scratch_dir")
    base = REAL_ROOT / m.group(1)
    base.mkdir(parents=True, exist_ok=True)
    return base


class T(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=scratch_base(), prefix="test-roadmap-", ignore_cleanup_errors=True)
        self.root = pathlib.Path(self.tmp.name)
        self.git("init", "-q")
        (self.root / "CAPABILITY_ROADMAP.md").write_text(CAP, encoding="utf-8")
        (self.root / "real.txt").write_text("x", encoding="utf-8")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "c1")
        self.sha = subprocess.run(["git", "-C", str(self.root), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        self.git("tag", "stage-x-done")

    def tearDown(self):
        self.tmp.cleanup()

    def git(self, *args):
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
        subprocess.run(["git", "-C", str(self.root), *args], check=True, capture_output=True, env=env)

    def run_script(self, rows=None, text=None):
        (self.root / "ROADMAP.md").write_text(text if text is not None else roadmap(rows or ROWS), encoding="utf-8")
        r = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--root", str(self.root)], capture_output=True, text=True)
        return r.returncode, r.stdout

    def with_row(self, node, **kw):
        rows = dict(ROWS)
        name, stage, status, ev, done = rows[node]
        rows[node] = (kw.get("name", name), kw.get("stage", stage), kw.get("status", status), kw.get("ev", ev), kw.get("done", done))
        return rows

    def test_ok(self):
        code, out = self.run_script()
        self.assertEqual(code, 0, out)
        self.assertIn("roadmap: ok, 4 nodes", out)

    def test_missing_node(self):
        rows = dict(ROWS)
        del rows["2.2"]
        code, out = self.run_script(rows)
        self.assertEqual(code, 1)
        self.assertIn("2.2: missing", out)

    def test_duplicate_and_extra_node(self):
        text = roadmap(ROWS) + "| 0.1 | Build thing | 1 | not started | - | It builds |\n| 9.9 | Z | none | not started | - | z |\n"
        code, out = self.run_script(text=text)
        self.assertEqual(code, 1)
        self.assertIn("0.1: appears 2 times", out)
        self.assertIn("9.9: in ROADMAP.md but not", out)

    def test_bad_status_word(self):
        code, out = self.run_script(self.with_row("0.1", status="finished"))
        self.assertEqual(code, 1)
        self.assertIn("not one of", out)

    def test_done_when_must_be_exact(self):
        code, out = self.run_script(self.with_row("0.1", done="It builds."))
        self.assertEqual(code, 1)
        self.assertIn("exact copy", out)

    def test_name_with_wrap_marker_and_parens(self):
        # 2.1's name stops at " (", 2.2's at the escaped bracket; both are in ROWS and pass in test_ok. A wrong name fails.
        code, out = self.run_script(self.with_row("2.2", name="Brush \\[wrap first: X\\]"))
        self.assertEqual(code, 1)
        self.assertIn("name", out)

    def test_stage_derived_from_part4(self):
        code, out = self.run_script(self.with_row("2.1", stage="4"))
        self.assertEqual(code, 1)
        self.assertIn("2.1: stage", out)
        code, out = self.run_script(self.with_row("2.2", stage="none"))
        self.assertEqual(code, 1)

    def test_in_progress_without_evidence_is_unverified(self):
        code, out = self.run_script(self.with_row("0.1", status="in progress"))
        self.assertEqual(code, 1)
        self.assertIn("UNVERIFIED", out)
        self.assertIn("no evidence", out)

    def test_evidence_that_checks_out(self):
        ev = f"`file:real.txt`; `tag:stage-x-done`; `commit:{self.sha}`"
        code, out = self.run_script(self.with_row("0.1", status="in progress", ev=ev))
        self.assertEqual(code, 0, out)
        self.assertIn("1 in progress", out)

    def test_bad_evidence_items(self):
        for ev in ("`file:nope.txt`", "`tag:no-such-tag`", "`commit:" + "a" * 40 + "`", "`file:../x`",
                   "`file:C:/Windows`", "`commit:nosuchrepo@" + self.sha + "`", "`web:http://x`"):
            code, out = self.run_script(self.with_row("0.1", status="in progress", ev=ev))
            self.assertEqual(code, 1, ev)
            self.assertIn("UNVERIFIED", out, ev)

    def test_done_needs_tag(self):
        code, out = self.run_script(self.with_row("0.1", status="done", ev="`file:real.txt`"))
        self.assertEqual(code, 1)
        self.assertIn("needs a tag", out)
        code, out = self.run_script(self.with_row("0.1", status="done", ev="`file:real.txt`; `tag:stage-x-done`"))
        self.assertEqual(code, 0, out)
        self.assertIn("1 done", out)

    def test_escaping_paths_are_unverified(self):
        # The outside targets exist, so only the containment rule can reject them.
        outside = self.root.parent / "outside-roadmap-test.txt"
        outside.write_text("x", encoding="utf-8")
        try:
            forms = [r"..\outside-roadmap-test.txt", "../outside-roadmap-test.txt", r"a\..\..\outside-roadmap-test.txt",
                     r"\Windows\win.ini", "/Windows/win.ini", r"\\server\share\x", "//server/share/x",
                     r"C:\Windows\win.ini", "C:win.ini", str(outside), "..", "real.txt/../../outside-roadmap-test.txt"]
            for f in forms:
                code, out = self.run_script(self.with_row("0.1", status="in progress", ev=f"`file:{f}`"))
                self.assertEqual(code, 1, f)
                self.assertIn("UNVERIFIED", out, f)
            # the same forms as the repo part of a commit item
            for f in (r"..\x", r"\Windows", r"C:\x", "../x"):
                code, out = self.run_script(self.with_row("0.1", status="in progress", ev=f"`commit:{f}@{self.sha}`"))
                self.assertEqual(code, 1, f)
        finally:
            outside.unlink()

    def test_symlink_or_junction_out_of_root_is_refused(self):
        outside = pathlib.Path(tempfile.mkdtemp(dir=scratch_base(), prefix="test-roadmap-out-"))
        try:
            (outside / "t.txt").write_text("x", encoding="utf-8")
            link = self.root / "lnk"
            try:
                os.symlink(outside, link, target_is_directory=True)
            except OSError:
                if os.name == "nt":
                    subprocess.run(["cmd", "/c", "mklink", "/J", str(link), str(outside)], check=True, capture_output=True)
                else:
                    raise
            code, out = self.run_script(self.with_row("0.1", status="in progress", ev="`file:lnk/t.txt`"))
            self.assertEqual(code, 1)
            self.assertIn("UNVERIFIED", out)
        finally:
            import shutil
            if (self.root / "lnk").exists():
                os.rmdir(self.root / "lnk") if (self.root / "lnk").is_junction() or (self.root / "lnk").is_symlink() else None
            shutil.rmtree(outside, ignore_errors=True)

    def test_malformed_part2_row_exits_2(self):
        cap = CAP.replace("| 0.2 Math | Stuff | Tests pass |", "| 0.2 Math | Stuff |")
        (self.root / "CAPABILITY_ROADMAP.md").write_text(cap, encoding="utf-8")
        code, out = self.run_script()
        self.assertEqual(code, 2, out)
        self.assertIn("node 0.2", out)

    def test_not_started_must_not_cite_evidence(self):
        code, out = self.run_script(self.with_row("0.1", ev="`file:real.txt`"))
        self.assertEqual(code, 1)

    def test_commit_in_sub_repo(self):
        sub = self.root / "projects" / "sub"
        sub.mkdir(parents=True)
        subprocess.run(["git", "-C", str(sub), "init", "-q"], check=True, capture_output=True)
        env = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@t", GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@t")
        subprocess.run(["git", "-C", str(sub), "commit", "-q", "--allow-empty", "-m", "s"], check=True, capture_output=True, env=env)
        sha = subprocess.run(["git", "-C", str(sub), "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
        code, out = self.run_script(self.with_row("0.1", status="in progress", ev=f"`commit:projects/sub@{sha}`"))
        self.assertEqual(code, 0, out)

    def test_missing_open_decisions_section(self):
        code, out = self.run_script(text=roadmap(ROWS, decisions=False))
        self.assertEqual(code, 1)
        self.assertIn("Open decisions", out)

    def test_real_roadmap_passes(self):
        r = subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--root", str(REAL_ROOT)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout)
        self.assertRegex(r.stdout, r"roadmap: ok, \d+ nodes")


if __name__ == "__main__":
    unittest.main()
