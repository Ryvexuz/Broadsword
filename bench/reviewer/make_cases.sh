#!/usr/bin/env bash
# Regenerate the benchmark diffs from git history. Run from the Broadsword root.
# Each case is a first-pass commit whose real blocking bug is known (see cases.json).
# Only source files are included, so a "required setting not added to broadsword.toml" finding is an artifact of this
# (the real commits did add it) and is not scored.
set -eu
D=bench/reviewer/cases; mkdir -p "$D"
git -C projects/crash-sim show --format= a19ee1a -- src > "$D/a19ee1a.diff"
git diff 67cf827 fd98728 -- clients/openai_mcp.py > "$D/fd98728.diff"
git diff c19f23a 3ad7320 -- 'crates/broadsword/src/*' > "$D/3ad7320.diff"
git diff cadfc87 cadbb6d -- 'crates/broadsword/src/*' > "$D/cadbb6d.diff"
git diff 13e8215 36bca91 -- 'crates/broadsword/src/*' > "$D/36bca91.diff"
wc -l "$D"/*.diff
