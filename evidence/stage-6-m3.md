# Stage 6 milestone 3 done check (task t-20261119-171333-42)

Run 2026-11-21 ~00:20Z to 00:45Z by the coder, from the main checkout (`C:\Users\MrComputer\Desktop\Broadsword`), through `build/bin/broadsword.exe`, in the foreground, each command exactly as `projects/crash-sim/docs/STAGE6.md` writes it (one path difference in 3.8, stated there). projects/crash-sim was at `0bb2b5e`, projects/render at `c4fda9b`. Raw outputs: `scratch/s6-3-<id>.txt` (the path STAGE6 names) and a copy `scratch/s6-m3-3.<id>.txt`. Scratch can be wiped; the quoted lines below are the record.

## Result: all nine checks PASS. No limit was loosened.

| id | command (short) | key output lines (quoted) | result | raw file |
|---|---|---|---|---|
| 3.1 | `broadsword test crash-sim` | `test: ok — 457 passed, 0 failed`; `test car_basic_round_trips ... ok`; `test car_settles_level ... ok`; `test braked_car_holds_below_grip_slope ... ok`; `test braked_car_slides_above_grip_slope ... ok`; `test same_input_same_bytes ... ok`; `test launch_matches_torque_or_grip_limit ... ok`; `test full_lock_circle_matches_kinematic_radius ... ok`; `test pad_and_wall_car_is_car_basic ... ok`; `test pad_lap_closes_and_hits_wall ... ok`; 0 lines containing `... FAILED` | PASS | `scratch/s6-m3-3.1.txt` |
| 3.2 | `run crash-sim -- cargo run ... --body scenarios/car_basic.bsn --duration 12 --settle-report ...` | `settled_from_s=4.0030`; `max_speed_after_settle_by=2.878e-7`; `settle: ok`; `run: ok (3.1 s)` | PASS | `scratch/s6-m3-3.2.txt` |
| 3.3 | `car_settles_level` (a to (e)) | covered by the 3.1 line `test car_settles_level ... ok` | PASS | `scratch/s6-m3-3.1.txt` |
| 3.4a | `... --input scenarios/full_brake.input --duration 12 --slope-grip-frac 0.8 --settle-report ...` | `settled_from_s=6.9500`; `max_speed_after_settle_by=3.927e-4`; `settle: ok`; `run: ok (0.2 s)` | PASS | `scratch/s6-m3-3.4a.txt` |
| 3.4b | `... --duration 4 --slope-grip-frac 1.2 --slope-report --slope-from 2 --slope-to 4` | `slope_mu=0.8800`; `slope_tan=1.056000`; `slope_accel_mps2=1.1606`; `slope_predicted_mps2=1.1872`; `run: ok (0.1 s)`. Predicted > 0; the difference is 0.0266, the limit is 0.15 x 1.1872 = 0.178 | PASS | `scratch/s6-m3-3.4b.txt` |
| 3.5 | formulas, not a script | `scratch/s6-3-1.txt` contains `test same_input_same_bytes ... ok`, `test launch_matches_torque_or_grip_limit ... ok`, `test full_lock_circle_matches_kinematic_radius ... ok` | PASS | `scratch/s6-m3-3.1.txt` |
| 3.6 | `run crash-sim -- cargo run ... --body scenarios/pad_and_wall.bsn --input scenarios/pad_lap.input --duration 72 --drive-report --lap-min-heading 360 ... --crumple-report ...` | see "3.6 detail" below | PASS | `scratch/s6-m3-3.6.txt` |
| 3.7 | the human drive in the viewer (Lazlo, 2026-11-21) | `saved input: ../../scratch/s6-3-7.input`; `final t=28.7800 x=40.2543 z=2.2897 heading_deg=10.15`; `cross-check: ok (replay x, z within 0.001 m of the last live frame)`; `run: ok (32.2 s)` (UTF-16 file; text read through a UTF-16 view). Lazlo's words in `state/pc-progress.md` 2026-11-21T00:17:38Z: "a lot better. The ice fix chain ... is confirmed by the human." | PASS (quoted from the live run, not re-run by the coder) | `scratch/s6-3-8-pass/s6-3-7.txt` (copy `scratch/s6-m3-3.7.txt`) |
| 3.8 | the 3.8 command on the committed `evidence/stage-6-m3-human.input` | `lap_heading_deg=370.3`; `lap_path_m=66.26`; `lap_close_m=1.721`; `wall_first_contact_s=25.0590`; `wall_impact_speed_kmh=72.4`; `final t=28.7800 x=40.2543 z=2.2897 heading_deg=10.15`; `drive: ok`. The `final` line is character for character the one in 3.7. | PASS | `scratch/s6-m3-3.8.txt` |
| 3.9 | `broadsword test render` | `test crashed_car_final_frame_matches_golden ... ok`; `test: ok — 144 passed, 0 failed` | PASS | `scratch/s6-m3-3.9.txt` |

### 3.6 detail (every STAGE6 PASS condition)

- `run: ok (0.7 s)`.
- Last line of `scenarios/pad_lap.input` is `60 0 1 0` (`tail -1`).
- `drive: ok` and `settle: ok` are present; the last program line is `crumple: ok`.
- `wall_first_contact_s=23.2790` (below 60.0000).
- No `FAIL` line (grep of `FAIL` found none).

```
lap_heading_deg=364.0
lap_path_m=64.56
lap_close_m=0.273
chassis_ground_contacts_before_wall=0
wall_first_contact_s=23.2790
wall_impact_speed_kmh=49.7
final t=72.0000 x=46.3857 z=3.2337 heading_deg=0.21
drive: ok
last_impact_s=24.7460
settled_from_s=30.9600
max_speed_after_settle_by=3.934e-12
energy_rise_after_settle_by_j=9.277e-11
max_energy_gain_frac=0.000e0
peak_strain=0.020000
final_center_y=0.8414230961
settle: ok
crumple_groups=rear,front
crumple_start_m=4.000000
crumple_end_m=3.475846
crumple_delta_m=0.524154
crumple_delta_min_after_settle_by_m=0.524154
crumple_delta_range_after_settle_by_m=0.000000
crumple_delta_max_m=0.726328
crumple: ok
```

Recorded for milestone 4 (STAGE6 3.6): `wall_impact_speed_kmh=49.7`, `crumple_delta_m=0.524154`.
The 3.6 run output has no NaN: a NaN abort would be a `settle: FAIL unstable ...` line, and none is present.

### The one path difference in 3.8

STAGE6 3.8 names `../../evidence/stage-6-m3-human.input`, relative to `projects/crash-sim` in the main checkout. That file only exists in main after this task merges. The coder ran the command with `--input ../../worktrees/broadsword/t-20261119-171333-42/evidence/stage-6-m3-human.input` (the committed copy in this worktree); nothing else in the command changed. After the merge, the STAGE6 path as written reads the same bytes. SHA-256 of the file, before and after the copy: `1a705f46800d1a2ccaf22360a723038ff84435b5169affb9570b34bc0818cce1` (the source `scratch/s6-3-8-pass/lazlo-lap-2026-11-21.input` and `evidence/stage-6-m3-human.input`; `cmp` said identical). An earlier replay of the same file, saved by the orchestrator in `scratch/s6-3-8-pass/s6-3-8.txt`, shows the same numbers.

## Golden image

`projects/render/tests/golden/crashed_car_last.ppm`: the last frame of the 3.6 run, compared byte for byte by `crashed_car_final_frame_matches_golden` (`projects/render/tests/crashed_car.rs`; regeneration is described in its header). 3.9 above shows the test passing.

## Inputs for the Stage 6 m1-m3 retro (for the orchestrator; AGENTS.md rule)

Raw facts, from `state/pc-progress.md` (timestamps are there) and this run:

1. **The ice feel and its 3 fixes.** Lazlo's first drive (18:31Z, 2026-11-20) saved 235 lines / 40.9 s; its replay gave `wall_first_contact_s=27.3680 at 10.5 km/h` and `drive: FAIL lap_heading_deg 47.9 < 360 (no loop)`. The fix chain was: throttle ramp and cap, speed-sensitive steering, reverse, then drag and rolling resistance (crash-sim 4fd84b0; reverse 15499f4). At 00:17:38Z Lazlo said "a lot better"; the engine torque stayed as it was by his earlier condition. Lesson to test: a physics check (3.5) that passes the formulas can still give a car that feels like ice; the human drive (3.7) was the only check that caught it, and 3.5 said there was no top-speed check "by design".
2. **The shared-build race.** A shared-build clean deleted `crash-sim.exe` under a running test (14:20Z, 2 false failures); `drive.rs` used `env!(CARGO_BIN_EXE)` (15:32Z, 10 NotFound failures); at 19:49Z the viewer's 8 drive tests failed with "no crash-sim binary under build"; at 11:37Z the stale-package clean did not remove `--release` artefacts from another checkout. Fixes: csv runtime lookup 805296c, drive.rs runtime lookup (t-20261120-153143), `buildsrc.rs` clean (t-20261120-191700). Cost: false check failures and reruns on the merge queue.
3. **The 1 fps viewer.** Cause (18:31Z): input lines were sent only on a key change and `--live` never runs ahead of its input, so the sim clock stalled (`drive.rs:78-79`). Fix task t-20261120-183043.
4. **The arena grid.** The first render grid (e9ccffa, 17:37Z) followed the car, so there was no arena or route (Lazlo, 19:27Z). Fix t-20261120-192813: a fixed grid from the header, a `--route-input` lap polyline and a `C` overhead camera (render e36e464, then 9a1adff, c4fda9b).
5. **Speed and cost angle.** Records-only changes paid a full broadsword suite twice (about 15 min each under load, 15:50Z); the m3 done check waited for 3.8 to pass (22:25Z). A quiet-board settle baseline moved from 57f2a3b (3.72 ms) to e222579 (4.256 ms, new physics), and the perf gate used interim 50% thresholds.
6. **Run facts.** 3.1 ran 457 tests (0 failed) and took more than 180 s in the foreground (the tool moved it to the background at 180 s); 3.6 itself ran in 0.7 s once built, and its cost was the build lock: "build lock: taken after waiting 12.6 s".
