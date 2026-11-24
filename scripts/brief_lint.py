"""Lint a task brief against the measured first-pass rules. Read-only.

Usage:
    python -I scripts/brief_lint.py --max-lines N --max-bullets N --max-files N --max-done-lines N [--root <repo>] <brief.md> [...]

The four limits are required flags (nothing is hard-coded here; the settings rule). The
measured values (state/staging/selfcheck/NOTES.md) are 30, 8 and 2; the done-check cap is 10.
Optional claim-gap limits: --min-hard-cases N, --big-file-lines N, --min-pasted-lines N.
Each enables only its own rule; omitted limits leave R9-R11 off.

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
  R6  FAIL  hot whole-file claims need a Hot file: reason. With --root, flagged hotspot
            report entries also accept Split first: <task id>; original HOT rules take precedence.
  R7  FAIL  whole src/ or tests/ claims, at the root or under crates/<name>/.
  R8  WARN  with --root, claimed files over broadsword.toml's hotspot_lines; unreadable
            settings or files are reported as warnings.
  R9  FAIL  with --min-hard-cases, Risk: high in Board needs that many Hard cases list items,
            each naming a backticked snake_case test also named in the Done check.
  R10 FAIL  with --root and --big-file-lines, an existing claimed file over that newline
            count needs "wiring" in its own Claims bullet or a claimed (new) file in
            the same folder or a subfolder of that folder.
            New files are not counted; outside-root or unreadable paths WARN.
  R11 FAIL  with --min-pasted-lines, a GPT Model in Board naming an outside-repo path
            needs a Pasted or Pasted protocol section with that many non-empty body lines.

Plan preflight (only with --root <repo> --preflight-ref <main branch>; `--queue-list <file>` holds
`broadsword queue list` output). Each rule prints `FAIL preflight-Pn` with the missing list; a brief with
none prints `PASS preflight`. A path counts as present when it is in the ref's tree, or is claimed
"(new)" (a file under a claimed new folder too). Paths under scratch/ state/ build/ worktrees/ projects/,
placeholders with `<` or `*`, and sentences saying "separate task" are not checked.
  P0  the tree of the ref cannot be read (bad ref or not a repo): fail closed
  P1  a path named anywhere in the brief (Done check commands included) missing from the ref
  P2  a .md doc the brief names (and does not claim) is read one level deep: its data/code paths
      (bsn rs py csv meta toml input json) missing from the ref, unless the brief claims them "(new)"
  P3  a test named as `test NAME ...` or `fn NAME` (in the brief or such a doc) found nowhere on the ref
      (git grep -w), unless the brief claims a new .rs or test file
  P4  a --flag in Requirements or the Done check (commands of broadsword, cargo and git excepted) found
      nowhere on the ref, unless its sentence says new/add/introduce/accept
  P5  a task id after "after", "needs", "requires", "depends on" or "blocked by" that has no line with
      status `merged` in the queue list; with no --queue-list at all it fails closed
  P6  a brief that claims a records file (under docs/ or evidence/, or PROGRESS.md, LAB_LOG.md, RETRO.md)
      must contain the line "Every number and claim is quoted from a file or command output."

Threat model: this lint is ADVISORY. Its inputs are our own agents' briefs, not an adversary's.
It catches honest mistakes and drift; making it bypass-proof is NOT a goal, and a deliberately
hand-crafted brief that defeats it is a known limit, not a defect.

A brief with no Claims line or no Done check section fails (fail closed).
Exit codes: 0 every brief passes, 1 some rule failed, 2 usage error or unreadable file.
"""
import argparse
import csv
import io
import os
import pathlib
import re
import subprocess
import sys
import tomllib

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
        tok = re.sub(r"[/\\]{2,}", "/", tok.strip().rstrip(".:"))
        if tok and PATH_RE.match(tok):
            paths.add(tok.replace("\\", "/"))
    # A bare src/tests is a claim only as a whole backticked span or as the first word of a
    # comma/semicolon/line item; elsewhere ("plus the tests") it is prose.
    bare = {"src", "tests"}
    for span in re.findall(r"`([^`]*)`", text):
        if span.strip() in bare:
            paths.add(span.strip())
    for item in re.split(r"[,;\n]", text):
        item = re.sub(r"^[\s*_>#-]*(?:claims?\s*:[\s*_]*)?", "", item, flags=re.I)
        words = item.split()
        if words and words[0].strip("`.:") in bare:
            paths.add(words[0].strip("`.:"))
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
NEGATED_RE = re.compile(r"\b(?:no|not|never|without)\b|n't\b", re.I)
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


def claim_blocks(lines, secs):
    """Raw text of every Claims block: the Claims section, plus each inline 'Claims: ...' line."""
    blocks = []
    for name, a, b in secs:
        if name == "claims":
            rest = section_of(lines[a])[1]
            if rest:
                block = [rest] + [ln for ln in lines[a + 1:b] if ln.startswith((" ", "	")) and ln.strip()]
            else:
                block = [ln for ln in lines[a + 1:b] if ln.strip()]
            blocks.append("\n".join(block))
    for ln in lines:
        m = re.search(r"Claims?:\s*(.*)", ln)
        if m:
            blocks.append(re.split(r"\.\s+[A-Z]", m.group(1))[0])
    return blocks


def claimed_paths(lines, secs):
    """Paths in the Claims section, plus those of an inline 'Claims: ...' on any line."""
    found = set()
    for block in claim_blocks(lines, secs):
        found |= claim_paths(block)
    return found


# Hot shared files (task t-20261122-050730: one task holding these blocked 31 to 42 of 42 open
# tasks). Value: True when a claim with a #key suffix is NOT hot, False when the file is hot even then.
HOT = {
    "crates/broadsword/src/queue.rs": False,
    "crates/broadsword/tests/queue.rs": False,
    "broadsword.toml": True,
    "crates/broadsword/src/config.rs": True,
    "crates/broadsword/tests/support/mod.rs": True,
}
HOT_REASON_RE = re.compile(r"^\s*(?:[-*]\s+)?Hot file:\s*(.*\S)\s*$")
HOT_REASON_MIN = 20


def report_hot(root):
    """Read the report's flagged paths; absence is advisory, never a lint failure."""
    try:
        with open(pathlib.Path(root) / "state" / "hotspots.tsv", encoding="utf-8", newline="") as f:
            return {row["path"].replace("\\", "/") for row in csv.DictReader(f, delimiter="\t")
                    if row.get("flag") in {"split", "hot"} and row.get("path")}
    except (OSError, UnicodeError, KeyError, csv.Error):
        print("hotspots: no report")
        return set()


def hot_claims(lines, secs, reported=()):
    """Hot files claimed as a whole file, sorted."""
    hot = set()
    for block in claim_blocks(lines, secs):
        for tok in re.split(r"[\s,;()\[\]`'\"]+", block):
            tok = tok.strip().rstrip(".:").replace("\\", "/")
            base, sep, _ = tok.partition("#")
            if (base in HOT and not (sep and HOT[base])) or (
                    base not in HOT and base in reported):
                hot.add(base)
    return sorted(hot)


def covers(claims, tok):
    t = tok.rstrip("/")
    for c in claims:
        c2 = c.rstrip("/")
        # Equal, or under a claimed folder, or (a bare file name only) the name of a claimed file.
        # A folder named in a brief is never covered by a file inside it, and a path with a folder
        # is never covered by a different path that merely ends the same way.
        if c2 == t or t.startswith(c2 + "/") or ("/" not in t and c2.endswith("/" + t)):
            return True
    return False


def named_paths(text):
    """Repo paths named in `text`, skipping other-task clauses and other repos' paths."""
    out = []
    for sent in SENT_RE.split(text):
        sent = OTHER_REPO_RE.sub("", sent)
        m = OTHER_TASK_RE.search(sent)
        if m:
            before = sent[:m.start()]
            close = sent.find(")", m.start())
            if before.rfind("(") > before.rfind(")") and close != -1:
                # The marker sits inside parentheses: skip only up to the closing one.
                sent = before + sent[close + 1:]
            else:
                sent = before
        for p in sorted(claim_paths(sent)):
            # A bare name (no folder) counts only for source and config files: "AGENTS.md" and
            # "queue.lock" in prose are references, not edits.
            bare_ok = "/" in p or p.rsplit(".", 1)[-1] in BARE_EXTS
            if PATH_EXT_RE.search(p) and bare_ok and "<" not in p and p not in out:
                out.append(p)
    return out


def claim_gaps(text, root=None, reported=None, *, min_hard_cases=None, big_file_lines=None,
               min_pasted_lines=None):
    """Return [(rule, level, message)] for R1-R11; level is FAIL or WARN. Empty when clean."""
    for name, value in (("min_hard_cases", min_hard_cases), ("big_file_lines", big_file_lines),
                        ("min_pasted_lines", min_pasted_lines)):
        if value is not None and (type(value) is not int or value < 1):
            raise ValueError(f"{name} must be a positive integer")
    lines = text.splitlines()
    secs = split_sections(lines)
    claims = claimed_paths(lines, secs)
    req = section_text(lines, secs, {"requirements"})
    dn = section_text(lines, secs, {"done check"})
    done_text = dn
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
        if re.search(r"\bsubmodules?\b", sent, re.I) and not NEGATED_RE.search(sent):
            names |= {t for t in TICKS_RE.findall(sent) if re.fullmatch(r"\w+", t)}
    miss = sorted(n for n in names
                  if not any(re.search(rf"(?:^|/){re.escape(n)}(?:/mod)?\.\w+$", c) for c in claims))
    if miss:
        res.append(("R2", "FAIL", "module declared but no claimed stub file (X.rs or X/mod.rs): "
                    + ", ".join(miss)))

    if any(c.startswith("crates/broadsword/") for c in claims) and not covers(claims, TOOLS_RS):
        flags = []
        word_new = set()
        for sent in SENT_RE.split(req):
            for f in FLAG_RE.findall(sent):
                if f not in flags:
                    flags.append(f)
                if NEW_WORD_RE.search(sent) and not NEGATED_RE.search(sent):
                    word_new.add(f)
        parser = None
        if root and flags:
            try:
                with open(os.path.join(root, TOOLS_RS), encoding="utf-8") as fh:
                    parser = fh.read()
            except (OSError, UnicodeDecodeError) as e:
                res.append(("R3", "WARN", f"could not read {TOOLS_RS} under --root to tell new flags from "
                            f"existing ones, judging by wording only: {e}"))
        if parser is not None:
            # With the parser source at hand, "new" means "not in it today", whatever the wording.
            fresh = [f for f in flags if not re.search(rf"(?<![\w-]){re.escape(f)}(?![\w-])", parser)]
        else:
            fresh = sorted(word_new)
        old = [f for f in flags if f not in fresh]
        if fresh:
            how = "absent from the current parser" if parser is not None else "worded as new"
            res.append(("R3", "FAIL", f"new flag(s) {', '.join(sorted(fresh))} ({how}) but {TOOLS_RS} "
                        "(the parser) is not claimed"))
        if old:
            res.append(("R3", "WARN", f"flag(s) {', '.join(sorted(old))} named in Requirements; "
                        f"{TOOLS_RS} (the parser) is not claimed: does a parser or call site there change?"))

    if reported is None:
        reported = report_hot(root) if root else set()
    hot = hot_claims(lines, secs, reported)
    reason = any((m := HOT_REASON_RE.match(ln)) and len(m.group(1)) >= HOT_REASON_MIN for ln in lines)
    split_first = any(re.fullmatch(r"\s*(?:[-*]\s+)?Split first:\s+t-\d{8}-\d{6}\s*", ln)
                      for ln in lines)
    # The original HOT entries retain their existing reason and key-claim rules.
    blocked = [path for path in hot if not reason and (path in HOT or not split_first)]
    if blocked:
        res.append(("R6", "FAIL", "whole-file claim on hot shared file(s) " + ", ".join(blocked)
                    + ": claim a #key, split the brief, or add a 'Hot file:' line with a reason "
                    + f"of at least {HOT_REASON_MIN} characters; report entries also accept Split first: <task id>"))

    for path in sorted(claims):
        if re.fullmatch(r"(?:crates/[^/]+/)?(?:src|tests)/*", path):
            res.append(("R7", "FAIL", f"folder claim {path} covers a whole src/ or tests/ folder; claim the files"))

    if root:
        res.extend(size_warnings(claim_blocks(lines, secs), root))

    if CONFIG_KEY_RE.search(req):
        miss = [n for n in CONFIG_NEEDS if not any(c == n or c.endswith("/" + n) for c in claims)]
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
                r = subprocess.run(["git", "-C", root, "grep", "-n", "-w", "-F", "--", s],
                                   capture_output=True, text=True, encoding="utf-8")
            except OSError as e:
                res.append(("R5", "WARN", f"git grep for `{s}` could not run: {e}"))
                continue
            if r.returncode > 1:
                res.append(("R5", "WARN", f"git grep for `{s}` failed: {r.stderr.strip()[:80]}"))
                continue
            hits = {}
            for ln in r.stdout.splitlines():
                m = re.match(r"(.+?):(\d+):", ln)
                if m and not covers(claims, m.group(1)):
                    hits.setdefault(m.group(1), m.group(2))
            if hits:
                res.append(("R5", "WARN", f"`{s}` is defined or used in unclaimed: "
                            + ", ".join(f"{f}:{n}" for f, n in hits.items())))
    board = board_fields(lines)
    if min_hard_cases is not None and any(
            re.match(r"high\b", v, re.I) for v in board.get("risk", [])):
        hard = section_text(lines, secs, {"hard cases"})
        items = []
        for ln in hard.splitlines():
            if ITEM_RE.match(ln):
                items.append(ln)
            elif items and ln.strip():
                items[-1] += " " + ln.strip()
        untied = []
        for i, item in enumerate(items, 1):
            names = [s for s in TICKS_RE.findall(item)
                     if re.fullmatch(r"[a-z][a-z0-9]*(?:_[a-z0-9]+)+", s)]
            if not any(re.search(rf"(?<![A-Za-z0-9_]){re.escape(n)}(?![A-Za-z0-9_])", done_text)
                       for n in names):
                untied.extend(names or [f"item {i} (no test name)"])
        if len(items) < min_hard_cases or untied:
            res.append(("R9", "FAIL", f"hard cases: need >= {min_hard_cases} items each naming "
                        f"a test from the Done check (found {len(items)}, "
                        f"untied: {', '.join(untied) or 'none'})"))
    if big_file_lines is not None and root is not None:
        res.extend(big_claim_gaps(lines, secs, root, big_file_lines))
    if min_pasted_lines is not None and any(
            "gpt" in v.lower() for v in board.get("model", [])):
        outside = re.search(r"(?<![\w./\\])\.\.[/\\]\S+", text) or any(
            any(marker in p.lower() for marker in ("stage7", "protocol"))
            for span in TICKS_RE.findall(text) for p in claim_paths(span))
        pasted = max((sum(bool(ln.strip()) for ln in lines[a + 1:b])
                      for name, a, b in secs if name in {"pasted", "pasted protocol"}), default=0)
        if outside and pasted < min_pasted_lines:
            res.append(("R11", "FAIL", f"pasted protocol: GPT brief names an outside-repo path; "
                        f"need >= {min_pasted_lines} non-empty Pasted lines (found {pasted})"))
    return res


def board_fields(lines):
    """Read Board/Board block fields without treating its Claims bullet as a new block."""
    fields = {}
    active = False
    for ln in lines:
        sec = section_of(ln)
        if sec and sec[0] in {"board", "board block"}:
            active = True
            continue
        field = KEY_RE.match(ln.replace("**", ""))
        if sec and not ITEM_RE.match(ln) and (
                HEADING_RE.match(ln) or sec[0] not in {"claims", "risk", "model", "title", "project"}):
            active = False
        if active and field:
            fields.setdefault(norm(field.group(1)), []).append(field.group(2).strip())
    return fields


def big_claim_gaps(lines, secs, root, limit):
    """R10: require local wiring or a local new file; count literal newlines in existing files."""
    blocks = claim_blocks(lines, secs)
    items = []
    for block in blocks:
        current = ""
        for ln in block.splitlines():
            if ITEM_RE.match(ln) or not ln.startswith((" ", "\t")):
                if current:
                    items.append(current)
                current = ln
            else:
                current += " " + ln.strip()
        if current:
            items.append(current)
    paths = {}
    new = set()
    new_files = set()
    for item in items:
        named = claim_paths(item)
        named |= {s.replace("\\", "/") for s in TICKS_RE.findall(item)
                  if re.fullmatch(r"[\w./\\:#-]+", s)}
        # claim_paths intentionally ignores drive letters and absolute POSIX paths for R1-R8.
        named |= {t.replace("\\", "/") for t in re.split(r"[\s,;()\[\]`'\"]+", item)
                  if t and (pathlib.PureWindowsPath(t).drive or t.startswith("/"))}
        for path in named:
            paths[path] = paths.get(path, False) or bool(re.search(r"\bwiring\b", item, re.I))
            marker = re.search(re.escape(path) + r"[`'\"]*\s*\(\s*new\b([^)]*)\)",
                               item.replace("\\", "/"), re.I)
            if marker:
                new.add(path)
                if not path.endswith("/") and not re.search(r"\bfolder\b", marker.group(1), re.I):
                    new_files.add(path)
    res = []
    try:
        base = pathlib.Path(root).resolve()
    except (OSError, ValueError, RuntimeError) as e:
        return [("R10", "WARN", f"cannot resolve root: {e}")]
    new_targets = set()
    for path in sorted(new_files):
        try:
            target = (base / path).resolve()
            win = pathlib.PureWindowsPath(path)
            if (win.drive and os.name != "nt") or not target.is_relative_to(base):
                res.append(("R10", "WARN", f"claimed path outside root: {path}"))
                continue
            new_targets.add(target)
        except (OSError, ValueError, RuntimeError) as e:
            res.append(("R10", "WARN", f"cannot resolve new claim {path}: {e}"))
    for path, wiring in sorted(paths.items()):
        if path in new:
            continue
        try:
            win = pathlib.PureWindowsPath(path)
            target = (base / path).resolve()
            if (win.drive and os.name != "nt") or not target.is_relative_to(base):
                res.append(("R10", "WARN", f"cannot count claimed path outside root: {path}"))
                continue
            try:
                target.stat()
            except FileNotFoundError:
                continue
            if not target.is_file():
                continue
            with target.open("rb") as f:
                count = sum(chunk.count(b"\n")
                            for chunk in iter(lambda: f.read(io.DEFAULT_BUFFER_SIZE), b""))
        except (OSError, ValueError, RuntimeError) as e:
            res.append(("R10", "WARN", f"cannot count {path}: {e}"))
            continue
        if count > limit and not wiring and not any(
                new_target.is_relative_to(target.parent) for new_target in new_targets):
            res.append(("R10", "FAIL", f"{path} has {count} lines (over {limit}); "
                        "claim wiring only or a (new) file for the new module"))
    return res


def size_warnings(blocks, root):
    """Count claimed files by streaming; never read a path outside the supplied root."""
    base = pathlib.Path(root).resolve()
    try:
        with open(base / "broadsword.toml", "rb") as f:
            limit = tomllib.load(f)["hotspot_lines"]
        if type(limit) is not int or limit < 1:
            raise ValueError("hotspot_lines must be a positive integer")
    except (OSError, ValueError, KeyError) as e:
        return [("R8", "WARN", f"cannot read hotspot_lines: {e}")]
    paths = claim_paths("\n".join(blocks))
    # Key claims still name files for the size advisory.
    paths |= {tok.split("#", 1)[0].replace("\\", "/")
              for block in blocks for tok in re.split(r"[\s,;()\[\]`'\"]+", block)
              if "#" in tok and PATH_RE.fullmatch(tok.split("#", 1)[0])}
    warnings = []
    for path in sorted(paths):
        target = (base / path).resolve()
        if not target.is_relative_to(base):
            warnings.append(("R8", "WARN", f"cannot count claimed path outside root: {path}"))
            continue
        if not target.is_file():
            continue
        try:
            with target.open("rb") as f:
                count = sum(1 for _ in f)
        except OSError as e:
            warnings.append(("R8", "WARN", f"cannot count {path}: {e}"))
            continue
        if count > limit:
            warnings.append(("R8", "WARN", f"warning: {path} has {count} lines; "
                             "put new code in a new module (PLAN.md R4)"))
    return warnings


PF_SKIP_RE = re.compile(r"^(?:scratch|state|build|worktrees|projects)/")
PF_SKIP_SENT_RE = re.compile(r"\bseparate task\b|\bafter this one\b", re.I)
DOC_EXTS = {"bsn", "rs", "py", "csv", "meta", "toml", "input", "json"}
TEST_NAME_RE = re.compile(r"\btest\s+([A-Za-z_][A-Za-z0-9_]{3,})\s+\.\.\.|`fn\s+([A-Za-z_]\w{3,})`")
PREREQ_RE = re.compile(r"\b(?:after|needs?|requires?|depends\s+on|blocked\s+by)\s+"
                       r"((?:t-\d{8}-\d{6}(?:-\d+)?(?:\s*(?:,|and)\s*)?)+)", re.I)
TASK_ID_RE = re.compile(r"t-\d{8}-\d{6}(?:-\d+)?")
PF_CMD_FLAGLESS = {"broadsword", "cargo", "git"}
RECORDS_DIRS = ("docs/", "evidence/")
RECORDS_FILES = {"PROGRESS.md", "LAB_LOG.md", "RETRO.md"}
RECORDS_LINE = "Every number and claim is quoted from a file or command output."


def _git(root, args):
    r = subprocess.run(["git", "-C", root] + args, capture_output=True, text=True, encoding="utf-8")
    return r


def new_claim_paths(lines, secs):
    """Claimed paths the Claims text marks new: `p (new)`, `p (new folder)`, `p (new, reason)`."""
    out = set()
    texts = []
    for name, a, b in secs:
        if name == "claims":
            texts.append(section_of(lines[a])[1])
            texts.extend(lines[a + 1:b])
    for ln in lines:
        m = re.search(r"\bClaims?:\s*(.*)", ln)
        if m:
            texts.append(m.group(1))
    text = "\n".join(texts).replace("\\", "/")
    for p in claim_paths(text):
        if re.search(re.escape(p) + r"[`'\"]*\s*\((?:[^)]*\b)?new\b", text):
            out.add(p)
    return out


def preflight(text, root, ref, queue_text):
    """Plan preflight: [(rule, 'FAIL', message)] for inputs a brief names that are not on `ref` of the
    repo at `root` and not claimed as new, and for prerequisite tasks not merged. Empty when clean.
      P0 the tree of `ref` cannot be read          P3 a named test missing   P5 a prerequisite not merged
      P1 a path named in the brief missing         P4 a named flag missing
      P2 a path or test named by a doc the brief names (read one level deep) missing"""
    r = _git(root, ["ls-tree", "-r", "--name-only", ref])
    if r.returncode != 0:
        return [("P0", "FAIL", f"cannot read the tree of {ref} under --root: {r.stderr.strip()[:120]}")]
    tree = set(r.stdout.splitlines())
    prefix = os.path.basename(os.path.abspath(root)).lower()

    def on_main(p):
        if p in tree or any(f.startswith(p.rstrip("/") + "/") for f in tree):
            return True
        return "/" not in p and any(f.endswith("/" + p) for f in tree)

    def trim(p):
        q = re.sub(rf"^projects/{re.escape(prefix)}/", "", p, flags=re.I)
        return None if (PF_SKIP_RE.match(q) or "<" in q or "*" in q) else q

    def grep(term):
        # Docs are excluded: a doc that names a test or flag does not prove it exists.
        g = _git(root, ["grep", "-q", "-w", "-F", "-e", term, ref, "--", ":!*.md"])
        return g.returncode == 0

    lines = text.splitlines()
    secs = split_sections(lines)
    claims = claimed_paths(lines, secs)
    newc = new_claim_paths(lines, secs)
    fresh = lambda p: covers(newc, p)
    res = []

    body = "\n".join(s for s in SENT_RE.split(text) if not PF_SKIP_SENT_RE.search(s))
    miss = []
    for p in named_paths(body):
        q = trim(p)
        if q and not on_main(q) and not fresh(q) and q not in miss:
            miss.append(q)
    if miss:
        res.append(("P1", "FAIL", "named in the brief, not on " + ref + " and not claimed as new: " + ", ".join(miss)))

    # Docs the brief names (and that exist on ref) are read one level deep: what they say to run needs
    # its inputs too. Only the brief's own "(new)" claims excuse a missing file, not the doc's wording.
    dmiss, tmiss, seen = [], [], set()
    for p in named_paths(body):
        q = trim(p)
        if not (q and q.endswith(".md") and q in tree) or covers(claims, q):
            continue
        d = _git(root, ["show", f"{ref}:{q}"])
        if d.returncode != 0:
            continue
        dtext = d.stdout
        for tok in re.split(r"[\s,;()\[\]`'\"]+", dtext):
            tok = tok.strip().rstrip(".:")
            # A token with a backslash is a regex or an escape in a command, not a path.
            if not tok or "\\" in tok or not claim_paths(tok) or tok.rsplit(".", 1)[-1] not in DOC_EXTS:
                continue
            t = trim(tok)
            if t and not on_main(t) and not fresh(t) and t not in seen:
                seen.add(t)
                dmiss.append(f"{t} (named by {q})")
        for m in TEST_NAME_RE.finditer(dtext):
            n = m.group(1) or m.group(2)
            if not grep(n) and n not in tmiss:
                tmiss.append(f"{n} (named by {q})")
    if dmiss:
        res.append(("P2", "FAIL", "named by a doc the brief reads, not on " + ref + ": " + ", ".join(dmiss)))

    new_test_file = any(re.search(r"(?:^|/)tests?/.*\.\w+$|(?:^|/)test_\w+\.\w+$|\.rs$", c) for c in newc)
    tm = [m.group(1) or m.group(2) for m in TEST_NAME_RE.finditer(body)]
    # The one place P3 is decided: brief-named and doc-named tests get the same new-test-file excuse.
    tests = [] if new_test_file else [n for n in dict.fromkeys(tm) if not grep(n)] + tmiss
    if tests:
        res.append(("P3", "FAIL", "test(s) named, not on " + ref + " and no new test file claimed: " + ", ".join(tests)))

    flags = []
    for sent in SENT_RE.split(TICKS_RE.sub(lambda m: "" if is_command(m.group(1)) and
                                           m.group(1).split()[0].split("/")[-1].split("\\")[-1].lower()
                                           .removesuffix(".exe") in PF_CMD_FLAGLESS else m.group(0),
                                           section_text(lines, secs, {"requirements", "done check"}))):
        if NEW_WORD_RE.search(sent) and not NEGATED_RE.search(sent):
            continue
        for f in FLAG_RE.findall(sent):
            if f not in flags and not grep(f):
                flags.append(f)
    if flags:
        res.append(("P4", "FAIL", "flag(s) named, not on " + ref + " and not described as new: " + ", ".join(flags)))

    ids = []
    for m in PREREQ_RE.finditer(text):
        ids += TASK_ID_RE.findall(m.group(1))
    ids = list(dict.fromkeys(ids))
    if ids:
        if queue_text is None:
            res.append(("P5", "FAIL", "prerequisite task(s) named but no --queue-list given to check them: "
                        + ", ".join(ids)))
        else:
            merged = {f[2] for f in (ln.split("\t") for ln in queue_text.splitlines())
                      if len(f) > 2 and f[1] == "merged"}
            bad = [i for i in ids if i not in merged]
            if bad:
                res.append(("P5", "FAIL", "prerequisite task(s) not merged in queue list: " + ", ".join(bad)))

    # The one place P6 is decided: a records claim needs the quoted-claims line in the brief.
    recs = sorted(c for c in claims if c.startswith(RECORDS_DIRS) or c.rsplit("/", 1)[-1] in RECORDS_FILES)
    if recs and not any(re.sub(r"^(?:[-*]|\d+[.)])\s+", "", ln.strip()) == RECORDS_LINE for ln in lines):
        res.append(("P6", "FAIL", "claims records file(s) " + ", ".join(recs)
                    + " but the brief lacks the line: " + RECORDS_LINE))
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
    p.add_argument("--min-hard-cases", type=positive, help="enable R9 with this minimum item count")
    p.add_argument("--big-file-lines", type=positive, help="enable R10 with --root and this newline limit")
    p.add_argument("--min-pasted-lines", type=positive, help="enable R11 with this minimum body line count")
    p.add_argument("--root", default=None, help="repo to search for R5 (git grep); omit to skip R5")
    p.add_argument("--preflight-ref", default=None,
                   help="git ref (the main branch) of --root; turns on the plan preflight P0-P5")
    p.add_argument("--queue-list", default=None,
                   help="file holding `broadsword queue list` output; P5 reads merged prerequisites from it")
    p.add_argument("briefs", nargs="+")
    try:
        a = p.parse_args(argv)
    except SystemExit as e:
        return 2 if e.code else 0
    if a.preflight_ref and not a.root:
        print("brief_lint: --preflight-ref needs --root", file=sys.stderr)
        return 2
    if a.queue_list and not a.preflight_ref:
        print("brief_lint: --queue-list needs --preflight-ref", file=sys.stderr)
        return 2
    queue_text = None
    if a.queue_list:
        try:
            with open(a.queue_list, encoding="utf-8") as f:
                queue_text = f.read()
        except (OSError, UnicodeDecodeError) as e:
            print(f"brief_lint: cannot read {a.queue_list}: {e}", file=sys.stderr)
            return 2
    failed = 0
    reported = report_hot(a.root) if a.root else set()
    for path in a.briefs:
        try:
            with open(path, encoding="utf-8") as f:
                text = f.read()
        except (OSError, UnicodeDecodeError) as e:
            print(f"brief_lint: cannot read {path}: {e}", file=sys.stderr)
            return 2
        res = lint(text, a.max_lines, a.max_bullets, a.max_files, a.max_done_lines)
        gaps = claim_gaps(text, a.root, reported, min_hard_cases=a.min_hard_cases,
                          big_file_lines=a.big_file_lines, min_pasted_lines=a.min_pasted_lines)
        pre = preflight(text, a.root, a.preflight_ref, queue_text) if a.preflight_ref else []
        bad = [r for r in res if not r[1]] + [g for g in gaps if g[1] == "FAIL"] + pre
        print(f"{path}: {'ok' if not bad else str(len(bad)) + ' rule(s) failed'}")
        for rule, ok, msg in res:
            print(f"  {'PASS' if ok else 'FAIL'} {rule}: {msg}")
        for rule, level, msg in gaps:
            print(f"  {level} claimgap-{rule}: {msg}")
        if not any(g[1] == "FAIL" for g in gaps):
            print("  PASS claimgap")
        if a.preflight_ref:
            for rule, level, msg in pre:
                print(f"  {level} preflight-{rule}: {msg}")
            if not pre:
                print("  PASS preflight")
        failed += bool(bad)
    print("brief_lint: ok" if not failed else f"brief_lint: {failed} brief(s) failed")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
