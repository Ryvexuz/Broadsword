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
corner (2), `--no-tilt` (default start has a 5 degree tilt about x and z).

## Output columns

`t,min_y,center_y,max_beam_strain`, one row per output step.

- `t`: time, seconds
- `min_y`: height of the lowest node, metres
- `center_y`: centre-of-mass height, metres
- `max_beam_strain`: largest |length - rest| / rest over all beams (unitless)

Constants (stiffness, damping, restitution, friction) are at the top of
`src/lib.rs` with the reasoning for each.
