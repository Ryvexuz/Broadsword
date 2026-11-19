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
- Files named `.env*`, `*.pem`, `*.key`, `*.pfx`, `id_rsa*`, or with `secret` or `credential` in the name are refused by name.
- A file whose content is a private-key block, or looks like a renamed `.env` (every non-comment line is `NAME=value` and one
  name is secret-like), is refused by content. Refused files are listed, never copied.
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

Exit codes: 0 done; 1 one or more files could not be read, or git failed; 2 bad command line or settings.
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
REQUIRED_KEYS = ("state_dir", "scratch_dir", "redact_min_chars")
KEY_PATTERN = re.compile(r"sk-[A-Za-z0-9_*\-]{3,}")  # the same rule as clients/openai_mcp.py
NAME_REFUSED = re.compile(r"(^\.env)|(\.(pem|key|pfx|p12)$)|(^id_(rsa|dsa|ecdsa|ed25519))|secret|credential", re.IGNORECASE)
PRIVATE_KEY = re.compile(rb"-----BEGIN [A-Z ]*PRIVATE KEY-----")
DOTENV_LINE = re.compile(r"^(export\s+)?[A-Za-z_][A-Za-z0-9_]*\s*=")


class Fail(Exception):
    pass


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


def refuse_reason(name: str, data: bytes) -> str | None:
    if NAME_REFUSED.search(name):
        return "secret-like file name"
    if PRIVATE_KEY.search(data):
        return "contains a private key block"
    lines = [ln.strip() for ln in data.decode("utf-8", errors="replace").splitlines()]
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
    if path.is_file():
        for line in path.read_text(encoding="utf-8").splitlines():
            parts = line.split("\t")
            if len(parts) >= 3:
                seen.add((parts[1], parts[2]))  # (source key, sha256)
    return seen


def git(archive: Path, *args: str) -> subprocess.CompletedProcess:
    cmd = ["git", "-c", "user.name=broadsword-archive", "-c", "user.email=archive@broadsword.invalid",
           "-c", "commit.gpgsign=false", *args]
    return subprocess.run(cmd, cwd=archive, capture_output=True, text=True)


def run(root: Path, dry_run: bool, date: str, environ: dict[str, str], out=print) -> int:
    cfg = load_settings(root)
    archive = root / cfg["state_dir"] / ARCHIVE_NAME
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
        reason = refuse_reason(path.name, data)
        if reason:
            refused.append(f"{key}: {reason}")
            continue
        digest = hashlib.sha256(data).hexdigest()
        if (key, digest) in seen:
            skipped += 1
            continue
        red = redact(data.decode("utf-8", errors="surrogateescape"), secrets).encode("utf-8", errors="surrogateescape")
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
            r = subprocess.run(["git", "init", "-q"], cwd=archive, capture_output=True, text=True)
            if r.returncode != 0:
                raise Fail(f"git init failed: {r.stderr.strip()}")
        lines = []
        for key, _, red, digest in to_copy:
            dest = day / key
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_bytes(red)
            lines.append(f"{date}\t{key}\t{digest}\n")
        with manifest_path.open("a", encoding="utf-8", newline="\n") as f:
            f.writelines(lines)
        r = git(archive, "add", "-A")
        if r.returncode == 0:
            r = git(archive, "commit", "-q", "-m", f"archive {date}: {len(to_copy)} files, {total} bytes")
        if r.returncode != 0:
            raise Fail(f"git failed in {archive}: {(r.stderr or r.stdout).strip()}")

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
        return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
