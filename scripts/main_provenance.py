#!/usr/bin/env python3
"""Flag commits on main that did not come through the merge queue (audit S3, part 1).

AGENTS.md says all code reaches main through the merge queue. Nothing detected a break; this script does. It is
READ-ONLY: it runs only `git rev-parse`, `git log`, `git show`, `git merge-base` and reads files. It never writes.

Repos checked: the Broadsword root (project `broadsword`) and every folder in projects_dir that is its own Git repo
(crash-sim, render, ...). For each, every FIRST-PARENT commit since --since is accounted for when it is
  - queued:  inside the range base_head..main_sha (first-parent) of a `state/queue/*.entry` whose status is `merged` or
             `merged_check_failed` (the entry's main_sha is on main; see queue.rs: s_rebase sets base_head to main's HEAD, the
             branch is rebased onto it, and `merge --ff-only` lands exactly that range); or
  - listed:  in docs/direct-merges.txt, with who approved it, why, and the source line (see that file for the format); or
  - records: touches only `*.md` at the repo root, `evidence/` or `docs/`. Reported on its own, NOT a failure: the
             orchestrator commits records directly while the queue is idle.
Every other commit prints one UNACCOUNTED line: repo, hash, subject, files.

Exit codes: 0 nothing unaccounted, 1 something unaccounted, 2 bad input (unknown ref, unreadable or malformed record).
All paths come from broadsword.toml (root, state_dir, projects_dir); nothing is hard-coded.
"""
import argparse
import os
import re
import subprocess
import sys
import tomllib

MERGED_STATUSES = ("merged", "merged_check_failed")
HEX_RE = re.compile(r"^[0-9a-f]{7,40}$")
# An approver field that only says nobody approved it. The one place this is enforced is parse_allow.
PLACEHOLDER_RE = re.compile(r"^[\s\"'(\[]*(not\s+recorded|not\s+known|unknown|unapproved|tbd|todo|none|n/?a|\?+|-+)(?![A-Za-z0-9])", re.I)
ROOT_PROJECT = "broadsword"


class BadInput(Exception):
    pass


def git_full(repo, *args):
    """Run one read-only git command in `repo`; return (exit code, stdout, stderr)."""
    env = dict(os.environ, GIT_OPTIONAL_LOCKS="0")
    p = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True, encoding="utf-8",
                       errors="replace", env=env)
    return p.returncode, p.stdout, p.stderr


def git(repo, *args):
    code, out, _ = git_full(repo, *args)
    return code, out


def git_ok(repo, *args):
    code, out, err = git_full(repo, *args)
    if code != 0:
        raise BadInput(f"git {' '.join(args)} failed in {repo} (exit {code}): {err.strip()[:300]}")
    return out


def resolve(repo, ref):
    """Full sha of `ref` in `repo`, or None."""
    if ref.startswith("-") or not ref:
        raise BadInput(f"refusing option-like or empty ref {ref!r}")
    code, out = git(repo, "rev-parse", "--verify", "--quiet", "--end-of-options", ref + "^{commit}")
    return out.strip() if code == 0 and out.strip() else None


def is_ancestor(repo, a, b):
    code, _ = git(repo, "merge-base", "--is-ancestor", a, b)
    return code == 0


def load_toml(root):
    path = os.path.join(root, "broadsword.toml")
    try:
        with open(path, "rb") as f:
            cfg = tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise BadInput(f"cannot read {path}: {e}")
    for key in ("state_dir", "projects_dir"):
        if not isinstance(cfg.get(key), str) or not cfg[key]:
            raise BadInput(f"{path}: required key {key} is missing")
    return cfg


def read_entries(queue_dir):
    """Every state/queue/*.entry as a dict (key = value per line). A malformed entry is bad input."""
    if not os.path.isdir(queue_dir):
        raise BadInput(f"queue folder not found: {queue_dir}")
    entries = []
    for name in sorted(os.listdir(queue_dir)):
        if not name.endswith(".entry"):
            continue
        path = os.path.join(queue_dir, name)
        e = {}
        try:
            with open(path, encoding="utf-8") as f:
                for line in f:
                    line = line.rstrip("\r\n")
                    if not line:
                        continue
                    k, sep, v = line.partition(" = ")
                    if not sep:
                        k, sep, v = line.partition(" =")
                        if not sep:
                            raise BadInput(f"{path}: line is not `key = value`: {line[:60]!r}")
                    e[k.strip()] = v
        except OSError as ex:
            raise BadInput(f"cannot read {path}: {ex}")
        for key in ("id", "project", "status"):
            if not e.get(key):
                raise BadInput(f"{path}: entry has no `{key}`")
        e["_file"] = name
        entries.append(e)
    return entries


def parse_allow(path):
    """docs/direct-merges.txt: `repo | hash | approved by | reason | source`, one per line; # comments, blanks skipped."""
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.read().splitlines()
    except OSError as ex:
        raise BadInput(f"cannot read the allow-list {path}: {ex}")
    allowed = []
    for n, line in enumerate(lines, 1):
        s = line.strip()
        if not s or s.startswith("#"):
            continue
        parts = [p.strip() for p in s.split("|")]
        if len(parts) != 5 or not all(parts):
            raise BadInput(f"{path}:{n}: need `repo | hash | approved by | reason | source`, all five non-empty")
        repo, h, who, why, src = parts
        if PLACEHOLDER_RE.match(who):
            raise BadInput(f"{path}:{n}: approver {who!r} is a placeholder; name who approved it, or leave the commit out (it stays UNACCOUNTED until someone approves it)")
        if not HEX_RE.match(h):
            raise BadInput(f"{path}:{n}: `{h}` is not a hex commit hash of 7 to 40 characters")
        allowed.append({"repo": repo, "hash": h, "who": who, "why": why, "source": src, "line": n})
    return allowed


def discover_repos(root, cfg):
    """[(name, path)]: the root as `broadsword`, then each projects_dir folder that is its own Git repo."""
    repos = [(ROOT_PROJECT, root)]
    pdir = os.path.join(root, cfg["projects_dir"])
    if os.path.isdir(pdir):
        for name in sorted(os.listdir(pdir)):
            p = os.path.join(pdir, name)
            if os.path.exists(os.path.join(p, ".git")):
                repos.append((name, p))
    return repos


def main_branch_of(repo, name, entries):
    """The branch a repo's entries merge into (queue entries record it); else the checked-out branch."""
    seen = {e.get("main_branch", "") for e in entries if e["project"] == name and e.get("main_branch")}
    if len(seen) > 1:
        raise BadInput(f"project {name}: queue entries disagree on the main branch: {sorted(seen)}")
    if seen:
        return seen.pop()
    code, out = git(repo, "rev-parse", "--abbrev-ref", "HEAD")
    if code != 0 or not out.strip() or out.strip() == "HEAD":
        raise BadInput(f"project {name}: no queue entry names its main branch and HEAD is not on a branch")
    return out.strip()


def queued_commits(repo, name, branch_tip, entries):
    """{full sha: entry id} for every first-parent commit a merged queue entry landed."""
    got = {}
    for e in entries:
        if e["project"] != name or e["status"] not in MERGED_STATUSES:
            continue
        main_sha = e.get("main_sha", "")
        if not HEX_RE.match(main_sha):
            raise BadInput(f"queue entry {e['id']}: status {e['status']} but main_sha {main_sha!r} is not a commit hash")
        sha = resolve(repo, main_sha)
        if sha is None:
            raise BadInput(f"queue entry {e['id']}: main_sha {main_sha} is not in the {name} repo")
        got.setdefault(sha, e["id"])
        base = e.get("base_head", "")
        base_sha = resolve(repo, base) if HEX_RE.match(base) else None
        if base_sha is not None and is_ancestor(repo, base_sha, sha):
            for c in git_ok(repo, "rev-list", "--first-parent", f"{base_sha}..{sha}").split():
                got.setdefault(c, e["id"])
    return got


def is_records_path(p):
    return ("/" not in p and p.endswith(".md")) or p.startswith("evidence/") or p.startswith("docs/")


def files_of(repo, sha):
    out = git_ok(repo, "show", "--first-parent", "-m", "--no-renames", "--name-only", "--format=", sha)
    return [l for l in out.splitlines() if l]


def audit_repo(name, repo, since, entries, allowed):
    """Return a result dict for one repo."""
    branch = main_branch_of(repo, name, entries)
    tip = resolve(repo, branch)
    if tip is None:
        raise BadInput(f"project {name}: branch {branch} not found in {repo}")
    note = ""
    since_sha = resolve(repo, since)
    if since_sha is None:
        if name == ROOT_PROJECT:
            raise BadInput(f"--since {since!r} is not a commit or tag in the Broadsword repo")
        note = f"--since {since} does not exist here: ALL first-parent history of {branch} is checked"
        rng = [tip]
    else:
        if not is_ancestor(repo, since_sha, tip):
            raise BadInput(f"--since {since!r} is not an ancestor of {name} {branch}")
        rng = [f"{since_sha}..{tip}"]
    commits = git_ok(repo, "log", "--first-parent", "--format=%H%x09%s", *rng).splitlines()
    queued = queued_commits(repo, name, tip, entries)
    mine = [a for a in allowed if a["repo"] == name]
    res = {"name": name, "branch": branch, "note": note, "total": len(commits), "queued": [], "listed": [],
           "records": [], "unaccounted": [], "stale_allow": []}
    for line in commits:
        sha, _, subject = line.partition("\t")
        hit = [a for a in mine if sha.startswith(a["hash"])]
        if queued.get(sha):
            res["queued"].append((sha, subject, queued[sha]))
            continue
        if hit:
            res["listed"].append((sha, subject, hit[0]))
            continue
        files = files_of(repo, sha)
        if files and all(is_records_path(f) for f in files):
            res["records"].append((sha, subject, files))
        else:
            res["unaccounted"].append((sha, subject, files))
    for a in mine:
        full = resolve(repo, a["hash"])
        if full is None or not is_ancestor(repo, full, tip):
            res["stale_allow"].append(a)
    return res


def parse_args(argv):
    here = os.path.dirname(os.path.abspath(__file__))
    ap = argparse.ArgumentParser(description="Flag first-parent commits on main that skipped the merge queue (read-only).")
    ap.add_argument("--since", required=True, help="commit or tag; commits after it are checked (a project repo without it is checked in full)")
    ap.add_argument("--root", default=os.path.dirname(here), help="Broadsword main checkout (holds broadsword.toml and state/); default: this script's project")
    ap.add_argument("--allow", default=None, help="allow-list file; default: <root>/docs/direct-merges.txt")
    return ap.parse_args(argv)


def run(argv, out=sys.stdout):
    args = parse_args(argv)
    root = os.path.abspath(args.root)
    cfg = load_toml(root)
    allow_path = args.allow or os.path.join(root, "docs", "direct-merges.txt")
    allowed = parse_allow(allow_path)
    entries = read_entries(os.path.join(root, cfg["state_dir"], "queue"))
    repos = discover_repos(root, cfg)
    known = {n for n, _ in repos}
    for a in allowed:
        if a["repo"] not in known:
            raise BadInput(f"{allow_path}:{a['line']}: repo `{a['repo']}` is not one of {sorted(known)}")
    results = [audit_repo(n, p, args.since, entries, allowed) for n, p in repos]
    bad = 0
    p = lambda s: print(s, file=out)
    p("main_provenance: every first-parent commit on main since --since must come from the merge queue or be listed in docs/direct-merges.txt.")
    p("Records-only commits (only *.md at the repo root, evidence/, docs/) are listed as RECORDS: not failures, because the orchestrator commits records directly while the queue is idle.")
    for r in results:
        p("")
        p(f"== {r['name']} ({r['branch']}) since {args.since}: {r['total']} commits: queue {len(r['queued'])}, listed {len(r['listed'])}, records {len(r['records'])}, UNACCOUNTED {len(r['unaccounted'])}")
        if r["note"]:
            p(f"   note: {r['note']}")
        for sha, subj, a in r["listed"]:
            p(f"LISTED {r['name']} {sha[:12]} {subj[:90]} | approved by {a['who']} | {a['why']} | source {a['source']}")
        for sha, subj, files in r["records"]:
            p(f"RECORDS {r['name']} {sha[:12]} {subj[:90]}")
        for a in r["stale_allow"]:
            p(f"WARNING allow-list line {a['line']}: {a['hash']} is not a commit on {r['name']} {r['branch']}")
        for sha, subj, files in r["unaccounted"]:
            bad += 1
            p(f"UNACCOUNTED {r['name']} {sha[:12]} {subj} | files: {', '.join(files) if files else '(none)'}")
    p("")
    p(f"result: {bad} unaccounted commit(s)")
    return 1 if bad else 0


def main(argv=None):
    try:
        return run(sys.argv[1:] if argv is None else argv)
    except BadInput as e:
        print(f"main_provenance: bad input: {e}", file=sys.stderr)
        return 2
    except SystemExit as e:  # argparse
        return 2 if e.code not in (0, None) else 0


if __name__ == "__main__":
    sys.exit(main())
