"""Tests for clients/codex_coder.py. No model is ever called. Every test but one uses a FAKE codex (a Python script behind
codex.cmd, first on PATH); the opt-in test_the_real_codex_sandbox_refuses_writes_outside_the_worktree (BSW_TEST_REAL_CODEX_CANARY=1)
runs the real codex binary against the canary's local fake model server, offline.
Each test builds a throwaway project under the main root's scratch folder: broadsword.toml, a copy of the scripts, a Git repo with
the task's linked worktree and a task record. The script finds that project's root from its own location, exactly as in use.

Run:  env -u OPENAI_API_KEY python -I -B -m unittest discover -s clients -p "test_codex_coder.py" -v
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import importlib.util
import json
import os
import re
import secrets
import shutil
import stat
import subprocess
import tomllib
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("openai_mcp_for_coder_tests", HERE / "openai_mcp.py")
oc = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(oc)
MAIN_ROOT = oc.find_main_root(HERE)
SCRATCH = MAIN_ROOT / tomllib.loads((MAIN_ROOT / "broadsword.toml").read_text(encoding="utf-8"))["scratch_dir"]

TASK = "t-20261119-000001"
FAKE_KEY = "sk-test-CODERSECRET123"
DOTENV_VALUE = "dotenv-value-7f3a9c"
USAGE = {"input_tokens": 1234, "cached_input_tokens": 1000, "output_tokens": 56}

FAKE_CODEX_PY = r'''
import json, os, subprocess, sys, time
d = os.path.dirname(os.path.abspath(__file__))
args = sys.argv[1:]
if args == ["--version"]:
    print("codex-cli 0.0.0-fake")
    sys.exit(0)
mode = open(os.path.join(d, "mode"), encoding="utf-8").read().strip()
stdin = sys.stdin.read()
json.dump({"args": args, "stdin": stdin, "cwd": os.getcwd(), "env": dict(os.environ)}, open(os.path.join(d, "seen.json"), "w", encoding="utf-8"))
out = args[args.index("-o") + 1]
wt = args[args.index("-C") + 1]
def write(rel, text="x\n"):
    p = os.path.join(wt, rel)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    open(p, "w", encoding="utf-8").write(text)
if mode in ("failoutside", "hangoutside", "holdtemp"):
    write("README.md", "changed outside the claim\n")
if mode in ("fail", "failoutside"):
    sys.stderr.write("boom\n")
    sys.exit(1)
if mode == "holdtemp":
    # a left-over child of codex keeps a file open in codex_coder's temp folder, so that folder cannot be removed
    hold = os.path.join(os.path.dirname(out), "hold.txt")
    pidf = os.path.join(d, "hold.pid")
    code = ("import os, sys, time; f = open(sys.argv[1], 'w'); open(sys.argv[2], 'w').write(str(os.getpid())); time.sleep(30)")
    flags = 0x00000008 | 0x00000200 if os.name == "nt" else 0  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    subprocess.Popen([sys.executable, "-I", "-c", code, hold, pidf], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, creationflags=flags, cwd=d)
    for _ in range(200):
        if os.path.exists(pidf) and open(pidf).read().strip():
            break
        time.sleep(0.05)
if mode == "hardlink":
    os.makedirs(os.path.join(wt, "src"), exist_ok=True)
    os.link(os.path.join(d, "outside-target.txt"), os.path.join(wt, "src", "hl.txt"))
if mode in ("inside", "hang", "mcp", "nousage", "outside", "gitfile", "junction", "newdir", "outsidedir", "hangoutside", "holdtemp",
            "adsinside", "tiernote", "tiernoteitem"):
    write("src/new/a.py", "print('hi')\n")
    write("notes.md", "edited\n")
if mode == "outside":
    write("README.md", "changed outside the claim\n")
    write("other/b.txt")
if mode == "settings":
    write("broadsword.toml", "alpha = 1\n")
if mode in ("support", "supportsibling"):
    write("crates/broadsword/tests/support/mod.rs", "// alpha 1\n")
if mode == "supportsibling":
    write("crates/broadsword/tests/support/other.rs", "// not claimed\n")
if mode == "outsidedir":
    os.makedirs(os.path.join(wt, "emptydir"))
if mode == "gitfile":
    open(os.path.join(wt, ".git"), "a", encoding="utf-8").write("\n")
if mode == "junction":
    subprocess.run(["cmd", "/d", "/c", "mklink", "/J", os.path.join(wt, "src", "jl"), d], capture_output=True)
ODD = "we ird ☃ a*b?<>|\"\tz"  # NTFS allows all of these in a stream name
def stream(rel, name, text="s\n"):
    open(os.path.join(wt, rel) + ":" + name, "w", encoding="utf-8").write(text)
if mode == "adsoutside":
    stream("README.md", "extra")
if mode == "adschange":
    stream("README.md", "old", "BBBB\n")  # same length as the stream the test planted
if mode == "adsremove":
    os.remove(os.path.join(wt, "README.md") + ":old")
if mode == "adsinside":
    stream("notes.md", "meta")
    stream("src/new/a.py", "meta")
    stream("src", "dirmeta")  # a stream on the claimed folder itself
if mode == "adsodd":
    stream("notes.md", ODD)
    stream("README.md", ODD)
if mode == "adstop":
    open(wt + ":topmeta", "w", encoding="utf-8").write("t\n")  # a stream on the worktree folder itself
if mode == "filetodir":  # the unclaimed FILE src (planted by the test) becomes a folder holding only the claimed src/a.py
    os.remove(os.path.join(wt, "src"))
    write("src/a.py")
if mode == "dirtofile":  # the folder docs (holding the claimed docs/x.md) becomes an unclaimed FILE docs
    os.remove(os.path.join(wt, "docs", "x.md"))
    os.rmdir(os.path.join(wt, "docs"))
    write("docs")
if mode == "filetolink":  # the claimed file lnk becomes a junction
    os.remove(os.path.join(wt, "lnk"))
    subprocess.run(["cmd", "/d", "/c", "mklink", "/J", os.path.join(wt, "lnk"), d], capture_output=True)
if mode == "streammove":  # the file src with stream src:x becomes a folder src with a stream src:x of the SAME bytes
    os.remove(os.path.join(wt, "src"))
    os.makedirs(os.path.join(wt, "src"))
    stream("src", "x", "S\n")
# ---- metadata only, never the bytes (review q-20261121-011825). M_* = README.md (unclaimed), docs (unclaimed folder);
# metainside = the same on notes.md and the folder src (both claimed); hiddenholder = the folder src under claim src/a.py.
def set_hidden(p):
    import ctypes
    k = ctypes.WinDLL("kernel32", use_last_error=True)
    if not k.SetFileAttributesW(p, k.GetFileAttributesW(p) | 0x2):  # FILE_ATTRIBUTE_HIDDEN
        raise OSError(ctypes.get_last_error(), "SetFileAttributesW", p)
def acl(p):
    r = subprocess.run(["icacls", p, "/grant", "*S-1-1-0:(R)"], capture_output=True, text=True)  # Everyone: read
    if r.returncode != 0:
        raise OSError(r.returncode, "icacls failed: " + r.stdout + r.stderr, p)
def meta(rel, what):
    p = os.path.join(wt, rel)
    if what == "ro":
        os.chmod(p, 0o444)
    elif what == "hidden":
        set_hidden(p)
    elif what == "acl":
        acl(p)
    elif what == "touch":
        os.utime(p, (1000000000, 1000000000))
if mode.startswith("M_"):
    what = mode[2:]
    meta("docs" if what == "hidden" else "README.md", what)
if mode == "metainside":
    for what in (("ro", "acl", "touch") if os.name == "nt" else ("ro", "touch")):
        meta("notes.md", what)
    if os.name == "nt":
        meta("src", "hidden")
        meta("src", "acl")
if mode == "hiddenholder":
    meta("src", "hidden")
# ---- folder times (review q-20261121-025544): the test plants docs/old.md; the claim is docs/x.md --------------------
if mode == "docstouch":
    os.utime(os.path.join(wt, "docs"), (1000000000, 1000000000))
if mode == "docsaddx":
    write("docs/x.md")
if mode == "docsaddy":
    write("docs/y.md")
if mode == "docsreplacex":  # an editor's save: a temp file renamed over the claimed docs/x.md (new identity, same name)
    write("docs/x.md.tmp", "new\n")
    os.replace(os.path.join(wt, "docs", "x.md.tmp"), os.path.join(wt, "docs", "x.md"))
if mode == "docsrecreateold":  # the unclaimed docs/old.md deleted and recreated with the SAME bytes
    b = open(os.path.join(wt, "docs", "old.md"), "rb").read()
    os.remove(os.path.join(wt, "docs", "old.md"))
    open(os.path.join(wt, "docs", "old.md"), "wb").write(b)
if mode == "roottouch":
    os.utime(wt, (1000000000, 1000000000))
if mode == "rootaddclaimed":
    write("new.md")
if mode in ("hang", "hangoutside"):
    time.sleep(60)
open(out, "w", encoding="utf-8").write("I changed src/new/a.py and notes.md.")
print(json.dumps({"type": "thread.started", "thread_id": "x"}))
print(json.dumps({"type": "turn.started"}))
NOTE = "Configured service tier `priority` is not advertised as supported for model `gpt-6.1-sol` and will be omitted from requests."
if mode == "tiernote":  # codex 0.170.0's real shape: a top-level error event
    print(json.dumps({"type": "error", "message": NOTE}))
if mode == "tiernoteitem":  # the same notice as an error item
    print(json.dumps({"type": "item.completed", "item": {"id": "e0", "type": "error", "message": NOTE}}))
print(json.dumps({"type": "item.completed", "item": {"id": "i0", "type": "command_execution", "command": "dir", "status": "completed"}}))
print(json.dumps({"type": "item.completed", "item": {"id": "i1", "type": "file_change", "changes": [], "status": "completed"}}))
if mode == "mcp":
    print(json.dumps({"type": "item.completed", "item": {"id": "i2", "type": "mcp_tool_call", "server": "s", "tool": "t", "status": "completed"}}))
print(json.dumps({"type": "item.completed", "item": {"id": "i3", "type": "agent_message", "text": "done"}}))
if mode != "nousage":
    print(json.dumps({"type": "turn.completed", "usage": USAGE}))
'''.replace("USAGE", json.dumps(USAGE))

TOML = """root = "."
worktrees_dir = "worktrees"
state_dir = "state"
scratch_dir = "scratch"
reviewer_brief_max_kb = 4
codex_coder_timeout_seconds = 8
"""


def git(*a, cwd):
    r = subprocess.run(["git", "-c", "core.hooksPath=NUL", *a], cwd=str(cwd), capture_output=True, text=True, timeout=60)
    if r.returncode != 0:
        raise RuntimeError(f"git {a} failed: {r.stderr}")
    return r


def force_rmtree(top: Path):
    for dirpath, dirnames, _ in os.walk(top):
        for name in list(dirnames):
            p = os.path.join(dirpath, name)
            if os.path.islink(p) or os.path.isjunction(p):
                dirnames.remove(name)
                os.rmdir(p) if os.path.isjunction(p) else os.unlink(p)

    def onexc(func, path, _e):
        os.chmod(path, stat.S_IWRITE)
        func(path)
    shutil.rmtree(top, onexc=onexc)


class CoderTests(unittest.TestCase):
    def setUp(self):
        self.base = SCRATCH / f"test-codex-coder-{os.getpid()}-{secrets.token_hex(4)}"
        self.root = self.base / "root"
        (self.root / "clients").mkdir(parents=True)
        (self.root / "templates").mkdir()
        for f in ("codex_coder.py", "openai_review.py", "openai_mcp.py", "start_gate.py"):
            shutil.copy(HERE / f, self.root / "clients" / f)
        (self.root / "scripts").mkdir()
        for f in ("brief_lint.py", "brief_preflight.py"):
            shutil.copy(HERE.parent / "scripts" / f, self.root / "scripts" / f)
        shutil.copy(HERE.parent / "templates" / "coder-prompt-codex.md", self.root / "templates")
        (self.root / "broadsword.toml").write_text(TOML, encoding="utf-8")
        (self.root / ".env").write_text(f"SOME_VALUE={DOTENV_VALUE}\n", encoding="utf-8")
        (self.root / ".gitignore").write_text(".env\nstate/\nworktrees/\n", encoding="utf-8")
        (self.root / "README.md").write_text("project\n", encoding="utf-8")
        (self.root / "notes.md").write_text("notes\n", encoding="utf-8")
        git("init", "-q", cwd=self.root)
        git("add", "README.md", "notes.md", ".gitignore", cwd=self.root)
        git("-c", "user.email=t@example.invalid", "-c", "user.name=t", "commit", "-qm", "init", cwd=self.root)
        self.wt_rel = f"worktrees/broadsword/{TASK}"
        git("worktree", "add", "-q", "-b", f"task/{TASK}", self.wt_rel, cwd=self.root)
        self.wt = self.root / self.wt_rel
        self.write_task()
        self.write_canary_record("codex-cli 0.0.0-fake")  # the fake codex's --version
        self.brief = self.base / "brief.md"
        self.brief.write_text("## Requirements\nBuild the thing. BRIEF-MARK\n"
                              "## Claims\nnotes.md\n## Done check\n"
                              "- `broadsword test broadsword` succeeds.\n", encoding="utf-8")
        self.bin = self.base / "fakebin"
        self.bin.mkdir()
        (self.bin / "fake_codex.py").write_text(FAKE_CODEX_PY, encoding="utf-8")
        if os.name == "nt":
            (self.bin / "codex.cmd").write_text(f'@"{sys.executable}" -I -B "%~dp0fake_codex.py" %*\r\n', encoding="utf-8")
        else:
            (self.bin / "codex").write_text(f'#!/bin/sh\nexec "{sys.executable}" -I -B "$(dirname "$0")/fake_codex.py" "$@"\n', encoding="utf-8")
            (self.bin / "codex").chmod(0o755)

    def tearDown(self):
        force_rmtree(self.base)

    def write_task(self, **over):
        rec = {"id": TASK, "project": "broadsword", "status": "running", "owner": "t", "claims": "src\\nnotes.md",
               "worktree": self.wt_rel, "worktree_creating": "", "worktree_removed": "", "branch": f"task/{TASK}"}
        rec.update(over)
        if "claims" in over and hasattr(self, "brief"):
            # Key-only board fixtures need a matching brief, not the default notes claim.
            claim = "notes.md" if "notes.md" in over["claims"].split("\\n") else over["claims"].split("\\n")[0]
            text = self.brief.read_text(encoding="utf-8")
            text = re.sub(r"(?<=## Claims\n)[^\n]+", lambda _: claim, text)
            self.brief.write_text(text, encoding="utf-8")
        d = self.root / "state" / "tasks"
        d.mkdir(parents=True, exist_ok=True)
        (d / f"{TASK}.task").write_text("".join(f"{k} = {v}\n" for k, v in rec.items()), encoding="utf-8")

    def write_canary_record(self, version, name="coder-last-pass.json"):
        d = self.root / "state" / "canary"
        d.mkdir(parents=True, exist_ok=True)
        (d / name).write_text(json.dumps({"codex_version": version}), encoding="utf-8")

    def ledger_rows(self):
        import datetime
        day = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
        f = self.root / "state" / "gpt-spend" / f"{day}.tsv"
        if not f.is_file():
            return []
        return [l.split("\t") for l in f.read_text(encoding="utf-8").splitlines()[1:] if l.strip()]

    def env(self):
        sysroot = os.environ.get("SystemRoot", r"C:\Windows")
        e = {k: v for k, v in os.environ.items() if k.upper() not in ("PATH", "APPDATA", "OPENAI_API_KEY")}
        e["PATH"] = os.pathsep.join([str(self.bin), os.path.join(sysroot, "System32"), sysroot])
        e["APPDATA"] = str(self.base / "no-appdata")  # the real codex under %APPDATA%\npm is never found
        e["OPENAI_API_KEY"] = FAKE_KEY
        e["COPY_OF_DOTENV"] = DOTENV_VALUE
        e.update(getattr(self, "extra_env", {}))
        return e

    def run_coder(self, mode="inside", extra=None, brief=None):
        (self.bin / "mode").write_text(mode, encoding="utf-8")
        argv = [sys.executable, "-I", "-B", str(self.root / "clients" / "codex_coder.py"), "--task", TASK, "--model", "gpt-6.1-terra",
                "--effort", "ultra", "--brief", str(brief or self.brief)] + (extra or [])
        p = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", timeout=120, env=self.env())
        lines = [l for l in p.stdout.splitlines() if l.strip()]
        self.assertEqual(len(lines), 1, f"expected one JSON line, got stdout={p.stdout!r} stderr={p.stderr!r}")
        return p.returncode, json.loads(lines[0]), p

    def seen(self):
        f = self.bin / "seen.json"
        return json.loads(f.read_text(encoding="utf-8")) if f.is_file() else None

    # ---- the run ---------------------------------------------------------------------------------------------------

    def test_changes_inside_the_claim_pass_with_tokens_and_backend(self):
        code, res, _ = self.run_coder("inside")
        self.assertEqual(code, 0, res)
        self.assertFalse(res["is_error"])
        self.assertEqual(res["backend"], "codex")
        self.assertEqual(res["model"], "gpt-6.1-terra")
        self.assertEqual(res["effort"], "ultra")
        self.assertEqual(res["usage"]["input_tokens"], 1234)
        self.assertEqual(res["usage"]["output_tokens"], 56)
        self.assertEqual(res["modelUsage"], {"gpt-6.1-terra": {"inputTokens": 1234, "outputTokens": 56}})
        self.assertIsInstance(res["seconds"], float)
        self.assertIn("duration_ms", res)
        self.assertEqual(res["result"], "I changed src/new/a.py and notes.md.")
        self.assertIn("added file src/new/a.py", res["changed_paths"])
        self.assertIn("modified file notes.md", res["changed_paths"])
        self.assertIn("added dir src", res["changed_paths"])
        self.assertEqual(res["outside_claim"], [])
        self.assertFalse(res["claim_breach"])
        self.assertEqual(res["codex_items"], ["agent_message", "command_execution", "file_change"])

    def test_changed_toml_under_its_key_claim_is_covered_but_not_under_a_loader_key(self):
        with (self.root / "broadsword.toml").open("a", encoding="utf-8") as f:
            f.write("alpha = 1\n")  # an existing key: the start gate's new-key rule (R4b) does not apply
        (self.wt / "broadsword.toml").write_text("alpha = 0\n", encoding="utf-8")
        self.write_task(claims="broadsword.toml#alpha")
        code, res, _ = self.run_coder("settings")
        self.assertEqual(code, 0, res)
        self.assertIn("modified file broadsword.toml", res["changed_paths"])
        self.assertEqual(res["outside_claim"], [])
        self.assertFalse(res["claim_breach"])
        (self.wt / "broadsword.toml").write_text("alpha = 0\n", encoding="utf-8")
        self.write_task(claims="crates/broadsword/src/config.rs#alpha")
        code, res, _ = self.run_coder("settings")
        self.assertEqual(code, 3, res)
        self.assertIn("modified file broadsword.toml", res["changed_paths"])
        self.assertEqual(res["outside_claim"], ["broadsword.toml"])
        self.assertTrue(res["claim_breach"])

    def test_changed_test_support_under_its_key_claim_is_covered_but_a_sibling_is_a_breach(self):
        support = self.wt / "crates" / "broadsword" / "tests" / "support"
        support.mkdir(parents=True, exist_ok=True)
        (support / "mod.rs").write_text("// alpha 0\n", encoding="utf-8")
        self.write_task(claims="crates/broadsword/tests/support/mod.rs#alpha")
        code, res, _ = self.run_coder("support")
        self.assertEqual(code, 0, res)
        self.assertIn("modified file crates/broadsword/tests/support/mod.rs", res["changed_paths"])
        self.assertEqual(res["outside_claim"], [])
        self.assertFalse(res["claim_breach"])
        (support / "mod.rs").write_text("// alpha 0\n", encoding="utf-8")
        code, res, _ = self.run_coder("supportsibling")
        self.assertEqual(code, 3, res)
        self.assertIn("modified file crates/broadsword/tests/support/mod.rs", res["changed_paths"])
        # the folder's times moved for the unclaimed new child, so it is outside the claim too (judge)
        self.assertEqual(res["outside_claim"], ["crates/broadsword/tests/support", "crates/broadsword/tests/support/other.rs"])
        self.assertTrue(res["claim_breach"])

    def test_a_write_outside_the_claim_is_a_claim_breach_exit_3_and_is_left_in_place(self):
        code, res, _ = self.run_coder("outside")
        self.assertEqual(code, 3, res)
        self.assertTrue(res["claim_breach"])
        self.assertTrue(res["is_error"])
        self.assertTrue(res["result"].startswith("CLAIM BREACH"), res["result"])
        self.assertEqual(sorted(res["outside_claim"]), [".", "README.md", "other", "other/b.txt"])  # `.` too: its times moved for an unclaimed child (judge)
        self.assertEqual((self.wt / "README.md").read_text(encoding="utf-8"), "changed outside the claim\n", "never reverted")
        self.assertTrue((self.wt / "other" / "b.txt").is_file(), "never deleted")
        self.assertEqual(res["usage"]["input_tokens"], 1234, "tokens are still reported")

    def test_an_empty_new_folder_outside_the_claim_is_a_breach(self):
        code, res, _ = self.run_coder("outsidedir")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], [".", "emptydir"])  # `.` too: its times moved for an unclaimed child (judge)

    def test_a_changed_git_file_is_a_breach(self):
        code, res, _ = self.run_coder("gitfile")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], [".git"])

    @unittest.skipUnless(os.name == "nt", "junctions exist only on Windows")
    def test_a_new_junction_even_inside_the_claim_is_a_breach(self):
        code, res, _ = self.run_coder("junction")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["src/jl"])
        self.assertTrue(any(c.startswith("added link src/jl") for c in res["changed_paths"]), res["changed_paths"])

    # ---- NTFS alternate data streams (review q-20261120-235345) ----------------------------------------------------

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_new_stream_on_a_file_outside_the_claim_is_a_claim_breach_exit_3(self):
        code, res, _ = self.run_coder("adsoutside")
        self.assertEqual(code, 3, res)
        self.assertTrue(res["claim_breach"])
        self.assertTrue(res["result"].startswith("CLAIM BREACH"), res["result"])
        self.assertEqual(res["outside_claim"], ["README.md", "README.md:extra"])  # writing a stream also moves its holder's times (metadata())
        self.assertIn("added stream README.md:extra", res["changed_paths"])
        with open(str(self.wt / "README.md") + ":extra", encoding="utf-8") as f:
            self.assertEqual(f.read(), "s\n", "never reverted")

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_changed_existing_stream_outside_the_claim_is_a_breach(self):
        with open(str(self.wt / "README.md") + ":old", "w", encoding="utf-8") as f:
            f.write("AAAA\n")  # there before the run: not a breach by itself
        code, res, _ = self.run_coder("adschange")  # same length, other bytes
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["README.md", "README.md:old"])  # writing a stream also moves its holder's times (metadata())
        self.assertIn("modified stream README.md:old", res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_removed_stream_outside_the_claim_is_a_breach(self):
        with open(str(self.wt / "README.md") + ":old", "w", encoding="utf-8") as f:
            f.write("AAAA\n")
        code, res, _ = self.run_coder("adsremove")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["README.md", "README.md:old"])  # writing a stream also moves its holder's times (metadata())
        self.assertIn("removed stream README.md:old", res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_streams_inside_the_claim_are_allowed(self):
        code, res, _ = self.run_coder("adsinside")
        self.assertEqual(code, 0, res)
        self.assertEqual(res["outside_claim"], [])
        for p in ("notes.md:meta", "src/new/a.py:meta", "src:dirmeta"):
            self.assertIn(f"added stream {p}", res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_stream_with_an_odd_name_is_judged_by_its_file(self):
        odd = "we ird ☃ a*b?<>|\"\tz"
        code, res, _ = self.run_coder("adsodd")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["README.md", f"README.md:{odd}"])  # writing a stream also moves its holder's times (metadata())
        self.assertIn(f"added stream notes.md:{odd}", res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_stream_on_the_worktree_folder_itself_is_a_breach(self):
        code, res, _ = self.run_coder("adstop")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], [".", ".:topmeta"])  # the stream sets the folder's ARCHIVE attribute: dir-meta

    # ---- a kind change is a removal plus an addition (review q-20261121-004122) -------------------------------------

    def test_an_unclaimed_file_replaced_by_a_folder_holding_a_claimed_file_is_a_breach(self):
        self.write_task(claims="src/a.py\\nnotes.md")
        (self.wt / "src").write_text("unclaimed file\n", encoding="utf-8")
        code, res, _ = self.run_coder("filetodir")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], [".", "src"])  # `.` too: its times moved for an unclaimed child (judge)
        self.assertIn("removed file src", res["changed_paths"])
        self.assertIn("added dir src", res["changed_paths"])
        self.assertIn("added file src/a.py", res["changed_paths"])

    def test_a_folder_replaced_by_an_unclaimed_file_lists_both_and_is_a_breach(self):
        self.write_task(claims="docs/x.md\\nnotes.md")
        (self.wt / "docs").mkdir()
        (self.wt / "docs" / "x.md").write_text("x\n", encoding="utf-8")
        code, res, _ = self.run_coder("dirtofile")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], [".", "docs"])  # `.` too: its times moved for an unclaimed child (judge)
        for line in ("removed dir docs", "added file docs", "removed file docs/x.md"):
            self.assertIn(line, res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "junctions exist only on Windows")
    def test_a_claimed_file_replaced_by_a_junction_lists_both_and_is_a_breach(self):
        self.write_task(claims="lnk\\nnotes.md")
        (self.wt / "lnk").write_text("file\n", encoding="utf-8")
        code, res, _ = self.run_coder("filetolink")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], [".", "lnk"])  # `.` too: its times moved for an unclaimed child (judge)
        self.assertIn("removed file lnk", res["changed_paths"])
        self.assertIn("added link lnk", res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_stream_moved_with_equal_bytes_from_a_file_to_a_folder_is_reported(self):
        """Hostile: the holder changes kind and the stream keeps its name and bytes; it is still a changed path."""
        (self.wt / "src").write_text("file\n", encoding="utf-8")
        with open(str(self.wt / "src") + ":x", "w", encoding="utf-8") as f:
            f.write("S\n")
        code, res, _ = self.run_coder("streammove")  # claims `src` and `notes.md` cover all of it: no breach
        self.assertEqual(code, 0, res)
        for line in ("removed file src", "added dir src", "modified stream src:x"):
            self.assertIn(line, res["changed_paths"])

    # ---- metadata changes without a byte changed (review q-20261121-011825) -----------------------------------------

    def assert_meta_breach(self, mode, path):
        code, res, _ = self.run_coder(mode)
        self.assertEqual(code, 3, res)
        self.assertTrue(res["result"].startswith("CLAIM BREACH"), res["result"])
        self.assertEqual(res["outside_claim"], [path])
        self.assertEqual((self.wt / "README.md").read_text(encoding="utf-8"), "project\n", "the bytes never changed")
        return res

    def test_read_only_set_on_an_unclaimed_file_is_a_breach(self):
        res = self.assert_meta_breach("M_ro", "README.md")
        self.assertIn("modified file README.md", res["changed_paths"])
        self.assertFalse(os.access(self.wt / "README.md", os.W_OK), "left as the coder set it")

    @unittest.skipUnless(os.name == "nt", "the hidden attribute exists only on Windows")
    def test_hidden_set_on_an_unclaimed_folder_is_a_breach(self):
        (self.wt / "docs").mkdir()
        (self.wt / "docs" / "k.txt").write_text("k\n", encoding="utf-8")
        res = self.assert_meta_breach("M_hidden", "docs")
        self.assertIn("modified dir-meta docs", res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "icacls exists only on Windows")
    def test_an_acl_change_on_an_unclaimed_file_is_a_breach(self):
        self.assert_meta_breach("M_acl", "README.md")

    def test_a_touch_of_an_unclaimed_file_is_a_breach(self):
        self.assert_meta_breach("M_touch", "README.md")

    def test_the_same_metadata_changes_inside_the_claims_are_allowed(self):
        (self.wt / "src").mkdir()
        (self.wt / "src" / "a.txt").write_text("a\n", encoding="utf-8")
        code, res, _ = self.run_coder("metainside")
        self.assertEqual(code, 0, res)
        self.assertEqual(res["outside_claim"], [])
        self.assertIn("modified file notes.md", res["changed_paths"])
        if os.name == "nt":
            self.assertIn("modified dir-meta src", res["changed_paths"])

    @unittest.skipUnless(os.name == "nt", "the hidden attribute exists only on Windows")
    def test_hidden_set_on_a_folder_that_only_holds_a_claim_is_a_breach(self):
        """Hostile: a folder's own attributes need a claim that covers the folder; holding a claimed file is not enough."""
        self.write_task(claims="src/a.py\\nnotes.md")
        (self.wt / "src").mkdir()
        (self.wt / "src" / "a.py").write_text("a\n", encoding="utf-8")
        res = self.assert_meta_breach("hiddenholder", "src")
        self.assertIn("modified dir-meta src", res["changed_paths"])

    # ---- a folder's times may move only for a claimed direct child (review q-20261121-025544) -----------------------

    def docs_run(self, mode, extra_claims=()):
        self.write_task(claims="\\n".join(["docs/x.md", "notes.md", *extra_claims]))
        (self.wt / "docs").mkdir()
        (self.wt / "docs" / "old.md").write_text("old\n", encoding="utf-8")
        if mode == "docsreplacex":
            (self.wt / "docs" / "x.md").write_text("x\n", encoding="utf-8")
        return self.run_coder(mode)

    def test_touching_an_unclaimed_folder_with_no_child_change_is_a_breach(self):
        code, res, _ = self.docs_run("docstouch")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["docs"])
        self.assertEqual(res["changed_paths"], ["modified dir docs"])

    def test_adding_the_claimed_child_explains_the_folders_times(self):
        code, res, _ = self.docs_run("docsaddx")
        self.assertEqual(code, 0, res)
        self.assertEqual(res["outside_claim"], [])
        self.assertEqual(res["changed_paths"], ["modified dir docs", "added file docs/x.md"])

    def test_adding_an_unclaimed_child_is_a_breach_of_the_child_and_the_folder(self):
        code, res, _ = self.docs_run("docsaddy")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["docs", "docs/y.md"])

    def test_a_claimed_file_replaced_by_rename_is_allowed(self):
        code, res, _ = self.docs_run("docsreplacex")
        self.assertEqual(code, 0, res)
        self.assertEqual(res["changed_paths"], ["modified dir docs", "removed file docs/x.md", "added file docs/x.md"])

    def test_an_unclaimed_file_recreated_with_the_same_bytes_is_a_breach(self):
        """Hostile: same name, same bytes, new identity: a removal plus an addition of an unclaimed path."""
        code, res, _ = self.docs_run("docsrecreateold")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["docs", "docs/old.md"])
        self.assertIn("removed file docs/old.md", res["changed_paths"])

    def test_a_claim_on_the_folder_itself_allows_its_times(self):
        code, res, _ = self.docs_run("docstouch", extra_claims=["docs"])
        self.assertEqual(code, 0, res)

    def test_touching_the_worktree_folder_is_a_breach(self):
        code, res, _ = self.run_coder("roottouch")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["."])
        self.assertEqual(res["changed_paths"], ["modified dir ."])

    def test_adding_a_claimed_top_level_file_explains_the_worktree_folders_times(self):
        self.write_task(claims="new.md\\nnotes.md")
        code, res, _ = self.run_coder("rootaddclaimed")
        self.assertEqual(code, 0, res)
        self.assertEqual(res["changed_paths"], ["modified dir .", "added file new.md"])

    def run_driver_failing_metadata(self, phase):
        """The real main()/run_coder with metadata() raising for README.md in the before-run or the after-run snapshot only."""
        (self.bin / "mode").write_text("inside", encoding="utf-8")
        driver = (
            "import importlib.util, sys\n"
            f"spec = importlib.util.spec_from_file_location('cc_drv', {str(self.root / 'clients' / 'codex_coder.py')!r})\n"
            "cc = importlib.util.module_from_spec(spec); spec.loader.exec_module(cc)\n"
            f"phase, real_meta, real_snap, calls = {phase!r}, cc.metadata, cc.snapshot, []\n"
            "def meta(p):\n"
            "    if p.endswith('README.md') and ((phase == 'before') == (len(calls) == 1)):\n"
            "        raise PermissionError(5, 'denied', p)\n"
            "    return real_meta(p)\n"
            "def snap(top):\n"
            "    calls.append(top)\n"
            "    return real_snap(top)\n"
            "cc.metadata, cc.snapshot = meta, snap\n"
            "sys.exit(cc.main(sys.argv[1:]))\n")
        argv = [sys.executable, "-I", "-B", "-c", driver, "--task", TASK, "--model", "gpt-6.1-terra", "--effort", "ultra",
                "--brief", str(self.brief)]
        p = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", timeout=120, env=self.env())
        lines = [l for l in p.stdout.splitlines() if l.strip()]
        self.assertEqual(len(lines), 1, f"stdout={p.stdout!r} stderr={p.stderr!r}")
        return p.returncode, json.loads(lines[0])

    def test_metadata_that_cannot_be_read_before_the_run_refuses_it(self):
        code, res = self.run_driver_failing_metadata("before")
        self.assertEqual(code, 2, res)
        self.assertIn("README.md", res["result"])
        self.assertIsNone(self.seen(), "codex never started")

    def test_metadata_that_cannot_be_read_after_the_run_is_a_breach(self):
        code, res = self.run_driver_failing_metadata("after")
        self.assertEqual(code, 3, res)
        self.assertIn("README.md", res["outside_claim"])
        self.assertIn("added unreadable README.md", res["changed_paths"])

    def test_an_unexpected_item_is_an_error_but_changes_are_still_listed(self):
        code, res, _ = self.run_coder("mcp")
        self.assertEqual(code, 1, res)
        self.assertIn("mcp_tool_call:s/t", res["result"])
        self.assertIn("added file src/new/a.py", res["changed_paths"])

    def test_no_usage_is_an_error(self):
        code, res, _ = self.run_coder("nousage")
        self.assertEqual(code, 1, res)
        self.assertIn("turn.completed", res["result"])

    def test_codex_failing_is_an_error_naming_its_exit_code(self):
        code, res, _ = self.run_coder("fail")
        self.assertEqual(code, 1, res)
        self.assertIn("exited with code 1", res["result"])
        self.assertEqual(res["codex_exit_code"], 1)

    def test_a_hung_codex_is_killed_at_the_timeout_and_its_changes_are_still_judged(self):
        code, res, _ = self.run_coder("hang")
        self.assertEqual(code, 1, res)
        self.assertIn("timed out after 8 s", res["result"])
        self.assertIn("added file src/new/a.py", res["changed_paths"])
        self.assertLess(res["seconds"], 40)

    # ---- a breach is reported however codex ends (review q-20261119-222244, point 1) ---------------------------------

    def assert_breach(self, res, code):
        self.assertEqual(code, 3, res)
        self.assertTrue(res["claim_breach"], res)
        self.assertTrue(res["result"].startswith("CLAIM BREACH"), res["result"])
        self.assertIn("README.md", res["outside_claim"])
        self.assertEqual((self.wt / "README.md").read_text(encoding="utf-8"), "changed outside the claim\n", "never reverted")

    def test_a_breach_is_reported_when_codex_exits_with_an_error(self):
        code, res, _ = self.run_coder("failoutside")
        self.assert_breach(res, code)
        self.assertIn("exited with code 1", res["result"], "the codex error is reported too")

    def test_a_breach_is_reported_when_codex_is_killed_at_the_timeout(self):
        code, res, _ = self.run_coder("hangoutside")
        self.assert_breach(res, code)
        self.assertIn("timed out after 8 s", res["result"])

    @unittest.skipUnless(os.name == "nt", "an open file blocks a folder's removal only on Windows")
    def test_a_breach_is_reported_when_the_temp_folder_cannot_be_removed(self):
        try:
            code, res, _ = self.run_coder("holdtemp")
        finally:
            pidf = self.bin / "hold.pid"
            if pidf.is_file():
                subprocess.run(["taskkill", "/F", "/PID", pidf.read_text().strip()], capture_output=True)
        self.assert_breach(res, code)

    # ---- the after-run snapshot failing is a breach, not an escape (review q-20261119-235542, point 2) ------------------

    def test_a_failing_after_snapshot_is_a_breach_exit_3_through_main(self):
        """The real main()/run_coder, with snapshot() raising on its second (after-run) call only."""
        (self.bin / "mode").write_text("inside", encoding="utf-8")
        driver = (
            "import importlib.util, sys\n"
            f"spec = importlib.util.spec_from_file_location('cc_drv', {str(self.root / 'clients' / 'codex_coder.py')!r})\n"
            "cc = importlib.util.module_from_spec(spec); spec.loader.exec_module(cc)\n"
            "real, calls = cc.snapshot, []\n"
            "def snap(top):\n"
            "    calls.append(top)\n"
            "    if len(calls) > 1: raise RecursionError('maximum recursion depth exceeded')\n"
            "    return real(top)\n"
            "cc.snapshot = snap\n"
            "sys.exit(cc.main(sys.argv[1:]))\n")
        argv = [sys.executable, "-I", "-B", "-c", driver, "--task", TASK, "--model", "gpt-6.1-terra", "--effort", "ultra",
                "--brief", str(self.brief)]
        p = subprocess.run(argv, capture_output=True, text=True, encoding="utf-8", timeout=120, env=self.env())
        lines = [l for l in p.stdout.splitlines() if l.strip()]
        self.assertEqual(len(lines), 1, f"stdout={p.stdout!r} stderr={p.stderr!r}")
        res = json.loads(lines[0])
        self.assertEqual(p.returncode, 3, res)
        self.assertTrue(res["claim_breach"], res)
        self.assertEqual(res["outside_claim"], ["."])
        self.assertTrue(res["result"].startswith("CLAIM BREACH"), res["result"])
        self.assertIn("could not be judged after the run (RecursionError", res["result"])
        self.assertEqual(res["usage"]["input_tokens"], 1234, "the run is still reported")

    def test_a_new_hard_link_even_inside_the_claim_is_a_breach(self):
        (self.bin / "outside-target.txt").write_text("outside\n", encoding="utf-8")
        code, res, _ = self.run_coder("hardlink")
        self.assertEqual(code, 3, res)
        self.assertEqual(res["outside_claim"], ["src/hl.txt"])
        self.assertIn("added hardlink src/hl.txt", res["changed_paths"])

    # ---- what codex is given ---------------------------------------------------------------------------------------

    def test_codex_runs_in_the_worktree_under_the_mxc_profile_that_writes_only_there(self):
        code, res, _ = self.run_coder("inside")
        self.assertEqual(code, 0, res)
        s = self.seen()
        args = s["args"]
        self.assertEqual(os.path.normcase(s["cwd"]), os.path.normcase(str(self.wt)))
        self.assertEqual(args[:4], ["exec", "-m", "gpt-6.1-terra", "--ephemeral"])
        self.assertEqual(os.path.normcase(args[args.index("-C") + 1]), os.path.normcase(str(self.wt)))
        for flag in ("--strict-config", "--ignore-user-config", "--ignore-rules", "--json"):
            self.assertIn(flag, args)
        for c in ('windows.sandbox="mxc"', 'default_permissions="bsw_coder"', 'approval_policy="never"', 'web_search="disabled"',
                  'shell_environment_policy.inherit="core"', 'model_reasoning_effort="ultra"', "project_doc_max_bytes=0"):
            self.assertIn(c, args)
        profile = [a for a in args if a.startswith("permissions.bsw_coder.filesystem=")]
        self.assertEqual(profile, [f'permissions.bsw_coder.filesystem={{":minimal"="read", \'{self.wt}\'="write"}}'])
        self.assertNotIn("--sandbox", args, "the legacy sandbox flag would override the profile")
        self.assertNotIn("--dangerously-bypass-approvals-and-sandbox", args)
        disabled = {args[i + 1] for i, a in enumerate(args) if a == "--disable"}
        for f in ("plugins", "daemon_auto_start", "view_image", "multi_agent", "multi_agent_v2", "apps", "unified_exec"):
            self.assertIn(f, disabled)
        cat_arg = [a for a in args if a.startswith("model_catalog_json=")]
        self.assertEqual(len(cat_arg), 1)

    def test_service_tier_fast_adds_exactly_one_config_pair_and_absent_adds_none(self):
        code, res, _ = self.run_coder("inside")
        self.assertEqual(code, 0, res)
        plain = self.seen()["args"]
        self.assertFalse([a for a in plain if "service_tier" in a], "no flag: today's command line, no tier config")
        self.assertIsNone(res["service_tier"])
        code, res, _ = self.run_coder("inside", extra=["--service-tier", "fast"])
        self.assertEqual(code, 0, res)
        fast = self.seen()["args"]
        i = fast.index('service_tier="priority"')  # codex 0.170.0's catalog id for the Fast tier
        self.assertEqual(fast[i - 1], "-c")
        norm = lambda xs: [re.sub(r"bsw-coder-\w+", "bsw-coder-X", x) for x in xs]  # each run has its own temp folder
        self.assertEqual(norm(fast[:i - 1] + fast[i + 1:]), norm(plain), "the flag adds only that one -c pair")
        self.assertEqual(res["service_tier"], "fast")

    def test_a_tier_codex_says_it_omitted_fails_closed_with_a_named_reason(self):
        for mode in ("tiernote", "tiernoteitem"):
            code, res, _ = self.run_coder(mode, extra=["--service-tier", "fast"])
            self.assertNotEqual(code, 0, res)
            self.assertTrue(res["is_error"], res)
            self.assertIn("service_tier_not_applied", res["result"])
            self.assertEqual(res["service_tier"], "fast")
        # without a requested tier the same notice is only a codex notice, not this run's failure
        code, res, _ = self.run_coder("tiernote")
        self.assertEqual(code, 0, res)

    def test_service_tier_is_in_the_ledger_row_with_the_plan_label(self):
        code, res, _ = self.run_coder("inside", extra=["--service-tier", "fast"])
        self.assertEqual(code, 0, res)
        rows = self.ledger_rows()
        self.assertEqual(len(rows), 1, rows)
        self.assertIn("service_tier=fast", rows[0][1])
        self.assertIn("codex-plan", rows[0])
        self.run_coder("inside")
        self.assertNotIn("service_tier", self.ledger_rows()[1][1])

    def test_service_tier_is_recorded_even_when_the_caller_variable_is_already_set(self):
        self.extra_env = {"OPENAI_REVIEW_CALLER": "orchestrator-label"}
        try:
            code, res, _ = self.run_coder("inside", extra=["--service-tier", "fast"])
        finally:
            self.extra_env = {}
        self.assertEqual(code, 0, res)
        caller = self.ledger_rows()[0][1]
        self.assertIn("orchestrator-label", caller)
        self.assertIn("service_tier=fast", caller)

    def test_another_service_tier_is_refused_before_codex_starts(self):
        for v in ("priority", "flex", "FAST", ""):
            self.assert_refused("is not a service tier", extra=["--service-tier", v])

    def test_codex_never_starts_a_network_fetch_plugins_and_daemon_are_always_off(self):
        # Without these codex 0.170.0 runs a background `git fetch` of github.com/openai/plugins.git (and a shared daemon).
        code, res, _ = self.run_coder("inside")
        args = self.seen()["args"]
        pairs = {(args[i], args[i + 1]) for i in range(len(args) - 1)}
        self.assertIn(("--disable", "plugins"), pairs)
        self.assertIn(("--disable", "daemon_auto_start"), pairs)
        self.assertIn(("--disable", "remote_plugin"), pairs)
        self.assertNotIn("--enable", args)

    def load_copy(self):
        spec = importlib.util.spec_from_file_location(f"cc_copy_{secrets.token_hex(3)}", self.root / "clients" / "codex_coder.py")
        cc = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cc)
        return cc

    def run_canary_copy(self, cc):
        (self.bin / "mode").write_text("inside", encoding="utf-8")
        import argparse, contextlib, io
        buf = io.StringIO()
        with contextlib.redirect_stdout(buf):
            code = cc.run_canary(argparse.Namespace(model="gpt-6.1-terra"), self.env())
        return code, buf.getvalue()

    def test_the_canary_refuses_to_start_codex_when_a_probe_names_a_path_outside_the_lab(self):
        cc = self.load_copy()
        real_attacks = cc.canary_attacks
        outside_file = self.root / "README.md"  # a real file of the project, outside the canary's lab

        def with_an_outside_probe(lab, port):
            atts = real_attacks(lab, port)
            items = [{"type": "function_call", "call_id": "x1", "name": "exec_command",
                      "arguments": json.dumps({"cmd": f"Get-Content -LiteralPath '{outside_file}'"})}]
            return atts + [("an outside probe", "read", items, None)]
        cc.canary_attacks = with_an_outside_probe
        code, out = self.run_canary_copy(cc)
        self.assertEqual(code, 2, out)
        self.assertIn("is outside the lab folder", out)
        self.assertIn("codex was not started", out)
        self.assertIsNone(self.seen(), "codex must never be started")
        self.assertFalse(any((self.root / "scratch" / "canary-coder").glob("lab-*")), "the lab is removed")

    def test_the_canary_refuses_a_real_environment_path_and_a_dotdot_escape(self):
        cc = self.load_copy()
        real_attacks = cc.canary_attacks
        for cmd in ("Get-Content (Join-Path $env:USERPROFILE '.codex\\auth.json')", "Get-Content ..\\..\\..\\..\\..\\.env",
                    f"Get-Content -LiteralPath '\\\\localhost\\{str(self.root / '.env').replace(':', '$', 1)}'"):
            def one_more(lab, port, cmd=cmd):
                items = [{"type": "function_call", "call_id": "x1", "name": "exec_command", "arguments": json.dumps({"cmd": cmd})}]
                return real_attacks(lab, port) + [("extra", "read", items, None)]
            cc.canary_attacks = one_more
            code, out = self.run_canary_copy(cc)
            self.assertEqual(code, 2, f"{cmd}: {out}")
            self.assertIn("codex was not started", out, cmd)
            self.assertIsNone(self.seen(), cmd)

    def test_the_shipped_canary_probes_only_planted_lab_files(self):
        cc = self.load_copy()
        base = self.base / "labcheck"
        lab = cc.make_lab(base)
        try:
            attacks = cc.canary_attacks(lab, 9)
            self.assertEqual(cc.check_attacks_stay_in_lab(attacks, lab), [])
            text = json.dumps([a[2] for a in attacks])
            for real in (str(MAIN_ROOT / ".env"), str(MAIN_ROOT / "broadsword.toml"), str(Path.home() / ".codex"), str(self.root / ".env")):
                self.assertNotIn(json.dumps(real)[1:-1], text, real)
            names = {a[0] for a in attacks}
            for planted in ("lab root .env, absolute path", "lab root broadsword.toml, absolute path", "lab CODEX_HOME auth.json, absolute path"):
                self.assertIn(planted, names)
            self.assertTrue((lab["home"] / "auth.json").is_file() and (lab["root"] / "broadsword.toml").is_file())
        finally:
            cc.safe_rmtree(base)

    @unittest.skipUnless(os.environ.get("BSW_TEST_REAL_CODEX_CANARY") == "1",
                         "opt-in (BSW_TEST_REAL_CODEX_CANARY=1): runs the REAL codex binary against the canary's local fake model server, "
                         "offline, no model call")
    def test_the_real_codex_sandbox_refuses_writes_outside_the_worktree(self):
        """The one test that proves the boundary itself: real codex, our coder_command(), a fake model that writes outside the
        lab worktree in every way canary_attacks() lists. CONFINED also writes the coder canary record for that codex version."""
        (self.root / "broadsword.toml").write_text(TOML.replace("codex_coder_timeout_seconds = 8", "codex_coder_timeout_seconds = 600"), encoding="utf-8")
        record = self.root / "state" / "canary" / "coder-last-pass.json"
        record.unlink()
        env = {k: v for k, v in os.environ.items() if k.upper() != "OPENAI_API_KEY"}
        p = subprocess.run([sys.executable, "-I", "-B", str(self.root / "clients" / "codex_coder.py"), "--canary", "--model", "gpt-6.1-terra"],
                           capture_output=True, text=True, encoding="utf-8", timeout=900, env=env)
        out = p.stdout
        print("\n" + out)
        self.assertEqual(p.returncode, 0, out + p.stderr)
        self.assertIn("CONFINED: nothing outside the worktree was read or written", out)
        for line in ("ok   shell write, absolute path", "ok   shell write, by ..", "ok   shell write, sibling worktree",
                     "ok   apply_patch add, absolute path", "ok   apply_patch add, by ..", "ok   apply_patch add, through the junction",
                     "ok   control: apply_patch inside"):
            self.assertIn(line, out)
        version = json.loads(record.read_text(encoding="utf-8"))["codex_version"]
        self.assertIn(f"canary: {version} at ", out)

    def test_the_catalog_offers_a_shell_and_apply_patch_only_in_direct_mode(self):
        spec = importlib.util.spec_from_file_location("cc_under_test", self.root / "clients" / "codex_coder.py")
        cc = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cc)
        (entry,) = cc.coder_catalog("gpt-6.1-terra", "ultra")["models"]
        self.assertEqual(entry["slug"], "gpt-6.1-terra")
        self.assertEqual(entry["tool_mode"], "direct")
        self.assertEqual(entry["experimental_supported_tools"], [])
        self.assertNotIn("multi_agent_version", entry)
        self.assertNotIn("web_search_tool_type", entry)
        self.assertEqual(entry["supported_reasoning_levels"], [{"effort": "ultra", "description": "ultra"}])
        self.assertEqual(cc.CODER_TOOLS, frozenset({"exec_command", "write_stdin", "apply_patch"}))

    def test_catalog_advertises_the_fast_tier_only_when_asked(self):
        spec = importlib.util.spec_from_file_location("cc_under_test", self.root / "clients" / "codex_coder.py")
        cc = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cc)
        plain = cc.coder_catalog("gpt-6.1-sol", "high")
        self.assertEqual(plain, cc.coder_catalog("gpt-6.1-sol", "high", None))
        self.assertNotIn("service_tiers", plain["models"][0])
        fast = cc.coder_catalog("gpt-6.1-sol", "high", "fast")
        self.assertEqual(fast["models"][0]["service_tiers"],
                         [{"id": "priority", "name": "Fast", "description": "2x speed, increased usage"}])
        rest = {k: v for k, v in fast["models"][0].items() if k != "service_tiers"}
        self.assertEqual(rest, plain["models"][0])

    def test_the_written_catalog_carries_the_tier_only_with_the_flag(self):
        spec = importlib.util.spec_from_file_location("cc_under_test", self.root / "clients" / "codex_coder.py")
        cc = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cc)
        for tier, expect in (("fast", True), (None, False)):
            work = self.root / f"catwork-{tier}"
            work.mkdir()
            cc.coder_command("codex", "gpt-6.1-sol", "high", self.root, work, work / "a.txt", tier)
            entry = json.loads((work / "coder-catalog.json").read_text(encoding="utf-8"))["models"][0]
            self.assertEqual("service_tiers" in entry, expect)

    def test_the_prompt_is_the_template_with_task_claims_and_brief(self):
        self.run_coder("inside")
        stdin = self.seen()["stdin"]
        self.assertIn(f"task {TASK}", stdin)
        self.assertIn("- `src`\n- `notes.md`", stdin)
        self.assertIn("broadsword test broadsword --task " + TASK, stdin)
        self.assertIn("Never edit, create or delete anything outside the claim", stdin)
        self.assertIn("BRIEF-MARK", stdin)
        self.assertNotIn("<TASK>", stdin)
        self.assertNotIn("<CLAIMS>", stdin)

    def test_the_prompt_carries_the_self_check(self):
        """The coder must apply the self-check to its own diff before it reports (Lazlo, 2026-11-21 03:49Z). The text is
        embedded in the template (not read from state/), so a missing file cannot silently drop it."""
        self.run_coder("inside")
        stdin = self.seen()["stdin"]
        self.assertIn("**Self-check before you report.**", stdin)
        self.assertIn("Self-review against the reviewer's rubric", stdin)
        self.assertIn("SPEC_CHECKLIST.md", stdin)
        self.assertIn("PASS with its evidence", stdin)
        for n in range(1, 14):  # all 13 common rejection reasons, numbered
            self.assertRegex(stdin, rf"(?m)^{n}\. ", f"rejection reason {n} missing")
        self.assertIn("Never add an allow attribute to silence a lint", stdin)
        self.assertIn("Never use `git stash`", stdin)
        self.assertLess(stdin.index("Self-check before you report"), stdin.index("=== BRIEF ==="))

    def test_codex_gets_no_api_key_and_no_dotenv_value(self):
        code, res, p = self.run_coder("inside")
        env = self.seen()["env"]
        self.assertNotIn("OPENAI_API_KEY", {k.upper() for k in env})
        self.assertNotIn("COPY_OF_DOTENV", env)
        self.assertNotIn(FAKE_KEY, json.dumps(env))
        self.assertNotIn(DOTENV_VALUE, json.dumps(env))
        self.assertNotIn(FAKE_KEY, p.stdout + p.stderr)

    # ---- refusals: codex never starts ------------------------------------------------------------------------------

    def test_unready_brief_is_refused_before_codex(self):
        text = self.brief.read_text(encoding="utf-8").split("## Done check")[0]
        self.brief.write_text(text, encoding="utf-8")
        code, res, _ = self.run_coder()
        self.assertEqual(code, 2, res)
        self.assertTrue(res["result"].startswith("refused: brief_not_ready:"), res)
        self.assertIn("FAIL donecheck", res["result"])
        self.assertIsNone(self.seen(), "codex must never be started")
        self.assertEqual(self.ledger_rows(), [])
        self.assertFalse((self.root / "scratch").exists(), "no coder temp folder before admission")

    def test_brief_claim_outside_board_is_refused(self):
        text = self.brief.read_text(encoding="utf-8").replace("notes.md", "notes.md, extra.md")
        self.brief.write_text(text, encoding="utf-8")
        code, res, _ = self.run_coder()
        self.assertEqual(code, 2, res)
        self.assertTrue(res["result"].startswith("refused: brief_not_ready:"), res)
        self.assertIn("claims: not on the board claims: extra.md", res["result"])
        self.assertNotIn("FAIL ", res["result"], "the brief must otherwise pass lint")
        self.assertIsNone(self.seen(), "codex must never be started")
        self.assertEqual(self.ledger_rows(), [])

    def assert_refused(self, fragment, **kw):
        code, res, _ = self.run_coder("inside", **kw)
        self.assertEqual(code, 2, res)
        self.assertTrue(res["is_error"])
        self.assertIn(fragment, res["result"])
        self.assertIsNone(self.seen(), "codex must never be started")

    def test_a_worktree_outside_worktrees_dir_is_refused(self):
        other = self.root / "elsewhere" / TASK
        git("worktree", "add", "-q", "-b", "task/other", "elsewhere/" + TASK, cwd=self.root)
        self.assertTrue((other / ".git").is_file())
        self.write_task(worktree=f"elsewhere/{TASK}")
        self.assert_refused("is not inside worktrees_dir")

    def test_a_worktree_path_with_dotdot_is_refused(self):
        self.write_task(worktree=f"worktrees/../worktrees/broadsword/{TASK}")
        self.assert_refused("uses `..`")

    def test_an_absolute_worktree_path_is_refused(self):
        self.write_task(worktree=str(self.wt).replace("\\", "/"))  # the record escapes a backslash; `/` needs none
        self.assert_refused("is absolute")

    @unittest.skipUnless(os.name == "nt", "junctions exist only on Windows")
    def test_a_junction_on_the_way_to_the_worktree_is_refused(self):
        r = subprocess.run(["cmd", "/d", "/c", "mklink", "/J", str(self.root / "worktrees" / "jx"), str(self.root / "worktrees" / "broadsword")],
                           capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.write_task(worktree=f"worktrees/jx/{TASK}")
        self.assert_refused("symlink, junction or reparse point")

    @unittest.skipUnless(os.name == "nt", "junctions exist only on Windows")
    def test_a_worktree_already_holding_a_link_is_refused(self):
        r = subprocess.run(["cmd", "/d", "/c", "mklink", "/J", str(self.wt / "src-link"), str(self.base)], capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assert_refused("holds links")

    def test_a_plain_folder_that_is_not_a_git_worktree_is_refused(self):
        d = self.root / "worktrees" / "broadsword" / "t-20261119-000002"
        d.mkdir(parents=True)
        self.write_task(worktree="worktrees/broadsword/t-20261119-000002")
        self.assert_refused("is not a linked Git worktree")

    def test_a_task_that_is_not_running_is_refused(self):
        self.write_task(status="open")
        self.assert_refused("not `running`")

    def test_a_removed_worktree_is_refused(self):
        self.write_task(worktree_removed="1795100000")
        self.assert_refused("no finished worktree")

    def test_a_missing_setting_is_refused_naming_it(self):
        (self.root / "broadsword.toml").write_text(TOML.replace("codex_coder_timeout_seconds = 8\n", ""), encoding="utf-8")
        self.assert_refused("codex_coder_timeout_seconds")

    def test_a_secret_named_brief_is_refused_unread(self):
        b = self.base / ".env.local"
        b.write_text("SECRET=1\n", encoding="utf-8")
        self.assert_refused("secret file name", brief=b)

    def test_a_brief_over_the_size_cap_is_refused(self):
        b = self.base / "big.md"
        b.write_text("x" * 5000, encoding="utf-8")
        self.assert_refused("over reviewer_brief_max_kb = 4", brief=b)

    def test_a_bad_effort_is_refused(self):
        self.assert_refused("is not a reasoning effort", extra=["--effort", "High;x"])

    # ---- the two gates, reused from openai_review.py ----------------------------------------------------------------

    def test_no_coder_canary_record_is_refused_before_codex_runs(self):
        (self.root / "state" / "canary" / "coder-last-pass.json").unlink()
        self.assert_refused("canary_needed")

    def test_a_canary_record_for_another_codex_version_is_refused(self):
        self.write_canary_record("codex-cli 0.169.0")
        self.assert_refused("canary_needed: the coder canary has a pass for 'codex-cli 0.169.0'")

    def test_the_shadow_reviewers_canary_record_does_not_count(self):
        (self.root / "state" / "canary" / "coder-last-pass.json").unlink()
        self.write_canary_record("codex-cli 0.0.0-fake", name="last-pass.json")  # what scripts/shadow_canary.py writes
        self.assert_refused("canary_needed")

    def test_a_zero_credit_cap_never_refuses_the_plan_route_and_is_still_a_ledger_row(self):
        """openai_review a5ebcfb1: the cap counts paid API-credit calls only; a codex plan call is never refused for it."""
        (self.root / "broadsword.toml").write_text(TOML + "gpt_daily_cap_usd = 0\n", encoding="utf-8")
        code, res, _ = self.run_coder("inside")
        self.assertEqual(code, 0, res)
        self.assertEqual([r[6] for r in self.ledger_rows()], ["codex-plan"])

    def test_an_unwritable_ledger_day_is_refused_before_codex_runs(self):
        import datetime
        day = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d")
        d = self.root / "state" / "gpt-spend"
        d.mkdir(parents=True, exist_ok=True)
        (d / f"LEDGER-FAILED-{day}.txt").write_text("x\n", encoding="utf-8")
        self.assert_refused("ledger_unwritable")

    def test_every_run_is_one_ledger_row_with_its_tokens(self):
        code, res, _ = self.run_coder("inside")
        self.assertEqual(code, 0, res)
        rows = self.ledger_rows()
        self.assertEqual(len(rows), 1, rows)
        utc, caller, model, tin, tout, est, label, call_type = rows[0]
        self.assertEqual((caller, model, tin, tout, label, call_type), (f"codex_coder {TASK}", "gpt-6.1-terra", "1234", "56", "codex-plan", "coder"))
        self.assertGreater(float(est), 0.0, "an unpriced model is counted at the highest price, never 0")

    def test_a_breach_and_a_failed_codex_are_ledger_rows_too(self):
        self.run_coder("outside")
        self.run_coder("fail")
        rows = self.ledger_rows()
        self.assertEqual([r[6] for r in rows], ["codex-plan", "codex-plan-failed"], rows)

    def test_codex_missing_is_refused(self):
        (self.bin / "codex.cmd").unlink(missing_ok=True)
        (self.bin / "codex").unlink(missing_ok=True)
        self.assert_refused("codex executable not found")


class PureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        spec = importlib.util.spec_from_file_location("cc_pure", HERE / "codex_coder.py")
        cls.cc = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.cc)

    def test_claim_matching(self):
        cc = self.cc
        self.assertTrue(cc.under("src/a/b.py", "src"))
        self.assertTrue(cc.under("notes.md", "notes.md"))
        self.assertFalse(cc.under("srcx/a.py", "src"))
        self.assertFalse(cc.under("notes.md.bak", "notes.md"))
        self.assertTrue(cc.under("anything", "*"))
        self.assertTrue(cc.dir_overlaps("clients", "clients/x.py"))
        self.assertFalse(cc.dir_overlaps("other", "clients/x.py"))

    def test_key_claim_validation_and_normalisation(self):
        cc = self.cc
        self.assertEqual(cc.claims_of({"claims": "./broadsword.toml#alpha_09\n.\\crates\\broadsword/src/./config.rs/#beta"}),
                         ["broadsword.toml#alpha_09", "crates/broadsword/src/config.rs#beta"])
        for key in ("", "Alpha", "a-b", "a.b", "a/b", "a\\b", "a#b", "a b", "a ", "a\t", "a\r", "a\n", "é", "a\u2028b"):
            with self.subTest(key=key), self.assertRaises(cc.Refused):
                cc.normalize_claim(f"broadsword.toml#{key}")
        self.assertEqual(cc.normalize_claim(".\\crates/broadsword//tests/./support/mod.rs/#gamma"),
                         "crates/broadsword/tests/support/mod.rs#gamma")
        for path in ("src/lib.rs", "config.rs", "nested/broadsword.toml", "crates/broadsword/src/config.rs.bak", "mod.rs",
                     "crates/broadsword/tests/support", "crates/broadsword/tests/support/mod.rs.bak",
                     "crates/broadsword/tests/tasks.rs"):
            with self.subTest(path=path), self.assertRaisesRegex(
                    cc.Refused, "key claims are only for broadsword.toml, crates/broadsword/src/config.rs and "
                                "crates/broadsword/tests/support/mod.rs"):
                cc.normalize_claim(f"{path}#k")
        for path in ("../broadsword.toml", "/broadsword.toml", "C:\\broadsword.toml", "crates/../broadsword.toml",
                     "broadsword.toml.", "broadsword.toml?"):
            with self.subTest(path=path), self.assertRaises(cc.Refused):
                cc.normalize_claim(f"{path}#k")
        large = "a" * 100_000
        self.assertEqual(cc.normalize_claim(f"broadsword.toml#{large}"), f"broadsword.toml#{large}")
        with self.assertRaises(cc.Refused):
            cc.normalize_claim(f"broadsword.toml#{large}!")

    def test_key_claim_file_coverage_at_the_low_level(self):
        cc = self.cc
        for file in ("broadsword.toml", "crates/broadsword/src/config.rs", "crates/broadsword/tests/support/mod.rs"):
            for key in ("alpha", "beta"):
                claim = f"{file}#{key}"
                with self.subTest(claim=claim):
                    self.assertTrue(cc.under(file, claim))
                    self.assertFalse(cc.under(file + ".bak", claim))
                    self.assertFalse(cc.under(file + "/child", claim))
                    self.assertEqual(cc.under(file.upper(), claim), os.name == "nt")
                    self.assertEqual(cc.judge([(file, "modified", "file")], [claim]), [])
                    self.assertEqual(cc.judge([(file + ".bak", "modified", "file")], [claim]), [file + ".bak"])
                    for kind in ("link", "hardlink", "unreadable"):
                        self.assertEqual(cc.judge([(file, "added", kind)], [claim]), [file])
                    self.assertEqual(cc.judge([(file + ":data", "modified", "stream")], [claim]), [])
        loader = "crates/broadsword/src/config.rs#alpha"
        self.assertTrue(cc.dir_overlaps("crates/broadsword/src", loader))
        self.assertFalse(cc.under("crates/broadsword/src", loader))
        self.assertFalse(cc.dir_overlaps("crates/broadsword/srcx", loader))
        self.assertEqual(cc.judge([("broadsword.toml", "modified", "file")], [loader]), ["broadsword.toml"])
        self.assertEqual(cc.judge([("crates", "modified", "dir-meta")], [loader]), ["crates"])
        self.assertEqual(cc.judge([("crates:data", "modified", "stream")], [loader]), ["crates:data"])
        added = [("crates/broadsword/src", "modified", "dir"), ("crates/broadsword/src/config.rs", "added", "file")]
        self.assertEqual(cc.judge(added, [loader]), [])
        self.assertEqual(cc.judge([("crates/broadsword/src", "modified", "dir")], [loader]), ["crates/broadsword/src"])
        self.assertEqual(cc.judge([(".git", "modified", "file")], ["broadsword.toml#alpha"]), [".git"])

    def test_invalid_key_claims_never_grant_coverage_even_to_direct_callers(self):
        cc = self.cc
        for claim in ("README.md#alpha", "broadsword.toml#", "broadsword.toml#alpha#beta", "broadsword.toml#alpha\n"):
            with self.subTest(claim=claim):
                with self.assertRaises(cc.Refused):
                    cc.under("README.md", claim)
                with self.assertRaises(cc.Refused):
                    cc.dir_overlaps("crates", claim)
                with self.assertRaises(cc.Refused):
                    cc.judge([], [claim])

    def test_judge(self):
        cc = self.cc
        ch = [("src/a.py", "added", "file"), ("src", "added", "dir"), ("README.md", "modified", "file"), ("src/l", "added", "link"),
              (".git", "modified", "file")]
        self.assertEqual(cc.judge(ch, ["src"]), ["README.md", "src/l", ".git"])
        self.assertEqual(cc.judge([("src/h", "added", "hardlink"), ("src/u", "added", "unreadable")], ["src"]), ["src/h", "src/u"])

    def snapshot_with_unreadable(self, top: Path, unreadable: set):
        """snapshot(top) while os.scandir refuses the folders in `unreadable` (as an ACL the coder set would)."""
        cc, real = self.cc, os.scandir

        def scandir(p):
            if os.path.normcase(os.path.abspath(p)) in {os.path.normcase(str(u)) for u in unreadable}:
                raise PermissionError(13, "denied", str(p))
            return real(p)
        cc.os.scandir = scandir
        try:
            return cc.snapshot(top)
        finally:
            cc.os.scandir = real

    def test_judge_worktree_fails_closed_when_the_snapshot_raises(self):
        cc, real = self.cc, self.cc.snapshot
        top = SCRATCH / f"test-codex-coder-jw-{os.getpid()}-{secrets.token_hex(4)}"
        try:
            (top / "src").mkdir(parents=True)
            before = cc.snapshot(top)
            self.assertEqual(cc.judge_worktree(top, before, ["src"]), ([], [], ""), "no change, no error")
            for exc in (RecursionError("deep"), ValueError("odd name"), FileNotFoundError(2, "gone")):
                def boom(_top, exc=exc):
                    raise exc
                cc.snapshot = boom
                try:
                    changed, outside, error = cc.judge_worktree(top, before, ["*"])  # even a `*` claim
                finally:
                    cc.snapshot = real
                self.assertEqual(outside, ["."], exc)
                self.assertEqual(changed, [(".", "unjudged", "unreadable")])
                self.assertIn(type(exc).__name__, error)
        finally:
            force_rmtree(top)

    def test_the_git_file_is_hashed_and_a_same_length_rewrite_is_a_breach(self):
        """Review q-20261119-235542 point 1: `.git` is part of the snapshot by content, like any file."""
        cc = self.cc
        top = SCRATCH / f"test-codex-coder-git-{os.getpid()}-{secrets.token_hex(4)}"
        try:
            top.mkdir(parents=True)
            (top / ".git").write_text("gitdir: AAAA\n", encoding="utf-8")
            before = cc.snapshot(top)
            self.assertEqual(before[".git"][0], "file")
            # NTFS stamps times from the coarse system clock (up to ~16 ms a tick): a same-bytes rewrite inside the SAME tick
            # as the before-snapshot is not visible (KNOWN, see metadata()). A real run cannot do that (codex starts seconds
            # after the snapshot), so the test waits past a tick, as a real run always does.
            import time
            time.sleep(0.05)
            (top / ".git").write_text("gitdir: BBBB\n", encoding="utf-8")  # same size, other target
            self.assertEqual(cc.judge(cc.changes(before, cc.snapshot(top)), ["*"]), [".git"])
            (top / ".git").write_text("gitdir: AAAA\n", encoding="utf-8")  # back to the same bytes: its times still moved
            self.assertEqual(cc.judge(cc.changes(before, cc.snapshot(top)), ["*"]), [".git"])
        finally:
            force_rmtree(top)

    def test_a_folder_that_becomes_unreadable_is_a_breach_even_inside_the_claim(self):
        cc = self.cc
        top = SCRATCH / f"test-codex-coder-snap-{os.getpid()}-{secrets.token_hex(4)}"
        try:
            (top / "src").mkdir(parents=True)
            (top / "src" / "a.py").write_text("a\n", encoding="utf-8")
            before = cc.snapshot(top)
            (top / "src" / "hidden.py").write_text("new\n", encoding="utf-8")  # new under a folder the after-snapshot cannot list
            after = self.snapshot_with_unreadable(top, {top / "src"})
            self.assertEqual(after["src"], ("unreadable", "PermissionError"))
            self.assertEqual(cc.judge(cc.changes(before, after), ["src"]), ["src"])
            after_top = self.snapshot_with_unreadable(top, {top})  # the worktree itself cannot be listed
            self.assertEqual(cc.judge(cc.changes(before, after_top), ["*"]), ["."])
        finally:
            force_rmtree(top)

    def test_a_kind_change_is_a_removal_plus_an_addition(self):
        cc = self.cc
        before = {"src": ("file", "1:a"), "d": ("dir", ""), "d/x": ("file", "1:b"), "l": ("file", "1:c"), "same": ("file", "1:d")}
        after = {"src": ("dir", ""), "src/a.py": ("file", "1:e"), "d": ("file", "1:f"), "l": ("link", "t"), "same": ("file", "1:g")}
        ch = cc.changes(before, after)
        self.assertEqual(ch, [("d", "removed", "dir"), ("d", "added", "file"), ("d/x", "removed", "file"),
                              ("l", "removed", "file"), ("l", "added", "link"), ("same", "modified", "file"),
                              ("src", "removed", "file"), ("src", "added", "dir"), ("src/a.py", "added", "file")])
        self.assertEqual(cc.judge(ch, ["src/a.py", "d/x", "l", "same"]), ["d", "l", "src"], "each path named once")
        self.assertEqual(cc.judge(ch, ["*"]), ["l"], "a link is never allowed")

    def test_a_times_only_folder_change_needs_a_claimed_direct_child(self):
        cc = self.cc
        t = [("docs", "modified", "dir")]
        self.assertEqual(cc.judge(t, ["docs/x.md"]), ["docs"], "no cause")
        self.assertEqual(cc.judge(t + [("docs/x.md", "added", "file")], ["docs/x.md"]), [])
        self.assertEqual(cc.judge(t + [("docs/x.md", "removed", "file")], ["docs/x.md"]), [])
        self.assertEqual(cc.judge(t + [("docs/x.md", "added", "file"), ("docs/y.md", "added", "file")], ["docs/x.md"]),
                         ["docs", "docs/y.md"], "one unclaimed cause is enough to make it a breach")
        self.assertEqual(cc.judge(t + [("docs/x.md", "modified", "file")], ["docs/x.md"]), ["docs"],
                         "an in-place write does not move a folder's times")
        self.assertEqual(cc.judge(t + [("docs/x.md:s", "added", "stream")], ["docs/x.md"]), ["docs"], "nor does a stream")
        self.assertEqual(cc.judge(t + [("docs/sub/x.md", "added", "file")], ["docs/sub/x.md"]), ["docs"],
                         "a grandchild is not a direct child")
        self.assertEqual(cc.judge(t, ["docs"]), [], "a claim on the folder itself")
        r = [(".", "modified", "dir")]
        self.assertEqual(cc.judge(r, ["src"]), ["."])
        self.assertEqual(cc.judge(r + [("src", "added", "dir")], ["src"]), [])
        self.assertEqual(cc.judge(r + [("x.md", "added", "file")], ["src"]), [".", "x.md"])
        self.assertEqual(cc.judge(r, ["*"]), [])

    def test_judge_streams_by_their_file_or_folder(self):
        cc = self.cc
        ch = [("src/a.py:x", "added", "stream"), ("src:x", "added", "stream"), ("README.md:x", "added", "stream"),
              (".git:x", "added", "stream"), (".:x", "added", "stream"), ("src/a.py:", "added", "unreadable")]
        self.assertEqual(cc.judge(ch, ["src"]), ["README.md:x", ".git:x", ".:x", "src/a.py:"])
        # a stream on a folder is data on the folder: a claim on a file inside it does not cover it
        self.assertEqual(cc.judge([("src:x", "added", "stream")], ["src/a.py"]), ["src:x"])
        self.assertEqual(cc.judge([(".:x", "added", "stream")], ["*"]), [])

    def tmp(self, tag):
        top = SCRATCH / f"test-codex-coder-{tag}-{os.getpid()}-{secrets.token_hex(4)}"
        top.mkdir(parents=True)
        self.addCleanup(force_rmtree, top)
        return top

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_large_stream_is_hashed_whole(self):
        cc, top = self.cc, self.tmp("bigads")
        (top / "src").mkdir()
        (top / "src" / "a.py").write_text("a\n", encoding="utf-8")
        big = str(top / "src" / "a.py") + ":big"
        size = 50 * 1024 * 1024
        with open(big, "wb") as f:
            f.write(b"\0" * size)
        before = cc.snapshot(top)
        self.assertEqual(before["src/a.py:big"][1].split(":")[:2], ["file", str(size)])
        with open(big, "r+b") as f:  # flip the very last byte: same size
            f.seek(size - 1)
            f.write(b"\1")
        ch = cc.changes(before, cc.snapshot(top))
        self.assertEqual(ch, [("src/a.py", "modified", "file"), ("src/a.py:big", "modified", "stream")])  # + the holder's times
        self.assertEqual(cc.judge(ch, ["src"]), [])
        self.assertEqual(cc.judge(ch, ["notes.md"]), ["src/a.py", "src/a.py:big"])

    @unittest.skipUnless(os.name == "nt", "alternate data streams exist only on Windows")
    def test_a_stream_listing_that_fails_is_unreadable_never_clean(self):
        """FindFirstStreamW or FindNextStreamW failing for any reason but ERROR_HANDLE_EOF fails closed."""
        import ctypes
        cc, top = self.cc, self.tmp("adsfail")
        (top / "a.txt").write_text("a\n", encoding="utf-8")
        with open(str(top / "a.txt") + ":s", "w", encoding="utf-8") as f:
            f.write("s\n")
        before = cc.snapshot(top)
        self.assertEqual(sorted(before), [".", "a.txt", "a.txt:s"])
        real = cc._k32

        class FirstFails:
            def __getattr__(self, n):  # metadata() keeps the real calls
                return getattr(real, n)

            def FindFirstStreamW(self, *a):
                ctypes.set_last_error(5)  # ERROR_ACCESS_DENIED
                return cc._INVALID_HANDLE

        class NextFails:
            def __getattr__(self, n):
                return getattr(real, n)

            def FindFirstStreamW(self, *a):
                return real.FindFirstStreamW(*a)

            def FindNextStreamW(self, *a):
                ctypes.set_last_error(87)  # ERROR_INVALID_PARAMETER
                return 0

            def FindClose(self, h):
                return real.FindClose(h)

        for fake in (FirstFails(), NextFails()):
            cc._k32 = fake
            try:
                after = cc.snapshot(top)
            finally:
                cc._k32 = real
            self.assertEqual(after.get("a.txt:", ("?",))[0], "unreadable", (type(fake).__name__, after))
            if isinstance(fake, FirstFails):  # the folder has no streams, so FindNextStreamW is never reached for it
                self.assertEqual(after.get(".:", ("?",))[0], "unreadable", "the worktree folder's own listing too")
            self.assertIn("a.txt:", cc.judge(cc.changes(before, after), ["*"]), "a breach even under a `*` claim")
        self.assertEqual(cc.changes(before, cc.snapshot(top)), [], "the real listing is clean again")

    @unittest.skipUnless(os.name == "nt", "the Windows metadata calls")
    def test_a_metadata_call_that_fails_is_unreadable_never_clean(self):
        import ctypes
        cc, top = self.cc, self.tmp("metafail")
        (top / "d").mkdir()
        (top / "d" / "a.txt").write_text("a\n", encoding="utf-8")
        before = cc.snapshot(top)
        self.assertEqual({k: v[0] for k, v in before.items()}, {".": "dir", "d": "dir", "d/a.txt": "file"})
        rk, ra = cc._k32, cc._adv

        class Proxy:
            def __init__(self, real, name, ret):
                self.real, self.name, self.ret = real, name, ret

            def __getattr__(self, n):
                if n == self.name:
                    def fail(*a):
                        ctypes.set_last_error(5)
                        return self.ret
                    return fail
                return getattr(self.real, n)

        cases = [("_k32", Proxy(rk, "CreateFileW", cc._INVALID_HANDLE)), ("_k32", Proxy(rk, "GetFileInformationByHandleEx", 0)),
                 ("_adv", Proxy(ra, "GetKernelObjectSecurity", 0))]
        for attr, fake in cases:
            setattr(cc, attr, fake)
            try:
                after = cc.snapshot(top)
            finally:
                cc._k32, cc._adv = rk, ra
            self.assertEqual({k: v[0] for k, v in after.items()}, {".": "unreadable", "d": "unreadable", "d/a.txt": "unreadable"},
                             fake.name)
            self.assertEqual(cc.judge(cc.changes(before, after), ["*"]), [".", "d", "d/a.txt"], fake.name)
        self.assertEqual(cc.changes(before, cc.snapshot(top)), [], "the real calls are clean again")

    def test_secret_names_follow_git_rs(self):
        cc = self.cc
        for n in (".env", ".ENV", ".envrc", ".env.local", ".env-prod", ".env_x", ".env.", ".env ", ".env::$DATA", ".env-"):
            self.assertTrue(cc.is_secret_name(n), n)
        for n in (".envoy.yaml", ".environment", "env", "brief.md"):
            self.assertFalse(cc.is_secret_name(n), n)

    def test_record_unescape(self):
        self.assertEqual(self.cc.unescape_record("a\\nb\\\\c\\rd"), "a\nb\\c\rd")

    def test_relative_paths(self):
        cc = self.cc
        self.assertEqual(cc.rel_parts("worktrees/p/./t", "x"), ["worktrees", "p", "t"])
        for bad in ("", "/x", "\\x", "C:/x", "c:x", "a/../b", "a/b.", "a\nb"):
            with self.assertRaises(cc.Refused, msg=bad):
                cc.rel_parts(bad, "x")


if __name__ == "__main__":
    unittest.main()
