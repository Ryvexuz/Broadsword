# Stage 4 v1 (rendering) done: evidence

Date: 2026-11-19, EVO-X3 (Windows 11, 111.6 GB visible RAM). Orchestrator: Claude Opus 5.5. Render repo: master `9e1060b` (the audited snapshot; render master is still at that commit). Broadsword main at the time of this file: `c08a4dd` and later.

## Done checks (DEVELOPMENT_PLAN.md, Stage 4 v1), answered in their own words

### Milestone 1: "the starter cube is visible and bouncing in a window"

**Met.** Before 2026-11-19 every recorded run was headless or off screen (audit finding A1, REAL BLOCKING). On 2026-11-19 the replay viewer was run in its default mode, a normal on-screen window, playing the crash-sim trajectory in real time:
- `broadsword run render -- cargo build --offline --release --example replay` then `build/release/examples/replay.exe projects/render/tests/fixtures/trajectory.csv` (no `--frames`, no `--present`).
- The window: title `Broadsword render: replay`, `visible=True`, on-screen rectangle (208,208)-(864,727), closed by CloseMainWindow with exit code 0 (`stage-4-m1-window/capture.log`).
- Window-only captures (`stage-4-m1-window/replay-t*.png`): 0.3 s the cube high and falling; 0.7 s just above the ground; 1.2 s rebounded and tipping on one edge (the bounce); 2.0 s and 3.5 s resting on the ground. Details: `stage-4-m1-window/stage-4-m1-window.md`.

### Milestone 2: "a car-shaped mesh renders at 60 fps or better"

**Met, re-measured on this PC at the audited commit** (audit finding A2: the 2026-11-18 numbers existed only in `scratch/`):
- `broadsword run render -- cargo run --offline --release --example car -- --frames 600`: `car: 600 frames at 640x480, shadow map 1024x1024, 410 triangles (408 cast shadows, 193 drawn in the main pass)`, `car: average frame time 6.036 ms = shadow pass 4.080 + main pass 1.936 + other 0.020` (`stage-4-m2-car-frames.log`).
- Through the real window path: `... --example car -- --present 300`: `car: average 6.045 ms per frame = shadow pass 4.055 + main pass 1.947 + blit 0.019 + other 0.024` (`stage-4-m2-car-present.log`).
- 6.05 ms per frame is about 165 frames per second, against the 16.6 ms (60 fps) target.

## Stage-end audit

- Claude auditor (Fable 5.2, high effort; it built none of Stage 4): `stage-4-v1-audit-claude.md`, 28 findings.
- Second sweep (gpt-6.2-astra via Codex, two runs, same snapshot): `stage-4-v1-audit-astra-run1.md` (220), `stage-4-v1-audit-astra-run2.md` (228).
- Independent verdicts (five Opus 5.5 high judges by section, merged by Sonnet 5.5 high): `stage-4-v1-audit-verdicts.md`. Every one of 477 finding ids appears once.
- REAL BLOCKING root causes, both resolved before this tag:
  1. **A1** (milestone 1 never shown on screen): resolved by the on-screen run above.
  2. **A6** (the Codex shadow reviewer could read files by absolute path; confirmed by a canary): mitigated by `cfadd9e` (shadow off), fixed by `178cae0` (codex is offered no tools), proven by `scripts/shadow_canary.py` from main on gpt-5.6-sol and gpt-6.1-sol (`stage-4-a6-canary/canary-main-*.txt`: every request `offers tools: []`, the planted PIN never recovered, the real call refused); shadow back on in `479a18a`'s merge.
- REAL SERIOUS root causes (8) are listed with proposed fix tasks in `stage-4-v1-audit-verdicts.md` section 6; they do not block the tag under the Review rule (blocking findings only) and are carried as tasks.

## Test results at close

- `broadsword test render` at `9e1060b`: `test: ok — 81 passed, 0 failed` (audit run).
- Broadsword: the merge queue's check on main passed for every merge today; the last full suite on main reported `test: ok — 422 passed, 0 failed`.

## Tag

`stage-4-v1-done` on Broadsword main (this commit), and on render master `9e1060b` as `stage-4-v1-done`. Pushing tags is publishing: it waits for Lazlo's OK.
