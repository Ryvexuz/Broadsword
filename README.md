# crash-sim

Starter crash sim for Broadsword: a cube of 8 nodes (1 kg each) joined by 28
spring-damper beams (12 edges, 12 face diagonals, 4 body diagonals) drops under
gravity onto a ground plane at y = 0, bounces and settles. Headless, std only,
deterministic: the same flags give byte-identical output.

## Run

```
cargo run --offline                       # 5 s, prints CSV to stdout
cargo run --offline -- --duration 2 --drop-height 3 --no-tilt
cargo test --offline
```

Flags: `--duration <s>` (5), `--dt <s>` output step (0.01), `--substeps <n>`
physics substeps per output step (10), `--drop-height <m>` height of the lowest
corner (2), `--gravity <m/s^2>` gravitational acceleration, for example 1.62 for the Moon (9.81), `--no-tilt` (default start has a 5 degree tilt about x and z),
`--summary` print one line instead of the CSV:
`first_contact_s=<s>,peak_strain=<x>,final_center_y=<m>` (first sample with
`min_y <= 0`, or `none`; largest `max_beam_strain`; last `center_y`). It combines
with the other flags; errors still print nothing on stdout and exit non-zero.

## Output columns

`t,min_y,center_y,max_beam_strain`, one row per output step.

- `t`: time, seconds
- `min_y`: height of the lowest node, metres
- `center_y`: centre-of-mass height, metres
- `max_beam_strain`: largest |length - rest| / rest over all beams (unitless)

Constants (stiffness, damping, restitution, friction) are at the top of
`src/lib.rs` with the reasoning for each.

## Behaviour notes

- The run aborts with a message on stderr and exit code 1 if a node goes
  non-finite or max strain passes 50%. Rows already computed are still printed.
- A physics step `dt / substeps` above the stable limit (about 3.8 ms, derived
  from the beam stiffness and node mass in `max_stable_step()`) is refused up front.
- The step count is `round(duration / dt)`, so with a `--dt` that does not divide
  the duration the last row can end up to dt/2 away from it (`--dt 0.006` ends at 4.998).
- Ground friction is a linear drag on horizontal speed (starter simplification,
  not Coulomb friction).
