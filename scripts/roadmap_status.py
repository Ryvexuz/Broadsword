"""Check ROADMAP.md against CAPABILITY_ROADMAP.md and against the repo.

  python -I scripts/roadmap_status.py [--root DIR] [--capability FILE] [--roadmap FILE]

Exit 0 and `roadmap: ok, N nodes` when:
  - every node row in CAPABILITY_ROADMAP Part 2 appears exactly once in ROADMAP.md, and nothing else does;
  - each node's name, stage list and "Done when" line match CAPABILITY_ROADMAP (stage list derived from Part 4's table);
  - every status is one of: not started, in progress, done;
  - every status other than "not started" has evidence, and every evidence item checks out in the repo:
      `file:<path>`            the path exists (under the root, or under the main checkout for sibling repos in projects/)
      `tag:<name>`             the Git tag exists
      `commit:<sha>`           the commit is reachable from HEAD
      `commit:<repo>@<sha>`    the same, in the repo at <repo> (for example projects/render)
    "done" also needs at least one `tag:` item. A status whose evidence is missing or does not check out is reported as
    UNVERIFIED and makes the exit code 1; it is never counted as done.
Exit 1 with one line per problem otherwise; exit 2 when a Part 2 node row is malformed (not three cells). No network, no model calls.

  python -I scripts/roadmap_status.py --plans [--root DIR] [--roadmap FILE]

Instead of the check above, print one line per ROADMAP.md node: whether docs/nodes/<node>.md exists, how many `check <id>`
fenced blocks it lists, how many of the eight plan sections (headings numbered 1 to 8) it has, and whether section 4 (External
needs) lists a decision for Lazlo. Read-only; it never changes ROADMAP.md and exits 0 unless a file is missing (1).
"""
import sys

sys.dont_write_bytecode = True

import argparse
import pathlib
import re
import subprocess

class MalformedRow(Exception):
    """A Part 2 node row that cannot be read; the script exits 2."""


STATUSES = ("not started", "in progress", "done")
NODE_RE = re.compile(r"^\d+\.\d+$")


def read(path):
    return pathlib.Path(path).read_text(encoding="utf-8")


def table_cells(line):
    """Cells of a markdown table row, outer pipes removed, each stripped."""
    return [c.strip() for c in line.strip().strip("|").split(" | ")]


def part(text, start, end):
    """Lines from the heading that starts with `start` up to (not including) the heading that starts with `end`."""
    lines = text.splitlines()
    first = next((i for i, l in enumerate(lines) if l.startswith(start)), None)
    if first is None:
        raise ValueError(f"CAPABILITY_ROADMAP has no heading starting {start!r}")
    last = next((i for i in range(first + 1, len(lines)) if lines[i].startswith(end)), len(lines))
    return lines[first:last]


def capability_nodes(text):
    """{node: (name, done_when)} from the Part 2 tables, in file order."""
    nodes = {}
    for line in part(text, "## Part 2", "## Part 3"):
        if not line.startswith("| "):
            continue
        cells = table_cells(line)
        m = re.match(r"^(\d+\.\d+) (.+)$", cells[0])
        if not m:
            continue
        if len(cells) != 3:
            raise MalformedRow(f"CAPABILITY_ROADMAP Part 2 row for node {m.group(1)} has {len(cells)} cells, not 3: {line[:80]!r}")
        name = re.split(r" \(| \\\[", m.group(2))[0].strip()
        if m.group(1) in nodes:
            raise ValueError(f"CAPABILITY_ROADMAP lists node {m.group(1)} twice")
        nodes[m.group(1)] = (name, cells[2])
    return nodes


def sort_key(node):
    a, b = node.split(".")
    return int(a), int(b)


def stage_map(text, nodes):
    """{node: sorted list of Broadsword stage numbers} from Part 4's mapping table. Nodes it never names get []."""
    result = {n: set() for n in nodes}
    for line in part(text, "### Mapping to your 12 stages", "Two suggested adjustments"):
        if not line.startswith("| "):
            continue
        cells = table_cells(line)
        m = re.match(r"^(\d+)\. ", cells[0])
        if not m or len(cells) != 2:
            continue
        stage = int(m.group(1))
        cell = cells[1]
        for t in re.findall(r"Tier (\d)", cell):
            for n in nodes:
                if n.startswith(t + "."):
                    result[n].add(stage)
        for a1, a2, b1, b2 in re.findall(r"(\d+)\.(\d+)–(\d+)\.(\d+)", cell):
            for n in nodes:
                tier, idx = sort_key(n)
                if tier == int(a1) == int(b1) and int(a2) <= idx <= int(b2):
                    result[n].add(stage)
        singles = re.sub(r"\d+\.\d+–\d+\.\d+", " ", cell)
        for n in re.findall(r"\b(\d+\.\d+)\b", singles):
            if n in result:
                result[n].add(stage)
    return {n: sorted(s) for n, s in result.items()}


def stage_cell(stages):
    return ", ".join(str(s) for s in stages) if stages else "none"


def roadmap_rows(text):
    """[(node, name, stage, status, evidence, done_when)] from the ROADMAP.md table rows."""
    rows = []
    for line in text.splitlines():
        if not line.startswith("| "):
            continue
        cells = table_cells(line)
        if len(cells) == 6 and NODE_RE.match(cells[0]):
            rows.append(tuple(cells))
    return rows


def main_checkout(root):
    """The main checkout's root: where projects/ sibling repos live when this is a worktree."""
    r = subprocess.run(["git", "-C", str(root), "rev-parse", "--path-format=absolute", "--git-common-dir"],
                       capture_output=True, text=True)
    if r.returncode != 0:
        return root
    return pathlib.Path(r.stdout.strip()).parent


def inside(base, rel):
    """The path rel under base if it is a plain relative path that resolves inside base, else None.
    Refuses empty, absolute, backslash-rooted, drive-letter and UNC paths and any '..' in either separator."""
    if not rel or rel.startswith(("/", "\\")) or re.match(r"^[A-Za-z]:", rel):
        return None
    if ".." in re.split(r"[/\\]", rel):
        return None
    try:
        full = (base / rel).resolve()
        if not full.is_relative_to(base.resolve()):
            return None
    except (OSError, ValueError):
        return None
    return full


def git_ok(repo, *args):
    return subprocess.run(["git", "-C", str(repo), *args], capture_output=True).returncode == 0


def check_evidence(item, root, main):
    """Return None when the evidence item checks out, else a reason."""
    kind, _, value = item.partition(":")
    if kind == "file":
        found = False
        for base in dict.fromkeys((root, main)):
            full = inside(base, value)
            if full is not None:
                found = True
                if full.exists():
                    return None
        if not found:
            return f"path {value!r} is not a relative path inside the repo"
        return f"file {value} does not exist"
    if kind == "tag":
        r = subprocess.run(["git", "-C", str(root), "tag", "--list", value], capture_output=True, text=True)
        return None if value and value in r.stdout.split() else f"tag {value} does not exist"
    if kind == "commit":
        repo_rel, sep, sha = value.rpartition("@")
        if not sep:
            repo_rel, sha = "", value
        if not re.fullmatch(r"[0-9a-f]{7,40}", sha):
            return f"commit id {sha!r} is not a hex sha"
        if repo_rel and not any(inside(b, repo_rel) for b in (root, main)):
            return f"repo path {repo_rel!r} is not a relative path inside the repo"
        for base in dict.fromkeys((root, main)):
            repo = inside(base, repo_rel) if repo_rel else base
            if repo is not None and repo.is_dir() and git_ok(repo, "cat-file", "-e", sha + "^{commit}") \
                    and git_ok(repo, "merge-base", "--is-ancestor", sha, "HEAD"):
                return None
        return f"commit {value} is not reachable from HEAD"
    return f"unknown evidence kind {kind!r} (use file:, tag: or commit:)"


def check(root, capability, roadmap):
    """Return (problems, unverified, n_nodes, counts)."""
    problems, unverified = [], []
    cap_text = read(capability)
    nodes = capability_nodes(cap_text)
    stages = stage_map(cap_text, nodes)
    rm_text = read(roadmap)
    rows = roadmap_rows(rm_text)
    main = main_checkout(root)
    counts = {s: 0 for s in STATUSES}

    seen = {}
    for row in rows:
        seen.setdefault(row[0], []).append(row)
    for n in nodes:
        if n not in seen:
            problems.append(f"{n}: missing from ROADMAP.md")
    for n, rs in seen.items():
        if n not in nodes:
            problems.append(f"{n}: in ROADMAP.md but not in CAPABILITY_ROADMAP Part 2")
        elif len(rs) > 1:
            problems.append(f"{n}: appears {len(rs)} times in ROADMAP.md")
    if not re.search(r"^## Open decisions", rm_text, re.M):
        problems.append("ROADMAP.md has no '## Open decisions' section")

    for node, name, stage, status, evidence, done_when in rows:
        if node not in nodes or len(seen[node]) > 1:
            continue
        cap_name, cap_done = nodes[node]
        if name != cap_name:
            problems.append(f"{node}: name {name!r} differs from CAPABILITY_ROADMAP {cap_name!r}")
        if stage != stage_cell(stages[node]):
            problems.append(f"{node}: stage {stage!r} differs from Part 4's mapping {stage_cell(stages[node])!r}")
        if done_when != cap_done:
            problems.append(f"{node}: 'Done when' is not an exact copy of CAPABILITY_ROADMAP")
        if status not in STATUSES:
            problems.append(f"{node}: status {status!r} is not one of {', '.join(STATUSES)}")
            continue
        items = [] if evidence in ("", "-") else [e.strip().strip("`") for e in evidence.split(";") if e.strip()]
        if status == "not started":
            counts[status] += 1
            if items:
                problems.append(f"{node}: 'not started' must have evidence '-', not {evidence!r}")
            continue
        why = []
        if not items:
            why.append("no evidence cited")
        for item in items:
            r = check_evidence(item, root, main)
            if r:
                why.append(r)
        if status == "done" and not any(i.startswith("tag:") for i in items):
            why.append("'done' needs a tag: evidence item")
        if why:
            unverified.append(f"{node}: UNVERIFIED {status}: " + "; ".join(why))
        else:
            counts[status] += 1
    return problems, unverified, len(nodes), counts


PLAN_SECTIONS = 8
HEADING_RE = re.compile(r"^#{2,3}\s+(\d+)[.):]?\s")
CHECK_FENCE_RE = re.compile(r"^\s*(`{3,}|~{3,})\s*check\s+\S+")
NONE_RE = re.compile(r"^\W*none\b", re.I)


def plan_info(text):
    """(check_count, sections_present, section4_has_decision) for one docs/nodes/<n>.md text.
    A section is present when a '##' or '###' heading starts with its number 1 to 8. A check is a fenced block whose info
    string is `check <id>`. Section 4 lists a decision for Lazlo when its body mentions Lazlo and is not just 'none'."""
    lines = text.splitlines()
    checks = sum(1 for l in lines if CHECK_FENCE_RE.match(l))
    present, bodies, current, in_fence = set(), {}, None, False
    for l in lines:
        if l.lstrip().startswith(("```", "~~~")):
            in_fence = not in_fence
        m = None if in_fence else HEADING_RE.match(l)
        if m and 1 <= int(m.group(1)) <= PLAN_SECTIONS:
            current = int(m.group(1))
            present.add(current)
            bodies.setdefault(current, [])
        elif current is not None:
            bodies[current].append(l)
    body4 = "\n".join(bodies.get(4, [])).strip()
    decision = bool(body4) and not NONE_RE.match(body4) and re.search(r"\bLazlo\b", body4) is not None
    return checks, sorted(present), decision


def plans_report(root, roadmap):
    """Lines for --plans: one per ROADMAP.md node, in node order, then a summary."""
    nodes = sorted({r[0] for r in roadmap_rows(read(roadmap))}, key=sort_key)
    out, have = [], 0
    for n in nodes:
        path = root / "docs" / "nodes" / f"{n}.md"
        if not path.is_file():
            out.append(f"{n}: plan no")
            continue
        have += 1
        checks, present, decision = plan_info(read(path))
        missing = [str(i) for i in range(1, PLAN_SECTIONS + 1) if i not in present]
        line = f"{n}: plan yes, {checks} checks, sections {len(present)}/{PLAN_SECTIONS}"
        if missing:
            line += " (missing " + ", ".join(missing) + ")"
        out.append(line + f", decision for Lazlo {'yes' if decision else 'no'}")
    out.append(f"plans: {have} of {len(nodes)} nodes have a plan")
    return out


def main():
    here = pathlib.Path(__file__).resolve().parent.parent
    ap = argparse.ArgumentParser()
    ap.add_argument("--root", default=str(here))
    ap.add_argument("--capability", default=None)
    ap.add_argument("--roadmap", default=None)
    ap.add_argument("--plans", action="store_true", help="show which nodes have a docs/nodes plan instead of checking")
    a = ap.parse_args()
    root = pathlib.Path(a.root).resolve()
    capability = pathlib.Path(a.capability) if a.capability else root / "CAPABILITY_ROADMAP.md"
    roadmap = pathlib.Path(a.roadmap) if a.roadmap else root / "ROADMAP.md"
    if a.plans:
        if not roadmap.is_file():
            print(f"roadmap: FAILED, {roadmap} not found")
            return 1
        print("\n".join(plans_report(root, roadmap)))
        return 0
    for f in (capability, roadmap):
        if not f.is_file():
            print(f"roadmap: FAILED, {f} not found")
            return 1
    try:
        problems, unverified, n, counts = check(root, capability, roadmap)
    except MalformedRow as e:
        print(f"roadmap: FAILED, {e}")
        return 2
    except ValueError as e:
        print(f"roadmap: FAILED, {e}")
        return 1
    for p in problems + unverified:
        print(p)
    if problems or unverified:
        print(f"roadmap: FAILED, {len(problems)} problems, {len(unverified)} unverified statuses, {n} nodes in CAPABILITY_ROADMAP")
        return 1
    print(f"roadmap: ok, {n} nodes")
    print("  " + ", ".join(f"{counts[s]} {s}" for s in STATUSES))
    return 0


if __name__ == "__main__":
    sys.exit(main())
