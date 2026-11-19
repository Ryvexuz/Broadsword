# Backups: what is not yet off this disk

Audit S12: backup and evidence live on one disk. `scripts/backup_status.py` makes that measurable.
Status is read-only and offline. Pushing is publishing, so it stays a deliberate, manual act.

## Run it

```
python -I -B scripts/backup_status.py --root <project root> --map <map.json>
```

Output is one line per repo, then `backup: ok` (exit 0) or `backup: N repos behind` (exit 1).
Exit 2 means a usage or config error (bad map, `--push` without `--yes`).

Each line shows: commits ahead of the remote branch, tags not on the remote, the age of the oldest
unpushed commit, and how long ago the local remote-tracking ref was last updated (stale tracking
means the "ahead" count may be out of date; refresh it by hand with `git fetch`, which is network
and is NOT done by the script).

### Map file (required; no names are hard-coded)

JSON. `path` and `tracking_path` are relative to `--root`. `tracking_path` is the repo that holds the
remote-tracking ref and the URL of the remote; it defaults to `path`. The nested projects have no
remote of their own: they are pushed to the root repo's `origin` under another branch name, so their
entries point `tracking_path` at the root (`"."`).

```json
{"repos": [
 {"name": "root", "path": ".", "branch": "main", "remote": "origin", "remote_branch": "main"},
 {"name": "crash-sim", "path": "projects/crash-sim", "branch": "main", "remote": "origin",
  "remote_branch": "crash-sim-main", "tracking_path": "."},
 {"name": "render", "path": "projects/render", "branch": "master", "remote": "origin",
  "remote_branch": "render-master", "tracking_path": "."}
]}
```

### Pushing

```
python -I -B scripts/backup_status.py --root <root> --map <map.json> --push          # prints the plan, pushes nothing, exit 2
python -I -B scripts/backup_status.py --root <root> --map <map.json> --push --yes    # pushes
```

Plain pushes only: never `--force`, never a `+` refspec, never a delete; a rejected push is reported
and left alone. Branch and unpushed tags are pushed as explicit refspecs. After a successful push the
script records what it pushed in the pushed repo itself (`refs/backup-pushed-branch/<remote>/<branch>` and `refs/backup-pushed/<remote>/<tag>`), because a nested repo's commits are not in the root repo that holds the tracking ref. It also updates the tracking ref when that repo has the commit.

## Known limits

- Tags: without network access a tag counts as pushed if its commit is reachable from the tracking ref
  or `--push` recorded it. A tag created on an already-pushed commit and never pushed is not detected.
- If the tracking ref is missing, or its commit is not in the repo, the repo is reported BEHIND
  ("cannot verify") rather than ok.

## Proposed schedule (not created; needs Lazlo)

Windows Task Scheduler, hourly, status only:

- Trigger: every 1 hour, run only while Lazlo is logged on (no stored password needed).
- Action: `python -I -B scripts\backup_status.py --root <root> --map <map.json>`, output appended to a log in `scratch\`.
- Never schedule `--push`. Push stays by hand until Lazlo says otherwise.
- Ideally the schedule would alert (phone alert, like the disk floor) when `backup:` reports behind for more than a day.
