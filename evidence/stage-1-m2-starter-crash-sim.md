# Stage 1, milestone 2: starter crash sim — done check

**Check:** it builds, runs headless and prints the cube's height over time.

**Result: PASS**, 2026-11-17. The crash sim is its own repo at `projects/crash-sim/`, on `main` at `9ff5289`. Broadsword's `.gitignore` leaves it out.

## Commands and output (run by the orchestrator)

```text
$ cargo build --offline
    Finished `dev` profile [unoptimized + debuginfo] target(s)
$ cargo test --offline
test result: ok. 11 passed; 0 failed
$ cargo run --offline -q > crash-default.csv      # exit 0, 502 lines (header + 501 rows)
t,min_y,center_y,max_beam_strain
0.0000,2.000000,2.583192,0.000000
0.0100,1.999460,2.582652,0.000000
...
4.9900,0.000000,0.499871,0.000259
5.0000,0.000000,0.499871,0.000259
```

Centre-of-mass height every 0.5 s:

| t (s) | 0.0 | 0.5 | 1.0 | 1.5 | 2.0 | 2.5 | 3.0 | 5.0 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| center_y (m) | 2.583 | 1.354 | 1.004 | 0.823 | 0.568 | 0.502 | 0.500 | 0.500 |

- **First contact** at t = 0.65 s. Free fall from 2 m predicts √(2·2/9.81) = 0.64 s.
- **The bounce is real.** center_y drops to 0.506 at t = 0.67 s, then rises to 1.004 at t = 1.00 s. The rebound ratio (1.004 − 0.5) / (2.583 − 0.5) = 0.24 is close to e² = 0.25 for restitution 0.5.
- **It settles** at center_y 0.4999, resting on a face. Peak beam strain over the run is 4.5%.
- **Deterministic.** A second run is byte-identical (`cmp`).
- **Unstable settings are refused.** `--drop-height 10 --substeps 1 --dt 0.02` exits 1 and explains why:

```text
crash-sim: physics step h = dt/substeps = 0.020000 s exceeds the stable limit 0.003780 s (symplectic Euler needs omega_max*h < 2 with omega_max <= sqrt(14*k/m) = 529 rad/s, k = 20000); raise --substeps or lower --dt
```

## Review

- **First pass: FAIL.** The physics was correct, but an unstable run printed `inf`/`NaN` rows, reported 0% strain and exited 0, because the max over strains dropped NaN.
- **Second pass: PASS.** The reviewer built the cube's 24×24 stiffness matrix and found ω_max = 400 rad/s. The code's bound of 529 rad/s is therefore a valid, conservative limit. A 10,000 km drop at the largest allowed step aborts mid-run with exit 1 and a clear message.

## Not covered

- No test exercises the mid-run abort end to end; the unit test calls `check()` directly. The reviewer reproduced it by hand with a 1e7 m drop.
- A doc comment in `src/lib.rs` says the real mode is "about 450" rad/s. The reviewer computed exactly 400.
