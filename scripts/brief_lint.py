"""Lint a task brief against the measured first-pass rules. Read-only.

Usage:
    python -I scripts/brief_lint.py --max-lines N --max-bullets N --max-files N --max-done-lines N [--root <repo>] <brief.md> [...]

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

A brief with more than one Done check section fails the donecheck rule ("more than one Done
check section").

Claim-gap rules (printed after the four rules as `FAIL claimgap-Rn` or `WARN claimgap-Rn`; a brief
with no FAIL prints `PASS claimgap`). They compare what Requirements and the Done check name with
the Claims (a claim covers a path when it equals it, is a folder prefix of it, or ends with it):
  R1  FAIL  a repo path (has a file extension or ends in "/") named in Requirements or the Done
            check and not covered by a claim. A path after "Broadsword's" or in a clause whose
            subject is another task ("n20-47 copies ... updates `ROADMAP.md`") is skipped.
  R1c FAIL  a path after the word "Claim" in any other section (an orchestrator note such as
            "Claim src/a.rs and lift it") that Claims does not cover.
  R1b WARN  "bench example" or "example binary/program" in Requirements, nothing claimed under
            examples/.
  R2  FAIL  `pub mod X;` or a backticked `X` in a "submodule" sentence of Requirements, and no
            claimed X.rs or X/mod.rs stub.
  R3        a `--flag` in Requirements of a brief that claims crates/broadsword/ while the parser
            crates/broadsword/src/tools.rs is not claimed: FAIL when the sentence says the flag is
            new (new, add, introduce, accept), else WARN.
  R4  FAIL  "new required setting/key" in Requirements without config.rs, broadsword.toml and
            tests/support/mod.rs claimed.
  R5  WARN  only with --root: each backticked function or symbol (snake_case, `::` or `()`) of
            Requirements or the Done check, and the files `git grep` finds it in that are not claimed.

Threat model: this lint is ADVISORY. Its inputs are our own agents' briefs, not an adversary's.
It catches honest mistakes and drift; making it bypass-proof is NOT a goal, and a deliberately
hand-crafted brief that defeats it is a known limit, not a defect.

A brief with no Claims line or no Done check section fails (fail closed).
Exit codes: 0 every brief passes, 1 some rule failed, 2 usage error or unreadable file.
"""
import argparse
import re
import subprocess
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
    elif len(done) > 1:
        results.append(("donecheck", False, "more than one Done check section"))
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


BARE_EXTS = {"rs", "toml", "py"}
# A missing claim is a FAIL for code and config paths; docs, data and output folders only WARN
# (they are often read, or written outside the task: scratch, evidence, state).
CODE_PATH_RE = re.compile(r"^(?:(?:src|tests|examples|scripts|crates|templates)/.*|[^/]*|.*\.(?:rs|toml|py))$")
PATH_EXT_RE =re.compile(r"(?:/|\.[A-Za-z0-9]{1,6})$")
OTHER_TASK_RE = re.compile(r"\bn\d+-\d+[a-z]?\s+(?:copies|updates|edits|creates|writes|owns)\b")
OTHER_REPO_RE = re.compile(r"Broadsword's\s+`?[^\s`]+`?")
FLAG_RE = re.compile(r"(?<![\w-])--[a-z][a-z0-9]*(?:-[a-z0-9]+)*")
NEW_WORD_RE = re.compile(r"\b(?:new|add|adds|adding|added|introduce|introduces|accept|accepts)\b", re.I)
CONFIG_KEY_RE = re.compile(r"\bnew\s+required\s+(?:config(?:uration)?\s+)?(?:setting|key)s?\b", re.I)
SYMBOL_RE = re.compile(r"^[A-Za-z_]\w*(?:::\w+)*(?:\(\))?$")
SENT_RE = re.compile(r"(?<=[.!?])\s+|;\s+|\n")
TOOLS_RS = "crates/broadsword/src/tools.rs"
CONFIG_NEEDS = ("config.rs", "broadsword.toml", "tests/support/mod.rs")


def section_text(lines, secs, names):
    """Text of every section whose name is in `names` (the heading line's own text included)."""
    out = []
    for name, a, b in secs:
        if name in names:
            out.append(section_of(lines[a])[1])
            out.extend(lines[a + 1:b])
    return "\n".join(out)


def claimed_paths(lines, secs):
    """Paths in the Claims section, plus those of an inline 'Claims: ...' on any line."""
    found = set()
    for name, a, b in secs:
        if name == "claims":
            rest = section_of(lines[a])[1]
            if rest:
                block = [rest] + [ln for ln in lines[a + 1:b] if ln.startswith((" ", "\t")) and ln.strip()]
            else:
                block = [ln for ln in lines[a + 1:b] if ln.strip()]
            found |= claim_paths("\n".join(block))
    for ln in lines:
        m = re.search(r"\bClaims?:\s*(.*)", ln)
        if m:
            found |= claim_paths(re.split(r"\.\s+[A-Z]", m.group(1))[0])
    return found


def covers(claims, tok):
    t = tok.rstrip("/")
    for c in claims:
        c2 = c.rstrip("/")
        if c2 == t or t.startswith(c2 + "/") or c2.endswith("/" + t) or (tok.endswith("/") and c2.startswith(t + "/")):
            return True
    return False


def named_paths(text):
    """Repo paths named in `text`, skipping other-task clauses and other repos' paths."""
    out = []
    for sent in SENT_RE.split(text):
        sent = OTHER_REPO_RE.sub("", sent)
        m = OTHER_TASK_RE.search(sent)
        if m:
            sent = sent[:m.start()]
        for p in sorted(claim_paths(sent)):
            # A bare name (no folder) counts only for source and config files: "AGENTS.md" and
            # "queue.lock" in prose are references, not edits.
            bare_ok = "/" in p or p.rsplit(".", 1)[-1] in BARE_EXTS
            if PATH_EXT_RE.search(p) and bare_ok and "<" not in p and p not in out:
                out.append(p)
    return out


def claim_gaps(text, root=None):
    """Return [(rule, level, message)] for R1-R5; level is FAIL or WARN. Empty when clean."""
    lines = text.splitlines()
    secs = split_sections(lines)
    claims = claimed_paths(lines, secs)
    req = section_text(lines, secs, {"requirements"})
    dn = section_text(lines, secs, {"done check"})
    res = []

    # A command in the Done check reads and runs paths; it does not edit them.
    dn = TICKS_RE.sub(lambda m: "" if is_command(m.group(1)) else m.group(0), dn)
    miss = [p for p in named_paths(req + "\n" + dn) if not covers(claims, p)]
    code = [p for p in miss if CODE_PATH_RE.match(p)]
    if code:
        res.append(("R1", "FAIL", "named in Requirements or Done check but not claimed: " + ", ".join(code)))
    other_miss = [p for p in miss if p not in code]
    if other_miss:
        res.append(("R1", "WARN", "docs, data or output paths named but not claimed (read-only, or a gap?): "
                    + ", ".join(other_miss)))

    other = "\n".join(section_text(lines, secs, {n}) for n in {s[0] for s in secs}
                      - {"requirements", "done check", "claims"})
    miss = []
    for sent in SENT_RE.split(other):
        m = re.search(r"\bclaim\s+(.*)", sent, re.I)
        if m:
            for p in named_paths(m.group(1)):
                if not covers(claims, p) and p not in miss:
                    miss.append(p)
    if miss:
        res.append(("R1c", "FAIL", "told to claim in a note but not in Claims: " + ", ".join(miss)))

    if re.search(r"\b(?:bench(?:mark)?|speedup|demo)\s+example\b|\bexample\s+(?:binary|program)\b", req, re.I) \
            and not any(c.startswith("examples/") or "/examples/" in c for c in claims):
        res.append(("R1b", "WARN", "Requirements name an example but nothing under examples/ is claimed"))

    names = set(re.findall(r"\bpub\s+mod\s+(\w+)\s*;", req))
    for sent in SENT_RE.split(req):
        if re.search(r"\bsubmodules?\b", sent, re.I):
            names |= {t for t in TICKS_RE.findall(sent) if re.fullmatch(r"\w+", t)}
    miss = sorted(n for n in names
                  if not any(re.search(rf"(?:^|/){re.escape(n)}(?:/mod)?\.\w+$", c) for c in claims))
    if miss:
        res.append(("R2", "FAIL", "module declared but no claimed stub file (X.rs or X/mod.rs): "
                    + ", ".join(miss)))

    if any(c.startswith("crates/broadsword/") for c in claims) and not covers(claims, TOOLS_RS):
        fresh, old = [], []
        for sent in SENT_RE.split(req):
            for f in FLAG_RE.findall(sent):
                (fresh if NEW_WORD_RE.search(sent) else old).append(f)
        if fresh:
            res.append(("R3", "FAIL", f"new flag(s) {', '.join(sorted(set(fresh)))} but {TOOLS_RS} "
                        "(the parser) is not claimed"))
        elif old:
            res.append(("R3", "WARN", f"flag(s) {', '.join(sorted(set(old)))} named in Requirements; "
                        f"{TOOLS_RS} (the parser) is not claimed: does a parser or call site there change?"))

    if CONFIG_KEY_RE.search(req):
        miss = [n for n in CONFIG_NEEDS if not covers(claims, n)]
        if miss:
            res.append(("R4", "FAIL", "a new required config key needs these claimed too: " + ", ".join(miss)))

    if root:
        syms = []
        for t in TICKS_RE.findall(req + "\n" + dn):
            t = t.strip()
            if SYMBOL_RE.match(t) and ("_" in t or "::" in t or t.endswith("()")) and not is_command(t):
                s = t[:-2] if t.endswith("()") else t
                s = s.split("::")[-1]
                if s not in syms:
                    syms.append(s)
        for s in syms:
            try:
                r = subprocess.run(["git", "-C", root, "grep", "-l", "-w", "-F", "--", s],
                                   capture_output=True, text=True, encoding="utf-8")
            except OSError as e:
                res.append(("R5", "WARN", f"git grep for `{s}` could not run: {e}"))
                continue
            if r.returncode > 1:
                res.append(("R5", "WARN", f"git grep for `{s}` failed: {r.stderr.strip()[:80]}"))
                continue
            files = [f for f in r.stdout.split() if not covers(claims, f)]
            if files:
                res.append(("R5", "WARN", f"`{s}` is defined or used in unclaimed: " + ", ".join(files)))
    return res


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
    p.add_argument("--root", default=None, help="repo to search for R5 (git grep); omit to skip R5")
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
        gaps = claim_gaps(text, a.root)
        bad = [r for r in res if not r[1]] + [g for g in gaps if g[1] == "FAIL"]
        print(f"{path}: {'ok' if not bad else str(len(bad)) + ' rule(s) failed'}")
        for rule, ok, msg in res:
            print(f"  {'PASS' if ok else 'FAIL'} {rule}: {msg}")
        for rule, level, msg in gaps:
            print(f"  {level} claimgap-{rule}: {msg}")
        if not any(g[1] == "FAIL" for g in gaps):
            print("  PASS claimgap")
        failed += bool(bad)
    print("brief_lint: ok" if not failed else f"brief_lint: {failed} brief(s) failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
