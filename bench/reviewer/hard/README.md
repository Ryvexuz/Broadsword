# hard1: one diff, 12 planted bugs, 3 decoys

Reviewer-benchmark case `hard1` (see `../cases.json`). A reviewer sees only the brief and `../cases/hard1.diff`
(the `src/` of `crate/buggy` as new files). Never send `crate/`, `proof/` or the `bugs` / `decoys` lists of the
answer key to a reviewer; `run.py` sends only `brief` and `diff`.

- `crate/clean/` correct version, `crate/buggy/` the same code with the plants. Std only.
- `make_diff.sh` writes `../cases/hard1.diff` (or the file given as its argument). `cases/` is git-ignored, so the diff is
  not committed: run `bash bench/reviewer/hard/make_diff.sh` before running the benchmark on hard1, the same way the
  single-bug cases are made by `make_cases.sh`. The scorer's test builds the diff itself and, if `cases/hard1.diff`
  exists, checks it is byte-identical.
- `proof/tests/proof.rs` one test per bug (fails on buggy, passes on clean) and per decoy (passes on both).
  The test for the never-failing test is a mutation check done by `prove.sh`: the delay cap is removed from
  `backoff()` and the crate's own test must then fail.
- `prove.sh` runs everything and prints the table. Run it through Broadsword, which sets `CARGO_TARGET_DIR`:
  `broadsword run broadsword --task <id> -- bash bench/reviewer/hard/prove.sh`
  Work copies go in `<scratch_dir>/hard-prove`. The race proof (bug02) is a stress test: 8 threads, 2000 rounds. It cannot be made deterministic here: the only seam between the
  check and the write would be a dangling symlink, which Windows refuses without a privilege. Measured with 2000 rounds,
  20 runs each: buggy 20/20 fail, clean 20/20 pass.
- `../score_hard.py <out-dir> [-v]` scores the result JSON files of `run.py` / `run_openai.py`.
- Tests of the scorer, from `bench/reviewer/`: `python -B -m unittest test_score_hard`
  (`-I` also drops the current folder from the module path, so `-m unittest` cannot find the module with it).
