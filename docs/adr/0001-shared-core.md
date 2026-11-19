# ADR 0001: where the shared core lives (math and file formats)

**Status: Accepted, option (a) (Lazlo, 2026-11-19: "approve ADR option (a)").** Keep the copies per repo; file formats are the contract between repos; move to (d2), never (b) or (c), when a trigger in "What would make this recommendation wrong" fires. Follow-ups F1 to F4 are applied to the briefs of nb-40, nb-37, nb-42 and nb-39.
The recommendation below is the orchestrator's input to that decision, not the decision itself.

- Date: 2026-11-19. Board task `t-20261119-171333-47` (brief nb-46).
- Measured on: crash-sim `main` at `2e6424c` (with the new `src/solver.rs`), render `master` at `9e1060b`, Broadsword
  `main` at `04c916b`. Line numbers below are from those commits.
- Scope: only what the crash sim needs now (AGENTS.md:95, "only the parts the crash sim needs now"; DEVELOPMENT_PLAN.md:40,
  the shared core "grows only when a stage needs more").

## The question

DEVELOPMENT_PLAN.md:38-48 lists a shared core (math, geometry, scene, file format, ...). Today crash-sim and render each
have their own `Vec3`, and Stage 6/7 add file formats that more than one side touches. Before a third project copies
`Vec3`, where should shared code live: nowhere yet, a crate in the Broadsword repo, its own repo, or a Cargo workspace?

## What is shared today, measured

### Math

crash-sim `src/vec3.rs` (53 lines, `f64`) and render `src/math.rs` (172 lines, `f32`). Every item in either file:

| Item | crash-sim `src/vec3.rs` | render `src/math.rs` | Same code? |
| --- | --- | --- | --- |
| `struct Vec3 {x, y, z}` + derives `Clone, Copy, Debug, PartialEq, Default` | :4-9 (`f64`) | :7-12 (`f32`) | same shape, different scalar |
| `Vec3::new` (const fn) | :14 | :15 | yes, except scalar |
| `Vec3::dot` | :17 | :18 | yes, except scalar |
| `Vec3::length` (`dot(self).sqrt()`) | :20 | :28 | yes, except scalar |
| `impl Add` | :25 | :45 | yes, except scalar |
| `impl Sub` | :31 | :51 | yes, except scalar |
| `impl Mul<scalar>` | :37 (`f64`) | :57 (`f32`) | yes, except scalar |
| `impl Neg` | :43 | :63 | yes, except scalar |
| `Vec3::ZERO` | :12 | (none; uses `Default`) | crash-sim only |
| `impl AddAssign` | :49 | (none) | crash-sim only |
| `Vec3::cross` | (none) | :21 | render only |
| `Vec3::normalized` (zero-safe) | (none) | :32 | render only |
| `Vec3::lerp` | (none) | :40 | render only |
| `struct Mat4` (row-major `[[f32; 4]; 4]`) | (none) | :70-74 | render only |
| `Mat4::IDENTITY`, `mul`, `translation`, `rotation_y`, `transform_point3`, `transform_dir`, `transform_point`, `look_at`, `perspective` | (none) | :77, :87, :97, :106, :119, :126, :136, :146, :161 | render only (9 items) |

Counts: **7 duplicated items** (the struct and 6 functions/operators), about 40 lines on each side, and **every one differs
in scalar type** (`f64` vs `f32`). crash-sim has 2 items render lacks; render has 3 `Vec3` items and all 10 `Mat4` items
(struct plus 9) that crash-sim lacks. crash-sim has no matrix type at all.

The scalar split is deliberate, not drift: crash-sim's output is frozen byte for byte (`f64`, nb-31's fixture in
`tests/solver.rs`; docs/STAGE6.md:306 "the default CSV byte for byte"), and render works in `f32` against golden images.
So one shared `Vec3` would have to be generic over the scalar (std only, so a hand-written float trait for `sqrt`,
`sin_cos`, `tan`) or ship two types. Either way the saving is about 40 lines.

Rotation code that looks similar but is not a duplicate: crash-sim tilts the cube by hand about x then z
(`src/lib.rs:216-224`, `f64`); render has `Mat4::rotation_y` (`f32`) and `wire_cube` rotates about y by hand
(`src/scene.rs:7-20`). Different axes, different precision. Moving crash-sim's tilt onto a shared matrix would change
the order of floating-point operations and therefore the frozen CSV.

Usage, to size any move: `Vec3` appears 44 times in 5 crash-sim files (src and tests); `Vec3|Mat4` appear 288 times in 18
render files (src, tests, examples). render's `math` is public and re-exported (`src/lib.rs:16`, `:26`) although its doc
comment calls it private to the crate (`src/math.rs:1`).

The only linear algebra in crash-sim is test-only: a 24x24 dense power iteration in `tests/stability.rs:132-160`
(29 lines) that checks the stiffest-mode eigenvalue constant.

### File formats

| Format | Written by | Read by | Duplicated code |
| --- | --- | --- | --- |
| crash-sim summary CSV `t,min_y,center_y,max_beam_strain` | crash-sim `src/lib.rs:352-361` | nobody else | none |
| Trajectory v1 `t,node,x,y,z` | crash-sim `examples/trajectory.rs:19` (header), `:26` (row) | render `src/replay.rs:22` (`HEADER`), `:49-124` (`Trajectory::parse`) | none: one writer, one reader |

No format **code** is duplicated today. What is duplicated is **knowledge**:

- The header string `t,node,x,y,z` exists twice (crash-sim `examples/trajectory.rs:19`, render `src/replay.rs:22`) with
  nothing tying them together.
- The cube's node numbering (bit 0 = x, bit 1 = y, bit 2 = z) is encoded three times: crash-sim `src/lib.rs:207-214`,
  render `src/replay.rs:11-13` and `:151-165` (`cube_edges`), render `src/scene.rs:10-16`.
- render tests against `tests/fixtures/trajectory.csv`, a frozen 409-line copy of crash-sim output (header plus 51
  frames of 8 nodes). Nothing re-checks it against what crash-sim writes now: if crash-sim's writer changed, render's
  tests would still pass.

### What the next briefs add

- nb-33 and nb-38: `.bsn` body and car format. Parser and writer in crash-sim only (`src/format.rs`). nb-38 says
  render's procedural car "is a visual reference only; do not import it".
- nb-37: trajectory v2 (node count and beam list in the header). crash-sim writes it, render parses it. Still one writer
  and one reader, but the format changes, which is exactly where the frozen fixture could drift.
- nb-40: render's drive viewer must run the sim live, "in-process (render depends on crash-sim by path inside
  `projects/`, relative, only if that is acceptable for the project layout; otherwise drive through a pipe". This ADR
  answers that question (below: not acceptable today).
- nb-39: the suspension acts along the chassis' local up axis, which needs `cross` and a normalise in `f64`. crash-sim
  has neither; nb-39's claims do not include `src/vec3.rs`.
- nb-42 and nb-43: the racing game, inside crash-sim or a new `projects/racing` repo "that depends on crash-sim by a
  relative path". Track format and lap timer are crash-sim (or racing) only.

So the overlap does not grow into shared format code in Stage 6 or Stage 7 m1, unless nb-40 or nb-42 choose a path
dependency between repos.

## The fact that decides most of this: relative paths inside task worktrees

Each project is its own Git repo in `projects/` (`.gitignore:15-16`, `broadsword.toml:14`). All coding happens in task
worktrees at `<worktrees_dir>/<project>/<task id>` (`broadsword.toml:42-43`), and the queue checks the worktree
(`check <project> --task`, `crates/broadsword/src/queue.rs:867-876`) before it merges.

The main checkout is 2 folders below the root (`projects/render`); a worktree is 3 (`worktrees/render/<task id>`).
A relative `path =` in a committed `Cargo.toml` is resolved from the folder that holds it, so the same string points to
different places:

| `path =` in render's `Cargo.toml` | from `projects/render` | from `worktrees/render/<id>` |
| --- | --- | --- |
| `../crash-sim` or `../core` | `projects/crash-sim` (exists) | `worktrees/render/crash-sim` (does not exist) |
| `../../crates/core` | `crates/core` (exists) | `worktrees/crates/core` (does not exist) |
| `../../../projects/crash-sim` | outside the root (wrong) | `projects/crash-sim` (exists) |

No single relative string works in both, and absolute paths are banned (AGENTS.md:75). So **any dependency from one
project repo to a crate outside that repo fails to build in every task worktree**, which is where every task is built
and checked. Cargo fails loudly ("failed to read .../Cargo.toml"), so this cannot slip through silently, but it blocks
the work.

A possible workaround, **not verified**: a `[patch]` table in the root `.cargo/config.toml` (which already holds
`[build] target-dir = "build"` and is found from both depths, since Cargo searches parent folders). Even if it works,
it points every build at the **main checkout's working tree** of the shared crate, uncommitted edits included, not at a
commit: a worktree that changes the shared crate cannot test its consumers, and consumers build against whatever is on
disk. A spike would have to prove both the resolution and the offline build before anyone relies on it.

The other costs, measured from the code and the board:

- **One project per task, one project per queue entry.** A change that spans two repos is already split today: nb-37,
  nb-40 and nb-41 became 6 board tasks (`[crash-sim half]`, `[render half]`, `[broadsword half]`). The queue checks only
  the entry's project, before merge (`queue.rs:867-876`) and on main after merge (`queue.rs:1199-1210`). A shared crate
  in its own project merges after its own check only; its consumers are not rebuilt until their next task.
- **Shared build folder.** Broadsword cleans and notes only the packages of the folder's own workspace
  (`crates/broadsword/src/buildsrc.rs:30-31`): "a path dependency that lives outside the workspace is not a member, so
  it is neither noted nor cleaned". For a library this is probably harmless (Cargo's artifact hash includes the package
  source path, so two checkouts get two `.rlib`s), but that is expected, not tested here.
- **Check time.** From the 475 job records in `state/jobs/` (status `done`): render `test` median 17 s (n = 12),
  crash-sim `test` median 4 s (n = 20). Building both on every change would cost seconds, not minutes.

## Options

Scored on the brief's criteria. "Worktree" means: does a task in a task worktree build with it, with no Broadsword change.

| | (a) Keep copies per repo | (b) Crate in the Broadsword repo, by relative path | (c) Own repo `projects/core`, by relative path | (d1) One Cargo workspace at the Broadsword root over the separate repos | (d2) One workload repo: a Cargo workspace holding core, crash-sim and render |
| --- | --- | --- | --- | --- | --- |
| Builds in task worktrees | yes | **no** (path differs by depth) | **no** (path differs by depth) | **no** (a worktree is under the root workspace but not a member) | yes (the path stays inside the repo) |
| Portability (relative to root, moves drives) | unchanged | relative, OK | relative, OK; one more repo to `worktree repair` (docs/MOVING.md) | relative, OK | relative, OK; one repo fewer to repair |
| Claims | no shared file | core edits are `broadsword` tasks, consumer edits are other projects' tasks: never one task | core is its own project: never one task with its consumers | per repo, as now | a task can claim core and consumer files together; only tasks editing the same core file serialize |
| Shared build folder | unchanged | core is outside consumers' workspaces: not noted or cleaned (buildsrc.rs:30-31) | same as (b) | `cargo metadata --no-deps` lists every member, so one project's job cleans all of them | members are listed, noted and cleaned (closes the buildsrc.rs:30-31 gap for these crates) |
| std only | yes | yes | yes | yes | yes |
| Cross-repo change in the queue | a format change is two tasks and two entries (today's practice) | core change checked by `check broadsword` only; crash-sim's CSV fixture and render's goldens not run | core change checked by `check core` only; consumers not rebuilt | one `Cargo.lock` in the Broadsword repo for code in other repos | one entry, one check builds and tests every member: crash-sim's CSV fixture and render's goldens run on every change |
| Migration now | none | new crate plus an unverified config workaround | new repo plus the same workaround | remove the `[workspace]` detachment from every project (crash-sim `Cargo.toml:9-10`, render `Cargo.toml:9-10`) | merge two histories (crash-sim 20 commits on `main`, render 9 on `master`: different branch names), with the queue idle |
| Main risk | copies and fixtures drift | builds use the main checkout's working tree; mixes workload code into the platform repo; `shared/*` is git-ignored (`.gitignore:9-10`) and AGENTS.md:47 reserves `shared/` for read-only copies, so it would really be `crates/core` | same as (b), plus no version pinning: a core merge silently changes what render builds | does not work with worktrees at all | a disruptive move mid-Stage 6 for about 40 lines of saving |

(d1) is ruled out: it fails in worktrees and couples the platform repo to every workload repo. (b) and (c) are ruled
out under today's worktree layout: they cannot build in a worktree without an unverified workaround that, even if it
works, builds against uncommitted files. That leaves (a) and (d2).

Cost of (d2) if done now, from the board: 12 open and 1 running crash-sim tasks, 2 open render tasks, and 1 live
crash-sim worktree would all have to be finished or re-pointed first; 4 command lines in crash-sim `docs/STAGE6.md` and
12 lines in 8 Broadsword files name `projects/crash-sim` or `projects/render`; the project names on the board change.
The move must happen with the merge queue idle (playbook lesson 29).

## Recommendation

**(a) Keep copies now, with the formats as the contract between repos. Take (d2), not (b) or (c), when a trigger below
fires.**

Reasons:

1. The real overlap is 7 small items, all in a different scalar type; sharing them saves about 40 lines and needs
   generic code. No format code is duplicated, and Stage 6 m1-m3 and Stage 7 m1 keep one writer and one reader per
   format.
2. Every sharing option except (d2) breaks in task worktrees, where all work is built and checked.
3. (d2) works, but costs a repo merge with 15 open tasks in flight, mid-Stage 6, to save 40 lines.
4. crash-sim's byte-identical output (nb-31 fixture, STAGE6.md:306) makes any move of its math a risk with no gain
   today.

What follows for briefs already on the board:

- **nb-40**: the drive viewer talks to crash-sim through a pipe (crash-sim as a child process: input lines in,
  trajectory rows out), not in-process. A path dependency from render to crash-sim does not build in render's worktree.
  Size check: a car of a few dozen nodes (nb-38) at 60 frames per second is about 50 x 40 bytes x 60, roughly 120 KB/s
  through the pipe.
- **nb-42**: the racing game lives inside crash-sim (a module and a binary), not in a separate repo with a path
  dependency, for the same reason. If Lazlo picks (d2), racing can be a workspace member instead.
- **nb-39**: if it needs `cross` or a normalise, it adds `f64` versions to crash-sim `src/vec3.rs`, and its claims must
  include that file (they do not today). It must not touch `dot`, `length` or the operators, whose order of operations
  the CSV fixture freezes.
- **nb-37**: the trajectory format changes here, so this is where the writer and the reader's frozen fixture can drift.
  Follow-up F2 below closes that.

## The numerics core (roadmap node 2.8): needed for Stage 6 now?

**No, not for Stage 6 milestones 1 to 3.** The solver is explicit symplectic Euler (crash-sim `src/solver.rs:94-127`): no
matrix and no linear solve. The step limits are closed form (`src/lib.rs:84-95`) and a Gershgorin sum per node
(`src/solver.rs:142-151`), which nb-39 extends to suspension springs and wheel masses without an eigen-solver. The only
matrix code is the test-only power iteration (`tests/stability.rs:132-160`). CAPABILITY_ROADMAP.md:304 ("a solid numerics
core makes stage 6 much faster") assumes an implicit or FEM solver, which is Stage 6 m5 ("soft bodies or FEM").

Decide it when the m4 or m5 done checks are written: m5 (implicit or FEM) needs sparse linear algebra, and m4 may need
signal filtering of measured crash pulses. If only crash-sim uses it, it lives inside crash-sim (`src/numerics/`). It
becomes shared code only when a second project needs it, and that is a (d2) trigger.

## What would make this recommendation wrong

Any one of these fires the move to (d2), through ADR 0002 and the F5 brief below:

1. A second project must run the sim or parse `.bsn` as a library: the pipe in nb-40 cannot hold the viewer's frame
   rate, or the Studio (stage 11) needs the sim in-process.
2. A third Rust project needs the same `Vec3` or the same parser (for example, racing as its own crate after all).
3. A bug is fixed in one copy of the math or of the cube numbering and the other copy has the same bug.
4. A numerics core is needed by two projects.
5. A Broadsword change makes relative paths resolve the same in worktrees and main checkouts (for example a worktree
   layout of one folder per task holding sibling checkouts), or a spike proves the root `.cargo/config.toml` `[patch]`
   route builds offline from both depths **and** against a commit rather than a working tree. Then (c) becomes cheaper
   and should be re-scored.

And the recommendation is wrong today if the pipe in nb-40 turns out to break determinism: the saved input script must
be stamped in simulation time, not wall-clock time, so that the headless replay (STAGE6 check 3.8) gives the same final
position.

## What must never happen, whichever option is chosen

- crash-sim's default CSV changes by one byte because math moved (nb-31's fixture in `tests/solver.rs`).
- render's golden images change because math moved (`f32` order of operations).
- An absolute path in any `Cargo.toml` or Cargo config (AGENTS.md:75), including as a "temporary" fix for the
  worktree problem.
- A project builds against another repo's uncommitted working tree as a dependency.
- A migration deletes or rewrites the history of crash-sim or render (AGENTS.md: never delete work or history). The old
  repos stay as they are, read-only.
- A migration while the merge queue is working on either repo (playbook lesson 29).

## Follow-up briefs

For the recommendation, (a):

| Id | Title | Claims | When |
| --- | --- | --- | --- |
| F1 | Amend nb-40 (both halves): `the drive viewer runs crash-sim as a child process: input lines in (sim-time stamped), trajectory v2 rows out` | unchanged from nb-40 (`projects/crash-sim/src/vehicle.rs`, `projects/crash-sim/src/input.rs`, `projects/crash-sim/src/main.rs`, `projects/crash-sim/tests/drive.rs`, `projects/render/examples/drive.rs`), plus `projects/crash-sim/docs/STAGE6.md` (check 3.7's command names the crash-sim program and shows it was built from crash-sim main) | before nb-40 starts |
| F2 | `crash-sim: trajectory writer in the library with a fixture test; render's trajectory fixtures are byte copies, checked by cmp in the Stage 6 checks` | `projects/crash-sim/src/trajectory.rs` (new), `projects/crash-sim/examples/trajectory.rs`, `projects/crash-sim/tests/trajectory.rs` (new), `projects/crash-sim/tests/fixtures/` (new), `projects/crash-sim/docs/STAGE6.md` (one `cmp` line in 2.8) | fold into nb-37's crash-sim half, which already claims `examples/trajectory.rs` |
| F3 | Amend nb-42: `racing lives inside crash-sim (module and binary)` | unchanged: `projects/crash-sim/docs/STAGE7.md` (new) | before nb-42 starts |
| F4 | Amend nb-39: add `projects/crash-sim/src/vec3.rs` to its claims if it adds `cross` or a normalise in `f64` | `projects/crash-sim/src/vec3.rs` added to nb-39's list | before nb-39 starts |

Only when a trigger above fires, or now if Lazlo picks (d2) instead:

| Id | Title | Claims | Done check (outline) |
| --- | --- | --- | --- |
| F5 | `ADR 0002 and spike: one workload repo with a Cargo workspace (crash-sim, render, later core), histories kept` | `docs/adr/0002-workload-workspace.md` (new); the spike works on copies in `scratch/` only | in the scratch copy: both test suites pass from the workspace root through `broadsword`, the default CSV is byte-identical, golden images unchanged, `git log --follow` shows each file's old history, a task worktree of the new repo builds with a member-to-member path dependency |
| F6 | `move crash-sim and render into projects/<new repo> as workspace members, queue idle` | `projects/<new repo>` (new), `docs/MOVING.md`, `PROGRESS.md`, crash-sim `docs/STAGE6.md` (paths), the old repos read-only (nothing deleted) | after F5 passes and with no running task in either repo: the same checks as F5 on the real repos, the board's open tasks re-pointed, each Git command beyond a plain commit explained in `LAB_LOG.md` |
| F7 | `core: Vec3 (and later numerics) as a workspace member; crash-sim and render use it` | `projects/<new repo>/core/` (new), the two `math`/`vec3` files | crash-sim CSV byte-identical, render goldens unchanged, one check runs both |

## How the counts were taken

All from the root, read-only (the `projects/` repos are not part of the Broadsword repo).

- Math items: `grep -n "pub fn\|pub const\|^impl\|derive" projects/crash-sim/src/vec3.rs projects/render/src/math.rs`,
  then each file read in full.
- Usage: `grep -rc "Vec3" projects/crash-sim/{src,tests,examples}` (44 in 5 files) and
  `grep -rcE "Vec3|Mat4" projects/render/{src,tests,examples}` (288 in 18 files).
- Format writers and readers: `grep -rn "t,node,x,y,z" projects/crash-sim` plus reading `projects/render/src/replay.rs`;
  `wc -l projects/render/tests/fixtures/trajectory.csv` (409).
- Worktree depth: `broadsword.toml:42-43` and the folders under `worktrees/` (`worktrees/crash-sim/<task id>`).
- Split tasks: the `title` lines containing `half]` in `state/tasks/*.task` (6 tasks for 3 features).
- Open work: `status` of every `state/tasks/*.task` whose `project` is crash-sim (12 open, 1 running) or render (2 open).
- Check time: `started` and `ended` of every `state/jobs/*-render-test.rec` and `*-crash-sim-test.rec` with
  `status = done` (medians 17 s, n = 12, and 4 s, n = 20).
- History: `git -C projects/crash-sim rev-list --count main` (20), `git -C projects/render rev-list --count HEAD` (9, on
  `master`).
- Path references: `grep -rlE "projects/(render|crash-sim)"` in the Broadsword repo, excluding `build` and `scratch`
  (8 files, 12 lines); the same pattern plus `\.\./crash-sim` in crash-sim `docs/STAGE6.md` (4 lines).
