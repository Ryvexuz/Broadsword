# Roadmap

One row per node in `CAPABILITY_ROADMAP.md` Part 2. Checked by `python -I scripts/roadmap_status.py`, which compares name, stage and the Done-when line with the capability roadmap and verifies every evidence item in the repo. Per-node notes go in `docs/nodes/` (see `docs/nodes/README.md`).

Status words: `not started`, `in progress`, `done`. A status other than `not started` must cite evidence: `file:<path>`, `tag:<name>`, `commit:<sha>` or `commit:<repo>@<sha>`. `done` also needs a `tag:` item. The script reports a status whose evidence is missing or fails as UNVERIFIED and exits 1; it is never counted as done. Nothing is `done` yet.

Stage is the Broadsword stage(s) Part 4's mapping table assigns the node (Tier 0 means stage 1); `none` means the table does not name it.

| Node | Name | Stage | Status | Evidence | Done when |
| --- | --- | --- | --- | --- | --- |
| 0.1 | Build and repo discipline | 1 | in progress | `file:crates/broadsword/src/buildsrc.rs`; `file:crates/broadsword/src/cleanup.rs`; `file:evidence/stage-3-worktree-builds.md`; `tag:stage-1-done` | Fresh clone builds and tests in one command; CI runs on every commit; build and scratch folders auto-prune and never eat into the 20 GB reserve |
| 0.2 | Math library | 1 | in progress | `file:projects/render/src/math.rs`; `file:projects/crash-sim/src/vec3.rs`; `file:docs/nodes/0.2.md`; `file:projects/foundation/docs/vec.md`; `file:projects/foundation/docs/mat.md`; `file:projects/foundation/docs/quat.md`; `file:projects/foundation/docs/transform.md`; `file:projects/foundation/docs/aabb.md`; `file:projects/foundation/docs/intersect.md`; `file:projects/foundation/docs/identities.md`; `commit:projects/foundation@3822e61`; `commit:projects/foundation@f1e4d62`; `commit:projects/foundation@67124d5`; `commit:projects/foundation@50ebeb8`; `commit:projects/foundation@1c53973`; `commit:projects/foundation@f28c69a`; `commit:projects/foundation@072c70e`; `commit:projects/foundation@61ef2b3` | Full unit tests against reference values; fuzz tests confirm inverse/compose identities |
| 0.3 | Data structures and memory | 1 | in progress | `file:docs/nodes/0.3.md`; `file:projects/foundation/docs/slotmap.md`; `file:projects/foundation/docs/arena.md`; `file:projects/foundation/docs/spatial_hash.md`; `file:projects/foundation/docs/octree.md`; `file:projects/foundation/docs/bvh.md`; `file:projects/foundation/docs/bench.md`; `commit:projects/foundation@51698ac`; `commit:projects/foundation@0e09b04`; `commit:projects/foundation@af2c302`; `commit:projects/foundation@dd7dd7e`; `commit:projects/foundation@2aa74f1`; `commit:projects/foundation@0bb82c7` | Benchmarks beat the naive version; zero leaks under sanitizers |
| 0.4 | Job system | 1 | in progress | `file:docs/nodes/0.4.md`; `file:projects/foundation/docs/jobs.md`; `file:projects/foundation/examples/speedup.rs`; `commit:projects/foundation@e704ee9`; `commit:projects/foundation@ef42343` | Near-linear speedup to your core count on a synthetic workload |
| 0.5 | Serialization and asset format | 1 | in progress | `file:docs/nodes/0.5.md`; `file:projects/foundation/docs/serial.md`; `commit:projects/foundation@5698f5a`; `commit:projects/foundation@03b3095`; `commit:projects/foundation@b3b6a38` | Save/load round-trip is byte-identical; an old-version file still loads after a schema change |
| 0.6 | Command and undo system | 1 | in progress | `file:docs/nodes/0.6.md`; `file:projects/foundation/docs/undo.md`; `commit:projects/foundation@0bb59be` | 10,000 random edits followed by full undo returns exactly to the start state |
| 0.7 | Plugin/module system | 1 | in progress | `file:docs/nodes/0.7.md`; `file:projects/foundation/docs/plugin.md`; `commit:projects/foundation@4f4ab21`; `commit:projects/foundation@765eb17` | A module hot-reloads without restarting the app |
| 0.8 | Scripting and automation API | 1, 2 | in progress | `file:crates/broadsword/src/mcp.rs`; `file:evidence/stage-2-m1-mcp-server.md`; `tag:stage-2-done` | A script can create, edit and query anything the UI can |
| 0.9 | Logging, tracing, test harness | 1, 3 | in progress | `file:projects/render/tests/golden.rs`; `commit:projects/render@30c92c4`; `file:crates/broadsword/src/job.rs`; `file:docs/nodes/0.9.md`; `file:projects/foundation/docs/trace.md`; `commit:projects/foundation@3d7254b`; `commit:projects/foundation@076f217` | One frame's timings open in a trace viewer; golden-image tests flag regressions |
| 1.1 | Window and input | 1 | not started | - | Input latency is measured and logged; tablet pressure curve is correct (your stylus-testing work fits here) |
| 1.2 | GPU abstraction | 4 | not started | - | The same render and compute shader run on two backends |
| 1.3 | 2D vector renderer | 4 | not started | - | Passes an SVG conformance subset within pixel tolerance |
| 1.4 | Text and fonts | 4 | not started | - | Ligatures and mixed scripts render; a million-line document scrolls smoothly |
| 1.5 | UI framework and editor shell | 1 | not started | - | A dockable layout saves and restores; 60 fps with 20 panels open |
| 1.6 | Reflection and properties | 1 | not started | - | Any struct is editable in an auto-generated inspector, serializable and scriptable with no hand-written glue |
| 1.7 | Node graph framework | none | not started | - | A 500-node graph re-evaluates only dirty nodes; a subgraph saves as a reusable asset; two graph types (math, image) share one editor |
| 1.8 | Timeline framework | 9 | not started | - | One widget drives an animation track, an audio track and a video track |
| 1.9 | Media I/O | 9 | not started | - | Encode/decode round-trips; 4K H.264 plays in real time |
| 1.10 | Audio device layer | 9 | not started | - | Under 10 ms round trip; no dropouts over a one-hour run |
| 1.11 | Color management | 4 | not started | - | The same image matches a reference across every app |
| 2.1 | Raster image engine | 4 | not started | - | A 16k × 16k image with 50 layers stays interactive |
| 2.2 | Brush engine | 4 | not started | - | Stroke lag on your tablet measures no worse than Krita's |
| 2.3 | Mesh kernel | 5 | not started | - | A boolean on two 100k-triangle meshes comes out watertight; the modifier stack is fully non-destructive |
| 2.4 | Scene system | 5 | not started | - | 1M instances update at 60 fps; glTF and USD import and export |
| 2.5 | Real-time renderer | 4 | in progress | `file:projects/render/src/raster.rs`; `commit:projects/render@30c92c4` | Khronos glTF sample models match the reference renders |
| 2.6 | Animation core | 5 | not started | - | An imported character blends walk and run, with foot IK on slopes |
| 2.7 | Audio DSP engine | 9 | not started | - | 64 tracks with effects play in real time; a third-party plugin loads |
| 2.8 | Numerics core | 10 | not started | - | Matches LAPACK/SciPy outputs on a test suite within tolerance |
| 2.9 | 2D constraint solver and sketcher | 8 | not started | - | A 30-constraint sketch solves instantly and reports over- or under-constrained states |
| 2.10 | Code editor core | 3 | not started | - | A 100 MB file opens; Rust and LL get completion and go-to-definition |
| 3.1 | Rigid-body physics | 6 | not started | - | A 1,000-box stack stays stable; replays are deterministic |
| 3.2 | Soft body and deformables | 6 | in progress | `file:projects/crash-sim/src/solver.rs`; `commit:projects/crash-sim@2e6424c` | Your car crash simulator: a car hits a wall and deforms plausibly in real time |
| 3.3 | Fluids and smoke | 6 | not started | - | Dam-break benchmark looks right; a smoke plume renders |
| 3.4 | Destruction | 6 | not started | - | A wall shatters into rigid-body chunks |
| 3.5 | Path tracer | 5 | not started | - | Cornell box converges to the reference; the same material looks consistent in 2.5 and 3.5 |
| 3.6 | Procedural geometry | 5 | not started | - | A procedural building or terrain generator saves as a reusable asset (Houdini HDA equivalent) |
| 3.7 | Sculpting | 5 | not started | - | A 5M-polygon sculpt stays interactive |
| 3.8 | Texture painting and material authoring | 5 | not started | - | A high-to-low normal map bakes cleanly; a smart-material wear pass exports straight into your engine |
| 3.9 | Rigging tools | 5 | not started | - | A biped is rigged and a walk cycle animated entirely in-tool |
| 3.10 | B-rep CAD kernel | 8 | not started | - | Import a STEP file, fillet an edge, export a STEP that other CAD opens |
| 3.11 | Parametric feature tree | 8 | not started | - | Change one dimension and the whole part regenerates correctly |
| 3.12 | Assemblies and drawings | 8 | not started | - | A 10-part assembly moves correctly through its mates; drawings export to DXF |
| 3.13 | CAM | 8 | not started | - | G-code simulates cleanly in a third-party viewer |
| 3.14 | EDA | 8 | not started | - | A small board design passes a fab's online Gerber check |
| 3.15 | Compositor and color grading | 9 | not started | - | Key a green screen, grade it, and track a patch onto a moving surface |
| 3.16 | Nonlinear video editing | 9 | not started | - | Edit and export a 10-minute 4K piece with no dropped frames |
| 3.17 | Motion graphics | 9 | not started | - | Recreate an animated title sequence |
| 3.18 | DAW features | 9 | not started | - | Record and mix a multitrack song |
| 3.19 | FEM solver | 10 | not started | - | Cantilever-beam deflection is within 1% of the analytic answer |
| 3.20 | CFD | 10 | not started | - | Lid-driven cavity matches the Ghia benchmark |
| 3.21 | Multiphysics | 10 | not started | - | A thermal-expansion problem matches a published COMSOL tutorial result |
| 3.22 | Block-diagram simulation | 10 | not started | - | A PID controller holds a motor model on target, shown on a scope |
| 3.23 | Matrix language and notebook | 10 | not started | - | A MATLAB signal-processing tutorial reproduces end to end |
| 3.24 | GIS core | 10 | not started | - | Load OSM data and a DEM, reproject, and measure distances correctly |
| 3.25 | Debugger and profiler | 10 | not started | - | Debug both a Rust program and an LL program from the editor |
| 3.26 | Semantic code index | 10 | not started | - | A project-wide rename is safe; dead-code inspection is correct |
| 4.1 | Game runtime | 7 | not started | - | A small playable build runs on another PC |
| 4.2 | Game editor | 7 | not started | - | A level of EVOLUTION or Habit is built entirely inside Broadsword |
| 4.3 | DCC Studio | 5 | not started | - | Model, rig, animate and render a character from nothing |
| 4.4 | Image and Texture Studio | 5 | not started | - | A hero asset is painted, textured and exported |
| 4.5 | CAD Studio | 8 | not started | - | A real part is designed and then 3D printed or milled |
| 4.6 | Media Studio | 9 | not started | - | A finished video with a mixed soundtrack is delivered |
| 4.7 | Audio Studio | 9 | not started | - | A finished mixed track |
| 4.8 | Sim and Analysis Studio | 10 | not started | - | The whole benchmark suite passes from one app |
| 4.9 | Dev Studio | 10 | not started | - | You develop Broadsword inside Broadsword (the self-hosting milestone) |
| 4.10 | EDA Studio | 8 | not started | - | A board gets manufactured and works |
| 4.11 | Geo Studio | 10 | not started | - | A printed map is produced, and real terrain feeds into the game engine |
| 5.1 | Unified asset database | 12 | not started | - | Edit a texture in the Image Studio and it updates live in the engine and the DCC |
| 5.2 | Live link and round-trip | 12 | not started | - | A CAD part goes through FEM and lands in the game with its physical properties intact |
| 5.3 | AI agent layer | 2, 12 | not started | - | An agent builds a simple level from a text brief using only the public API |
| 5.4 | Studio UI shell | 11 | not started | - | Switch Model → Paint → Engine → Edit in one window without losing context |

## Notes on the in-progress rows

- 0.1: a one-command build, test and check exists (`broadsword build|test|check`) with disk guard and cleanup; there is no CI yet (no `.github/`), so the node is not done.
- 0.2: the shared math library is in `projects/foundation` (Vec2-4, Mat3/4, Quat, Transform, Aabb, ray and shape intersection, each with hand-worked reference tests; seeded fuzz of inverse and compose identities, `docs/identities.md`). The older renderer and crash-sim math copies are not yet replaced by it. No tag, so not done; see `docs/nodes/0.2.md`.
- 0.3: SlotMap, Arena, spatial hash, octree and BVH exist with tests equal to brute force and bench examples whose ratios beat the naive scan (octree 112.7x, BVH 74x closest-hit, per their docs). Sanitizers (Miri, ASan) were not run: they need a nightly toolchain and the slot map doc leaves that as an open question for Lazlo. See `docs/nodes/0.3.md`.
- 0.4: a fixed thread pool with `parallel_for` and `parallel_reduce`, and a task graph, exist. The coder's speedup run reached efficiency 0.965 at 16 workers on a busy board; the quiet-board run the done check calls for is not recorded. See `docs/nodes/0.4.md`.
- 0.5: a canonical text encoding, a canonical binary encoding with byte-identical round trip tests, and a schema-version migration registry exist. See `docs/nodes/0.5.md`.
- 0.6: `History` with undo, redo, merge, groups and a cap exists; a seeded 10,000-edit test undoes to a byte-identical start state. Not yet wired to any application. See `docs/nodes/0.6.md`.
- 0.7: a plugin host loads a cdylib through a versioned C ABI and hot reloads it, tested on Windows only (other platforms get an `Unsupported` stub); loading is gated to this repo's test plugins until the OS sandbox node is done. See `docs/nodes/0.7.md`.
- 0.8: the MCP server makes Broadsword's own tools agent-drivable; there is no embedded scripting language or UI to drive.
- 0.9: golden-image tests exist in the render repo, jobs have logs, and `foundation::trace` records scoped spans and exports Chrome Trace Event JSON. No recorded run opens one frame's timings in a trace viewer yet, and no structured logging crate exists. See `docs/nodes/0.9.md`.
- 2.5: a basic software rasterizer (Lambert, one shadow map); no PBR, IBL, TAA or glTF reference comparison.
- 3.2: a node-beam solver behind a `Solver` trait; no crash-into-wall deformation demo at real time yet.

## Open decisions for Lazlo (not applied)

From CAPABILITY_ROADMAP Part 4. The stage column above follows the current mapping, so both nodes show as the table has them.

1. Move 1.7 (node graph) and 1.8 (timeline) into Foundation (stage 1). Today 1.7 is in no stage and 1.8 is in stage 9.
2. Move 2.8 (numerics core) up to Graphics or earlier. Today only its extensions are in stage 10.
