"""Slot check for every inbox poll (coordinator 2026-11-20, after RETRO lesson 37).

Prints one line: board running, waiting in the merge queue, active coders (running minus waiting), cap,
ready-task count, CPU and RAM. Prints a second line starting with FLAG when active < cap while ready tasks
exist and the board is full, or when headroom is large (RAM < 50% and CPU < 50%) while ready tasks wait.
When open tasks are blocked, prints holders and the blocked tasks' overlapping own claims (top three each).

A running task is WAITING (not an active coder) when its merge-queue entry is queued or running; this is the
same rule `task start` uses for max_agents. The only write is appending missing hold registrations to state/pauses.tsv.

  python -I scripts/slot_check.py <root>

<root> holds broadsword.toml, build/bin/broadsword.exe and state/queue. Every path is read relative to it.
"""
import pathlib
import re
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timedelta, timezone

WAITING_STATUSES = {"queued", "running"}
FAILED_STATUSES = ("rejected", "check_failed", "merged_check_failed")
LOOP_ROUNDS = 3
PAUSES_TSV_EMPTY_EXPIRY_MIN = 60  # Format rule of state/pauses.tsv header, not a tunable.


def hold_lines(root, now=None):
    """Report every hold, appending registrations only to an existing readable pauses file."""
    now = now or datetime.now(timezone.utc)
    now = now.replace(microsecond=0)

    def stamp(value):
        return value.strftime("%Y-%m-%dT%H:%M:%SZ")

    def parse_time(value):
        if not re.fullmatch(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ", value):
            raise ValueError(f"bad time: {value!r}")
        return datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=timezone.utc)

    lines = []
    for path in sorted((root / "state" / "queue").glob("hold-*")):
        fallback = path.name[len("hold-"):]
        try:
            text = path.read_text(encoding="utf-8")
        except (OSError, UnicodeError) as error:
            lines.append(f"HOLD: {fallback} since unknown entry unknown (unreadable: {error})")
            continue
        fields = dict(re.findall(r"(?m)^(\w+) = (.*)$", text))
        project = fields.get("project", "").strip() or fallback
        entry = fields.get("entry", "").strip() or "unknown"
        raw_time = fields.get("time", "").strip()
        try:
            hold_time = parse_time(raw_time.replace(" ", "T", 1).replace(" UTC", "Z"))
        except ValueError as error:
            lines.extend([f"HOLD: {project} since unknown entry {entry}",
                          f"HOLD NOT REGISTERED: {path.name}: {error}"])
            continue
        lines.append(f"HOLD: {project} since {stamp(hold_time)} entry {entry}")
        what = "hold-" + project
        if not re.fullmatch(r"[A-Za-z0-9_-]+", project):
            lines.append(f"HOLD NOT REGISTERED: invalid project: {project!r}")
            continue
        pauses = root / "state" / "pauses.tsv"
        try:
            # r+ never creates a missing file. Preserve all existing bytes on append.
            with pauses.open("r+", encoding="utf-8", newline="") as stream:
                content = stream.read()
                active = []
                for row in content.splitlines():
                    if not row.strip() or row.startswith("#"):
                        continue
                    parts = row.split("\t")
                    if parts[0] == "PAUSE" and len(parts) >= 6:
                        start = parse_time(parts[1])
                        expiry = (parse_time(parts[4]) if parts[4] else
                                  start + timedelta(minutes=PAUSES_TSV_EMPTY_EXPIRY_MIN))
                        if parts[3] == what:
                            active.append(start >= hold_time and start <= now < expiry)
                    elif parts[0] == "END" and len(parts) >= 3:
                        parse_time(parts[1])
                        if parts[2] == what:
                            active.clear()
                    else:
                        raise ValueError(f"malformed pauses row: {row!r}")
                if any(active):
                    continue
                # Keep the entire reason, including continuation lines, on one TSV line.
                match = re.search(r"(?ms)^reason = (.*?)(?=^\w+ = |\Z)", text)
                reason = " ".join(match.group(1).splitlines()) if match else ""
                reason = reason.replace("\t", " ")
                expiry = now + timedelta(minutes=PAUSES_TSV_EMPTY_EXPIRY_MIN)
                if content and not content.endswith("\n"):
                    raise ValueError("pauses file lacks final newline; refusing to change its last line")
                stream.seek(0, 2)
                stream.write(f"PAUSE\t{stamp(now)}\tqueue\t{what}\t\tregistered by slot_check: {reason}\n")
                stream.flush()
            lines.append(f"HOLD REGISTERED: {what} until {stamp(expiry)}")
        except (OSError, UnicodeError, ValueError, OverflowError) as error:
            lines.append(f"HOLD NOT REGISTERED: {what}: {error}")
    return lines


def conflict_lines(root, board_rows):
    """Only the newest queue entry can signal a conflict for a non-done task."""
    eligible = {row[0] for row in board_rows if len(row) >= 2 and row[1] != "done"}
    newest = {}
    for path in (root / "state" / "queue").glob("*.entry"):
        fields = dict(re.findall(r"(?m)^(\w+) = (.*)$", path.read_text(encoding="utf-8")))
        task = fields.get("task")
        entry = fields.get("id", path.stem)
        if task in eligible and (task not in newest or entry > newest[task][0]):
            newest[task] = (entry, fields)
    lines = []
    for task, (entry, fields) in sorted(newest.items()):
        if fields.get("status") == "conflict":
            match = re.search(r"conflicts in: (.*?)(?:\. |$)", fields.get("reason", ""))
            files = match.group(1).strip() if match else ""
            lines.append(f"CONFLICT: {task} {entry} {files or 'unknown'}")
    return lines


def broadsword_cmd(root):
    exe = root / "build" / "bin" / "broadsword.exe"
    if not exe.is_file():
        raise SystemExit(f"slot_check: {exe} not found")
    return [str(exe)]


def run_cli(root, *args):
    proc = subprocess.run([*broadsword_cmd(root), *args], cwd=root, capture_output=True, text=True)
    if proc.returncode != 0:
        raise SystemExit(f"slot_check: broadsword {' '.join(args)} exited {proc.returncode}: {proc.stderr.strip()}")
    return proc.stdout


def read_cap(root):
    m = re.search(r"(?m)^max_agents\s*=\s*(\d+)", (root / "broadsword.toml").read_text(encoding="utf-8"))
    if not m:
        raise SystemExit("slot_check: max_agents not found in broadsword.toml")
    return int(m.group(1))


def waiting_tasks(root):
    waiting = set()
    for e in (root / "state" / "queue").glob("*.entry"):
        kv = dict(re.findall(r"(?m)^(\w+) = (.*)$", e.read_text(encoding="utf-8")))
        if kv.get("status") in WAITING_STATUSES:
            waiting.add(kv.get("task"))
    return waiting


def rows(text):
    return [l.split("\t") for l in text.splitlines() if l.strip()]


def count_ready(running, open_rows):
    """Open tasks whose claims (columns: id, status, name, project, title, claims) do not overlap a running task's."""
    held = [(r[3], c.strip()) for r in running if len(r) > 5 for c in r[5].split(", ")]
    ready = 0
    for c in open_rows:
        if len(c) < 6:
            continue
        mine = [(c[3], x.strip()) for x in c[5].split(", ")]
        clash = any(p == hp and (a.startswith(b) or b.startswith(a)) for (p, a) in mine for (hp, b) in held)
        if not clash:
            ready += 1
    return ready


def claims_overlap(a, b):
    """Board overlaps_with for normalised claims, using Windows case folding."""
    if a == "*" or b == "*":
        return True
    a, _, ak = a.lower().partition("#")
    b, _, bk = b.lower().partition("#")
    if a == b:
        return not ak or not bk or ak == bk
    return (not ak and b.startswith(a + "/")) or (not bk and a.startswith(b + "/"))


def blame_lines(running, open_rows):
    """Count distinct blocked open task ids, on both sides of each overlap.

    Board TSV claims are already normalised. Keep their spelling for display.
    A holder's parenthetical names its most blocking claim (ties by claim text).
    """
    held = [(r[0], r[3], c.strip()) for r in running if len(r) > 5 for c in r[5].split(", ")]
    mine = [(r[0], r[3], c.strip()) for r in open_rows if len(r) > 5 for c in r[5].split(", ")]
    holder_tasks, held_tasks, own_tasks = defaultdict(set), defaultdict(set), defaultdict(set)
    keyed_files = {c.lower().partition("#")[0] for _, _, c in held + mine if "#" in c}
    for task_id, project, claim in mine:
        for holder, held_project, held_claim in held:
            if project == held_project and claims_overlap(claim, held_claim):
                holder_tasks[holder].add(task_id)
                held_tasks[holder, held_claim].add(task_id)
                own_tasks[claim].add(task_id)
    if not own_tasks:
        return []
    holders = []
    for holder in sorted(holder_tasks, key=lambda t: (-len(holder_tasks[t]), t))[:3]:
        claim = min((c for h, c in held_tasks if h == holder),
                    key=lambda c: (-len(held_tasks[holder, c]), c))
        holders.append(f"{holder} blocks {len(holder_tasks[holder])} "
                       f"(held: {claim} x{len(held_tasks[holder, claim])})")
    own = []
    for claim in sorted(own_tasks, key=lambda c: (-len(own_tasks[c]), c))[:3]:
        label = claim + " (whole file)" if claim.lower() in keyed_files else claim
        own.append(f"{label} x{len(own_tasks[claim])}")
    return ["holders: " + "; ".join(holders), "blocked by own claim: " + "; ".join(own)]


def evaluate(cap, run_ids, wait_n, ready, ram, cpu):
    """Returns (summary line, FLAG line or None)."""
    active = len(run_ids) - wait_n
    line = (f"slots: board running {len(run_ids)}/{cap}, waiting in queue {wait_n}, active coders {active}, "
            f"ready tasks (claims free) {ready}, RAM {ram}% CPU {cpu}%")
    flag = None
    if ready and active < cap and len(run_ids) >= cap:
        flag = f"FLAG: only {active} active coders of cap {cap} while {ready} ready tasks wait (board full of queued work)"
    elif ready and 0 <= ram < 50 and 0 <= cpu < 50 and active < cap:
        flag = f"FLAG: large headroom (RAM {ram}%, CPU {cpu}%) while {ready} ready tasks wait"
    return line, flag


def machine_load():
    ps = subprocess.run(["powershell", "-NoProfile", "-Command",
        "$o=Get-CimInstance Win32_OperatingSystem; $c=(Get-CimInstance Win32_Processor | Measure-Object LoadPercentage -Average).Average;"
        "'{0:N1} {1:N1}' -f (100-$o.FreePhysicalMemory/$o.TotalVisibleMemorySize*100), $c"],
        capture_output=True, text=True).stdout.split()
    if len(ps) == 2:
        return float(ps[0].replace(",", ".")), float(ps[1].replace(",", "."))
    return -1.0, -1.0


def main(argv):
    if len(argv) != 2:
        raise SystemExit("usage: python -I scripts/slot_check.py <root>")
    root = pathlib.Path(argv[1]).resolve()
    cap = read_cap(root)
    running = rows(run_cli(root, "task", "list", "--status", "running"))
    run_ids = {r[0] for r in running}
    wait_n = len(run_ids & waiting_tasks(root))
    open_rows = rows(run_cli(root, "task", "list", "--status", "open"))
    ready = count_ready(running, open_rows)
    ram, cpu = machine_load()
    line, flag = evaluate(cap, run_ids, wait_n, ready, ram, cpu)
    print(line)
    if flag:
        print(flag)
    for blame in blame_lines(running, open_rows):
        print(blame)
    # Loop breaker (2026-11-20, Lazlo "not get stuck in a loop"): a running task with 3 or more rejected/check_failed
    # queue entries is stuck. Stop retrying it as-is: adjudicate, then split or escalate once (state/priority.md).
    fails = {}
    for e in (root / "state" / "queue").glob("*.entry"):
        kv = dict(re.findall(r"(?m)^(\w+) = (.*)$", e.read_text(encoding="utf-8")))
        if kv.get("status") in FAILED_STATUSES and kv.get("task") in run_ids:
            fails[kv["task"]] = fails.get(kv["task"], 0) + 1
    for t, n in sorted(fails.items(), key=lambda x: -x[1]):
        if n >= LOOP_ROUNDS:
            print(f"LOOP: {t} has {n} failed queue rounds: adjudicate, then split or escalate once; no plain retry")
    for alert in hold_lines(root):
        print(alert)
    board_rows = rows(run_cli(root, "task", "list"))
    for alert in conflict_lines(root, board_rows):
        print(alert)


if __name__ == "__main__":
    main(sys.argv)
