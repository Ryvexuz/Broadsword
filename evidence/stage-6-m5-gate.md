# Stage 6 m5 gate: the m4 table read against W1 to W4 (task t-20261122-195543)

Written 2026-11-22 by the coder (coder-sonnet) at the orchestrator's instruction, after Lazlo's answer below. It records which row of `docs/M5-SOLVER.md` section 7 the m4 table matched (`projects/crash-sim/docs/STAGE6-M4-M5.md`, "Gate before the m5 calibration" and 5.2b). The numbers come from `evidence/stage-6-m4.md` (same task), which quotes the raw outputs.

matched: W3
decision: go

## The reading, row by row

The rows are quoted from `projects/crash-sim/docs/M5-SOLVER.md` lines 107 to 110.

**W1.** "On test A at the chosen point, the misses are `peak_mps2` and `peak_time_s`, or the sim's first-contact pulse rises much faster or slower than the reference, while `static_crush_m` and `duration_s` are fine." Stop row. **Does not match.** Test A at the chosen point (4.4, `pulse: ok`): `peak_mps2` norm=0.0873 ok, `peak_time_s` norm=0.6818 ok; no metric is a miss. Test B (4.7): `peak_mps2` norm=0.0428 ok, `peak_time_s` norm=0.5864 ok; the one miss is `dv_mps` norm=1.0650 FAIL. Peak and peak time are not misses.

**W2.** "`static_crush_m` is too large and `duration_s`/`olc_mps2` too short at every good grid point, with the chosen yield strain and area not on an edge." Go row. **Does not match.** The grid has no good point (0 ok of 140), so "at every good grid point" has nothing to hold on. At the chosen point: on test B `static_crush_m` sim=0.4185 ref=0.4368 (sim smaller, norm 0.2099 ok) and `duration_s` sim=0.0782 ref=0.0795 (norm 0.1635 ok); on test A `static_crush_m` sim=0.4162 ref=0.3508 (sim larger, norm 0.9325 ok) while `duration_s` sim=0.0784 ref=0.0748 is longer, not shorter, and `olc_mps2` sim=299.7 ref=347.8 is lower (norm 0.9218 ok). The static crush is not too large with duration and OLC too short on both tests, and the chosen area is on its range floor (see W3), which the W2 wording excludes. Static crush is not too large in the sense of W2.

**W3.** "The chosen point is on an `edge`, or many grid points are `refused` for the step limit (`docs/STAGE6-M4-M5.md:185`)." Row action: "Go if W2 also holds; otherwise ask Lazlo whether to implement (1), which needs the consume-numerics decision first." **Matches.**
- The calibrated line is `calibrated node-beam deviation area_m2=0.00007000000000110004 yield_strain=0.0024 damping_ratio=0.01500000336789605 score=0.6832`; the front area sits at its range floor, `frozen: range crush.area_m2 = 0.00007 0.002`. It carries no `edge` mark because its area computes back 1.6e-11 above the floor (the doc text says so); the grid's own choice, `grid_best node-beam 41 ... score=0.6456 edge verdict=FAIL`, does carry `edge`.
- Of 140 grid points, 80 are `refused`: 20 (points 61 to 80) were run and refused by the step limit ("limit 0.000093 s against h = 0.0001 s"), and 60 (points 81 to 140) were not run because their rear stage area (4 x the front) is outside the per-stage range 0.00007 to 0.002 (the range rule). The M5-SOLVER note predicted 60 step-limit refusals; the measured step-limit refusals are 20, the other 60 are the range rule.
- Since W2 does not hold, the row's action is to ask Lazlo whether to implement option (1) of the note. That was asked, and answered below.

**W4.** "m4 B passes with a `pulse_score` well under 0.5." **Does not match.** Test B: `pulse_score=0.4244`, `pulse: FAIL dv_mps norm=1.0650`. B does not pass.

## Lazlo's decision on W3's ask

2026-11-22 19:50:55Z, in the thread, message `cmsg_019Gy38ApTnA8WHdHwRbXZzHMk1wmBWuaRZB9vKMgoR7pq`, answering `cmsg_019Gy38ApTnA8WHdHwRbXZzHPewFGqYMXJTkhvkGv8REzQ`. His words: "1". He chose option (1): implicit node-beam with sparse CG, the solver numerics copied into crash-sim. Recorded in `state/standing-orders.md` (19:50:55Z entry) and in `state/pc-progress.md` at 19:51:19Z ("m5 = option (1) implicit node-beam + sparse CG, copied (standing-orders). m5 gate: matched W3, decision go").

So: the gate matched W3, W1 (the stop row) did not match, and the decision is go.
