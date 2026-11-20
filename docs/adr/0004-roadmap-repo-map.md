# ADR 0004: which repos the roadmap's tiers live in, and when ADR 0001's (d2) move happens

**Status: Proposed.** Nothing here is decided. Lazlo accepts, changes or rejects it. ADR 0001 stays Accepted as it is; this
ADR proposes what to do when its triggers fire, and one new trigger (T6 below) that comes from the product intent.

- Date: 2026-11-19. Board task `t-20261119-225028-2` (brief `[roadmap-ahead] ADR 0004`). First draft: it must be revised
  once the ra-<node> plans name a home for each node (see "Revision rule").
- Input documents: `docs/adr/0001-shared-core.md` (options a, b, c, d1, d2; triggers 1 to 5; follow-ups F5 to F7),
  `CAPABILITY_ROADMAP.md` (the node list, 71 nodes, 0.1 to 5.4), `DEVELOPMENT_PLAN.md:38-48`.
- Related, not decided here: `docs/SCALING.md` (being designed now). It plans one merge-queue shard per crate group. This
  ADR names crate groups only so the node table can be read; **how many shards, their queues and their limits belong to
  `docs/SCALING.md`**, and this ADR changes only if that document needs a different grouping.
- Product intent (Lazlo, 2026-11-20, quoted from `state/backlog/t-20261120-013957.md`, "Product intent"): one unified tool.
  While building a car model you switch between modeling, rendering and the physics engine in the same place, on the
  same live data, with no export or import. One shared core scene and data model (geometry, materials, structure, sim
  state) that every capability reads and writes; views and tools are layers on top. Picking a mode switches the active
  crate group; all groups work on the shared core. Groups are switchable plugins over a stable core API, loaded on
  selection; no mode depends on another mode's internals; the harness enforces that dependency rule. The core's
  stability, versioning and migration are critical path: any change to it is a cross-group change.

## The question

ADR 0001 chose separate repos with file formats as the contract, valid "for what the crash sim needs now". The roadmap has
71 nodes and most need each other's code: the mesh kernel needs math, the renderer needs the scene system, every
studio needs the asset database. The product intent adds something ADR 0001 did not weigh: modes must work on **live,
in-memory shared data**, not on exported files. Files-as-contract between repos cannot give that. Before the first brief
invents a path dependency between repos (which does not build in task worktrees, ADR 0001 "The fact that decides most of
this"), this ADR maps every node to a home and says when the move happens.

## Proposal in short

1. **Broadsword repo (B)** stays the platform: build, jobs, queue, tool registry, MCP server, agents, records. It does not
   hold workload code (math, scene, renderers).
2. **One workload workspace repo (W)** is ADR 0001's (d2): a Cargo workspace that holds the shared core and every
   crate group. Name to be chosen by Lazlo; "W" below. crash-sim and render become members (history kept: F5/F6).
3. Inside W, **core** is a small set of crates (math, data structures, job system, serialization, command/undo, plugin
   API, reflection, scene and data model, mesh kernel, numerics). **Crate groups** are one folder of crates per mode
   (model, render, engine/physics, cad, ...). A group depends on core and never on another group; the harness enforces it
   (see "Rules for the map").
4. Native libraries we wrap (FFmpeg, OpenEXR, OCIO, OpenCASCADE, PROJ/GDAL, CLAP/VST3) are `-sys`/wrapper crates owned by
   the one group that needs them first; if a second group needs one, it moves into core as a service crate.
5. **Move timing:** the F5 spike (scratch copies only) starts now and costs no queue time. F6 (the real move) happens in
   the first window where the merge queue is idle and either T2 or T6 has fired (below). Until then ADR 0001's (a)
   stands: copies per repo, formats as the contract.

## Rules for the map (hard rules kept)

- No absolute paths in any `Cargo.toml`, config or script (AGENTS.md:75). Every path is relative and stays inside one repo.
- No project builds against another repo's working tree. Cross-repo code reuse happens only inside W, by member path.
- No history rewrite or deletion: crash-sim and render stay as read-only repos after F6 (`git log --follow` must work
  in W); a move copies history in, it never edits the old repos.
- No move while the merge queue is working on either repo (playbook lesson 29), and no cross-repo change in one entry.
- crash-sim's default CSV stays byte-identical and render's golden images stay unchanged through any move of math
  (ADR 0001 "What must never happen").
- **Dependency rule (from the product intent):** group -> core is allowed; group -> group is not; core -> group is not. A
  mode talks to another mode only through core data (the scene and data model) and core services. To be enforced as a
  check in the harness (a `cargo metadata` walk that fails on a forbidden edge). Until that check exists the rule is
  labelled **not yet in force** (playbook lesson 28); the check itself is a task for `docs/SCALING.md` and the roadmap
  plans, not this ADR.
- Core API stability: every change to a core public item is a cross-group change. The core carries an explicit API
  version and a format/schema version (node 0.5 migrations); a core change must run every group's tests in one entry,
  which only a single workspace can do (ADR 0001, "Cross-repo change in the queue").

## Node -> home table (all 71 nodes, 0.1 to 5.4)

Homes: **B** = Broadsword repo. **W/core** = core crates in W. **W/<group>** = crate group of that name in W (proposed
names: `shell`, `media-io`, `image`, `model`, `render`, `physics`, `cad`, `eda`, `media`, `audio`, `sim`, `geo`, `dev`,
`game`). "now" = exists today in its own repo and moves in with F6. Groups are working names for the table; the
sharding plan in `docs/SCALING.md` may merge or split them.

### Tier 0, foundation

| Node | Home | Reason |
| --- | --- | --- |
| 0.1 Build and repo discipline | B | Already Broadsword itself (build, test, clean, disk guard). W adds nothing but its own CI config. |
| 0.2 Math library | W/core | First shared code. crash-sim `f64` and render `f32` Vec3 become one generic/two-scalar core crate; see "Order". |
| 0.3 Data structures and memory | W/core | Used by mesh kernel, physics, EDA, GIS. Depends only on math. |
| 0.4 Job system | W/core | Used by physics, CFD, renderer, media. Depends on nothing else. |
| 0.5 Serialization and asset format | W/core | The format and migration layer for the shared data model; the contract between sessions. |
| 0.6 Command and undo system | W/core | Every edit in every mode is a command on the shared model; must be one implementation. |
| 0.7 Plugin/module system | W/core | The mechanism that loads crate groups on mode selection; part of the stable core API. |
| 0.8 Scripting and automation API | W/core (API) and B (MCP/agent side) | The API over the core is core; Broadsword's MCP server and tool registry stay in B and call it. |
| 0.9 Logging, tracing, test harness | W/core | Golden-file and golden-image helpers every group uses; B keeps job logs. |

### Tier 1, engines and frameworks

| Node | Home | Reason |
| --- | --- | --- |
| 1.1 Window and input | W/shell | The editor shell; always loaded, not a mode. |
| 1.2 GPU abstraction | W/shell | Shared GPU resources for render, image, game; always loaded. |
| 1.3 2D vector renderer | W/shell | Used by UI, CAD drawings, EDA, GIS, motion graphics: shell service. |
| 1.4 Text and fonts | W/shell | Used by UI and by media, CAD, dev: shell service. |
| 1.5 UI framework and editor shell | W/shell | The shell itself. |
| 1.6 Reflection and properties | W/core | Auto-inspector, serialization and scripting over every core type; must be in core to see the data model. |
| 1.7 Node graph framework | W/shell | Framework used by shaders, procedural, compositing, audio, block diagrams; no mode-specific data. |
| 1.8 Timeline framework | W/shell | Framework used by animation, video, motion, DAW. |
| 1.9 Media I/O (wrap FFmpeg, OpenEXR) | W/media-io | A `-sys` wrapper group with heavy native build; consumers (image, media, audio) depend on its API only. |
| 1.10 Audio device layer | W/media-io | Native audio I/O wrapper; same reason as 1.9. |
| 1.11 Color management (wrap OCIO) | W/media-io | Native wrapper; used by image, render, media. |

### Tier 2, domain engines

| Node | Home | Reason |
| --- | --- | --- |
| 2.1 Raster image engine | W/image | Mode (image/texture). Uses 1.2, 1.7, 1.11. |
| 2.2 Brush engine | W/image | Same mode; tablet input comes from 1.1. |
| 2.3 Mesh kernel | W/core | Geometry is part of the shared scene model (the product intent names geometry first). Needed by model, physics (3.2), CAD, destruction: second consumer is certain. |
| 2.4 Scene system | W/core | The shared scene and data model itself. Highest stability requirement in the repo. |
| 2.5 Real-time renderer | W/render | Mode (render). Reads the scene; render (today) is the seed. |
| 2.6 Animation core | W/model | Mode (model/animate). Owns skeleton and curve types only if no other mode reads them live; if the physics or game mode must read skeleton state live, the type moves to core (decide in its ra plan). |
| 2.7 Audio DSP engine (wrap CLAP/VST3) | W/audio | Mode (audio). Native plugin hosting stays inside the group. |
| 2.8 Numerics core | W/core | Needed by physics, CAD, sim, CFD, EDA: not a mode. Fires T4 (see "Order"). |
| 2.9 2D constraint solver and sketcher | W/cad | Mode (CAD). Solver math from 2.8. |
| 2.10 Code editor core | W/dev | Mode (dev studio). |

### Tier 3, tools

| Node | Home | Reason |
| --- | --- | --- |
| 3.1 Rigid-body physics | W/physics | Mode (engine/physics). crash-sim ("now") joins this group. |
| 3.2 Soft body and deformables | W/physics | The crash sim's real home: node-beam/FEM on core meshes. crash-sim becomes this group's seed. |
| 3.3 Fluids and smoke | W/physics | Same mode; no other mode needs its internals. |
| 3.4 Destruction | W/physics | Same mode; reads core meshes, writes core scene. |
| 3.5 Path tracer | W/render | Same mode as 2.5, so materials match (its done check). |
| 3.6 Procedural geometry | W/model | Mode (model). Graph ops over core meshes. |
| 3.7 Sculpting | W/model | Mode (model). |
| 3.8 Texture painting and material authoring | W/image | Mode (image); reads and writes core materials, reads meshes, uses render for previews through core services, not render internals. |
| 3.9 Rigging tools | W/model | Mode (model/animate). |
| 3.10 B-rep CAD kernel (wrap OpenCASCADE) | W/cad | Native wrapper inside the CAD group; its B-rep result becomes core geometry through a core mesh/boundary type. |
| 3.11 Parametric feature tree | W/cad | Mode (CAD). Uses command system (0.6). |
| 3.12 Assemblies and drawings | W/cad | Mode (CAD). |
| 3.13 CAM | W/cad | Mode (CAD). |
| 3.14 EDA | W/eda | Own mode; different data (netlists, boards). |
| 3.15 Compositor and color grading | W/media | Mode (media). |
| 3.16 Nonlinear video editing | W/media | Mode (media). |
| 3.17 Motion graphics | W/media | Mode (media). |
| 3.18 DAW features | W/audio | Mode (audio). |
| 3.19 FEM solver | W/sim | Mode (sim and analysis). Meshing comes from core 2.3. |
| 3.20 CFD | W/sim | Mode (sim). |
| 3.21 Multiphysics | W/sim | Mode (sim). Couples 3.19 and 3.20 inside one group. |
| 3.22 Block-diagram simulation | W/sim | Mode (sim). Uses 1.7. |
| 3.23 Matrix language and notebook | W/sim | Mode (sim). |
| 3.24 GIS core (wrap PROJ, GDAL) | W/geo | Own mode and native wrapper. |
| 3.25 Debugger and profiler | W/dev | Mode (dev). |
| 3.26 Semantic code index | W/dev | Mode (dev). |

### Tier 4, studios (a studio is a named set of crate groups plus a layout, not new engine code)

| Node | Home | Reason |
| --- | --- | --- |
| 4.1 Game runtime | W/game | Mode. Depends on core and the physics/audio/render *interfaces through core*, not their internals. |
| 4.2 Game editor | W/game | Same group; play-in-editor runs the game group inside the shell. |
| 4.3 DCC Studio | W/model + W/render (selection) | A preset that loads the model and render groups over the same core scene: the product intent's own example. Studio code is a thin preset in W/shell. |
| 4.4 Image and Texture Studio | W/image (preset in W/shell) | Preset. |
| 4.5 CAD Studio | W/cad (preset in W/shell) | Preset. |
| 4.6 Media Studio | W/media + W/audio (preset in W/shell) | Preset. |
| 4.7 Audio Studio | W/audio (preset in W/shell) | Preset. |
| 4.8 Sim and Analysis Studio | W/sim (preset in W/shell) | Preset. |
| 4.9 Dev Studio | W/dev (preset in W/shell) | Preset; self-hosting means B and W both open here. |
| 4.10 EDA Studio | W/eda (preset in W/shell) | Preset. |
| 4.11 Geo Studio | W/geo (preset in W/shell) | Preset. |

### Tier 5, integration

| Node | Home | Reason |
| --- | --- | --- |
| 5.1 Unified asset database | W/core | Extends 0.5 and the scene model; one library every group reads. |
| 5.2 Live link and round-trip | W/core | Live data sharing across modes is the core's job (the product intent); no exports. |
| 5.3 AI agent layer | B (MCP, agent roles) over W/core 0.8 | Agents are Broadsword's domain; they drive W through the public scripting API only. |
| 5.4 Studio UI shell | W/shell | The one workspace and its mode switcher; also how crate groups load (with 0.7). |

Count check: tier 0 has 9 rows, tier 1 has 11, tier 2 has 10, tier 3 has 26, tier 4 has 11, tier 5 has 4; total 71.

## Order of the ADR 0001 triggers

ADR 0001 triggers: 1 a second project needs the sim or `.bsn` in-process; 2 a third Rust project needs the same `Vec3` or
parser; 3 a bug is fixed in one copy only; 4 a numerics core is needed by two projects; 5 a worktree path fix lands.

| Order | Node that fires it | Trigger | Why then |
| --- | --- | --- | --- |
| 1 | 0.2 Math library (first consumer beyond crash-sim and render: 0.3 or 2.3) | **T2** | Today there are two copies (crash-sim `f64`, render `f32`). The first new crate that needs `Vec3` is the third user. Also the first moment a brief would invent a path dependency. |
| 2 | 0.5 Serialization (a second reader of the shared format, e.g. 2.4 or 5.1) | **T2** (same parser, different consumer) | Formats stay a contract until a model must load in-process in two groups. |
| 3 | 2.3 Mesh kernel and 2.4 Scene system | T2 again, and **T6** | Every mode reads them live; this is the product intent's core. |
| 4 | 2.8 Numerics core (second consumer: 3.1/3.2 and one of 2.9, 3.19, 3.20, 3.22) | **T4** | crash-sim keeps its own solver until a second project needs numerics (ADR 0001 on 2.8). Fires when the second consumer's plan starts. |
| 5 | 4.x studio presets and 5.1, 5.2 | **T1** (the Studio needs the sim in-process) | Fully reached only at stage 11; by then W already exists. |

### T6, proposed new trigger (needs Lazlo's OK because ADR 0001 is Accepted)

**T6: the product intent is adopted.** Modes switching live over one shared core data model needs in-memory shared
crates, which files-as-contract cannot give, and every roadmap node above 0.2 then depends on core. If Lazlo accepts this
ADR, T6 is considered fired for **planning** (briefs may assume W) but the **move** still waits for the rule below.

## When the (d2) migration happens

- **F5 (spike, ADR 0002 + scratch copies):** may start now. It only touches `docs/adr/0002-workload-workspace.md` and
  copies in `scratch/`, so it never blocks the queue. Its done check is ADR 0001's F5, extended with the new
  requirement that a group-to-group dependency is rejected by a check and a group-to-core dependency builds in a task
  worktree.
- **F6 (the real move) starts when all of these hold:**
  1. F5 passed and Lazlo accepted this ADR (or at least approved W as the home).
  2. T2 or T6 has fired for a task that is next in line (so the move is not waiting for a reason).
  3. The merge queue is idle: no running task and no open queue entry in crash-sim, render or the move's own tasks
     (playbook lesson 29, 30). Open crash-sim and render tasks are finished or re-pointed first; crash-sim had 12 open
     and render 2 open when ADR 0001 was written, so check the board again, do not reuse those numbers.
  4. Nothing in the move rewrites or deletes history; every Git command beyond a plain commit is explained in `LAB_LOG.md`.
- **If the queue is never idle:** pause starting new crash-sim and render tasks for the window, rather than moving
  mid-queue. `docs/SCALING.md` should say how a shard drains; this ADR only requires that W's move waits for it.
- **Before F6, no brief may add a path dependency between repos.** A brief that needs shared code before F6 either copies
  the few lines (ADR 0001 option a) or waits for F6.

## What this changes in ADR 0001, if accepted

- (a) stays in force until F6; then (d2) takes over for crash-sim and render. The "never (b) or (c)" conclusion stands: they
  still cannot build in task worktrees.
- F7 (core `Vec3` as a workspace member) is split by the table above into the core crates 0.2 to 0.9, 1.6, 2.3, 2.4, 2.8,
  each its own task after F6, in the order of the trigger table.
- The follow-ups F1 to F4 for nb-40, nb-37, nb-42 and nb-39 are unchanged until F6.

## Open questions for Lazlo (not decided here)

1. Name of the W repo (the table uses W).
2. Whether the model/animate boundary (2.6 skeleton types) belongs in core. The table keeps it in `model` unless a
   second mode must read it live; each ra plan decides.
3. The exact crate-group names and count: left to `docs/SCALING.md`, because they set the shard boundaries.
4. How the dependency rule is enforced (harness check, Cargo feature flags, or a lint on `cargo metadata`); that is a
   task, not part of this ADR.

## Revision rule

When each `ra-<node>` plan lands, it names its node's home. Where it differs from the table, change the table row and
add a dated line here; do not leave the plan and the table disagreeing. Status stays Proposed until Lazlo accepts it.

## What must never happen, whichever map is chosen

- An absolute path, or a build of one repo against another repo's working tree, to "bridge" the time before F6.
- A history rewrite or deletion in crash-sim, render or Broadsword.
- A move with the merge queue working on an involved repo.
- Any change to crash-sim's default CSV bytes or render's goldens because code moved.
- A group depending on another group's internals, or core depending on a group.
