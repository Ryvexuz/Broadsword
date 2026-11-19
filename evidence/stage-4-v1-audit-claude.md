# Stage 4 v1 stage-end audit, Claude auditor, 2026-11-19

**Auditor:** Claude Fable 5.2 (`claude-fable-5-2`), an independent read-only agent that built none of Stage 4 (see "Independence check"). I read the render repo and the records myself. Two read-only sub-reviewers, also Fable 5.2, read the Broadsword Rust changes and the Python changes plus the benchmark record; I checked their main points against the code before recording them.

- **Commits audited:**
  - Broadsword `main`: `7832cf85fe1b17566969c6fb9af81c10bcc448ca`
  - `projects/render` `master`: `9e1060b072742abbca9a41cd69e513e00e2f316f`
  - crash-sim `main`: `b0d237d`, for reference only
- **Scope:** the render repo at that commit; every change on Broadsword main since the last full review (`2123101..7832cf8`, 62 commits); the records `PROGRESS.md`, `LAB_LOG.md`, `RETRO.md` and `SPEC_CHECKLIST.md` against the raw evidence in `state/queue/`, `state/tasks/`, `state/jobs/`, the job logs and the golden images. No file named `evidence/*stage-4*` exists yet (finding A2).
- **Method:** main moved twice while I worked, so all code was read at the commits above with `git show` and `git diff`, not from the working copy. The two test runs were made while both working copies still stood at those commits.
- **Tests:** I ran both suites and both passed: `test: ok — 81 passed, 0 failed` for render and `test: ok — 422 passed, 0 failed` for Broadsword.
- **Duration:** about 24 minutes of wall time (18:36 to 19:00 UTC).
- **Tokens:** 477,125 for the two sub-reviewers (242,894 and 234,231). My own count is not visible to me.

**Result:** 1 BLOCKING, 7 SERIOUS and 20 MINOR findings.

The render code holds up. The rasterizer, the fill rule, the near-plane clip, the shadow map and the OBJ reader do what the README says. The tests check each picture against an independent f64 model before comparing it with a golden image, a missing or changed golden image fails, and I found no stub, skipped test, hard-coded result or weakened check. The frame-time figure in the records matches the raw job logs.

The findings are about what was proven and written down, not about the renderer. Milestone 1's done check is not shown in its own words; the evidence for both milestones is unsaved and sits in scratch; a failed check on main went unrecorded; code reached main without a reviewer PASS; and the new shadow reviewer has one unconfirmed secret risk and two record-keeping faults.

**Answers to the two done checks** (details in "Done checks"):
- **m1, "the starter cube is visible and bouncing in a window":** not proven in those words. The frames and the bounce are proven; that anything was visible in a window is not.
- **m2, "a car-shaped mesh renders at 60 fps or better":** proven by raw job logs for the code and PC measured on 18 November (8.2 to 10.0 ms a frame). Not re-measured on the audited commit or this PC, and not saved as evidence.

**Before the tag, in this order:**
1. Show the cube in a real window and record it (A1).
2. Re-run the car timing on the commit to be tagged and write the Stage 4 evidence file from raw output. Copy out of scratch whatever the records rest on: the job logs, the failed main check log and the benchmark labels (A2, A3, A24).
3. Test whether the Codex shadow reviewer can read a file outside its work folder. If it can, turn the shadow off until that is fixed (A6).
4. Record the failed main check of 18 November 05:49 UTC and settle whether that test can fail by chance (A3).
5. Decide who judges shadow-only findings, and stop recording a shadow model that nobody reported (A7, A8).

## Independence check

The brief asks me to check that I am not a model that coded any Stage 4 task, and to name the check.

- **The check:** the `owner` field of every Stage 4 task record in `state/tasks/*.task`, and the `coder` field of every queue entry in `state/queue/*.entry` for those tasks.
- **Stage 4 tasks (project `render`):**

| Task | Owner (task record) | Coder (queue entries) | Gating reviewer | Merged as |
| --- | --- | --- | --- | --- |
| `t-20261118-073303` (m1 part A) | `coder-render` | `sonnet` (`q-20261118-074111`) | `opus`, PASS | `c89e572` |
| `t-20261118-074525` (m1 part B) | `coder-replay` | `sonnet` (`q-20261118-080254`) | `opus`, PASS | `d684fa7` |
| `t-20261118-075147` (m2) | `coder-raster` | `sonnet` (`q-20261118-085505` FAIL, `q-20261118-085834` PASS) | `opus` | `30c92c4` |
| `t-20261119-171333-3` (m2 minors) | `coder-nb04` | `sonnet` (`q-20261119-174115` FAIL, `q-20261119-175739` PASS) | `haiku` | `9e1060b` |

- **Outside the queue:** render commits `170653f` (empty skeleton) and `4184930` (`.gitattributes`) were made by the orchestrator, which the records name as Opus 5.5 (`LAB_LOG.md:8`).
- **Across all 47 queue entries created since the last full review,** the `coder` field is only ever `sonnet` or `opus`.
- **Result:** I am Claude Fable 5.2 (`claude-fable-5-2`). No task record or queue entry names Fable as a coder or owner. The `owner` fields hold labels, not model names, so the model evidence is the `coder` field plus the records. Fable 5.2 did write the previous audit (`evidence/full-review-2026-11-18.md`); that is a review, not Stage 4 code.
- **My two sub-reviewers** also ran on Fable 5.2, in fresh contexts, read-only.

## Test runs

I ran both myself, in the foreground, through the installed binary. `git rev-parse HEAD` gave the audited commits before and after each run. Full output is in `scratch/stage4-audit-claude/test-render.txt` and `scratch/stage4-audit-claude/test-broadsword.txt`. The lines below are copied from those files; the file name in brackets after each result line is my note.

**`build\bin\broadsword.exe test render`** (render `9e1060b`, 18:37:03 to 18:37:18 UTC, exit 0):

```
job: test render (id 20261119-183703-render-test, ...)
source: projects/render
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.05s    [unittests src\lib.rs]
test result: ok. 26 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s   [tests\behaviour.rs]
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s    [tests\golden.rs]
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s    [tests\mesh.rs]
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 14.01s   [tests\near_clip_shared_edge.rs]
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s   [tests\raster.rs]
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s   [tests\replay.rs]
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.08s    [tests\shadows.rs]
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s    [tests\window_smoke.rs]
--- summary ---
test: ok — 81 passed, 0 failed
```

**`build\bin\broadsword.exe test broadsword`** (Broadsword `7832cf8`, 18:37:23 to 18:39:04 UTC, exit 0):

```
job 20261119-183704-broadsword-run: still running (broadsword pid 14044), left alone
job: test broadsword (id 20261119-183723-broadsword-test, ...)
source: .
test result: ok. 155 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.44s  [unittests src\main.rs]
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.87s  [tests\buildsrc.rs]
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.25s   [tests\cli.rs]
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.83s    [tests\install.rs]
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.40s   [tests\jobs.rs]
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.23s   [tests\mcp.rs]
test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.42s   [tests\mcp_call.rs]
test result: ok. 50 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 26.47s  [tests\queue.rs]
test result: ok. 22 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.44s   [tests\recovery.rs]
test result: ok. 33 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 21.27s  [tests\tasks.rs]
test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.83s   [tests\tools.rs]
test result: ok. 42 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 9.58s   [tests\worktrees.rs]
--- summary ---
test: ok — 422 passed, 0 failed
```

No test in either suite is ignored. The first line of the second run is another agent's job, which Broadsword left alone.

## Done checks

The plan (`DEVELOPMENT_PLAN.md:100-101`) gives Stage 4 v1 two done checks. The gate rule (`DEVELOPMENT_PLAN.md:15`) says a milestone is done only when its done check passes and the evidence is saved.

### m1: "the starter cube is visible and bouncing in a window"

**Does the saved evidence prove it, in those words? No.** It proves the picture and the motion. It does not prove that anything was ever visible in a window.

| Part of the sentence | Proven? | Evidence |
| --- | --- | --- |
| "the starter cube" | Yes | `tests/fixtures/trajectory.csv` is a real crash-sim run: `scratch/render-m1b/run.log` shows job `20261118-074459-crash-sim-run` printing the same first rows (`0.0000,0,-0.4585,2.0868,-0.5417`). |
| "bouncing" | Yes, in the data | My own reading of the fixture: the mean node height falls from 2.58 m (t = 0) to 0.57 m (t = 0.7), rises to 1.00 m (t = 1.0), falls again, and settles at 0.50 m from t = 2.6. No test asserts the rebound; the three golden frames are the first frame, first contact and settled. |
| the frames are drawn right | Yes | `test: ok — 81 passed` above includes `golden_replay_frame_0_cube_in_the_air_tilted`, `golden_replay_first_ground_contact` and `golden_replay_last_frame_cube_settled_on_the_ground`. Each is checked against an f64 model in the test before the byte compare. My pixel count of the three committed images: 176 to 179 edge pixels (`ff c0 40`), 72 node pixels (8 nodes of 9 pixels) and about 2,130 ground-grid pixels each. I also looked at `scratch/render-m1b/first.png` and `last.png`: a wireframe cube in the air, then resting on the grid. |
| "visible ... in a window" | **No** | Every recorded run of a render example used `--frames` (no window) or `--present` (window placed at -32000, which `src/window.rs:165` describes as "cannot be seen"). The tests use `Placement::OffScreen` and `Placement::Minimised` only. Nothing reads the window's pixels back; `present` only checks that `StretchDIBits` copied scan lines (`src/window.rs:353`). No record says a person or an agent ever ran the viewer on screen. See finding A1. |

Raw run output (job logs in `scratch/logs/jobs/`):

```
20261118-080141-render-run  cargo run --offline --release --example replay -- tests/fixtures/trajectory.csv --frames 120
replay: 120 frames of tests/fixtures/trajectory.csv (51 trajectory frames) at 640x480, average frame time 0.045 ms
20261118-080146-render-run  cargo run --offline --release --example replay -- tests/fixtures/trajectory.csv --present 30
replay: presented 30 frames
```

**What would close it:** one run of `broadsword run render -- cargo run --offline --release --example replay -- tests/fixtures/trajectory.csv` with Lazlo confirming he saw the cube fall and bounce, recorded with the commit. Or a test that opens the window on screen, captures its client area and compares it with the framebuffer.

### m2: "a car-shaped mesh renders at 60 fps or better"

**Does the saved evidence prove it, in those words? Yes for the code and machine that were measured. Not yet for the commit that will be tagged, and the evidence is not saved.**

| Part of the sentence | Proven? | Evidence |
| --- | --- | --- |
| "a car-shaped mesh" | Yes | `car_body_mesh()` and `wheel_mesh()` in `src/mesh.rs:247-309`: a body box, a cabin box and four 24-sided wheels, 408 triangles, plus a 2-triangle ground. `golden_car_on_a_ground_plane_casts_its_shadow_away_from_the_light` passes. My pixel count of `tests/golden/car_shadow.ppm`: 28,007 lit ground, 956 shadowed ground, 615 lit and 1,439 unlit body, and 217 wheel pixels. `scratch/render-m2/car_shadow.png` shows a red box car with dark wheels and its shadow on green ground. |
| "renders at 60 fps or better" | Yes, on the old PC, for the code as it stood just before commit `5d98847` | The raw job logs below: 9.954 ms and 8.223 ms a frame headless, and 9.531 ms with an off-screen present. That is 100 to 122 frames a second, against a limit of 16.6 ms. The measured loop redraws the shadow map and the whole scene every frame (`examples/car.rs:132-137`); nothing is cached between frames. |
| at the audited commit `9e1060b`, on this PC | **Not measured** | Commits `588408a` and `9e1060b` changed `src/raster.rs` (`clip_near`) and `src/mesh.rs` after the measurement. No job since then ran the car example: all 19 November render jobs are tests. I was not allowed to run it. The change only adds work for triangles that cross the near plane, so I expect no real difference, but that is an expectation, not a measurement. |
| evidence saved | **No** | See finding A2: the only raw output is in `scratch/`, and it is older than `scratch_max_age_hours`. |

Raw run output (job logs in `scratch/logs/jobs/`, source `worktrees/render/t-20261118-075147`, 2026-11-18 08:53 UTC):

```
20261118-085344-render-run  cargo run --offline --release --example car -- --frames 120
car: 120 frames at 640x480, shadow map 1024x1024, 410 triangles (408 cast shadows, 199 drawn in the main pass)
car: average frame time 9.954 ms = shadow pass 6.313 + main pass 3.535 + other 0.105; blit: none, no window in this mode
20261118-085352-render-run  cargo run --offline --release --example car -- --frames 120
car: average frame time 8.223 ms = shadow pass 5.442 + main pass 2.703 + other 0.078; blit: none, no window in this mode
20261118-085354-render-run  cargo run --offline --release --example car -- --present 30
car: presented 30 frames at 640x480
car: average 9.531 ms per frame = shadow pass 6.173 + main pass 3.152 + blit 0.085 + other 0.121
```

**What would close it:** rerun the three commands on the commit to be tagged, on this PC, and save the output in `evidence/`.

## Findings

How to read the tables: **Where** gives `file:line` at the audited commits (Broadsword `7832cf8` unless the path starts with `projects/render`, which is at `9e1060b`). Paths under `state/` and `scratch/` are outside Git and were read from disk. A row marked *(sub-reviewer)* rests on a sub-reviewer's reading that I did not fully re-check; every other row I checked myself.

Severity as used here:
- **BLOCKING:** must be fixed before the stage tag: a rule that must always hold is broken, or a done check is not proven.
- **SERIOUS:** a real defect, a rule that is not in force, or a claim the raw evidence does not support.
- **MINOR:** a small defect or a gap already known and tracked.

## BLOCKING

| # | Severity | Where | What is wrong | Evidence | How to confirm |
| --- | --- | --- | --- | --- | --- |
| A1 | BLOCKING | `DEVELOPMENT_PLAN.md:100`; `projects/render/src/window.rs:147-149,165`; `projects/render/examples/replay.rs:113`; `projects/render/tests/window_smoke.rs:11,29` | Milestone 1's done check says the cube is "visible ... in a window". Nothing shows that it ever was. Every automated mode places the window off every monitor. No test or tool reads the window's pixels back, and no record says anyone looked at the viewer on screen. Tagging `stage-4-v1-done` now would claim a done check that is not shown in its own words (`AGENTS.md:19`, Done means verified). The code path for an on-screen window differs only in position and show command, so I expect it works; nobody has verified it. | All six recorded runs of a render example (`state/jobs/20261118-074018`, `-080141`, `-080146`, `-085344`, `-085352`, `-085354`, each `-render-run.rec`) has `--frames 120` or `--present 30` in its `command =` line. `window.rs:165`: "Shown far off every monitor and without activation: never steals focus and cannot be seen. For tests." | `grep -l "example" state/jobs/*-render-run.rec`, then read each file's `command =` line. Then run the replay viewer without `--frames` or `--present` and look. |

## SERIOUS

| # | Severity | Where | What is wrong | Evidence | How to confirm |
| --- | --- | --- | --- | --- | --- |
| A2 | SERIOUS | `evidence/` (no Stage 4 file); `scratch/logs/jobs/20261118-08*-render-run.log`; `broadsword.toml:8`; `DEVELOPMENT_PLAN.md:15` | The evidence for both done checks is not saved. The only raw output of the frame-time and replay runs is in `scratch/`, which may be wiped at any time, and the files are already older than `scratch_max_age_hours = 24`, so the next `broadsword cleanup` deletes them. The job records in `state/jobs/` keep the command and status, not the output. The 60 fps figure was measured in the task worktree just before commit `5d98847`, on the old PC (paths under `C:\Users\setup`); it has not been measured on the audited commit or on this PC. Milestone 2 started at 08:33:55 UTC on 18 November, straight after milestone 1 merged, with no evidence saved, against the gate rule. | `git ls-tree --name-only 7832cf8 evidence/` lists 18 files, none with `stage-4` in its name. The car runs are dated 2026-11-18 08:53 UTC; this audit ran on 2026-11-19 from 18:36 UTC. I copied the key lines into the Done checks section above so they survive a cleanup. | Run that `git ls-tree` command. `ls scratch/logs/jobs/20261118-0853*`. |
| A3 | SERIOUS | `state/queue/q-20261118-054321.entry`; `scratch/logs/jobs/20261118-054913-broadsword-check.log:350-353`; `crates/broadsword/tests/queue.rs:1077-1104` (at `1280f06`); `PROGRESS.md:27` | A failed check on main was never recorded. The queue fix for audit findings S2, S4 and S7 (`1280f06`) passed review and merged, then the check on main failed: the test `a_second_queue_run_gets_busy_while_one_is_running` found the entry `blocked` where it asserts `merged`. The entry ended `merged_check_failed` and says a human must decide and that the task was not marked done. The task was marked done about ten minutes later. `PROGRESS.md:27` lists S2, S4 and S7 as "Fixed and merged through the queue"; no record names the failure, its cause or who decided. The test has not changed since and passes today, so either the test can fail by chance on the merge gate, or the queue has a real race. My guess, not verified: the test's process-tree kill sometimes ends the fake reviewer first, and the queue run records `blocked` before it dies. The raw log is in `scratch/` and will be wiped. | Log: `assertion left == right failed`, `left: "blocked"`, `right: "merged"`, `test result: FAILED. 44 passed; 1 failed`. Entry: `status = merged_check_failed`; `reason = ... check broadsword there did not pass (exit 1, job 20261118-054913-broadsword-check) ... Main was NOT reset or reverted. A human decides: fix forward, or git revert the commit. The task was NOT marked done and its worktree is kept`. | `grep -E "^(status) = " state/queue/q-20261118-054321.entry`. `git grep -n -i "gets_busy" 7832cf8 -- "*.md"` prints nothing. |
| A4 | SERIOUS | `scripts/dev_metrics.py` (commits `e188dd4`, `79a0656`, `880f7da`, `baaf804`, `b9ae9e5`); `bench/reviewer/run_openai.py` (`f3bdd38`, `ec7a479`); `broadsword.toml` (`f4123a1`); `projects/render/.gitattributes` (`4184930`); `AGENTS.md:15,32`; `RETRO.md:88` | Nine commits reached a main branch with no reviewer PASS. For both tools, the commit on main is exactly the commit a reviewer last FAILED. `dev_metrics.py` is the orchestrator's own code (`coder = opus`), so the author overruled its reviewer five times. The records are open about this (`LAB_LOG.md:37`, the task notes) and cite Lazlo's "take the file" rule; I cannot verify his approval from raw evidence. `AGENTS.md:32` lists "an independent review before every merge" among the rules that never change, and `AGENTS.md:15` says all code reaches main through the queue. My sub-reviewer re-checked every blocking point of the two last FAILs against the code and found each one wrong, so no known bug came in this way. But nothing independent had checked that before the merges. This is the count the ledger row at `RETRO.md:88` asks for and does not give. | `q-20261118-074036`: `verdict = FAIL`, `checked_tip = b9ae9e5...`. `q-20261118-081559`: `verdict = FAIL`, `checked_tip = ec7a479...`. Both are the newest commits on those files at the snapshot. Of the non-document commits in `2123101..7832cf8`, eight lie outside every merged entry's `base_head..main_sha` range. | `git merge-base --is-ancestor b9ae9e5 7832cf8` exits 0. `git log -1 --format=%h 7832cf8 -- scripts/dev_metrics.py` prints `b9ae9e5`. |
| A5 | SERIOUS | `crates/broadsword/src/git.rs:65`; `wtgit.rs:23`; `buildsrc.rs:151`; `cleanup.rs:79,89`; `pause.rs:77,102`; `reviewer.rs:507-510`; `scrub.rs:1-2,38` | The fix for S6 was merged as "One secret boundary for every child" (`5aa31fd`), but the boundary covers two spawn sites only: `job.rs:720` and `mcp.rs:686`. Git children (which run a repo's hooks), `cargo metadata`, `cargo clean`, `cargo sweep` and the `alert_command` shell inherit the full environment. The reviewer child is scrubbed by a second mechanism, a loop of `env_remove`. No test checks that every spawn is scrubbed. The exposure today is small: secret-named variables in the parent's environment, and `alert_command` is empty. It is still the pattern playbook lesson 22 names: a rule set in some builders and missed by other paths. `PROGRESS.md:54` words it more narrowly ("jobs and MCP tools"), which is accurate. | A search of `src/` finds 17 `Command::new` lines (a few of them in test code) and 2 `scrub::apply` calls. `scrub.rs:1`: "The ONE secret boundary for child processes. Every child Broadsword starts for a job ... and every tool child the MCP server starts". | `git grep -n "scrub::apply" 7832cf8 -- crates/broadsword/src`, then the same search for `Command::new`. |
| A6 | SERIOUS, not confirmed | `clients/openai_review.py:46,187,209` *(sub-reviewer; I read the lines)* | The shadow reviewer's Codex path starts `codex exec --sandbox read-only`: an agent that can read files, given a prompt that contains an untrusted diff. The `--tools ""` argument is parsed and ignored. The gating Claude reviewer has no tools. If the read-only sandbox lets Codex read any file on disk (I believe it does; not tested), a diff that carries instructions could make it read `.env` and put the contents in its answer and in what it sends to OpenAI. On this path the Python side redacts only `sk-` shaped strings from the answer. This runs on every merge. If a test confirms it, it is blocking under the Secrets rule. | `:187`: `cmd = [exe, "exec", "-m", args.model, "--sandbox", "read-only", "--ephemeral", "--skip-git-repo-check", "--json", "-o", str(answer), "-"]`. `:46`: `p.add_argument("--tools", default="")`, never read again. | Run one shadow review on a throwaway diff whose text asks the reviewer to quote a harmless canary file that sits outside its work folder, and see whether the answer contains the canary. |
| A7 | SERIOUS | `clients/openai_review.py:212-213`; `crates/broadsword/src/reviewer.rs:474-481`; `state/queue/*.shadow.review` | The record says which model shadow-reviewed, but nobody checked. On the Codex path the wrapper fills `modelUsage` with the model name that was asked for, so the Rust check `check_shadow_model` can never fail. All 13 shadow review files carry `"model_reported": false` in the raw result, while their header says `models reported: gpt-5.6-sol` and the entries store `shadow_model_used = gpt-5.6-sol`. Playbook lesson 32 says to check that the model asked for is the model that answered. These results feed the contest that will pick a reviewer model. | `q-20261119-175739.shadow.review` contains `models reported: gpt-5.6-sol`, `"model_reported": false`, `"backend": "codex"`. | `grep -l "model_reported.: false" state/queue/*.shadow.review` lists all 13 files. |
| A8 | SERIOUS | `SPEC_CHECKLIST.md:46`; `RETRO.md:96`; `state/shadow-verdicts.tsv` | Lazlo's rule says every finding only the shadow reviewer raised gets a verdict from "an independent strong judge (neither the coder nor the gating reviewer)". None of the 24 recorded verdicts has such a judge: 17 were given by the gating reviewer itself (16 Haiku, 1 Opus) and 7 by the orchestrator. Three findings were ruled `false` by the same reviewer that had missed them. The ledger row names this harm ("verdicts by a Claude judge favouring Claude"). Either the practice or the rule has to change; a change to the rule needs a ledger row under the Self-optimization rule. | Judge column counts: `haiku (gating)` 16, `opus (gating)` 1, `orchestrator (...)` 7. Verdicts: 10 real, 11 known, 3 false. | `cut -f5 state/shadow-verdicts.tsv`, then count the values. |

## MINOR

| # | Severity | Where | What is wrong | Evidence | How to confirm |
| --- | --- | --- | --- | --- | --- |
| A9 | MINOR | `projects/render` commits `588408a`, `9e1060b`; `state/tasks/t-20261119-171333-3.task`; `state/queue/q-20261119-175739.entry` | The last render merge changed files outside its claim, on a low-risk (Haiku) review of rasterizer code. The brief's claim allowed `src/raster.rs`, `src/mesh.rs` and new files under `tests/`. The merge also changed `README.md` and the existing `tests/mesh.rs`. The brief itself asked for a README note while leaving README out of the claim. The shadow reviewer raised the `tests/mesh.rs` point as blocking; it was ruled real and the merge went ahead. It is recorded only in `state/shadow-verdicts.tsv`. | `git diff --stat 30c92c4 9e1060b`: `README.md 3 +-`, `tests/mesh.rs 39 +`. Entry: `risk = low`, `reviewer_model = haiku`, `shadow_verdict = FAIL`. | `git -C projects/render diff --stat 30c92c4 9e1060b` |
| A10 | MINOR | `projects/render/.gitattributes:1` (commit `4184930`); `LAB_LOG.md:39` | No reviewer has seen 9 of the 15 golden images. The orchestrator committed `*.ppm binary` to render master outside the queue, so that image diffs would fit the reviewer's size limit. Since then a review prompt shows only that a binary file changed: the three replay images, the five milestone 2 images and `near_clip_shared_edge.ppm`. The protection that remains is the f64 model each test runs before the byte compare, which reviewers did read. The gap is known as S5 and has an open task (`t-20261119-171333-11`). | Commit message of `4184930`: "Mark *.ppm binary (orchestrator, outside the queue)". | `git -C projects/render log --format="%h %s" -1 4184930` |
| A11 | MINOR | `PROGRESS.md:3,35,66,111` | PROGRESS at the snapshot describes 18 November. It gives render master as `30c92c4` with 77 tests (now `9e1060b`, 81), crash-sim main as `abe471c` (now `b0d237d`), and still lists as a known minor that `OPENAI_API_KEY` reaches the Codex child, which `3d57b02` fixed. A session was in progress during the audit, so this is expected to be rewritten at its end; it must be before the tag. | `PROGRESS.md:35`: "The render repo's master is at `30c92c4`, with 77 tests." | `git show 7832cf8:PROGRESS.md`, lines 3, 35, 66, 111. |
| A12 | MINOR | `LAB_LOG.md:31`; `PROGRESS.md:28-29` | LAB_LOG says "Every serious audit code finding is fixed and merged through the queue." PROGRESS lists S3 and S5 as still open, and both are findings about `queue.rs` and `reviewer.rs`. One of the two is an over-claim. | The two quoted lines. | `git show 7832cf8:LAB_LOG.md`, line 31. |
| A13 | MINOR | `RETRO.md:79,226`; `RETRO.md:105-160` | The last audit's retro carried "No new process machinery in Stage 4 unless an audit finding needs it." Stage 4 then added a shadow reviewer, a Codex wrapper, three reviewer and coder benchmarks, a metrics script and a RAM gate. Since that retro, main has 12 commits touching Rust, 13 touching Python tooling and 35 touching only records, against 9 commits in the whole render repo. Lazlo asked for much of this, but the Stage 4 retro neither applies the carry item nor says it was dropped. | `RETRO.md:226`: "3. No new process machinery in Stage 4 unless an audit finding needs it." | Count with `git rev-list --count 2bf71c3..7832cf8` (61) and `git -C projects/render rev-list --count 9e1060b` (9). |
| A14 | MINOR | `AGENTS.md:17`; `crates/broadsword/src/queue.rs:230-246` | Two rules collide. AGENTS says each brief lives in `state/backlog/<task id>.md`. `queue add --brief` refuses any file inside `state_dir`. So every brief is copied into `scratch/` before queueing (`scratch/briefs/`, `scratch/m2-brief.md`), and the copy the reviewer sees can differ from the backlog brief and can be wiped. The refusal message is also wrong about job logs: they live in `scratch/logs/jobs/`, which is accepted as a brief source, as are `.git/config` and `.claude/settings.local.json` *(sub-reviewer for the last two)*. Read from the code, not run. | `queue.rs:244`: "the brief ... is inside {name}; queue records, job logs and build output are never briefs". `broadsword.toml:15`: `state_dir = "state"`. | `git show 7832cf8:crates/broadsword/src/queue.rs`, lines 230-250. |
| A15 | MINOR | `crates/broadsword/src/tools.rs:456-459,482,503`; `recover.rs:140-162` | Any model connected over MCP can call `recover` with `all = true`, which rolls back every interrupted `run` job in every folder whose Broadsword process is gone, worktrees included. That is the behaviour the previous audit flagged as S1, now behind one boolean. The only guard is a sentence in the tool description. The edits stay in refs, so nothing is lost. | `tools.rs:459`: "Dangerous: use only when the user explicitly asks." `tools.rs:482`: `mcp_callable: true`. | `git show 7832cf8:crates/broadsword/src/tools.rs`, lines 454-504. |
| A16 | MINOR | `crates/broadsword/src/mcp.rs:45,48,55`; `reviewer.rs:432`; `config.rs:241,517-538`; `clients/openai_review.py:103-106,129-130` | Limits outside `broadsword.toml` remain. In Rust: the MCP line limit (1 MiB), pending-call limit (256) and pipe grace (2 s), and the shadow script path. In Python: commit `7832cf8` says the Codex deadline and share come from `OPENAI_REVIEW_TIMEOUT` and `OPENAI_REVIEW_CODEX_SHARE`, but nothing sets them, so the limits are the constants 240 and 0.6; the code says so itself ("TEMPORARY"). `ram_warn_percent` has no range check, so 101 would silently switch the RAM gate off. The test that every setting is used is a text search for `.key` in `src/`, so a comment counts as a use. | `git grep -n -E "OPENAI_REVIEW_" 7832cf8 -- crates broadsword.toml` prints nothing. `openai_review.py:105-106`: `LEGACY_TIMEOUT = 240.0`, `LEGACY_CODEX_SHARE = 0.6`. | That `git grep`. |
| A17 | MINOR | `AGENTS.md:40,70`; `broadsword.toml:9`; `PROGRESS.md:98` | AGENTS still shows `max_agents = 2` in its settings sample; the setting is 6. AGENTS also states hourly automatic cleanup and a phone alert as rules; PROGRESS says neither runs yet. The rule text is not marked "not yet in force" (playbook lesson 28). The previous audit raised both points (S9, S11). | `AGENTS.md:70`: `max_agents = 2`. `broadsword.toml:9`: `max_agents = 6`. | `git show 7832cf8:AGENTS.md`, line 70. |
| A18 | MINOR | `.claude/settings.json` (hook command); `crates/broadsword/src/pause.rs:129` | PROGRESS says the pause hook fails closed. The hook command runs `"$b" hook pre-tool`; if neither binary exists the shell returns 127, which a Claude Code hook treats as "do not block". And `broadsword resume --force` is on the hook's allow-list, so a paused agent can lift the pause itself *(sub-reviewer)*. Read from the files, not run. | The hook command falls back from `build/bin/broadsword.exe` to `build/debug/broadsword.exe` and has no check that either exists before running it. | `git show 7832cf8:.claude/settings.json` |
| A19 | MINOR | `crates/broadsword/src/queue.rs:975,1013,1033` | A failure while saving the shadow review can end a queue run, against the function's own comment that no shadow failure "ends or blocks the entry": the calls at `:975` and `:1013` pass the error up with `?`. The shadow also runs before a gating PASS is merged, so a slow shadow can make an entry stale. Both are tracked (IDEAS line 62, task `t-20261119-171333-15`). The shadow's error text is printed to the console without redaction at `:1033` *(sub-reviewer)*. | `queue.rs:975`: `self.s_shadow(&prompt)?;` | `git show 7832cf8:crates/broadsword/src/queue.rs`, lines 970-1035. |
| A20 | MINOR | `crates/broadsword/src/reviewer.rs:200`; `.gitignore` (the `.env` line); `crates/broadsword/src/job.rs:561`; `mcp.rs:752` | Smaller secret gaps. Secret values are read from `<root>/.env` only, while the checkpoint rule treats every `.env*` file as secret; today only `.env` exists. `.gitignore` lists `.env` and not `.env.*` (open task `t-20261119-171333-17`). A `run` job's whole command line is stored unredacted in `state/jobs/*.rec`. The MCP log is redacted only after the first completed tool call *(sub-reviewer for this last point)*. | `reviewer.rs:200`: `std::fs::read_to_string(root.join(".env"))`. `job.rs:561`: `rec.set("command", command_text);` | `git show 7832cf8:crates/broadsword/src/reviewer.rs`, lines 198-212. |
| A21 | MINOR | `clients/openai_review.py:100,143` *(sub-reviewer; I read line 100)* | The Codex child's environment is built by a second, weaker rule than the one in `clients/openai_mcp.py`: it drops `OPENAI_API_KEY` by name and variables whose value equals a `.env` value, with no secret-name rule and no "contains" match. It is safe when the queue calls it, because Rust scrubs first; run by hand or from tests, a variable such as `ANTHROPIC_API_KEY` set outside `.env` would reach Codex. `taskkill` is started by bare name from the review work folder, the same class of lookup that `3d57b02` closed for `codex`. | `:100`: `return {k: v for k, v in environ.items() if k.upper() != "OPENAI_API_KEY" and v.strip() not in secrets}` | `git show 7832cf8:clients/openai_review.py`, line 100. |
| A22 | MINOR | `projects/render/tests/common/mod.rs:17,21,32`; `projects/render/tests/golden.rs:40,46,58`; `crates/broadsword/tests/jobs.rs:342`; `clients/openai_review.py:185` | Temp files outside `scratch/` (`AGENTS.md:39`). A failing golden test writes `<name>.actual.ppm` into `tests/golden/` in the source tree (git-ignored; the brief asked for this). One Broadsword test and the Codex wrapper write to the system temp folder. | `jobs.rs:342`: `std::env::temp_dir().join(...)`. `openai_review.py:185`: `tempfile.TemporaryDirectory(prefix="bsw-codex-")`. | `git grep -n "temp_dir()" 7832cf8 -- crates/broadsword/tests` |
| A23 | MINOR | `scripts/dev_metrics.py:72-85`; `scripts/review_costs.py:43`; `scripts/shadow_report.py:46-59` | The metrics scripts drifted when the shadow reviewer arrived. `dev_metrics.py` times a step from one `: begin` log mark to the next, and the shadow has no mark, so its time is counted as review time: in `q-20261119-175739` the review step is 82 s, of which the gating review took 25 s and the shadow 57 s. `review_costs.py` globs `*.review`, which now also matches the 13 `*.shadow.review` files. `shadow_report.py` run from another folder prints that no entry has a shadow review and exits 0 *(sub-reviewer)*. None of the three scripts has a test. The retro figures already published predate the shadow. | Entry log: `18:10:44 review: begin`, `18:11:09 review: PASS`, `18:12:06 shadow: FAIL`, `18:12:06 merge: begin`. The queue's step names are preconditions, rebase, check, review, merge, main_check and done. | `git show 7832cf8:scripts/dev_metrics.py`, lines 72-85. |
| A24 | MINOR | `evidence/benchmarks-2026-11-18.md:5,79,166` *(sub-reviewer; I checked line 79 and the two file facts)* | The benchmark record is behind its raw data and part of that data is in scratch. Line 79 (`claude-opus-4-5 default 1/2 ... 0.336`) is a mid-run snapshot: 0.336 is the cost of the first two of ten calls in `state/bench-results/reviewer/run.log:43-52`. The Claude 4.5 Sonnet and Haiku rows are missing. Line 5 says the raw outputs are in `state/bench-results/`, but the blind labels behind the hard-case table exist only in `scratch/rescore/labels.json`, and `state/bench-results/coder/score.txt` is empty. Line 166's "every row from gpt-5.5 onward shows 0%" is loose: gpt-5.5 shows 58 of 97. The sub-reviewer reproduced the headline numbers (Sonnet high 10 of 12; nine each for gpt-5.6-sol, Fable and Opus; no decoy flagged; the 100% coding rows) from the raw files. | `run.log:43-44`: costs 0.12444 and 0.21204, which sum to 0.336. `ls -la state/bench-results/coder/score.txt` shows 0 bytes. | `grep -n "opus-4-5" state/bench-results/reviewer/run.log` |
| A25 | MINOR | `projects/render/tests/`; `projects/render/examples/car.rs:34`; `DEVELOPMENT_PLAN.md:45` | Nothing guards the frame rate, and one design choice is unrecorded. No test or check fails if the car scene gets slower than 16.6 ms; the figure rests on three manual runs. It was measured for 410 triangles at 640x480 with about 1.7 to 2 times headroom. The plan's shared core names "GPU layer: wgpu"; Stage 4 v1 is a std-only software renderer on one thread. That may well be the right call for now, but no record (LAB_LOG, RETRO, IDEAS) gives the reason or what it means for the later "full track" milestone. | `git grep -n -i wgpu 7832cf8 -- "*.md"` finds it only in `DEVELOPMENT_PLAN.md` and `CAPABILITY_ROADMAP.md`. | That `git grep`. |
| A26 | MINOR | `projects/render/README.md:113`; `projects/render/src/mesh.rs:104-106,117` | The README says any finite `vn`, "however small or large", is normalised. `vn 1e-50 0 0` parses to 0.0 as `f32` and is refused as the zero vector, and `vn 1e39 0 0` parses to infinity and is refused as not finite. The Haiku reviewer listed this as a minor on the merged entry; it is not in IDEAS at the snapshot. | `mesh.rs:104-106`: the value is parsed with `w.parse::<f32>()` and refused unless finite, before the f64 normalisation. | `git -C projects/render show 9e1060b:src/mesh.rs`, lines 98-127. |
| A27 | MINOR | `LAB_LOG.md:3`; `DEVELOPMENT_PLAN.md:203`; `projects/crash-sim` commit `b0d237d` | Stage close-out items. LAB_LOG has lesson blocks for Stages 1 and 2 but none for Stage 3, which is tagged. Stage 0 is still untagged (PROGRESS says why). A Stage 6 document (`docs/STAGE6.md`) was merged into crash-sim on 19 November, before the Stage 4 tag; writing a later stage's done checks early is allowed by the plan, so I note it only because Lazlo's recorded order was backlog, Stage 4 audit and tag, then onward. | `git show 7832cf8:LAB_LOG.md` has headings "Stage 2 lessons" (line 264) and "Stage 1 lessons" (line 402) and no "Stage 3 lessons". | Search `LAB_LOG.md` at `7832cf8` for the word "lessons". |
| A28 | MINOR | `crates/broadsword/tests/tasks.rs:657-700`; `crates/broadsword/src/task.rs:394`; `clients/test_openai_review.py:225-230` *(sub-reviewer)* | Test gaps. The RAM gate is tested at 90 and at 1 against real memory use, never at the boundary, so changing `>=` to `>` at `task.rs:394` would pass. One assertion in the Codex lookup test cannot fail, because the planted `codex.cmd` never writes the file the test looks for. No test checks that the shadow child's environment is scrubbed. | `task.rs:394`: `Ok((used, free)) if used >= cfg.ram_warn_percent as f64 => {` | `git show 7832cf8:crates/broadsword/tests/tasks.rs`, lines 657-700. |

## Confirmed true by the auditor

- **Test counts:** 422 Broadsword tests (`PROGRESS.md:66`) and 81 render tests at `9e1060b`. The 77 in PROGRESS matches the queue's check at `30c92c4`.
- **Stage 4 merges:** `c89e572`, `d684fa7`, `30c92c4` and `9e1060b` each have a queue entry with a PASS from a model other than the coder, through the real reviewer CLI (`test_hook = false`), merged at exactly the checked and reviewed commit, with a passing check on main afterwards. Every queue entry that records a `test_hook` value (59 of them) has `false`.
- **Frame time:** "8 to 10 ms a frame at 640x480 with shadows" (`PROGRESS.md:34`) and the split in the render README (line 103) match the raw job logs.
- **Fixture:** the replay fixture is a real crash-sim run, and its cube does bounce.
- **Golden images:** a missing or changed golden image fails the test, and nothing accepts one automatically. The pixel counts of the six committed images I examined match the tests' claims; for example `shared_edge.ppm` holds 1,342 red and 1,482 blue pixels, the numbers printed in job `20261118-085805`.
- **No faked success in render:** no ignored test, no environment switch, no stub and no hard-coded result in `src/`, `tests/` or `examples/`. The coder showed that the new tests can fail: jobs `20261119-173926` and `20261119-174055` are the failing runs without the fixes.
- **No faked success in the Broadsword Rust changes** (sub-reviewer): no ignored test; each of the 13 removed assertions was replaced by an equal or stricter one; the test seams only make a guard stricter.
- **Previous findings fixed in the code** (sub-reviewer): B1, S1, S2, S7, S8 and S16, and the scratch-hook half of S4. S6, S9 and S11 are fixed in part (A5, A16, A17, A18).
- **Secrets at rest:** none of the 356 `refs/broadsword/*` refs in Broadsword, the 49 in render or the 207 in crash-sim holds a path named `.env*`. No key-shaped string and no absolute user path is in the tracked files, apart from one fake key and one path inside test code.
- **The Codex wrapper's stated fixes hold** (sub-reviewer): the child gets no `OPENAI_API_KEY`, `codex` is found only through absolute PATH entries, a timeout kills the whole process tree, and an empty or unparseable answer is recorded as an error, never as a verdict.
- **Claims never overlapped:** milestone 2's task was created at 07:51 UTC but started at 08:33:55, after part B ended at 08:29:52; both claim the whole render repo.
- **Build output:** no worktree holds private build output; render builds into the shared build folder.
- **Backups as of the snapshot:** `origin/main` is at `a4f00c4`, `render-master` at `30c92c4` and `crash-sim-main` at `abe471c`, as PROGRESS says for 18 November. The two commits made in each repo on 19 November were not yet pushed.
- **Review totals:** the queue records are consistent with LAB_LOG's 18 Opus and 22 Haiku reviews and RETRO's five real Opus FAILs for the second half of 18 November. I matched the counts, not every case.
- **The benchmark headlines** (sub-reviewer): reproduced from the raw files, with the exceptions in A24.

## Not reviewed

- **Not run:** the `car`, `replay` and `spin` examples, any on-screen window, the Python tests and scripts, crash-sim, clippy, a real reviewer call, a shadow call and Codex. My brief allowed only the two test suites.
- **Render files I did not read in full:** `src/camera.rs`, `src/framebuffer.rs`, `src/math.rs`, most of `src/replay.rs`, `examples/spin.rs`, and the tests `behaviour.rs`, `replay.rs`, `shadows.rs`, `mesh.rs`, `near_clip_shared_edge.rs` and the second half of `raster.rs`. I read `src/raster.rs`, `src/mesh.rs`, `src/window.rs`, the `car` and `replay` examples, `tests/golden.rs`, `tests/window_smoke.rs`, the golden checker and the first 330 lines of `tests/raster.rs`, and searched the rest for skips, stubs and switches.
- **Broadsword Rust, not read by the sub-reviewer:** `install.rs`, `sys.rs`, `record.rs`, `worktree.rs`, `lock.rs` and `json.rs` (unchanged since the last review); the test diffs of `buildsrc`, `install`, `worktrees`, `tools`, `mcp`, `mcp_call`, `cli` and `fake_claude` were only searched for ignores and removed assertions; `buildsrc.rs` was read at its changed lines only.
- **Python, not read by the sub-reviewer:** `openai_mcp.py` outside its diff; section 4 of the benchmark record and its cost and time columns beyond spot checks.
- **Other files:** `IDEAS.md`, `CAPABILITY_ROADMAP.md`, `docs/MOVING.md` and `docs/claude-memory/` were only searched. crash-sim was a reference; I did not read `docs/STAGE6.md`.
- **Lazlo's approvals** (the pushes, the two direct merges, the shadow reviewer's spending) are recorded only in the orchestrator's words. I could not verify them.
- **The PNG pictures** I looked at are scratch copies made on 18 November. I did not check them byte for byte against the committed images; the pixel counts were made on the committed files.

## Cost and notes

- **Wall time:** about 24 minutes (18:36 to 19:00 UTC on 2026-11-19).
- **Tokens:** 477,125 for the two sub-reviewers; the main agent's count is not visible to me.
- **Files I created:** this file, and `scratch/stage4-audit-claude/` (the two test outputs and three small work lists). My role has no file-writing tool, so this file was written with a shell command, as the orchestrator asked.
- **One breach to disclose:** against its brief, the Python sub-reviewer wrote two temporary copies of `bench/reviewer/cases.json` into the system temp folder. Nothing in the project was touched.