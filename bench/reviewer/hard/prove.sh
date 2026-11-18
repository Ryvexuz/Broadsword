#!/usr/bin/env bash
# Runs the proof suite against crate/clean and crate/buggy and prints the table.
# Run from anywhere, through Broadsword, which sets CARGO_TARGET_DIR:
#   broadsword run broadsword --task <id> -- bash bench/reviewer/hard/prove.sh
# Work files go in <scratch_dir>/hard-prove (scratch_dir is read from broadsword.toml).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
SCRATCH_DIR=$(sed -n 's/^scratch_dir *= *"\([^"]*\)".*/\1/p' "$ROOT/broadsword.toml")
[ -n "$SCRATCH_DIR" ] || { echo "scratch_dir not found in broadsword.toml" >&2; exit 2; }
: "${CARGO_TARGET_DIR:?CARGO_TARGET_DIR is not set: run this through broadsword run}"
W="$ROOT/$SCRATCH_DIR/hard-prove"
rm -rf "$W"; mkdir -p "$W"

for V in clean buggy; do
  # 1. the proof tests against an untouched copy
  cp -r "$HERE/crate/$V" "$W/$V"; mkdir "$W/$V/tests"; cp "$HERE/proof/tests/proof.rs" "$W/$V/tests/"
  cargo test --offline --manifest-path "$W/$V/Cargo.toml" --test proof --no-fail-fast -- --test-threads=1 >"$W/$V.proof.txt" 2>&1
  # the crate's own unit tests, for information (they pass on both versions)
  cargo test --offline --manifest-path "$W/$V/Cargo.toml" --lib >"$W/$V.lib.txt" 2>&1
  echo "$V: own unit tests: $(grep -E '^test result' "$W/$V.lib.txt")"
  # 2. mutation proof for the test that can never fail: cap removed from backoff(), then the crate's own test must fail
  cp -r "$HERE/crate/$V" "$W/$V-mut"
  python -I -B "$HERE/proof/tools.py" mutate "$W/$V-mut" || exit 2
  cargo test --offline --manifest-path "$W/$V-mut/Cargo.toml" --lib backoff_stays_within_cap >"$W/$V.mut.txt" 2>&1
  echo $? >"$W/$V.mut.exit"
done

python -I -B "$HERE/proof/tools.py" report "$W"
