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

## 3. Longest path

**Method.** Each node counts 1; a path's length is its number of nodes. For every node, its level is 1 if it has no prerequisite, otherwise 1 + the highest level among its prerequisites (a memoised depth-first walk over the edges of section 2; no cycle was found). The longest path is the chain behind the highest level; ties are all listed. Brief counts are not used as weights, because only nine nodes have plans with briefs. The levels are in section 4.

**Graph A (roadmap as written): 14 nodes, three ties.**

- 0.1 -> 0.2 -> 0.3 -> 0.4 -> 1.1 -> 1.2 -> 1.3 -> 1.4 -> 1.5 -> 1.7 -> 2.1 -> 2.2 -> 3.7 -> 4.3
- 0.1 -> 0.2 -> 0.3 -> 0.4 -> 1.1 -> 1.2 -> 1.3 -> 1.4 -> 1.5 -> 1.7 -> 2.1 -> 2.2 -> 3.8 -> 4.3
- 0.1 -> 0.2 -> 0.3 -> 0.4 -> 1.1 -> 1.2 -> 1.3 -> 1.4 -> 1.5 -> 1.7 -> 2.1 -> 2.2 -> 3.8 -> 4.4

What makes it long is Tier 1 as a single file: 1.1 needs 0.1 to 0.4 (line 422), then 1.2 needs 1.1 (430), 1.3 needs 1.2 (438), 1.4 needs 1.3 (446), 1.5 needs 1.4 (454), 1.7 needs 1.5 (470), and 2.1 needs 1.7 (512). **Estimate:** counted in brief rounds, the Tier 1 part of this chain alone, 1.1 to 1.5 then 1.7, is about 36 rounds. The sum is 1.1: 6 (Depends-on column `1.1.md:284-293`: W1; W2, W3, W4; W5, W10; W6, W7; W8; W9; the plan's prose says "five or six rounds", `1.1.md:295`) + 1.2: 6 (Dep column `1.2.md:205-214`: G1; G2, G5, G6; G3, G4; G7, G8; G9; G10) + 1.3: 5 (`1.3.md:264-273`: V1; V2 to V5, V9; V6; V7, V8; V10) + 1.4: 7 (`1.4.md:264-273`: X1; X2, X6; X3; X4, X5; X7; X8; X9, X10) + 1.5: 7 ("seven rounds", `1.5.md:424`) + 1.7: 5 ("Five waves", `1.7.md:365`) = 36. It assumes unlimited slots inside each plan and that each node starts only once the node before it is done, which is what the Needs lines above require when no waiver is given. It is an estimate, not a measurement: the rounds are the plans' planned waves, and no brief has run.

**Graph B (every Tier 1 waiver in section 2b accepted): 9 nodes, three ties.**

- 0.1 -> 0.2 -> 0.3 -> 0.4 -> 2.8 -> 2.9 -> 3.11 -> 3.12 -> 4.5
- 0.1 -> 0.2 -> 0.3 -> 0.4 -> 2.8 -> 3.10 -> 3.11 -> 3.12 -> 4.5
- 0.1 -> 0.2 -> 0.3 -> 0.5 -> 0.6 -> 2.3 -> 3.19 -> 3.21 -> 4.8

In graph B the Tier 1 chain is gone and the long paths run through Tier 0 and then numerics (2.8) into CAD, or through serialization and undo (0.5, 0.6) into the mesh kernel and simulation. 1.7 is on graph A's longest path; 2.8 is on two of graph B's. Section 5 uses both facts.

**Not counted, because the Needs line names no node.** 5.1 ("at least two studios from Tier 4", line 894), 5.3 ("as each app comes online", line 910) and 5.4 ("the studios built so far", line 918). If 5.1 waited for the two studios at the top of graph A (4.3 and 4.4, level 14), graph A's longest path would end 4.3 -> 5.1 -> 5.2: 16 nodes. Which studios count is not stated.

## 4. Lanes that can run side by side

### 4a. Waves of graph A

Every node in a wave has all its prerequisites in earlier waves, so all nodes of one wave can run side by side once the waves before it are done. ("Done" here means the dependency is met; `ROADMAP.md:5` says "Nothing is `done` yet", and the plans treat 0.1 as usable now, for example "Already usable" in `1.2.md:154`.)

| Wave | Nodes | Width |
| --- | --- | --- |
| 1 | 0.1 | 1 |
| 2 | 0.2, 0.7, 0.9 | 3 |
| 3 | 0.3 | 1 |
| 4 | 0.4, 0.5 | 2 |
| 5 | 0.6, 1.1, 1.6, 1.9, 1.10, 2.8, 5.1 | 7 |
| 6 | 0.8, 1.2, 2.3, 2.4, 3.1, 3.3, 3.10, 3.20, 5.2 | 9 |
| 7 | 1.3, 1.11, 2.5, 3.2, 3.4, 3.13, 3.19, 5.3 | 8 |
| 8 | 1.4, 2.9, 3.5, 3.21, 3.23, 3.24 | 6 |
| 9 | 1.5, 3.11, 3.14, 4.11 | 4 |
| 10 | 1.7, 1.8, 2.10, 3.12, 4.10, 5.4 | 6 |
| 11 | 2.1, 2.6, 2.7, 3.6, 3.16, 3.17, 3.22, 3.25, 3.26, 4.5 | 10 |
| 12 | 2.2, 3.9, 3.15, 3.18, 4.1, 4.8, 4.9 | 7 |
| 13 | 3.7, 3.8, 4.2, 4.6, 4.7 | 5 |
| 14 | 4.3, 4.4 | 2 |

5.1 sits in wave 5 only because its studios are not named (section 3).

### 4b. Waves of graph B

| Wave | Nodes | Width |
| --- | --- | --- |
| 1 | 0.1, 1.7 | 2 |
| 2 | 0.2, 0.7, 0.9, 1.1, 1.2, 1.3, 1.4, 1.5, 1.6, 1.8 | 10 |
| 3 | 0.3, 1.11, 2.10, 5.4 | 4 |
| 4 | 0.4, 0.5, 2.1, 3.24, 3.25, 3.26 | 6 |
| 5 | 0.6, 1.9, 1.10, 2.2, 2.4, 2.8, 3.15, 4.9, 4.11, 5.1 | 10 |
| 6 | 0.8, 2.3, 2.5, 2.6, 2.7, 2.9, 3.1, 3.3, 3.10, 3.16, 3.20, 3.22, 5.2 | 13 |
| 7 | 3.2, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 3.11, 3.13, 3.14, 3.17, 3.18, 3.19, 3.23, 4.1, 5.3 | 16 |
| 8 | 3.12, 3.21, 4.2, 4.3, 4.4, 4.6, 4.7, 4.10 | 8 |
| 9 | 4.5, 4.8 | 2 |

Graph B is only as true as the waivers: a wave-2 Tier 1 node whose waiver Lazlo refuses moves back to its graph A place. The later parts in section 2c (1.3's GPU backend, the 1.7 panel, the 1.8 widget on screen, the sandbox) are outside both tables.

### 4c. The lanes that can start now: the nine plans

These are the only nodes with first briefs written. Each plan creates its own new repo (ADR 0001 option a: "no Cargo dependency (path or git) on another repo", for example `1.7.md:227`), so their claims never overlap and the nine lanes can run side by side, subject to the gate in the last column. "Rounds" counts brief waves from each plan's Depends-on column with unlimited slots; "peak" is the most briefs that column lets run at once.

| Lane | Repo | Briefs | Rounds | Peak | Order stated in the plan | Gate before the first brief |
| --- | --- | --- | --- | --- | --- | --- |
| 1.1 | `projects/window-input` | W1 to W10 | 6 ("five or six rounds", `1.1.md:295`) | 3 | `1.1.md:295` | new repo, a step for Lazlo to approve (`1.1.md:284`); waiver (`1.1.md:223`) |
| 1.2 | `projects/gpu` | G1 to G10 | 6 by the Dep column (`1.2.md:205-214`); the prose at `1.2.md:201` puts G3 beside G2, but G3's Dep is G2 | 3 | `1.2.md:216` | decisions D1 (G1) and D2, D3 (G2) (`1.2.md:201`) |
| 1.3 | `projects/vector2d` | V1 to V10 | 5 by the Dep column; "about six rounds" with three slots (`1.3.md:275`) | 5 | `1.3.md:275` | new repo, Lazlo approves (`1.3.md:264`); V10 also needs Lazlo's suite and licence choice (`1.3.md:273`) |
| 1.4 | `projects/text` | X1 to X10 | 7 | 2 | `1.4.md:260`, `1.4.md:275` | D1 (X1); D2 to D4 (X2) (`1.4.md:260`) |
| 1.5 | `projects/shell` (project name, `1.5.md:408`) | S1 to S10, P1 to P3 | 7 ("seven rounds", `1.5.md:424`) | 3 | `1.5.md:424` | new repo, Lazlo approves (`1.5.md:408`); waiver and the undo-stack question (`1.5.md:324`); the 0.7 overlap (`1.5.md:322`) |
| 1.6 | `projects/reflect` | R1 to R10 | 5 by the Dep column (R9 needs R3 to R6 only); the prose orders R9 after R7 and R8, which makes 6 (`1.6.md:240`) | 4 | `1.6.md:240` | D1 (`1.6.md:229`) |
| 1.7 | `projects/nodegraph` | N1 to N10 | 5 ("Five waves, at most four briefs at once", `1.7.md:365`) | 4 | `1.7.md:365` | D1 (`1.7.md:354`); waiver (`1.7.md:273`) |
| 1.8 | `projects/timeline` | TL1 to TL10 | 4 ("four rounds", `1.8.md:363`) | 5 | `1.8.md:363` | new repo, Lazlo approves (`1.8.md:350`); waiver (`1.8.md:290`) |
| sandbox | the 1.5 shell repo (edits P2's files) and `broadsword.toml` (X0) | X0 to X6 | 6 ("six rounds", `sandbox.md:380`) | 2 | `sandbox.md:380` | 1.5's P3 merged (`sandbox.md:283`); Lazlo's values (X0) and OK (X1) (`sandbox.md:372-373`) |

**What this gives the board.** Round 1 is eight skeleton briefs (W1, G1, V1, X1, S1, R1, N1, TL1), one per repo. Round 2 offers 3 + 3 + 5 + 2 + 2 + 1 + 2 + 5 = 23 briefs (1.1 W2 to W4; 1.2 G2, G5, G6; 1.3 V2 to V5 and V9; 1.4 X2, X6; 1.5 S2, S5; 1.6 R2; 1.7 N2, N5; 1.8 TL2, TL3, TL4, TL6, TL7), more than the 14 slots `broadsword.toml` sets today (`max_agents = 14`). With the waivers, all eight Tier 1 plans finish in 7 rounds at unlimited slots (the longest are 1.4 and 1.5); the sandbox follows 1.5's P3 (round 5 of 1.5) and ends about round 11. Without the waivers, the same plans run one node after another along graph A (an estimated 36 rounds for 1.1 to 1.5 and 1.7; the sum and its sources are in section 3).

**Suggested lane order when slots are short** (the orchestrator's call, from the facts above, not from the plans): 1.7 and 1.8 first, since they are the two compounders in decision 1 and 1.7 has no prerequisite in graph B; then 1.5 and 1.4 (7 rounds each, the longest lanes, and the sandbox waits on 1.5); then 1.1, 1.2, 1.3, 1.6.

**Tier 0 lane, beside the plans.** Graph B's longest paths all start 0.1 -> 0.2 -> 0.3 and then 0.4 or 0.5 (section 3), so Tier 0 work (now started in `projects/foundation`) is on the critical path in both graphs and should keep a slot of its own. No Tier 0 node other than 0.1 has a plan in `docs/nodes/`.

## 5. Part 4's two open suggestions: decisions for Lazlo

Both are **Lazlo's decisions**. Nothing in `ROADMAP.md` or `CAPABILITY_ROADMAP.md` changes here; `ROADMAP.md:92-97` lists both as "Open decisions for Lazlo (not applied)". Each part below gives what the files say (quoted, with file and line), what the graph of section 2 adds, and a recommendation that is only that.

### Stage order today, and edges that run against it

`DEVELOPMENT_PLAN.md:21-36` builds the stages in this order: 0, 1, 2, 3, 4, 6, 7, 5, (4 again), 11, 9, 8, 10, 12. Using each node's stage from `ROADMAP.md` (first build position where a node has two stages), these 20 graph A edges have a prerequisite whose stage is built later than the node that needs it, or has no stage:

- 1.7 (stage none) -> 2.1 (4), 2.7 (9), 3.6 (5), 3.15 (9), 3.22 (10), 4.2 (7)
- 1.8 (stage 9) -> 2.6 (5)
- 2.8 (stage 10) -> 2.9 (8), 3.1 (6), 3.3 (6), 3.10 (8)
- 1.4 (stage 4) -> 1.5 (1), 2.10 (3)
- 2.3 (stage 5) -> 3.2 (6), 3.4 (6)
- 2.4 (stage 5) -> 2.5 (4), 4.1 (7)
- 2.6 (stage 5) -> 4.1 (7); 2.7 (stage 9) -> 4.1 (7); 2.9 (stage 8) -> 3.9 (5)

The two suggestions cover 11 of the 20 (the first three lines).

**Edges that cannot be compared.** One node has no stage: 1.7 (`ROADMAP.md:26`, stage `none`). An edge whose prerequisite has no stage is counted above as against the order (the six 1.7 edges). An edge whose dependent node has no stage cannot be placed in the build order at all, so it is not in the 20 and is listed here instead: 1.5 (stage 1) -> 1.7, 1.6 (stage 1) -> 1.7, 0.6 (stage 1) -> 1.7 (`CAPABILITY_ROADMAP.md:470`). If 1.7 moved into stage 1 (decision 1), these three would sit inside one stage and none would run against the order.

### Decision 1 for Lazlo: move 1.7 (node graph) and 1.8 (timeline) into Foundation (stage 1)

**What the files say.**

- The suggestion: "**Move 1.7 (node graph) and 1.8 (timeline) into Foundation.** They feed at least four later stages each. Built late, they end up rebuilt three times in slightly different forms." (`CAPABILITY_ROADMAP.md:303`)
- Both are "big compounders": 1.7 serves "about 9 tools" and 1.8 "about 8 tools" (`CAPABILITY_ROADMAP.md:108-109`).
- Today: "Today 1.7 is in no stage and 1.8 is in stage 9." (`ROADMAP.md:96`); stage 1 is "Tier 0; 1.1, 1.5, 1.6" (`CAPABILITY_ROADMAP.md:288`) and stage 9 lists 1.8 (`CAPABILITY_ROADMAP.md:296`).
- 1.7's plan: "Broadsword stage: none." (`1.7.md:5`). It is "a **framework under many modes**, not a mode" (`1.7.md:7`), and "A later mode (procedural geometry, node 3.6; compositor, 3.15; block simulation, 3.22) adds a kind and nothing else." (`1.7.md:13`).
- 1.8's plan: "Broadsword stage (ROADMAP.md): 9." and "Later nodes that need it: 2.6 (animation core), 3.16 (nonlinear video editing), 3.17 (motion graphics), 3.18 (DAW features)." (`1.8.md:5`); "a **shared framework under several modes**, not a mode" (`1.8.md:7`).
- Both can start without their roadmap prerequisites if Lazlo waives them: `1.7.md:273` and `1.8.md:290` (section 2b).
- Both version their file format from the first commit, because other modes will store it: `1.7.md:13` ("its file form (section 5, check n10) is versioned from the first commit") and `1.8.md:7` ("that is why the file form ... is versioned from the first commit").
- Size: 1.7 is "10 briefs (N1 to N10), roughly 250 lines each, about 2,500 changed lines" and "about half of one stage" (`1.7.md:396`); 1.8 is "about 2,800 lines in all" and "a little over half a stage of work" (`1.8.md:393`).
- What stays tied to 1.5 and ADR 0002 either way: "ADR 0002 must be decided before 1.5 can host the graph editor." (`1.7.md:394`); "a live editor cannot embed the Rust view without ADR 0002" (`1.8.md:388`); the Done-when's "UI" may need the later panel brief (`1.7.md:384`, risk R1), and "the roadmap's "one timeline widget" is only fully true when 1.5 hosts it" (`1.8.md:382`).
- ADR 0004 (Proposed) puts both in `W/shell` as frameworks (`docs/adr/0004-roadmap-repo-map.md:95-96`).

**What the graph adds.**

- 1.7 has 6 direct dependents (2.1, 2.7, 3.6, 3.15, 3.22, 4.2) in 5 stages (4, 5, 7, 9, 10), and 16 nodes downstream. The "at least four later stages" claim holds for 1.7.
- 1.8 has 4 direct dependents (2.6, 3.16, 3.17, 3.18) in 2 stages (5 and 9), and 10 nodes downstream in 3 stages (5, 7, 9). By the Needs lines, the "at least four later stages" claim does **not** hold for 1.8.
- 1.7 is on graph A's longest path (section 3), and every one of its 6 direct dependents is an edge against the stage order, because 1.7 has no stage. 1.8 has one such edge (2.6, stage 5, built before stage 9).
- Moving both into stage 1 creates no new edge against the order for the lanes the plans propose, since their first briefs need only 0.1 (1.8) or nothing (1.7). Under the roadmap's Needs lines it would: 1.7 and 1.8 need 1.5 and 0.6, and 1.5 needs 1.4 (stage 4).

**Recommendation (Lazlo's decision).** Move both into Foundation as their headless frameworks, as their plans scope them (graph data, evaluation or edits, the versioned file format, the C ABI), with the waivers `1.7.md:273` and `1.8.md:290` ask for; keep the in-window panel and widget with 1.5 and ADR 0002. Per node:

- **1.7:** briefs N1 to N10 (graph model, evaluator, math and image kinds, file format, subgraphs, editor model and text view, C ABI; `1.7.md:354-363`) move to Foundation; the later brief that hosts the editor in a real dockable panel, which "starts only after 1.5 exists" (`1.7.md:270`), stays with 1.5 and needs ADR 0002 (`1.7.md:394`).
- **1.8:** briefs TL1 to TL10 (time and curves, edits and undo, file format, the three drivers, the headless `TimelineView`, C API; `1.8.md:350-359`) move to Foundation; putting the widget on screen with a painter and live input, which "1.5 must later supply" (`1.8.md:282`), stays with 1.5 and needs ADR 0002 (`1.8.md:388`).

Reasons: each is about half a stage, neither needs other nodes' code by its plan, both fix a file format other modes will store, and as lanes they fill 4 and 5 slots at their peaks. Give 1.7 the earlier slot: its case is stronger by the graph (5 stages, on the longest path), while 1.8's dependents sit in stages 5 and 9 only.

### Decision 2 for Lazlo: move 2.8 (numerics core) up to Graphics or earlier

**What the files say.**

- The suggestion: "**Move 2.8 (numerics core) up to Graphics or earlier.** Physics, CAD and simulation all sit on it, and a solid numerics core makes stage 6 much faster." (`CAPABILITY_ROADMAP.md:304`)
- Compounder: "**2.8 Numerics core**: all physics, FEM, CFD, MATLAB, Simulink, CAD constraint solving" (`CAPABILITY_ROADMAP.md:111`).
- Today: "Today only its extensions are in stage 10." (`ROADMAP.md:97`); stage 10 lists "2.8 extensions" (`CAPABILITY_ROADMAP.md:297`). Its own needs: "Needs: 0.2, 0.4" (`CAPABILITY_ROADMAP.md:568`).
- No node plan exists: there is no `docs/nodes/2.8.md`. The plans mention 2.8 only twice: "**Trigger 4** ("a numerics core is needed by two projects") is not fired: the math kind here is a handful of `f64` operations, not a numerics core." (`1.7.md:235`), and a reference-data row, "LAPACK/SciPy reference outputs for numerics", marked "UNVERIFIED (written from memory)" (`REFERENCE_DATA.md:105`).
- ADR 0001 (Accepted): "**No, not for Stage 6 milestones 1 to 3.**" (`docs/adr/0001-shared-core.md:192`); it says line 304's claim "assumes an implicit or FEM solver, which is Stage 6 m5" (`docs/adr/0001-shared-core.md:195-196`), and "Decide it when the m4 or m5 done checks are written ... If only crash-sim uses it, it lives inside crash-sim (`src/numerics/`). It becomes shared code only when a second project needs it" (`docs/adr/0001-shared-core.md:198-200`).
- ADR 0004 (Proposed) orders it fourth among the triggers, firing "when the second consumer's plan starts" (`docs/adr/0004-roadmap-repo-map.md:184`), with home `W/core` (`:112`).

**What the graph adds.**

- 2.8 has 8 direct dependents (2.9, 3.1, 3.3, 3.10, 3.19, 3.20, 3.22, 3.23) in stages 6, 8 and 10, and 22 nodes downstream in stages 5, 6, 7, 8 and 10.
- 4 of its direct dependents are edges against the stage order today: 3.1 and 3.3 (stage 6) and 2.9 and 3.10 (stage 8) need a node mapped to stage 10. So the current mapping already asks stage 6 to use a stage 10 node.
- No stage 4 (Graphics) node needs 2.8: stage 4 is 1.2, 1.3, 1.4, 1.11, 2.1, 2.2, 2.5 (`CAPABILITY_ROADMAP.md:291`), and none of their Needs lines names 2.8. "Graphics" is earlier than needed by the graph; stage 6 is the first stage that needs it.
- 2.8 is on two of graph B's three longest paths (section 3), so once the Tier 1 waivers are given it becomes part of the critical path.

**Recommendation (Lazlo's decision).** Move 2.8 into stage 6 (Physics), not stage 4: stage 6 is where the first nodes that need it (3.1, 3.3) are mapped, and no Graphics node needs it. Do not start its code ahead of a consumer, which agrees with ADR 0001: write `docs/nodes/2.8.md` as a plan now (low-priority ahead-work like the other plans), and start its first briefs when the crash-sim m4 or m5 done checks, or the first 3.1 plan, need linear algebra. Its prerequisites 0.2 and 0.4 are in the Tier 0 lane already.
