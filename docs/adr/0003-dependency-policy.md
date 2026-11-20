# ADR 0003: dependency policy (crates beyond std, wrapped C/C++ libraries, the GPU)

**Status: Proposed.** Lazlo decides. The recommendation below is the orchestrator's input to that decision, not the
decision itself. Nothing in this ADR is in force until Lazlo accepts it; until then every node stays std only (as today).

- Date: 2026-11-19. Board task `t-20261119-225028` (brief `ra-adr-0003`, `[roadmap-ahead]`, lowest priority).
- Written from the repo as of Broadsword `main` at `fefe8b1`. Facts about the repo below were read from it on that day.
  Facts about outside libraries (licences, platform support) are from memory of their projects, not checked online (this
  task has no network): each one is marked "verify" and is checked when its node is first planned, before any vendoring.
- Scope: only the dependency rule that the roadmap plans (`docs/nodes/<node>.md`) point at. No code, no downloads.

## The question

Everything built so far uses only the Rust standard library: `crates/broadsword/Cargo.toml` has an empty
`[dependencies]`, and so do `projects/crash-sim` and `projects/render` (render's README: "uses only the Rust standard
library, with no crates"). CAPABILITY_ROADMAP.md marks seven libraries "wrap first" and node 1.2 needs a GPU layer
(DEVELOPMENT_PLAN.md:45 says "wgpu"). Each node plan needs one rule to point at: when may a node use a crate, wrap a
C/C++ library, or use the GPU, and what must it do to keep builds offline, shared and checkable?

## What is true today (measured, not assumed)

- **Build folder is shared and relative:** `.cargo/config.toml` sets `target-dir = "build"`. Projects and task worktrees
  (`worktrees/<project>/<task id>`) all sit under the Broadsword root, so Cargo finds this config for all of them.
  Relative paths in a Cargo config are resolved against the folder that holds the `.cargo` directory, so a source
  replacement written once at the root would reach every worktree too. (Verify in the first vendoring task; this is the
  mechanism, not yet a measured fact.)
- **`shared/` is git-ignored** (`.gitignore`: `/shared/*` except `.gitkeep`) and is empty. So anything vendored there is
  not in Git: Git must hold a manifest (name, version, source checksum) from which `shared/` can be rebuilt, or a moved
  or fresh clone cannot build. This is a design requirement of option 2 and 3 below.
- **Offline is already the rule for tooling:** the coder self-check block runs `cargo clippy --offline`. A crate that is
  not already on disk fails that command. With std only the flag costs nothing; with crates it is what decides whether a
  task worktree can build at all.
- **Tools available on this PC:** `cargo-deny 0.22.0` (checks licences, advisories, banned crates and allowed sources
  from a config), `cargo-nextest`, `cargo-mutants`, `cargo clippy`. `cargo-deny` is therefore a ready mechanism for the
  licence and source checks below (it can run fully offline once its advisory database is fetched once: that fetch is a
  network step and needs Lazlo's OK under "Ask first").
- **Disk:** `disk_floor_gb = 20`, `job_disk_reserve_gb = 2`; C: has 1.5 TB free today (365 GB used). The risk is not
  the disk today but growth in the one shared build folder, which the hourly `cargo sweep` trims (files unused for
  `sweep_older_than_days = 7`).
- **Golden images today are byte for byte** against files in `tests/golden/` (render README), produced by a software
  rasteriser in `f32`, with the stated caveat that `f32` sine/cosine may differ by a pixel on another platform.
- **The GPU PC:** `docs/MOVING.md` records the EVO-X3 (Ryzen AI Max+ 395, Radeon 8060S, 128 GB shared LPDDR5X, up to 96
  GB to the GPU). The CPU/GPU split is a BIOS setting that Lazlo changes by hand and that needs a reboot. The RAM gate
  (`ram_warn_percent`) measures the RAM Windows reports to the CPU side, so a larger GPU share lowers what agents can use.

## Language rule these options sit under

AGENTS.md "Languages": Rust for the core, "C/C++ libraries called from Rust: where Rust is weak, such as CAD kernels
(OpenCascade) and video (FFmpeg). Use them, don't rewrite them." So wrapping those two is already allowed in principle;
this ADR decides how, and extends the same treatment to the other wrap-first libraries. Python is for ML and quick
tooling scripts, not a place to hide a runtime dependency of a Broadsword binary. Any other language still needs a
one-line reason in `LAB_LOG.md`.

## Options

Three kinds of dependency, each with its own options. "Option 1" is always std only.

### Kind A: Rust crates (for example `wgpu`, `image`-style codecs, `serde`-style helpers)

| | A1: std only, always | A2: vetted crates per node, vendored into `shared/`, pinned, licence-checked | A3: crates fetched freely by each task |
| --- | --- | --- | --- |
| Offline builds in task worktrees | Works today, nothing to do | Works if `shared/` holds every crate and a root Cargo config replaces crates.io with it; one missing crate fails `--offline` loudly (good) | Needs the network in every worktree; breaks `--offline`; not acceptable |
| Shared build folder and disk floor | Smallest | Dependency trees add compile output to the one build folder (a GPU stack is hundreds of crates; size to be measured in the first task, not guessed). The existing sweep trims it | Same, plus a per-task download cache |
| Licence | Nothing to check | `cargo-deny` allow-list (MIT, Apache-2.0, BSD, ISC, Zlib, Unicode; anything else needs a ledger line); run on the lockfile before vendoring | Unchecked: not acceptable |
| Security review cost | None | One review per crate version added, plus `cargo-deny` advisories; cost is paid once at vendoring, not per task. Supply-chain risk stays (build scripts and proc macros run at build time) | Highest, and unbounded |
| How agents test it | Unchanged | Same tests; the vendored copy is read-only, so no agent can edit a dependency to make a test pass | Same |
| Fit with "learn how it works" | Best | Good if each use is small and named | Poor |

### Kind B: C/C++ libraries (FFmpeg, OpenEXR, OpenColorIO, OpenCASCADE, PROJ, GDAL, CLAP/VST3 SDKs)

| | B1: std only (write it ourselves) | B2: thin `-sys` binding over a library built once into `shared/` | B3: each task builds the library itself |
| --- | --- | --- | --- |
| Offline builds in task worktrees | Works | Works if the compiled library and headers sit in `shared/` and the binding finds them by a path from `broadsword.toml` (new required key, no default). The library is built by one dedicated task, never per agent | Needs source downloads and a C/C++ toolchain per task: no |
| Shared build folder and disk floor | Smallest | One copy. Large for OpenCASCADE, FFmpeg and GDAL (sizes to be measured; do not assume). Sits in `shared/`, outside Git, so it can be rebuilt | N copies: breaks "no agent makes a personal copy" (AGENTS.md) |
| Licence | Nothing to check | See "Licences and what a friend can run" below: depends per library | Same |
| Security review cost | Our code only | Large: C/C++ with parsers of untrusted files (video, images, STEP, GeoTIFF) is the main attack surface in the whole project. Pin the exact release and checksum; review the binding (small); do not review the library itself, but only open files Lazlo supplies | Highest |
| How agents test it | Ordinary tests | Round-trip and reference-file tests through our own interface (never through the library's types); a recorded library version in test output; one test that fails if the library is missing (no skip) | Same |
| Fit with the roadmap | Years of specialist work for codecs, a B-rep kernel and projections (CAPABILITY_ROADMAP.md:16) | Matches "wrap first" and AGENTS.md "use them, don't rewrite them" | n/a |

### Kind C: the GPU

| | C1: software only (CPU rasteriser, no GPU API) | C2: `wgpu`, tested headless with a CPU reference as the oracle | C3: raw Vulkan (`ash`) |
| --- | --- | --- | --- |
| Offline builds | Works | As Kind A (a large vendored tree) | As Kind A, smaller tree, far more of our own unsafe code |
| Headless testing | Trivial, byte-exact today | Possible: `wgpu` can request a software adapter (forced fallback adapter: WARP on Windows, lavapipe/llvmpipe where installed). Verify which of these exist on the EVO-X3 and CI | Same adapters, but we write the plumbing |
| Determinism across drivers | Exact | Not exact for floating-point work: see "Golden images" below | Same |
| Roadmap fit | 1.2 fails its own done check ("the same render and compute shader run on two backends") | Meets it: wgpu itself targets DX12, Vulkan, Metal, GL | Meets it only with a second backend we write |
| Learning value | Low for GPU skills | High, without owning a driver layer | Highest, highest cost |

## Licences and what a friend can run

The case that matters: Lazlo builds a game or tool and a friend runs it on their own PC (not a shop release, not
sold). "Friend runs it" means distributing a binary, so copyleft terms apply even to a free hand-out. This is a
summary for planning, not legal advice. Every row is "verify against the release's own licence file before vendoring".

| Library | Licence as I know it (verify) | What it means for a binary handed to a friend |
| --- | --- | --- |
| FFmpeg | LGPL 2.1+ by default; GPL if built with GPL parts (for example x264, x265). Several codecs are patent-encumbered (H.264, H.265, AAC) | Build LGPL-only (no `--enable-gpl`), load FFmpeg as DLLs (dynamic), and ship its licence text and a way to get its source. Static linking into our exe is the thing to avoid. If a node needs x264, the whole distributed program becomes GPL: that is a new ADR decision, not a node decision |
| OpenCASCADE (OCCT) | LGPL 2.1 with an exception (as I know it) | Same pattern: dynamic, unmodified, licence shipped. Modifying OCCT would oblige publishing the changes |
| OpenEXR | BSD-3-Clause | No obstacle |
| OpenColorIO | BSD-3-Clause | No obstacle |
| PROJ | MIT | No obstacle. Needs its `proj.db` data file shipped |
| GDAL | MIT/X | No obstacle for the core; some optional drivers pull in other licences (check which are built) |
| CLAP | MIT, plain C API | No obstacle |
| VST3 SDK | Was GPLv3 or a Steinberg proprietary agreement; recent SDK versions were reported to move to MIT | Highest uncertainty of the list: verify the exact version's licence before anything is wrapped. If it is only GPLv3 or proprietary, a distributed host cannot ship it freely: default to CLAP only |
| `wgpu` (crate) | MIT or Apache-2.0 | No obstacle. Its dependency tree is checked by `cargo-deny` like any other |

Rule that follows: a library that is GPL or needs a signed agreement is never wrapped by a node on its own. An
LGPL library is allowed only dynamically and unmodified, and the node plan must list the licence-text shipping step.
Games that stay on Lazlo's own machines have no distribution question; the rule applies from the first friend.

## GPU: how the policy is made testable

### Headless testing

- Tests request an adapter with the software fallback forced, so they need no display and no particular graphics card.
  If no adapter can be created, the test **fails** with a named error. It never skips (AGENTS.md "Never stub, fake or
  skip something to make a test pass").
- The first GPU task records, as evidence, which software adapters exist on the EVO-X3 and what name and driver each
  reports. Until that evidence exists, the claim "wgpu runs headless here" is an assumption.
- A hardware adapter run is a second, optional line in the same test, so the same shader runs on the software adapter
  and on the Radeon (the node 1.2 done check needs two backends).

### Golden images that survive different drivers

Floating-point results (sine, division, fused multiply-add, rounding of `f32`) are allowed to differ between GPU
drivers, so byte-exact goldens from one adapter will fail on another and prove nothing about correctness. Policy:

1. **The CPU reference is the oracle,** not a GPU-made file. The existing software rasteriser and its f64 cross-checks
   (`tests/replay.rs`) produce the expected image; the GPU image is compared to it.
2. **Compare with a stated tolerance:** per-channel maximum difference and the count of pixels over it, both given in the
   test and recorded in its output. A tolerance of zero is allowed only for integer-only shaders.
3. **Byte-exact goldens only for the pinned software adapter,** with its name and driver version in the test output so a
   Windows update that changes it shows up as a labelled failure, not a mystery.
4. **Every golden test prints the adapter** (name, backend, driver) and the measured difference, and fails on an empty
   image (a check that can pass vacuously is the rejection reason 4 in the self-check block).
5. A change to a golden is accepted by hand, as the render README already says.

### The EVO-X3 rule as a check

Moving shared memory to the GPU is a BIOS change Lazlo makes by hand, followed by a reboot that stops every agent, and it
lowers the RAM agents can use (`docs/MOVING.md`, steps 2 and 3). A memory note is not enough, so:

- **Marking:** a plan or board task that needs the GPU more than the software adapter does (large textures, big
  compute, local model) says `gpu-heavy` in its brief Board block.
- **The check, run by the orchestrator before it starts a `gpu-heavy` task:** read the GPU memory the machine reports
  and compare it with the number in a new required `broadsword.toml` key (proposed name `gpu_heavy_min_dedicated_gb`,
  no default in code). Pass: the task starts. Fail: the task stays `open`, and the orchestrator writes one line in
  `state/pc-inbox.md` asking Lazlo to change the split, then starts other work. It never changes BIOS or system
  settings itself.
- **Batching:** because a split change reboots the PC, `gpu-heavy` tasks are started together after one change, and the
  split is changed back only when none is left.
- **How the number is read** (the mechanism is a follow-up task, not decided here): candidates are the dedicated-memory
  counter Windows keeps per adapter, or the adapter's reported limits from `wgpu`. The old WMI `AdapterRAM` field is
  known to cap near 4 GB and must not be used. The follow-up chooses one and proves it on the EVO-X3 by showing the
  value before and after a split change.
- **Not yet in force:** neither the key nor the check exists. Until the follow-up task is merged this is a stated rule
  with no mechanism, and the orchestrator checks by hand (Task Manager, Performance, GPU), as `docs/MOVING.md` already
  describes. Per the playbook it must be labelled that way until it is enforced.

## How vendoring would work (for options A2 and B2)

This is the mechanism the recommendation depends on; each point is a requirement for the first vendoring task.

1. **One shared copy in `shared/`:** crates under `shared/vendor/`, C/C++ builds under `shared/native/<library>-<version>/`.
   Read-only to agents (set read-only, and `cargo-deny` and a checksum list verify it).
2. **A manifest in Git** (a file under `docs/` or a new top-level folder; the first task decides) listing each crate or
   library with version, source URL, checksum and licence. `shared/` is rebuilt from it. Without it a clone or the move to
   an external SSD (`docs/MOVING.md`) cannot build.
3. **A source replacement in the root `.cargo/config.toml`** pointing at `shared/vendor` (a path relative to root, never
   absolute), with `--offline` always on for agents.
4. **Native library path as a required `broadsword.toml` key** per wrapped library; a `-sys` crate reads it and fails
   with a named error if it is unset or wrong. No fallback search of the system.
5. **One owner adds or updates a dependency** through a board task whose claim is the manifest and `shared/`; it is a
   high-risk review (supply chain, licence). Ordinary tasks never edit `shared/`.
6. **`cargo-deny` runs as a check** on the lockfile: licences against the allow-list, advisories, and allowed sources
   (only `shared/vendor`). It is meant to enforce the licence rule in code, not in prose. It is not wired in yet.
7. **A new dependency needs a node plan section** naming: why std only is not enough, the licence, the size added to the
   build folder, what untrusted input it parses, and the failing test that proves our interface (not the library) works.

## Recommendation

- **Kind A (crates):** option **A2**, but only per node and only when the node's plan justifies it. Until then, A1:
  every node that does not appear in the table below stays std only. Option A3 is rejected.
- **Kind B (C/C++):** option **B2**, with LGPL libraries dynamic and unmodified, and never GPL. Option B3 is rejected.
  Write-it-ourselves (B1) is kept as the choice for anything small (for example a PPM or WAV reader), and as the fallback
  if a library's licence turns out unusable.
- **Kind C (GPU):** option **C2**: `wgpu`, tested headless against a CPU oracle with tolerances, and the EVO-X3 check
  above. C1 remains the default for every node that does not need the GPU, so the crash sim and the existing renderer
  stay std only and byte-exact.
- **Order:** do the dependency-free parts of each node first; vendor the first crate only when a node's plan says it is
  next, and as its own small task with a high-risk review. Do not vendor ahead of a need.
- **Why:** it is the only combination that keeps builds offline for every agent, keeps one copy of big things, makes the
  licence check mechanical, and lets the roadmap's done checks (4K H.264, STEP in and out, two GPU backends) be met at
  all. The cost is one gate, a manifest, and a review for each addition. That is the point: dependencies are where an
  AI-built project quietly loses control.

## What each option unblocks (every wrap-first and GPU node named)

"Needs" is what the node cannot reach its roadmap done check without. Node numbers and wording are from
CAPABILITY_ROADMAP.md.

| Node | Needs | Option | Licence note | Without it |
| --- | --- | --- | --- | --- |
| 1.2 GPU abstraction | `wgpu` (crate), headless software adapter | A2 + C2 | MIT/Apache-2.0 | Done check ("same shader on two backends") cannot be met |
| 1.9 Media I/O \[FFmpeg, OpenEXR\] | FFmpeg (LGPL, dynamic) and OpenEXR | B2 | FFmpeg LGPL-only build; OpenEXR BSD | Own PPM/WAV/PNG-level I/O only; "4K H.264 plays in real time" cannot be met |
| 1.11 Color management \[OpenColorIO\] | OpenColorIO | B2 | BSD-3 | Own linear/sRGB maths only; "matches a reference across every app" cannot be met |
| 2.7 Audio DSP engine \[CLAP/VST3 SDK\] | CLAP first (C API); VST3 only if its licence allows | B2 | CLAP MIT; VST3 verify | Built-in effects only; "a third-party plugin loads" cannot be met |
| 3.10 B-rep CAD kernel \[OpenCASCADE\] | OpenCASCADE | B2 | LGPL with exception, dynamic | "Import a STEP file, fillet an edge, export a STEP" cannot be met |
| 3.24 GIS core \[PROJ, GDAL\] | PROJ and GDAL | B2 | MIT | Hand-written projections only; OSM/DEM loading limited to formats we write ourselves |

Nodes that use the GPU but do not themselves wrap a library (they need C2 from node 1.2 and, where they depend on a
wrap-first node, that node too):

| Node | Why it needs the GPU or a wrapped node | Option |
| --- | --- | --- |
| 1.3 2D vector renderer | The roadmap says "GPU paths"; a CPU path is possible first (C1) | C1 first, C2 later |
| 2.1 Raster image engine | Depends on 1.2 and 1.11 | C2 and B2 (OCIO) |
| 2.5 Real-time renderer | Depends on 1.2 | C2 |
| 3.15 Compositor and colour grading | Depends on 2.1 and 1.11 | C2 and B2 (OCIO) |
| 3.16 Nonlinear video editing | Depends on 1.9 (FFmpeg) | B2 |
| 3.11 to 3.13 CAD features, CAM | Depend on 3.10 (OpenCASCADE) | B2 |
| 3.18 DAW features | Depends on 2.7 | B2 (CLAP) |
| 4.1, 4.5, 4.6, 4.7, 4.11 (studios and game runtime) | Inherit the nodes above | whichever their parents use |

Everything not named in either table stays on A1 and B1 unless its own plan proves otherwise and Lazlo agrees.
(The second table is built from the dependency column of CAPABILITY_ROADMAP.md; the plans for each node, written in
the `ra-<node>` tasks, confirm or correct it.)

## What would make this recommendation wrong (triggers)

- **Move toward A3-style freedom or wider A2:** if more than a handful of nodes each need many small crates and the
  one-task-per-addition gate becomes the slowest step (measured as vendoring tasks waiting in the queue), loosen the gate
  for crates that `cargo-deny` already passes with no build script.
- **Move a library from B2 to B1 (write it):** if its licence cannot be satisfied (GPL needed, or VST3 unusable), or its
  shipped size or build time breaks the disk floor, or it cannot be built on both the EVO-X3 and the laptop that Broadsword
  must also run on (docs/MOVING.md).
- **Move from C2 to C1 or C3:** if no software adapter works headless on the EVO-X3 (then GPU tests cannot run in the
  queue and only a human can check them), or `wgpu`'s dependency tree is too large to vendor and review, or node 1.2's
  two-backend done check is dropped.
- **Revisit the licence rule:** the moment Lazlo wants to give a build to someone outside friends, or to sell one.
- **Revisit all of it:** if Rust crates for a node mature to the point that "wrap first" no longer saves years (for
  example a pure-Rust STEP kernel), or if a crate in `shared/` gets a published security advisory that `cargo-deny`
  flags and the fix is not available offline.

## Follow-ups, none started

These are proposals for the orchestrator to board only after Lazlo accepts or changes this ADR. They are not part of
this task and nothing here has been done.

- F1: `broadsword.toml` keys `gpu_heavy_min_dedicated_gb` and one `native_<library>_dir` key per wrapped library
  (required, no defaults), plus the orchestrator check for `gpu-heavy` tasks (mechanism proved on the EVO-X3).
- F2: the dependency manifest and `shared/` rebuild command, with the root source replacement and `--offline` proven from a
  fresh task worktree.
- F3: a `cargo-deny` config (licence allow-list, advisories, allowed sources) and a `broadsword check` hook; the advisory
  database fetch needs Lazlo's OK (network).
- F4: the first GPU task records the headless adapters on the EVO-X3 as evidence.
- F5: add the "new dependency" section of this ADR to `templates/brief.md` and `SPEC_CHECKLIST.md` so a plan cannot
  skip it.
