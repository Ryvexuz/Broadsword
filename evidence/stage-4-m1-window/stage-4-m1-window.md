# Stage 4 milestone 1, "the starter cube is visible and bouncing in a window": on-screen evidence (audit finding A1)

Date: 2026-11-19, about 20:05 UTC, on EVO-X3. Orchestrator (Opus 5.5) run, evidence only, no code change.

**Binary:** `broadsword run render -- cargo build --offline --release --example replay` (render master 9e1060b at the Stage 4 audit snapshot; the replay example is unchanged since), `run: ok`. Then `build/release/examples/replay.exe projects/render/tests/fixtures/trajectory.csv` with NO `--frames` and NO `--present`: the default mode, which opens a normal window (CreateWindowExW at CW_USEDEFAULT, not the -32000 off-screen placement) and plays the crash-sim trajectory in real time.

**Capture:** `capture.ps1` (in this folder) waited for the window, brought it to the front, and copied ONLY the window's own rectangle from the screen at 0.3, 0.7, 1.2, 2.0 and 3.5 s, then closed it with CloseMainWindow. Log (`capture.log`):
- `pid=17000 handle=1837290 title='Broadsword render: replay' visible=True`
- window rectangle (208,208)-(864,727), 656x519, the same in all five captures (on screen, not off screen)
- `closed by CloseMainWindow=True, exit code 0`

**What the images show (checked by eye):**
- `replay-t0.3s.png`: the cube high above the ground grid, tilted, falling.
- `replay-t0.7s.png`: the cube just above the ground.
- `replay-t1.2s.png`: after contact, the cube rebounded and is tipping up on one edge (the bounce).
- `replay-t2.0s.png`, `replay-t3.5s.png`: the cube resting flat on the ground.

So the cube is visible in an on-screen window and bounces, which is milestone 1's done check in its own words. Lazlo may also watch it himself: run the same command; the window closes with its close button.
