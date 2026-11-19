#!/usr/bin/env bash
# Writes bench/reviewer/cases/hard1.diff: every source file of the case crate as a new file
# (what a reviewer would see for a first commit of the module). Run from anywhere.
# Optional argument: write to that file instead (the tests use this to build the diff themselves).
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
OUT="${1:-$HERE/../cases/hard1.diff}"
mkdir -p "$(dirname "$OUT")"
cd "$HERE/crate/buggy"
: > "$OUT"
for f in src/*.rs; do
  git diff --no-index --no-color -- /dev/null "$f" >> "$OUT" || true
done
wc -l "$OUT"
