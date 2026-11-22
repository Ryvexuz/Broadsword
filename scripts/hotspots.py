"""Hotspot report: which files cost the most contention (size, commits on the trunk, board claims, claim waits).

Carries Lazlo's file-size audit rule (2026-11-22): REPORT ONLY. It changes nothing on the board and starts nothing.

  python -I scripts/hotspots.py [--root R] [--hours H] [--until UNIX_SECONDS] [--write]

--root R   the checkout that holds state/ and projects/ (default: the main checkout of the repo the current folder
           belongs to, found through `git rev-parse --git-common-dir`, so a task worktree reads the real board).
           Settings come from R/broadsword.toml when --root is given, else from the broadsword.toml next to this
           script (so a worktree sees its own new keys). The keys, all required, whole numbers:
             hotspot_lines, hotspot_window_hours, hotspot_wait_hours, hotspot_claims, hotspot_commits
--hours H  overrides hotspot_window_hours.   --until T  window end (default: now).   --write  also writes the TSV to
           <state_dir>/hotspots.tsv (atomic).

Projects: the root (`broadsword`) and each Git repo under projects_dir. Files: `git ls-files` with suffix .rs .py .ps1
.toml .md. A row is printed for a file with any commit, claim or wait in the window, or a flag.
Output: TSV, header `project path lines commits claims waited_tasks wait_hours flag`, sorted by wait_hours, claims,
commits, lines (all descending); last line `hotspots: S split, H hot; board waits T task-hours from K tasks in W h`
plus `; N records skipped` (task records that could not be read).

Waits: a started task that waited over 60 s (started - created) waited on a file when another started task of the same
project, whose claim collides on that file, ran during the wait (started..ended, or to the waiter's start if not
ended). A claim `f#k` collides only with the same key or a whole-file claim of f; a folder claim covers every file
under it. Per file a task's wait is its longest overlap (counted once); a task's own wait for the last line is its
longest per-file wait.
"""
import argparse
import os
import pathlib
import subprocess
import sys
import time
import tomllib

SUFFIXES = (".rs", ".py", ".ps1", ".toml", ".md")
MIN_WAIT_SECONDS = 60
KEYS = ("hotspot_lines", "hotspot_window_hours", "hotspot_wait_hours", "hotspot_claims", "hotspot_commits")
HEADER = "project\tpath\tlines\tcommits\tclaims\twaited_tasks\twait_hours\tflag"


def fail(msg):
    raise SystemExit(f"hotspots: {msg}")


def git(cwd, *args):
    proc = subprocess.run(["git", "-c", "core.quotepath=off", *args], cwd=cwd, capture_output=True)
    if proc.returncode != 0:
        return None
    return proc.stdout.decode("utf-8", "replace")


def load_settings(toml_path):
    try:
        data = tomllib.loads(toml_path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as e:
        fail(f"cannot read {toml_path}: {e}")
    for k in KEYS + ("state_dir", "projects_dir"):
        if k not in data:
            fail(f"{toml_path} has no key {k}")
    for k in KEYS:
        v = data[k]
        if isinstance(v, bool) or not isinstance(v, int) or v < 1:
            fail(f"{k} must be a whole number of at least 1, got {v!r}")
    return data


def default_root():
    out = git(pathlib.Path.cwd(), "rev-parse", "--path-format=absolute", "--git-common-dir")
    if out:
        return pathlib.Path(out.strip()).parent
    return pathlib.Path.cwd()


def trunk(repo):
    for name in ("main", "master"):
        if git(repo, "rev-parse", "--verify", "--quiet", name + "^{commit}") is not None:
            return name
    return "HEAD"


def projects(root, projects_dir):
    found = [("broadsword", root)]
    pd = root / projects_dir
    if pd.is_dir():
        for d in sorted(pd.iterdir()):
            if (d / ".git").exists():
                found.append((d.name, d))
    return found


def count_lines(path):
    try:
        data = path.read_bytes()
    except OSError:
        return None
    return data.count(b"\n") + (1 if data and not data.endswith(b"\n") else 0)


def list_files(repo):
    out = git(repo, "ls-files")
    return [f for f in (out or "").splitlines() if f.endswith(SUFFIXES)]


def commit_counts(repo, start, until):
    out = git(repo, "log", trunk(repo), f"--since={int(start)}", f"--until={int(until)}", "--name-only", "--pretty=format:@@@")
    counts = {}
    for line in (out or "").splitlines():
        if line and line != "@@@":
            counts[line] = counts.get(line, 0) + 1
    return counts


def read_task(path):
    """The task record as a dict of ints/strings, or None when unreadable or missing its core fields."""
    try:
        text = path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None
    kv = {}
    for line in text.splitlines():
        if " = " in line:
            k, v = line.split(" = ", 1)
            kv[k.strip()] = v
        elif line.endswith(" =") :
            kv[line[:-2].strip()] = ""
    try:
        t = {
            "id": kv["id"],
            "project": kv["project"],
            "created": int(kv["created"]),
            "started": int(kv["started"]) if kv.get("started", "").strip() else None,
            "ended": int(kv["ended"]) if kv.get("ended", "").strip() else None,
            "claims": [c.strip() for c in kv.get("claims", "").split("\\n") if c.strip()],
        }
    except (KeyError, ValueError):
        return None
    return t


def read_board(state):
    tasks, skipped = [], 0
    d = state / "tasks"
    if not d.is_dir():
        return tasks, skipped
    for p in sorted(d.glob("*.task")):
        t = read_task(p)
        if t is None:
            skipped += 1
        else:
            tasks.append(t)
    return tasks, skipped


def claim_on(claims, f):
    """None when no claim covers file f, True for a whole-file claim, else the set of keys claimed."""
    whole, keys, covered = False, set(), False
    for c in claims:
        path, _, key = c.partition("#")
        path = path.strip().rstrip("/")
        if path == f or f.startswith(path + "/"):
            covered = True
            if key and path == f:
                keys.add(key.strip())
            else:
                whole = True
    if not covered:
        return None
    return True if whole else keys


def collide(a, b):
    if a is True or b is True:
        return True
    return bool(a & b)


def waits(project_tasks, files):
    """({file: {task id: wait seconds}}, {task id: longest wait seconds}) for one project."""
    per_file, per_task = {}, {}
    started = [t for t in project_tasks if t["started"] is not None]
    for w in started:
        if w["started"] - w["created"] <= MIN_WAIT_SECONDS:
            continue
        for f in files:
            mine = claim_on(w["claims"], f)
            if mine is None:
                continue
            best = 0
            for o in started:
                if o is w:
                    continue
                theirs = claim_on(o["claims"], f)
                if theirs is None or not collide(mine, theirs):
                    continue
                lo = max(w["created"], o["started"])
                hi = min(w["started"], o["ended"] if o["ended"] is not None else w["started"])
                best = max(best, hi - lo)
            if best > 0:
                per_file.setdefault(f, {})[w["id"]] = best
                per_task[w["id"]] = max(per_task.get(w["id"], 0), best)
    return per_file, per_task


def flag(cfg, lines, wait_h, claims, commits):
    hot = wait_h >= cfg["hotspot_wait_hours"] or claims >= cfg["hotspot_claims"] or commits >= cfg["hotspot_commits"]
    if lines > cfg["hotspot_lines"] and hot:
        return "split"
    return "hot" if hot else ""


def build_report(root, cfg, hours, until):
    start = until - hours * 3600
    state = root / cfg["state_dir"]
    tasks, skipped = read_board(state)
    tasks = [t for t in tasks if start <= t["created"] <= until]
    rows, task_waits = [], {}
    for name, repo in projects(root, cfg["projects_dir"]):
        files = list_files(repo)
        commits = commit_counts(repo, start, until)
        ptasks = [t for t in tasks if t["project"] == name]
        per_file, per_task = waits(ptasks, files)
        for tid, secs in per_task.items():
            task_waits[(name, tid)] = secs
        for f in files:
            n_commits = commits.get(f, 0)
            n_claims = sum(1 for t in ptasks if claim_on(t["claims"], f) is not None)
            w = per_file.get(f, {})
            wait_h = sum(w.values()) / 3600
            lines = count_lines(repo / f)
            if lines is None:
                continue
            fl = flag(cfg, lines, wait_h, n_claims, n_commits)
            if not (n_commits or n_claims or w or fl):
                continue
            rows.append((name, f, lines, n_commits, n_claims, len(w), wait_h, fl))
    rows.sort(key=lambda r: (r[6], r[4], r[3], r[2]), reverse=True)
    out = [HEADER]
    for r in rows:
        out.append(f"{r[0]}\t{r[1]}\t{r[2]}\t{r[3]}\t{r[4]}\t{r[5]}\t{r[6]:.2f}\t{r[7]}")
    s = sum(1 for r in rows if r[7] == "split")
    h = sum(1 for r in rows if r[7] == "hot")
    total = sum(task_waits.values()) / 3600
    out.append(f"hotspots: {s} split, {h} hot; board waits {total:.1f} task-hours from {len(task_waits)} tasks in {hours:g} h; {skipped} records skipped")
    return out


def main(argv):
    ap = argparse.ArgumentParser(prog="hotspots.py", add_help=True)
    ap.add_argument("--root")
    ap.add_argument("--hours", type=float)
    ap.add_argument("--until", type=int)
    ap.add_argument("--write", action="store_true")
    a = ap.parse_args(argv)
    if a.root:
        root = pathlib.Path(a.root).resolve()
        toml_path = root / "broadsword.toml"
    else:
        root = default_root()
        toml_path = pathlib.Path(__file__).resolve().parent.parent / "broadsword.toml"
    cfg = load_settings(toml_path)
    hours = a.hours if a.hours is not None else float(cfg["hotspot_window_hours"])
    if hours <= 0:
        fail("--hours must be above 0")
    until = a.until if a.until is not None else int(time.time())
    out = build_report(root, cfg, hours, until)
    text = "\n".join(out) + "\n"
    sys.stdout.write(text)
    if a.write:
        dest = root / cfg["state_dir"] / "hotspots.tsv"
        dest.parent.mkdir(parents=True, exist_ok=True)
        tmp = dest.with_name(dest.name + f".{os.getpid()}.tmp")
        tmp.write_text(text, encoding="utf-8", newline="\n")
        os.replace(tmp, dest)


if __name__ == "__main__":
    main(sys.argv[1:])
