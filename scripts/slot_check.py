"""Slot check for every inbox poll (coordinator 2026-11-20, after RETRO lesson 37).

Prints one line: board running, waiting in the merge queue, active coders (running minus waiting), cap,
ready-task count, CPU and RAM. Prints a second line starting with FLAG when active < cap while ready tasks
exist and the board is full, or when headroom is large (RAM < 50% and CPU < 50%) while ready tasks wait.
When open tasks are blocked, prints holders and the blocked tasks' overlapping own claims (top three each).

A running task is WAITING (not an active coder) when its merge-queue entry is queued or running; this is the
same rule `task start` uses for max_agents. Read-only.

  python -I scripts/slot_check.py <root>

<root> holds broadsword.toml, build/bin/broadsword.exe and state/queue. Every path is read relative to it.
"""
import pathlib
import re
import subprocess
import sys
from collections import defaultdict

WAITING_STATUSES = {"queued", "running"}
FAILED_STATUSES = ("rejected", "check_failed", "merged_check_failed")
LOOP_ROUNDS = 3


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


if __name__ == "__main__":
    main(sys.argv)
