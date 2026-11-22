"""Identify normalized line moves in committed changes since a merge base."""

import argparse
import ast
from collections import Counter
from dataclasses import dataclass
import json
import re
import subprocess
import sys


class CheckError(Exception):
    """An argument, Git, or diff-format failure."""


class Parser(argparse.ArgumentParser):
    def error(self, message):
        raise CheckError(f"usage: {self.prog} --trunk BRANCH --max-other N [--repo DIR]: {message}")


def budget(value):
    if not re.fullmatch(r"[0-9]+", value):
        raise argparse.ArgumentTypeError("--max-other must be a nonnegative integer")
    try:
        return int(value)
    except ValueError as exc:
        raise argparse.ArgumentTypeError("--max-other integer is too large") from exc


def git(repo, *args):
    result = subprocess.run(
        ["git", "-C", repo, *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode:
        raise CheckError(result.stderr.decode("utf-8", "replace").strip() or "git failed")
    return result.stdout.decode("utf-8", "surrogateescape")


def path_name(value):
    value = value.split("\t", 1)[0]
    if value.startswith('"'):
        try:
            value = ast.literal_eval(value).encode("latin1").decode("utf-8", "surrogateescape")
        except (ValueError, SyntaxError, UnicodeError) as exc:
            raise CheckError("invalid quoted Git path") from exc
    return value[2:] if value.startswith(("a/", "b/")) else value


def normalize(text):
    text = text.strip()
    for prefix in ("pub(super) ", "pub(crate) "):
        if text.startswith(prefix):
            return text[len(prefix):]
    return text


@dataclass
class Line:
    sign: str
    path: str
    number: int
    text: str
    key: str


def parse_diff(diff):
    lines = []
    old_path = new_path = None
    old = new = 0
    removed = added = 0
    for raw in diff.split("\n"):
        if removed or added:
            if raw.startswith("\\ No newline at end of file"):
                continue
            if raw.startswith("-") and removed:
                text = raw[1:]
                lines.append(Line("-", old_path, old, text, normalize(text)))
                old += 1
                removed -= 1
            elif raw.startswith("+") and added:
                text = raw[1:]
                lines.append(Line("+", new_path, new, text, normalize(text)))
                new += 1
                added -= 1
            else:
                raise CheckError("invalid zero-context diff body")
        elif raw.startswith("diff --git "):
            old_path = new_path = None
        elif raw.startswith("--- "):
            old_path = path_name(raw[4:])
        elif raw.startswith("+++ "):
            new_path = path_name(raw[4:])
        elif raw.startswith("@@ "):
            match = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", raw)
            if not match or old_path is None or new_path is None:
                raise CheckError("invalid diff hunk")
            old, old_count, new, new_count = match.groups()
            old, new = int(old), int(new)
            removed = int(old_count) if old_count is not None else 1
            added = int(new_count) if new_count is not None else 1
        elif raw.startswith(("Binary files ", "GIT binary patch")):
            raise CheckError("binary changes cannot be classified as line moves")
    if removed or added:
        raise CheckError("truncated diff hunk")
    return [line for line in lines if line.key]


def classify(lines):
    counts = {
        sign: Counter(line.key for line in lines if line.sign == sign)
        for sign in ("-", "+")
    }
    matches = counts["-"] & counts["+"]
    remaining = {sign: matches.copy() for sign in counts}
    unmatched = []
    for line in lines:
        if remaining[line.sign][line.key]:
            remaining[line.sign][line.key] -= 1
        else:
            unmatched.append(line)
    wiring = sum(line.key.startswith(
        ("mod ", "pub mod ", "use ", "pub use ", "#[path", "import ")
    ) for line in unmatched)
    return sum(matches.values()), wiring, len(unmatched) - wiring, unmatched


def main(argv=None):
    parser = Parser(description=__doc__)
    parser.add_argument("--trunk", required=True)
    parser.add_argument("--max-other", required=True, type=budget)
    parser.add_argument("--repo", default=".")
    try:
        args = parser.parse_args(argv)
        for name, value in (("--trunk", args.trunk), ("--repo", args.repo)):
            if not value or any(ord(char) < 32 for char in value):
                raise CheckError(f"{name} must be nonempty and contain no control characters")
        trunk = git(args.repo, "rev-parse", "--verify", "--end-of-options",
                    args.trunk + "^{commit}").strip()
        head = git(args.repo, "rev-parse", "--verify", "HEAD^{commit}").strip()
        base = git(args.repo, "merge-base", trunk, head).strip()
        diff = git(args.repo, "diff", "-U0", "--no-color", "--no-renames",
                   "--no-ext-diff", "--no-textconv", "--no-relative",
                   "--src-prefix=a/", "--dst-prefix=b/", base, head, "--")
        if not diff:
            print("move_check: FAILED (empty diff)")
            return 1
        moved, wiring, other, unmatched = classify(parse_diff(diff))
        for line in unmatched:
            # JSON quoting keeps hostile filenames and text on one output line.
            print(f"{line.sign} {json.dumps(line.path)}:{line.number} "
                  f"{json.dumps(line.text)}")
        if moved > 0 and wiring + other <= args.max_other:
            print(f"move_check: ok (moved {moved}, wiring {wiring}, other {other})")
            return 0
        print(f"move_check: FAILED (moved {moved}, wiring {wiring}, "
              f"other {other}, limit {args.max_other})")
        return 1
    except (CheckError, OSError, ValueError) as exc:
        print("move_check: " + " ".join(str(exc).splitlines()), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
