"""Lint a task brief against the measured first-pass rules. Read-only.

Usage:
    python -I scripts/brief_lint.py --max-lines N --max-bullets N --max-files N <brief.md> [...]

The three limits are required flags (nothing is hard-coded here; the settings rule). The
measured values (state/staging/selfcheck/NOTES.md) are 30, 8 and 2.

Rules, each printed as `PASS` or `FAIL` per brief:
  lines     non-blank lines, not counting the done-check block, at most --max-lines
  bullets   top-level bullets in the Requirements section, at most --max-bullets
  files     distinct backtick spans on the Claims line, at most --max-files unless the Claims
            line states a reason (the word "reason" or "because")
  donecheck a Done check section exists, has at least one item, and every item holds a
            backtick span that is a command (first word: broadsword, python, cargo, git, ...)

A brief with no Claims line or no Done check section fails (fail closed).
Exit codes: 0 every brief passes, 1 some rule failed, 2 usage error or unreadable file.
"""
import argparse
import re
import sys

COMMANDS = {"broadsword", "python", "python3", "cargo", "git", "pytest", "npm", "bash", "sh",
            "powershell", "pwsh"}
# Section names a brief uses; a line starting with one (after "- " and bold markers) starts a section.
SECTIONS = ("requirements", "done check", "claim", "claims", "task", "board", "build route",
            "depends on", "model", "title", "source", "review", "order")
SECTION_RE = re.compile(r"^(?:[-*]\s+)?\**\s*([A-Za-z][A-Za-z ]*?)\s*:\s*\**\s*(.*)$")
ITEM_RE = re.compile(r"^\s*(?:\d+[.)]|[-*])\s")
TICKS_RE = re.compile(r"`([^`]+)`")


class LintError(Exception):
    pass


def section_of(line):
    """(name, rest) when the line opens a known section, else None."""
    if line.startswith(" ") or line.startswith("\t"):
        return None
    m = SECTION_RE.match(line.replace("**", ""))
    if m and m.group(1).strip().lower() in SECTIONS:
        return m.group(1).strip().lower(), m.group(2)
    return None


def split_sections(lines):
    """List of (name, start, end_exclusive) for each section; line indexes into `lines`."""
    starts = []
    for i, ln in enumerate(lines):
        s = section_of(ln)
        if s:
            starts.append((s[0], i))
    out = []
    for k, (name, i) in enumerate(starts):
        end = starts[k + 1][1] if k + 1 < len(starts) else len(lines)
        out.append((name, i, end))
    return out


def is_command(span):
    words = span.strip().split()
    if not words:
        return False
    first = re.split(r"[\\/]", words[0])[-1].lower()
    if first.endswith(".exe"):
        first = first[:-4]
    return first in COMMANDS


def lint(text, max_lines, max_bullets, max_files):
    """Return a list of (rule, ok, message) for the four rules."""
    lines = text.splitlines()
    secs = split_sections(lines)
    results = []

    done = [s for s in secs if s[0] == "done check"]
    done_idx = set()
    for _, a, b in done:
        done_idx.update(range(a, b))

    n = sum(1 for i, ln in enumerate(lines) if ln.strip() and i not in done_idx)
    results.append(("lines", n <= max_lines,
                    f"{n} non-blank lines outside the done check (max {max_lines})"))

    bullets = 0
    for name, a, b in secs:
        if name == "requirements":
            for ln in lines[a + 1:b]:
                if ITEM_RE.match(ln) and not ln.startswith((" ", "\t")):
                    bullets += 1
    results.append(("bullets", bullets <= max_bullets,
                    f"{bullets} requirement bullets (max {max_bullets})"))

    claims = [s for s in secs if s[0] in ("claim", "claims")]
    if not claims:
        results.append(("files", False, "no Claims line found"))
    else:
        _, a, b = claims[0]
        block = [lines[a].replace("**", "")] + [ln for ln in lines[a + 1:b]
                                                if ln.startswith((" ", "\t")) and ln.strip()]
        first = re.sub(r"^(?:[-*]\s+)?\s*[A-Za-z ]+:\s*", "", block[0], count=1)
        block = [first] + block[1:]
        joined = "\n".join(block)
        files = set(TICKS_RE.findall(joined))
        prose = TICKS_RE.sub("", joined).lower()
        has_reason = "reason" in prose or "because" in prose
        ok = len(files) <= max_files or has_reason
        msg = f"{len(files)} claimed files (max {max_files} unless a reason is stated)"
        if len(files) > max_files:
            msg += "; reason stated" if has_reason else "; no reason stated"
        results.append(("files", ok, msg))

    if not done:
        results.append(("donecheck", False, "no Done check section found"))
    else:
        _, a, b = done[0]
        head = re.sub(r"^(?:[-*]\s+)?\s*[A-Za-z ]+:\s*", "", lines[a].replace("**", ""), count=1)
        items = []
        if head.strip():
            items.append(head)
        for ln in lines[a + 1:b]:
            if not ln.strip():
                continue
            if ITEM_RE.match(ln) and not ln.startswith((" ", "\t")):
                items.append(ln)
            elif items:
                items[-1] += " " + ln.strip()
        bad = [it for it in items if not any(is_command(s) for s in TICKS_RE.findall(it))]
        if not items:
            results.append(("donecheck", False, "Done check has no items"))
        elif bad:
            results.append(("donecheck", False,
                            f"{len(bad)} of {len(items)} done-check items hold no command: "
                            + "; ".join(b_[:60] for b_ in bad)))
        else:
            results.append(("donecheck", True, f"{len(items)} done-check items, all commands"))
    return results


def positive(value):
    n = int(value)
    if n < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return n


def main(argv):
    p = argparse.ArgumentParser(prog="brief_lint.py")
    p.add_argument("--max-lines", type=positive, required=True)
    p.add_argument("--max-bullets", type=positive, required=True)
    p.add_argument("--max-files", type=positive, required=True)
    p.add_argument("briefs", nargs="+")
    try:
        a = p.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    failed = 0
    for path in a.briefs:
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except (OSError, UnicodeDecodeError) as e:
            print(f"brief_lint: cannot read {path}: {e}", file=sys.stderr)
            return 2
        res = lint(text, a.max_lines, a.max_bullets, a.max_files)
        bad = [r for r in res if not r[1]]
        print(f"{path}: {'ok' if not bad else str(len(bad)) + ' rule(s) failed'}")
        for rule, ok, msg in res:
            print(f"  {'PASS' if ok else 'FAIL'} {rule}: {msg}")
        failed += bool(bad)
    print("brief_lint: ok" if not failed else f"brief_lint: {failed} brief(s) failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
