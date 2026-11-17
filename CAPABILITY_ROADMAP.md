# Broadsword Long-Term Capability Roadmap

Nov 17, 2026 · @Lazlo

## How to read this

Thirty tools sounds like thirty projects. It isn't. Break them down and they share roughly a dozen underlying engines. A **node graph** powers Houdini, Substance, Simulink, Unreal Blueprints, Blender's shader and geometry nodes, and Resolve's Fusion page. A **timeline** powers Premiere, Resolve, REAPER, After Effects and every animation editor. A **2D constraint solver** powers AutoCAD, SolidWorks and FreeCAD sketches, and KiCad. A **numerics core** powers physics, FEM, CFD, MATLAB and Simulink. Build each shared engine once and each "tool" becomes a front end on top of it.

The four parts:

1. **Component inventory**: what each of the 30 tools actually contains.
2. **Compounding hierarchy**: those components sorted into tiers, where each tier depends only on the tiers below it. Every node has a **Done when** endpoint an agent can be tested against.
3. **Coverage map**: for each original tool, the nodes you'd need for your own version of it.
4. **Broadsword alignment**: how this maps onto your 12 stages, plus a rough order over several years.

Ground rule: "your own" doesn't have to mean "from scratch" everywhere. Some nodes (a B-rep CAD kernel, video codecs, map projections) are decades of specialist work. Wrapping a proven open-source library behind *your* interface still gives you something you control and can replace later. Those nodes are marked **\[wrap first\]**.

## Part 1: Component inventory

### Game engines

| Tool | Core components |
| --- | --- |
| Unreal Engine | Renderer (Nanite virtualized geometry, Lumen global illumination, material editor), Blueprints visual scripting, C++ gameplay framework (actors, components, game modes), Chaos physics and destruction, animation (Anim Blueprints, Control Rig, retargeting), Niagara VFX, landscape and foliage, World Partition streaming, PCG framework, Sequencer cinematics, MetaSounds audio, networking and replication, AI (behavior trees, navmesh, EQS), UMG UI, Live Coding, cooking and packaging per platform |
| Unity | GameObject and component model, DOTS/ECS, C# scripting with hot reload, URP/HDRP render pipelines, Shader Graph, VFX Graph, 3D and 2D physics, Mecanim animation, Timeline, Cinemachine, UI Toolkit, Addressables asset system, Netcode, NavMesh, Profiler, multi-platform build pipeline, package manager |
| Godot | Scene tree of nodes, GDScript plus C# and GDExtension, dedicated 2D engine (tilemaps, 2D physics), 3D renderer (Forward+, Mobile, Compatibility), signals, AnimationPlayer and AnimationTree, editor built with its own UI toolkit, resource system, export templates, high-level multiplayer |

### 3D content creation

| Tool | Core components |
| --- | --- |
| Blender | Polygon modeling and modifier stack, sculpting (dyntopo, multires), UV editing, Geometry Nodes, shader nodes, Cycles path tracer, EEVEE real-time renderer, armatures and rigging, keyframes and graph editor, Grease Pencil 2D animation, sims (rigid, cloth, fluid, smoke), compositor, video sequencer, texture painting, camera and motion tracking, Python API and add-ons |
| Autodesk Maya | Polygon and NURBS modeling, rigging (deformers, constraints, HumanIK), character animation (graph editor, time editor), dependency-graph evaluation engine, Bifrost procedural sims, nCloth, nParticles, XGen hair, Arnold renderer, MEL and Python, USD pipeline |
| Houdini | Procedural node contexts (SOPs geometry, DOPs dynamics, VOPs, COPs compositing, CHOPs channels, LOPs/Solaris for USD), VEX language, Vellum (cloth, soft body, hair), FLIP fluids, Pyro fire and smoke, RBD destruction, Karma renderer, digital assets (HDAs) that package tools for other apps, PDG/TOPs task scheduling |
| 3ds Max | Modifier-stack modeling, splines, archviz tooling, Arnold, Particle Flow, Biped/CAT rigging, MaxScript, scene states and render setup |

### CAD and engineering design

| Tool | Core components |
| --- | --- |
| SolidWorks | Parametric parts (sketch to feature tree), constraint sketcher, assemblies with mates, drawings with GD&T, sheet metal, weldments, surfacing, configurations and design tables, built-in FEA and motion studies, PDM, Parasolid kernel |
| Fusion 360 | Parametric, direct and T-spline freeform modeling, assemblies with joints, CAM toolpaths (milling, turning, additive), generative design, simulation, electronics/PCB, cloud versioning and collaboration, rendering |
| AutoCAD | 2D drafting (lines, arcs, polylines, hatches), precision input (object snaps, coordinates), layers, blocks and xrefs, dimensions and annotation, paper-space layouts and plotting, DWG/DXF formats, basic 3D solids, AutoLISP and .NET API, sheet sets |
| FreeCAD | OpenCASCADE kernel, workbench architecture (Part, PartDesign, Sketcher, Assembly, TechDraw, FEM, CAM, BIM), constraint sketcher, spreadsheet-driven parameters, Python throughout |

### 2D imaging and texturing

| Tool | Core components |
| --- | --- |
| Photoshop | Layers and masks, selection tools, brush engine, adjustment layers, filters, color management (ICC, CMYK), smart objects (non-destructive), content-aware and generative fill, text engine, PSD format, actions and scripting |
| GIMP | Layer, channel and path model, GEGL node-based processing core, selection tools, filters, Python and Script-Fu plug-ins, broad format support |
| Krita | Multiple brush engines with pressure and tilt, stabilizers, frame-by-frame animation timeline, vector layers, wrap-around and mirror tools, color management, resource bundles |
| Substance 3D Painter | Texture painting in the 3D viewport across UV sets, layered PBR and smart materials, procedural masks and generators (curvature/AO-driven wear), mesh-map baking (normal, AO, curvature, thickness, ID), particle brushes, real-time PBR viewport, per-engine export presets |

### Video, compositing, motion graphics

| Tool | Core components |
| --- | --- |
| DaVinci Resolve | Edit timeline (Cut and Edit pages), node-based color grading (primaries, curves, qualifiers, power windows, tracking), Fusion node compositing, Fairlight audio post, media management and proxies, render/delivery queue, GPU image pipeline, codec support |
| Adobe Premiere Pro | Multi-track nonlinear timeline, trim tools, effects and transitions, Lumetri color, audio mixing, captions and transcription, proxies, Dynamic Link to After Effects, export through Media Encoder |
| After Effects | Layer compositing on a timeline, keyframes and graph editor, JavaScript expressions, masks and rotoscoping, point and camera tracking, shape layers and text animators, 2.5D/3D layers, effects plugin ecosystem, precomps |

### Audio

| Tool | Core components |
| --- | --- |
| REAPER | Multitrack recording and editing, flexible routing (any track to any track), plugin hosting (VST, VST3, AU, CLAP), MIDI editor, automation envelopes, ReaScript (Lua, Python, EEL), JSFX real-time DSP language, low-latency audio engine, stem rendering |
| Audacity | Waveform editing (cut, copy, fades), effects (EQ, noise reduction, normalize), spectral view, recording, format export, Nyquist scripting, plugin support |

### Software development

| Tool | Core components |
| --- | --- |
| Visual Studio | C++/C# compilers and MSBuild, debugger (breakpoints, watches, memory, mixed-mode), CPU/memory/GPU profilers, IntelliSense, refactoring, UI designers, test explorer, Git integration |
| VS Code | Fast editor core, extension host and API, Language Server Protocol clients, Debug Adapter Protocol, integrated terminal, source-control view, command palette, remote development (SSH, containers), settings and keybindings |
| JetBrains IDEs | Deep code index and semantic model (PSI), large refactoring suite, inspections and static analysis, smart completion, debugger and profiler, database tools, test runners, VCS UI, one platform with per-language IDEs |

### Electronics

| Tool | Core components |
| --- | --- |
| KiCad | Schematic capture, symbol and footprint libraries, netlist, PCB layout (layers, copper zones, push-and-shove interactive router), electrical and design rule checks (ERC/DRC), 3D board viewer, Gerber and drill output, SPICE simulation, BOM |

### Numerical computing and simulation

| Tool | Core components |
| --- | --- |
| MATLAB | Matrix-first language with interpreter/JIT, linear algebra (LAPACK/BLAS), plotting, toolboxes (signal processing, control, optimization, statistics, image processing), Live Script notebooks, ODE solvers, code generation |
| Simulink | Block-diagram modeling, fixed- and variable-step solvers, subsystem hierarchy, Stateflow state machines, scopes and data logging, embedded code generation, hardware-in-the-loop |
| ANSYS | Geometry prep, meshing (tet/hex, refinement), structural FEA (static, modal, nonlinear), CFD (Navier–Stokes, turbulence models), thermal, electromagnetics, explicit dynamics for crash and impact, multiphysics coupling, post-processing |
| COMSOL Multiphysics | Physics-agnostic PDE engine (weak form), physics interfaces on top (structural, heat, EM, acoustics, chemical), true multiphysics coupling, meshing, parametric sweeps and optimization, App Builder for turning models into simple apps |

### Geographic information

| Tool | Core components |
| --- | --- |
| QGIS | Vector and raster layers, coordinate reference systems and reprojection (PROJ), styling and symbology, Processing toolbox (GDAL, GRASS), spatial queries, print layouts, PostGIS connections, Python plugins |
| ArcGIS | Everything QGIS does, plus geodatabases, Spatial Analyst (terrain, hydrology), network analysis and routing, 3D scenes, ModelBuilder visual workflows, online/enterprise publishing, ArcPy |

## Part 2: Compounding skill hierarchy

Each tier depends only on the tiers below it. Node numbers (e.g. **1.7**) are used in the coverage map and stage mapping. A **Done when** endpoint is a concrete test an agent has to pass before the node counts as finished. It's scoped to "works for your needs," not "matches the commercial product."

### The big compounders

These few nodes are reused by the most tools, so they should be built early and built well:

- **1.7 Node graph**: shaders, procedural geometry, materials, compositing, visual scripting, audio graphs, Simulink blocks, GIS workflows (about 9 tools)
- **1.8 Timeline**: animation, video editing, motion graphics, DAW, cinematics, Krita animation (about 8 tools)
- **2.3 Mesh kernel**: every 3D tool, game engines, FEM meshing, texture painting
- **2.8 Numerics core**: all physics, FEM, CFD, MATLAB, Simulink, CAD constraint solving
- **2.9 Constraint solver**: AutoCAD, all parametric CAD, KiCad, rig constraints
- **0.8 Scripting/API**: lets agents drive and test every other node (this is how Genesis plugs in)

### Tier 0: Bedrock

| Node | What it is | Done when |
| --- | --- | --- |
| 0.1 Build and repo discipline | One-command build, test and clean; Git; CI; disk hygiene | Fresh clone builds and tests in one command; CI runs on every commit; build and scratch folders auto-prune and never eat into the 20 GB reserve |
| 0.2 Math library | Vectors, matrices, quaternions, transforms, bounding boxes, ray and shape intersection | Full unit tests against reference values; fuzz tests confirm inverse/compose identities |
| 0.3 Data structures and memory | Arenas, handles, slot maps, BVH, octree, spatial hash | Benchmarks beat the naive version; zero leaks under sanitizers |
| 0.4 Job system | Thread pool and task graph | Near-linear speedup to your core count on a synthetic workload |
| 0.5 Serialization and asset format | Versioned binary and text formats with migrations | Save/load round-trip is byte-identical; an old-version file still loads after a schema change |
| 0.6 Command and undo system | Every edit is a reversible command | 10,000 random edits followed by full undo returns exactly to the start state |
| 0.7 Plugin/module system | Runtime-loaded modules behind a stable API | A module hot-reloads without restarting the app |
| 0.8 Scripting and automation API | Embed Lazlo's Language (Lua/Python as a stopgap) | A script can create, edit and query anything the UI can |
| 0.9 Logging, tracing, test harness | Structured logs, timing capture, golden-file tests | One frame's timings open in a trace viewer; golden-image tests flag regressions |

### Tier 1: Platform

| Node | What it is | Done when |
| --- | --- | --- |
| 1.1 Window and input | Multi-window; keyboard, mouse, gamepad, pen pressure and tilt | Input latency is measured and logged; tablet pressure curve is correct (your stylus-testing work fits here) |
| 1.2 GPU abstraction | wgpu/Vulkan resources, pipelines, compute | The same render and compute shader run on two backends |
| 1.3 2D vector renderer | Paths, fills, strokes, gradients, clipping | Passes an SVG conformance subset within pixel tolerance |
| 1.4 Text and fonts | Shaping, layout, rich text | Ligatures and mixed scripts render; a million-line document scrolls smoothly |
| 1.5 UI framework and editor shell | Widgets, docking panels, themes, viewports | A dockable layout saves and restores; 60 fps with 20 panels open |
| 1.6 Reflection and properties | Type introspection | Any struct is editable in an auto-generated inspector, serializable and scriptable with no hand-written glue |
| 1.7 Node graph framework | Typed ports, lazy cached evaluation, dirty propagation, subgraphs, editor UI | A 500-node graph re-evaluates only dirty nodes; a subgraph saves as a reusable asset; two graph types (math, image) share one editor |
| 1.8 Timeline framework | Tracks, clips, keyframes, curves, playhead, scrubbing | One widget drives an animation track, an audio track and a video track |
| 1.9 Media I/O \[wrap first: FFmpeg, OpenEXR\] | Image, video and audio codecs | Encode/decode round-trips; 4K H.264 plays in real time |
| 1.10 Audio device layer | Low-latency in/out | Under 10 ms round trip; no dropouts over a one-hour run |
| 1.11 Color management \[wrap first: OCIO\] | Linear workflow, ICC profiles, view transforms | The same image matches a reference across every app |

### Tier 2: Core engines

| Node | Builds on | Done when |
| --- | --- | --- |
| 2.1 Raster image engine (tiled layers, blend modes, masks, selections, non-destructive adjustments) | 1.2, 1.7, 1.11 | A 16k × 16k image with 50 layers stays interactive |
| 2.2 Brush engine (dabs, pressure dynamics, stabilizer, smudge) | 1.1, 2.1 | Stroke lag on your tablet measures no worse than Krita's |
| 2.3 Mesh kernel (half-edge mesh, attributes, normals, UVs, booleans, subdivision, remesh, modifier stack) | 0.2, 0.3, 0.6 | A boolean on two 100k-triangle meshes comes out watertight; the modifier stack is fully non-destructive |
| 2.4 Scene system (hierarchy/ECS, transforms, instancing, layered scenes) | 0.5, 1.6 | 1M instances update at 60 fps; glTF and USD import and export |
| 2.5 Real-time renderer (PBR, shadows, IBL, TAA, bloom, tonemapping, debug views) | 1.2, 2.4 | Khronos glTF sample models match the reference renders |
| 2.6 Animation core (keyframes, curves, skeletons, skinning, blending, IK) | 1.8, 2.4 | An imported character blends walk and run, with foot IK on slopes |
| 2.7 Audio DSP engine (mixer graph, effects, sample-accurate automation, MIDI; plugin hosting \[wrap first: CLAP/VST3 SDK\]) | 1.7, 1.10 | 64 tracks with effects play in real time; a third-party plugin loads |
| 2.8 Numerics core (dense/sparse linear algebra, FFT, ODE integrators, optimizers, autodiff) | 0.2, 0.4 | Matches LAPACK/SciPy outputs on a test suite within tolerance |
| 2.9 2D constraint solver and sketcher (coincident, tangent, distance and angle constraints; DOF reporting) | 2.8, 1.3 | A 30-constraint sketch solves instantly and reports over- or under-constrained states |
| 2.10 Code editor core (rope buffer, tree-sitter highlighting, LSP client, multi-cursor) | 1.4, 1.5 | A 100 MB file opens; Rust and LL get completion and go-to-definition |

### Tier 3: Domain engines

**Physics**

| Node | Builds on | Done when |
| --- | --- | --- |
| 3.1 Rigid-body physics (broadphase, GJK/EPA, contact solver, joints) | 2.8, 0.3 | A 1,000-box stack stays stable; replays are deterministic |
| 3.2 Soft body and deformables (node-beam/FEM, cloth, plasticity, breakage) | 3.1, 2.3 | Your car crash simulator: a car hits a wall and deforms plausibly in real time |
| 3.3 Fluids and smoke (SPH/FLIP, grid smoke and fire) | 2.8, 0.4 | Dam-break benchmark looks right; a smoke plume renders |
| 3.4 Destruction (Voronoi fracture, debris) | 3.1, 2.3 | A wall shatters into rigid-body chunks |

**3D creation**

| Node | Builds on | Done when |
| --- | --- | --- |
| 3.5 Path tracer (BVH, materials, importance sampling, denoiser) | 2.4, 2.5 materials | Cornell box converges to the reference; the same material looks consistent in 2.5 and 3.5 |
| 3.6 Procedural geometry (node ops on meshes, attribute scripting) | 1.7, 2.3, 0.8 | A procedural building or terrain generator saves as a reusable asset (Houdini HDA equivalent) |
| 3.7 Sculpting (dynamic topology, multires, 3D brushes) | 2.3, 2.2 | A 5M-polygon sculpt stays interactive |
| 3.8 Texture painting and material authoring (paint on mesh, layered PBR, bakers) | 2.1, 2.2, 2.5, 2.3 | A high-to-low normal map bakes cleanly; a smart-material wear pass exports straight into your engine |
| 3.9 Rigging tools (bones, weight painting, constraints, control rigs) | 2.6, 2.9 | A biped is rigged and a walk cycle animated entirely in-tool |

**CAD and fabrication**

| Node | Builds on | Done when |
| --- | --- | --- |
| 3.10 B-rep CAD kernel \[wrap first: OpenCASCADE\] (NURBS, solids, fillets, STEP I/O) | 2.8 | Import a STEP file, fillet an edge, export a STEP that other CAD opens |
| 3.11 Parametric feature tree (sketch, extrude, revolve, fillet, with history) | 2.9, 3.10, 0.6 | Change one dimension and the whole part regenerates correctly |
| 3.12 Assemblies and drawings (mates/joints, dimensioned 2D drawings, DXF/DWG) | 3.11, 1.3 | A 10-part assembly moves correctly through its mates; drawings export to DXF |
| 3.13 CAM (2.5D pocketing, contours, G-code) | 3.10 | G-code simulates cleanly in a third-party viewer |
| 3.14 EDA (schematic, netlist, PCB layout, router, DRC, Gerber) | 2.9, 1.3, 0.3 | A small board design passes a fab's online Gerber check |

**Media**

| Node | Builds on | Done when |
| --- | --- | --- |
| 3.15 Compositor and color grading (node comp, keying, color wheels, tracking) | 1.7, 2.1, 1.11 | Key a green screen, grade it, and track a patch onto a moving surface |
| 3.16 Nonlinear video editing (multi-track, trims, proxies, render queue) | 1.8, 1.9 | Edit and export a 10-minute 4K piece with no dropped frames |
| 3.17 Motion graphics (layer animation, shape layers, text animators, expressions) | 1.8, 1.3, 0.8 | Recreate an animated title sequence |
| 3.18 DAW features (recording, MIDI editor, routing, automation) | 2.7, 1.8 | Record and mix a multitrack song |

**Simulation and analysis**

| Node | Builds on | Done when |
| --- | --- | --- |
| 3.19 FEM solver (tet meshing, linear elasticity, thermal, modal) | 2.8, 2.3 | Cantilever-beam deflection is within 1% of the analytic answer |
| 3.20 CFD (incompressible Navier–Stokes, k-ε turbulence) | 2.8, 0.4 | Lid-driven cavity matches the Ghia benchmark |
| 3.21 Multiphysics (weak-form PDE framework, coupled fields) | 3.19, 3.20 | A thermal-expansion problem matches a published COMSOL tutorial result |
| 3.22 Block-diagram simulation (signal flow, solvers, state machines) | 1.7, 2.8 | A PID controller holds a motor model on target, shown on a scope |
| 3.23 Matrix language and notebook (REPL, plotting) | 2.8, 0.8, 1.3 | A MATLAB signal-processing tutorial reproduces end to end |
| 3.24 GIS core \[wrap first: PROJ, GDAL\] (projections, vector/raster layers, tiles, spatial index) | 0.3, 1.3 | Load OSM data and a DEM, reproject, and measure distances correctly |

**Developer tooling**

| Node | Builds on | Done when |
| --- | --- | --- |
| 3.25 Debugger and profiler (DAP client, breakpoints, watches, memory view, flame graphs) | 2.10, 0.9 | Debug both a Rust program and an LL program from the editor |
| 3.26 Semantic code index (cross-file symbols, refactors, inspections) | 2.10 | A project-wide rename is safe; dead-code inspection is correct |

### Tier 4: Products (your versions of the tools)

| Node | Builds on | Done when |
| --- | --- | --- |
| 4.1 Game runtime (game loop, ECS, asset streaming, gameplay scripting, nav and AI, networking, packaging) | 2.4–2.7, 3.1 | A small playable build runs on another PC |
| 4.2 Game editor (scene editing, play-in-editor, visual scripting, prefabs, VFX editor, level streaming) | 4.1, 1.5, 1.7 | A level of EVOLUTION or Habit is built entirely inside Broadsword |
| 4.3 DCC Studio (model, sculpt, UV, rig, animate, render) | 2.3, 2.6, 3.5–3.9 | Model, rig, animate and render a character from nothing |
| 4.4 Image and Texture Studio | 2.1, 2.2, 3.8 | A hero asset is painted, textured and exported |
| 4.5 CAD Studio (part, assembly, drawing, CAM) | 3.10–3.13 | A real part is designed and then 3D printed or milled |
| 4.6 Media Studio (edit, color, comp, motion graphics, audio post, delivery) | 3.15–3.18 | A finished video with a mixed soundtrack is delivered |
| 4.7 Audio Studio (DAW) | 2.7, 3.18 | A finished mixed track |
| 4.8 Sim and Analysis Studio | 3.19–3.23 | The whole benchmark suite passes from one app |
| 4.9 Dev Studio (IDE with LL as a first-class language) | 2.10, 3.25, 3.26 | You develop Broadsword inside Broadsword (the self-hosting milestone) |
| 4.10 EDA Studio | 3.14 | A board gets manufactured and works |
| 4.11 Geo Studio | 3.24 | A printed map is produced, and real terrain feeds into the game engine |

### Tier 5: Integration

| Node | Builds on | Done when |
| --- | --- | --- |
| 5.1 Unified asset database (one library, versioned, shared by every studio) | 0.5, all studios | Edit a texture in the Image Studio and it updates live in the engine and the DCC |
| 5.2 Live link and round-trip | 5.1 | A CAD part goes through FEM and lands in the game with its physical properties intact |
| 5.3 AI agent layer (Genesis drives every app through 0.8) | 0.8, all | An agent builds a simple level from a text brief using only the public API |
| 5.4 Studio UI shell (one workspace, consistent keys and themes) | 1.5, all | Switch Model → Paint → Engine → Edit in one window without losing context |

## Part 3: Coverage map

All of Tier 0 and the relevant parts of Tier 1 are assumed for every row. The "Key nodes" column lists what makes each tool distinct.

| Tool | Key nodes | Your product |
| --- | --- | --- |
| Unreal Engine | 1.7, 2.4–2.7, 3.1–3.4, 3.6, 3.17 (Sequencer) | 4.1, 4.2 |
| Unity | 2.4–2.7, 3.1, 1.7 (Shader/VFX Graph), 1.8 (Timeline) | 4.1, 4.2 |
| Godot | 1.3 (2D), 2.4–2.6, 3.1 | 4.1, 4.2 |
| Blender | 2.3–2.6, 3.2, 3.3, 3.5–3.9, 3.15, 3.16 | 4.3 |
| Autodesk Maya | 2.3, 2.6, 3.2, 3.5, 3.6, 3.9 | 4.3 |
| Houdini | 1.7, 3.2–3.6, 3.15 | 4.3 |
| 3ds Max | 2.3, 3.5, 3.6 | 4.3 |
| SolidWorks | 2.9, 3.10–3.12, 3.19 | 4.5 |
| Fusion 360 | 2.9, 3.10–3.13, 3.14, 3.19 | 4.5 |
| AutoCAD | 1.3, 2.9, 3.12 | 4.5 |
| FreeCAD | 2.9, 3.10–3.13, 3.19 | 4.5 |
| Photoshop | 1.11, 2.1, 2.2 | 4.4 |
| GIMP | 1.7, 2.1, 2.2 | 4.4 |
| Krita | 2.1, 2.2, 1.8 (animation) | 4.4 |
| Substance 3D Painter | 2.5, 3.8 | 4.4 |
| DaVinci Resolve | 1.8, 1.9, 3.15, 3.16, 3.18 | 4.6 |
| Adobe Premiere Pro | 1.8, 1.9, 3.16 | 4.6 |
| After Effects | 3.15, 3.17 | 4.6 |
| REAPER | 1.10, 2.7, 3.18 | 4.7 |
| Audacity | 1.10, 2.7 | 4.7 (a subset; a good first audio milestone) |
| Visual Studio | 2.10, 3.25 | 4.9 |
| VS Code | 2.10, 0.7 (extensions) | 4.9 |
| JetBrains IDEs | 2.10, 3.25, 3.26 | 4.9 |
| KiCad | 2.9, 3.14 | 4.10 |
| MATLAB | 2.8, 3.23 | 4.8 |
| Simulink | 1.7, 2.8, 3.22 | 4.8 |
| ANSYS | 3.2 (explicit crash dynamics), 3.19–3.21 | 4.8 |
| COMSOL Multiphysics | 3.19, 3.21 | 4.8 |
| QGIS | 1.3, 3.24 | 4.11 |
| ArcGIS | 1.7 (ModelBuilder), 3.24 | 4.11 |

The map shows the payoff: 30 tools collapse into **11 products**, and from Tier 3 up most nodes serve more than one of them.

## Part 4: Broadsword alignment and order

### Mapping to your 12 stages

| Broadsword stage | Nodes |
| --- | --- |
| 1. Foundation | Tier 0; 1.1, 1.5, 1.6 |
| 2. AI integration | 0.8 scripting API made agent-drivable from day one; groundwork for 5.3 |
| 3. Autonomous development | 0.9 test harness and golden tests (agents need pass/fail signals); 2.10 |
| 4. Graphics | 1.2, 1.3, 1.4, 1.11, 2.1, 2.2, 2.5 |
| 5. 3D creation | 2.3, 2.4, 2.6, 3.5–3.9, 4.3, 4.4 |
| 6. Physics | 3.1–3.4 |
| 7. Games/apps | 4.1, 4.2 |
| 8. CAD | 2.9, 3.10–3.14, 4.5, 4.10 |
| 9. Media | 1.8, 1.9, 1.10, 2.7, 3.15–3.18, 4.6, 4.7 |
| 10. Advanced | 2.8 extensions, 3.19–3.24, 3.25, 3.26, 4.8, 4.9, 4.11 |
| 11. Studio UI | 5.4 |
| 12. Integration | 5.1, 5.2, 5.3 |

Two suggested adjustments to your stage order:

- **Move 1.7 (node graph) and 1.8 (timeline) into Foundation.** They feed at least four later stages each. Built late, they end up rebuilt three times in slightly different forms.
- **Move 2.8 (numerics core) up to Graphics or earlier.** Physics, CAD and simulation all sit on it, and a solid numerics core makes stage 6 much faster.

### Rough multi-year order

Assumes part-time, agent-driven work. The rule for each phase is to finish on something you can *use*, so motivation never depends on a far-off payoff.

1. **Phase A (about year 1): Bedrock and 2D.** Tier 0, Tier 1, then 2.1 and 2.2. Payoff: your own painting/image app. 2D gives fast, visible wins and exercises the UI, undo, node graph and GPU layers without 3D complexity.
2. **Phase B (about year 2): First 3D and first game.** 2.3–2.5, 3.1, a minimal 4.1. Payoff: a tiny playable game in your own engine.
3. **Phase C (about year 3): Motion and deformation.** 2.6, 3.2 (your car crash simulator becomes the test bed), 3.6, 3.9, a basic 4.2 and 4.3. Payoff: build and animate assets and drop them into your own editor.
4. **Phase D (about year 4): Media and audio.** 2.7, 3.15–3.18, with Audacity-level audio first. Payoff: edit and score a trailer for one of your games.
5. **Phase E (year 5 and beyond): Engineering and science.** 2.9, 3.10–3.14 (OpenCASCADE wrapped), 3.19–3.24, 4.9. Payoff: design a real part, simulate it, then develop Broadsword inside Broadsword.
6. **Phase F (ongoing): Integration.** Tier 5, starting with 5.3 early in thin slices, since that's where the AI-workflow learning pays off most.

### Notes for when you pick this up

- **Language fit.** Rust suits nearly all of this. The exceptions are mostly the \[wrap first\] nodes: OpenCASCADE and the VST3 SDK are C++, so bind to them rather than porting. CLAP is a plain C API and binds easily.
- **Endpoints are the agent contract.** Each **Done when** line can become a test file before any code exists. An agent then works until it passes, which suits your standing-instructions approach and makes progress measurable.
- **Low-payoff corners to defer.** ArcGIS-only features (enterprise publishing, geodatabases), ANSYS-grade CFD, and full DWG fidelity cost a lot for little personal use. Keep them listed but unscheduled.
- **Scale check.** Each commercial tool here represents hundreds to thousands of person-years. These endpoints aim at "functional for my needs," which is what makes the whole thing possible at all. Treat any endpoint as something you can raise later.

## Part 5: Copy-paste agent prompts

One prompt per node. Copy the block, paste it into the Broadsword project, and leave. Every prompt has the same closing rules, so each one stands alone. The rules tell the agent to write the **Done when** checks as tests first, work without you, log its decisions, and stop instead of building around a missing prerequisite.

Two things make these work better over time:

- Keep a `ROADMAP.md` in the repo listing each node as **not started / in progress / done**. The prompts tell the agent to update it, so the next session (or the next prompt) knows where things stand.
- Per-node notes go in `docs/nodes/`. If you resume a node, paste the same prompt again: the agent reads its own notes and continues.

**Template for anything not listed:**

```text
Broadsword node <number> — <name>
Build: <what it is>
Needs: <prerequisite nodes>
Done when: <endpoint>
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

### Tier 0 prompts

```text
Broadsword node 0.1 — Build and repo discipline
Build: one-command build, test and clean; Git repo; CI on every commit; automatic pruning of build and scratch folders.
Needs: nothing
Done when: a fresh clone builds and tests in one command; CI runs on every commit; build and scratch folders auto-prune every few hours and never eat into the 20 GB reserve.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.2 — Math library
Build: vectors, matrices, quaternions, transforms, bounding boxes, ray and shape intersection.
Needs: 0.1
Done when: full unit tests pass against reference values; fuzz tests confirm inverse and compose identities.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.3 — Data structures and memory
Build: arenas, handles, slot maps, BVH, octree, spatial hash.
Needs: 0.1, 0.2
Done when: benchmarks beat a naive implementation of each; zero leaks under sanitizers.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.4 — Job system
Build: thread pool and task graph with dependencies.
Needs: 0.1, 0.3
Done when: a synthetic workload shows near-linear speedup up to the machine's core count.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.5 — Serialization and asset format
Build: versioned binary and text formats with schema migrations.
Needs: 0.1, 0.3
Done when: save/load round-trips are byte-identical; a file from an older schema version still loads after a schema change.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.6 — Command and undo system
Build: every edit is a reversible command; undo/redo stacks; command merging.
Needs: 0.1, 0.5
Done when: 10,000 random edits followed by full undo return exactly to the starting state.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.7 — Plugin/module system
Build: runtime-loaded modules behind a stable, versioned API.
Needs: 0.1
Done when: a module hot-reloads without restarting the host app, and an incompatible module version is refused cleanly.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.8 — Scripting and automation API
Build: embedded scripting (Lazlo's Language when ready; Lua or Python as a stopgap) exposing the full object model, designed so AI agents can drive it.
Needs: 0.6, 0.7
Done when: a script can create, edit and query anything the UI can, and every scripted edit is undoable.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 0.9 — Logging, tracing and test harness
Build: structured logging, timing capture exportable to a trace viewer, golden-file and golden-image regression tests.
Needs: 0.1
Done when: one frame's timings open in a trace viewer; a deliberately changed image output is flagged by the golden-image test.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

### Tier 1 prompts

```text
Broadsword node 1.1 — Window and input
Build: multi-window support; keyboard, mouse, gamepad, and pen input with pressure and tilt; input-latency measurement.
Needs: 0.1–0.4
Done when: input latency is measured and logged; tablet pressure and tilt values are correct across the full range; two windows run at once.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.2 — GPU abstraction
Build: wgpu/Vulkan layer for buffers, textures, pipelines and compute.
Needs: 1.1
Done when: the same render shader and compute shader produce matching output on two backends.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.3 — 2D vector renderer
Build: GPU paths, fills, strokes, gradients, clipping.
Needs: 1.2
Done when: a chosen subset of the SVG conformance suite renders within pixel tolerance (golden-image tests via 0.9).
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.4 — Text and fonts
Build: font loading, shaping, layout, rich text rendering.
Needs: 1.3
Done when: ligatures and mixed scripts render correctly; a one-million-line document scrolls smoothly.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.5 — UI framework and editor shell
Build: widgets, docking panels, themes, viewports, editor shell.
Needs: 1.1, 1.4, 1.6, 0.6
Done when: a docked layout saves and restores exactly; the shell holds 60 fps with 20 panels open.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.6 — Reflection and properties
Build: type introspection so structs become editable, serializable and scriptable automatically.
Needs: 0.5
Done when: any annotated struct appears in an auto-generated inspector, saves/loads, and is scriptable with no hand-written glue.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.7 — Node graph framework
Build: typed ports, lazy cached evaluation, dirty propagation, subgraphs, and a graph editor UI, generic enough to serve shaders, procedural geometry, compositing, audio and block simulation later.
Needs: 1.5, 1.6, 0.6
Done when: a 500-node graph re-evaluates only dirty nodes; a subgraph saves as a reusable asset; two different graph types (math and image) share one editor.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.8 — Timeline framework
Build: tracks, clips, keyframes, curves, playhead and scrubbing, generic across animation, audio and video.
Needs: 1.5, 0.6
Done when: one timeline widget drives an animation track, an audio track and a video track.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.9 — Media I/O
Build: image, video and audio codecs, wrapping FFmpeg and OpenEXR behind our own interface.
Needs: 0.5
Done when: encode/decode round-trips pass for PNG, EXR, JPEG, WAV and H.264; 4K H.264 plays in real time.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.10 — Audio device layer
Build: low-latency audio input and output.
Needs: 0.4
Done when: round-trip latency is under 10 ms and a one-hour run has zero dropouts.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 1.11 — Color management
Build: linear workflow, ICC profiles and view transforms, wrapping OpenColorIO.
Needs: 1.2
Done when: the same test image matches a reference rendering in every app that uses this layer.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

### Tier 2 prompts

```text
Broadsword node 2.1 — Raster image engine
Build: tiled layers, blend modes, masks, selections, non-destructive adjustments built on the node graph.
Needs: 1.2, 1.7, 1.11
Done when: a 16k × 16k image with 50 layers stays interactive while painting and adjusting.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.2 — Brush engine
Build: dab-based brushes, pressure and tilt dynamics, stabilizer, smudge.
Needs: 1.1, 2.1
Done when: measured stroke lag with a tablet is no worse than Krita's on the same machine.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.3 — Mesh kernel
Build: half-edge mesh, attributes, normals, UVs, booleans, subdivision, remeshing, non-destructive modifier stack.
Needs: 0.2, 0.3, 0.6
Done when: a boolean on two 100k-triangle meshes comes out watertight; the modifier stack is fully non-destructive and undoable.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.4 — Scene system
Build: hierarchy/ECS, transforms, instancing, layered scenes, glTF and USD import/export.
Needs: 0.5, 1.6
Done when: 1M instances update at 60 fps; glTF and USD files import and export round-trip.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.5 — Real-time renderer
Build: PBR materials, shadows, image-based lighting, TAA, bloom, tonemapping, debug views.
Needs: 1.2, 2.4
Done when: the Khronos glTF sample models match their reference renders within tolerance.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.6 — Animation core
Build: keyframes, curves, skeletons, skinning, animation blending, IK.
Needs: 1.8, 2.4
Done when: an imported rigged character blends walk and run smoothly with foot IK on slopes.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.7 — Audio DSP engine
Build: mixer graph on the node graph, effects, sample-accurate automation, MIDI, and plugin hosting via the CLAP/VST3 SDKs.
Needs: 1.7, 1.10
Done when: 64 tracks with effects play in real time; a third-party plugin loads and processes audio.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.8 — Numerics core
Build: dense and sparse linear algebra, FFT, ODE integrators, optimizers, automatic differentiation.
Needs: 0.2, 0.4
Done when: outputs match LAPACK/SciPy on a reference test suite within tolerance.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.9 — 2D constraint solver and sketcher
Build: geometric constraints (coincident, tangent, parallel, distance, angle) with degrees-of-freedom reporting and a sketch UI.
Needs: 2.8, 1.3
Done when: a 30-constraint sketch solves instantly and correctly reports over- and under-constrained states.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 2.10 — Code editor core
Build: rope text buffer, tree-sitter highlighting, LSP client, multi-cursor editing.
Needs: 1.4, 1.5
Done when: a 100 MB file opens and edits smoothly; Rust and Lazlo's Language get completion and go-to-definition via LSP.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

### Tier 3 prompts

```text
Broadsword node 3.1 — Rigid-body physics
Build: broadphase, GJK/EPA collision, contact solver, joints, deterministic stepping.
Needs: 2.8, 0.3
Done when: a 1,000-box stack stays stable; replaying the same inputs gives identical results.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.2 — Soft body and deformables
Build: node-beam/FEM soft bodies, cloth, plastic deformation, breakage (BeamNG-style vehicle physics).
Needs: 3.1, 2.3
Done when: in the car crash simulator, a car hits a wall and deforms plausibly in real time.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.3 — Fluids and smoke
Build: SPH/FLIP liquids, grid-based smoke and fire.
Needs: 2.8, 0.4
Done when: the dam-break benchmark behaves correctly; a smoke plume simulates and renders.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.4 — Destruction
Build: Voronoi fracture and debris handed off to rigid-body physics.
Needs: 3.1, 2.3
Done when: a wall shatters into rigid-body chunks that settle correctly.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.5 — Path tracer
Build: BVH, physically based materials shared with the real-time renderer, importance sampling, denoiser.
Needs: 2.4, 2.5
Done when: the Cornell box converges to the reference image; the same material looks consistent in the real-time renderer and the path tracer.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.6 — Procedural geometry
Build: node-based mesh operations on the node graph, plus attribute scripting.
Needs: 1.7, 2.3, 0.8
Done when: a procedural building or terrain generator saves as a reusable asset with exposed parameters (Houdini HDA equivalent).
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.7 — Sculpting
Build: dynamic topology, multiresolution, 3D sculpt brushes reusing the brush engine.
Needs: 2.3, 2.2
Done when: a 5M-polygon sculpt stays interactive.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.8 — Texture painting and material authoring
Build: painting on meshes in 3D, layered PBR materials, procedural masks, bakers (normal, AO, curvature, thickness, ID).
Needs: 2.1, 2.2, 2.3, 2.5
Done when: a high-to-low normal map bakes cleanly; a smart-material wear pass exports straight into the Broadsword engine.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.9 — Rigging tools
Build: bone creation, weight painting, constraints, control rigs.
Needs: 2.6, 2.9
Done when: a biped is rigged and a walk cycle animated entirely in-tool.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.10 — B-rep CAD kernel
Build: NURBS surfaces, solids, fillets and STEP I/O, wrapping OpenCASCADE behind our own interface (bind, don't port).
Needs: 2.8
Done when: import a STEP file, fillet an edge, and export a STEP that other CAD software opens correctly.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.11 — Parametric feature tree
Build: sketch → extrude, revolve, fillet, chamfer, with full history and regeneration.
Needs: 2.9, 3.10, 0.6
Done when: changing one sketch dimension regenerates the whole part correctly.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.12 — Assemblies and drawings
Build: mates/joints between parts, dimensioned 2D drawings, DXF export (DWG later).
Needs: 3.11, 1.3
Done when: a 10-part assembly moves correctly through its mates; its drawing exports to DXF and opens elsewhere.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.13 — CAM
Build: 2.5D pocketing and contour toolpaths, G-code output.
Needs: 3.10
Done when: generated G-code simulates cleanly in a third-party G-code viewer.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.14 — EDA (electronics)
Build: schematic capture, netlist, PCB layout, interactive router, DRC, Gerber and drill output.
Needs: 2.9, 1.3, 0.3
Done when: a small board design passes a PCB fab's online Gerber check.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.15 — Compositor and color grading
Build: node-based compositing, keying, color wheels and curves, qualifiers, point tracking.
Needs: 1.7, 2.1, 1.11
Done when: a green-screen clip is keyed and graded, and a patch is tracked onto a moving surface.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.16 — Nonlinear video editing
Build: multi-track video timeline, trim tools, proxies, render queue.
Needs: 1.8, 1.9
Done when: a 10-minute 4K edit plays back with no dropped frames and exports correctly.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.17 — Motion graphics
Build: layer animation, shape layers, text animators, expressions in our scripting language.
Needs: 1.8, 1.3, 0.8
Done when: an animated title sequence (kinetic text, shapes, easing) is recreated end to end.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.18 — DAW features
Build: multitrack recording, MIDI editor, flexible routing, automation envelopes, stem rendering.
Needs: 2.7, 1.8
Done when: a multitrack song is recorded, mixed with automation, and rendered to stems.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.19 — FEM solver
Build: tetrahedral meshing, linear elasticity, thermal and modal analysis, result visualization.
Needs: 2.8, 2.3
Done when: cantilever-beam deflection is within 1% of the analytic solution.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.20 — CFD
Build: incompressible Navier–Stokes solver with k-ε turbulence, flow visualization.
Needs: 2.8, 0.4
Done when: the lid-driven cavity case matches the Ghia et al. benchmark data.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.21 — Multiphysics
Build: weak-form PDE framework with coupled physics fields.
Needs: 3.19, 3.20
Done when: a coupled thermal-expansion problem matches a published reference result.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.22 — Block-diagram simulation
Build: signal-flow blocks on the node graph, fixed and variable-step solvers, state machines, scopes.
Needs: 1.7, 2.8
Done when: a PID controller holds a DC motor model on target, shown live on a scope.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.23 — Matrix language and notebook
Build: interactive REPL and notebook with matrix-first syntax and plotting, backed by the numerics core.
Needs: 2.8, 0.8, 1.3
Done when: a standard MATLAB signal-processing tutorial (filter design, FFT, plots) reproduces end to end.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.24 — GIS core
Build: projections, vector and raster layers, tiles and a spatial index, wrapping PROJ and GDAL.
Needs: 0.3, 1.3
Done when: OpenStreetMap data and a DEM load, reproject correctly, and distance measurements match known values.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.25 — Debugger and profiler
Build: DAP client, breakpoints, watches, memory view, flame-graph profiler in the editor.
Needs: 2.10, 0.9
Done when: both a Rust program and a Lazlo's Language program can be debugged from the editor.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 3.26 — Semantic code index
Build: cross-file symbol index, safe refactors, inspections.
Needs: 2.10
Done when: a project-wide rename is safe (all tests still pass) and the dead-code inspection is correct on a test project.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

### Tier 4 prompts

```text
Broadsword node 4.1 — Game runtime
Build: game loop, ECS gameplay, asset streaming, gameplay scripting, navmesh and behavior-tree AI, networking, packaging.
Needs: 2.4, 2.5, 2.6, 2.7, 3.1
Done when: a small playable game packages and runs on a different PC.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.2 — Game editor
Build: scene editing, play-in-editor, visual scripting, prefabs, VFX editor, level streaming.
Needs: 4.1, 1.5, 1.7
Done when: a playable level of EVOLUTION or Habit is built entirely inside the Broadsword editor.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.3 — DCC Studio
Build: one app combining modeling, sculpting, UVs, rigging, animation and rendering.
Needs: 2.3, 2.6, 3.5, 3.6, 3.7, 3.8, 3.9
Done when: a character is modeled, rigged, animated and rendered from nothing without leaving the app.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.4 — Image and Texture Studio
Build: one app combining the painting/image editor and the 3D texture painter.
Needs: 2.1, 2.2, 3.8
Done when: a hero asset is painted, textured and exported to the engine from this app alone.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.5 — CAD Studio
Build: one app for parts, assemblies, drawings and CAM.
Needs: 3.10, 3.11, 3.12, 3.13
Done when: a real part is designed and exported in a form ready to 3D print or mill (STL and G-code).
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.6 — Media Studio
Build: one app for editing, color, compositing, motion graphics, audio post and delivery.
Needs: 3.15, 3.16, 3.17, 3.18
Done when: a finished video with a mixed soundtrack is delivered from this app alone.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.7 — Audio Studio
Build: a standalone DAW app (start Audacity-level, grow toward REAPER-level).
Needs: 2.7, 3.18
Done when: a finished, mixed and mastered track is produced in the app.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.8 — Sim and Analysis Studio
Build: one app hosting FEM, CFD, multiphysics, block-diagram simulation and the matrix notebook.
Needs: 3.19, 3.20, 3.21, 3.22, 3.23
Done when: the full benchmark suite from those nodes passes when run from this app.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.9 — Dev Studio
Build: a full IDE with Lazlo's Language as a first-class language alongside Rust.
Needs: 2.10, 3.25, 3.26
Done when: a real Broadsword feature is developed, debugged and committed entirely inside Dev Studio (the self-hosting milestone).
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.10 — EDA Studio
Build: one app for schematic, layout and fab output.
Needs: 3.14
Done when: a board designed in the app is ready to order (all fab checks pass, BOM generated).
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 4.11 — Geo Studio
Build: one app for map making and terrain export.
Needs: 3.24
Done when: a print-ready map exports, and real-world terrain exports as a heightmap that loads in the Broadsword engine.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

### Tier 5 prompts

```text
Broadsword node 5.1 — Unified asset database
Build: one versioned asset library shared by every studio, with live change notification.
Needs: 0.5, and at least two studios from Tier 4
Done when: editing a texture in the Image and Texture Studio updates it live in the engine and the DCC Studio.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 5.2 — Live link and round-trip
Build: data flowing between studios with properties preserved (geometry, materials, physical properties).
Needs: 5.1
Done when: a CAD part goes through FEM analysis and lands in the game with its mass and material properties intact.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 5.3 — AI agent layer
Build: one agent-facing API over every app (via 0.8) so Genesis or Claude agents can drive any tool.
Needs: 0.8; build in thin slices as each app comes online
Done when: an agent builds a simple playable level from a text brief using only the public API.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```

```text
Broadsword node 5.4 — Studio UI shell
Build: one workspace hosting every studio, with consistent keybindings and themes.
Needs: 1.5, and the studios built so far
Done when: switching Model → Paint → Engine → Edit happens in one window without losing context or unsaved work.
Rules: follow Broadsword's standing instructions (Git, shared build folder, scratch cleanup, 20 GB disk reserve). Check ROADMAP.md and docs/nodes/ first and resume any earlier work on this node. Write the Done-when checks as automated tests first, then build until they pass, committing in small steps. I'm away, so don't wait on me: make sensible calls and log them in docs/nodes/ under this node's number. If a prerequisite is missing or failing its tests, stop and report it instead of building around it. Finish by reporting test results and what's left, and updating this node's status in ROADMAP.md.
```
