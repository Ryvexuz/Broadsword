# Stage 1, milestone 3: job runner and test runner — done check

**Check:** one command builds and tests the starter crash sim.

**Result: PASS**, 2026-11-17, on `main` at merge commit `5410333`.

## The one command

```text
$ build/debug/broadsword.exe projects
broadsword	C:\Users\setup\Desktop\Broadsword
crash-sim	C:\Users\setup\Desktop\Broadsword\projects\crash-sim

$ build/debug/broadsword.exe check crash-sim          # exit 0
job: check crash-sim (log: ...\scratch\logs\jobs\20261117-171233-crash-sim-check.log)
test result: ok. 0 passed; ...
test result: ok. 0 passed; ...
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
test result: ok. 0 passed; ...
--- summary ---
build: ok (0.1 s)
test: ok — 11 passed, 0 failed
```

The build took 0.1 s because the crash sim was already built in the shared `build/` folder. Every job sets `CARGO_TARGET_DIR` to the main root's `build/`, worktrees included.

## Stage 1 done check 3: refuse a job that would break the disk reserve

This was run on the real disk with 128.5 GB free. A temporary root was given a copy of `broadsword.toml` with `disk_floor_gb = 127`:

```text
$ broadsword check crash-sim
broadsword: refusing to start build: free space 128.5 GB minus the 2.0 GB job reserve would be below the 127.0 GB floor
exit=3
```

Cargo was never started. The temporary root was deleted afterwards.

## Tests and review

- 23 unit tests, 15 tests in `cli.rs` and 10 in `jobs.rs`, all passing. Clippy runs with `-D warnings` and is clean.
- **Reviewer (Opus), first pass: FAIL.** The test-only free-space override could report more space than the disk had. Any agent could write a large number into a scratch file and get a real job past a real low disk. The reviewer showed it on the real disk.
- **Second pass on `343a306`: PASS.** Every override reading is now capped at the real free space, and the same bypass attempt exits 3. Logs no longer overwrite each other: four parallel builds in one second produced four separate logs. The three new settings are required, and the printed paths are clean.

## Not covered

- The disk watch during a job was tested only through the override. Lowering free space mid-job through the override killed the cargo process tree, wrote the pause file and exited 4. The real disk was not filled during a job.
- `broadsword check broadsword` was not run as part of the done check. It relinks `build/debug/broadsword.exe`, the same binary the hook runs, while that binary may be in use. This is noted in `IDEAS.md`.
