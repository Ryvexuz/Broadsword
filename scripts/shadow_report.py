"""Compare the gating review with the shadow review for every queue entry that has both.

Run:  python -I -B scripts/shadow_report.py [--root <folder with broadsword.toml>]

Reads <root>/state/queue/*.entry (the `key = value` records the queue writes). Prints one row per entry with both verdicts,
whether they agree, the BLOCKING lines one model found that the other did not (by file:line, else by text), the cost and the
time of the shadow review, and ends with the agreement rate over entries where the shadow gave a real PASS or FAIL.
Read-only; never calls a model.

Adjudication: `--open` lists every shadow-only BLOCKING line that has no verdict in <root>/state/shadow-verdicts.tsv and exits 3 when
any are open, else 0. Verdicts (real | false | known) are recorded with scripts/shadow_adjudicate.py. The report ends with a
summary per shadow model: findings, verdicts, precision = real / (real + false), gating misses = real findings in merged entries.
"""

import sys

sys.dont_write_bytecode = True  # never write .pyc files into the repo

import argparse
import re
from pathlib import Path


def parse_entry(text: str) -> dict:
    """The queue's record format: `key = value` lines; a value with newlines is stored with `\\n` escapes."""
    rec = {}
    for line in text.splitlines():
        if " = " in line:
            k, _, v = line.partition(" = ")
        elif line.endswith(" ="):
            k, v = line[:-2], ""
        else:
            continue
        rec[k.strip()] = v.replace("\\n", "\n").replace("\\\\", "\\")
    return rec


BULLET = re.compile(r"^(\s*)(?:[-*]|\d+[.)])\s+")


def finding_key(line: str) -> str:
    """`src/a.rs:12 - text` (or `src/a.rs:~12`) is identified by its file:line; a finding without one by its normalised text.
    A leading bullet (`-`, `*`, `1.`) is not part of the finding."""
    line = BULLET.sub("", line, count=1)
    m = re.match(r"\s*([\w./\\-]+:~?\d+)", line)
    return m.group(1).replace("~", "").lower() if m else re.sub(r"\W+", " ", line).strip().lower()


def findings(rec: dict, key: str) -> dict:
    return {finding_key(l): l.strip() for l in rec.get(key, "").splitlines() if l.strip()}


def fingerprint(text: str) -> str:
    """The first 8 normalised words of a finding's text after its `path:line` (empty for a text-only finding)."""
    body = BULLET.sub("", text, count=1).strip()
    m = re.match(r"[\w./\\-]+:~?\d+", body)
    if not m:
        return ""
    return " ".join(re.sub(r"\W+", " ", body[m.end():]).lower().split()[:8])


def finding_id(loc: str, text: str) -> str:
    """`path:line#first eight words` for a located finding, so two findings at one line stay apart; text-only: the text key."""
    fp = fingerprint(text)
    return f"{loc}#{fp}" if fp else loc


def grouped_findings(rec: dict, key: str) -> list:
    """Like findings(), but a line that does not start a finding is the continuation of the finding before it (a reviewer quote
    that spans lines). A line starts a finding when it begins with `path:line` or is a bullet (`-`, `*`, `1.`) at the indent of
    the finding's own bullet or shallower; a text-only bullet is a finding of its own. A block with neither paths nor bullets is
    one finding per line. Returns [(location key, text)] in order; two findings at one location are both kept."""
    lines = [l.rstrip() for l in rec.get(key, "").splitlines() if l.strip()]
    head = re.compile(r"[\w./\\-]+:~?\d+")
    bodies = [BULLET.sub("", l, count=1).strip() for l in lines]
    if not any(head.match(b) or BULLET.match(l) for l, b in zip(lines, bodies)):
        return [(finding_key(l), l.strip()) for l in lines]
    out, last, last_indent = [], None, 0
    for l, b in zip(lines, bodies):
        m = BULLET.match(l)
        indent = len(l) - len(l.lstrip())
        if last is None or head.match(b) or (m and indent <= last_indent):
            last, last_indent = len(out), indent
            out.append([finding_key(l), b])
        else:
            out[last][1] += " " + l.strip()
    return [(k, t) for k, t in out]


VERDICTS = ("real", "false", "known")
VERDICT_HEADER = "\t".join(["entry", "finding_key", "verdict", "evidence", "judge", "date"])
VERDICT_FILE = Path("state") / "shadow-verdicts.tsv"


def norm_key(key: str) -> str:
    return re.sub(r":~(\d)", r":\1", key.strip().lower())


def load_entries(root: Path) -> list:
    """(entry id, record) for every queue entry that has a shadow record."""
    qdir = root / "state" / "queue"
    out = []
    for p in sorted(qdir.glob("*.entry")) if qdir.is_dir() else []:
        try:
            rec = parse_entry(p.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
        if rec.get("shadow_verdict"):
            out.append((p.stem, rec))
    return out


LOCATED = re.compile(r"[\w./\\-]+:\d+$")


def words(text: str) -> set:
    """The normalised words of a finding's text after its `path:line`."""
    body = BULLET.sub("", text, count=1).strip()
    m = re.match(r"[\w./\\-]+:~?\d+", body)
    return set(re.sub(r"\W+", " ", body[m.end():] if m else body).lower().split())


def matched_by_gating(sb: list, gb: list) -> set:
    """Indexes into sb of the shadow findings the gating review also found. A gating finding at path:line accounts for ONE shadow
    finding at that line: the one whose words overlap it most (greedy, best pair first). A second, different shadow finding at the
    same line is not accounted for and stays open. A text-only shadow finding is matched by its text key."""
    gating_keys = {k for k, _ in gb}
    done = {i for i, (loc, _) in enumerate(sb) if not LOCATED.match(norm_key(loc)) and loc in gating_keys}
    pairs = []
    for i, (loc, text) in enumerate(sb):
        if LOCATED.match(norm_key(loc)):
            for j, (gloc, gtext) in enumerate(gb):
                if gloc == loc:
                    a, b = words(text), words(gtext)
                    pairs.append((len(a & b) / len(a | b) if a | b else 0.0, -i, -j, i, j))
    used_g = set()
    for _, _, _, i, j in sorted(pairs, reverse=True):
        if i not in done and j not in used_g:
            done.add(i)
            used_g.add(j)
    return done


def shadow_only(entries: list) -> list:
    """Every shadow-only BLOCKING line (gating FAIL entries included; ERROR/SKIPPED shadows have none and are skipped)."""
    out = []
    for eid, r in entries:
        if r["shadow_verdict"] in ("ERROR", "SKIPPED"):
            continue
        gb, sb = grouped_findings(r, "blocking"), grouped_findings(r, "shadow_blocking")
        merged = r.get("status") == "merged"
        commit = (r.get("main_sha") if merged else "") or r.get("checked_tip") or r.get("reviewed_tip") or r.get("rebased_tip") or "-"
        model = r.get("shadow_model_used") or r.get("shadow_model") or "unknown"
        known = matched_by_gating(sb, gb)
        seen = {}
        for idx, (loc, line) in enumerate(sb):
            fid = norm_key(finding_id(loc, line))
            if idx in known or fid in seen:
                continue
            seen[fid] = {"entry": eid, "commit": commit, "key": fid, "loc": norm_key(loc), "line": line, "model": model, "merged": merged}
            out.append(seen[fid])
    return out


def judge(items: list, verdicts: dict) -> list:
    """[(item, verdict row or None, note)]. A verdict row closes a finding by its full key (`path:line#words`). A row that gives
    only `path:line` closes the finding at that location only when exactly one shadow-only finding sits there; with several it
    closes none, and the open lines carry a note."""
    at = {}
    for it in items:
        at.setdefault((it["entry"], it["loc"]), []).append(it)
    out = []
    for it in items:
        v, note = verdicts.get((it["entry"], it["key"])), ""
        if v is None and it["key"] != it["loc"] and (it["entry"], it["loc"]) in verdicts:
            if len(at[(it["entry"], it["loc"])]) == 1:
                v = verdicts[(it["entry"], it["loc"])]
            else:
                note = f"verdict for bare {it['loc']} ignored: {len(at[(it['entry'], it['loc'])])} findings there; use the full key"
        out.append((it, v, note))
    return out


def load_verdicts(root: Path) -> dict:
    """{(entry, key): row}. Rows are read as they are; a later row for the same pair replaces an earlier one; rows with a
    verdict outside real/false/known, or fewer than 3 cells, are ignored."""
    path = root / VERDICT_FILE
    out = {}
    if not path.is_file():
        return out
    for i, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines()):
        cells = line.split("\t")
        if i == 0 and cells[0] == "entry" or len(cells) < 3 or not line.strip():
            continue
        cells += [""] * (6 - len(cells))
        entry, key, verdict, evidence, judge, date = (c.strip() for c in cells[:6])
        if verdict not in VERDICTS:
            continue
        out[(entry, norm_key(key))] = {"verdict": verdict, "evidence": evidence, "judge": judge, "date": date}
    return out


def summarize(items: list, verdicts: dict) -> dict:
    """Per shadow model: findings, judged, real, false, known, precision (None when real+false is 0), gating misses."""
    stats = {}
    for it, v, _ in judge(items, verdicts):
        s = stats.setdefault(it["model"], {"findings": 0, "judged": 0, "real": 0, "false": 0, "known": 0, "misses": 0})
        s["findings"] += 1
        if v:
            s["judged"] += 1
            s[v["verdict"]] += 1
            if v["verdict"] == "real" and it["merged"]:
                s["misses"] += 1
    for s in stats.values():
        d = s["real"] + s["false"]
        s["precision"] = s["real"] / d if d else None
    return stats


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=".", help="the Broadsword root (holds broadsword.toml and state/)")
    ap.add_argument("--open", action="store_true", help="list shadow-only BLOCKING lines with no verdict; exit 3 if any, else 0")
    args = ap.parse_args(argv)
    root = Path(args.root)
    entries = load_entries(root)
    if args.open:
        verdicts = load_verdicts(root)
        open_items = [(it, note) for it, v, note in judge(shadow_only(entries), verdicts) if v is None]
        if not open_items:
            print("no open shadow-only findings")
            return 0
        print("entry\tcommit\tstatus\tfinding_key\tfinding")
        for it, note in open_items:
            status = "merged" if it["merged"] else "not merged"
            print(f"{it['entry']}\t{it['commit']}\t{status}\t{it['key']}\t{it['line']}" + (f"\t[{note}]" if note else ""))
        print(f"\n{len(open_items)} open shadow-only finding(s); record verdicts with scripts/shadow_adjudicate.py add")
        return 3
    if not entries:
        print("no queue entry has a shadow review yet")
        return 0
    print("entry\tstatus\tgating\tshadow\tagree\tshadow_cost_usd\tshadow_secs\tshadow_model_used")
    agree = compared = 0
    details = []
    for eid, r in entries:
        g, s = r.get("verdict", "") or "-", r["shadow_verdict"]
        real = g in ("PASS", "FAIL") and s in ("PASS", "FAIL")
        same = "yes" if real and g == s else "NO" if real else "n/a"
        compared += real
        agree += real and g == s
        print(f"{eid}\t{r.get('status', '')}\t{g}\t{s}\t{same}\t{r.get('shadow_cost_usd') or '-'}\t{r.get('shadow_secs') or '-'}\t{r.get('shadow_model_used') or '-'}")
        gb, sb = findings(r, "blocking"), findings(r, "shadow_blocking")
        only_g = [v for k, v in gb.items() if k not in sb]
        only_s = [v for k, v in sb.items() if k not in gb]
        if only_g or only_s or s in ("ERROR", "SKIPPED"):
            details.append((eid, only_g, only_s, r.get("shadow_reason", "") if s in ("ERROR", "SKIPPED") else ""))
    for eid, only_g, only_s, why in details:
        print(f"\n{eid}")
        if why:
            print(f"  shadow did not review: {why}")
        for l in only_g:
            print(f"  BLOCKING only in the gating review: {l}")
        for l in only_s:
            print(f"  BLOCKING only in the shadow review: {l}")
    rate = f"{agree}/{compared} = {100 * agree / compared:.0f}%" if compared else "n/a (no entry has a real verdict from both)"
    print(f"\nentries with a shadow record: {len(entries)}; compared (both PASS or FAIL): {compared}; agreement rate: {rate}")
    stats = summarize(shadow_only(entries), load_verdicts(root))
    print("\nshadow-only findings by shadow model (known counts in neither precision nor misses)")
    print("model\tfindings\tjudged\treal\tfalse\tknown\tprecision\tgating_misses")
    tot = {k: 0 for k in ("findings", "judged", "real", "false", "known", "misses")}
    for m, s in sorted(stats.items()):
        prec = f"{s['real']}/{s['real'] + s['false']} = {100 * s['precision']:.0f}%" if s["precision"] is not None else "n/a"
        print(f"{m}\t{s['findings']}\t{s['judged']}\t{s['real']}\t{s['false']}\t{s['known']}\t{prec}\t{s['misses']}")
        for k in tot:
            tot[k] += s[k]
    d = tot["real"] + tot["false"]
    prec = f"{tot['real']}/{d} = {100 * tot['real'] / d:.0f}%" if d else "n/a"
    print(f"all\t{tot['findings']}\t{tot['judged']}\t{tot['real']}\t{tot['false']}\t{tot['known']}\t{prec}\t{tot['misses']}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
