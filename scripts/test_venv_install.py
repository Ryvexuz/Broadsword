"""Tests for the venv-install.ps1 safety boundary.

Each case runs a copy in a throwaway root. Dry runs check refusal before writes; real venv cases use the
interpreter's bundled pip wheel, already installed in the test venv, without downloading packages.

    python -I -B scripts/test_venv_install.py
    python -I -B scripts/test_venv_install.py --prove-fail

Each weakening must cause its named behavioural assertions to fail; missing targets and test errors are BAD.
"""
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest
import uuid

SCRIPTS = os.path.dirname(os.path.abspath(__file__))
NAMES = ("venv-install.ps1",)
# Set only by --prove-fail: {script name: [(old, new), ...]} applied to the temp copies.
WEAKEN = {}
GIT_ENV = {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t", "GIT_COMMITTER_EMAIL": "t@t"}


def ps(script, args, cwd, timeout=120):
    env = dict(os.environ)
    env.update(GIT_ENV)
    return subprocess.run(["powershell", "-NoProfile", "-File", script] + list(args), cwd=cwd, env=env,
                          capture_output=True, text=True, timeout=timeout)


def wr(path, text, mode="w"):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, mode, newline="") as f:
        f.write(text)


def rd(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read()


def git(repo, *args):
    env = dict(os.environ)
    env.update(GIT_ENV)
    r = subprocess.run(["git", "-C", repo] + list(args), env=env, capture_output=True, text=True, timeout=120)
    if r.returncode:
        raise RuntimeError(f"git fixture command failed: {args!r}: {r.stderr}")
    return r.stdout.strip()


def junction(link, target):
    """Create a required test junction (mklink /J needs no admin), or report a test error."""
    os.makedirs(target, exist_ok=True)
    r = subprocess.run(["cmd", "/c", "mklink", "/J", link, target], capture_output=True, timeout=120)
    if r.returncode:
        raise RuntimeError(f"cannot create test junction: {r.stdout!r} {r.stderr!r}")


def outside_dir(test):
    d = os.path.join(os.path.realpath(tempfile.gettempdir()), "gsx" + uuid.uuid4().hex[:10])
    os.makedirs(d)
    test.addCleanup(shutil.rmtree, d, True)
    return d


def make_root():
    # realpath: the long form of the temp folder (git reports long paths; the 8.3 form would not compare equal).
    root = os.path.join(os.path.realpath(tempfile.gettempdir()), "gs" + uuid.uuid4().hex[:10])
    os.makedirs(os.path.join(root, "scripts"))
    for name in NAMES:
        text = rd(os.path.join(SCRIPTS, name))
        for old, new in WEAKEN.get(name, []):
            if old not in text:
                raise RuntimeError(f"weakening target missing in {name}: {old}")
            text = text.replace(old, new)
        with open(os.path.join(root, "scripts", name), "w", encoding="utf-8") as f:
            f.write(text)
    return root


class Base(unittest.TestCase):
    script_name = None

    def setUp(self):
        self.root = make_root()
        self.addCleanup(shutil.rmtree, self.root, True)
        self.script = os.path.join(self.root, "scripts", self.script_name)

    def run_s(self, *args, timeout=120):
        return ps(self.script, args, self.root, timeout=timeout)

    def assertRefused(self, *args):
        r = self.run_s(*args)
        self.assertEqual(r.returncode, 2, (args, r.stdout, r.stderr))
        self.assertIn("REFUSED", r.stderr)
        self.assertNotIn("dry run", r.stdout)
        return r

    def assertAllowed(self, *args):
        r = self.run_s(*args)
        self.assertEqual(r.returncode, 0, (args, r.stdout, r.stderr))
        return r.stdout


class TestVenvInstall(Base):
    script_name = "venv-install.ps1"
    REQ = "shared/requirements-demo.txt"

    def setUp(self):
        super().setUp()
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "core.autocrlf", "false")
        # Preserve deliberately CRLF blobs as well as LF blobs, regardless of the host's attributes.
        wr(os.path.join(self.root, ".gitattributes"), "shared/requirements-*.txt -text\n")
        wr(os.path.join(self.root, self.REQ), "numpy==2.1.0\nscipy==1.14.1\n")
        wr(os.path.join(self.root, "shared", "requirements-crlf.txt"), "numpy==2.1.0\r\nPyYAML==6.0.2\r\n")
        wr(os.path.join(self.root, "shared", "notreq.txt"), "numpy==2.1.0\n")
        wr(os.path.join(self.root, "requirements-top.txt"), "numpy==2.1.0\n")
        wr(os.path.join(self.root, "shared", "sub", "requirements-x.txt"), "numpy==2.1.0\n")
        git(self.root, "add", ".gitattributes", "shared", "requirements-top.txt")
        git(self.root, "commit", "-q", "-m", "reqs")

    def commit_req(self, name, text):
        wr(os.path.join(self.root, "shared", name), text)
        git(self.root, "add", "shared/" + name)
        git(self.root, "commit", "-q", "-m", name)
        return "shared/" + name

    def test_accepts_tracked_pinned_file_dry_run(self):
        out = self.assertAllowed(self.REQ, "--dry-run")
        self.assertIn("python -I -m venv", out)
        self.assertIn("PIP_CONFIG_FILE=nul ", out)
        self.assertIn("-I -m pip install --isolated --only-binary=:all: --disable-pip-version-check --report ", out)
        head = git(self.root, "rev-parse", "HEAD")
        sha = hashlib.sha256(b"numpy==2.1.0\nscipy==1.14.1\n").hexdigest()
        self.assertIn(f"{self.REQ} at {head}, sha256 {sha}", out)
        self.assertIn("dry run, nothing executed", out)
        self.assertFalse(os.path.exists(os.path.join(self.root, "shared", "venv")), "dry run created the venv")
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")), "dry run wrote the copy")

    def test_accepts_crlf_and_backslash_path_with_existing_venv(self):
        wr(os.path.join(self.root, "shared", "venv", "Scripts", "python.exe"), "")
        out = self.assertAllowed("shared\\requirements-crlf.txt", "--dry-run")
        self.assertNotIn("-m venv", out)
        self.assertIn("pip install", out)

    def test_accepts_lf_worktree_equal_to_raw_blob(self):
        git(self.root, "config", "core.autocrlf", "true")
        wr(os.path.join(self.root, ".gitattributes"), "shared/requirements-*.txt text\n")
        git(self.root, "add", ".gitattributes")
        git(self.root, "commit", "-q", "-m", "text checkout policy")
        raw = subprocess.run(["git", "-C", self.root, "cat-file", "blob", "HEAD:" + self.REQ],
                             capture_output=True, timeout=120)
        self.assertEqual(raw.returncode, 0, raw.stderr)
        self.assertEqual(raw.stdout, b"numpy==2.1.0\nscipy==1.14.1\n")
        with open(os.path.join(self.root, self.REQ), "rb") as f:
            self.assertEqual(f.read(), raw.stdout, "setup: working bytes must equal the raw LF blob")
        out = self.assertAllowed(self.REQ, "--dry-run")
        self.assertIn(hashlib.sha256(raw.stdout).hexdigest(), out)
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")))

    def test_refuses_crlf_worktree_of_lf_blob(self):
        git(self.root, "config", "core.autocrlf", "true")
        wr(os.path.join(self.root, ".gitattributes"), "shared/requirements-*.txt text\n")
        git(self.root, "add", ".gitattributes")
        git(self.root, "commit", "-q", "-m", "text checkout policy")
        raw = subprocess.run(["git", "-C", self.root, "cat-file", "blob", "HEAD:" + self.REQ],
                             capture_output=True, timeout=120)
        self.assertEqual(raw.returncode, 0, raw.stderr)
        self.assertEqual(raw.stdout, b"numpy==2.1.0\nscipy==1.14.1\n")
        wr(os.path.join(self.root, self.REQ), "numpy==2.1.0\r\nscipy==1.14.1\r\n")
        git(self.root, "diff", "--exit-code", "HEAD", "--", self.REQ)
        # Git considers the EOL conversion clean; the safety boundary must still reject the different bytes.
        r = self.assertRefused(self.REQ, "--dry-run")
        self.assertIn("differs from its committed bytes", r.stderr)
        self.assertRefused(self.REQ)
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")))
        self.assertFalse(os.path.exists(os.path.join(self.root, "shared", "venv")))

    def filter_case(self, dry):
        # A weakened or round-4 copy must stop before pip even if the filter guard is absent.
        # This deliberately incomplete venv is a fixture, not a script-created side effect.
        venv = os.path.join(self.root, "shared", "venv")
        os.makedirs(venv)
        # Configure after staging the attribute so the test setup itself cannot invoke the driver.
        wr(os.path.join(self.root, ".gitattributes"), f"{self.REQ} -text filter=x\n")
        git(self.root, "add", ".gitattributes")
        git(self.root, "commit", "-q", "-m", "filter attribute")
        git(self.root, "config", "filter.x.smudge", "echo smudged > smudge-marker; cat")
        git(self.root, "config", "filter.x.clean", "echo cleaned > clean-marker; cat")
        args = (self.REQ, "--dry-run") if dry else (self.REQ,)
        r = self.run_s(*args)
        # Check markers first: even a subsequent refusal must not hide earlier driver execution.
        self.assertFalse(os.path.exists(os.path.join(self.root, "smudge-marker")), (r.stdout, r.stderr))
        self.assertFalse(os.path.exists(os.path.join(self.root, "clean-marker")), (r.stdout, r.stderr))
        self.assertEqual(r.returncode, 2, (r.stdout, r.stderr))
        self.assertIn("unsafe 'filter' attribute", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")))
        self.assertEqual(os.listdir(venv), [], "the refusal changed the incomplete venv fixture")

    def test_refuses_filter_without_running_driver(self):
        self.filter_case(False)

    def test_refuses_filter_without_running_driver_dry_run(self):
        self.filter_case(True)

    def test_refuses_all_specified_filter_states(self):
        for state in ("filter", "-filter", "filter=x", "filter=unspecified"):
            with self.subTest(state=state):
                wr(os.path.join(self.root, ".gitattributes"), f"{self.REQ} -text {state}\n")
                r = self.assertRefused(self.REQ, "--dry-run")
                self.assertIn("unsafe 'filter' attribute", r.stderr)

    def test_refuses_diff_and_text_drivers(self):
        for attr in ("diff", "text"):
            with self.subTest(attr=attr):
                wr(os.path.join(self.root, ".gitattributes"), f"{self.REQ} {attr}=x\n")
                git(self.root, "config", "diff.x.command", "echo diff > diff-marker")
                git(self.root, "config", "diff.x.textconv", "echo textconv > textconv-marker")
                r = self.assertRefused(self.REQ, "--dry-run")
                self.assertIn(f"unsafe '{attr}' attribute", r.stderr)
                for marker in ("diff-marker", "textconv-marker"):
                    self.assertFalse(os.path.exists(os.path.join(self.root, marker)))

    def test_refuses_driver_names_that_look_like_boolean_states(self):
        for value in ("set", "unset", "unspecified"):
            with self.subTest(value=value):
                wr(os.path.join(self.root, ".gitattributes"), f"{self.REQ} diff={value}\n")
                git(self.root, "config", f"diff.{value}.textconv", "echo ran > textconv-marker")
                r = self.assertRefused(self.REQ, "--dry-run")
                self.assertIn("unsafe 'diff' attribute", r.stderr)
                self.assertFalse(os.path.exists(os.path.join(self.root, "textconv-marker")))

    def test_refuses_fsmonitor_before_index_reads(self):
        helper = os.path.join(self.root, "monitor-helper")
        wr(helper, "#!/bin/sh\necho ran > monitor-marker\nexit 1\n")
        git(self.root, "config", "core.fsmonitor", '"' + helper.replace("\\", "/") + '"')
        r = self.assertRefused(self.REQ, "--dry-run")
        self.assertIn("core.fsmonitor is configured", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.root, "monitor-marker")))

    def test_diff_never_runs_configured_external_command(self):
        wr(os.path.join(self.root, self.REQ), "requests==2.32.3\n")
        helper = os.path.join(self.root, "diff-helper")
        wr(helper, "#!/bin/sh\necho ran > external-diff-marker\nexit 0\n")
        git(self.root, "config", "diff.external", '"' + helper.replace("\\", "/") + '"')
        git(self.root, "config", "diff.trustExitCode", "true")
        r = self.run_s(self.REQ, "--dry-run")
        self.assertFalse(os.path.exists(os.path.join(self.root, "external-diff-marker")), (r.stdout, r.stderr))
        self.assertEqual(r.returncode, 2, (r.stdout, r.stderr))
        self.assertIn("modified against HEAD", r.stderr)

    def test_missing_blob_never_fetches_through_a_configured_command(self):
        blob = git(self.root, "rev-parse", "HEAD:" + self.REQ)
        helper = os.path.join(self.root, "ssh-helper")
        wr(helper, "#!/bin/sh\necho ran > lazy-fetch-marker\nexit 1\n")
        git(self.root, "config", "core.sshCommand", '"' + helper.replace("\\", "/") + '"')
        # The helper never opens a connection; any invocation is already a failure.
        git(self.root, "config", "remote.origin.url", "ssh://invalid.example/unused")
        git(self.root, "config", "remote.origin.promisor", "true")
        git(self.root, "config", "extensions.partialClone", "origin")
        # Delete only a loose object in this throwaway repo, never an object in the source checkout.
        loose = os.path.join(self.root, ".git", "objects", blob[:2], blob[2:])
        os.chmod(loose, 0o600)  # git writes loose objects read-only; Windows refuses to remove a read-only file
        os.remove(loose)
        r = self.run_s(self.REQ, "--dry-run")
        self.assertFalse(os.path.exists(os.path.join(self.root, "lazy-fetch-marker")), (r.stdout, r.stderr))
        self.assertEqual(r.returncode, 2, (r.stdout, r.stderr))
        self.assertIn("REFUSED", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")))

    def test_refuses_wrong_paths(self):
        self.assertRefused("shared/notreq.txt", "--dry-run")
        self.assertRefused("requirements-top.txt", "--dry-run")
        self.assertRefused("shared/sub/requirements-x.txt", "--dry-run")
        self.assertRefused("shared/../shared/requirements-demo.txt", "--dry-run")
        self.assertRefused(os.path.join(self.root, "shared", "requirements-demo.txt"), "--dry-run")
        self.assertRefused("shared/REQUIRE~1.TXT", "--dry-run")
        self.assertRefused("shared/requirements-missing.txt", "--dry-run")
        self.assertRefused("", "--dry-run")
        for path in ("C:shared/requirements-demo.txt", "shared/requirements-demo.txt\r",
                     "shared/requirements-demo.txt\n", "shared/requirements-demo.txt:stream"):
            with self.subTest(path=path):
                self.assertRefused(path, "--dry-run")

    def test_refuses_extra_and_injected_arguments(self):
        self.assertRefused()
        self.assertRefused(self.REQ, "--dry-run", "x")
        self.assertRefused(self.REQ, "--index-url", "http://evil")
        self.assertRefused("--dry-run", self.REQ)
        self.assertRefused(self.REQ + "; rm -rf x", "--dry-run")
        self.assertRefused(self.REQ + " -r other.txt", "--dry-run")
        self.assertRefused(self.REQ, "--DRY-RUN")

    def test_refuses_untracked_file(self):
        wr(os.path.join(self.root, "shared", "requirements-new.txt"), "numpy==2.1.0\n")
        self.assertRefused("shared/requirements-new.txt", "--dry-run")

    def test_refuses_modified_file(self):
        wr(os.path.join(self.root, self.REQ), "numpy==2.1.0\nscipy==1.14.1\nrequests==2.32.3\n")
        self.assertRefused(self.REQ, "--dry-run")
        # staged but not committed is modified against HEAD too
        git(self.root, "add", self.REQ)
        self.assertRefused(self.REQ, "--dry-run")

    def test_refuses_skip_worktree_swap(self):
        # r3 B2: `git diff` cannot see a skip-worktree file, so other (well-formed, pinned) requirements in the worktree
        # passed the diff and their bytes reached pip. Now pip gets only the committed blob, and a worktree that differs
        # from it refuses.
        git(self.root, "update-index", "--skip-worktree", self.REQ)
        wr(os.path.join(self.root, self.REQ), "requests==2.32.3\n")
        r = subprocess.run(["git", "-C", self.root, "diff", "--quiet", "HEAD", "--", self.REQ], timeout=120)
        self.assertEqual(r.returncode, 0, "setup: git diff should not see the skip-worktree change")
        self.assertRefused(self.REQ, "--dry-run")
        self.assertRefused(self.REQ)
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")), "a refusal wrote the copy")

    def test_refuses_bad_lines(self):
        bad = ["https://evil.example/pkg.whl\n", "-e .\n", "--index-url https://evil\n", "-r other.txt\n",
               "numpy>=2.0\n", "numpy\n", "numpy[extra]==2.1.0\n", "numpy==2.1.0 ; python_version>'3'\n",
               "numpy==2.1.0\n\nscipy==1.14.1\n", "# comment\nnumpy==2.1.0\n", "numpy==2.1.0 --hash=sha256:00\n",
               "numpy @ https://evil/x.whl\n", "numpy==2.1.0\n--extra-index-url https://evil\n", "numpy==\u00e92\n"]
        for i, text in enumerate(bad):
            rel = self.commit_req(f"requirements-bad{i}.txt", text)
            self.assertRefused(rel, "--dry-run")

    def test_validates_the_end_of_large_content(self):
        text = "numpy==2.1.0\n" * 10000 + "--index-url https://evil.example\n"
        rel = self.commit_req("requirements-large.txt", text)
        r = self.assertRefused(rel, "--dry-run")
        self.assertIn("not exactly name==version", r.stderr)
        self.assertFalse(os.path.exists(os.path.join(self.root, "scratch")))

    def test_refuses_scratch_junction(self):
        # F3: the validated copy is written under scratch/venv-install; a junction there would put it elsewhere.
        out = outside_dir(self)
        junction(os.path.join(self.root, "scratch"), out)
        self.assertRefused(self.REQ, "--dry-run")
        self.assertRefused(self.REQ)
        self.assertEqual(os.listdir(out), [], "something was written through the junction")

    def test_refuses_venv_junction(self):
        out = outside_dir(self)
        wr(os.path.join(out, "Scripts", "python.exe"), "")
        junction(os.path.join(self.root, "shared", "venv"), out)
        self.assertRefused(self.REQ, "--dry-run")

    def test_refuses_venv_folder_without_python(self):
        os.makedirs(os.path.join(self.root, "shared", "venv", "Lib"))
        self.assertRefused(self.REQ, "--dry-run")


def bundled_pip_wheel():
    import ensurepip
    d = os.path.join(os.path.dirname(ensurepip.__file__), "_bundled")
    return ensurepip.version(), os.path.join(d, [f for f in os.listdir(d) if f.startswith("pip-")][0])


class TestVenvInstallReal(Base):
    """Real venv cases (no network): a real shared/venv made by this python, and the requirement `pip==<its version>`,
    which the venv already has, so a correct run is "already satisfied" and never contacts an index."""
    script_name = "venv-install.ps1"
    REQ = "shared/requirements-pip.txt"

    def setUp(self):
        super().setUp()
        self.pip_version, self.wheel = bundled_pip_wheel()
        git(self.root, "init", "-q", "-b", "main")
        git(self.root, "config", "core.autocrlf", "false")
        wr(os.path.join(self.root, self.REQ), f"pip=={self.pip_version}\n")
        git(self.root, "add", "shared")
        git(self.root, "commit", "-q", "-m", "reqs")
        self.venv = os.path.join(self.root, "shared", "venv")
        r = subprocess.run([sys.executable, "-I", "-m", "venv", self.venv],
                           capture_output=True, text=True, timeout=300)
        if r.returncode:
            raise RuntimeError(f"venv fixture creation failed: {r.stderr}")
        self.vpy = os.path.join(self.venv, "Scripts", "python.exe")

    def test_site_config_cannot_redirect_the_install(self):
        # r3 B1: shared/venv/pip.ini (pip's SITE config, read even with --isolated) and a PIP_CONFIG_FILE / PIP_TARGET
        # in the caller's environment all say: install into a folder outside the venv, from a local wheel folder.
        # pip must read none of it: the run is "already satisfied" in the venv and nothing appears outside.
        outside = outside_dir(self)
        target = os.path.join(outside, "target")
        wheels = os.path.join(outside, "wheels")
        os.makedirs(wheels)
        shutil.copy(self.wheel, wheels)
        ini = (f"[global]\nno-index = true\nfind-links = {wheels}\ndisable-pip-version-check = true\n"
               f"[install]\ntarget = {target}\n")
        wr(os.path.join(self.venv, "pip.ini"), ini)
        envini = os.path.join(outside, "env-pip.ini")
        wr(envini, ini)
        env = dict(os.environ)
        env.update(GIT_ENV)
        env.update({"PIP_CONFIG_FILE": envini, "PIP_TARGET": target, "PIP_NO_INDEX": "1", "PIP_FIND_LINKS": wheels})
        r = subprocess.run(["powershell", "-NoProfile", "-File", self.script, self.REQ], cwd=self.root, env=env,
                           capture_output=True, text=True, timeout=300)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        self.assertIn("post-check ok", r.stdout)
        self.assertFalse(os.path.exists(target) and os.listdir(target), f"pip wrote outside the venv: {target}")

    def post_check(self, report, reqs):
        text = rd(self.script)
        m = re.search(r"\$postCheck = @'\r?\n(.*?)\r?\n'@", text, re.S)
        self.assertIsNotNone(m, "post-check block not found")
        d = os.path.join(self.root, "scratch")
        wr(os.path.join(d, "check.py"), m.group(1))
        wr(os.path.join(d, "report.json"), json.dumps(report))
        wr(os.path.join(d, "reqs.txt"), reqs)
        return subprocess.run([self.vpy, "-I", os.path.join(d, "check.py"), self.venv, os.path.join(d, "report.json"),
                               os.path.join(d, "reqs.txt")], capture_output=True, text=True, timeout=120)

    def test_post_check_refuses_what_is_not_in_the_venv(self):
        # r3 one-place rule: after pip, everything pip reports and every requirement must be installed IN the venv.
        req = f"pip=={self.pip_version}\n"
        ok = self.post_check({"install": [{"metadata": {"name": "pip", "version": self.pip_version}}]}, req)
        self.assertEqual(ok.returncode, 0, (ok.stdout, ok.stderr))
        self.assertIn("post-check ok", ok.stdout)
        missing = self.post_check({"install": [{"metadata": {"name": "gs-not-there", "version": "1.0"}}]}, req)
        self.assertEqual(missing.returncode, 1, (missing.stdout, missing.stderr))
        self.assertIn("gs-not-there is not installed in the venv", missing.stdout)
        wrong = self.post_check({"install": []}, "pip==1.0\n")
        self.assertEqual(wrong.returncode, 1, (wrong.stdout, wrong.stderr))
        self.assertIn("not 1.0", wrong.stdout)


# --prove-fail changes copies only; each named test must reach a failing behavioural assertion.
WEAKENINGS = [
    ("without the unmodified checks",
     {"venv-install.ps1": [
         ("if ($LASTEXITCODE -ne 0) { Refuse \"'$relIn' is modified",
          "if ($false) { Refuse \"'$relIn' is modified"),
         ("if ($wh -ne $hash) {", "if ($false) {")]},
     ["test_refuses_modified_file"]),
    ("reading working bytes instead of the committed blob",
     {"venv-install.ps1": [
         ('$bytes = GitBlob "${rev}:$relIn"', "$bytes = [IO.File]::ReadAllBytes($full)")]},
     ["test_refuses_skip_worktree_swap"]),
    ("without the raw working-byte comparison",
     {"venv-install.ps1": [("if ($wh -ne $hash) {", "if ($false) {")]},
     ["test_refuses_crlf_worktree_of_lf_blob"]),
    ("checkout conversion instead of raw cat-file (round-4 behaviour)",
     {"venv-install.ps1": [("cat-file blob {1}", "cat-file --filters {1}")]},
     ["test_refuses_crlf_worktree_of_lf_blob", "test_accepts_lf_worktree_equal_to_raw_blob"]),
    ("without attribute refusal before content reads",
     {"venv-install.ps1": [
         ("if ($attr -cne 'text' -or @('set', 'unset', 'auto') -cnotcontains $value) {",
          "if ($false) {")]},
     ["test_refuses_filter_without_running_driver", "test_refuses_filter_without_running_driver_dry_run",
      "test_refuses_diff_and_text_drivers", "test_refuses_all_specified_filter_states",
      "test_refuses_driver_names_that_look_like_boolean_states"]),
    ("round-4 smudge execution before validation",
     {"venv-install.ps1": [
         ("if ($attr -cne 'text' -or @('set', 'unset', 'auto') -cnotcontains $value) {",
          "if ($false) {"),
         ("cat-file blob {1}", "cat-file --filters {1}")]},
     ["test_refuses_filter_without_running_driver", "test_refuses_filter_without_running_driver_dry_run"]),
    ("without fsmonitor refusal",
     {"venv-install.ps1": [
         ("if ($LASTEXITCODE -eq 0) { Refuse 'core.fsmonitor is configured",
          "if ($false) { Refuse 'core.fsmonitor is configured"),
         ("if ($LASTEXITCODE -ne 1) { Refuse 'cannot inspect core.fsmonitor",
          "if ($false) { Refuse 'cannot inspect core.fsmonitor")]},
     ["test_refuses_fsmonitor_before_index_reads"]),
    ("allowing external diff commands",
     {"venv-install.ps1": [
         ("diff --no-ext-diff --no-textconv --quiet", "diff --ext-diff --no-textconv --exit-code")]},
     ["test_diff_never_runs_configured_external_command"]),
    ("allowing a missing blob to trigger lazy fetch",
     {"venv-install.ps1": [("$env:GIT_NO_LAZY_FETCH = '1'\n", "")]},
     ["test_missing_blob_never_fetches_through_a_configured_command"]),
    ("without PIP_CONFIG_FILE=nul",
     {"venv-install.ps1": [("$env:PIP_CONFIG_FILE = 'nul'\n", "")]},
     ["test_site_config_cannot_redirect_the_install"]),
    ("post-check without the not-installed rule",
     {"venv-install.ps1": [('        bad.append(f"{name} is not installed in the venv")\n',
                           "        pass\n")]},
     ["test_post_check_refuses_what_is_not_in_the_venv"]),
    ("without the name==version rule",
     {"venv-install.ps1": [
         ("-cnotmatch '^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?==[0-9][A-Za-z0-9.!+]*$'",
          "-cnotmatch '.*'")]},
     ["test_refuses_bad_lines", "test_validates_the_end_of_large_content"]),
    ("without the scratch-copy link check",
     {"venv-install.ps1": [("foreach ($p in $copy, $report, $checkPy) { AssertNoLinks $p }\n", "")]},
     ["test_refuses_scratch_junction"]),
]

CLASSES = (TestVenvInstall, TestVenvInstallReal)


def prove_fail():
    if not WEAKENINGS:
        print("prove-fail BAD: no weakenings")
        return 1
    bad = 0
    for label, weaken, must_fail in WEAKENINGS:
        # A weakening whose target text is gone would "fail" the tests in setUp and look like a pass: count it BAD.
        missing = [old for name, pairs in weaken.items() for old, _ in pairs if old not in rd(os.path.join(SCRIPTS, name))]
        if missing:
            print(f"prove-fail BAD {label}: weakening target missing: {missing[0][:120]}")
            bad += 1
            continue
        WEAKEN.clear()
        WEAKEN.update(weaken)
        suite = unittest.TestSuite()
        for cls in CLASSES:
            for name in must_fail:
                if hasattr(cls, name):
                    suite.addTest(cls(name))
        if not must_fail or suite.countTestCases() != len(must_fail):
            print(f"prove-fail BAD {label}: missing or ambiguous test targets: {must_fail}")
            bad += 1
            continue
        with open(os.devnull, "w") as sink:
            res = unittest.TextTestRunner(stream=sink, verbosity=0).run(suite)
        failed = {t.id().split(".")[-1].split(" ")[0] for t, _ in res.failures}
        first = ""
        for _, tb in res.failures + res.errors:
            lines = [l for l in tb.strip().splitlines() if l.strip()]
            first = lines[-1][:200] if lines else ""
            break
        ok = not res.errors and not res.skipped and set(must_fail) <= failed
        print(f"prove-fail {'OK ' if ok else 'BAD'} {label}: failed {sorted(failed)}, "
              f"errors={len(res.errors)}, skipped={len(res.skipped)}: {first}")
        bad += 0 if ok else 1
    WEAKEN.clear()
    return bad


if __name__ == "__main__":
    if "--prove-fail" in sys.argv:
        sys.exit(1 if prove_fail() else 0)
    unittest.main(verbosity=2)
