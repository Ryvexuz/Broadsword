# Full review of Broadsword (Stages 0–3), 2026-11-18

**Reviewer:** Claude Fable 5.2, high effort, an independent read-only subagent. It read the high-risk code itself and handed the MCP surface, the integration tests, the process documents and crash-sim to four read-only sub-reviewers.
- **Duration:** about 28 minutes.
- **Tokens:** about 334k for the main agent.
- **Tests:** it ran both suites, and both passed: 355 Broadsword tests and 40 crash-sim tests.

**Result:** 1 BLOCKING, 17 SERIOUS and 14 MINOR findings.

The core safety code holds up: rollback, worktree removal, binding a merge to the exact reviewed commit, and locks. The findings are about secrets, recovery scope, queue edge cases, review independence, rules that are not in force, overstated claims, and crash-sim defects.

## Checked by the orchestrator after the review

| Finding | How I checked it | Result |
| --- | --- | --- |
| B1: `.env` in checkpoint refs | `git hash-object --no-filters .env` against the trees of `refs/broadsword/checkpoints-ignored/*`. Hashes only; no value was printed. | **Confirmed.** 21 of 22 refs hold the current `.env` blob. |
| B1: did it leave the machine? | `git reflog` push entries; `refs/remotes`. | **No.** The only push ever made was `e514156` (Stage 0, 2026-11-17 10:25), before any checkpoint ref existed. Only `origin/main` is tracked, and a normal push never sends `refs/broadsword/*`. crash-sim has no remote. |
| S2: queue wedge after a recorded FAIL | Read `queue.rs`: `s_review` returns early once any verdict is recorded, then `s_merge` refuses without a PASS. | **Confirmed.** Nothing merges, but the entry gets stuck. |
| S13: PHYSICS.md is stale | `docs/PHYSICS.md:59` | **Confirmed.** It still says the damping condition is "not checked by `validate()`". |
| S14: summary misses contact | `crash-sim --duration 0.64 --summary` | **Confirmed.** It prints `first_contact_s=none,peak_strain=0.014177`. |

## BLOCKING

- **B1. Every job's checkpoint stores git-ignored files, `.env` included, as commits** under `refs/broadsword/checkpoints-ignored/<job>`.
  - **Where:** `git.rs:209-241`, called from `job.rs:605`. `broadsword.toml:30` documents this as intended.
  - **Why it matters:** it breaks "never put keys in commits". The refs are never pruned; every worktree can read the blob; a mirror push, a bundle or a copy of the repo would publish it.
  - **Also:** checkpoints of build, test and check jobs are never used. Only `run` is ever rolled back.

## SERIOUS

| # | Finding | Where |
| --- | --- | --- |
| S1 | `recover` rolls back every interrupted `run` in every folder, including one under a live agent that kept editing. The edits are kept in refs, but the working tree is reverted. There is no `recover <id>`. | `recover.rs:126-221`, `tools.rs:406-425` |
| S2 | A FAIL verdict followed by a crash or a failed log write wedges the queue. Every `queue run` then exits 1, `cancel` refuses, and `add` refuses. | `queue.rs:865, 933, 981-988, 460` |
| S3 | Independent review isn't enforced. Three orchestrator commits (`16c9279`, `b578103`, `d23b969`) were reviewed by headless calls outside the queue. `--coder` is free text, `--risk` is the caller's choice, and direct commits to main are possible. | `queue.rs:236-241` |
| S4 | A stand-in reviewer program can merge. `BROADSWORD_REVIEWER_CLI` pointing outside the root is accepted as a real reviewer, and `fake_claude` ships as a product binary. All 13 real reviews so far used the real CLI. | `reviewer.rs:394-399`, `tests/queue.rs:794-799` |
| S5 | A queue PASS doesn't mean the done check was run. The reviewer has no tools and one turn; it sees only the diff and the end of the test output. Binary files and files marked `-diff` are invisible to it. | `reviewer.rs:418`, `wtgit.rs:262` |
| S6 | The MCP server gives tool children the full environment, and their output goes back unredacted. There are three separate scrubbers with different rules (client, reviewer, none). | `mcp.rs:671-673`, `job.rs:686`, `openai_mcp.py:271`, `reviewer.rs:178` |
| S7 | `queue add --brief` reads any local file under 32 KB except `.env*`. It stores it, shows it, and sends it to the reviewer model. | `queue.rs:218-233` |
| S8 | Cancelling a queued, not-yet-running MCP call is ignored. A cancelled `queue_run` still runs later. | `mcp.rs:776-781` |
| S9 | Automatic cleanup isn't in force. Nothing starts `watch`, a low-disk kill doesn't attempt cleanup, cargo-sweep isn't installed, and alerts are empty. The pause hook fails open on a config error and allows by substring. | `tools.rs:1215`, `job.rs:709`, `main.rs:73`, `pause.rs:128` |
| S10 | Stage 2 claims are overstated. A secrets fix merged after FAIL, FAIL, then an orchestrator read, with no independent PASS. The "fresh session that has never seen the project" prompt named the server, the project and the action. | `LAB_LOG.md:196`, `evidence/stage-2-m3…:57`, `stage-2-m2…:20` |
| S11 | Hard-coded limits: the 5 s recover lock wait, cargo-sweep `--time 7`, redaction minimum length 6, silent `.max(1)` clamps, the Python secret-suffix list. `shared_dir` is never used. AGENTS.md says `max_agents = 2`; the config says 4. | `recover.rs:112,266`, `cleanup.rs:83`, `reviewer.rs:188`, … |
| S12 | Backup and evidence are on one disk. main is 75 commits ahead of origin; crash-sim has no remote; `state/queue` evidence is git-ignored; failed-job logs are in scratch, which is wiped after 24 hours. | — |
| S13 | crash-sim: the damping guard can never fire (stiffness is always the limit first), and PHYSICS.md contradicts it. Sub-reviewer's maths, not independently derived: the true limit is about c·h/m < 0.25, not 1.0. | `lib.rs:41-55`, `PHYSICS.md:59` |
| S14 | crash-sim `--summary` samples contact once per output step, so it can report `none` after an impact. | `main.rs:40` |
| S15 | crash-sim tests leave risky paths open: post-impact energy growth, the step limit in an actual run, the mid-run abort, restitution and friction. | crash-sim tests |
| S16 | Broadsword tests: one test kills a process tree by recorded pid with no start-time check, so pid reuse could kill an unrelated process. The disk guard is tested only with an impossible floor. Several queue safety branches are untested. | `tests/recovery.rs:147`, `tests/jobs.rs:154` |
| S17 | The Python client tests look stale: they expect 7 default tools. They sit outside the suite and were not run. | `clients/test_openai_mcp.py:201` |

## MINOR

1. Tool descriptions differ from behaviour: `queue_add` and ignored files; exit 4 "nothing was changed"; the scope of `run`'s checkpoint.
2. `PROGRESS.md` is stale in places: the reviewer-path claim, finished work listed as open, and no resume file named.
3. Unbounded object growth from checkpoint refs: 83 in root and 159 in crash-sim after one day.
4. Tests in a Broadsword worktree write to that worktree's own `scratch/`.
5. `.env` parsing misses inline `# comments`, so those values aren't redacted.
6. The verdict parser is lenient: quoted lines count, and a code fence inside a diff closes the prompt's fence.
7. Orphan kill acts only if the direct child is alive; grandchildren survive.
8. `cap_log` truncates a live log non-atomically.
9. A `test` job with no test output reports ok.
10. The queue check builds the live worktree, not a clean export, and the diff isn't compared with the claims.
11. `housekeeper.md` says to run plain `cargo sweep`.
12. The test count is inflated by `json.rs` unit tests that re-run in several binaries: about 310 distinct tests, not 355.
13. Windows only: elsewhere `process_state` always reports alive.
14. MCP: batches are refused, `mcp.log` can be forged with newlines, `resume --force` is open to any client, and a timed-out job re-runs inside the next call.

## Confirmed true by the reviewer

- The test counts above, a clean tree, no leftover worktrees, no running tasks, and `.env` ignored on main.
- All 13 queue reviews used the real CLI with `test_hook: false`, from a different model family than the coder.
- The planted bug was rejected for real.
- The two close-out features merged after Haiku and Opus reviews.
- Rollback snapshots the current state before touching files.
- Worktree removal never forces and saves orphan commits.
- The queue merges only the exact checked and reviewed commit.
- All locks are OS locks.
- The disk guard fails closed.
- Portability holds.

## Reviewer's top 5 process improvements

1. **Secrets:** stop checkpointing secrets, and capture ignored files only for `run`. Have one scrub function at every child spawn. With the owner's approval, prune the existing refs and rotate the key.
2. **One merge path:** a single path to main that runs the brief's done-check commands and shows their output to the reviewer. The coder model comes from the task record, orchestrator fixes go through the queue too, and there is a scoped `recover <id>`.
3. **Rules that are true:** make the standing rules true or cut them. Start `watch` automatically, or shrink the cleanup rule; add a test that fails on any unread setting; fix the cap shown in AGENTS.md.
4. **Less record-keeping:** generate the milestone record from `state/`, and measure orchestrator cost, not just review cost. 9 of the last 10 commits are process.
5. **Backups and parallel work:** push main and give crash-sim a remote (needs the owner's OK). Keep evidence out of scratch. Hold the build lock only while building, not for the 3-minute test run.

## Not reviewed

- **Not read in full by the main agent:** `mcp.rs`, `tools.rs` and `json.rs` (covered by sub-reviewers), `install.rs`, `config.rs`, `bench/`, `CAPABILITY_ROADMAP.md`.
- **Not executed:** the Python tests, a real kill-and-recover run outside the suite, any reviewer call, clippy.
