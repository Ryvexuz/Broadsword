# Lab log

One entry per session, newest first. At the end of each stage, three lessons for that stage go at the top.

## 2026-11-18 — Audit fixes, Stage 4 v1, work-ahead backlog, model benchmarks

**Models and roles**
- Opus 5.5 as orchestrator.
- About 20 Sonnet coder runs this half (34 all session, about 4.4M tokens).
- Reviewers in the queue: Opus (18 reviews all session, $7.17) and Haiku (22, $0.08).
- Fable 5.2 at high effort for the full audit.
- Benchmark subjects: 10 Claude configurations and 30 OpenAI models.

**What went wrong and what fixed it**
- Every serious audit code finding is fixed and merged through the queue.
- Opus caught 5 real bugs on first passes, each fixed in a fresh-coder round. See RETRO.md for the list, and for the orchestrator's own mistakes: moving main during queue runs, racing wait loops, a mislabelled coder and an ambiguous push, the last two denied by the permission classifier.
- A coder killed its own job. The per-folder recovery block contained it to that worktree, and Lazlo approved `recover --abandon`.

**Git commands used, beyond a plain commit**
- `git rebase main` (in task worktrees, before a direct merge): replays the task's commits on top of current main.
- `git merge --ff-only task/<id>` (twice: dev_metrics and the OpenAI runner, both on Lazlo's "take the file" rule after refuted Haiku FAILs): moves main to the task's commit without a merge commit.
- `git push origin main refs/tags/stage-*` and `git -C projects/<p> push <origin url> <branch>:refs/heads/<name>` (with Lazlo's explicit OK): backs up Broadsword, and crash-sim and render as branches of the private repo.
- `git -C projects/render commit .gitattributes` (outside the queue): `*.ppm binary`, so the queue's diff of golden images stays small.

**Rough cost and time**
- About 8 hours 12 minutes of pipeline window, with 24 merged tasks.
- Reviews $7.25.
- Benchmark costs are in `state/bench-results/`. OpenAI cost is not computed, because no prices were given.

**One lesson about AI development**

Throughput came from keeping a coder working ahead, not from faster coders. The coders were never the slow part. The queue's double test run and the orchestrator's own serial steps were. Measure where the waiting is before optimising anything.

## 2026-11-18 — Audit fix B1: secrets out of checkpoints

**Models and roles**
- Opus orchestrator.
- Sonnet coder: about 174k tokens.
- Opus reviewer in the queue: $0.38, PASS.

**What went wrong and what fixed it**
- The full audit found that every job checkpoint stored `.env` as a Git object.
- The fix: secret files are never captured, restored or deleted, and only `run` takes a checkpoint.
- 23 refs held `.env`, 2 more than the audit counted, because the queue still used the old binary. All 23 were deleted with Lazlo's approval.

**Git commands used, beyond a plain commit**
- `git update-ref refs/broadsword/kept/stash-20261117-cac61fd cac61fd`: gave a dropped coder stash commit a name, so the prune below could not delete that work.
- `git update-ref -d <ref>`, once for each of the 23 refs: deleted a ref. The commit it pointed to, and the `.env` blob inside, became unreachable.
- `git reflog expire --expire-unreachable=now --all`: dropped reflog entries that could still have kept those objects alive.
- `git gc --prune=now`: removed unreachable objects from the object store immediately, instead of after the usual two weeks. Afterwards, `git cat-file -e <blob>` confirmed the `.env` blob was gone.

**Rough cost and time**
- About 15 minutes of agent time.
- $0.38 of review.

**One lesson about AI development**

When you clean up after a bug, re-derive what to clean from the rule ("every ref holding a secret"), not from the count in the report. The bug kept running between the report and the fix.

## 2026-11-18 — Stage 3 close-out: two reviewer models

**Models and roles**
- Opus orchestrator.
- 2 Sonnet coders in parallel, about 73k tokens each.
- Reviewers through the queue: Haiku ($0.0026) for the damping-stability feature, Opus ($0.0875) for the `--summary` feature.

**What went wrong and what fixed it**
- Nothing failed. Both reviews passed first time and both merged; the queue rebased the second onto the first.
- Haiku's one minor (a NaN step) did not apply once checked against the code.

**Rough cost and time**
- About 2 minutes of agent wall clock and $0.09 of reviews.

**Git commands used, beyond a plain commit**
- `git tag stage-3-done`: marks the verified end of Stage 3, so it can be found or checked out later.

**One lesson about AI development**

Writing "never pipe broadsword output" into the coder prompt cost one line, and no job was interrupted this time. A guard in the tool and a line in the brief work together: the fix stops the damage, and the line stops the habit.

## 2026-11-18 — Stage 3, milestone 4: scale-up test (max_agents 3, then 4)

**Models and roles**
- Claude Opus 5.5 as orchestrator (main session): spec, task split, merges, Broadsword fixes, done check, records.
- 7 Sonnet coders, one per task, about 72k tokens each.
- Haiku as the reviewer in the merge queue: 8 reviews, about $0.002 each (one task failed once, then passed).
- 2 headless Opus reviews of the Broadsword fixes: about $0.19 (`max_agents` cap) and $0.21 (closed-stdout fix).

**What went wrong and what fixed it**
- **`max_agents` was never enforced.** The setting was in `broadsword.toml`, but no code read it, so raising it would have tested nothing. Fixed in `16c9279`: `task start` refuses with exit 3 at the cap, across all projects. Then the cap was raised to 4 in `4349051`.
- **`| head` killed Broadsword mid-job and blocked every agent on the project.** A coder ran a job piped into `head`. The closed pipe panicked the supervisor, the record stayed `running`, and new jobs were refused until `recover`. The same coder was blocked four times, the physics-doc coder's only test run was refused, and the queue's check for the CSV task exited 3 with nothing changed. The affected coders stopped and reported, and nobody ran `recover` on another agent's job. The orchestrator confirmed the worktree was clean, ran `recover --abandon`, and re-ran the blocked checks without pipes. Root fix in `b578103`: crate-wide non-panicking print macros, with a test that fails without the fix.
- **An ambiguous brief caused a false Haiku rejection.** The brief listed public API items in parentheses, and the reviewer read the list as complete, so it rejected a call to `max_speed()`. The code was right. The brief now says "anything `pub` in src/lib.rs (for example ...)", and the same commit passed on re-queue.

**Rough cost and time**
- About 7 minutes of agent wall clock for both rounds (02:30 to 02:37 UTC). Each round's first start to last merge: 2 min 27 s at cap 3, 3 min 20 s at cap 4.
- Coders: about 72k Sonnet tokens each. Reviews: about $0.002 each in Haiku, plus the two Opus reviews above.

**Git commands used, beyond a plain commit**
- `git worktree list`: lists the worktrees Git knows about, so a leftover one would show. Run on crash-sim after the scale-up: it listed only the main checkout at `9f1b5fc`, and `git branch` showed only `main`, so no worktree or task branch was left.
- Broadsword's queue ran `git rebase <main HEAD>` and `git merge --ff-only <sha>`, as explained in the milestone 3 entry. Each task's `task done` also removed its worktree and branch, through `git worktree remove` and `git branch -d` as before.

**One lesson about AI development**

At small scale, the shared-resource failure was not build contention. It was a shared failure state: one agent's crashed job blocked everyone. Scale-up exposes blast radius before it exposes throughput, so the test for the next cap should be about what one failure can stop, not only how fast agents finish.

## 2026-11-18 — Stage 3, milestone 3: reviewer step and merge queue

**Models and roles**
- Claude Opus 5.5 as orchestrator (main session): spec, merge, done check, records.
- Sonnet as coder, first pass, commit `965d52f`: about 384k tokens, 23 min.
- Sonnet as a FRESH coder for the fix round, commit `671ca93`: about 250k tokens, 14 min. The same fresh coder made the follow-up `a173b7a`, about 3k more tokens.
- Opus as reviewer of the queue code (high risk, because it merges and rebases). First pass FAIL, about 182k tokens. Second pass PASS, about 228k cumulative.
- Haiku as the reviewer of the queue in the done check. The two real reviews cost $0.0014 and $0.0017.

**What went wrong and what fixed it**
- **Opus's first pass failed on three blockers and six minor points.**
  - B1: a commit added between check and review could be merged unchecked. The reviewer reproduced it: it killed a run mid-review, committed non-compiling code, resumed, and the code landed on main.
  - B2: `.env` values with ordinary names (`DB_PASSWORD`) reached the reviewer's prompt, and variables such as `GITHUB_PAT` reached its environment.
  - B3: `BROADSWORD_REVIEWER_CLI` could point at the shipped fake reviewer silently. The reported model was never checked, and non-JSON output was parsed as a verdict.
  - Minor, all fixed: an empty `BLOCKING:` header passed; no cap on review calls; the same task could be queued twice; model names were compared as raw text; check output was stored unredacted; an ignored `.env` blocked the queue permanently.
  - The fix round fixed all of them. 12 of the new tests fail against the old code.
- **The second pass re-ran the reproductions** and the attack attempts. One minor point remained: a copy of the fake reviewer outside the root passed as real. The follow-up fixed it.
- **The done check caught the planted bug.** The planted commit passed all 12 tests, because the flag was parsed but never used. Haiku rejected it in about 11 s, with a finding that named the line. The fix passed review and merged.
- **The merge kept the rejected commit in history.** Fast-forward merge put both `7ab23fc` (planted) and `9cd162f` (fix) on crash-sim main. The tree is correct, but the history contains a commit rejected on review. Recorded in `IDEAS.md`.
- **Known limits, documented:** a secret split across two diff lines is not redacted, and a `.env` value that is also normal code is blanked everywhere. An edit made and undone during a check can leave the tip unchanged, so the check may have run on uncommitted files. Recorded in `IDEAS.md`.
- **`claude` is not on PATH on this machine.** Real reviews need `BROADSWORD_REVIEWER_CLI` set to the install path.
- **The coder's worktree again started on the Stage 0 commit,** the ninth time.

**Rough cost and time**
- About 2 h 15 min of wall time.
- Subagent tokens: coder about 384k, fresh coder about 253k, Opus reviewer about 228k (cumulative). Real reviews in the done check: $0.0031 together.

**Git commands used, beyond a plain commit**
- Broadsword's queue code now runs these:
  - `git rebase <main HEAD>`: replays the task branch onto current main.
  - `git rebase --abort`: cancels a rebase that hit a conflict, so the branch is back at its tip.
  - `git diff --name-only --diff-filter=U`: lists the files in conflict.
  - `git diff <base>...<sha>`: the change the branch adds since it split from main. This is what the reviewer reads.
  - `git merge --ff-only <sha>`: moves main to exactly that commit, with no merge commit, and fails if that is not possible.
  - `git rev-list --count`: counts the commits in a range.
  - `git rev-parse --git-path rebase-merge`: finds where Git keeps a rebase in progress, so a rebase left behind is detected.
  - The pre-rebase tip is kept under `refs/broadsword/queue/<id>/pre-rebase`, and the ref is created only if it does not exist yet.
- The orchestrator used `git merge --no-ff`, `git worktree remove` and `git branch -d`, as before.

**One lesson about AI development**

The planted bug passed every test and was caught by the cheapest reviewer, for a third of a cent, because the review saw the brief, the diff and the check output together. Tests prove what they test; review checks the change against what was asked. The expensive part was making the pipeline itself trustworthy. Opus found that it could merge an unchecked commit and leak secrets, the kind of failure that would have made every later review meaningless.

## 2026-11-17 — Between Stage 3 milestones 2 and 3: worktree builds

**Models and roles**
- Claude Opus 5.5 as orchestrator (main session): spec, merge, bootstrap, done check, the `build/CACHEDIR.TAG` fix, and checking the records.
- Haiku as Housekeeper: wrote this entry, the evidence file, `PROGRESS.md` and the `IDEAS.md` changes, about 108k tokens. This was the first time records were moved off the orchestrator. The orchestrator corrected four small gaps, all in its own brief.
- Sonnet as coder, first pass, commit `36bca91`: about 265k tokens, 15 min.
- Sonnet as a fresh coder for the fix round, commit `9a05924`: about 130k tokens. First use of the "fresh coder for fix rounds" practice, instead of resuming the first coder.
- Haiku as reviewer, over two passes. First use of the cheaper review tier.

**What went wrong and what fixed it**
- **Haiku's first pass failed on one real bug and one false positive.**
  - Real: `run broadsword --task <id> -- cargo build` left a worktree binary that `install` would accept. Fixed.
  - False: "resume skips the lock". The loop is `enumerate().skip(first_step)`, so a resume already took the lock. The fresh coder pushed back, and the orchestrator confirmed it by reading the original code. The coder still added a regression test, which fails under a mutation.
  - The minor points were real, and were fixed or documented in the fix round:
    - junctions: `clean` refuses them;
    - `current_exe`: `install` requires main's `build/debug` exe;
    - path dependencies: documented as not covered.
- **Haiku's second pass passed** after it ran the end-to-end check the coder had skipped: `run --task -- cargo build`, then `install` refused, then `build broadsword` from main, then `install` accepted.
- **The first done-check marker was wrong.** Changing `GRAVITY` did not fail any test, because the free-fall test computes its expected value from the same constant. The orchestrator chose a bad marker. The rerun used a test that always panics, in the worktree only.
- **The first build after the merge failed.** Cargo 1.100 refused `cargo clean -p` because `build/` had no `CACHEDIR.TAG`. The folder was made in Stage 0 with a `.gitkeep`, so cargo never wrote the tag. Broadsword failed safely: exit 1, nothing built, no note written, `install` refused. Fix: `build/CACHEDIR.TAG` added and un-ignored in `.gitignore`. Then the bootstrap: build from a temporary copy of the binary, write the note `.`, then `install`.
- **Two junctions were left behind** in the coder's worktree by the junction tests, which do not remove their links. Removed by hand, non-recursively. Now in `IDEAS.md`.
- **The coder's worktree again started on the Stage 0 commit,** the eighth time.

**Rough cost and time**
- About 50 min of wall time.
- Subagent tokens: coder about 265k, fresh coder about 130k, Haiku reviewer about 155k (cumulative over two passes). Opus reviews of similar changes earlier today: about 130–180k each.
- Claude plan usage, when checked earlier: 5% of the weekly all-models limit. Extra usage was on, with $35.46 spent.

**Git commands used, beyond a plain commit**
- `git merge --no-ff`: merged the reviewed branch into `main` as merge commit `4ebba07`, so the coder's commits show as one piece of work.
- `git worktree remove`: removed the coder's worktree folder after the merge.
- `git branch -d`: deleted the merged branch. It refuses unmerged work.
- Broadsword's code now runs `cargo clean -p <package> --offline`. It removes one package's build output from the shared folder, so the next build relinks it from the right source. This is not a git command.

**One lesson about AI development**

A cheaper reviewer found a real blocking bug and gave sound design advice, and also raised one false positive that the coder correctly pushed back on. Cheap review works when its findings are checked against the code before acting. And a done check in the real environment found what every test in fresh folders could not.

## 2026-11-17 — Stage 3, milestone 2: worktree per agent

**Models and roles**
- Claude Opus 5.5 as orchestrator: spec, diff read, merge, done check, records.
- Sonnet as coder, in a worktree, over three passes.
- Opus as reviewer, over two passes. Each ran stress scripts for 5–10 minutes.

**What went wrong and what fixed it**
- **Ignored files.** The coder's comment said ignored files are "regenerable by definition". `.env` is ignored, and `AGENTS.md` says that's where keys live. `task done` deleted a `.env` and an ignored notes folder without a word. Now any ignored file blocks removal, with no override.
- **The board lock.** It was held through `git worktree add`. A realistic 20k-file repo locked other agents out for 10.8 s. Creation is now three steps, with the git work done outside the lock.
- **Rebase-then-merge.** The reviewer's minor point mattered for the next milestone: the orphan check would have blocked removal for good once the merge queue rebases. Orphan commits are now saved under `refs/broadsword/kept/` instead.
- **My done-check script stopped early.** It used `set -e`, and an expected refusal (exit 3) ended it. I finished the run by hand. Lesson for scripts that check refusals: don't use `set -e`, or wrap the expected failures.
- **The fix was real work.** The done check corrected the crash sim's "about 450 rad/s" comments to 400, a leftover from Stage 1.

**Rough cost and time**
- About 1 h 15 min of wall time.
- Subagent tokens: coder about 295k, reviewer about 175k.

**Git commands used, beyond a plain commit**
- Broadsword's code now uses the commands listed in `evidence/stage-3-m2-worktree-per-agent.md`.
- Two of them, with what each does:
  - `git merge-base --is-ancestor <branch> HEAD`: is every commit on the task branch already in main?
  - `git rev-list --stdin --not --all`: which of these commits does no ref reach?
- Orchestrator, in the crash sim: `git merge --ff-only task/<id>`. This moved crash-sim main forward to the task's commit without a merge commit. A fast-forward was possible because main hadn't moved since the worktree was created.

**One lesson about AI development**

The coder's wrong assumption was written down as a comment ("regenerable by definition"). That made it easy for the reviewer to find and test. Comments that state an assumption invite exactly the adversarial check that catches it.

## 2026-11-17 — Stage 3, milestone 1: task board

**Models and roles**

- Claude Opus 5.5 as orchestrator: spec, merge, done check, records.
- Sonnet as coder, in a worktree, over two passes.
- Opus as reviewer, over two passes. The first pass ran 20 minutes of stress scripts.

**What went wrong and what fixed it**

- **The first version failed the done check under the reviewer's attack.** Two tasks claiming `src` were running at once.
  - The cause was the stale-lock takeover the coder reused from `recover`. It was Stage 1's "known limitation 7": "rare, no lost work". On the task board, that same race breaks the one guarantee the board exists for.
  - **A known limitation in one place became a blocking bug when the code was reused somewhere with a stronger requirement.**
- **The fix was a different design, not a patch:** OS file locks (`File::try_lock`), which the OS releases when a process dies. All the stale-detection code was deleted, and `recover` switched too.
- **Project names** were compared raw. On Windows `Crash` and `crash` are the same folder but didn't conflict. Fixed with one canonicalising function used everywhere.
- **A setting with a silent default came back** (`task_lock_wait_seconds`), the same mistake as Stage 1 m3. The rule is in `AGENTS.md`, but coders don't reliably apply it. Worth adding to the coder role file.
- **Shared build folder:** after merging, I updated `tools.rs`'s timestamp before rebuilding, to avoid the stale-binary hazard found in the Stage 2 close-out. The rebuild was confirmed by the tool count.
- **The worktree again started on the Stage 0 commit,** the seventh time.

**Rough cost and time**

- About 1 h 15 min of wall time.
- Subagent tokens: coder about 265k, reviewer about 180k.

**Git commands used, beyond a plain commit**

- `git merge --no-ff`, `git worktree remove` and `git branch -d`, as before.

**One lesson about AI development**

Reuse carries limitations along with code. The coder correctly reused a reviewed lock, but its documented weakness was acceptable only for `recover`. Before reusing reviewed code, re-read its known limitations against the new requirement.

## Stage 2 lessons (AI Integration, closed 2026-11-17)

1. **Name the hard cases in the spec, and review passes drop.**
   - Every Stage 2 milestone passed review on its first or second pass except milestone 3, whose secrets fix merged after FAIL, FAIL and then only an orchestrator read, with no independent PASS. Stage 1 never passed on a first pass. [Corrected 2026-11-18 after the full audit, finding S10.]
   - The difference was the spec, which listed the risky cases up front: stdout purity, argument injection, timeout and cancel cleanup, version gating, key leakage.
   - Specs got longer; review rounds got shorter. That's a good trade.
2. **Secrets leak through side channels.**
   - The redaction everyone checked held.
   - The leak was the child process's environment, and then the same key under a different variable name.
   - Only an adversarial reviewer actively trying to exfiltrate the key found both.
   - Protect a secret at the boundary where it enters a process, not where it's printed.
3. **Test a tool surface with a real model as the client.**
   - In milestone 1, the first real client followed the server's instructions into a dead end that 22 protocol tests had missed.
   - The description experiment then showed both models choosing well already, 16 of 16 runs.
   - 4 runs per condition can only catch big effects. Measuring wording needs dozens of runs and harder prompts.

## 2026-11-17 — Stage 2 close-out: description experiment

**Models and roles**

- Claude Opus 5.5 as orchestrator.
- It designed and ran the experiment itself. The variant was three description sentences on a branch that was not merged, so no coder or reviewer was used.
- Clients: `gpt-4.1-mini` and `claude-opus-5-5`, 8 runs each.

**What went wrong and what fixed it**

- **Ceiling effect.** The baseline already scored 8/8 on `check`, so the experiment could not show an improvement.
- **Stale binary from the shared build folder.** After building the variant in a worktree, rebuilding `main` printed "Finished" in 0.01 s and left the variant binary in `build/debug`. Two checkouts of one package share fingerprints in the same build folder. Updating the timestamp on `main`'s `tools.rs` forced the rebuild. This is a real hazard of one shared build folder: a stale binary with no warning. Added to `IDEAS.md`.

**Rough cost and time**

- About 20 minutes of wall time.
- Claude runs: about $0.43. OpenAI: about 30k tokens.

**Git commands used, beyond a plain commit**

- `git worktree add scratch/exp/wt -b exp/check-routing-first`: a separate checkout on a new branch for the variant, so `main` was untouched.
- `git worktree remove`: removed that checkout afterwards. The branch is kept, to rerun the variant later.
- `git tag -a stage-2-done`: marks the end of Stage 2.

**One lesson about AI development**

Measure before you fix. The tool-choice problem seen once in milestone 2 did not reproduce in 16 runs, so rewording to fix it would have been a change made on an anecdote.

## 2026-11-17 — Stage 2, milestone 3: second provider

**Models and roles**

- Claude Opus 5.5 as orchestrator: spec, merge, both real runs, records.
- Sonnet as coder, in a worktree, over three passes.
- Opus as reviewer, over two passes. It used fake keys only and never called the real API.
- Clients in the done check: `gpt-4.1-mini` through `clients/openai_mcp.py`, and Claude Opus 5.5 through headless Claude Code.

**What went wrong and what fixed it**

- **Python, not Rust.** The client is Python's standard library, because Rust's std has no TLS. `AGENTS.md` allows Python for tooling. Reason: HTTPS without crates.
- **The reviewer found a real key leak.** The MCP server inherited `OPENAI_API_KEY`. A tool that echoed the environment sent the full key back into the model's context and wrote it to Broadsword's job log in plain text.
  - The first fix dropped the variable by name. The reviewer then stored the same key as `OPENAI_KEY` and as `MYVAR`, and it leaked again.
  - The final fix also drops any variable whose value contains the key. This is milestone 4's lesson again: the reported case was a sample of a class.
- **The first real run got HTTP 401.** An older, different `OPENAI_API_KEY` is saved in Lazlo's Windows user environment, and the client reads the environment before `.env`. I cleared it for one run only and did not change Windows settings. Lazlo needs to know about it.
- **Both models made the identical single call:** `check` on crash-sim. That's with the "in one job" prompt; without that phrasing, milestone 2's reviewer run saw Claude split it into `build` + `test`.

**Rough cost and time**

- About 35 minutes of wall time.
- Subagent tokens: coder about 165k, reviewer about 130k.
- Real runs: OpenAI 3,724 tokens (a fraction of a cent for `gpt-4.1-mini`), Claude $0.061.

**Git commands used, beyond a plain commit**

- `git rm --cached` (by the coder): removed committed `.pyc` files from the index while keeping them on disk. They are now ignored.
- `git merge --no-ff`, `git worktree remove` and `git branch -d`, as before.

**One lesson about AI development**

Secrets leak through side channels, not the obvious ones. The redaction everyone checked held fine; the leak was the child process's environment, and then a different variable name. Only an adversarial reviewer actively trying to exfiltrate the key found both.

## 2026-11-17 — Stage 2, milestone 2: headless mode

**Models and roles**

- Claude Opus 5.5 as orchestrator: spec, merge, install, done check, records.
- Sonnet as coder, one pass, about 270k tokens and 15 minutes. This was the largest single coder pass so far.
- Opus as reviewer, one pass, with two real-client runs and a Python protocol driver.
- Claude Opus 5.5 as the headless client in the done check.

**What went wrong and what fixed it**

- **Second first-pass PASS in a row.** The spec named the hard parts up front: injection through arguments, cleanup on timeout and cancel, version gating, install while running, and the hook's MCP allow-list.
- **The shared build folder bit again.** The coder's worktree build overwrote `build/debug/broadsword.exe` with code that needs settings `main` didn't have yet. Until the merge, the pause hook on `main` failed open (exit 1). That was low risk with 127 GB free, and it is exactly the problem `install` now solves: the hook runs the stable `build/bin/` copy.
- **Tool choice depends on wording.** In the reviewer's run, the model used `build` then `test` rather than `check`, even when only `check` was allowed. In the done check, a prompt saying "in one job" made it pick `check` first time. Noted in `IDEAS.md` as the description experiment.

**Rough cost and time**

- About 30 minutes of wall time.
- Subagent tokens: coder about 270k, reviewer about 165k.
- Client sessions: two reviewer runs at about $0.19 each, and the done check at $0.12.

**Git commands used, beyond a plain commit**

- `git merge --no-ff`, `git worktree remove` and `git branch -d`, as before.

**One lesson about AI development**

Naming the risky edge cases in the spec, before the coder starts, has now produced two first-pass PASSes in a row after Stage 1's run of first-pass FAILs. The review still found real limits, but nothing blocking. The cost moved from review rounds into a longer spec.

## 2026-11-17 — Stage 2, milestone 1: MCP server

**Models and roles**

- Claude Opus 5.5 as orchestrator. It wrote the spec, read the minor-fix diff itself, merged and ran the done check.
- Sonnet as coder, in a worktree, over two passes.
- Opus as reviewer, one pass, with a real Claude Code client.
- Claude Opus 5.5 again as the fresh headless client in the done check.

**What went wrong and what fixed it**

- **First first-pass PASS.** The spec named the protocol edge cases up front: stdout purity, notifications, id rules, a parser depth limit, a line size limit, and no faked `tools/call`. The coder built to that list.
- **The instructions misled the client.** They said "call the `tools` tool first", and the real client tried exactly that before calls existed. After the fix, the client in the done check still said the `tools` tool "would give the full list". It treats `tools` as a discovery tool because of its name and description. This is data for the Stage 2 lesson focus.
- **The new "Build through Broadsword" rule doesn't work for worktrees yet.** `broadsword build broadsword` always builds the main checkout. The coder used plain `cargo` with the shared build folder, as the spec allowed. Noted in `IDEAS.md`.
- **`git worktree remove` failed with "Permission denied".** My shell's working directory was inside the worktree. Git still unregistered the worktree but left empty folders. Those folders, and one empty folder left over from milestone 5, were removed with `rmdir`, which only removes empty folders.
- **The worktree again started on the Stage 0 commit,** the fourth time.

**Rough cost and time**

- About 25 minutes of wall time.
- Subagent tokens: coder about 125k, reviewer about 105k.
- The done-check client session cost $0.098.

**Git commands used, beyond a plain commit**

- `git merge --no-ff` and `git branch -d`, as before.
- `git worktree prune -v`: asked Git to drop records of worktrees whose folders are gone. There were none left to prune.

**One lesson about AI development**

The first thing a real AI client did was follow the server's instructions literally, into a dead end. Testing a tool surface with a real model as the client caught what 22 protocol tests could not.

## Stage 1 lessons (Foundation, closed 2026-11-17)

1. **An independent reviewer on a strong model is the role that pays for itself.**
   - It failed every Stage 1 milestone on its first pass, 12 review passes in all.
   - Every failure was a real defect the coder's own tests had passed:
     - the pause was ignored in worktrees;
     - the sim failed silently;
     - a test override could switch off the disk guard;
     - six ways recovery could lose work;
     - descriptions an AI would misread;
     - a test that could never fail.
   - Skipping the test writer in milestones 2–5 cost nothing visible; skipping the reviewer would have shipped every one of these.
   - Plan for at least two review passes per milestone.
2. **A review finding is a sample of a class, so fix the class.**
   - When the coder patched only the reported case, the same bug came back through a neighbouring path (recovery's size cap, twice).
   - Asking for one structural guard that every write and delete passes through ended the loop.
   - When a finding is about an invariant, the fix should enforce the invariant in one place.
3. **"Passes its tests" and "is what an AI caller needs" are different bars.**
   - The crash sim printed NaN and exited 0.
   - The registry's descriptions were accurate to the author but wrong to a reader acting on them literally.
   - Done checks that run the real thing on real inputs found what unit tests didn't: the 5 GB disk fill, killing Broadsword mid-job on real repos, piping `--json` into Python.

## 2026-11-17 — Stage 1, milestone 5: tool registry

**Models and roles**

- Claude Opus 5.5 as orchestrator.
- Sonnet as coder (worktree, three passes).
- Opus as reviewer (three passes).
- No test writer.

**What went wrong and what fixed it**

- The first pass's code and dispatch were sound, but four descriptions would have misled an AI caller:
  - `run`'s exit 3 meant either "refused" or the command's own 3;
  - `cleanup` claimed it ran `cargo sweep`;
  - "changes files" was undefined;
  - and the exit-code test searched its own source for its needles, so it couldn't fail.
- The second pass still had two wrong output texts. Both were fixed on the third pass, which also added a test that a command can't fake Broadsword's verdict line.
- The coder's worktree again started on the Stage 0 commit and had to fast-forward to `main`. This is the third time; see `IDEAS.md`.
- Eleven test folders were left in `scratch/` by an interrupted test run. A fresh run cleans up, so this is not a bug. They were deleted.

**Rough cost and time**

- About 20 minutes of wall time.
- Subagent tokens: coder about 190k, reviewer about 145k (cumulative per agent).
- No dollar figure was available.

**Git commands used, beyond a plain commit**

- `git merge --no-ff`, `git worktree remove` and `git branch -d`, as in earlier milestones.
- `git tag stage-1-done`: marks the commit where Stage 1's done checks passed, as the plan's checkpoint asks.

**One lesson about AI development**

Tool descriptions are a contract with a reader who acts on them literally and can't ask what you meant. The reviewer caught them by asking "what would an AI do with exit 3?", not by checking the code.

## 2026-11-17 — Stage 1, milestone 4: recovery

**Models and roles**

- Claude Opus 5.5 as the orchestrator: spec, five review handoffs, merge, done check, records.
- Sonnet as the coder, in a Broadsword worktree, over five passes in one continuing agent.
- Opus as the reviewer, over five passes in one continuing agent.
- No test writer.

**What went wrong and what fixed it**

- This was the hardest milestone so far: five review passes, all on real ways to lose work, each reproduced by the reviewer in throwaway Git repos.
  1. A retried rollback overwrote later edits.
  2. Under `core.autocrlf=true` restored files weren't byte-exact.
  3. `.env` (ignored) was not protected.
  4. The ignored-file size cap could overwrite unsaved bytes, twice through different code paths.
  5. A file turned into a folder blocked recovery.
  6. Restoring the index dropped content staged after the kill.
- Patching each path separately failed twice for the same class of bug. What worked was changing the structure: one `Guard` that every working-tree write and delete passes through. It allows an action only on a missing path or on bytes already in this attempt's snapshot.
- The orchestrator also changed one design point: auto-recover no longer rolls back silently before an unrelated build. It refuses new jobs on that project until an explicit `recover` or `recover --abandon`.
- The done check ran on real projects: a file-changing job on `crash-sim` was rolled back, and a `check broadsword` killed mid-test was resumed.

**Rough cost and time**

- About 45 minutes of wall time. Subagent tokens: coder about 290k (cumulative over five passes in one agent), reviewer about 220k. This was the most expensive milestone. No dollar figure was available.

**Git commands used, beyond a plain commit**

These are used by Broadsword's code, explained here as AGENTS.md asks:

- `GIT_INDEX_FILE=<temp> git read-tree HEAD` / `update-index -z --index-info` / `write-tree` / `commit-tree`: build a commit of the exact working tree in a separate temporary index, so the real index, HEAD and branch are untouched.
- `git hash-object -w --no-filters`: store a file's raw bytes with no line-ending conversion.
- `git cat-file blob`: read those bytes back to restore a file exactly.
- `git update-ref <ref> <sha> ""`: create a ref only if it doesn't exist yet. Checkpoints can't overwrite each other.
- `git ls-tree -r`: list a snapshot's files and their blobs, to compare against what is on disk.
- `git read-tree <tree>` (no `-u`): reset the real index to a saved tree without touching any files.

Orchestrator: `git merge --no-ff`, `git worktree remove` and `git branch -d`, as in milestone 1.

**One lesson about AI development**

The coder fixed exactly the case the reviewer reported, each time, so the same class of bug came back through a neighbouring path. A review finding is a sample of a class, not the whole class. Asking for one function that enforces the invariant everywhere ended the loop where patching each path hadn't. The review loop was expensive but bought something real: each of the six data-loss bugs would have deleted someone's work.

## 2026-11-17 — Stage 1, milestone 3: job runner and test runner

**Models and roles**

- Claude Opus 5.5 as the orchestrator: spec, merge, done check, records.
- Sonnet as the coder, in a Broadsword worktree.
- Opus as the reviewer, over two passes.
- No test writer. The coder wrote both unit and integration tests, as in milestone 2.

**What went wrong and what fixed it**

- To test "kill the job when the disk runs low", the coder added a test-only environment variable to the real binary. It fakes the free-space reading from a file in `scratch/`. The reviewer showed that it could fake more space too, so any agent could switch off the disk guard. The fix caps every faked reading at the real free space. The lesson: a test seam must only be able to make the guard stricter.
- Same-second job logs overwrote each other, losing failure evidence. They now get `-2`, `-3` suffixes.
- Three new settings had silent defaults, against the rule that every value lives in `broadsword.toml`. They are now required keys.
- **The coder's worktree started on the Stage 0 commit,** not on `main`'s tip. The coder ran `git merge --ff-only main` inside its branch to catch up. The same thing happened in milestone 1. The worktree tool seems to branch from an older commit than `main`'s tip; worth checking before Stage 3.
- **Disk space jumped** from 23.9 GB to 128.5 GB free during the session. That was outside Broadsword, presumably Lazlo freeing space.

**Rough cost and time**

- About 15 minutes of wall time. Subagent tokens: coder about 250k over two passes, reviewer about 220k over two passes. No dollar figure was available.

**Git commands used, beyond a plain commit**

- `git merge --ff-only main`, run by the coder on its own branch: moved the branch forward to `main`'s tip without a merge commit, because the branch had no commits of its own yet.
- `git merge --no-ff`, `git worktree remove` and `git branch -d`: the same as milestone 1. A merge commit for the milestone, then removal of the merged worktree and branch.

**One lesson about AI development**

The reviewer failed all three Stage 1 milestones on their first pass, each time on a real problem the tests didn't catch:

1. The monitor ignored worktrees.
2. The sim failed silently.
3. The override could disable the guard.

Each fix was small. Budget for a second review pass as the normal case, not a failure.

## 2026-11-17 — Stage 1, milestone 2: starter crash sim

**Models and roles**

- Claude Opus 5.5 as the orchestrator. It wrote the spec, ran the done check and kept the records.
- Sonnet as the coder, working directly in the new `projects/crash-sim` repo. The repo was new and only the coder worked in it, so no worktree was needed.
- Opus as the reviewer, over two passes.
- **No test writer this time.** The spec told the coder to write physics tests (free fall against h0 − ½gt², energy never rising, bounce, settle, determinism). The reviewer checked that those tests drive the real sim. This was an experiment in skipping a role; see the lesson.

**What went wrong and what fixed it**

- The first version passed all 7 of its tests and its done check, and the reviewer still failed it. With too large a time step the sim blew up silently: it printed `inf`/`NaN` rows, reported 0% strain (`fold(0.0, f64::max)` drops NaN) and exited 0. The coder added a per-step finiteness and strain check that exits 1, and an up-front refusal of time steps past a stability limit computed from k and m.
- On the second pass the reviewer checked the stability bound by building the stiffness matrix. The true ω_max is 400 rad/s; the code's bound of 529 rad/s is safe.

**Rough cost and time**

- About 15 minutes of wall time. Subagent tokens: coder about 190k over two passes, reviewer about 180k over two passes. No dollar figure was available.

**Git commands used, beyond a plain commit**

- `git -C projects/crash-sim init -b main`: made the crash sim its own repository inside the project folder, as the plan asks, so it moves with the project. Broadsword's `.gitignore` excludes `projects/`.

**One lesson about AI development**

"Tests pass" and "the done check passes" both held for a program that would print nonsense and report success. The reviewer found it by trying inputs the coder never tried, rather than by reading the tests. Skipping the test writer cost nothing here, because the reviewer did the adversarial testing. The role that matters most is the one that tries to break the code.

## 2026-11-17 — Stage 1, milestone 1: resource monitor

**Models and roles**

- Claude Opus 5.5 as the orchestrator (main session). It wrote the task spec and claim, merged, and ran the done check.
- Sonnet as the coder, in its own worktree. Haiku as the test writer, in the same worktree. Opus as the reviewer, which cannot edit files.
- The role files in `.claude/agents/` couldn't be called by name: they were created after this session started, and a session reads them only at startup. Each subagent instead ran as a general-purpose agent on the role's model, with its role file as the brief.

**What went wrong and what fixed it**

- The reviewer failed the first version, on a bug that 28 passing tests missed. Each worktree has its own `broadsword.toml`, so a worktree agent checked its own pause flag and was never paused. The coder fixed it by following a worktree's `.git` file back to the main checkout. The reviewer then reproduced the original failure with a real `git worktree` and passed the fix.
- Still open: Cargo's `target-dir` is resolved per worktree, so worktree builds only share `build/` when `CARGO_TARGET_DIR` is set in the shell. The orchestrator set it in every subagent's brief. Noted in `IDEAS.md` for the job runner.
- The done check used a real 5 GB fill. The pause blocked the orchestrator's own Write call, which is the strongest evidence that agents pause.

**Rough cost and time**

- About 2 h of wall time, most of it waiting on subagents. Subagent token use: coder about 120k tokens over two passes, test writer about 92k, reviewer about 105k over two passes. No dollar figure was available.

**Git commands used, beyond a plain commit**

- `git merge --no-ff worktree-agent-…`: brought the reviewed branch into `main` as a merge commit, so the history shows the coder's, test writer's and fix commits as one milestone.
- `git worktree remove .claude/worktrees/agent-…`: deleted the coder's worktree folder now that its work is merged, per the disk rules.
- `git branch -d worktree-agent-…`: deleted the merged branch. `-d` refuses to delete unmerged work.

**One lesson about AI development**

A separate reviewer on a stronger model found the one bug that mattered. The coder's and test writer's tests all ran inside a single folder, so they tested the case the code was written for. The reviewer asked where agents actually run. Tests written by the side that wrote the code tend to share its blind spots.

## 2026-11-17 — Stage 0, Setup

**Models and roles**

- Claude Fable 5.2 as the orchestrator (main session). It did all of the setup itself; no subagents ran. `AGENTS.md` names Opus for this role, so this session did not match it.
- Claude Opus 5.5 in one fresh headless session, used only for the done check.

**What went wrong and what fixed it**

- The done check took three runs of a headless Claude Code session. The first produced no output for more than three minutes and was stopped by hand. The second, with `--strict-mcp-config` added to skip connectors, started and began its answer, then stalled until a 100 second limit cut it off. The third, with output written to a file instead of a pipe, finished in about 7 seconds and passed.
- The cause of the two stalls was not found. Two things changed between the runs (connectors skipped, output to a file), so it isn't known which one mattered, or whether it was a passing slowdown. Worth knowing before Stage 2, which runs builds through headless sessions.
- "TokenWise kept, task-router removed" is not done. The fresh session's startup record still lists `anthropic-skills:task-router`. It is an account-level skill, stored outside this repo, so nothing in the repo removes it. Nothing named TokenWise was found on this machine or in the plugin catalog. Both need a decision from Lazlo.

**Rough cost and time**

- About 25 minutes, 10:09 to 10:33.
- The done-check session reported $0.012. The main session's cost was not measured; no cost figure was available to it.

**Git commands used, beyond a plain commit**

- `git init -b main`: turned the folder into a Git repository so every later change has history. `-b main` names the first branch `main`; this Git version would otherwise call it `master`.
- `git add -A`: staged every new file for the first commit. `.gitignore` kept the contents of `scratch/`, `build/` and `shared/` out of it.
- `git status --short` and `git log --oneline --stat`: read-only checks of what was staged and what the commit holds.

**One lesson about AI development**

The rules file did change behavior: a fresh session with no tools gave the disk floor, the agent cap and the first task from `AGENTS.md` alone. But the same check showed what the rules file does not reach. task-router sits in the account, outside the repo, and it was still loaded. Check a setup from the fresh session's side, by reading what it loaded, not by reading the files that were meant to configure it.
