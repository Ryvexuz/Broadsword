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
  All keys except tracking_path are required.

Status never touches the network: it only reads local refs with `git rev-list`.
Tags: a tag counts as not on the remote when its commit is not reachable from the
remote-tracking ref AND no record of pushing it exists (refs/backup-pushed/<remote>/<tag>,
written by --push). A tag on an already-pushed commit that was never pushed itself is
NOT detected without network access (see docs/BACKUPS.md).

--push needs --yes as well. Plain pushes only: never --force, never a '+' refspec, never a
delete. Exit codes: 0 all repos ok, 1 some repo behind, 2 usage or config error.
"""
import argparse
import json
import os
import subprocess
import sys
import time

REQUIRED = ("name", "path", "branch", "remote", "remote_branch")
GIT_TIMEOUT = 60


class ConfigError(Exception):
    pass


def git(repo, *args, check=True):
    r = subprocess.run(["git", "-C", repo, *args], capture_output=True, text=True,
                       timeout=GIT_TIMEOUT)
    if check and r.returncode != 0:
        raise RuntimeError("git %s failed in %s: %s" % (" ".join(args), repo, r.stderr.strip()))
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
        for k in REQUIRED:
            if not isinstance(r.get(k), str) or not r[k]:
                raise ConfigError("map entry %r is missing required key %r" % (r.get("name"), k))
        r.setdefault("tracking_path", r["path"])
    return repos


def fmt_age(seconds):
    seconds = max(0, int(seconds))
    if seconds < 3600:
        return "%dm" % (seconds // 60)
    if seconds < 86400:
        return "%dh" % (seconds // 3600)
    return "%dd" % (seconds // 86400)


def ref_age(tracking, ref, now):
    """Age of the local remote-tracking ref (when it was last updated), or None."""
    common = git(tracking, "rev-parse", "--git-common-dir").stdout.strip()
    common = os.path.normpath(os.path.join(tracking, common))
    loose = os.path.join(common, "refs", "remotes", *ref.split("/"))
    for p in (loose, os.path.join(common, "packed-refs")):
        if os.path.exists(p):
            return now - os.path.getmtime(p)
    return None


def check_repo(root, entry, now):
    """Return a dict describing one repo's backup state."""
    repo = os.path.join(root, entry["path"])
    tracking = os.path.join(root, entry["tracking_path"])
    ref = "%s/%s" % (entry["remote"], entry["remote_branch"])
    res = {"name": entry["name"], "ref": ref, "behind": True, "ahead": None, "tags": [],
            "oldest_age": None, "ref_age": None, "error": None, "remote_sha": None}
    sha = git(tracking, "rev-parse", "--verify", "--quiet",
              "refs/remotes/%s^{commit}" % ref, check=False).stdout.strip()
    if not sha:
        res["error"] = "no local tracking ref %s" % ref
        return res
    res["remote_sha"] = sha
    if git(repo, "cat-file", "-e", sha + "^{commit}", check=False).returncode != 0:
        res["error"] = "tracking ref %s (%s) is not a commit in %s" % (ref, sha[:8], entry["path"])
        return res
    local = git(repo, "rev-parse", "--verify", "--quiet", "refs/heads/%s^{commit}" % entry["branch"],
                check=False).stdout.strip()
    if not local:
        res["error"] = "no local branch %s" % entry["branch"]
        return res
    res["ref_age"] = ref_age(tracking, ref, now)
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
            continue
        res["tags"].append(name)
    res["tags"].sort()
    res["behind"] = bool(res["ahead"]) or bool(res["tags"])
    return res


def line_for(res):
    if res["error"]:
        return "%s: BEHIND (cannot verify: %s)" % (res["name"], res["error"])
    parts = ["ahead %d" % res["ahead"], "tags not on remote %d" % len(res["tags"])]
    if res["tags"]:
        parts[-1] += " (%s)" % ", ".join(res["tags"])
    parts.append("oldest unpushed %s" % (fmt_age(res["oldest_age"]) if res["oldest_age"] is not None else "-"))
    parts.append("%s updated %s ago" % (res["ref"], fmt_age(res["ref_age"]) if res["ref_age"] is not None else "?"))
    return "%s: %s - %s" % (res["name"], "BEHIND" if res["behind"] else "ok", ", ".join(parts))


def push_plan(root, entry, res):
    """Argument lists for the plain pushes this repo needs, plus a printable description."""
    tracking = os.path.join(root, entry["tracking_path"])
    url = git(tracking, "remote", "get-url", entry["remote"]).stdout.strip()
    refspecs = []
    if res["ahead"]:
        refspecs.append("refs/heads/%s:refs/heads/%s" % (entry["branch"], entry["remote_branch"]))
    for t in res["tags"]:
        refspecs.append("refs/tags/%s:refs/tags/%s" % (t, t))
    for spec in refspecs:   # one place that keeps pushes plain
        if spec.startswith(("+", ":")) or spec.split(":")[0] == "":
            raise RuntimeError("refusing non-plain refspec %r" % spec)
    return url, refspecs


def do_push(root, entry, url, refspecs):
    repo = os.path.join(root, entry["path"])
    tracking = os.path.join(root, entry["tracking_path"])
    cmd = ["git", "-C", repo, "push", url, *refspecs]
    assert "--force" not in cmd and "-f" not in cmd and "--delete" not in cmd
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if r.returncode != 0:
        print("%s: push FAILED: %s" % (entry["name"], r.stderr.strip()))
        return False
    # Record what is now on the remote, as git itself would for a named remote.
    for spec in refspecs:
        src = spec.split(":")[0]
        sha = git(repo, "rev-parse", src).stdout.strip()
        if src.startswith("refs/heads/"):
            if git(tracking, "cat-file", "-e", sha + "^{commit}", check=False).returncode == 0:
                git(tracking, "update-ref", "refs/remotes/%s/%s" % (entry["remote"], entry["remote_branch"]), sha)
        else:
            git(repo, "update-ref", "refs/backup-pushed/%s/%s" % (entry["remote"], src[len("refs/tags/"):]), sha)
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
        todo = [(e, r) for e, r in results if not r["error"] and r["behind"]]
        plans = []
        try:
            for e, r in todo:
                url, specs = push_plan(args.root, e, r)
                plans.append((e, url, specs))
        except (RuntimeError, OSError) as e:
            print("backup: cannot plan push: %s" % e)
            return 2
        for e, url, specs in plans:
            print("will push %s to %s: %s" % (e["name"], url, " ".join(specs)))
        if not args.yes:
            print("backup: --push publishes your work; nothing pushed. Add --yes to confirm.")
            return 2
        for e, url, specs in plans:
            do_push(args.root, e, url, specs)
        now = time.time()
        results = []
        for entry in repos:
            res = check_repo(args.root, entry, now)
            results.append((entry, res))
            print(line_for(res))
    elif args.yes:
        print("backup: --yes only has a meaning together with --push")
        return 2
    behind = sum(1 for _, r in results if r["behind"])
    print("backup: ok" if behind == 0 else "backup: %d repos behind" % behind)
    return 0 if behind == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
