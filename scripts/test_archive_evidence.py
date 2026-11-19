"""Tests for scripts/archive_evidence.py. Each test builds a fake Broadsword root under the project's scratch folder and
removes it afterwards, so the real state/ is never read or written.

  python -I -B scripts/test_archive_evidence.py -v

BS_ARCHIVE_SCRIPT=<path> runs the same tests against another copy of the script (used to prove the tests can fail).
"""
import sys

sys.dont_write_bytecode = True

import hashlib
import importlib.util
import os
import pathlib
import shutil
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
SCRIPT = pathlib.Path(os.environ.get("BS_ARCHIVE_SCRIPT") or HERE / "archive_evidence.py")
SCRATCH = HERE.parent / "scratch"
OPENAI_MCP = HERE.parent / "clients" / "openai_mcp.py"

FAKE_KEY = "zq9-FAKE-key-value-123456"
CONFIG = 'root = "."\nstate_dir = "state"\nscratch_dir = "scratch"\nredact_min_chars = 6\n'


def load(path):
    spec = importlib.util.spec_from_file_location("mod_under_test", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def rmtree(path):
    """Remove a test root; Git marks object files read-only, which Windows refuses to delete."""
    def onerror(func, p, _exc):
        os.chmod(p, 0o700)
        func(p)
    shutil.rmtree(path, onerror=onerror)


class Base(unittest.TestCase):
    def setUp(self):
        SCRATCH.mkdir(exist_ok=True)
        self.root = pathlib.Path(tempfile.mkdtemp(prefix="archive-test-", dir=SCRATCH))
        self.addCleanup(rmtree, self.root)
        (self.root / "broadsword.toml").write_text(CONFIG)
        (self.root / "state" / "queue").mkdir(parents=True)
        (self.root / "state" / "jobs").mkdir(parents=True)
        (self.root / "scratch" / "logs" / "jobs").mkdir(parents=True)
        self.archive = self.root / "state" / "evidence-archive"

    def queue(self, name, content):
        p = self.root / "state" / "queue" / name
        p.write_bytes(content if isinstance(content, bytes) else content.encode())
        return p

    def job(self, jid, status, log_text=None, log_path=None):
        logname = f"{jid}.log"
        recorded = log_path or f"C:\\\\Other\\\\Machine\\\\scratch\\\\logs\\\\jobs\\\\{logname}"
        (self.root / "state" / "jobs" / f"{jid}.rec").write_text(f"id = {jid}\nlog = {recorded}\nstatus = {status}\n")
        if log_text is not None:
            (self.root / "scratch" / "logs" / "jobs" / logname).write_text(log_text)

    def run_cli(self, *extra, env=None):
        e = {k: v for k, v in os.environ.items() if k.upper() != "OPENAI_API_KEY"}
        e.update(env or {})
        return subprocess.run([sys.executable, "-I", "-B", str(SCRIPT), "--root", str(self.root), "--date", "2026-11-19", *extra],
                              capture_output=True, text=True, env=e)

    def archived(self):
        day = self.archive / "2026-11-19"
        return sorted(p.relative_to(day).as_posix() for p in day.rglob("*") if p.is_file()) if day.exists() else []


class Copying(Base):
    def test_copies_queue_and_failed_logs_only(self):
        self.queue("q-1.entry", "entry one")
        self.queue("q-1.review", "review one")
        self.queue("queue.lock", "")
        self.job("j-fail", "failed", "boom log")
        self.job("j-done", "done", "fine log")
        r = self.run_cli()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.archived(), ["jobs/j-fail.log", "queue/q-1.entry", "queue/q-1.review"])
        self.assertEqual((self.archive / "2026-11-19" / "jobs" / "j-fail.log").read_text(), "boom log")
        # committed in its own repo
        log = subprocess.run(["git", "log", "--oneline"], cwd=self.archive, capture_output=True, text=True)
        self.assertEqual(len(log.stdout.strip().splitlines()), 1, log.stdout)
        self.assertTrue((self.archive / ".git").is_dir())

    def test_never_moves_or_deletes_sources(self):
        q = self.queue("q-1.entry", "keep me")
        self.job("j-fail", "failed", "keep log")
        self.run_cli()
        self.assertEqual(q.read_text(), "keep me")
        self.assertTrue((self.root / "scratch" / "logs" / "jobs" / "j-fail.log").exists())
        self.assertTrue((self.root / "state" / "jobs" / "j-fail.rec").exists())

    def test_second_run_copies_nothing(self):
        self.queue("q-1.entry", "entry one")
        self.job("j-fail", "failed", "boom")
        self.assertEqual(self.run_cli().returncode, 0)
        before = subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.archive, capture_output=True, text=True).stdout
        r = self.run_cli("--date", "2026-11-20")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("0 files, 0 bytes", r.stdout)
        self.assertIn("2 already archived", r.stdout)
        self.assertFalse((self.archive / "2026-11-20").exists())
        after = subprocess.run(["git", "rev-parse", "HEAD"], cwd=self.archive, capture_output=True, text=True).stdout
        self.assertEqual(before, after)

    def test_changed_file_is_archived_again(self):
        self.queue("q-1.entry", "v1")
        self.run_cli()
        self.queue("q-1.entry", "v2")
        r = self.run_cli("--date", "2026-11-21")
        self.assertIn("1 files", r.stdout)
        self.assertEqual((self.archive / "2026-11-21" / "queue" / "q-1.entry").read_text(), "v2")

    def test_missing_log_is_counted_not_an_error(self):
        self.job("j-gone", "failed", None)
        r = self.run_cli()
        self.assertEqual(r.returncode, 0)
        self.assertIn("1 failed-job logs missing", r.stdout)

    def test_dry_run_lists_and_writes_nothing(self):
        self.queue("q-1.entry", "12345")
        self.job("j-fail", "failed", "1234567")
        r = self.run_cli("--dry-run")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("would copy queue/q-1.entry (5 bytes)", r.stdout)
        self.assertIn("dry run: 2 files, 12 bytes would be copied", r.stdout)
        self.assertFalse(self.archive.exists())

    def test_unreadable_file_is_an_error_naming_it(self):
        self.queue("q-ok.entry", "fine")
        bad = self.queue("q-bad.entry", "x")
        mod = load(SCRIPT)
        orig = pathlib.Path.read_bytes

        def fake(self_):
            if self_.name == "q-bad.entry":
                raise PermissionError("denied")
            return orig(self_)

        pathlib.Path.read_bytes = fake
        try:
            lines = []
            rc = mod.run(self.root, False, "2026-11-19", {}, out=lines.append)
        finally:
            pathlib.Path.read_bytes = orig
        self.assertEqual(rc, 1)
        self.assertTrue(any("ERROR" in ln and "q-bad.entry" in ln for ln in lines), lines)
        self.assertEqual(self.archived(), ["queue/q-ok.entry"])


class Hardening(Base):
    def git_out(self, repo, *args, env=None):
        e = {k: v for k, v in os.environ.items() if not k.upper().startswith("GIT_")}
        return subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True, env=e).stdout.strip()

    def test_a_held_lock_refuses_a_second_run_with_exit_1(self):
        self.queue("q-1.entry", "x")
        lock_path = self.root / "state" / "evidence-archive.lock"
        f = open(lock_path, "a+b")  # the test's own OS lock, taken the way a running archive would
        try:
            if os.name == "nt":
                import msvcrt
                f.seek(0)
                msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
            else:
                import fcntl
                fcntl.flock(f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
            r = self.run_cli()
            self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
            self.assertIn("lock", r.stderr)
            self.assertEqual(self.archived(), [])
            self.assertFalse(self.archive.exists())
        finally:
            f.close()
        r = self.run_cli()  # released: the next run works (no stale lock)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.archived(), ["queue/q-1.entry"])

    def test_manifest_is_written_only_after_the_commit_succeeds(self):
        self.queue("q-1.entry", "x")
        self.assertEqual(self.run_cli().returncode, 0)
        manifest = self.archive / "manifest.tsv"
        self.assertIn("queue/q-1.entry", manifest.read_text())
        # a commit hook that fails: the files are staged but the manifest must not record them
        self.queue("q-2.entry", "y")
        hook = self.archive / ".git" / "hooks" / "pre-commit"
        hook.write_text("#!/bin/sh\nexit 1\n")
        hook.chmod(0o755)
        r = self.run_cli("--date", "2026-11-20")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertNotIn("q-2.entry", manifest.read_text())
        hook.unlink()
        r = self.run_cli("--date", "2026-11-20")  # retried, now recorded
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("queue/q-2.entry", manifest.read_text())
        self.assertEqual(self.git_out(self.archive, "status", "--porcelain"), "")

    def test_failed_manifest_amend_restores_the_manifest_so_the_next_run_retries(self):
        self.queue("q-1.entry", "x")
        self.assertEqual(self.run_cli().returncode, 0)
        manifest = self.archive / "manifest.tsv"
        before = manifest.read_bytes()
        self.queue("q-2.entry", "y")
        hook = self.archive / ".git" / "hooks" / "pre-commit"  # fails only when the manifest is part of the commit
        hook.write_text("#!/bin/sh\ngit diff --cached --name-only | grep -q manifest.tsv && exit 1\nexit 0\n")
        hook.chmod(0o755)
        r = self.run_cli("--date", "2026-11-20")
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertEqual(manifest.read_bytes(), before)
        hook.unlink()
        r = self.run_cli("--date", "2026-11-20")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("1 files", r.stdout)
        self.assertIn("queue/q-2.entry", manifest.read_text())
        self.assertEqual(self.git_out(self.archive, "status", "--porcelain"), "")

    def test_manifest_that_is_a_directory_is_a_named_error(self):
        self.queue("q-1.entry", "x")
        (self.archive / "manifest.tsv").mkdir(parents=True)
        for extra in ([], ["--dry-run"]):
            r = self.run_cli(*extra)
            self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
            self.assertIn("manifest", r.stderr)
            self.assertNotIn("Traceback", r.stderr)
        self.assertEqual(self.archived(), [])

    def test_git_environment_cannot_redirect_the_archive_repo(self):
        decoy = self.root / "decoy"
        decoy.mkdir()
        subprocess.run(["git", "init", "-q", str(decoy)], check=True, capture_output=True)
        self.queue("q-1.entry", "x")
        r = self.run_cli(env={"GIT_DIR": str(decoy / ".git"), "GIT_WORK_TREE": str(decoy),
                              "GIT_INDEX_FILE": str(decoy / "idx")})
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertTrue((self.archive / ".git").is_dir())
        self.assertEqual(self.git_out(self.archive, "log", "--oneline").count("\n"), 0)
        self.assertEqual(self.git_out(self.archive, "rev-parse", "--show-toplevel").lower().replace("\\", "/"),
                         self.archive.resolve().as_posix().lower())
        self.assertEqual(subprocess.run(["git", "rev-parse", "-q", "--verify", "HEAD"], cwd=decoy, capture_output=True,
                                        env={k: v for k, v in os.environ.items() if not k.upper().startswith("GIT_")}).returncode, 1)
        self.assertFalse((decoy / "2026-11-19").exists())

    def test_unreadable_manifest_is_a_named_error_not_a_traceback(self):
        self.queue("q-1.entry", "x")
        self.archive.mkdir(parents=True)
        (self.archive / "manifest.tsv").write_bytes(b"2026\tqueue/a\t\xff\xfe\xfa\n")
        r = self.run_cli()
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("manifest", r.stderr)
        self.assertNotIn("Traceback", r.stderr)
        self.assertEqual(self.archived(), [])


class Secrets(Base):
    def test_env_files_are_never_copied(self):
        self.queue(".env", f"OPENAI_API_KEY={FAKE_KEY}\n")
        self.queue(".env.local", "X=1\n")
        self.queue("notes.entry", "hello")
        (self.root / "state" / "queue" / "sub").mkdir()
        (self.root / "state" / "queue" / "sub" / ".env").write_text(f"A_TOKEN={FAKE_KEY}\n")
        r = self.run_cli()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.archived(), ["queue/notes.entry"])
        self.assertIn("REFUSED queue/.env:", r.stdout)
        self.assertNotIn(FAKE_KEY, r.stdout + r.stderr)
        self.assertNotIn(FAKE_KEY, self.all_archive_text())

    def test_renamed_env_and_private_key_refused_by_content(self):
        self.queue("renamed.txt", f"# c\nSERVICE_API_KEY={FAKE_KEY}\nREGION=eu\n")
        self.queue("k.entry", "x\n-----BEGIN RSA PRIVATE KEY-----\nabc\n-----END RSA PRIVATE KEY-----\n")
        self.queue("plain.entry", "KEY=value is mentioned in a sentence here")
        r = self.run_cli()
        self.assertEqual(self.archived(), ["queue/plain.entry"])
        self.assertIn("looks like a .env", r.stdout)
        self.assertIn("private key", r.stdout)

    def test_log_with_env_value_is_redacted(self):
        (self.root / ".env").write_text(f"OPENAI_API_KEY={FAKE_KEY}\nSHORT=ab\n")
        self.job("j-fail", "failed", f"calling with {FAKE_KEY} ok; also sk-abcdef123456 and ab stays\n")
        self.queue("q-1.review", f"key was {FAKE_KEY}")
        r = self.run_cli()
        self.assertEqual(r.returncode, 0, r.stderr)
        text = (self.archive / "2026-11-19" / "jobs" / "j-fail.log").read_text()
        self.assertEqual(text, "calling with [REDACTED] ok; also [REDACTED] and ab stays\n")
        self.assertEqual((self.archive / "2026-11-19" / "queue" / "q-1.review").read_text(), "key was [REDACTED]")
        self.assertNotIn(FAKE_KEY, self.all_archive_text())

    def test_secret_named_environment_variable_is_redacted(self):
        self.queue("q-1.entry", "token zz-env-secret-value here")
        self.run_cli(env={"MY_SERVICE_TOKEN": "zz-env-secret-value"})
        self.assertEqual((self.archive / "2026-11-19" / "queue" / "q-1.entry").read_text(), "token [REDACTED] here")

    def test_agrees_with_clients_openai_mcp_redact(self):
        mod = load(SCRIPT)
        oc = load(OPENAI_MCP)
        oc._SECRETS.clear()
        oc._SECRETS.add(FAKE_KEY)
        sample = f"a {FAKE_KEY} b sk-live_abc123 c sk-x d {FAKE_KEY}{FAKE_KEY} e plain text"
        self.assertEqual(mod.redact(sample, [FAKE_KEY]), oc.redact(sample))
        oc._SECRETS.clear()

    def test_secret_name_matches_reviewer_rule(self):
        mod = load(SCRIPT)
        for n in ["OPENAI_API_KEY", "x_token", "MY_PAT", "DB_PASSWORD", "A_CREDENTIALS", "X_AUTH_Y", "TOKEN", "PASS", "api_key"]:
            self.assertTrue(mod.secret_name(n), n)
        for n in ["PATH", "REGION", "KEYBOARD", "PASSWORDS", "HOME"]:
            self.assertFalse(mod.secret_name(n), n)

    def test_secret_file_name_rule_matches_git_rs(self):
        mod = load(SCRIPT)
        for n in [".env", ".ENV", ".envrc", ".env.local", ".env.", ".env ", ".env-prod", ".env_x", ".env-", ".env_", ".env::$DATA", "x:.env"]:
            self.assertTrue(mod.name_refusal(n), n)
        for n in [".envoy.yaml", ".environment", "env.txt", "my.env", ".envrcx", "notes.entry"]:
            self.assertIsNone(mod.name_refusal(n), n)

    def test_envoy_and_environment_files_are_archived_envrc_is_not(self):
        self.queue(".envoy.yaml", "a: 1")
        self.queue(".environment", "b")
        self.queue(".envrc", "use nix")
        r = self.run_cli()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.archived(), ["queue/.environment", "queue/.envoy.yaml"])
        self.assertIn("REFUSED queue/.envrc:", r.stdout)

    def test_failed_log_that_trips_content_check_is_archived_redacted(self):
        self.job("j-key", "failed", "before\n-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----\nafter\n")
        self.job("j-dotenv", "failed", f"# c\nSERVICE_API_KEY={FAKE_KEY}\nREGION=eu\n")
        r = self.run_cli()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertEqual(self.archived(), ["jobs/j-dotenv.log", "jobs/j-key.log"])
        key = (self.archive / "2026-11-19" / "jobs" / "j-key.log").read_text()
        self.assertEqual(key, "before\n[REDACTED]\nafter\n")
        dot = (self.archive / "2026-11-19" / "jobs" / "j-dotenv.log").read_text()
        self.assertEqual(dot, "# c\nSERVICE_API_KEY=[REDACTED]\nREGION=[REDACTED]\n")
        self.assertNotIn(FAKE_KEY, self.all_archive_text())
        self.assertNotIn("MIIabc", self.all_archive_text())
        # a queue file with the same content is still refused whole
        self.queue("renamed.txt", f"SERVICE_API_KEY={FAKE_KEY}\n")
        r = self.run_cli("--date", "2026-11-20")
        self.assertIn("REFUSED queue/renamed.txt", r.stdout)

    SEPARATORS = {"cr": "\r", "vt": "\x0b", "ff": "\x0c", "fs": "\x1c", "gs": "\x1d", "rs": "\x1e", "nel": "\x85",
                  "ls": " ", "ps": " ", "crlf": "\r\n"}

    def test_scrub_covers_every_line_separator_the_check_splits_on(self):
        for tag, sep in self.SEPARATORS.items():
            jid = f"j-{tag}"
            (self.root / "state" / "jobs" / f"{jid}.rec").write_text(f"id = {jid}\nlog = x\\{jid}.log\nstatus = failed\n")
            text = f"# x{sep}DB_PASSWORD=hunter2hunter2\n"
            (self.root / "scratch" / "logs" / "jobs" / f"{jid}.log").write_bytes(text.encode("utf-8"))
        r = self.run_cli()
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertNotIn("hunter2hunter2", self.all_archive_text())
        self.assertNotIn("hunter2hunter2", r.stdout + r.stderr)
        for tag in self.SEPARATORS:  # each one is archived redacted (not silently dropped), or refused whole: never clear
            p = self.archive / "2026-11-19" / "jobs" / f"j-{tag}.log"
            self.assertTrue(p.exists(), tag)
            self.assertIn("DB_PASSWORD=[REDACTED]", p.read_text(encoding="utf-8"), tag)

    def test_scrub_that_still_leaves_a_secret_is_refused_whole(self):
        mod = load(SCRIPT)
        self.assertIsNotNone(mod.scrub_residue("A_KEY=abc\n"))
        self.assertIsNotNone(mod.scrub_residue("-----BEGIN PRIVATE KEY-----\n"))
        self.assertIsNone(mod.scrub_residue("# c\nA_KEY=[REDACTED]\n"))
        self.job("j-fail", "failed", "# c\nDB_PASSWORD=hunter2hunter2\n")
        orig = mod.scrub_content
        mod.scrub_content = lambda t: t  # a scrub that missed everything
        try:
            lines = []
            rc = mod.run(self.root, False, "2026-11-19", {}, out=lines.append)
        finally:
            mod.scrub_content = orig
        self.assertEqual(rc, 0, lines)
        self.assertEqual(self.archived(), [])
        self.assertTrue(any("REFUSED jobs/j-fail.log" in ln for ln in lines), lines)
        self.assertFalse(any("hunter2hunter2" in ln for ln in lines), lines)

    def test_log_whose_name_marks_a_secret_is_still_refused_whole(self):
        self.job("j-secret", "failed", "boom")
        r = self.run_cli()
        self.assertEqual(self.archived(), [])
        self.assertIn("REFUSED jobs/j-secret.log", r.stdout)

    FIXTURE = (
        "# fixture\nREGION=eu-west-1\nGITHUB_PAT=ghp_abcdef123456\nexport DB_PASSWORD=\"hunter2hunter2\"\n"
        "TINY=ab\nEMPTY=\nNOTE='quoted value here'\n"
    )

    def test_parser_and_redaction_agree_with_reviewer_and_openai_mcp_on_one_fixture(self):
        mod = load(SCRIPT)
        oc = load(OPENAI_MCP)
        (self.root / ".env").write_text(self.FIXTURE)
        values = mod.env_file_values(self.root)
        self.assertEqual(values, ["eu-west-1", "ghp_abcdef123456", "hunter2hunter2", "ab", "quoted value here"])
        secrets = mod.collect_secrets(self.root, {}, 6)
        self.assertEqual(secrets, ["quoted value here", "ghp_abcdef123456", "hunter2hunter2", "eu-west-1"])
        sample = "r=eu-west-1 p=ghp_abcdef123456 q=hunter2hunter2 t=ab n=quoted value here sk-live_abc123"
        expected = "r=[REDACTED] p=[REDACTED] q=[REDACTED] t=ab n=[REDACTED] [REDACTED]"
        self.assertEqual(mod.redact(sample, secrets), expected)
        oc._SECRETS.clear()
        oc._SECRETS.update(secrets)
        self.assertEqual(oc.redact(sample), expected)
        oc._SECRETS.clear()
        # reviewer.rs: its own test asserts the same fixture's redaction ("r=[REDACTED] p=[REDACTED] q=[REDACTED] t=ab")
        rs = (HERE.parent / "crates" / "broadsword" / "src" / "reviewer.rs").read_text(encoding="utf-8")
        self.assertIn('"# c\\nREGION=eu-west-1\\nGITHUB_PAT=ghp_abcdef123456\\nexport DB_PASSWORD=\\"hunter2hunter2\\"', rs)
        self.assertIn('"r=[REDACTED] p=[REDACTED] q=[REDACTED] t=ab"', rs)

    def test_secret_name_lists_come_from_reviewer_rs(self):
        import re
        mod = load(SCRIPT)
        rs = (HERE.parent / "crates" / "broadsword" / "src" / "reviewer.rs").read_text(encoding="utf-8")
        m = re.search(r"fn secret_names_and_values\(\) \{\s*for n in \[(.*?)\] \{\s*assert!\(secret_name.*?for n in \[(.*?)\] \{", rs, re.S)
        self.assertIsNotNone(m, "reviewer.rs test list not found")
        yes, no = (re.findall(r'"([^"]+)"', g) for g in m.groups())
        self.assertGreater(len(yes), 5)
        self.assertGreater(len(no), 5)
        for n in yes:
            self.assertTrue(mod.secret_name(n), n)
        for n in no:
            self.assertFalse(mod.secret_name(n), n)
        # and the function's own suffix list
        fn = re.search(r"pub fn secret_name.*?\n\}", rs, re.S).group(0)
        for suffix in re.findall(r'"(_[A-Z]+)"', fn):
            self.assertTrue(mod.secret_name("X" + suffix), suffix)

    def all_archive_text(self):
        out = ""
        for p in self.archive.rglob("*"):
            if p.is_file() and ".git" not in p.parts:
                out += p.read_bytes().decode("utf-8", "replace")
        return out


class Settings(Base):
    def test_missing_required_key_is_an_error(self):
        (self.root / "broadsword.toml").write_text('state_dir = "state"\nscratch_dir = "scratch"\n')
        r = self.run_cli()
        self.assertEqual(r.returncode, 2)
        self.assertIn("redact_min_chars", r.stderr)


if __name__ == "__main__":
    unittest.main()
