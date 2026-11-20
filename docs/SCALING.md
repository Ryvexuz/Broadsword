# Scaling the harness to 70-125M lines (design only)

Board task `t-20261120-013957`. Opus 5.5, 2026-11-20. **Design only: no code changed, nothing boarded.** Lazlo reads this
before any of the follow-up briefs at the end are boarded.

Every number below cites the file it came from or shows its arithmetic. "Measured" means read from a file in the repo or in
`state/` (outside Git) on 2026-11-20 about 01:50Z; "estimate" means arithmetic on measured numbers, with the assumption named.
Paths are relative to the Broadsword root.

## 0. Today, measured

| What | Value | Source |
| --- | --- | --- |
| Broadsword crate | 13,745 lines in `src/` (21 files) + 7,921 in `tests/` = 21,666 | `wc -l` on `crates/broadsword/src/*.rs` and `find crates -name '*.rs'` |
| Project repos | crash-sim 5,755 Rust lines, render 5,493 | `find projects/<p> -name '*.rs' \| xargs cat \| wc -l` |
| All Rust | 21,666 + 5,755 + 5,493 = 32,914 lines | sum of the two rows above |
| Python (scripts, clients, bench) | 13,522 lines | `find scripts clients bench -name '*.py' \| xargs cat \| wc -l` |
| Board | 238 task files by `ls state/tasks/*.task \| wc -l`; the four status counts 80 done + 69 abandoned + 63 open + 25 running sum to 237, so one file has a status outside these four (not identified); 118 broadsword, 114 crash-sim, 6 render | `grep -h '^status = ' state/tasks/*.task \| sort \| uniq -c`, same for `project` |
| Queue | 256 entry files; 68 merged (42 broadsword, 22 crash-sim, 4 render); 198 `risk = low` + 57 `risk = high` = 255, so one of the 256 files has no `risk` line (not identified) | `grep` over `state/queue/*.entry` |
| Throughput | 6.79 merged/h with 14 slots (22:00-23:46Z), 8.44/h at 10, 4.45/h over a 4.5 h session, 3.27/h in S4 | `state/baselines/slots-14-pre16.json`, `pc-agents-10.json`, `pc-session-2026-11-19.json`, `s4-session.json` (`merged_per_hour`) |
| Queue step times | check median 109 s, review median 54 s, entry median 1,066 s | `slots-14-pre16.json` (`check_median_s`, `review_median_s`, `entry_median_s`) |
| Review rounds | 1.67 per merged task, first-pass fail rate 0.50 | `slots-14-pre16.json` |
| Review cost | $0.184 to $0.405 per merged task (four windows); $21.41 over 160 priced reviews = $0.134 per review | the four baselines (`review_usd_per_merged`); sum of `cost_usd` over `state/queue/*.entry` |
| Coder tokens | 185,273 per merged task (S4, 27 merged); 422,877 (pc-session, only 2 of 20 merged tasks had a run) | `s4-session.json`, `pc-session-2026-11-19.json` (`coder_tokens_per_merged`) |
| Test jobs | 101 test jobs for 20 merged tasks = 5.05 per merged task, median 98 s | `pc-session-2026-11-19.json` (`samples.test_jobs`, `test_job_median_s`) |
| Machine | 32 logical CPUs; 111.6 GB RAM, 12-16% used at 14 slots; C: 1.8 TB, 1,473 GB free (80.4%) | `nproc`; `state/pc-progress.md` 2026-11-19T23:56Z line; `df -h .` |
| CPU | mean 19.7%, max 87.2% over 3 min at 14 slots; 100% with one queue check plus ahead compiles | `state/pc-progress.md` 23:56Z and 2026-11-20T00:52:28Z lines |
| Disk use | `build/` 848 MB, `scratch/` 3.2 GB, `state/` 37 MB, `.git` 26 MB | `du -sh` |
| Records | LAB_LOG.md 51,857 B (23 entries), RETRO.md 41,761 B, AGENTS.md 12,355 B, PROGRESS.md 12,718 B, `state/pc-progress.md` 107,676 B (one 8.3 h session, 17:12Z to 01:32Z) | `wc -c`; `grep -c '^## ' LAB_LOG.md` |
| Briefs | 198 files, 1,042 KB = 5.3 KB per brief | `du -sk state/backlog`, `ls \| wc -l` |

### The three global locks

Everything serial in the harness today goes through three OS file locks (lock.rs: exclusive `File::try_lock`):

1. `state/queue/queue.lock`, held for a whole `queue run` (queue.rs header and `cmd_run`). It is **one lock for every
   project**: a crash-sim entry waits for a broadsword entry although they share no file and no main branch.
2. `state/build.lock`, held while cargo compiles (buildsrc.rs header; job.rs "THE TEST STEP RUNS FROM COPIES"). One
   compile at a time across all projects and all worktrees, because they share one `CARGO_TARGET_DIR`.
3. `state/tasks/board.lock`, held for milliseconds by every task status change; `task start` reads every task file and the
   whole queue folder under it (task.rs header).

### The queue's ceiling, from its step times

An entry that merges pays check + review + main check; a rejected one pays check + review. With 1.67 review rounds per
merged task, 0.67 rejected attempts ride along with each merge. Assuming the main check takes as long as the entry's check:

- **ESTIMATE.** Measured inputs: check 109 s, review 54 s, 1.67 rounds (`slots-14-pre16.json`). **Assumed inputs, not
  measured:** (a) the main check costs the same 109 s as an entry check (no baseline file records a main-check time); (b) a
  rejected attempt costs one check plus one review and nothing else; (c) the queue is never idle and no bisect or retry
  time is counted.
- per merged task: (109 + 54 + 109) + 0.67 x (109 + 54) = 272 + 109 = 381 s, so the ceiling is 3,600 / 381 = **9.4 merged/h
  (ESTIMATE)**. If the real main check is 60 s the ceiling is 3,600 / 332 = 10.8/h; at 150 s it is 3,600 / 422 = 8.5/h.
- measured 6.79/h is 72% of that estimated ceiling (6.79 / 9.4). The queue, not the 14 coder slots, sets the pace, which matches
  the log: 14 tasks running, 11 to 14 of them waiting in the queue (`state/pc-progress.md` 00:24Z to 00:32Z lines).

### Why size breaks the current design

Assume check time grows linearly with the code a check builds and tests (optimistic: link time and test count grow at
least as fast). Today's check covers the broadsword crate: 109 s / 21,666 lines = 5.03 ms per line.

- A full check of 70M lines: 70e6 x 5.03 ms = 352,100 s = **97.8 h**. Of 125M lines: **174.7 h**.
- So "check everything on every merge" gives about 0.01 merges/h at that size. The two things that must change are
  (a) a check covers only what a change can affect, and (b) more than one lane merges at a time.

## 1. Sharded merge queues

### The product shape the shards follow

Lazlo's intent (coordinator, 2026-11-20): **one unified tool**. While building the car he switches between modeling,
rendering and the physics engine in one place, on the same live data, with no export and import between them. So the
code has one shape, and the shards follow it:

- **The shared core**: one scene and data model (geometry, materials, structure and sim state) that every capability
  reads and writes, plus the command/undo layer that changes it. In roadmap terms: 0.5 serialization, 0.6 command and undo,
  1.6 reflection, 2.4 scene system and later 5.1 the unified asset database (ROADMAP.md rows; DEVELOPMENT_PLAN.md
  "Shared core").
- **Modes**: modeling, render, physics, engine, CAD, media and so on. Choosing a mode in the app activates that mode's
  crate group. Every mode works on the same core data. A mode is a plugin over the core (roadmap node 0.7, plugins and hot
  reload): it is loaded or activated on selection and depends **only on the core API, never on another mode's internals**.
- **Views and tools** are layers on top of a mode and the core.
- **Bedrock** (math, job system, logging; Tier 0) sits under the core.

This lines up one to one with the harness: **one merge-queue shard per crate group**. A mode's crate group is one shard, the
core is its own shard, bedrock is one shard. A change inside one mode cannot break another mode by construction, because
no mode depends on another (section 3 enforces this); it can only break through the core API, and a core change is a
cross-shard change by definition (section 5).

ADR 0001 (docs/adr/0001-shared-core.md) keeps per-repo copies today and moves to option (d2) when a trigger fires. One
live data model shared by every mode is very likely such a trigger. **Decision for Lazlo**, not taken here: this design works
per repo first (each of today's three repos is already its own shard candidate) and needs (d2) only for step S6.

### What a shard owns

A shard is a named set of path prefixes inside one repo (a crate group), listed in a shard map file. It owns:

- its **claims**: `task start` refuses a task whose claims fall in more than one shard, unless the task is marked
  cross-shard (section 5);
- its **queue lane**: entries of its tasks, and its own `queue.lock` (for example `state/queue/<shard>/queue.lock`);
- its **build folder**: a subfolder of `build/` (for example `build/<shard>/`) with its own build lock and its own
  `state/build-sources/` notes. Today's lesson from the merge-train port says why it must be separate: two checkouts sharing
  one `CARGO_TARGET_DIR` ran each other's stale test executables (18 of 22 recovery tests failed;
  `state/staging/t-20261119-222633/on-main/NOTES.md`, "Scratch clone");
- its **owner**: one orchestrator lane that boards, briefs and adjudicates for it;
- its **baselines**: perf baselines (`state/perf/<shard>/`) and its last-green ref (section 2);
- its **risk tier**: core and harness shards are always `--risk high`.

The Disk rules say "one build folder" (AGENTS.md, Disk rules). Per-shard subfolders inside `build/` keep one folder tree that
moves with the project, but they change the letter of that rule. **Decision for Lazlo.**

### One driver per shard

Lesson 30 (one driver per queue) applies per shard: one `queue run --all --shard <name>` process drains that lane in
order. A supervisor (`queue drive`) may start one driver per shard and restart a dead one; it never runs two for one
shard, and the shard's `queue.lock` makes a second driver exit 4 "queue busy" (as today).

### How shards share main

Each repo has one `main`. Today `s_merge` (queue.rs) ends an entry `stale` whenever main moved after its rebase
(`head != base`). With several lanes, main moves all the time, so every lane would go stale. The rule that lets lanes share
main:

- **Main lock**: `state/queue/main-<project>.lock`, held only for the rebase onto main, the patch-id comparison and the
  fast-forward (seconds), never during a check or review.
- **Commutation rule**: let D(x) be the crates a change touches and R(S) the reverse-dependency closure of S (section 2).
  When main moved from `base` to `head`, the entry may be rebased and fast-forwarded without a new check only if
  R(D(entry)) and R(D(base..head)) are disjoint, the rebase is clean, and every rebased commit has the same
  `git patch-id --stable` as the reviewed one (the merge train already binds a review to its patches this way:
  `train_link`). Otherwise the entry is `stale` exactly as today.
- Fail closed: a change to any file outside a crate (Cargo.lock, a workspace Cargo.toml, `broadsword.toml`, toolchain files)
  counts as touching every crate, so it never commutes.

## 2. Affected-only build and test selection

### Selection

1. Changed files: `git diff --name-only <base>..<tip>`.
2. Map each file to its package: the package whose manifest folder is the longest prefix of the path. Packages come from
   `cargo metadata --format-version 1 --offline` **with** dependencies (today buildsrc.rs `metadata_json` passes
   `--no-deps`, which is enough to list members but not to build the graph).
3. D = the changed packages. Affected = R(D): every workspace package that depends on one of them, transitively, through
   normal, build or dev dependencies.
4. The check runs `cargo build` / `cargo test` with `-p` for each affected package, through the same job runner (disk guard,
   build lock, job record). The job record lists D, R(D) and the reason for each package.

### Fail closed

- metadata cannot be read or parsed: full check of the shard;
- a changed file maps to no package, or is a global file (list above): full check of the repo;
- a changed `build.rs`, proc-macro crate or feature list: its R(D) as usual (cargo already records those edges), plus the
  shard's full run on the schedule below.
- Known gap, stated, not hidden: `cargo test -p a -p b` unifies features over the selected packages only, so a feature
  combination that exists only in a full build is first tested by the scheduled full run.

### The full run on a schedule

Section 0 shows a full check of 70-125M lines takes 98-175 h on this PC, so "nightly full run" is impossible at that size.
The safety net is therefore per shard and rotating:

- Each shard keeps a last-green ref (for example `refs/broadsword/green/<shard>`), the newest main commit whose full shard
  check passed.
- On a schedule (a required setting), one shard at a time runs a full check of main. Pass: move its green ref. Fail: bisect
  between green and main with the same machinery as the train's bisect, file a task for the culprit, and never reset main
  (the `merged_check_failed` rule in queue.rs: "Main was NOT reset or reverted. A human decides").
- Repo-wide full runs happen only when a global file changes, and at stage tags.

### Budget

The selection only helps if R(D) stays small. With an affected-set budget of 50k lines, a check takes about
50,000 x 5.03 ms = 251 s, about 2.3x today's 109 s (the 2.3x used in section 8). Section 3 is what keeps R(D) inside that budget.

## 3. Crate boundaries and ownership rules

### Evidence that today's files are too hot

Of 118 broadsword tasks, 25 claim `broadsword.toml`, 21 claim `queue.rs`, 18 claim `config.rs`, 16 claim `tools.rs`, 11
claim `job.rs` (`grep -l '^claims = .*<file>' state/tasks/*.task | wc -l`). Overlapping claims run one after another
(AGENTS.md, "Claim before editing"), so a fifth of broadsword's tasks queue behind each other on one file. The perf gate brief
says it outright: "It claims queue.rs and config.rs, so it runs after the train" (`state/backlog/t-20261120-013256-3.md`).

### Rules

1. **Dependency direction (the mode rule).** Groups: bedrock -> core API -> core impl; modes depend on bedrock and the core
   API only; views and tools depend on their mode and the core API. **No mode crate may depend on another mode's crates.**
   Shared code two modes need moves down into the core API (or bedrock), as a cross-shard change.
2. **How the rule is enforced.** A check reads `cargo metadata` and the shard map (each crate's group) and rejects any edge
   not allowed by a small allow table (group -> groups it may depend on). It runs inside `broadsword check` before cargo
   builds, so the queue's check step fails the entry (`check_failed`, the reason names the forbidden edge, for example
   "mode-render crate `render-raster` depends on mode-physics crate `sim-solver`"). A crate missing from the shard map fails
   the check too (fail closed). It must be shown failing once on a planted mode-to-mode edge (lesson 10).
3. **API crates.** The core and every mode export through a small `-api` crate (types, traits, events); the implementation
   crate depends on it, dependents depend on the `-api` only. A change inside an implementation then has R(D) = its own
   crate, its tests and the app crate that wires modes together, not every mode.
4. **Crate size cap.** A crate over a set number of lines (proposal: 20k, about today's whole broadsword crate) gets a
   split task. Big crates make every task in them overlap and every check slow.
5. **Mode plugins.** A mode registers with the core through the 0.7 plugin interface and is activated on selection; the
   app crate is the only crate that names every mode. This is what makes "switch mode on the same live data" possible and
   what keeps modes from reaching into each other.
6. **Settings without a hot file.** `broadsword.toml` stays the one settings file (AGENTS.md, Settings), but each module
   reads its own table (for example `[queue]`, `[perf]`) through its own config struct, and a claim may name a table
   (`broadsword.toml#queue`), so two tasks adding keys to different tables no longer conflict. Claims are plain paths today
   (task.rs: "never patterns"), so this is a board change. **Decision for Lazlo**, since it touches the one-file rule's form.
7. **Tests live with their crate**; cross-crate integration tests live in one `-it` crate per shard, so they enter R(D) only
   when one of their dependencies changes.

### The shared core gets the strictest protection

A change to the core is a cross-shard change by definition: every mode reads and writes its data. The harness protects it
with:

- **its own shard and owner**: the core shard's owner reviews every core brief before it is boarded;
- **a stricter review tier**: always `--risk high` (the Opus gate today, `reviewer_model_high`), plus the shadow reviewer,
  plus, when Lazlo approves it, the second blocking reviewer trialled in t-20261119-232113-2;
- **compatibility tests**: a folder of golden files, one per released schema version, that every core change must still load
  and round-trip byte-identically (roadmap 0.5: "an old-version file still loads after a schema change"); and a public-API
  snapshot of the core `-api` crate (a checked-in list of its `pub` items) whose change marks the entry as an API change
  that needs the cross-shard path in section 5;
- **versioning and migration**: every saved format and the in-memory schema carry a version; a reader accepts version N
  and N-1; a migration is code with its own golden test;
- **a deprecation path**: an API item is marked deprecated, consumers migrate, and it is removed only when no crate in the
  repo uses it (the removal's own check proves it). Section 5 is that path.

### A language-neutral core boundary

Lazlo may write the upper layers (modes, views, tools) in languages other than Rust once the foundation exists
(coordinator, 2026-11-20). AGENTS.md already allows this with a one-line reason per language in LAB_LOG.md ("Languages").
So the core's API must not be Rust-only:

- **A C ABI boundary**: the core `-api` crate has a `-capi` twin (`extern "C"` functions, `#[repr(C)]` plain data, opaque
  handles, explicit ownership, error codes rather than panics across the boundary), and a generated C header is checked in.
  Rust modes may use the Rust API. Other languages use the header, and so does any mode that must hot-reload as a plugin
  (node 0.7), because Rust has no stable ABI between compiler versions.
- **Serializable data**: core data also crosses as versioned text and binary formats. The crash sim's `.bsn` is the
  pattern: plain text, one record per line, `bsn 1` on line 1 as the version
  (`projects/crash-sim/docs/FORMAT.md`). Any language can read and write such a format without linking the core.
- **ABI compatibility tests at the boundary**: the checked-in header is compared with the one generated from the code
  (a change marks the entry as a core API change, section 5); a C test program built against the **previous** released
  header must still link and pass against the new library; every format version's golden files must load from both Rust
  and one non-Rust client.

### Multi-language shards in the queue and tests

- **One job interface, one runner per language**: `broadsword build|test|check <project>` stays the only entry point. A
  shard names its language(s) in the shard map, and each language has a runner behind that interface (cargo for Rust; for
  example pytest or `python -I` for Python, CMake/ctest for C/C++) with the same disk guard, job record, log, lock and
  exit codes. Evidence that this is needed now: the 13,522 Python lines are tested today by running `python -I scripts/test_*.py`
  directly; job.rs never starts Python (`grep -ci python crates/broadsword/src/job.rs` = 0), so the queue's check never
  runs those tests.
- **An affected-only graph that crosses languages**: section 2's graph gets nodes for non-Rust packages too. Their edges
  come from their own manifests (for example `pyproject.toml`) plus one declared edge from each non-Rust package to the core
  `-capi` header or format version it uses. A change to the header or to a format makes every package that declares it
  affected. A non-Rust package with no manifest or no declared edges makes the check run in full (fail closed).
- **Review routing by language**: the shard map gives each language a reviewer list, and the queue picks the reviewer from
  the entry's languages (still never the coder's model; still high risk for anything touching the boundary). An entry that
  touches two languages, for example a C ABI change and its Python binding, gets the boundary's high-risk review.
- **Dependency rule across languages**: the rule 2 check also reads the declared edges, so a Python mode importing another
  mode's package is rejected just like a Rust one.

## 4. Merge train, perf gate and reviews per shard

### Merge train (staged as t-20261119-222633, not reviewed yet)

The staged train already groups by project: "Approved entries of one project whose task claims do not overlap form a train
of up to merge_train_max entries" (`state/staging/t-20261119-222633/on-main/t-20261119-222633.patch`, queue.rs header). Per
shard it changes only the grouping key (shard instead of project) and the check:

- the train's one check runs on R(D) of the union of the members' changes, inside the shard's build folder;
- the bisect stays inside the shard; the left-train rule (NOTES.md: "left a train: rebase your own commits onto main") is
  unchanged.

Ceiling with a train of 4 and deferred per-entry checks (**both figures are ESTIMATES**): review per merged 1.67 x 54 = 90 s
(measured inputs), plus one check and one main check shared by 4 = (109 + 109) / 4 = 54.5 s, so 144.5 s per merged =
**24.9 merged/h per lane (ESTIMATE)**. Assumed, not measured: a train size of 4 (`merge_train_max` is a setting, no train has
run at that size), main check = 109 s (as in section 0), no bisect time, no failing trains, review time unchanged by batching.
With affected checks at the 50k-line budget (251 s = 50,000 x 5.03 ms, which assumes check time linear in lines and the 50k
budget are both assumed): 90 + (251 + 251) / 4 = 215.5 s = **16.7 merged/h per lane (ESTIMATE)**.

### Perf gate (t-20261120-013256-3, boarded, not built)

The brief already skips untouched projects ("Projects without a baseline are skipped"). Per shard: baselines live under the
shard, and the gate runs when R(D) contains a crate the shard's bench measures. One risk is new: timings taken while another
shard compiles on the same 32 CPUs are noisy (CPU hit 100% with one check plus ahead compiles, pc-progress 00:52Z). So
perf runs take a machine-wide perf lock in exclusive mode while every build takes it in shared mode (std's
`File::try_lock_shared`; lock.rs today has exclusive locks only). Builds queue behind a perf run for the
`perf_runs` x bench time.

### Reviews

- Every entry still gets its own independent review before it merges (AGENTS.md: Lazlo's rule; its form may change, its
  existence may not).
- The review prompt per shard adds the shard's README (its API and rules) and the affected-set report from section 2, so the
  reviewer sees what the check did and did not cover (lesson 8: give the reviewer the test output).
- Risk tier comes from the shard map: core, harness, locks, deletion and secrets shards are high; mode shards default to low.
- The reviewer is also routed by the entry's language(s) from the shard map; an entry that touches the core's C ABI boundary
  is always high risk (section 3, "Multi-language shards").
- The shadow reviewer's daily cap (`GPT daily spend cap of 20 USD`, commit 27c24e2) is one budget for all shards; its OS-locked
  ledger already serializes writers, so many drivers can share it. Section 8 shows when the cap binds.

## 5. Cross-shard changes

Two ways, in order of preference.

### Staged API change (default: expand, migrate, contract)

1. **Expand**: the provider shard adds the new API beside the old one. One normal single-shard merge.
2. **Migrate**: one task per consumer shard switches to the new API. These run in parallel, each in its own lane.
3. **Contract**: the provider shard removes the old API. Its check covers R(D), which includes every former consumer, so
   it fails if any consumer still uses it.

A change-set record (for example `state/changesets/<id>.md`) lists the steps and their tasks; the board refuses the contract
task until every migrate task is done. Across repos (ADR 0001 keeps three repos) this is the **only** way: Git has no atomic
merge across repositories, so a file format crosses repos by versioning (readers accept N and N+1 before any writer moves).

### Atomic multi-shard merge (rare)

For a change that cannot be split (for example a core type and the only two crates that use it, inside one repo):

- the task is marked cross-shard and lists its shards; `task start` refuses an unmarked task whose claims span shards;
- the queue takes the involved shards' queue locks **in sorted shard-name order** (no deadlock), then the main lock;
- one check on R(D) of the whole change, one review by the high-risk reviewer with every shard's README, one fast-forward;
- a cap on shards per atomic entry (proposal: 3), and each one is counted in the metrics, because each stops several lanes.

A change to the core is always one of these two, never a plain single-shard merge.

## 6. Staged path from today

### Current fixes that are already steps toward it

| Fix | State | Step toward |
| --- | --- | --- |
| Cap fix: `max_agents` counts only working coders, new `max_queued` | on main (ad8f932, 7b37a4f, 51d3732) | Separates coder capacity from queue capacity, which is what showed the queue is the limit (14 running, 11-14 waiting) |
| Merge train | staged, 465 tests green, not reviewed (`.../on-main/NOTES.md`) | Batching within a group with non-overlapping claims; patch-id binding of reviews; bisect; the one-driver `--all` drain. All reused per shard |
| Perf gate | boarded (t-20261120-013256-3) | "Untouched projects skipped" is the first affected-only rule, at project granularity |

### Next steps (each measured with `scripts/baseline.py` before and after, each revertable by a setting)

- **S1. Per-project queue lanes and build folders.** One `queue.lock` and one build subfolder plus build lock per project.
  The three repos have separate mains already, so no commutation rule is needed yet. Expected: crash-sim and broadsword merge in
  parallel; today they are 89 and 160 of 256 entries in one lane. Revert: one lane setting.
- **S2. Affected-only check** (`check --affected <base>`), fail closed, with the rotating full run and the green ref.
- **S3. Dependency-rule check** (section 3 rule 2) over the shard map, run inside `check`.
- **S4. Shard map and per-shard lanes within a repo**, with the main lock and the commutation rule in `s_merge`.
- **S5. State archive** (section 7): finished tasks and entries move to dated archive folders (never deleted), so reading
  the board and the queue stays bounded.
- **S6 (after Lazlo's ADR decision).** The core shard with its compatibility tests, API snapshot and deprecation path.

## 7. Stage and task pipeline for about 1,400 stages

### How many stages and tasks

The orchestrator's grounded count is appended to the brief (`state/backlog/t-20261120-013957.md`, "Roadmap size (grounded)",
from `scratch/roadmap-size.md`). The figures used here are from that section:

- 12 Broadsword stages (DEVELOPMENT_PLAN.md), 71 roadmap nodes (`grep -cE '^\| [0-5]\.[0-9]+ \|' ROADMAP.md` = 71), 0
  done and 6 in progress.
- Yardstick from stages 1-4: about 20 merged tasks and 12k lines of product code plus tests per stage, 600-660 lines per task.
- The capability layer alone (a floor): 80 / 205 / 540 stages, 1,600 / 4,000 / 10,800 tasks (low / mid / high).
- Reading Lazlo's figures as the whole project, products included: 1,400 stages x 20 tasks = **28,000 tasks**. By lines:
  70e6 / 600 = **116,667** and 125e6 / 600 = **208,333 tasks**.
- The two readings disagree: 70e6 / 1,400 = 50,000 lines per stage, 4.2x the 12k yardstick. So either later stages are
  about 4x bigger than the first ones, or there are more tasks per stage. This document plans for the larger, line-based count
  (117k-208k) and treats 28,000 as the low end.

### Time at today's rate

At 6.79 merged/h: 28,000 / 6.79 = 4,124 h; 116,667 / 6.79 = 17,182 h; 208,333 / 6.79 = 30,682 h. At 8,766 h a year running
24/7: **0.47, 1.96 and 3.5 years.** And that is at today's check time, which section 0 shows does not hold at size.

### Brief generation

- **DERIVED, not measured directly.** Briefs per landed task: the board has 80 done and 69 abandoned tasks (section 0), each
  with one brief, so (80 + 69) / 80 = 149 / 80 = **1.86** briefs written for each task that lands. It assumes every abandoned
  task cost one brief and that the abandon share stays as it is. Some abandons are early experiments, so it is probably
  pessimistic.
- Briefs needed: 28,000 x 1.86 = 52,080 to 208,333 x 1.86 = 387,500. At 5.3 KB each: 276 MB to 2.05 GB.
- Rate needed (**DERIVED**): today's measured 6.79 merged/h x the derived 1.86 = **12.6 briefs/h**, one every 4.8 min (60 /
  12.6), from the orchestrator alone. Today it writes them in
  batches (46 briefs boarded at 17:13Z, `state/pc-progress.md`).

The pipeline that scales is four levels, each owned by a different role, each with a size limit:

1. **Roadmap** (71 nodes, ROADMAP.md, checked by `scripts/roadmap_status.py`): Lazlo's level.
2. **Node plans** (the 68 `ra-*` plan briefs in `state/roadmap-ahead-2026-11-20/`, 69 files there): one plan per node, which
   splits it into stages with done checks. Written by a strong model, read by Lazlo.
3. **Stage specs**: one per stage, its done check and its task list (task titles, claims, dependencies), written by the
   shard owner from the node plan.
4. **Task briefs** from `templates/brief.md`, written by a cheaper planner per shard from the stage spec, and **linted at
   `task add`**: at most about 30 lines, 8 requirements and 2 claimed files, a done check made of commands, a claim inside one
   shard. The orchestrator reviews stage specs, not every brief.

### Long-horizon state

| Record | Today | At 28k / 208k tasks | What changes |
| --- | --- | --- | --- |
| Task files (read whole under `board.lock` by `task start`) | 238 files, 92,748 B = 390 B each | 10.9 MB / 81 MB per read | Archive done/failed/abandoned tasks to `state/tasks/archive/<yyyy-mm>/` (never deleted); the board reads open and running ones only |
| Queue entries (`load_entries` reads every `.entry`; `task start` reads the folder too) | 256 files, 3,945,004 B = 15.4 KB each; 256 / 238 = 1.08 per task | 28k x 1.08 x 15.4 KB = 466 MB; 208k x 1.08 x 15.4 KB = 3.46 GB per read | Same archive for ended entries; an index of open entries |
| RETRO.md | 41,761 B after 4 stages = 10.4 KB per stage | 1,400 x 10.4 KB = 14.6 MB | One retro file per stage; RETRO.md keeps the ledger, the portable lessons and the "carry" list only |
| LAB_LOG.md | 51,857 B, 23 entries = 2.25 KB per entry, 3 days | grows with sessions without bound | One file per month; agents read only the newest |
| `state/pc-progress.md` | 107,676 B in one 8.3 h session = 13 KB/h | grows without bound | One file per session (it is outside Git already) |
| AGENTS.md | 12,355 B, loaded by every agent every session | must not grow with stages | Stage sections move to DEVELOPMENT_PLAN.md and the node plans; AGENTS.md keeps rules only |
| Claude memory | 16 files, 23,885 B, 16 index lines (`ls` and `cat memory/*.md \| wc -c` in the Claude project memory folder, 2026-11-20; it has grown since) | bounded by hand | A cap on index lines, and the `consolidate-memory` pass at each stage end |
| PROGRESS.md (handoff) | 12,718 B, rewritten each session | bounded by design | One per shard plus a short top-level one |
| evidence/ | 774 KB (`du -sk evidence`) after 4 stages = 194 KB per stage | 1,400 x 194 KB = 271 MB in Git | Per-shard folders; fine in size |

## 8. Limits and risks

### Disk

- Build output: 848 MB for 32,914 Rust lines = 25.8 KB per line. Building everything into one folder: 70e6 x 25.8 KB =
  **1.81 TB**, 125e6 x 25.8 KB = **3.22 TB**, against 1.8 TB total and 1,473 GB free. One shared folder of the whole tree
  does not fit on this disk. Per-shard folders that build only R(D) and its dependencies: a shard of 2M lines needs
  2e6 x 25.8 KB = 51.6 GB, so 8 active shards need 413 GB.
- The build folder is **not swept today**: "cargo-sweep is NOT installed, so the build folder was NOT swept"
  (`state/pc-progress.md`, 22:37:50Z autoclean). At size this fills the disk; installing it is Lazlo's call (Ask first).
- Worktrees: tracked Rust and Python in the broadsword repo is 2,032,239 B (`git ls-files '*.rs' '*.py' \| xargs cat \| wc -c`, 2026-11-20 about 01:50Z; the repo has grown since) for 35,188 lines (21,666 + 13,522) = 57.8 B per
  line. A full checkout of 100M lines is 5.78 GB (70M: 4.0 GB, 125M: 7.2 GB); 14 worktrees x 5.78 GB = 81 GB, 32 worktrees =
  185 GB. Worktrees need sparse checkout limited to the shard plus its dependencies.
- State: 37 MB for 238 tasks = 155 KB per task, so 4.3 GB at 28k tasks and 32 GB at 208k: fine on disk; the cost is the
  read time in section 7, not the space.

### CPU

- One full check at size: 98-175 h on this PC (section 0). Affected checks inside the 50k-line budget: about 251 s.
- Machine-wide merges per hour (**ESTIMATE**, two samples). Assumption named: CPU use is linear in merges per hour, with no
  idle baseline, so 100% CPU = merged/h at 100%. Sample 1 (weak): `state/pc-progress.md` 23:56Z, mean 19.7% over 3 minutes
  at 6.8 merged/h gives 6.8 / 0.197 = 34.5 merged/h; with checks 2.3x bigger (50k / 21.7k lines), 34.5 / 2.3 = **15
  merged/h**. Sample 2 (longer): `scratch/scale/sys-14.csv` (380 rows at 10 s, 21:56:21Z-23:00:32Z) mean 35.3% and
  `sys-14-resume.csv` (168 rows, 23:16:30Z-23:44:51Z) mean 42.1%, so weighted (380 x 35.3 + 168 x 42.1) / 548 = 37.4%
  across the 14-slot window that measured 6.79 merged/h (`slots-14-pre16.json`): 6.79 / 0.374 = 18.2 merged/h; 18.2 / 2.3 =
  **7.9 merged/h**. The CPU there also carries coder compiles and tests that do not scale with merges, so the true figure
  lies between the two: **about 8 to 15 merged/h for the whole PC, however many shards run (ESTIMATE)**. Later windows run
  hotter (`sys-14-resume2.csv` mean 50.3%, `sys-14-resume3.csv` 53.3%, `sys-14-resume4.csv` 78.2%, 705, 706 and 166 rows) but
  have no merge-rate baseline, so they are not used.
- So the PC, not the queue design, becomes the limit at about 1 lane (16.7/h per lane, section 4, against 8 to 15/h for the
  PC). 208,333 tasks at 15/h is 13,889 h = 1.58 years 24/7; at 8/h it is 26,042 h = 2.97 years. Beyond that, checks need a second machine or build runners. Spending money is Ask first.
- Coder test runs dominate: 5.05 test jobs per merged task at 98 s median (pc-session) = 495 s of test jobs per merge,
  against 0.5 queue checks per merge in a train of 4. Affected-only selection must apply to `broadsword test --task` too, not
  only to the queue.
- Perf gate noise when shards compile at the same time: section 4 (perf lock).

### Model cost

- Reviews: $0.184 to $0.405 per merged task (section 0). 28,000 tasks: $5.2k-$11.3k. 116,667: $21.5k-$47.3k. 208,333:
  **$38.3k-$84.4k**.
- Coders: 185,273 to 422,877 tokens per merged task. 28,000 x 185,273 = 5.19 Gtok; 208,333 x 422,877 = 88.1 Gtok. No coder
  price is recorded ("state/agent-runs.tsv has no price column today", `scripts/dev_metrics.py:308`). The review records give
  implied rates: Opus $7.145 / 0.8146 Mtok = $8.77 per Mtok, Haiku $0.963 / 1.652 Mtok = $0.58 per Mtok (pc-session
  `per_model`). Bracketing coders between them: $3.0k (5.19 Gtok x $0.58) to **$773k** (88.1 Gtok x $8.77). The range is too
  wide to plan with; recording coder prices is the first follow-up.
- Shadow reviews: the cap is $20/day for all shards. At 15 merged/h x 24 h x 1.67 reviews = 601 reviews/day, that allows
  $20 / 601 = $0.033 per shadow review. The shadow's own cost per review is not in the entries (`shadow_cost_usd` is empty in
  `state/queue/*.entry`), so whether the cap binds is not known yet.
- Stage audits: one Fable audit used 333,759 tokens (`s4-session.json`, `auditor:fable`). 1,400 audits x 333,759 = 467 Mtok,
  and that is today's project size. Reading 100M lines once is about 100e6 x 57.8 B / 4 B per token = 1.45 Gtok, beyond any
  context. "A strong model audits the whole project" (AGENTS.md, Review) has to become per-shard audits plus an audit of the
  core API and the cross-shard rules. That changes the audit's form, not its existence. **Decision for Lazlo.**

### Review capacity

- Serial review in one lane: 1.67 rounds x 54 s = 90 s per merged task, so at most 40 merged/h from review alone. Shards
  multiply this, the PC does not limit it (reviews run on the provider), and cost (above) does.
- First-pass fail 0.43-0.50 (baselines): about half of all reviewed entries come back for a fix round, so coder capacity has
  to cover 1.67 rounds per task.
- The stage-4 audit produced 477 finding ids, adjudicated by five Opus judges (PROGRESS.md, "Stage 4 audit"). 1,400 stages x
  477 = 668k findings if every stage audit keeps that size: only per-shard audits with a size limit are possible.
- Lazlo: he reads roadmap-level and core-level decisions. With section 7's four levels his reading is the node plans (71) and
  the decisions marked in this document, not briefs.

### RAM

- 12-16% of 111.6 GB at 14 slots = at most 0.16 x 111.6 / 14 = 1.28 GB per slot; 32 slots = 41 GB (37%). Not a limit.
- Not measured: rust-analyzer on a large workspace (installed 00:43Z, `state/pc-progress.md`, not enabled for coders). It can
  need many GB per open workspace; sparse per-shard checkouts also bound that.

### Risks to this design

1. The commutation rule is only as good as the dependency graph: anything that couples crates outside Cargo (files read at
   run time, environment variables, generated code) is invisible to it. The rotating full run is the net; its interval sets
   how long such a break can sit on main.
2. Feature unification differs between `-p` selections and a full build (section 2).
3. The core shard becomes the new single lane: every mode needs it. The expand-migrate-contract path keeps core changes
   additive and small; the core's queue should get the most capacity.
4. Per-shard build folders change the letter of the one-build-folder Disk rule (section 1). Lazlo decides.
5. The section 7 counts rest on four delivered stages; the line-based and stage-based counts disagree by 4.2x.

## Follow-up briefs (proposals only; nothing is boarded until Lazlo reads this)

Each is sized by the brief rule (at most about 30 lines, 8 requirements, 1-2 files). Claims are the files as they stand on
main fefe8b1. The size table counts the proposals below as written here; "brief lines" is an estimate of the finished
`templates/brief.md` brief (these lines plus about 12 lines of template: board, task, claim, never-do, report).

| Brief | Lines here | Requirements | Files | Brief lines (est.) | Within the rule |
| --- | --- | --- | --- | --- | --- |
| F0 | 4 | 3 | 3 (stated reason) | 16 | the one exception to "1-2 files", granted by the orchestrator |
| F1 | 4 | 4 | 2 | 16 | yes |
| F2 | 4 | 4 | 2 | 16 | yes (was 5 requirements and 4 files with the setting) |
| F3 | 4 | 3 | 2 | 16 | yes (setting moved to F0) |
| F4 | 4 | 4 | 2 | 16 | yes |
| F5 | 4 | 4 | 2 | 16 | yes |
| F6 | 4 | 3 | 2 | 16 | yes |
| F7 | 4 | 4 | 2 | 16 | yes (setting moved to F0) |
| F8 | 4 | 4 | 2 | 16 | yes |
| F9 | 4 | 4 | 2 | 16 | yes (was 6 requirements and 4 files with the setting) |

Changes made in the fix round. The "new required setting" requirement of F2, F3, F7 and F9 touches `broadsword.toml`,
`config.rs` and the test configs. `grep -l disk_floor_gb` finds `config.rs`, `tests/support/mod.rs`, `tests/cli.rs`,
`tests/jobs.rs` and `tests/tools.rs` besides `broadsword.toml`, so a new key can touch more than 2 files. All four keys now
live in F0, which has no logic and states why it needs more than 2 files; the other three test files use `support::config`
and need no change; a hand-written full config found when it is boarded joins F0's claim (orchestrator exception). F0 lands first. F5 and F9 share `job.rs`, so F9 runs after F5 (a sequencing note,
not an extra requirement).

Every done check below is a command with the output it must show, runnable as written. "Shown failing once" is dropped
(it needs a step that is not built). The proof that a test can fail is a negative case inside the test set, which the
command itself runs.

### F0. Four required settings (for F2, F3, F7, F9)
- Claim: `crates/broadsword/src/config.rs`, `broadsword.toml`, `crates/broadsword/tests/support/mod.rs` (3 files). Exception granted by the orchestrator (2026-11-20 UTC), reason: a new required config key always touches all three files (AGENTS.md "Settings": the key goes in the code, the real `broadsword.toml` and every test config), so the claim cannot be split. A fourth or later file under the same exception is fine.
- Requirements: add required keys `queue_per_project` and `build_per_project` (both `false`), `python_tests` (a list, may be empty) and `task_archive_days`; a missing key is an error naming it; every test config gets them. `grep -l disk_floor_gb` lists `tests/cli.rs`, `tests/jobs.rs` and `tests/tools.rs` as well, but they build their config through `support::config(&[...])` (overrides on the defaults in `tests/support/mod.rs`), so they need no change; `tests/cli.rs` line 240 writes two deliberately broken configs and needs none. If the grep finds another test file that writes a full config by hand, add it to this claim (same exception) instead of stopping.
- Done check: `broadsword test broadsword` ends `test: ok`; a config test with one key removed expects the error naming that key, so the run covers the missing-key case.

### F1. Record coder prices (cost data for section 8)
- Claim: `scripts/dev_metrics.py`, `scripts/test_dev_metrics.py`.
- Requirements: read a `cost_usd` column in `state/agent-runs.tsv` when present; keep "unknown, never 0" for rows without one; report coder $ per merged task beside review $; a test with priced and unpriced rows.
- Done check: `python -I scripts/test_dev_metrics.py` ends `OK`; the test set includes an unpriced row that must print `unknown`, not `0`.

### F2. Per-project queue lock (step S1, part 1; after F0, the merge train and the perf gate)
- Claim: `crates/broadsword/src/queue.rs`, `crates/broadsword/tests/queue.rs`.
- Requirements: with `queue_per_project = true`, `queue.lock` is per project (`state/queue/<project>.queue.lock`); `queue run --all --project <p>` drains one project; two drivers on different projects run at once, two on one project get exit 4 "queue busy"; the entries lock stays global.
- Done check: `broadsword test broadsword` ends `test: ok`; the queue tests include one that starts two drivers on different projects (both succeed) and two on one project (the second exits 4).

### F3. Per-project build subfolders (step S1, part 2; after F0 and Lazlo's yes on the Disk rule, section 1)
- Claim: `crates/broadsword/src/buildsrc.rs`, `crates/broadsword/tests/buildsrc.rs`.
- Requirements: with `build_per_project = true`, `CARGO_TARGET_DIR` = `build/<project>/` with its own build lock and source notes; never outside `build/`; the install path is unchanged.
- Done check: `broadsword test broadsword` ends `test: ok`; a buildsrc test holds two projects' build locks at once, and a path test refuses a project name that escapes `build/` (for example `..`).

### F4. Affected-package selection, library part (step S2)
- Claim: `crates/broadsword/src/affected.rs` (new), `crates/broadsword/tests/affected.rs` (new).
- Requirements: from `cargo metadata` JSON and a changed-file list, return D, R(D) and a reason per package; global files and unmapped files return "full"; unreadable metadata returns an error (the caller runs full); no cargo call inside the function under test (JSON in, list out).
- Done check: `broadsword test broadsword` ends `test: ok`; the tests include a fixture with a 3-level reverse dependency where the leaf's dependent is selected, and a changed `Cargo.lock` that must return "full".

### F5. `check --affected <base>` (step S2, wiring; after F4)
- Claim: `crates/broadsword/src/job.rs`, `crates/broadsword/tests/jobs.rs`.
- Requirements: runs `-p` for each package of R(D); writes D and R(D) to the job record; any error from F4 runs the full check and says so; the plain `check` is unchanged.
- Done check: `broadsword test broadsword` ends `test: ok`; one test changes a leaf and asserts only it and its dependents appear in the job record, another feeds unreadable metadata and asserts the full check ran.

### F6. Mode dependency rule check (step S3)
- Claim: `crates/broadsword/src/deprule.rs` (new), `crates/broadsword/tests/deprule.rs` (new).
- Requirements: from `cargo metadata`, a group map (crate -> group) and an allow table (group -> groups), list every forbidden edge with both crates and groups; a crate missing from the map is an error; mode-to-mode edges are forbidden in the shipped table.
- Done check: `broadsword test broadsword` ends `test: ok`; the tests include a fixture where a render-mode crate depends on a physics-mode crate (rejected, naming both crates) and a crate absent from the map (an error).

### F7. Board archive (step S5; the queue half is its own brief; after F0)
- Claim: `crates/broadsword/src/task.rs`, `crates/broadsword/tests/tasks.rs`.
- Requirements: `task archive` moves done, failed and abandoned tasks older than `task_archive_days` into `state/tasks/archive/<yyyy-mm>/`; never deletes; `task show <id>` still finds archived tasks; `start` reads only the board folder, and archiving runs under the board lock.
- Done check: `broadsword test broadsword` ends `test: ok`; a test archives one old task, `task show` still prints it, `start` ignores it, and an open task is not archived.

### F8. Brief lint at `task add`
- Claim: `scripts/brief_lint.py` (new), `scripts/test_brief_lint.py` (new).
- Requirements: refuse a brief over 30 lines (excluding the done-check block), over 8 requirement bullets, or claiming more than 2 files without a stated reason; require a done check whose lines are commands; print each failed rule.
- Done check: `python -I scripts/test_brief_lint.py` ends `OK`; the test set has one over-limit brief per rule, each expected to be refused with that rule named.

### F9. Python tests behind the job interface (first multi-language runner; after F0 and F5)
- Claim: `crates/broadsword/src/job.rs`, `crates/broadsword/tests/jobs.rs`.
- Requirements: `test` and `check` run each command in `python_tests` after cargo, with the same disk guard, log, job record and exit codes; a Python failure fails the job; the scrubbed environment applies (no secrets in the child); Broadsword's own `python -I scripts/...` entries in `broadsword.toml` are a one-line follow-up outside this claim.
- Done check: `broadsword test broadsword` ends `test: ok`; a jobs test with a failing Python command asserts the job exits non-zero and the log names it.

### Decisions for Lazlo named in this document
1. Move toward ADR 0001 option (d2) for the shared core (section 1).
2. Per-shard build subfolders inside `build/` (section 1, F3).
3. Claims on a `broadsword.toml` table (section 3, rule 6).
4. Per-shard audits in place of one whole-project audit at size (section 8).
5. Installing cargo-sweep (section 8, disk).
6. A C ABI (`-capi`) boundary for the shared core, alongside the Rust API (section 3).
