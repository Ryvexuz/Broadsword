# Stage 6 m4 evidence: the frozen car on test A, step size, freeze, the single test B run (task t-20261122-195543)

Written 2026-11-22 by the coder (coder-sonnet) from the raw files named in each section. Nothing here was re-run except 4.6b, which the coder ran on 2026-11-22 from the Broadsword root through `build/bin/broadsword.exe`'s repository tooling (4.6b is plain git and shell, no `broadsword` command). Commands are as `projects/crash-sim/docs/STAGE6-M4-M5.md` writes them. Raw output is quoted verbatim; no claim goes beyond it.

State of crash-sim main when this was written: `df5b9e8` (the test B record) on the freeze commit F = `6822ee71c25e72ce4e566a0de93ddb3cbaca8720`.

Where each raw file came from:

- 4.4 and 4.5: `scratch/t-20261122-190342/s6-4-4-step8.txt`, `s6-4-5-step8.txt` and `s6-4-5-deltas-step8.txt`. `scratch/s6-4-4.txt` and `scratch/s6-4-5.txt` are still there and, apart from their first three job-notice lines, are byte-equal in content to those step8 copies (checked with `diff`, no difference). The other files in `scratch/t-20261122-190342/` named `s6-4-4.txt`, `s6-4-5.txt` (`pulse_score=0.6456` and `0.6434`) are the runs of grid point 41, which was not adopted, and are not quoted here.
- 4.6a and 4.7: `scratch/s6-4-6a.txt` and `scratch/s6-4-7.txt`. Both files exist, and `cmp` shows each byte-equal to its copy in `state/staging/n20-10b/`.
- 4.6b: `scratch/s6-4-6b.txt`, run now.

## Verdicts in one table

| id | PASS rule from the doc | result |
|---|---|---|
| 4.4 | every pulse-report line, both `setup` lines `ok`, `ground_contacts_in_window=0`, `pulse_score` equals the `calibrated` line's score to 4 decimals (`0.6832`) | PASS |
| 4.5 | each gated metric within 0.2 x its margin between `--substeps 10` and `--substeps 20` | PASS |
| 4.6a | exactly 18 lines, as listed in the doc | PASS |
| 4.7 | the single test B run, recorded whatever the verdict | RAN ONCE: `pulse_score=0.4244`, `pulse: FAIL dv_mps norm=1.0650`; recorded, not fixed |
| 4.6b | exactly the 16 lines of the doc | FAIL on one line, `surface_changed_since_freeze=25 rc=0` (wanted `0 rc=0`); see 4.6b |

## 4.4 The frozen car on test A

Command (from the doc; the `Running` line in the raw output below shows the same flags):

`broadsword run crash-sim -- cargo run --offline --release -- --body scenarios/validation_wall.bsn --pulse-report validation/test-a.pulse.csv --gravity 0 --duration 0.2 --dt 0.001 --substeps 10 --pulse-sensor sensor --pulse-crush-groups rear,front --pulse-cfc-shape 60 --pulse-cfc-int 180 --pulse-window 0.15 --pulse-threshold 20 --pulse-crush-tail 0.05 --setup-mass-tol 0.01 --setup-speed-tol 0.005 --margin-dv 0.06 --margin-peak 0.18 --margin-peak-time 0.022 --margin-duration 0.10 --margin-olc 0.15 --margin-static-crush 0.20 > scratch/s6-4-4.txt 2>&1`

Raw output (`scratch/t-20261122-190342/s6-4-4-step8.txt`, 29 lines):

```
job 20261121-184333-broadsword-check: check broadsword (task t-20261121-011525) was interrupted (broadsword pid 19948 is gone); left alone, as it is not a job of this call's project and task: `broadsword recover` or a call for --task t-20261121-011525 deals with it
job 20261122-190816-broadsword-check: still running (broadsword pid 27820), left alone
job 20261122-191102-broadsword-check: still running (broadsword pid 32444), left alone
job: run crash-sim (id 20261122-192622-crash-sim-run, log: C:\Users\MrComputer\Desktop\Broadsword\scratch\logs\jobs\20261122-192622-crash-sim-run.log)
source: worktrees/crash-sim/t-20261122-190342 (task t-20261122-190342)
checkpoint: refs/broadsword/checkpoints/20261122-192622-crash-sim-run (5a75be7e20)
build lock: taken after waiting 0.0 s
build source: worktrees/crash-sim/t-20261122-190342 (packages: crash-sim)
   Compiling crash-sim v0.1.0 (C:\Users\MrComputer\Desktop\Broadsword\worktrees\crash-sim\t-20261122-190342)
    Finished `release` profile [optimized] target(s) in 6.12s
     Running `C:\Users\MrComputer\Desktop\Broadsword\build\release\crash-sim.exe --body scenarios/validation_wall.bsn --pulse-report validation/test-a.pulse.csv --gravity 0 --duration 0.2 --dt 0.001 --substeps 10 --pulse-sensor sensor --pulse-crush-groups rear,front --pulse-cfc-shape 60 --pulse-cfc-int 180 --pulse-window 0.15 --pulse-threshold 20 --pulse-crush-tail 0.05 --setup-mass-tol 0.01 --setup-speed-tol 0.005 --margin-dv 0.06 --margin-peak 0.18 --margin-peak-time 0.022 --margin-duration 0.10 --margin-olc 0.15 --margin-static-crush 0.20`
pulse_ref=7504
pulse_channel=curve 94 FLRR XG G'S (RIGHT REAR SEAT CROSSMEMBER X)
setup mass_kg sim=1330.0 ref=1330.0 err=+0.0000 tol=0.0100 ok
setup speed_kmh sim=56.20 ref=56.20 err=-0.0000 tol=0.0050 ok
contact_s=0.0033
ground_contacts_in_window=0
metric dv_mps sim=17.1624 ref=16.1958 err=+0.0597 norm=0.9947 ok
metric peak_mps2 sim=510.5 ref=502.6 err=+0.0157 norm=0.0873 ok
metric peak_time_s sim=0.0464 ref=0.0314 err=+0.0150 norm=0.6818 ok
metric duration_s sim=0.0784 ref=0.0748 err=+0.0481 norm=0.4813 ok
metric olc_mps2 sim=299.7 ref=347.8 err=-0.1383 norm=0.9218 ok
metric static_crush_m sim=0.4162 ref=0.3508 err=+0.1865 norm=0.9325 ok
info static_crush_centroid_m sim=0.4137
info dyn_crush_m sim=0.5045 ref=0.5386
pulse_score=0.6832
pulse: ok
--- summary ---
run: ok (6.4 s)
```

PASS rule check, line by line: both `setup` lines end `ok` (quoted above); `ground_contacts_in_window=0`; `pulse_score=0.6832` equals the calibrated line `calibrated node-beam deviation area_m2=0.00007000000000110004 yield_strain=0.0024 damping_ratio=0.01500000336789605 score=0.6832` in `projects/crash-sim/validation/CALIBRATION.md`; `pulse: ok`. The doc says `run: ok` vs `run: FAILED` is not gated here; this one prints `run: ok (6.4 s)`. Result: PASS.

## 4.5 Step size (on A)

Command: the 4.4 command with `--substeps 20`, `> scratch/s6-4-5.txt 2>&1`. Raw output (`scratch/t-20261122-190342/s6-4-5-step8.txt`, 27 lines):

```
job 20261121-184333-broadsword-check: check broadsword (task t-20261121-011525) was interrupted (broadsword pid 19948 is gone); left alone, as it is not a job of this call's project and task: `broadsword recover` or a call for --task t-20261121-011525 deals with it
job 20261122-191102-broadsword-check: still running (broadsword pid 32444), left alone
job: run crash-sim (id 20261122-192633-crash-sim-run, log: C:\Users\MrComputer\Desktop\Broadsword\scratch\logs\jobs\20261122-192633-crash-sim-run.log)
source: worktrees/crash-sim/t-20261122-190342 (task t-20261122-190342)
checkpoint: refs/broadsword/checkpoints/20261122-192633-crash-sim-run (5916f446cd)
build lock: taken after waiting 0.0 s
build source: worktrees/crash-sim/t-20261122-190342 (packages: crash-sim)
    Finished `release` profile [optimized] target(s) in 0.01s
     Running `C:\Users\MrComputer\Desktop\Broadsword\build\release\crash-sim.exe --body scenarios/validation_wall.bsn --pulse-report validation/test-a.pulse.csv --gravity 0 --duration 0.2 --dt 0.001 --substeps 20 --pulse-sensor sensor --pulse-crush-groups rear,front --pulse-cfc-shape 60 --pulse-cfc-int 180 --pulse-window 0.15 --pulse-threshold 20 --pulse-crush-tail 0.05 --setup-mass-tol 0.01 --setup-speed-tol 0.005 --margin-dv 0.06 --margin-peak 0.18 --margin-peak-time 0.022 --margin-duration 0.10 --margin-olc 0.15 --margin-static-crush 0.20`
pulse_ref=7504
pulse_channel=curve 94 FLRR XG G'S (RIGHT REAR SEAT CROSSMEMBER X)
setup mass_kg sim=1330.0 ref=1330.0 err=+0.0000 tol=0.0100 ok
setup speed_kmh sim=56.20 ref=56.20 err=-0.0000 tol=0.0050 ok
contact_s=0.0033
ground_contacts_in_window=0
metric dv_mps sim=17.1613 ref=16.1958 err=+0.0596 norm=0.9935 ok
metric peak_mps2 sim=508.9 ref=502.6 err=+0.0126 norm=0.0697 ok
metric peak_time_s sim=0.0464 ref=0.0314 err=+0.0150 norm=0.6818 ok
metric duration_s sim=0.0785 ref=0.0748 err=+0.0488 norm=0.4876 ok
metric olc_mps2 sim=299.5 ref=347.8 err=-0.1388 norm=0.9253 ok
metric static_crush_m sim=0.4165 ref=0.3508 err=+0.1874 norm=0.9371 ok
info static_crush_centroid_m sim=0.4140
info dyn_crush_m sim=0.5048 ref=0.5386
pulse_score=0.6825
pulse: ok
--- summary ---
run: ok (0.2 s)
```

The comparison of the two runs, as the raw file `scratch/t-20261122-190342/s6-4-5-deltas-step8.txt` printed it (6 lines):

```
dv_mps sim10=17.1624 sim20=17.1613 rel 0.000064 tol 0.01 ok
peak_mps2 sim10=510.5 sim20=508.9 rel 0.003134 tol 0.03 ok
peak_time_s sim10=0.0464 sim20=0.0464 abs 0.000000 tol 0.002 ok
duration_s sim10=0.0784 sim20=0.0785 rel 0.001276 tol 0.02 ok
olc_mps2 sim10=299.7 sim20=299.5 rel 0.000667 tol 0.03 ok
static_crush_m sim10=0.4162 sim20=0.4165 rel 0.000721 tol 0.04 ok
```

PASS rule check against the doc's limits (0.2 x margin: dv 1%, peak 3%, duration 2%, OLC 3%, static crush 4%, peak time 2 ms absolute): every one of the six lines ends `ok`, and each `rel`/`abs` value is under its `tol` (largest is `peak_mps2` 0.003134 against 0.03). Result: PASS.

## 4.6a Frozen before B

Raw output (`scratch/s6-4-6a.txt`, 18 lines; the command is the one in the doc, run in the B-run task's worktree `worktrees/crash-sim/t-20261122-194242`):

```
freeze=6822ee71c25e72ce4e566a0de93ddb3cbaca8720
head=6822ee71c25e72ce4e566a0de93ddb3cbaca8720
head_is_freeze=yes
freeze_on_main=yes
surface_dirty_lines=0
at_freeze scenarios/validation_car.bsn
at_freeze scenarios/validation_wall.bsn
at_freeze scenarios/validation_car_b.bsn
at_freeze scenarios/validation_wall_b.bsn
at_freeze src/main.rs
at_freeze src/lib.rs
at_freeze Cargo.toml
at_freeze Cargo.lock
at_freeze validation/sweep.py
at_freeze validation/test-a.pulse.csv
at_freeze validation/test-a.meta
at_freeze validation/test-b.pulse.csv
at_freeze validation/test-b.meta
```

PASS rule check: 18 lines; `freeze=` and `head=` both `6822ee71c25e72ce4e566a0de93ddb3cbaca8720`; `head_is_freeze=yes`; `freeze_on_main=yes`; `surface_dirty_lines=0`; then 13 `at_freeze` lines and no `missing_at_freeze`. Result: PASS.

## 4.7 The single test B run

Command: the 4.4 command with `--body scenarios/validation_wall_b.bsn --pulse-report validation/test-b.pulse.csv`, `--substeps 10`, run once (broadsword job `20261122-194344-crash-sim-run`, checkpoint `a819254de6`), `> scratch/s6-4-7.txt 2>&1`. Raw output (`scratch/s6-4-7.txt`, 31 lines):

```
job 20261121-184333-broadsword-check: check broadsword (task t-20261121-011525) was interrupted (broadsword pid 19948 is gone); left alone, as it is not a job of this call's project and task: `broadsword recover` or a call for --task t-20261121-011525 deals with it
job 20261122-192645-broadsword-check: still running (broadsword pid 6556), left alone
job: run crash-sim (id 20261122-194344-crash-sim-run, log: C:\Users\MrComputer\Desktop\Broadsword\scratch\logs\jobs\20261122-194344-crash-sim-run.log)
source: worktrees/crash-sim/t-20261122-194242 (task t-20261122-194242)
checkpoint: refs/broadsword/checkpoints/20261122-194344-crash-sim-run (a819254de6)
build lock: taken after waiting 0.0 s
build source: worktrees/crash-sim/t-20261122-194242 (packages: crash-sim)
cleaned `crash-sim` in the shared build folder (it was last built from projects/crash-sim); its other packages and all dependencies were kept
   Compiling crash-sim v0.1.0 (C:\Users\MrComputer\Desktop\Broadsword\worktrees\crash-sim\t-20261122-194242)
    Finished `release` profile [optimized] target(s) in 4.57s
     Running `C:\Users\MrComputer\Desktop\Broadsword\build\release\crash-sim.exe --body scenarios/validation_wall_b.bsn --pulse-report validation/test-b.pulse.csv --gravity 0 --duration 0.2 --dt 0.001 --substeps 10 --pulse-sensor sensor --pulse-crush-groups rear,front --pulse-cfc-shape 60 --pulse-cfc-int 180 --pulse-window 0.15 --pulse-threshold 20 --pulse-crush-tail 0.05 --setup-mass-tol 0.01 --setup-speed-tol 0.005 --margin-dv 0.06 --margin-peak 0.18 --margin-peak-time 0.022 --margin-duration 0.10 --margin-olc 0.15 --margin-static-crush 0.20`
pulse_ref=7751
pulse_channel=curve 94 FLRR XG G'S (RIGHT REAR SEAT CROSSMEMBER X)
setup mass_kg sim=1377.0 ref=1377.0 err=+0.0000 tol=0.0100 ok
setup speed_kmh sim=56.10 ref=56.10 err=-0.0000 tol=0.0050 ok
contact_s=0.0033
ground_contacts_in_window=0
metric dv_mps sim=17.1768 ref=16.1451 err=+0.0639 norm=1.0650 FAIL
metric peak_mps2 sim=505.1 ref=501.2 err=+0.0077 norm=0.0428 ok
metric peak_time_s sim=0.0440 ref=0.0311 err=+0.0129 norm=0.5864 ok
metric duration_s sim=0.0782 ref=0.0795 err=-0.0164 norm=0.1635 ok
metric olc_mps2 sim=298.3 ref=321.4 err=-0.0718 norm=0.4788 ok
metric static_crush_m sim=0.4185 ref=0.4368 err=-0.0420 norm=0.2099 ok
info static_crush_centroid_m sim=0.4182
info dyn_crush_m sim=0.5164 ref=0.5749
pulse_score=0.4244
pulse: FAIL dv_mps norm=1.0650

error: process didn't exit successfully: `C:\Users\MrComputer\Desktop\Broadsword\build\release\crash-sim.exe --body scenarios/validation_wall_b.bsn --pulse-report validation/test-b.pulse.csv --gravity 0 --duration 0.2 --dt 0.001 --substeps 10 --pulse-sensor sensor --pulse-crush-groups rear,front --pulse-cfc-shape 60 --pulse-cfc-int 180 --pulse-window 0.15 --pulse-threshold 20 --pulse-crush-tail 0.05 --setup-mass-tol 0.01 --setup-speed-tol 0.005 --margin-dv 0.06 --margin-peak 0.18 --margin-peak-time 0.022 --margin-duration 0.10 --margin-olc 0.15 --margin-static-crush 0.20` (exit code: 1)
--- summary ---
run: FAILED (exit 1)
```

Recorded, not fixed: `pulse_score=0.4244` and `pulse: FAIL dv_mps norm=1.0650`. Every other gated metric line ends `ok`. The `error: process didn't exit successfully ... (exit code: 1)` and `run: FAILED (exit 1)` lines are cargo's report of the pulse FAIL exit.

## 4.6b On main, after the B-run task merged

Command: the doc's 4.6b block, unchanged, from the Broadsword root in Git Bash, `> scratch/s6-4-6b.txt 2>&1`. Raw output (`scratch/s6-4-6b.txt`, 41 lines: the 16 doc lines plus the 25 file names that the `surface_changed_since_freeze` line prints):

```
freeze=6822ee71c25e72ce4e566a0de93ddb3cbaca8720
freeze_on_main=yes
run_b_lines=1
run_b_head_is_freeze=yes
run_b_score_matches_4_7=yes
order: freeze
order: run_b
b_run_jobs=1
b_run_parent=6822ee71c25e72ce4e566a0de93ddb3cbaca8720
run_b_command_is_frozen=yes
surface_changed_since_freeze=25 rc=0
Cargo.lock
Cargo.toml
src/ai.rs
src/collide.rs
src/format.rs
src/input.rs
src/lib.rs
src/main.rs
src/menu.rs
src/net.rs
src/net/session.rs
src/pulse.rs
src/reference.rs
src/save.rs
src/settings.rs
src/settle.rs
src/solver.rs
src/solver_m5.rs
src/speed_profile.rs
src/statehash.rs
src/track.rs
src/trajectory.rs
src/vec3.rs
src/vehicle.rs
validation/sweep.py
first_calibration=d35012e1feb91aa0522a6a16e755ecbb1ba7905c
frozen_lines_changed=0 rc=0
v6_label_before_calibration=| V6 (*sourced*)
v6_confirmed_before_calibration=4
test_b_in_code=0
```

The doc's PASS rule is the whole file, line for line. Each line against it:

| line in the file | the doc wants | result |
|---|---|---|
| `freeze=6822ee71c25e72ce4e566a0de93ddb3cbaca8720` | `freeze=<F, 40 hex>` | ok |
| `freeze_on_main=yes` | same | ok |
| `run_b_lines=1` | same | ok |
| `run_b_head_is_freeze=yes` | same | ok |
| `run_b_score_matches_4_7=yes` | same | ok |
| `order: freeze` then `order: run_b` | same, in that order | ok |
| `b_run_jobs=1` | same | ok |
| `b_run_parent=6822ee71c25e72ce4e566a0de93ddb3cbaca8720` | `b_run_parent=<F>` | ok |
| `run_b_command_is_frozen=yes` | same | ok |
| `surface_changed_since_freeze=25 rc=0`, then 25 file names | `surface_changed_since_freeze=0 rc=0` | **FAIL** |
| `first_calibration=d35012e1feb91aa0522a6a16e755ecbb1ba7905c` | `first_calibration=<40 hex>` | ok |
| `frozen_lines_changed=0 rc=0` | same | ok |
| `v6_label_before_calibration=\| V6 (*sourced*)` | same | ok |
| `v6_confirmed_before_calibration=4` | 1 or more | ok |
| `test_b_in_code=0` | same | ok |

So 4.6b is a FAIL as the doc writes it, on that one line. The check was not edited and not re-run.

Cause (orchestrator, 2026-11-22, from task t-20261122-203503): the B-run checkpoint a819254 stored the CRLF working-tree bytes, because Broadsword's `crates/broadsword/src/git.rs` builds checkpoint trees with `git hash-object -w --no-filters --stdin-paths` and crash-sim has `core.autocrlf=true`. It is not a change to the frozen surface: every one of the 103 changed files differs from F only in line endings (`git diff --ignore-cr-at-eol F a819254` shows 0 differences), and a clean checkout of F under autocrlf, captured with the fixed method, gives F's own tree (119f541). The fix for future checkpoints is t-20261122-203503 (not merged when this was written). Commits cannot change, so 4.6b on this record can never pass, and the B run is not repeated (one B run only). The PASS rule is not met on this record.

What the coder looked at afterwards (read-only git commands, run once, not part of the doc's check): the 25 names are `Cargo.lock`, `Cargo.toml`, 22 files under `src/` and `validation/sweep.py`. The job's `checkpoint_commit` is `a819254de68731b5c950a63fe68c87014d2eed2f`, and its parent is F (the `b_run_parent` line). Two commands, outputs as printed:

- `git diff --stat 6822ee7 a819254de68731b5c950a63fe68c87014d2eed2f` in `projects/crash-sim` ends `103 files changed, 41396 insertions(+), 41395 deletions(-)`, and for `src/vec3.rs`, `Cargo.toml` and `validation/sweep.py` it printed `3 files changed, 773 insertions(+), 773 deletions(-)` (equal insertions and deletions per file).
- `git diff --ignore-cr-at-eol --name-only 6822ee7 a819254de68731b5c950a63fe68c87014d2eed2f -- 'scenarios/validation_*.bsn' src Cargo.toml Cargo.lock validation/sweep.py 'validation/test-*.pulse.csv' 'validation/test-*.meta'` printed no name (`wc -l` gave `0`). `git config core.autocrlf` in `projects/crash-sim` prints `true`.

That is what the output shows: with carriage returns at line ends ignored, the checkpoint's surface equals F's, and without ignoring them 25 surface files differ in every line. This record claims no more than that. It does not claim the B run was on the frozen bytes beyond what 4.6a's `surface_dirty_lines=0` (at the start of the run, in the task's worktree) and the `b_run_parent` line show; whether the doc's check should compare with line endings ignored is the orchestrator's call, and no frozen line, check or file was changed here.

## The grid result and the recorded deviation

From `projects/crash-sim/validation/CALIBRATION.md` (the m4 block) and `projects/crash-sim/docs/STAGE6-M4-M5.md` (lines 282 to 285):

- The grid has 140 points (`run A node-beam 1` to `run A node-beam 140`). Result: **0 ok, 60 FAIL, 80 refused**. Counts of `CALIBRATION.md` lines starting `run A node-beam`: `verdict=FAIL` 60, `verdict=ok` 0, `verdict=refused` 80. Five of the 60 FAIL lines have `score=none` (points 1 to 4 and 21; the doc says "beam strain over 0.5, the sim's own abort").
- Refused: 20 (points 61 to 80, front area 0.000374) were run and refused by the step limit ("limit 0.000093 s against h = 0.0001 s"); 60 (points 81 to 140, front area 0.000654 and up) were not run, because their rear stage area (4 x the front) is outside the per-stage range 0.00007 to 0.002.
- The grid's own choice, the lowest score: `grid_best node-beam 41 area_m2=0.0002139974961130158 yield_strain=0.001 damping_ratio=0 score=0.6456 edge verdict=FAIL`. Its test A verdict was FAIL on dv (norm 1.0274). No point of the grid passes test A.
- The recorded deviation, quoted from the file: `deviation: chosen from step 8's 558 scratch-sweep points (270+288), more than the doc's 140-point grid; the formal 140-point grid found no passing point; decided by the orchestrator 2026-11-22 because doc wording is the orchestrator's; no threshold, margin or test B change`, followed by `calibrated node-beam deviation area_m2=0.00007000000000110004 yield_strain=0.0024 damping_ratio=0.01500000336789605 score=0.6832`, then `freeze = 6822ee71c25e72ce4e566a0de93ddb3cbaca8720` and `run B node-beam substeps=10 freeze=6822ee71c25e72ce4e566a0de93ddb3cbaca8720 head=6822ee71c25e72ce4e566a0de93ddb3cbaca8720 score=0.4244 verdict=FAIL`.
- Range conflict, from the doc text: the per-stage area range (0.00007 to 0.002) and Lazlo's pre-approved front range (7e-5 to 0.004, rear 4 x front) do not agree; that is why 60 points were refused unrun. No frozen line was changed.

## The m4 table

All values are copied from the raw outputs above. Test A is 4.4 (substeps 10); test B is 4.7. Norm is error / margin; the margin passes at norm <= 1.

| metric | A sim | A ref | A norm | B sim | B ref | B norm | B verdict |
|---|---|---|---|---|---|---|---|
| dv_mps | 17.1624 | 16.1958 | 0.9947 | 17.1768 | 16.1451 | 1.0650 | FAIL |
| peak_mps2 | 510.5 | 502.6 | 0.0873 | 505.1 | 501.2 | 0.0428 | ok |
| peak_time_s | 0.0464 | 0.0314 | 0.6818 | 0.0440 | 0.0311 | 0.5864 | ok |
| duration_s | 0.0784 | 0.0748 | 0.4813 | 0.0782 | 0.0795 | 0.1635 | ok |
| olc_mps2 | 299.7 | 347.8 | 0.9218 | 298.3 | 321.4 | 0.4788 | ok |
| static_crush_m | 0.4162 | 0.3508 | 0.9325 | 0.4185 | 0.4368 | 0.2099 | ok |
| pulse_score | 0.6832 | | | 0.4244 | | | FAIL |

Calibrated front area: `area_m2=0.00007000000000110004`, the range floor 0.00007 (`frozen: range crush.area_m2 = 0.00007 0.002`).

## Verdict

Test B: **FAIL on dv** (`pulse: FAIL dv_mps norm=1.0650`, `pulse_score=0.4244`). The other five gated metrics are `ok`. The run was made once on the frozen files. 4.6b did not pass as written (one line, above).
