"""Report what is not yet backed up off this disk, per repo. Read-only unless --push --yes.

Usage:
    python -I scripts/backup_status.py --root <dir> --map <map.json> [--push --yes]

Map file (JSON, required, nothing is hard-coded here):
    {"repos": [
      {"name": "root", "path": ".", "branch": "main",
       "remote": "origin", "remote_branch": "main"},
      {"name": "crash-sim", "path": "projects/crash-sim", "branch": "main",
       "remote": "origin", "remote_branch": "crash-sim-main",
       "tracking_path": "."}
    ]}
  path, tracking_path: relative to --root. tracking_path is the repo that holds the
  remote-tracking ref <remote>/<remote_branch> (and the URL of <remote>); it defaults to path.
  All keys except tracking_path are required (tracking_path may be absent or null).

Status never touches the network: it only reads local refs with `git rev-list`.
Tags: a tag counts as not on the remote when its commit is not reachable from the
remote-tracking ref AND no record of pushing it exists (refs/backup-pushed/<remote>/<tag>,
written by --push). A tag on an already-pushed commit that was never pushed itself is
NOT detected without network access (see docs/BACKUPS.md).

--push needs --yes as well. Plain pushes only: never --force, never a '+' refspec, never a
delete (enforced by check_plain_push, a real check that also holds under python -O). A mapped
branch with no tracking ref yet is published as a new remote branch by --push --yes. Exit codes: 0 all repos ok, 1 some repo behind, 2 usage or config error.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import time

REQUIRED = ("name", "path", "branch", "remote", "remote_branch")
GIT_TIMEOUT = 60


class ConfigError(Exception):
    pass


class PushRefused(RuntimeError):
    pass


_USERINFO = re.compile(r"(://)[^/\s@]*@")
_PLAIN_SPEC = re.compile(r"^refs/(heads|tags)/[^:+\s-][^:\s]*:refs/(heads|tags)/[^:+\s-][^:\s]*$")
# The ONLY flag a push may carry: git's lease with an EMPTY expected value, which makes the remote refuse
# the update when the branch already exists. It is how a first-time publish stays create-only.
_CREATE_ONLY = re.compile(r"^--force-with-lease=refs/heads/([^:+\s-][^:\s]*):$")
_FORBIDDEN_FLAGS = ("--force", "-f", "--delete", "-d", "--mirror", "--prune", "--all")


def scrub(text):
    """Remove URL credentials (https://user:token@host -> https://host) from anything printed."""
    return _USERINFO.sub(r"\1", str(text))


def check_plain_push(cmd):
    """The one guard that keeps pushes plain. A real check, not an assert (python -O removes those).
    The single flag allowed is the create-only lease (see _CREATE_ONLY), and only when one of the
    refspecs sends a branch to exactly that remote branch."""
    i = cmd.index("push")
    rest = cmd[i + 1:]
    lease = None
    if rest and rest[0].startswith("-"):
        lease = _CREATE_ONLY.match(rest[0])
        if not lease:
            raise PushRefused("refusing push flag %r" % rest[0])
        rest = rest[1:]
    if len(rest) < 2 or rest[0].startswith("-"):
        raise PushRefused("refusing push command without a plain url and refspecs")
    for a in rest[1:]:
        if a.startswith("-") or a in _FORBIDDEN_FLAGS:
            raise PushRefused("refusing push flag %r" % a)
        if not _PLAIN_SPEC.match(a):
            raise PushRefused("refusing non-plain refspec %r" % a)
    if lease and not any(a.split(":")[1] == "refs/heads/" + lease.group(1) for a in rest[1:]):
        raise PushRefused("create-only lease %r matches no branch refspec" % cmd[i + 1])
    return cmd


def git(repo, *args, check=True):
    r = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True,
                       timeout=GIT_TIMEOUT)
    if check and r.returncode != 0:
        raise RuntimeError(scrub("git %s failed in %s: %s" % (" ".join(args), repo, r.stderr.strip())))
    return r


def load_map(path):
    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
    except (OSError, ValueError) as e:
        raise ConfigError("cannot read map %s: %s" % (path, e))
    repos = data.get("repos") if isinstance(data, dict) else None
    if not isinstance(repos, list) or not repos:
        raise ConfigError("map must have a non-empty 'repos' list")
    for r in repos:
        if not isinstance(r, dict):
            raise ConfigError("every map entry must be an object, got %r" % (r,))
        for k in REQUIRED:
            if not isinstance(r.get(k), str) or not r[k]:
                raise ConfigError("map entry %r is missing required key %r" % (r.get("name"), k))
        tp = r.get("tracking_path")
        if tp is None:   # absent or null both mean: same repo as path
            r["tracking_path"] = r["path"]
        elif not isinstance(tp, str) or not tp:
            raise ConfigError("map entry %r: tracking_path must be a non-empty string or null" % r["name"])
    return repos


def fmt_age(seconds):
    seconds = max(0, int(seconds))
    if seconds < 3600:
        return "%dm" % (seconds // 60)
    if seconds < 86400:
        return "%dh" % (seconds // 3600)
    return "%dd" % (seconds // 86400)


def ref_age(repo, full_ref, now):
    """Age of a local ref from its own reflog (time of its last update), or None when it has none.
    File mtimes are not used: git rewrites packed-refs whenever any ref is packed or deleted."""
    r = git(repo, "reflog", "show", "-1", "--date=unix", "--format=%gd", full_ref, check=False)
    m = re.search(r"@\{(\d+)\}$", r.stdout.strip())   # %gd is "<ref>@{<time of the reflog entry>}"
    if r.returncode != 0 or not m:
        return None
    out = m.group(1)
    return now - int(out)


def check_repo(root, entry, now):
    """Return a dict describing one repo's backup state."""
    repo = os.path.join(root, entry["path"])
    tracking = os.path.join(root, entry["tracking_path"])
    ref = "%s/%s" % (entry["remote"], entry["remote_branch"])
    res = {"name": entry["name"], "ref": ref, "behind": True, "ahead": None, "tags": [],
            "oldest_age": None, "ref_age": None, "error": None, "remote_sha": None,
            "recorded": False, "first_time": False, "tags_unverified": False}
    sha = git(tracking, "rev-parse", "--verify", "--quiet",
              "refs/remotes/%s" % ref, check=False).stdout.strip()   # not peeled: a ref to a missing object must still show up
    if sha and git(repo, "cat-file", "-e", sha + "^{commit}", check=False).returncode != 0:
        res["error"] = "tracking ref %s (%s) is not a commit in %s" % (ref, sha[:8], entry["path"])
        sha = ""
    # A push from a repo other than the tracking repo is recorded in the pushed repo itself.
    rec_ref = "refs/backup-pushed-branch/%s/%s" % (entry["remote"], entry["remote_branch"])
    rec = git(repo, "rev-parse", "--verify", "--quiet", rec_ref + "^{commit}", check=False).stdout.strip()
    age_from = (tracking, "refs/remotes/" + ref)
    if rec and (not sha or git(repo, "merge-base", "--is-ancestor", sha, rec, check=False).returncode == 0):
        sha, res["error"] = rec, None
        age_from = (repo, rec_ref)
        res["recorded"] = True
    if not sha:
        if not res["error"]:
            res["error"] = "no local tracking ref %s" % ref
            # never published from here: --push --yes may create the remote branch
            res["first_time"] = git(repo, "rev-parse", "--verify", "--quiet",
                                    "refs/heads/%s^{commit}" % entry["branch"], check=False).returncode == 0
        return res
    res["remote_sha"] = sha
    local = git(repo, "rev-parse", "--verify", "--quiet", "refs/heads/%s^{commit}" % entry["branch"],
                check=False).stdout.strip()
    if not local:
        res["error"] = "no local branch %s" % entry["branch"]
        return res
    res["ref_age"] = ref_age(age_from[0], age_from[1], now)
    commits = git(repo, "rev-list", "--reverse", "%s..%s" % (sha, local)).stdout.split()
    res["ahead"] = len(commits)
    if commits:
        t = git(repo, "log", "-1", "--format=%ct", commits[0]).stdout.strip()
        res["oldest_age"] = now - int(t)
    tags = git(repo, "for-each-ref", "--format=%(refname:strip=2) %(objectname)", "refs/tags").stdout
    for line in tags.splitlines():
        name, obj = line.split(" ", 1)
        rec = git(repo, "rev-parse", "--verify", "--quiet",
                  "refs/backup-pushed/%s/%s" % (entry["remote"], name), check=False).stdout.strip()
        if rec == obj:
            continue
        target = git(repo, "rev-parse", "%s^{commit}" % name, check=False).stdout.strip()
        if target and git(repo, "merge-base", "--is-ancestor", target, sha, check=False).returncode == 0:
            res["tags_unverified"] = True   # reachable, but nothing proves the tag itself was pushed
            continue
        res["tags"].append(name)
    res["tags"].sort()
    res["behind"] = bool(res["ahead"]) or bool(res["tags"])
    return res


def line_for(res):
    if res["error"]:
        return "%s: BEHIND (cannot verify: %s)" % (res["name"], scrub(res["error"]))
    parts = ["ahead %d" % res["ahead"], "tags not on remote %d" % len(res["tags"])]
    if res["tags"]:
        parts[-1] += " (%s)" % ", ".join(res["tags"])
    parts.append("oldest unpushed %s" % (fmt_age(res["oldest_age"]) if res["oldest_age"] is not None else "-"))
    verb = "pushed (recorded)" if res.get("recorded") else "updated"
    if res["ref_age"] is not None:
        parts.append("%s %s %s ago" % (res["ref"], verb, fmt_age(res["ref_age"])))
    else:
        parts.append("%s %s" % (res["ref"], "pushed (recorded), age unknown" if res.get("recorded") else "age unknown"))
    return "%s: %s - %s" % (res["name"], "BEHIND" if res["behind"] else "ok", ", ".join(parts))


def push_plan(root, entry, res):
    """Argument lists for the plain pushes this repo needs, plus a printable description."""
    tracking = os.path.join(root, entry["tracking_path"])
    url = git(tracking, "remote", "get-url", entry["remote"]).stdout.strip()
    refspecs = []
    if res["first_time"] or res["ahead"]:
        refspecs.append("refs/heads/%s:refs/heads/%s" % (entry["branch"], entry["remote_branch"]))
    for t in res["tags"]:
        refspecs.append("refs/tags/%s:refs/tags/%s" % (t, t))
    return url, refspecs


def push_cmd(root, entry, url, refspecs, first_time=False):
    """The full push command; refused unless plain (see check_plain_push). A first-time publish
    carries the create-only lease, so the remote refuses it if the branch appeared after our check."""
    repo = os.path.join(root, entry["path"])
    flags = ["--force-with-lease=refs/heads/%s:" % entry["remote_branch"]] if first_time else []
    return check_plain_push(["git", "-C", repo, "push", *flags, url, *refspecs])


def remote_branch_exists(url, entry):
    """First-time publish may only create the branch. True (and a message) when the remote already
    has it, or when that cannot be checked; the repo is then not pushed and stays BEHIND."""
    b = entry["remote_branch"]
    try:
        r = subprocess.run(["git", "ls-remote", "--heads", url, "refs/heads/%s" % b],
                           capture_output=True, text=True, timeout=GIT_TIMEOUT)
    except (subprocess.TimeoutExpired, OSError) as ex:
        print("%s: cannot check remote branch %s: %s" % (entry["name"], b, scrub(ex)))
        return True
    if r.returncode != 0:
        print("%s: cannot check remote branch %s: %s" % (entry["name"], b, scrub(r.stderr.strip())))
        return True
    if r.stdout.strip():
        print("%s: remote branch %s exists but there is no tracking ref; run git fetch first"
              % (entry["name"], b))
        return True
    return False


def do_push(root, entry, cmd, refspecs):
    """Plain push. Returns True on success; any failure (git error, timeout) is reported, not raised."""
    repo = os.path.join(root, entry["path"])
    tracking = os.path.join(root, entry["tracking_path"])
    try:
        check_plain_push(cmd)
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
        if r.returncode != 0:
            print("%s: push FAILED: %s" % (entry["name"], scrub(r.stderr.strip())))
            return False
        # Record what is now on the remote, as git itself would for a named remote.
        for spec in refspecs:
            src = spec.split(":")[0]
            sha = git(repo, "rev-parse", src).stdout.strip()
            if src.startswith("refs/heads/"):
                git(repo, "update-ref", "--create-reflog",
                    "refs/backup-pushed-branch/%s/%s" % (entry["remote"], entry["remote_branch"]), sha)
                if git(tracking, "cat-file", "-e", sha + "^{commit}", check=False).returncode == 0:
                    git(tracking, "update-ref", "refs/remotes/%s/%s" % (entry["remote"], entry["remote_branch"]), sha)
            else:
                git(repo, "update-ref", "refs/backup-pushed/%s/%s" % (entry["remote"], src[len("refs/tags/"):]), sha)
    except (RuntimeError, subprocess.TimeoutExpired, OSError) as e:
        print("%s: push FAILED: %s" % (entry["name"], scrub(e)))
        return False
    return True


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", required=True, help="project root; repo paths in the map are relative to it")
    ap.add_argument("--map", required=True, help="JSON map file (see module docstring)")
    ap.add_argument("--push", action="store_true", help="publish unpushed branches and tags (needs --yes)")
    ap.add_argument("--yes", action="store_true", help="confirm --push")
    args = ap.parse_args(argv)
    try:
        repos = load_map(args.map)
    except ConfigError as e:
        print("backup: config error: %s" % e)
        return 2
    now = time.time()
    results = []
    for entry in repos:
        try:
            res = check_repo(args.root, entry, now)
        except (RuntimeError, subprocess.TimeoutExpired, OSError) as e:
            res = {"name": entry["name"], "ref": "", "behind": True, "ahead": None, "tags": [],
                   "oldest_age": None, "ref_age": None, "error": str(e), "remote_sha": None}
        results.append((entry, res))
        print(line_for(res))
    if args.push:
        todo = [(e, r) for e, r in results if (not r["error"] and r["behind"]) or r.get("first_time")]
        plans = []
        try:
            for e, r in todo:
                url, specs = push_plan(args.root, e, r)
                plans.append((e, url, specs, push_cmd(args.root, e, url, specs, bool(r.get("first_time"))),
                              bool(r.get("first_time"))))
        except (RuntimeError, OSError) as e:
            print("backup: cannot plan push: %s" % scrub(e))
            return 2
        for e, url, specs, _cmd, _first in plans:
            print("will push %s to %s: %s" % (e["name"], scrub(url), " ".join(specs)))
        if not args.yes:
            print("backup: --push publishes your work; nothing pushed. Add --yes to confirm.")
            return 2
        for e, url, specs, cmd, first in plans:
            if first and remote_branch_exists(url, e):
                continue   # only CREATING a remote branch is allowed without a tracking ref
            do_push(args.root, e, cmd, specs)
        now = time.time()
        results = []
        for entry in repos:
            try:
                res = check_repo(args.root, entry, now)
            except (RuntimeError, subprocess.TimeoutExpired, OSError) as e:
                res = {"name": entry["name"], "ref": "", "behind": True, "ahead": None, "tags": [],
                       "oldest_age": None, "ref_age": None, "error": str(e), "remote_sha": None}
            results.append((entry, res))
            print(line_for(res))
    elif args.yes:
        print("backup: --yes only has a meaning together with --push")
        return 2
    behind = sum(1 for _, r in results if r["behind"])
    note = " (tags: offline check only)" if any(r.get("tags_unverified") for _, r in results) else ""
    print(("backup: ok" if behind == 0 else "backup: %d repos behind" % behind) + note)
    return 0 if behind == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
