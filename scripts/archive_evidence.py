#!/usr/bin/env python3
"""Copy Broadsword's evidence into a separate, durable Git repo before the scratch sweep can reach it.

Why: `state/queue/*` is git-ignored, and failed-job logs live in `scratch/logs/jobs/`, which the hourly cleanup wipes after
`scratch_max_age_hours`. Evidence is never deleted (AGENTS.md), so it is copied to `<state_dir>/evidence-archive/<yyyy-mm-dd>/`
(its own repo, outside the main one) and committed there. Nothing is ever moved or deleted from state/ or scratch/, and nothing is
ever pushed.

What is copied
- every file under `<state_dir>/queue/` (queue entries and reviews; `*.lock` files are not evidence and are skipped) into
  `<date>/queue/`;
- the log of every job whose record `<state_dir>/jobs/*.rec` says `status = failed`, from `<scratch_dir>/logs/jobs/<log name>`,
  into `<date>/jobs/`. The record's `log =` path may come from another machine, so only its file name is used. A failed job whose
  log the sweep already removed is counted as "missing", not an error.
A file whose source bytes (sha256) were already archived under the same source path is skipped (the manifest `manifest.tsv`
in the archive repo records them), so a second run copies nothing.

Secrets (never relaxed)
- Files whose name marks a secret are refused whole: the secret-file name rule of crates/broadsword/src/git.rs `is_secret_name`
  (`.env`, `.envrc`, `.env.<name>`, `.env-*`, `.env_*`, any case, trailing dots and spaces dropped, each `:` piece of an NTFS
  stream name checked; `.envoy.yaml` and `.environment` are ordinary), `*.pem`, `*.key`, `*.pfx`, `*.p12`, `id_rsa*` (also dsa,
  ecdsa, ed25519), or `secret` or `credential` in the name. Refused files are listed, never copied.
- A queue file whose content is a private-key block, or looks like a renamed `.env` (every non-comment line is `NAME=value` and
  one name is secret-like), is refused by content. A failed-job LOG that trips the same content check is archived REDACTED
  instead (key block and every `NAME=value` value replaced by `[REDACTED]`), because the sweep would delete it for good.
- Every copied text is redacted first with the same rule as the reviewer's (`Secrets` in crates/broadsword/src/reviewer.rs):
  every value in `<root>/.env` of at least `redact_min_chars` characters, and the value of every secret-named environment
  variable, become `[REDACTED]`, longest first; plus the `sk-...` look-alike rule of clients/openai_mcp.py. The script reads the
  root `.env` only to learn what to redact; it never copies it. A file that cannot be read is an error naming it (exit 1);
  the other files are still archived.

Where the hourly cleanup should call it: in the cleanup pass (crates/broadsword/src/cleanup.rs), as its FIRST step, before
the scratch files are deleted by age and before logs are cut to `log_max_mb`:
    python -I -B scripts/archive_evidence.py --root <root>
If it exits non-zero the cleanup must go on (the disk floor comes first) but must say that evidence was not archived. A
follow-up task wires it in; this task does not touch cleanup.rs.

Usage:
    python -I -B scripts/archive_evidence.py [--root <main checkout>] [--dry-run] [--date YYYY-MM-DD]
`--dry-run` lists what would be copied (and the file count and bytes) and writes nothing, not even the archive repo.

A second run (the hourly cleanup and a manual run) is refused while one holds the OS lock `<state_dir>/evidence-archive.lock`
(dry runs take no lock). The manifest is appended only after the archive commit succeeded.

Exit codes: 0 done; 1 one or more files could not be read, git failed, the manifest cannot be read, or another run holds the
lock; 2 bad command line or settings.
Python 3.11+, standard library only.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import os
import re
import subprocess
import sys
import tomllib
from pathlib import Path

CONFIG_FILE = "broadsword.toml"
ARCHIVE_NAME = "evidence-archive"  # inside state_dir
MANIFEST = "manifest.tsv"
LOCK_NAME = "evidence-archive.lock"  # inside state_dir, outside the archive repo
REQUIRED_KEYS = ("state_dir", "scratch_dir", "redact_min_chars")
KEY_PATTERN = re.compile(r"sk-[A-Za-z0-9_*\-]{3,}")  # the same rule as clients/openai_mcp.py
NAME_REFUSED = re.compile(r"(\.(pem|key|pfx|p12)$)|(^id_(rsa|dsa|ecdsa|ed25519))|secret|credential", re.IGNORECASE)
PRIVATE_KEY = re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY-----")
PRIVATE_KEY_STR = re.compile(PRIVATE_KEY.pattern.decode("ascii"))
PRIVATE_KEY_BLOCK =re.compile(r"-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|\Z)", re.DOTALL)
DOTENV_VALUE = re.compile(r"^(\s*(?:export\s+)?[A-Za-z_][A-Za-z0-9_]*\s*=).*$", re.MULTILINE)
DOTENV_LINE = re.compile(r"^(export\s+)?[A-Za-z_][A-Za-z0-9_]*\s*=")


class Fail(Exception):
    """A named error. `code` is the exit code: 2 for bad settings, 1 for everything that went wrong while archiving."""

    def __init__(self, message: str, code: int = 2):
        super().__init__(message)
        self.code = code


# ---- settings -----------------------------------------------------------------------------------


def load_settings(root: Path) -> dict:
    cfg_path = root / CONFIG_FILE
    try:
        cfg = tomllib.loads(cfg_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise Fail(f"cannot read {cfg_path}: {e}")
    for key in REQUIRED_KEYS:
        if key not in cfg:
            raise Fail(f"{CONFIG_FILE} has no `{key}` (a required setting)")
    if not isinstance(cfg["redact_min_chars"], int) or cfg["redact_min_chars"] < 1:
        raise Fail("redact_min_chars must be a whole number of at least 1")
    return cfg


# ---- redaction: the reviewer's rule (reviewer.rs `Secrets`) plus the sk- rule of clients/openai_mcp.py ------------------


def secret_name(name: str) -> bool:
    n = name.upper()
    return (
        any(n.endswith(s) for s in ("_KEY", "_TOKEN", "_SECRET", "_PAT", "_PASSWORD", "_PASS"))
        or "_CREDENTIAL" in n
        or "_AUTH" in n
        or n in ("API_KEY", "TOKEN", "SECRET", "PAT", "PASSWORD", "PASS")
    )


def env_file_values(root: Path) -> list[str]:
    """Every value in `<root>/.env`, whatever its name (read only to know what to redact)."""
    try:
        text = (root / ".env").read_text(encoding="utf-8-sig", errors="replace")
    except FileNotFoundError:
        return []
    except OSError as e:
        raise Fail(f"cannot read the root .env to learn what to redact ({type(e).__name__})")
    values = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[len("export "):]
        if "=" in line:
            val = line.split("=", 1)[1].strip().strip("\"'")
            if val:
                values.append(val)
    return values


def collect_secrets(root: Path, environ: dict[str, str], min_chars: int) -> list[str]:
    secrets = [v for v in env_file_values(root) if len(v) >= min_chars]
    secrets += [v for k, v in environ.items() if secret_name(k) and len(v) >= min_chars]
    return sorted(set(secrets), key=lambda s: (-len(s.encode("utf-8")), s))


def redact(text: str, secrets: list[str]) -> str:
    for s in secrets:
        text = text.replace(s, "[REDACTED]")
    return KEY_PATTERN.sub("[REDACTED]", text)


# ---- refusal ------------------------------------------------------------------------------------


def is_secret_env_name(name: str) -> bool:
    """The Rust rule `is_secret_name` (crates/broadsword/src/git.rs), ported exactly: `.env`, `.envrc`, `.env.<name>`, `.env-*`,
    `.env_*`, any case; trailing dots and spaces dropped; every `:` piece (NTFS stream names) is checked."""
    for piece in name.split(":"):
        n = piece.rstrip(" .").lower()
        if n == ".env" or n == ".envrc" or (len(n) > len(".env.") and n.startswith(".env.")) or n.startswith(".env-") or n.startswith(".env_"):
            return True
    return False


def name_refusal(name: str) -> str | None:
    if is_secret_env_name(name) or NAME_REFUSED.search(name):
        return "secret-like file name"
    return None


def split_lines(text: str, keepends: bool = False) -> list[str]:
    """THE one line splitter: the content check, the scrub and the residual check all use it, so they always agree on where
    a line ends (str.splitlines breaks on \\n, \\r, \\r\\n, \\v, \\f, \\x1c-\\x1e, \\x85, \\u2028 and \\u2029)."""
    return text.splitlines(keepends)


def scrub_content(text: str) -> str:
    """For a log that trips the content check: private key blocks and every `NAME=value` value become [REDACTED]."""
    text = PRIVATE_KEY_BLOCK.sub("[REDACTED]", text)
    out = []
    for raw in split_lines(text, keepends=True):
        body = "".join(split_lines(raw))  # the line without its terminator
        m = DOTENV_VALUE.match(body)
        out.append(m.group(1) + "[REDACTED]" + raw[len(body):] if m else raw)
    return "".join(out)


def scrub_residue(text: str) -> str | None:
    """After a scrub: why the text must still not be archived (fail closed), or None. A private key header, or any
    `NAME=value` line whose value is not exactly [REDACTED], means the scrub missed something."""
    if PRIVATE_KEY_STR.search(text):
        return "a private key block survived the scrub"
    for line in split_lines(text):
        m = DOTENV_VALUE.match(line)
        if m and line[len(m.group(1)):].strip() != "[REDACTED]":
            return "a NAME=value line survived the scrub"
    return None


def refuse_reason(name: str, data: bytes) -> str | None:
    """The content check only (the name check is `name_refusal`)."""
    if PRIVATE_KEY.search(data):
        return "contains a private key block"
    lines = [ln.strip() for ln in split_lines(data.decode("utf-8", errors="replace"))]
    lines = [ln for ln in lines if ln and not ln.startswith("#")]
    if lines and all(DOTENV_LINE.match(ln) for ln in lines):
        if any(secret_name(re.sub(r"^export\s+", "", ln).split("=", 1)[0].strip()) for ln in lines):
            return "looks like a .env file (NAME=value lines with a secret-named variable)"
    return None


# ---- sources ------------------------------------------------------------------------------------


def parse_rec(text: str) -> dict[str, str]:
    out = {}
    for line in text.splitlines():
        if "=" in line:
            k, _, v = line.partition("=")
            out[k.strip()] = v.strip()
    return out


def find_sources(root: Path, cfg: dict) -> tuple[list[tuple[str, Path]], list[str], list[str]]:
    """(relative source key, path) pairs, plus errors and the failed jobs whose log is gone."""
    state = root / cfg["state_dir"]
    scratch = root / cfg["scratch_dir"]
    found: list[tuple[str, Path]] = []
    errors: list[str] = []
    missing: list[str] = []
    qdir = state / "queue"
    if qdir.is_dir():
        for p in sorted(qdir.rglob("*")):
            if p.is_symlink() or not p.is_file() or p.name.endswith(".lock"):
                continue
            found.append(("queue/" + p.relative_to(qdir).as_posix(), p))
    jdir = state / "jobs"
    if jdir.is_dir():
        for rec in sorted(jdir.glob("*.rec")):
            try:
                fields = parse_rec(rec.read_text(encoding="utf-8", errors="replace"))
            except OSError as e:
                errors.append(f"{rec}: cannot read ({type(e).__name__})")
                continue
            if fields.get("status") != "failed":
                continue
            log_name = re.split(r"[\\/]+", fields.get("log", ""))[-1]
            if not log_name or log_name in (".", ".."):
                missing.append(f"{rec.name} (no log recorded)")
                continue
            log = scratch / "logs" / "jobs" / log_name
            if log.is_file() and not log.is_symlink():
                found.append(("jobs/" + log_name, log))
            else:
                missing.append(f"{rec.name} (log {log_name} not in scratch)")
    return found, errors, missing


# ---- archive ------------------------------------------------------------------------------------


def read_manifest(path: Path) -> set[tuple[str, str]]:
    seen = set()
    try:
        # lstat, not exists(): exists() may raise PermissionError (older Python) or silently report False (newer Python) when the
        # archive folder cannot be traversed; either way that must be the named error, never "absent" and never a traceback
        os.lstat(path)
        present = True
    except FileNotFoundError:
        present = False
    except OSError as e:
        raise Fail(f"cannot read the manifest {path} ({type(e).__name__}); fix or move it, nothing was archived", 1)
    if present:  # a directory or a dangling link is NOT "absent": it is a named error below
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError) as e:
            raise Fail(f"cannot read the manifest {path} ({type(e).__name__}); fix or move it, nothing was archived", 1)
        for line in text.splitlines():
            parts = line.split("\t")
            if len(parts) >= 3:
                seen.add((parts[1], parts[2]))  # (source key, sha256)
    return seen


def scrubbed_git_env(environ: dict[str, str]) -> dict[str, str]:
    """The environment without any GIT_* variable (GIT_DIR, GIT_WORK_TREE, GIT_INDEX_FILE ... would aim git at another repo)."""
    return {k: v for k, v in environ.items() if not k.upper().startswith("GIT_")}


def git(archive: Path, *args: str) -> subprocess.CompletedProcess:
    cmd = ["git", "-c", "user.name=broadsword-archive", "-c", "user.email=archive@broadsword.invalid",
           "-c", "commit.gpgsign=false", *args]
    return subprocess.run(cmd, cwd=archive, capture_output=True, text=True, env=scrubbed_git_env(dict(os.environ)))


def acquire_lock(path: Path):
    """An OS lock on `path` (released when the file closes or the process dies, so a crash never leaves it stuck)."""
    path.parent.mkdir(parents=True, exist_ok=True)
    f = open(path, "a+b")
    try:
        if os.name == "nt":
            import msvcrt
            f.seek(0)
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(f.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        f.close()
        raise Fail(f"another archive run holds the lock {path}; this run did nothing", 1)
    return f


def run(root: Path, dry_run: bool, date: str, environ: dict[str, str], out=print) -> int:
    cfg = load_settings(root)
    archive = root / cfg["state_dir"] / ARCHIVE_NAME
    lock = None if dry_run else acquire_lock(root / cfg["state_dir"] / LOCK_NAME)
    try:
        return _run_locked(root, cfg, archive, dry_run, date, environ, out)
    finally:
        if lock is not None:
            lock.close()


def _run_locked(root: Path, cfg: dict, archive: Path, dry_run: bool, date: str, environ: dict[str, str], out) -> int:
    secrets = collect_secrets(root, environ, cfg["redact_min_chars"])
    sources, errors, missing = find_sources(root, cfg)
    manifest_path = archive / MANIFEST
    seen = read_manifest(manifest_path)

    to_copy: list[tuple[str, Path, bytes, str]] = []  # key, path, redacted bytes, sha256 of source
    refused: list[str] = []
    skipped = 0
    for key, path in sources:
        try:
            data = path.read_bytes()
        except OSError as e:
            errors.append(f"{path}: cannot read ({type(e).__name__}: {e})")
            continue
        reason = name_refusal(path.name)
        if reason:
            refused.append(f"{key}: {reason}")
            continue
        reason = refuse_reason(path.name, data)
        scrub = False
        if reason:
            if key.startswith("jobs/"):  # a failed-job log is lost at the next sweep: archive it redacted, not dropped
                scrub = True
                out(f"scrubbed {key}: {reason}; archived redacted")
            else:
                refused.append(f"{key}: {reason}")
                continue
        digest = hashlib.sha256(data).hexdigest()
        if (key, digest) in seen:
            skipped += 1
            continue
        text = data.decode("utf-8", errors="surrogateescape")
        if scrub:
            text = scrub_content(text)
            residue = scrub_residue(redact(text, secrets))
            if residue:  # fail closed: the scrub did not make it safe, so the file is refused whole
                out(f"scrub failed for {key}: {residue}")
                refused.append(f"{key}: {residue}; refused whole")
                continue
        red = redact(text, secrets).encode("utf-8", errors="surrogateescape")
        to_copy.append((key, path, red, digest))

    total = sum(len(r) for _, _, r, _ in to_copy)
    verb = "would copy" if dry_run else "copying"
    for key, _, red, _ in to_copy:
        out(f"{verb} {key} ({len(red)} bytes)")
    for line in refused:
        out(f"REFUSED {line}")
    for line in missing:
        out(f"missing (already swept) {line}")
    for line in errors:
        out(f"ERROR {line}")

    if not dry_run and to_copy:
        day = archive / date
        archive.mkdir(parents=True, exist_ok=True)
        if not (archive / ".git").exists():
            r = git(archive, "init", "-q")
            if r.returncode != 0:
                raise Fail(f"git init failed: {r.stderr.strip()}", 1)
        lines = []
        for key, _, red, digest in to_copy:
            dest = day / key
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(red)
            lines.append(f"{date}\t{key}\t{digest}\n")
        r = git(archive, "add", "-A")
        committed = False
        if r.returncode == 0 and git(archive, "diff", "--cached", "--quiet").returncode != 0:  # nothing staged: a crashed earlier run
            committed = True
            r = git(archive, "commit", "-q", "-m", f"archive {date}: {len(to_copy)} files, {total} bytes")
        if r.returncode != 0:
            raise Fail(f"git failed in {archive}: {(r.stderr or r.stdout).strip()}", 1)
        # the manifest only after the commit succeeded: a failed commit leaves these files unrecorded, so the next run retries
        before = manifest_path.read_bytes() if manifest_path.exists() else None
        with manifest_path.open("a", encoding="utf-8", newline="\n") as f:
            f.writelines(lines)
        r = git(archive, "add", MANIFEST)
        if r.returncode == 0:
            if committed:
                r = git(archive, "commit", "-q", "--amend", "--no-edit")
            else:
                r = git(archive, "commit", "-q", "-m", f"manifest {date}")
        if r.returncode != 0:
            # put the manifest back as it was, so the next run does not skip these sources as already archived
            if before is None:
                manifest_path.unlink(missing_ok=True)
            else:
                manifest_path.write_bytes(before)
            raise Fail(f"git failed in {archive}: {(r.stderr or r.stdout).strip()}", 1)

    out(f"{'dry run: ' if dry_run else ''}{len(to_copy)} files, {total} bytes {'would be copied' if dry_run else 'copied'}; "
        f"{skipped} already archived, {len(refused)} refused, {len(missing)} failed-job logs missing, {len(errors)} errors")
    return 1 if errors else 0


def main(argv: list[str]) -> int:
    ap = argparse.ArgumentParser(description="Copy queue entries and failed-job logs into the evidence archive repo.")
    ap.add_argument("--root", default=str(Path(__file__).resolve().parent.parent), help="the main checkout (default: this script's project)")
    ap.add_argument("--dry-run", action="store_true", help="list what would be copied; write nothing")
    ap.add_argument("--date", default=None, help="archive folder name yyyy-mm-dd (default: today)")
    args = ap.parse_args(argv)
    date = args.date or datetime.date.today().isoformat()
    if not re.fullmatch(r"\d{4}-\d{2}-\d{2}", date):
        print("--date must be yyyy-mm-dd", file=sys.stderr)
        return 2
    try:
        return run(Path(args.root).resolve(), args.dry_run, date, dict(os.environ))
    except Fail as e:
        print(f"error: {e}", file=sys.stderr)
        return e.code


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
