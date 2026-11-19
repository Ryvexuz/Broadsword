#!/usr/bin/env bash
# Regenerate the benchmark diffs from git history. Run from the Broadsword root.
# Each case is a first-pass commit whose real blocking bug is known (see cases.json).
# Every file the commit touched is in its diff (tests, broadsword.toml, .gitignore, lock files). Only compiled
# Python caches (__pycache__, binary, no review value) are left out. A diff that hid broadsword.toml pushed
# reviewers into false "the new setting was not added to broadsword.toml" findings (5 of the 43 extra BLOCKING
# lines in state/bench-results/extra-lines), so the check at the end refuses to finish unless each case that
# changed broadsword.toml shows it. These diffs are larger than the old source-only ones: do not compare costs
# or catch rates with runs made on the old diffs without saying so.
set -eu
D=bench/reviewer/cases; mkdir -p "$D"
EXCL=(':(exclude)*__pycache__*')
git -C projects/crash-sim show --format= a19ee1a > "$D/a19ee1a.diff"
git diff 67cf827 fd98728 -- . "${EXCL[@]}" > "$D/fd98728.diff"
git diff c19f23a 3ad7320 -- . "${EXCL[@]}" > "$D/3ad7320.diff"
git diff cadfc87 cadbb6d -- . "${EXCL[@]}" > "$D/cadbb6d.diff"
git diff 13e8215 36bca91 -- . "${EXCL[@]}" > "$D/36bca91.diff"
# Check: every touched file (but pycache) must be in its case diff, and broadsword.toml where it changed.
check() { # <id> <names of files touched, one per line>
  local id=$1 touched=$2 f
  while IFS= read -r f; do
    [ -n "$f" ] || continue
    grep -q "^diff --git a/$f b/$f" "$D/$id.diff" || { echo "make_cases: $id.diff is missing $f" >&2; exit 1; }
  done <<< "$touched"
}
check a19ee1a "$(git -C projects/crash-sim show --format= --name-only a19ee1a)"
check fd98728 "$(git diff --name-only 67cf827 fd98728 -- . "${EXCL[@]}")"
check 3ad7320 "$(git diff --name-only c19f23a 3ad7320 -- . "${EXCL[@]}")"
check cadbb6d "$(git diff --name-only cadfc87 cadbb6d -- . "${EXCL[@]}")"
check 36bca91 "$(git diff --name-only 13e8215 36bca91 -- . "${EXCL[@]}")"
for id in 3ad7320 cadbb6d 36bca91; do
  grep -q '^diff --git a/broadsword.toml b/broadsword.toml' "$D/$id.diff" || { echo "make_cases: $id.diff lacks broadsword.toml" >&2; exit 1; }
done
wc -l "$D"/*.diff
