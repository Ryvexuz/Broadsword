"""Lint a task brief against the measured first-pass rules. Read-only.

Usage:
    python -I scripts/brief_lint.py --max-lines N --max-bullets N --max-files N --max-done-lines N <brief.md> [...]

The four limits are required flags (nothing is hard-coded here; the settings rule). The
measured values (state/staging/selfcheck/NOTES.md) are 30, 8 and 2; the done-check cap is 10.

Rules, each printed as `PASS` or `FAIL` per brief:
  lines     non-blank lines at most --max-lines; the first --max-done-lines lines of the done check
            are not counted, any beyond that are
  bullets   top-level bullets anywhere outside the Done check (before the first section too; in
            Claims only bullets that name no path), at most --max-bullets; a brief with no
            Requirements section (any heading form) is a failure
  files     distinct path-like tokens on any line outside the Done check (backticked or not), at
            most --max-files unless the Claims text states a reason (the word "reason" or "because")
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
# Names the lint reads. A section starts at a heading line ("## X"), a bold line ("**X:**",
# "**X**") or, for these key names only, a plain or bullet line ("Claims: ...", "- Done check: ...").
# Only a heading, a bold line or a key-name line ends a section: "- Order: lint first" does not.
KEY_NAMES = {"requirements": "requirements", "requirement": "requirements", "claim": "claims",
             "claims": "claims", "done check": "done check"}
KEY_RE = re.compile(r"^(?:[-*]\s+)?([A-Za-z][A-Za-z -]*?)\s*:\s*(.*)$")
HEADING_RE = re.compile(r"^#{1,6}\s+(.*)$")
BOLD_RE = re.compile(r"^\*\*\s*([^*]+?)\s*:?\s*\*\*\s*:?\s*(.*)$")
ITEM_RE = re.compile(r"^\s*(?:\d+[.)]|[-*])\s")
TICKS_RE = re.compile(r"`([^`]+)`")
PATH_RE = re.compile(r"^(?:[\w.-]+[/\\])+[\w.-]*$|^[\w-][\w.-]*\.[A-Za-z][A-Za-z0-9]{1,5}$")


class LintError(Exception):
    pass


def norm(name):
    return re.sub(r"[\s-]+", " ", name.replace("*", "").strip().rstrip(":").strip()).lower()


def section_of(line):
    """(name, rest) when the line opens a section, else None. Indented lines never do."""
    if line.startswith(" ") or line.startswith("\t") or not line.strip():
        return None
    m = HEADING_RE.match(line)
    if m:
        text = m.group(1)
        name, _, rest = text.partition(":")
        return norm(name), rest.strip()
    m = BOLD_RE.match(line.rstrip())
    if m:
        return norm(m.group(1)), m.group(2).strip()
    m = KEY_RE.match(line.replace("**", ""))
    if m and norm(m.group(1)) in KEY_NAMES:
        return norm(m.group(1)), m.group(2).strip()
    return None


def split_sections(lines):
    """List of (name, start, end_exclusive) for each section; line indexes into `lines`."""
    starts = []
    for i, ln in enumerate(lines):
        s = section_of(ln)
        if s:
            starts.append((KEY_NAMES.get(s[0], s[0]), i))
    out = []
    for k, (name, i) in enumerate(starts):
        end = starts[k + 1][1] if k + 1 < len(starts) else len(lines)
        out.append((name, i, end))
    return out


def claim_paths(text):
    """Distinct path-like tokens in `text` (backticks or not), each path of a span on its own."""
    paths = set()
    for tok in re.split(r"[\s,;()\[\]`'\"]+", text):
        tok = tok.strip().rstrip(".:")
        if tok and PATH_RE.match(tok):
            paths.add(tok.replace("\\", "/"))
    return paths


def is_command(span):
    words = span.strip().split()
    if not words:
        return False
    first = re.split(r"[\\/]", words[0])[-1].lower()
    if first.endswith(".exe"):
        first = first[:-4]
    return first in COMMANDS


def lint(text, max_lines, max_bullets, max_files, max_done_lines):
    """Return a list of (rule, ok, message) for the four rules."""
    lines = text.splitlines()
    secs = split_sections(lines)
    results = []

    done = [s for s in secs if s[0] == "done check"]
    done_n = sum(1 for _, a, b in done for ln in lines[a:b] if ln.strip())
    total = sum(1 for ln in lines if ln.strip())
    n = total - min(done_n, max_done_lines)
    msg = f"{n} non-blank lines (max {max_lines}); done check {done_n} lines, first {max_done_lines} excluded"
    results.append(("lines", n <= max_lines, msg))

    # Bullets in every section except Done check and Claims count (an unlisted section such as
    # "Also" or "Notes" cannot hold bullets the limit never sees). A Requirements heading line that
    # carries text counts as one bullet.
    reqs = [s for s in secs if s[0] == "requirements"]
    if not reqs:
        results.append(("bullets", False, "no Requirements section found (accepted forms: "
                        "'Requirements:', '## Requirements', '**Requirements**')"))
    else:
        bullets = 0
        first = secs[0][1] if secs else len(lines)
        for ln in lines[:first]:
            if ITEM_RE.match(ln) and not ln.startswith((" ", "	")):
                bullets += 1
        for name, a, b in secs:
            if name == "done check":
                continue
            if name == "requirements" and section_of(lines[a])[1]:
                bullets += 1
            for ln in lines[a + 1:b]:
                if ITEM_RE.match(ln) and not ln.startswith((" ", "	")):
                    # In Claims a bullet that names a path is a claimed file, not a requirement.
                    if name == "claims" and claim_paths(ln):
                        continue
                    bullets += 1
        results.append(("bullets", bullets <= max_bullets,
                        f"{bullets} requirement bullets (max {max_bullets}; all sections but "
                        "Claims and Done check count)"))

    claims = [s for s in secs if s[0] == "claims"]
    if not claims:
        results.append(("files", False, "no Claims line found"))
    else:
        _, a, b = claims[0]
        rest = section_of(lines[a])[1]
        if rest:
            block = [rest] + [ln for ln in lines[a + 1:b] if ln.startswith((" ", "\t")) and ln.strip()]
        else:
            block = [ln for ln in lines[a + 1:b] if ln.strip()]
        joined = "\n".join(block)
        # Paths on any line outside the Done check count, so a bold line after Claims hides none.
        in_done = {i for _, x, y in done for i in range(x, y)}
        files = claim_paths(joined) | claim_paths(chr(10).join(
            ln for i, ln in enumerate(lines) if i not in in_done))
        prose = " ".join(t for t in re.split(r"[\s,;()\[\]`'\"]+", joined)
                         if t.strip().rstrip(".:").replace("\\", "/") not in files).lower()
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
        head = section_of(lines[a])[1]
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
    p.add_argument("--max-done-lines", type=positive, required=True)
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
        res = lint(text, a.max_lines, a.max_bullets, a.max_files, a.max_done_lines)
        bad = [r for r in res if not r[1]]
        print(f"{path}: {'ok' if not bad else str(len(bad)) + ' rule(s) failed'}")
        for rule, ok, msg in res:
            print(f"  {'PASS' if ok else 'FAIL'} {rule}: {msg}")
        failed += bool(bad)
    print("brief_lint: ok" if not failed else f"brief_lint: {failed} brief(s) failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
