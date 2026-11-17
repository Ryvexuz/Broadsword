# BROADSWORD — Development Plan

Nov 17, 2026 · @Lazlo

## How this plan works

This plan turns the 12 stages into an order of work. The rules for *how* agents work (roles, disk rules, file claims, settings, reports) are in **BROADSWORD — Development Instructions**, kept in the repo as `AGENTS.md`. This doc covers *what* gets built and in what order.

Three ideas shape the whole plan:

- **The crash sim is the spine.** Each stage first serves the crash sim, which later grows into a racing game. A stage starts only when a real project needs it.
- **Small first, then grow.** Every area starts as a version small enough to finish and understand, then grows through milestones. Matching Unreal or Blender is not the goal; owning and adjusting every piece is.
- **Every milestone teaches something about AI development.** Each stage lists the lesson to watch for. Record what you see in `LAB_LOG.md`.

**Gate rule:** a milestone is done only when its done check passes and the evidence is saved. The next milestone doesn't start until then.

## Build order

The stages keep their original numbers, but they're built in the order the crash sim needs them:

| Order | Stage | Why here |
| --- | --- | --- |
| 1 | 0. Setup | Repo, rules and settings before any agent writes code |
| 2 | 1. Foundation | Job runner and resource monitor protect everything after |
| 3 | 2. AI Integration | Agents need tools before they can build anything big |
| 4 | 3. Autonomous Development | Parallel agents, safely, before the workload grows |
| 5 | 4. Graphics (v1) | You need to see the crash sim |
| 6 | 6. Physics | The crash sim itself |
| 7 | 7. Games | The crash sim becomes a racing game |
| 8 | 5. 3D Creation | Make and edit your own car models |
| 9 | 4. Graphics (v2) | Better lighting, materials, offline renders |
| 10 | 11. Studio | One window once there's enough to host |
| 11 | 9. Media | Record, edit and render crash replays |
| 12 | 8. Engineering and CAD | Precise parts, a new kind of geometry |
| 13 | 10. Advanced | Whatever real need comes up |
| 14 | 12. Integration | End-to-end workflows across everything |

## Shared core

Most stages reuse the same base. It's built in Stage 1 and grows only when a stage needs more:

- **Math:** vectors, matrices, transforms, quaternions
- **Geometry:** meshes, curves, bounding boxes, spatial queries
- **Scene:** objects, their relationships and their properties
- **GPU layer:** wgpu
- **Job system:** parallel work with priorities and cancellation
- **File format:** Broadsword's own format for saving everything, versioned from day one
- **Undo/redo:** every change is a reversible command

## Stage 0 — Setup

**Goal:** the repo exists and the rules load before any code is written.

1. Project folder, `AGENTS.md`, a `CLAUDE.md` pointer, `broadsword.toml`, and the `scratch/`, `build/` and `shared/` folders with `.gitignore`.
2. TokenWise kept, task-router removed. Subagent role files in `.claude/agents/`.
3. `git init` and a first commit.

**Done check:** a fresh Claude Code session in the folder can state the disk floor, the agent cap and the first task without being told.

**Lesson focus:** does the rules file actually change agent behavior, or does it get ignored?

## Stage 1 — Foundation

**Goal:** a core that runs jobs and protects the machine.

1. **Resource monitor:** disk, RAM and CPU checks, the disk floor, hourly cleanup, pause and alert. *Done:* fill scratch past the floor on purpose; cleanup runs and agents pause.
2. **Starter crash sim:** a cube of nodes and beams that falls and bounces, in its own repo. *Done:* it builds, runs headless and prints the cube's height over time.
3. **Job runner and test runner:** one command builds and tests any project. *Done:* one command builds and tests the starter crash sim.
4. **Recovery:** Git checkpoints before risky jobs. *Done:* kill Broadsword mid-job; on restart it resumes or rolls back and nothing is lost.
5. **Tool registry:** every capability is a named tool with inputs and outputs. *Done:* list all tools from the command line.

**Lesson focus:** how well agents build infrastructure with no visible output. Watch for placeholder code that "passes."

## Stage 2 — AI Integration

**Goal:** any AI can connect, see every tool and use it.

1. **MCP server** exposing the tool registry. *Done:* Claude Code connects and lists the tools.
2. **Headless mode:** Broadsword runs as a background service. *Done:* a build runs through MCP with no window open.
3. **Second provider:** a different cloud model connects through the same server. *Done:* both models run the same build.
4. **Local model (optional, later):** *Done:* the same build through a local model.

**Lesson focus:** how well an AI understands tools from their descriptions alone. Rewrite descriptions and measure the difference.

## Stage 3 — Autonomous Development

**Goal:** agents work in parallel without stepping on each other.

1. **Task board:** tasks with file claims, status and owner. *Done:* two tasks with overlapping claims never run at once.
2. **Worktree per agent:** created and removed automatically. *Done:* worktrees disappear after merging.
3. **Reviewer step and merge queue.** *Done:* a planted bug gets caught before merging.
4. **Scale-up test:** raise `max_agents` to 3, then 4. *Done:* a full session at the higher cap with clean merges and the disk under control.

**Lesson focus:** the right number of agents for your machine and budget. This is where the last orchestrator failed, so measure collisions, disk use and wasted work at each cap.

## Stage 4 — Graphics and Rendering

**v1, for the crash sim:**

1. **Debug renderer:** lines, points and a fly camera. *Done:* the starter cube is visible and bouncing in a window.
2. **Basic rasterizer:** shaded meshes, one light, shadows. *Done:* a car-shaped mesh renders at 60 fps or better.

**v2, after the racing game:**

3. **Physically based materials:** metal, paint, glass, rubber. *Done:* side-by-side renders look plausibly different.
4. **Large environments:** level of detail and culling. *Done:* a full track renders at a steady frame rate.
5. **Offline path tracer:** *Done:* a still render of a crash with realistic light.

**Lesson focus:** agents can't see the screen easily. Use screenshot comparisons ("golden images") so visual work can be verified automatically.

## Stage 6 — Physics and Simulation

**Goal:** the crash sim, made more realistic over time.

1. **Stable node-beam solver:** fixed timestep, substeps, damping, behind a swappable interface. *Done:* the cube settles without jitter or explosions.
2. **Deformation and breaking:** *Done:* a box hits a wall and crumples permanently.
3. **Drivable car:** your own car file format, wheels and suspension. *Done:* drive a car around and crash it.
4. **Validation against real data:** compare against published IIHS or NHTSA crash tests. *Done:* one test matches within a stated margin.
5. **Higher-fidelity solver:** soft bodies or FEM in the crash zone. *Done:* it measurably beats the node-beam solver in the validation test.

**Lesson focus:** agents tuning toward a real-world measurement, not just passing tests.

## Stage 7 — Games and Applications

**Goal:** the crash sim becomes a racing game, then a general game toolkit.

1. **Racing basics:** tire grip, a track, lap timing. *Done:* a timed lap you can drive.
2. **AI drivers:** *Done:* three AI cars finish a race without crashing into walls.
3. **Game shell:** menus, settings, saving. *Done:* a standalone executable a friend can run.
4. **Multiplayer:** *Done:* two machines race each other.
5. **Second game:** a small game in another genre on the same toolkit. *Done:* it ships without changes to the engine core.

**Lesson focus:** whole-product integration, where many parts must work together and "tests pass" stops being enough.

## Stage 5 — 3D Creation

**Goal:** make and edit your own car models inside Broadsword.

1. **Mesh editing:** select, move and extrude with undo. *Done:* edit a car body and save it.
2. **Car rigging:** attach a visual mesh to the physics skeleton. *Done:* a hand-made car crashes and deforms correctly.
3. **Texturing and materials:** *Done:* paint a car and see it in the renderer.
4. **Procedural generation:** *Done:* generate a track or a city block from parameters.
5. **Sculpting and animation:** *Done:* sculpt a detail and animate a simple rig.

**Lesson focus:** agents working on large shared state (the scene and undo history) without corrupting it.

## Stage 11 — Studio

**Goal:** one window that hosts every module. Everything still works without it.

1. **Shell:** a viewport, a project panel and a log panel. *Done:* open a project and run the crash sim inside Studio.
2. **Live progress:** agent status, disk use and the task board. *Done:* watch a session in progress.
3. **Editors as panels:** mesh editor, track editor, settings. *Done:* build and drive a car without leaving Studio.
4. **Remote and mobile view:** *Done:* check progress from your phone.

**Lesson focus:** verifying interface work. Agents are weakest when "correct" means "looks and feels right."

## Stage 9 — Media Creation

**Goal:** turn crashes into videos and images.

1. **Replay recorder:** *Done:* record a crash and play it back from any camera.
2. **Video export** through FFmpeg. *Done:* a crash replay exported as an MP4.
3. **Image editor:** layers and basic tools. *Done:* make a thumbnail from a render.
4. **Timeline editor and audio:** *Done:* a short edited crash compilation with sound.
5. **AI-assisted generation:** *Done:* generate a texture or sound effect and use it in the game.

**Lesson focus:** cross-language work (Rust calling FFmpeg's C code) and where agents break at that boundary.

## Stage 8 — Engineering and CAD

**Goal:** precise, dimension-driven parts, using a C++ kernel (OpenCascade) from Rust.

1. **Kernel bridge:** *Done:* create a box and a cylinder through OpenCascade from Rust.
2. **Sketch and extrude with constraints:** *Done:* change one dimension and the part updates.
3. **Part to physics:** *Done:* a CAD-designed bumper bracket used in a crash test.
4. **Export for manufacturing:** STEP and STL. *Done:* a file that opens in another CAD tool.

**Lesson focus:** agents working with a large, unfamiliar C++ library through bindings.

## Stage 10 — Advanced Capabilities

**Goal:** add areas only when a real need appears. Candidates: data analysis of crash results, GIS for real-world track maps, robotics simulation, documents.

Each one starts with a one-paragraph need statement and a done check written before work begins.

**Lesson focus:** whether agents can scope a vague goal into a finishable milestone.

## Stage 12 — Integration and Optimization

**Goal:** everything works together in real workflows.

1. **The full pipeline:** model a car (5), crash it (6), render it (4), export the video (9), all from Studio (11). *Done:* the pipeline runs start to finish with no manual fixes.
2. **Performance pass:** profile and fix the slowest step in each workflow. *Done:* measured before and after numbers.
3. **Gap list:** every missing feature found during real use goes into `IDEAS.md` and gets ranked.

**Lesson focus:** looking back over the whole lab log and turning it into your own playbook for AI development.

## Checkpoints along the way

At the end of each stage, before starting the next:

- Read back `LAB_LOG.md` for that stage and write three lessons at the top.
- Adjust `AGENTS.md` if a rule failed or got in the way.
- Check whether the agent cap, roles or models should change.
- Tag the stage in Git, for example `stage-1-done`.
