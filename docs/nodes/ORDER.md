# Order of work across the roadmap nodes

Board task `t-20261120-050525`, written 2026-11-20. This file orders the nodes; it decides nothing. `ROADMAP.md` and `CAPABILITY_ROADMAP.md` are not changed by it, and the two open suggestions in `CAPABILITY_ROADMAP.md` Part 4 stay Lazlo's decisions (section 5).

**Sources, and only these.** Every edge below comes from a file in this repo, cited by file and line:

- `CAPABILITY_ROADMAP.md` Part 5, the `Needs:` line of each node's prompt (lines 346 to 918). Part 2's "Builds on" column (lines 147 to 241) names the same nodes for Tiers 2 to 5: a script compared all 51 rows and found no difference. Tiers 0 and 1 have no "Builds on" column, so Part 5 is the only roadmap source for them.
- The node plans in `docs/nodes/`: section 3 (Prerequisites) and section 7 (First briefs) of `1.1.md` to `1.8.md` and `sandbox.md`. `0.1.md` has no numbered sections. `README.md` and `REFERENCE_DATA.md` are not node plans.
- Where the order needs more than the plans say, `ROADMAP.md` (stage and status columns), `DEVELOPMENT_PLAN.md:21-36` (the order stages are built in) and the ADRs `docs/adr/0001-shared-core.md` (Accepted) and `docs/adr/0004-roadmap-repo-map.md` (Proposed) are quoted, and named as such.

Where a file states no prerequisite, this file says "not stated". Nothing here is inferred from what a node "obviously" needs.

**Context.** Tier 0 has started: a `projects/foundation` repo exists with module stubs (outside this worktree; not read for this file). Every Tier 1 plan says it never links into it, for example `1.7.md:227` ("the plan never links into `projects/foundation`") and `1.8.md:244`.

**How the numbers were made.** `scratch/order_graph.py` parses every `Needs:` line, expands the one range (`0.1–0.4` on line 422 means 0.1, 0.2, 0.3, 0.4), and keeps only node ids as edges. Words that name no node are not edges: "and at least two studios from Tier 4" (5.1, line 894), "build in thin slices as each app comes online" (5.3, line 910), "and the studios built so far" (5.4, line 918). The scratch folder can be wiped; the method is written out in section 3 so it can be redone by hand.

## 1. Node table

Every node in `ROADMAP.md` (71 rows, 0.1 to 5.4) once, plus `sandbox`, the one plan file in `docs/nodes/` that is not a roadmap node. "Stage" and "Status" are copied from `ROADMAP.md`. "Roadmap Needs" is the Part 5 `Needs:` text with its line in `CAPABILITY_ROADMAP.md`. "Node plan" is what the plan's section 3 says the node really uses, with its lines; "no plan file" means `docs/nodes/` has no plan for that node.

| Node | Name (ROADMAP.md) | Stage | Status | Roadmap Needs (`CAPABILITY_ROADMAP.md` line) | Node plan, section 3 |
| --- | --- | --- | --- | --- | --- |
| 0.1 | Build and repo discipline | 1 | in progress | nothing (line 348) | `0.1.md` has no section 3; prerequisites not stated there |
| 0.2 | Math library | 1 | in progress | 0.1 (line 356) | no plan file |
| 0.3 | Data structures and memory | 1 | not started | 0.1, 0.2 (line 364) | no plan file |
| 0.4 | Job system | 1 | not started | 0.1, 0.3 (line 372) | no plan file |
| 0.5 | Serialization and asset format | 1 | not started | 0.1, 0.3 (line 380) | no plan file |
| 0.6 | Command and undo system | 1 | not started | 0.1, 0.5 (line 388) | no plan file |
| 0.7 | Plugin/module system | 1 | not started | 0.1 (line 396) | no plan file |
| 0.8 | Scripting and automation API | 1, 2 | in progress | 0.6, 0.7 (line 404) | no plan file |
| 0.9 | Logging, tracing, test harness | 1, 3 | in progress | 0.1 (line 412) | no plan file |
| 1.1 | Window and input | 1 | not started | 0.1–0.4 (line 422) | `1.1.md:214-223`: uses only 0.1; 0.2, 0.3, 0.4 "Nothing"; "treat 0.1 as the only hard prerequisite" until Lazlo decides |
| 1.2 | GPU abstraction | 4 | not started | 1.1 (line 430) | `1.2.md:151-156`: 1.1 "None for the Done-when"; uses 0.1; 0.9 "The idea only"; 0.2 "None" |
| 1.3 | 2D vector renderer | 4 | not started | 1.2 (line 438) | `1.3.md:197-204`: 1.2 nothing for c1 to c14, "the whole of 1.2's Done-when" for c15 (GPU); uses 0.1; 0.9 pattern only; 0.2 nothing shared |
| 1.4 | Text and fonts | 4 | not started | 1.3 (line 446) | `1.4.md:197-203`: 1.3 "None for the Done-when, by choice" (decision D6); uses 0.1; 0.9 idea only; 0.2, 1.1, 1.2, 1.5 "None" |
| 1.5 | UI framework and editor shell | 1 | not started | 1.1, 1.4, 1.6, 0.6 (line 454) | `1.5.md:312-324`: 1.1, 1.4, 1.6, 0.6 as plain-data interfaces only, "briefs S1 to S9 need none of their code"; uses 0.1; names 2.4, 0.5 (`CoreAccess` only) and 0.7 (overlap, a decision) |
| 1.6 | Reflection and properties | 1 | not started | 0.5 (line 462) | `1.6.md:169-176`: 0.5 "Only the idea", recommends not waiting; uses 0.1; 0.9 idea only; 0.2 "None" |
| 1.7 | Node graph framework | none | not started | 1.5, 1.6, 0.6 (line 470) | `1.7.md:265-275`: 0.6, 1.5, 1.6 "Nothing from its code"; the panel brief waits for 1.5; does not depend on 0.2, 0.4, 0.5, 2.1 |
| 1.8 | Timeline framework | 9 | not started | 1.5, 0.6 (line 478) | `1.8.md:278-290`: 1.5, 0.6 nothing of their code; "treat 0.1 as the only hard prerequisite"; 0.5, 0.2, 1.1, 1.9, 1.10 "Nothing" |
| 1.9 | Media I/O | 9 | not started | 0.5 (line 486) | no plan file |
| 1.10 | Audio device layer | 9 | not started | 0.4 (line 494) | no plan file |
| 1.11 | Color management | 4 | not started | 1.2 (line 502) | no plan file |
| 2.1 | Raster image engine | 4 | not started | 1.2, 1.7, 1.11 (line 512) | no plan file |
| 2.2 | Brush engine | 4 | not started | 1.1, 2.1 (line 520) | no plan file |
| 2.3 | Mesh kernel | 5 | not started | 0.2, 0.3, 0.6 (line 528) | no plan file |
| 2.4 | Scene system | 5 | not started | 0.5, 1.6 (line 536) | no plan file |
| 2.5 | Real-time renderer | 4 | in progress | 1.2, 2.4 (line 544) | no plan file |
| 2.6 | Animation core | 5 | not started | 1.8, 2.4 (line 552) | no plan file |
| 2.7 | Audio DSP engine | 9 | not started | 1.7, 1.10 (line 560) | no plan file |
| 2.8 | Numerics core | 10 | not started | 0.2, 0.4 (line 568) | no plan file |
| 2.9 | 2D constraint solver and sketcher | 8 | not started | 2.8, 1.3 (line 576) | no plan file |
| 2.10 | Code editor core | 3 | not started | 1.4, 1.5 (line 584) | no plan file |
| 3.1 | Rigid-body physics | 6 | not started | 2.8, 0.3 (line 594) | no plan file |
| 3.2 | Soft body and deformables | 6 | in progress | 3.1, 2.3 (line 602) | no plan file |
| 3.3 | Fluids and smoke | 6 | not started | 2.8, 0.4 (line 610) | no plan file |
| 3.4 | Destruction | 6 | not started | 3.1, 2.3 (line 618) | no plan file |
| 3.5 | Path tracer | 5 | not started | 2.4, 2.5 (line 626) | no plan file |
| 3.6 | Procedural geometry | 5 | not started | 1.7, 2.3, 0.8 (line 634) | no plan file |
| 3.7 | Sculpting | 5 | not started | 2.3, 2.2 (line 642) | no plan file |
| 3.8 | Texture painting and material authoring | 5 | not started | 2.1, 2.2, 2.3, 2.5 (line 650) | no plan file |
| 3.9 | Rigging tools | 5 | not started | 2.6, 2.9 (line 658) | no plan file |
| 3.10 | B-rep CAD kernel | 8 | not started | 2.8 (line 666) | no plan file |
| 3.11 | Parametric feature tree | 8 | not started | 2.9, 3.10, 0.6 (line 674) | no plan file |
| 3.12 | Assemblies and drawings | 8 | not started | 3.11, 1.3 (line 682) | no plan file |
| 3.13 | CAM | 8 | not started | 3.10 (line 690) | no plan file |
| 3.14 | EDA | 8 | not started | 2.9, 1.3, 0.3 (line 698) | no plan file |
| 3.15 | Compositor and color grading | 9 | not started | 1.7, 2.1, 1.11 (line 706) | no plan file |
| 3.16 | Nonlinear video editing | 9 | not started | 1.8, 1.9 (line 714) | no plan file |
| 3.17 | Motion graphics | 9 | not started | 1.8, 1.3, 0.8 (line 722) | no plan file |
| 3.18 | DAW features | 9 | not started | 2.7, 1.8 (line 730) | no plan file |
| 3.19 | FEM solver | 10 | not started | 2.8, 2.3 (line 738) | no plan file |
| 3.20 | CFD | 10 | not started | 2.8, 0.4 (line 746) | no plan file |
| 3.21 | Multiphysics | 10 | not started | 3.19, 3.20 (line 754) | no plan file |
| 3.22 | Block-diagram simulation | 10 | not started | 1.7, 2.8 (line 762) | no plan file |
| 3.23 | Matrix language and notebook | 10 | not started | 2.8, 0.8, 1.3 (line 770) | no plan file |
| 3.24 | GIS core | 10 | not started | 0.3, 1.3 (line 778) | no plan file |
| 3.25 | Debugger and profiler | 10 | not started | 2.10, 0.9 (line 786) | no plan file |
| 3.26 | Semantic code index | 10 | not started | 2.10 (line 794) | no plan file |
| 4.1 | Game runtime | 7 | not started | 2.4, 2.5, 2.6, 2.7, 3.1 (line 804) | no plan file |
| 4.2 | Game editor | 7 | not started | 4.1, 1.5, 1.7 (line 812) | no plan file |
| 4.3 | DCC Studio | 5 | not started | 2.3, 2.6, 3.5, 3.6, 3.7, 3.8, 3.9 (line 820) | no plan file |
| 4.4 | Image and Texture Studio | 5 | not started | 2.1, 2.2, 3.8 (line 828) | no plan file |
| 4.5 | CAD Studio | 8 | not started | 3.10, 3.11, 3.12, 3.13 (line 836) | no plan file |
| 4.6 | Media Studio | 9 | not started | 3.15, 3.16, 3.17, 3.18 (line 844) | no plan file |
| 4.7 | Audio Studio | 9 | not started | 2.7, 3.18 (line 852) | no plan file |
| 4.8 | Sim and Analysis Studio | 10 | not started | 3.19, 3.20, 3.21, 3.22, 3.23 (line 860) | no plan file |
| 4.9 | Dev Studio | 10 | not started | 2.10, 3.25, 3.26 (line 868) | no plan file |
| 4.10 | EDA Studio | 8 | not started | 3.14 (line 876) | no plan file |
| 4.11 | Geo Studio | 10 | not started | 3.24 (line 884) | no plan file |
| 5.1 | Unified asset database | 12 | not started | 0.5, and at least two studios from Tier 4 (line 894) | no plan file |
| 5.2 | Live link and round-trip | 12 | not started | 5.1 (line 902) | no plan file |
| 5.3 | AI agent layer | 2, 12 | not started | 0.8; build in thin slices as each app comes online (line 910) | no plan file |
| 5.4 | Studio UI shell | 11 | not started | 1.5, and the studios built so far (line 918) | no plan file |
| sandbox | Plugin sandbox (Windows), from 1.5 risk R13 (`sandbox.md:1`, `sandbox.md:13`) | not stated | not stated | not a roadmap node: no Needs line | `sandbox.md:281-289`: 1.5 briefs P1 to P3 merged ("starts only after P3 merges"), 1.5 S1, S4, S5; 0.7 overlap (decide before X2); Lazlo's decisions in its section 4 |

## 2. Dependency graph (edges)

### 2a. Roadmap edges (graph A)

154 edges, one line per node: `prerequisites -> node (CAPABILITY_ROADMAP.md line of its Needs)`. This is the roadmap as written, before any waiver.

- none -> 0.1 (line 348: "nothing")
- 0.1 -> 0.2 (line 356)
- 0.1, 0.2 -> 0.3 (line 364)
- 0.1, 0.3 -> 0.4 (line 372)
- 0.1, 0.3 -> 0.5 (line 380)
- 0.1, 0.5 -> 0.6 (line 388)
- 0.1 -> 0.7 (line 396)
- 0.6, 0.7 -> 0.8 (line 404)
- 0.1 -> 0.9 (line 412)
- 0.1, 0.2, 0.3, 0.4 -> 1.1 (line 422)
- 1.1 -> 1.2 (line 430)
- 1.2 -> 1.3 (line 438)
- 1.3 -> 1.4 (line 446)
- 1.1, 1.4, 1.6, 0.6 -> 1.5 (line 454)
- 0.5 -> 1.6 (line 462)
- 1.5, 1.6, 0.6 -> 1.7 (line 470)
- 1.5, 0.6 -> 1.8 (line 478)
- 0.5 -> 1.9 (line 486)
- 0.4 -> 1.10 (line 494)
- 1.2 -> 1.11 (line 502)
- 1.2, 1.7, 1.11 -> 2.1 (line 512)
- 1.1, 2.1 -> 2.2 (line 520)
- 0.2, 0.3, 0.6 -> 2.3 (line 528)
- 0.5, 1.6 -> 2.4 (line 536)
- 1.2, 2.4 -> 2.5 (line 544)
- 1.8, 2.4 -> 2.6 (line 552)
- 1.7, 1.10 -> 2.7 (line 560)
- 0.2, 0.4 -> 2.8 (line 568)
- 2.8, 1.3 -> 2.9 (line 576)
- 1.4, 1.5 -> 2.10 (line 584)
- 2.8, 0.3 -> 3.1 (line 594)
- 3.1, 2.3 -> 3.2 (line 602)
- 2.8, 0.4 -> 3.3 (line 610)
- 3.1, 2.3 -> 3.4 (line 618)
- 2.4, 2.5 -> 3.5 (line 626)
- 1.7, 2.3, 0.8 -> 3.6 (line 634)
- 2.3, 2.2 -> 3.7 (line 642)
- 2.1, 2.2, 2.3, 2.5 -> 3.8 (line 650)
- 2.6, 2.9 -> 3.9 (line 658)
- 2.8 -> 3.10 (line 666)
- 2.9, 3.10, 0.6 -> 3.11 (line 674)
- 3.11, 1.3 -> 3.12 (line 682)
- 3.10 -> 3.13 (line 690)
- 2.9, 1.3, 0.3 -> 3.14 (line 698)
- 1.7, 2.1, 1.11 -> 3.15 (line 706)
- 1.8, 1.9 -> 3.16 (line 714)
- 1.8, 1.3, 0.8 -> 3.17 (line 722)
- 2.7, 1.8 -> 3.18 (line 730)
- 2.8, 2.3 -> 3.19 (line 738)
- 2.8, 0.4 -> 3.20 (line 746)
- 3.19, 3.20 -> 3.21 (line 754)
- 1.7, 2.8 -> 3.22 (line 762)
- 2.8, 0.8, 1.3 -> 3.23 (line 770)
- 0.3, 1.3 -> 3.24 (line 778)
- 2.10, 0.9 -> 3.25 (line 786)
- 2.10 -> 3.26 (line 794)
- 2.4, 2.5, 2.6, 2.7, 3.1 -> 4.1 (line 804)
- 4.1, 1.5, 1.7 -> 4.2 (line 812)
- 2.3, 2.6, 3.5, 3.6, 3.7, 3.8, 3.9 -> 4.3 (line 820)
- 2.1, 2.2, 3.8 -> 4.4 (line 828)
- 3.10, 3.11, 3.12, 3.13 -> 4.5 (line 836)
- 3.15, 3.16, 3.17, 3.18 -> 4.6 (line 844)
- 2.7, 3.18 -> 4.7 (line 852)
- 3.19, 3.20, 3.21, 3.22, 3.23 -> 4.8 (line 860)
- 2.10, 3.25, 3.26 -> 4.9 (line 868)
- 3.14 -> 4.10 (line 876)
- 3.24 -> 4.11 (line 884)
- 0.5 -> 5.1 (line 894)
- 5.1 -> 5.2 (line 902)
- 0.8 -> 5.3 (line 910)
- 1.5 -> 5.4 (line 918)

### 2b. What the plans change (graph B)

Each Tier 1 plan says its first briefs need none of the code of its roadmap prerequisites, and asks Lazlo to confirm a waiver before the node starts. Graph B is graph A with every one of those waivers accepted: the Needs edges into 1.1 to 1.8 are replaced by what each plan's section 3 lists as used, which is 0.1 for seven of them and nothing for 1.7 (its section 3 has no 0.1 row: not stated). Graph B has 144 edges.

| Node | Roadmap edges removed in graph B | Plan text that asks for it | Decision named in the plan |
| --- | --- | --- | --- |
| 1.1 | 0.2, 0.3, 0.4 | "Until then, treat 0.1 as the only hard prerequisite." (`1.1.md:223`) | "either "0.2-0.4 are waived for 1.1" or "wait for them"" (`1.1.md:223`) |
| 1.2 | 1.1 (0.1 added) | "The recommendation is to record that 1.2 does not wait for 1.1" (`1.2.md:153`) | Lazlo may confirm (`1.2.md:153`) |
| 1.3 | 1.2 (0.1 added) | "either "CPU first, GPU after 1.2" (the plan's recommendation) or "wait for 1.2"" (`1.3.md:204`) | same line |
| 1.4 | 1.3 (0.1 added) | "The recommendation is the copy, to keep the node startable now." (`1.4.md:199`) | decision D6 (`1.4.md:199`) |
| 1.5 | 1.1, 1.4, 1.6, 0.6 (0.1 added) | "briefs S1 to S9 need none of their code" (`1.5.md:324`) | "waive 1.1, 1.4, 1.6 and 0.6 for S1 to S10; wire them after ADR 0002", or "wait for them" (`1.5.md:324`) |
| 1.6 | 0.5 (0.1 added) | "The recommendation is to record that 1.6 does not wait for 0.5" (`1.6.md:171`) | Lazlo may confirm (`1.6.md:171`) |
| 1.7 | 1.5, 1.6, 0.6 (nothing added: not stated) | "Until then, none of them is a dependency." (`1.7.md:273`) | "0.6, 1.5, 1.6 are waived for 1.7 (graph model and editor model only; the panel waits for 1.5)" or "wait for them" (`1.7.md:273`) |
| 1.8 | 1.5, 0.6 (0.1 added) | "Until then, treat 0.1 as the only hard prerequisite (risk R1)." (`1.8.md:290`) | "1.5 and 0.6 are waived for 1.8's framework" or "wait for them" (`1.8.md:290`) |

"0.1 added" means the roadmap's Needs line for that node does not name 0.1, but the plan's section 3 lists 0.1 as used (`1.2.md:154`, `1.3.md:200`, `1.4.md:200`, `1.5.md:320`, `1.6.md:172`, `1.8.md:284`; `1.1.md:218` already has it). `1.7.md`'s section 3 (lines 265 to 275) has no 0.1 row, so graph B gives 1.7 no prerequisite at all; adding 0.1 would change no path length in section 3.

### 2c. Edges the plans add that the roadmap does not have

These are later parts of a node, not its first briefs, so they are listed here and not counted in graph A or B:

- 1.2 -> 1.3's GPU backend (check c15 and the briefs after V10): "For c15 (GPU): the whole of 1.2's Done-when" (`1.3.md:199`).
- 1.5 -> 1.7's editor panel: "a **later brief that starts only after 1.5 exists**" (`1.7.md:270`).
- 1.5 -> 1.8's on-screen widget: "What 1.5 must later supply is a place to put the widget" (`1.8.md:282`).
- 1.5 briefs P1, P2, P3, S1, S4, S5 -> sandbox: "This node edits P2's files, so it starts only after P3 merges." (`sandbox.md:283`).
- 0.7 <-> sandbox and 0.7 <-> 1.5 briefs P1 to P3: an overlap, not an edge: "If 0.7 takes the plugin mechanism, this node goes with it; decide before X2." (`sandbox.md:286`); "Decision for Lazlo: either P1 to P3 are 0.7's first part, built here and moved into core with ADR 0002, or they wait for 0.7." (`1.5.md:322`).
- ADR 0002 -> any in-process use of 1.7 or 1.8 by another repo: "ADR 0002 must be decided before 1.5 can host the graph editor." (`1.7.md:394`); "a live editor cannot embed the Rust view without ADR 0002" (`1.8.md:388`).

### 2d. Where the sources disagree (facts, not fixed here)

- 0.4: `CAPABILITY_ROADMAP.md:372` says "Needs: 0.1, 0.3"; ADR 0004's table says "Depends on nothing else" (`docs/adr/0004-roadmap-repo-map.md:78`). Graph A uses the roadmap.
- 1.5 is mapped to stage 1 (`ROADMAP.md:24`) but needs 1.4, mapped to stage 4 (`ROADMAP.md:23`); `DEVELOPMENT_PLAN.md:24` and `:27` build stage 1 before stage 4. Section 5 lists every edge of this kind.
