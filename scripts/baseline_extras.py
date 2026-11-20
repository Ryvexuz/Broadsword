"""Extras around scripts/baseline.py: queue-entry statistics and corrected baselines. Read-only on state/queue, std only.

  python -I scripts/baseline_extras.py shadow  --since <ISO time> --until <ISO time> [--root <dir>]
  python -I scripts/baseline_extras.py entries --since <ISO time> --until <ISO time> --shadow-off <ISO time> [--root <dir>]
  python -I scripts/baseline_extras.py rounds  --since <ISO time> --until <ISO time> [--root <dir>]
  python -I scripts/baseline_extras.py record-v2 <name> --since <ISO time> --until <ISO time>

shadow     entries created in the window: how many have a shadow verdict, verdict counts, ERROR rate, shadow seconds, ERROR rows.
entries    created -> ended times for all, shadow-on and shadow-off entries, and per task rounds (shadow-off = a time after
           which shadow reviews were switched off).
rounds     tasks with a merged entry that ended in the window: entries, review entries and span per task.
record-v2  like `baseline.py record` (same window rules, never overwrites a baseline) but coder_tokens_per_merged uses the
           definition v2: the coder tokens of MERGED tasks / merged tasks with a coder run. The v1 value also counted the
           tokens of tasks that did not merge in the numerator but divided by merged tasks only.

--root defaults to the main checkout (the one with broadsword.toml and state/). Exit codes: 0 done, 1 bad input, 2 usage.
"""
import sys

sys.dont_write_bytecode = True

import argparse
import datetime as dt
import hashlib
import importlib.util
import json
import os
import pathlib
import statistics
import time
import uuid

HERE = pathlib.Path(__file__).resolve().parent
V2 = "v2"


class InputError(Exception):
    """Bad input: exit 1 with the message."""


def load_baseline_module():
    """Import scripts/baseline.py by path (read-only reuse, no copy of its logic)."""
    path = HERE / "baseline.py"
    if not path.is_file():
        raise InputError(f"{path}: not found (baseline_extras imports it)")
    spec = importlib.util.spec_from_file_location("_extras_baseline", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def read_fields(path):
    """`key = value` lines of an entry file; the first occurrence of a key wins."""
    fields = {}
    try:
        with open(path, encoding="utf-8", errors="replace") as fh:
            for line in fh:
                if " = " in line:
                    key, _, value = line.rstrip("\n").partition(" = ")
                    fields.setdefault(key, value)
    except OSError as e:
        raise InputError(f"{path}: unreadable ({e})") from None
    return fields


def load_entries(root):
    """All queue entries with a numeric `created`: list of (created, file name, fields), oldest first."""
    qdir = pathlib.Path(root) / "state" / "queue"
    if not qdir.is_dir():
        raise InputError(f"{qdir}: no such folder")
    rows = []
    for path in sorted(qdir.glob("*.entry")):
        f = read_fields(path)
        try:
            created = float(f.get("created", ""))
        except ValueError:
            continue
        rows.append((created, path.name, f))
    rows.sort(key=lambda r: (r[0], r[1]))
    return rows


def in_window(rows, since, until):
    return [r for r in rows if since <= r[0] < until]


def _num(f, key):
    try:
        return float(f[key])
    except (KeyError, ValueError):
        return None


def shadow_lines(rows):
    """Statistics of the shadow reviews among entries (already filtered to the window)."""
    out = [f"entries created in window: {len(rows)}"]
    ran = [r for r in rows if r[2].get("shadow_verdict", "") != ""]
    out.append(f"entries with a shadow verdict: {len(ran)}")
    out.append(f"entries with no shadow verdict: {len(rows) - len(ran)}")
    counts = {}
    for _, _, f in ran:
        counts[f["shadow_verdict"]] = counts.get(f["shadow_verdict"], 0) + 1
    out.append(f"verdict counts: {sorted(counts.items())}")
    if ran:
        errs = counts.get("ERROR", 0)
        out.append(f"ERROR rate among shadow runs: {errs} of {len(ran)} = {100.0 * errs / len(ran):.1f}%")
    secs = [s for s in (_num(f, "shadow_secs") for _, _, f in ran) if s is not None]
    if secs:
        out.append(f"shadow_secs: n={len(secs)} median={statistics.median(secs):.1f} min={min(secs):.1f} max={max(secs):.1f}")
    ages = [e - c for c, _, f in rows for e in [_num(f, "ended")] if e is not None]
    if ages:
        out.append(f"entry created->ended (all entries in window): n={len(ages)} median={statistics.median(ages):.0f}s")
    for _, _, f in ran:
        if f["shadow_verdict"] == "ERROR":
            out.append(f"ERROR entry: {f.get('id')} model {f.get('shadow_model_used', '')} secs {f.get('shadow_secs', '')} "
                       f"reason: {(f.get('shadow_reason', '') or '')[:120]}")
    return out


def _age_line(label, sel):
    ages = [e - c for c, _, f in sel for e in [_num(f, "ended")] if e is not None]
    if ages:
        return f"{label}: entries={len(sel)} with_ended={len(ages)} median_s={statistics.median(ages):.1f}"
    return f"{label}: entries={len(sel)} with_ended=0"


def _iso(epoch):
    return dt.datetime.fromtimestamp(epoch, dt.timezone.utc).isoformat()


def entries_lines(rows, shadow_off):
    """Entry times split by shadow on/off and by the shadow-off cutoff, then rounds per task."""
    out = [f"entries created in window: {len(rows)}", _age_line("ALL entries in window", rows)]
    on = [r for r in rows if r[2].get("shadow_verdict", "") != ""]
    off = [r for r in rows if r[2].get("shadow_verdict", "") == ""]
    out.append(_age_line("SHADOW-ON (shadow_verdict present)", on))
    out.append(_age_line("SHADOW-OFF (no shadow_verdict)", off))
    before = [r for r in rows if r[0] < shadow_off]
    after = [r for r in rows if r[0] >= shadow_off]
    out.append(_age_line("created before the cutoff", before))
    out.append(_age_line("created at/after the cutoff", after))
    out.append("created before cutoff but no shadow_verdict: " + str(sum(1 for r in before if r[2].get("shadow_verdict", "") == "")))
    out.append("created after cutoff but with shadow_verdict: " + str(sum(1 for r in after if r[2].get("shadow_verdict", "") != "")))
    counts = {}
    for r in on:
        counts[r[2]["shadow_verdict"]] = counts.get(r[2]["shadow_verdict"], 0) + 1
    out.append(f"shadow_verdict counts (shadow-on): {sorted(counts.items())}")
    out.append(f"statuses in window: {sorted({r[2].get('status', '') for r in rows})}")
    out.append("first and last shadow-on created (UTC): " + (f"{_iso(min(r[0] for r in on))} {_iso(max(r[0] for r in on))}" if on else "- -"))
    out.append("")
    out.append("per task (all entries in window, by created):")
    by_task = {}
    for r in rows:
        by_task.setdefault(r[2].get("task", "?"), []).append(r)
    for task, items in sorted(by_task.items()):
        coders = "/".join(sorted({f.get("coder", "") for _, _, f in items}))
        seq = ",".join(f"{f.get('status', '')}:{f.get('verdict', '') or '-'}" for _, _, f in items)
        out.append(f"{task}\tcoder={coders}\trounds={len(items)}\t{seq}")
    return out


def rounds_lines(all_rows, since, until):
    """Per task that has a merged entry ended in the window: entries, review entries (merged or rejected), span."""
    merged = set()
    for _, _, f in all_rows:
        e = _num(f, "ended")
        if f.get("status") == "merged" and e is not None and since <= e < until:
            merged.add(f.get("task", ""))
    out = [f"tasks with a merged entry ended in window: {len(merged)}"]
    for task in sorted(merged):
        items = [r for r in all_rows if r[2].get("task", "") == task]
        coders = "/".join(sorted({r[2].get("coder", "") for r in items}))
        first = dt.datetime.fromtimestamp(items[0][0], dt.timezone.utc).strftime("%m-%d %H:%M")
        last_end = max(_num(r[2], "ended") or 0 for r in items)
        seq = ",".join(f"{r[2].get('status', '')}:{r[2].get('verdict', '') or '-'}" for r in items)
        reviews = sum(1 for r in items if r[2].get("status", "") in ("merged", "rejected"))
        out.append(f"{task}\tcoder={coders}\tentries={len(items)}\treview_entries={reviews}\tfirst={first}"
                   f"\tspan_min={(last_end - items[0][0]) / 60.0:.0f}\t{seq}")
    return out


def coder_tokens_v2(metrics, samples):
    """coder_tokens_per_merged under v2 from a v1 measurement: merged tasks' coder tokens / merged tasks with a coder run.
    v1 = (all finished tasks' coder tokens) / n, so v1 * n is the numerator and coder_tokens_unmerged is the part to remove."""
    v1 = metrics.get("coder_tokens_per_merged")
    n = samples.get("merged_with_coder_run")
    if v1 is None:
        return None  # no coder runs at all: no data (record_v2 says so in the notes)
    if not n:
        raise InputError("baseline.py measured coder_tokens_per_merged but merged_with_coder_run is 0 or missing; "
                         "v2 cannot be computed (fail closed)")
    unmerged = metrics.get("coder_tokens_unmerged")
    if unmerged is None:
        raise InputError("baseline.py measured coder_tokens_per_merged but not coder_tokens_unmerged; "
                         "v2 cannot be computed without it (fail closed)")
    return (round(v1 * n) - unmerged) / n


def record_v2(root, name, since, until, base=None):
    """Write state/baselines/<name>.json like baseline.py record, with coder_tokens_per_merged at definition v2."""
    base = base or load_baseline_module()
    if not base.NAME_RE.match(name):
        raise InputError(f"baseline name {name!r}: use letters, digits, '.', '_' and '-' only")
    root = pathlib.Path(root)
    out = base.baselines_dir(root) / f"{name}.json"
    if out.exists():
        raise InputError(f"{out}: already exists; a baseline is evidence and is never overwritten, pick a new name")
    toml = root / "broadsword.toml"
    try:
        toml_hash = hashlib.sha256(toml.read_bytes()).hexdigest()
    except OSError as e:
        raise InputError(f"{toml}: unreadable ({e})") from None
    try:
        values, samples, per_model, main_sha, notes = base.measure(root, since, until)
    except base.InputError as e:
        raise InputError(str(e)) from None
    old = values.get("coder_tokens_per_merged")
    values["coder_tokens_per_merged"] = coder_tokens_v2(values, samples)
    base.check_finite(values, per_model)
    definitions = dict(base.DEFINITIONS)
    definitions["coder_tokens_per_merged"] = V2
    if values["coder_tokens_per_merged"] is None:
        notes = list(notes) + [f"coder_tokens_per_merged is definition {V2} but there is no data in this window "
                               "(no coder runs): the value is null, not a measurement"]
    else:
        notes = list(notes) + [f"coder_tokens_per_merged is definition {V2}: merged tasks' coder tokens / merged tasks with a coder run "
                               f"(the v1 measurement was {old})"]
    data = {
        "schema": base.SCHEMA, "name": name, "recorded_at": base.iso(int(time.time())),
        "window": {"since": base.iso(since), "until": base.iso(until), "hours": round((until - since) / 3600, 3)},
        "main_commit": main_sha, "broadsword_toml_sha256": toml_hash,
        "definitions": definitions, "metrics": values, "samples": samples, "per_model": per_model, "notes": notes,
    }
    out.parent.mkdir(parents=True, exist_ok=True)
    # Same atomic never-overwrite as baseline.py: unique temp file, then os.link (fails if the name exists).
    tmp = out.with_name(f".{out.name}.{os.getpid()}.{uuid.uuid4().hex}.tmp")
    try:
        tmp.write_text(json.dumps(data, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        try:
            os.link(tmp, out)
        except FileExistsError:
            raise InputError(f"{out}: already exists; a baseline is evidence and is never overwritten, pick a new name") from None
        except OSError as e:
            raise InputError(f"{out}: cannot create ({e})") from None
    finally:
        try:
            tmp.unlink()
        except OSError:
            pass
    return out, data


def parser():
    p = argparse.ArgumentParser(prog="baseline_extras.py", description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    for name in ("shadow", "entries", "rounds"):
        s = sub.add_parser(name)
        s.add_argument("--since", required=True)
        s.add_argument("--until", required=True)
        s.add_argument("--root")
        if name == "entries":
            s.add_argument("--shadow-off", required=True)
    r = sub.add_parser("record-v2")
    r.add_argument("name")
    r.add_argument("--since", required=True)
    r.add_argument("--until", required=True)
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    try:
        base = load_baseline_module()
    except InputError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    # baseline.py has its own InputError class: catch it through the module object that was loaded, not a name lookup.
    try:
        since, until = base.parse_time(args.since, "--since"), base.parse_time(args.until, "--until")
        if until <= since:
            raise InputError("--until is not after --since")
        if args.cmd == "record-v2":
            out, data = record_v2(base.main_root(), args.name, since, until, base)
            print(f"baseline {args.name}: {data['window']['since']} to {data['window']['until']}")
            print(f"coder_tokens_per_merged (v2) = {data['metrics']['coder_tokens_per_merged']}")
            print(f"written: {out}")
            return 0
        root = pathlib.Path(args.root) if args.root else base.main_root()
        rows = load_entries(root)
        if args.cmd == "shadow":
            lines = shadow_lines(in_window(rows, since, until))
        elif args.cmd == "entries":
            lines = entries_lines(in_window(rows, since, until), base.parse_time(args.shadow_off, "--shadow-off"))
        else:
            lines = rounds_lines(rows, since, until)
        print("\n".join(lines))
        return 0
    except (InputError, base.InputError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
