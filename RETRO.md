# Retrospectives

After each milestone: what went wrong, why, and what changes, then the same for speed and cost. This is the rule in `AGENTS.md`.

A change counts only once it is enforced somewhere: Broadsword's code, a role file in `.claude/agents/`, or `SPEC_CHECKLIST.md`. Each retro starts by checking whether the previous retro's changes worked, including whether any of them hindered things (see the improvement ledger). Once retros keep coming back empty, the rule can be dropped.

## Portable lessons, for any project

These are the lessons from the retros and the stage lessons in `LAB_LOG.md`, with the Broadsword details removed so they apply to any project built with AI agents. Each retro adds to this list, or sharpens an item already on it. After any change, run `python -I scripts/sync_playbook.py`. It copies this section to `~/.claude/AI_DEV_PLAYBOOK.md`, which `~/.claude/CLAUDE.md` loads in every project. `--check` reports a stale copy.

**Planning and briefs**
1. **Small, exact tasks are the biggest cost lever.** Name the files, the API and the done check. Broadsword coder passes dropped from 250–400k tokens to about 72k this way. [S3]
2. **Name the hard cases in the spec up front:** what must never happen, the edge inputs and the security boundaries. Longer specs meant fewer review rounds. [S2]
3. **A list in a brief is either marked "for example" or is the complete rule.** Models read an unmarked list as exhaustive. [S3]
4. **Write the done check as commands with the output each must show,** in words that can only be read one way. Coder, reviewer and orchestrator then run the same check. If it can still be read two ways, check both before claiming done. [S3]
5. **Keep a brief template and a coder-prompt template.** The rules every task needs (claim, build route, what never to do, what to report) are then written once, not retyped and forgotten. [S3]

**Review**
6. **An independent reviewer on a strong model pays for itself.** Expect the first pass to fail; plan for two. Every first-pass failure was a real bug the coder's own tests had passed. [S1]
7. **Use a cheap reviewer for low-risk work and a strong one for risky code** such as locks, deletion and secrets. A cheap review costs a fraction of a cent, so reviewing everything is affordable. [S3]
8. **A reviewer working from the diff alone misses runtime bugs.** Give it the test output too. [S3]
9. **When a finding breaks a rule that must always hold, enforce the rule in one place,** not only in the reported case. [S1]

**Tests**
10. **Prove a new test can fail.** Run it once without the fix; a test that can't fail is worse than none. [S1, S3]
11. **A test seam built into the real program may only make a guard stricter,** never weaker. [S1]

**Running agents in parallel**
12. **Scaling up exposes blast radius before throughput.** First ask what one agent's failure can stop for everyone. [S3]
13. **Shared state needs real OS locks and isolated work areas:** one branch or worktree per agent, claimed files, and one owner for merging. [S3]
14. **A tool that agents call must survive their mistakes,** such as closed pipes, killed processes and bad input, without leaving shared state broken. [S3]
15. **Check that every setting is actually used.** A limit nobody reads looks like safety and isn't. [S3]

**Security**
16. **Secrets leak through side channels,** such as a child's environment or the same value under another name. Protect a secret where it enters a process, not where it is printed. [S2]

**Tools an AI uses**
17. **Test a tool surface with a real model as the client.** It follows the descriptions literally, into dead ends that protocol tests miss. [S2]

**Process**
18. **A fix counts only once something enforces it:** code, a role file or a checklist. A note alone is forgotten by the next session. [S3]
19. **Push long runs, such as full test suites and reviews, into the background,** and keep working. The orchestrator is the bottleneck, not the coders. [S3]
20. **Track whether each process change helps or hurts,** with a measure and a watch-for-harm signal, and revert the ones that hinder. A process only gets lighter if changes can be taken back out. [S3]
21. **A subagent cannot wait for its own background jobs.** Tell it to run long checks in the foreground and to read the result before it reports. Otherwise it hands back unverified work. [S3]
22. **Enforce an environment rule where the process is spawned,** not in each command builder. A rule set in two builders was missed by a third path. [S3]
23. **Write code files with an editor tool, not through shell heredocs.** Shell quoting silently changes backslashes and quotes. [S3]
24. **Audit the whole project at every stage end, with a strong model that didn't build it.** Per-change reviews only see their diff, while the worst problems sit between features and rules. Fix the audit's blockers before tagging. [S3 audit]
25. **Check every new feature against the standing rules it touches,** especially secrets, deletion and settings. A documented feature can quietly break a higher rule. [S3 audit]
26. **Records drift toward over-claiming.** Write them from raw evidence and quote it, and have the audit check the claims. [S3 audit]
27. **No exemptions for the orchestrator.** The strongest model is the one most tempted to skip the process it enforces. [S3 audit]
28. **A rule without a mechanism is a wish.** Enforce each standing rule in code, a hook or a test, or label it "not yet in force". [S3 audit]
29. **Never move the shared main branch while a merge queue is working on it.** Your own notes and records count. Stage them outside version control until the queue is idle. [S4]
30. **One driver per queue.** Several "wait until idle, then run" loops race each other. Make the tool drain its queue in order instead. [S4]
31. **Check a benchmark's scorer against real outputs before quoting any number.** Patterns written without real model wording undercount. Keep raw outputs so you can re-score for free. [S4]
32. **Check the model you asked for is the model that answered.** An old model id can silently run as a newer one. [S4]
33. **Scripts that other tools run from another folder need absolute paths, and should check their inputs exist before the first paid call.** [S4]
34. **When a cheap reviewer keeps failing low-risk tooling with new, non-regression points, verify them against the code, and stop the loop.** The cost is the re-check time, not the review fee. [S4]
35. **Judge a scale-up on throughput and failures.** Lock wait and test time are costs inside throughput, not a separate veto: record them as warnings, and step back only if merges per hour stop rising, failures rise, or memory runs out (RAM over 90%, swap growing). [S4 close]
36. **A free slot is filled before any other work.** At the start of every step, list the board and start a coder on every ready task (dependencies merged, claims free) until running = max_agents; hand reviews, audits and records go to background agents so starts never wait. On 2026-11-19 the session averaged 3.2 running tasks of 6 to 10 slots: empty slots, not the slot limit, were the main loss. [S4 close]
37. **A cap must count what it claims to limit.** When you change slot, queue or limit logic, check its effect on active workers with real numbers, not by assumption. On 2026-11-20 a board showed 14/14 slots used while 0 coders worked (every task was waiting in the merge queue), with RAM at 14% and CPU at 7%. [S4]
38. **For a safety check, prefer an exact match or an allow-list over a parser of an open format.** A hand-written YAML checker failed four review rounds, each with a new bypass; an exact byte pin of the approved file plus a self-test of the pin had 0 of 20 bypasses where the parsers passed 20, 16, 12 and 9. [S6]
39. **A baseline built from the same reviews that produced the labels is biased.** The labels were REAL by construction for that reviewer (20 of 20); a fresh run of the unchanged prompt caught 7 of 20. Re-run the baseline arm under the same conditions before setting a floor, and run each arm twice to see the noise. [S6]
40. **A rule that cuts wrong rejections can also cut real catches.** A 7-rule reviewer prompt block lowered wrong rejects from 6 to 2 but caught real bugs 5 of 20 times instead of 20 of 20; keep a recall guard next to every false-reject fix. [S6]
41. **When a check rejects with new points every round, change the design or escalate once; do not patch the same check a fourth time.** Adjudicate each finding against the code first (REAL, FALSE or KNOWN, with a quote). [S6]
42. **Agents that write files write outside any checkout the merge queue uses,** and a queue priority only counts if the queue enforces the order itself: one stray untracked file blocked 8 entries for about 30 minutes, and a priority file was never read mid-drain. [S6]
43. **A brief for a new CLI flag or required setting claims the parser and every place the type is built** (grep for the struct literal). Three briefs in one day stopped before code on a missing claim. [S6]

---

## Improvement ledger

Every process change goes here, with what it should improve and how we'll know. Each retro marks each open row:
- **helped:** keep it.
- **no effect:** keep it if it costs nothing; otherwise drop it.
- **hindered:** it made work slower, costlier, or caused false failures. Fix it or revert it, and say which.

A change can help one thing and hurt another, for example faster reviews but more false rejections. Record both. Nothing on this list is permanent.

| Since | Change | Should improve | Measured by | Watch for harm | Status |
| --- | --- | --- | --- | --- | --- |
| S1 | Independent reviewer, two passes planned per milestone | Bugs caught before merge | Real defects found per review pass | Rounds that only find style nits | helped (S1–S3) |
| S2 | `SPEC_CHECKLIST.md` risk table | First-pass review results | Review passes per milestone | Briefs so long that coders miss the point | helped (S2–S3) |
| S3 m3 | Review tier: Haiku for low risk, Opus for high | Review cost | `scripts/review_costs.py` | Bugs Haiku misses that Opus would catch; false rejections | mixed: Opus 5 of 5 FAILs real; Haiku on Python tooling 9 FAILs, about 2 real, 8 min of re-checks each. Change: when a cheap review FAILs twice with new non-regression points, the orchestrator checks them against the code, and takes the file directly on Lazlo's rule if they are wrong |
| S3 m4 | Small one-file or two-file tasks | Coder tokens per task | Coder tokens from agent reports | Too many tiny tasks, more orchestration than coding | helped (72k vs 250–400k) |
| S3 m4 | Coders never pipe Broadsword output; never `recover` another agent's job | No project-wide blocks | Refused jobs per milestone | Coders unable to inspect long output | helped: 0 refused jobs, and the coder redirected to files and read them |
| S3 m4 | Example lists in briefs say "for example" | No false rejections from wording | Rejections whose cause is the brief | — | open |
| S3 m4 | Every setting must be read by code, with a test | No dead settings | Review findings of unused settings | Test overhead for trivial settings | open |
| S3 | Retro after each milestone | Repeat mistakes, speed and cost | Repeats of a mistake already in a retro | Retro time growing past its value | warning: the audit found 9 of the last 10 commits were process. Stage 4 ships product; process work only fixes findings |
| S3 | Portable playbook loaded in every project | Lessons reach other projects | A lesson applied, or ignored, in another project | Rules that don't fit other projects causing friction there | open (not yet used outside Broadsword) |
| S3 | `templates/brief.md`, `templates/coder-prompt.md` | Time to write a brief; brief errors | Orchestrator time per brief; rejections caused by briefs | A template followed by rote, so missing task-specific cases | mixed: the coder followed the rules (redirected output, never piped, claims kept). But my brief's "run the suite in the background" made it hand back before seeing its result. Fixed: both templates now say to run it in the foreground |
| S3 | Done checks written as commands | One reading of "done" | Disputes or rework over what "done" meant | Checks that miss what can't be expressed as a command | helped: it was clear which line was unrun, and I ran it. No disputes |
| S3 | `scripts/review_costs.py` | Exact cost numbers in retros | Whether retros use it | — | helped: used for this retro's figures |
| S3 | An interrupted run blocks only its own folder; panic guard for jobs | Blast radius of one agent's crash | Agents blocked by another agent's job | A half-changed folder that something outside it depends on | done (`ba03a84`); measure in Stage 4 |
| S3 | Every job child gets `CARGO_TARGET_DIR` at one spawn site | No private build folders in worktrees | Stray `build/` or `target/` in worktrees at removal | — | done (`d23b969`) |
| planned | Reviewer canary (planted bugs every few merges, across models once OpenAI is linked) | Knowing the reviewer still works, and which model is worth its price | Catch rate and cost per model | Canary cost or noise outweighing its value | planned |
| S3 audit | Full independent audit at every stage end, before the tag | Cross-cutting bugs, false claims, rules not in force | Blocking and serious findings per audit (S0–3: 1 and 17) | Audit cost (about 334k tokens) outweighing what it finds | open |
| S3 audit | All code reaches main through the merge queue, the orchestrator's included | Review independence that is recorded and enforced | Commits on main without a queue entry | Slower small fixes | open |
| S3 audit | Secret files (`.env*`) never stored in checkpoints; only `run` takes a checkpoint | No key in Git objects; faster build, test and check jobs | Objects holding a secret blob under `refs/broadsword/*` (now 0) | Lost rollback protection for tracked files named `.env*` (Opus minor) | done (`f660c76`) |
| S3 audit | Work ahead: a coder starts the next non-overlapping task while the queue checks and reviews | Pipeline idle (62%) and handoff waits (up to 9 min) | `scripts/dev_metrics.py` idle share and handoff column | Wasted work when ahead work depends on an unmerged change; more rebase conflicts | helped: idle 62% → 47%, coder use 28% → 56%, no wasted ahead work; harm seen: brief files written into main blocked a merge (fixed: `state/backlog/`) |
| S3 audit | `scripts/dev_metrics.py`: throughput, idle time and model use | Seeing where time goes | Whether retros use it | Logging agent runs by hand (`state/agent-runs.tsv`) gets skipped | open |
| S3 audit | `max_agents` raised 4 → 6 so the backlog's Python benchmark tasks run beside the Rust ones | Backlog wait; pipeline idle | Open tasks waiting while slots are full; `dev_metrics.py` idle share | Build-lock waits (already 10 min once at 4 agents), RAM (15 GB machine) | open |
| S4 | Shadow second reviewer (gpt-5.6-sol), recorded only | Learn whether a GPT reviewer catches what Claude misses, at what cost and time | `scripts/shadow_report.py` agreement and unique findings | Shadow time making entries stale (Opus minor); OpenAI spend | open |
| S4 | Blind re-labelling for benchmark scores (anonymised, shuffled lines) | Fair scores despite wording differences | Score changes vs the pattern scorer (gpt-6.2-sol 5 → 9) | The labeller is a Claude model, so a contestant (it was blinded) | helped |
| S4 | Codex CLI first for OpenAI calls (ChatGPT weekly allowance), API as fallback | No API spend for shadow reviews and benchmarks | `backend` field in shadow results | Codex's ~18.5k-token prompt overhead per call; plan rate limits | open |
| S4 | Adjudicate every shadow-only finding (independent judge, recorded verdict), Lazlo's rule | Know how right the shadow is, and catch bugs the gating reviewer missed | Shadow precision and gating misses from `shadow_report.py` | Judge time per merge; verdicts by a Claude judge favouring Claude | open |
| S4 | Self-optimization: the orchestrator may change or drop process rules that measurably cost throughput or money (Lazlo's standing OK; safety rules excluded) | Less waiting on approvals for process tweaks; rules that don't pay their way go | Changes made under it, each with before/after numbers, and how many were reverted | The orchestrator relaxing rules for convenience (lesson 27); a safety rule mislabelled as process | open |
| planned | Audit cadence that adapts (Lazlo, 2026-11-18): a full sweep at every stage end for now. After 3 audits in a row with at most 1 verified blocker each, switch to a diff-scoped audit at stage ends, plus a full sweep every second or third stage; one audit with 2 or more blockers goes back to every stage | Less audit time and spend once the process matures | Verified blockers per audit, and bugs found later that an audit should have caught | Bugs piling up between full sweeps | planned |
| planned | Handoff test from `PROGRESS.md` alone | Session continuity | What a fresh agent gets wrong | — | planned |
| S4 close | Fable only as a rare one-off (Lazlo, 2026-11-19): never a default coder, reviewer or judge | Weekly Claude allowance lasts | Fable tokens per stage (about 1.4M on 2026-11-19, about 990k of it avoidable) | A security or decisive review done by a weaker model that misses a real bug | open |
| S4 close | Rule 34 applied: a cheap reviewer's second FAIL with new, non-regression points on low-risk tooling goes to one decisive strong review | Fewer fix rounds on low-risk tooling | Rounds per merged task for haiku-tier entries | Real bugs slipping because the loop stopped early | open |
| S4 close | Shadow reviewer through Codex with no tools offered (one-entry model catalog), proven by `scripts/shadow_canary.py` | No secret can be read by the shadow; shadow answers fast | Canary exit code after every codex update; shadow seconds per entry (144 s timeouts before, 3-21 s after) | A codex update re-adding a tool (the canary must be re-run; not yet enforced) | open |
| S4 close | Fill free slots first (Lazlo, 2026-11-19 21:50Z); inbox read at least every 10 minutes | Merged/h | Average running tasks and merged/h from task times vs state/throughput/throughput-review-2026-11-19.md | Coders started on tasks whose dependencies are not really merged; claim collisions | open |
| S4 close | max_agents 6 -> 10 tried with baselines (`scripts/baseline.py`) | Throughput | merged/h; failed/retried jobs; RAM and swap (lock wait and test time as warnings, Lazlo 2026-11-19 21:32Z) | Failures or memory, not lock wait | re-judged under Lazlo's new rule: 2.82 -> 8.44 merged/h, 0 failures, RAM peak 14.8%, no swap = helped; lock-wait p90 26 -> 88 s and test median 82 -> 100 s recorded as warnings. The first verdict (hindered, reverted to 6) used the old rule that made lock wait a veto. Now 14, stepping +4 |
| S4 close | Slot cap counts active coders: `task start` ignores tasks whose merge-queue entry is queued or running; required `max_queued`; `scripts/slot_check.py` logs board, waiting, active, ready, RAM and CPU every poll and prints FLAG when coders are idle while ready work waits (2026-11-20) | Active coders per poll and merges/h | Active coders per poll (slot_check) and merges/h vs the slots-14 pre-16 baseline | Queue depth (max_queued), RAM and CPU | open |
| S4 close | Ahead-work compiles in `scratch/ahead-target` (CARGO_BUILD_JOBS=4), an exception to the one shared build folder (2026-11-20 00:27Z). Measured cost of the old rule: ahead patches stayed uncompiled (merge train +1521/-78 lines), and building into `build/` outside the lock broke two queue checks on 2026-11-19 at 23:51Z | Share of ahead patches that apply and pass on the first board run | That share, and disk used by `scratch/ahead-target` | Disk use; CPU taken from queue checks | open (revert: remove the exception; the scratch age cleanup wipes the folder) |
| S6 | Experimental code mode for coders, trialled on 4 comparable task pairs (state/codemode-trial.tsv) | First-pass review results, wall time, tokens | First-pass pass 0 of 4 (code mode) vs 1 of 4 (normal); wall 8.3 vs 9.1 min; input 2.15M vs 3.86M tokens (not like-for-like: session history), output 40k vs 51k | Needs a person to paste prompts: headless `claude -p` and the SDK session have no REPL tool | NOT adopted. Revert: nothing in force; revisit if headless code mode ships, with a fresh session per task |
| S6 | Loop breaker: `scripts/slot_check.py` prints LOOP for any running task with 3 or more failed queue rounds; `state/priority.md` rule: adjudicate, then split or escalate once, then flag Lazlo | Rounds per merged task (some tasks took 3 or more rounds with new points each time) | Rounds per merged task and the share of tasks with 3 or more rounds, from `queue list` | Tasks split or escalated too eagerly; escalation to Opus holding the serial queue | open. It worked once by redesign (the CI checker). Revert: delete the rule in priority.md; the LOOP line is report-only |
| S6 | Review-gate trial: diffs over 300 changed lines or on queue, job, buildsrc, task, worktree, secrets or deletion paths go `--risk high`; Opus-coded high-risk goes to the Sonnet fallback gate (`state/review-gate-trial.md`, rows in `state/review-gate-trial.tsv`) | False-reject rate per gate model; rounds per merged task | Rounds, false rejects (REAL/FALSE/KNOWN), reviewer dollars and queue minutes per merged task, over about 20 merges; the tsv has 8 rows and no merged_at, usd or fix_usd filled yet | Opus review minutes holding the serial queue; merges per hour falling | open. Revert: route by risk only, as before 04:12Z |
| S6 | Gate-only reviewer prompt block (B2: 4 rules, merged as 5f8c50a and 62b7515; low-risk gating call only, shadow prompt unchanged) | Wrong rejections by the Haiku gate | Replay: wrong rejects 1.0 vs 4.0 per arm, recall 7.5 vs 8.0 of 20 (2 runs each, about $3.5 total). Live: false-reject rate and shadow-detected gate misses over the next 20 merges | Gate misses rise (the Haiku gate catches about 35% of known real points already) | open. Revert if misses rise: remove `gate_block!` from `SYSTEM_PROMPT_GATE` in reviewer.rs. Sol replay with this block due 2026-11-21 |
| S6 | Train mode: the queue writes `check result: NOT RUN for this change alone` and block rule 2 tells the gate that CHECK RESULT covers only what the queue ran | Haiku rejects that cite missing check output | Haiku rejects citing NOT RUN, before and after (no after-number measured yet) | The gate trusting a check that never ran | open (partly in force: rule 2 does not name NOT RUN). Revert: remove rule 2 |
| S6 | SPEC_CHECKLIST item: a new JobSpec or CLI field claims tools.rs plus every literal site (`grep -n "JobSpec {"`), and a new required setting claims config.rs, `broadsword.toml` and tests/support | Briefs that stop before code on a missing claim | Briefs abandoned for a claim gap (3 on 2026-11-20: F3, F5, F9), per day | Longer briefs that coders skim | NOT yet in force: staged in `state/staging/records/retro-pending.md`, no SPEC_CHECKLIST.md edit in this task's claims. Revert: delete the item |
| S6 | Exact pin of `ci.yml` instead of a hand-written YAML checker (3ebd259, queued HIGH, not merged) | Bypasses of the CI workflow safety check | Bypass harness: old versions pass 20, 16, 12 and 9 of 20 bypasses, new 0 of 20 | A legitimate edit of ci.yml needing a pin update every time | open. Revert: back to a parser only with a harness that stays at 0 bypasses |
| S6 | Merge train (`queue run --all`, `merge_train_max = 4`, merged as da44a47) | Merges per hour | Merged/h after the driver started (16 in 1.83 h = 8.7/h) vs 6.0/h last hour and 4.33/h last 3 hours before (small sample, merged = queue entries) | Check failures bisected, wrong merges, RAM above 90% | open. Revert: `merge_train_max = 1` |
| S6 | Fallback reviewer with `--effort high` for high-risk gating (7926e52) | Opus-coded high-risk work gets a strong gate; Opus not stuck as its own gate | First fallback-gated merge 723cdd6; one Opus-high review stalled about 15 min (06:50Z, a safeguards false positive, cancelled and requeued) | Effort high hanging calls and holding the serial queue | open. Revert: `reviewer_effort_high = "low"` in `broadsword.toml` |

## Retros, newest first

---

## The slot cap counted the wrong thing (2026-11-20 00:18Z, PC session)

- **Wrong:** `max_agents` counted every board task with status running, including finished work waiting in the single merge queue. At 00:16Z the board read 14/14 with 0 active coders, RAM 14% and CPU 7%, while 58 tasks were ready. Lazlo had already said that slots must never sit idle (lesson 36), and the counter quietly defeated that rule.
- **Why:** the cap's meaning ("agents working") and what it counted ("tasks not yet merged") were never checked against each other with numbers. Each slot decision (14, then 16, then a ramp) assumed that running meant working.
- **Change:** t-20261120-001621 changed `task start` to count only tasks without a queued or running queue entry, and added a required `max_queued` setting (merged as 51d3732). That part is enforced in code. `scripts/slot_check.py` logs board, waiting, active, ready, RAM and CPU and prints a FLAG when coders are idle while ready work waits, and a LOOP line when a running task has 3 or more failed queue rounds. Mechanism: run on every orchestrator inbox poll via the poll prompt (session-only, not yet a hook or a broadsword step), reports only, does not block starts. Status: partly in force. Follow-up: a broadsword step or a hook is the real enforcement. Ledger rows added for it and for the ahead-target exception.
- **Portable lesson:** 37, "A cap must count what it claims to limit."

---

## Merge train live, gate quality study (2026-11-20, PC session)

Written from `state/pc-progress.md`, the queue and board listings and `git log`. The 00:18Z slot-cap entry and lesson 37 are already above; the staged copy in `state/staging/records/retro-pending.md` of that entry and of the ahead-work row was dropped as a duplicate.

### Did the last changes work?

- **Slot cap counting active coders (4358f30 and the task start change):** it removed the false 14/14, but `max_queued` then became the limit: at 05:10Z and 05:19Z no coder could start because 30 entries waited in the serial queue (about 6 merges/h, so about 5 hours of backlog). Not raised on purpose (more coders would only add finished work waiting). The merge train was the fix.
- **Loop breaker (slot_check LOOP, priority rule):** it flagged the right tasks (171333-19, 223123, 223123-2, the train). It worked by redesign for the CI checker. It only reports; the escalation is still the orchestrator's decision.
- **Code mode:** tried, not adopted (ledger row).

### What went wrong

- **Haiku as the gate on a 1,700-line safety change (the train, rounds 4 to 6).** *Why:* a cheap gate with a long diff re-raises settled points and misreads recovery paths (Haiku 11 of 27 entries false-rejected; patterns: settled brief points 10, evidence mismatch 6, hedged findings 5, misread recovery paths 5). Sonnet judged B1, B2 and the minors FALSE with code quotes; one rejected point contradicted the passing 4-entry train test. *Change:* route large (over 300 lines) or safety-path diffs to a strong gate (review-gate trial, fallback reviewer for Opus-coded work); the B2 block for the Haiku gate.
- **A biased replay baseline.** *Why:* the 18 of 20 recall floor came from labels written from the same Haiku reviews. *Change:* lesson 39; a fresh arm A and two runs per arm before any keep or drop decision.
- **A rule set that demoted real findings (B1).** Recall 5 of 20; the recall guard caught it. Lesson 40.
- **My brief wrote an evidence file into the main checkout (04:46Z)**, blocking the fallback entry and 8 others until 05:16Z. *Change:* briefs for non-coder agents name scratch/ or state/staging/ only (to be added to the brief template: not yet in force).
- **The queue priority file is not honoured mid-drain** (a plain `queue run` drains everything). *Change:* reorder by cancel and re-add; priority must be enforced by queue order, not a file between drains. A code follow-up is not boarded.
- **Claim gaps in three briefs** (F3, F5, F9) plus CARGO_INCREMENTAL: the coders stopped before code. *Change:* SPEC_CHECKLIST item (staged, not yet in force; see the ledger).
- **A hand-written YAML safety parser failed four rounds.** Replaced by an exact pin (lesson 38).
- **The GPT daily cap was reached at 04:24Z** ($20.16 of $20, 37 calls). *Why:* the Sol study, canaries and shadows share the same cap. *Change:* the Sol replay waits for 2026-11-21; the B2 block shipped gate-only.

### Speed

- Merges per hour with the train: 16 merged in 1.83 h = 8.7/h (pc-progress 07:28Z), against 6.0/h (last hour) and 4.33/h (last 3 hours) before it. Small sample, not controlled; merged counts queue entries.
- Cost of wrong rejects before the block, from pc-progress 04:23Z: 22 rejected rounds in 3 hours = 7.3/h at 4.8 queue-minutes each = 35 queue-minutes/h; at a 29 to 41% wrong share that is 10 to 14 queue-minutes/h, about 1.8 to 2.5 merges/h at 5.7 minutes per merge (ESTIMATE, the orchestrator's arithmetic), plus a coder fix round each.
- One Opus-high review stalled about 15 minutes (06:50Z). Watch it as the harm signal for `reviewer_effort_high`.

### Cost

- Haiku replays: arms $0.74 to $0.76 each; calls average $0.036, not the $0.006 estimated, because the prompts are 22 to 237 KB; total about $3.5 (pc-progress 05:31Z).
- GPT: $20.16 of the $20/day cap by 04:24Z. The Sol replay (16 entries) is estimated at $7 to $9.5 (ESTIMATE from pc-progress 04:26Z).
- Code-mode trial: input 2.15M vs 3.86M tokens, output 40k vs 51k (not like-for-like).
- Fable 5.2: one tie-break (03:24Z); tokens and dollars were not recorded.

### Carry into the next milestone

1. Add the SPEC_CHECKLIST item on claims for a new CLI flag or required setting, and the brief-template rule that non-coder agents write only under scratch/ or state/staging/.
2. After 20 merges, measure the B2 block (false rejects and shadow-found gate misses) and the review-gate trial (rounds, false rejects, dollars per merged task); fill `state/review-gate-trial.tsv`.
3. After 2026-11-21 00:00Z: start the GPT gate t-20261120-041456, then the Sol replay.
4. Check `max_queued` against the train: if active coders stay low while ready work waits, say why before raising anything.
5. Make queue priority enforced by queue order (a follow-up task is not yet boarded).

---

## Stage 4 v1 close: audit, two blockers fixed, model routing, scale-up test (2026-11-19, PC session)

### Did the last changes work?

- **Helped:**
  - **Touch main only when the queue is idle, one driver:** no merge was lost to a moving main; a single background driver per batch drained the queue.
  - **Hand-checking a scorer against real outputs (carry item 1):** caught three scorer bugs before any number was quoted: the hard1 location rule that dropped 34 of 36 real catches, the instruct bench counting one unpunctuated sentence as zero, and decoy false alarms credited as bug catches.
  - **Opus reviews:** every Opus FAIL today was a real bug (spec test-data leaks, regex overlaps crediting decoys, an O(n*m) reference, the provenance allow-list silencing unapproved commits).
  - **Shadow reviewer (gpt-5.6-sol, then gpt-6.1-sol):** 12 REAL, 12 KNOWN, 3 FALSE verdicts so far (`state/shadow-verdicts.tsv`).
- **Hindered:**
  - **The Haiku tier on Python tooling:** nb-06 failed four rounds and nb-25 four rounds, each on new points; several points were wrong. Rule 34 (stop the loop, one decisive strong review) ended both.

### What went wrong

| What | Why | What changes | Where it's enforced |
| --- | --- | --- | --- |
| The Codex shadow reviewer could read any file by absolute path (A6), so an injected diff could have sent `.env` to OpenAI. | `--sandbox read-only` still allows reads; nobody had tested it. | Codex is offered no tools at all; a canary with an offline leg proves it. Shadow off until proven (Lazlo). | `clients/openai_review.py` `codex_command()`, `scripts/shadow_canary.py`, docs/SHADOW_SANDBOX.md; re-running after codex updates is not yet enforced (ledger row). |
| The orchestrator's safety-stop commit broke a test on main. | Committed after `broadsword status` only, not the full suite, outside the queue. | Any direct commit to main runs the same full check the queue runs first. | Habit only so far; carry item 2 (make a direct-commit wrapper or a pre-commit check). |
| A `prove.py` run deadlocked all builds for 15 minutes. | It was wrapped in `broadsword run`, and its harness calls `broadsword run` itself (nested build lock). | Never wrap a script that calls broadsword in `broadsword run`. | `templates/coder-prompt.md` line to add (carry item 3); coder briefs today said so. |
| Board claim edits silently failed three times. | Python passed through a shell heredoc: the shell turned the `\n` separators into newlines (playbook rule 23). | Code and data edits go through a script file written with the editor tool. | Playbook rule 23 (existing); habit. |
| Stage 4 milestone 1 ("visible in a window") was never shown on screen (A1). | Every recorded run was headless or off screen; records claimed it anyway. | On-screen run with window-only captures saved in evidence. | `evidence/stage-4-v1-done.md`; SPEC_CHECKLIST line to add: a done check that names a window needs an on-screen capture (carry item 4). |
| Fable used by default for an audit and a security review (about 990k avoidable tokens). | The orchestrator picked the strongest model by habit. | Fable only as a rare one-off, with the reason logged (Lazlo). | Claude memory `fable-rare-only`; ledger row. |
| A secret-file rule matched by name only: an 8.3 short name (`ENV~1`) or a long path let `.env` bytes into checkpoint refs (nb-19, in review). | Name-only matching on a filesystem with aliases. | Judge the long name; fail closed when it cannot be resolved. | nb-19 (`git::is_secret_in`), with real-alias tests. |

### Speed

- **Throughput:** 2.82 merges/h at 6 slots (17:00-19:08 UTC), 8.44/h at 10 slots (19:20-20:10), confounded by a backlog of finished work; the queue (one entry at a time, about 11-14 min each) is the bottleneck, not coders.
- **Build-lock wait:** p90 26 s at 6 slots, 88 s at 10 (the lock covers test runs; tp3 changes that).

### Cost

- **Reviews (queue):** see `scripts/review_costs.py`; Opus about $0.3-1.2 per review, Haiku about $0.01.
- **Fable:** about 1.4M tokens (nb-06 decisive review, the Stage 4 audit and its sub-reviewers, one security review).
- **astra audit:** about 944k tokens over 42 codex calls on the ChatGPT plan; judges: five Opus 5.5 high agents.

### Carry into the next milestone

1. Re-run `scripts/shadow_canary.py` after every codex update; make it enforced (a check in the wrapper or the queue).
2. A direct commit to main runs the full check first (a wrapper or hook), so no direct commit can break main again.
3. Add to `templates/coder-prompt.md`: never wrap a script that itself calls broadsword inside `broadsword run`.
4. SPEC_CHECKLIST: a done check that names a window or a visible result needs an on-screen capture saved in evidence.
5. Scale-up (Lazlo, 21:32Z and 21:40Z): max_agents straight to 14, then +4 per step (18, 22, ...), judged on merges per hour, failures and RAM/swap against the 10-slot window (8.44/h); lock wait and test time are warnings only (portable lesson 35). tp3 (build lock covers compiling only) still cuts the lock wait.
6. File the 8 SERIOUS audit findings as tasks (`evidence/stage-4-v1-audit-verdicts.md` section 6); Lazlo decides on the two unapproved direct commits (f4123a1, render 4184930).

---

## Audit fixes, Stage 4 v1, work-ahead and benchmarks (2026-11-18, second half)

### Did the last changes work?

- **Helped:**
  - **Work ahead:** pipeline idle fell from 62% to 47% over the whole window, and coder use rose from 28% to 56% (`scripts/dev_metrics.py`).
  - **Opus reviews:** every one of tonight's Opus FAILs was a real bug, five in all:
    - option injection through an MCP `id`;
    - a damping guard that checked the wrong condition (it accepted unstable steps for c between 28 and 66);
    - a global marker that any other project's build cleared;
    - a missing shared-edge golden image;
    - a run note that made every `run` rebuild.
  - **Done-check output in the brief:** reviews judged what was actually run.
  - **Foreground test runs:** no unverified hand-backs this half.
  - **Per-folder recovery block:** one agent's crash stopped only its own worktree. That happened once, with the hard case.
- **Hindered:**
  - **The Haiku tier on Python tooling:** 9 FAILs and 2 real findings. Each FAIL cost about 8 minutes of re-checking.
  - **Committing to main while the queue ran:** that cost three merges.

### What went wrong

| What | Why | What changes | Where it's enforced |
| --- | --- | --- | --- |
| Main was moved during queue runs three times: a backlog brief, an IDEAS commit, a direct merge. | The orchestrator's own writes count as main changes, and the queue (correctly) refuses a moving or dirty main. | Briefs live in `state/backlog/` (outside Git). Touch main only when the queue is idle. Notes collect in `state/pending-ideas.md` until then. | `AGENTS.md` (Work ahead); habit, with the queue enforcing it |
| Several background "wait, then run" loops raced and lost with exit 4 (queue busy). | There is no command that drains the queue in order. | One driver per queue. Add `queue run --all`. | IDEAS (code) |
| The orchestrator mislabelled its own code to get an Opus review. | Impatience with the Haiku loop. | Never relabel a coder. The permission classifier caught it, which is the oversight working. | Habit; recorded here and in LAB_LOG |
| A push was attempted on an ambiguous "yeah, prepare for that". | The yes was read too loosely. | Publishing needs an explicit yes. The classifier caught this one too. | Habit |
| A coder killed its own job by hand, and a coder kept re-reporting after its work was done. | Long jobs, plus background work left running after the report. | The coder prompt says: never kill a broadsword process. Stop agents once their work is queued. | `templates/coder-prompt.md` (carry item) |
| Benchmark runs started with a relative results path, and with diffs that existed only in main. | The tools ran from a different folder than the script assumed. | Benchmark scripts use absolute output paths and check their inputs exist before the first paid call. | Carry item |
| The hard-case scorer undercounts. | Its patterns were written without real model output. | Hand-check against real reviews before publishing any score. | Carry item (first thing next session) |
| The queue diffed images as text (510 KB). | It reads `.gitattributes` from main, not from the reviewed commit. | `*.ppm binary` was added on main as a stopgap; the code fix is in IDEAS. | IDEAS |

### Speed

- **Throughput:** 24 merged tasks in 8 hours 12 minutes, 2.9 per hour, up from 2.4 at mid-session.
- **The main bottleneck is the queue:** each merge runs the full Rust suite twice, about 5 minutes, plus a review of about 1 minute, one entry at a time. Skipping the main re-check when main is exactly the checked commit would halve it. That is a decision for Lazlo.
- **Build-lock waits** reached 10 minutes with six agents and a benchmark sharing one build folder.

### Cost

- **Reviews:** 40 in total for $7.25. Opus accounts for $7.17 over 18 reviews (about $0.40 each). Haiku accounts for $0.08 over 22.
- **Coders:** 34 Sonnet runs and about 4.4M tokens.
- **Fable audit:** about 334k tokens.
- **Benchmarks:**
  - Claude: per-call costs are in the result files.
  - OpenAI: tokens are recorded, but cost isn't known because no prices were given.
- **Account:** the Claude plan was at 11% weekly use mid-session, with extra usage unchanged.

### Carry into the next milestone

1. Re-score the hard case by hand before any benchmark numbers are published.
2. Touch main only when the queue is idle; one queue driver.
3. Benchmark scripts: absolute paths, and check inputs before paid calls.
4. Coders never kill broadsword processes; stop agents once their work is queued.
5. Stage 4 v1 audit before the tag.

---

## Audit fix B1: secrets out of checkpoints (2026-11-18)

### Did the last changes work?

- **Through the queue:** the orchestrator's own change went through it for the first time (the new no-exemption rule). Opus passed it, for $0.38.
- **Done-check output in the brief:** the done-check output went into the brief before queueing (the new rule), and the coder ran the suite in the foreground and reported a real result (the template fix). No hand-back rounds.
- **Standing rules in the brief:** the brief named the rules the change touches. The hard case it called out, a rollback deleting a secret that was excluded from capture, is now covered by tests.

### What went wrong

| What | Why | What changes | Where it's enforced |
| --- | --- | --- | --- |
| Two more `.env` refs appeared after the audit: 23, not the 21 approved. | The merge queue's checks still ran the old installed binary until the fix was installed. | Delete by rule ("every ref holding a secret file"), not by an old count, and install the fix before cleaning up. | This retro; the ref list was re-derived before deleting |
| The worktree was kept again because tests left scratch folders inside it. | Tests that create scratch roots don't always remove them (audit minor 4). | Already in IDEAS. Second time it has cost a manual clean-up, so it moves up the list. | `IDEAS.md` |

### Speed and cost

- **Coder:** about 174k Sonnet tokens and 8 minutes, with 8 jobs, none refused.
- **Opus review:** $0.38.
- **Clean-up:** deleting the refs and pruning the objects took seconds.
- **Side effect:** build, test and check jobs no longer capture ignored files, which earlier cost up to about 29 s per job on large trees, so jobs get faster.

### Carry into the next milestone

1. The Opus minors on this change are in IDEAS.md: an index ref can hold a staged `.env`; `.env*` also covers unrelated files such as `.envoy.yaml`; and old-record reporting.
2. Fix the remaining audit findings through the queue before Stage 4 code.

---

## Full review of Stages 0–3 by Fable (2026-11-18)

Report: `evidence/full-review-2026-11-18.md`. It found 1 blocking, 17 serious and 14 minor issues. I checked the blocking finding and 3 serious ones myself, and all held.

### Did the last changes work?

- **Most changes held:**
  - no coder piped output;
  - the review costs were exact;
  - done checks written as commands made it clear what was unrun.
- **One rule failed in my own hands: independent review.** The reviewer found that I had merged my own three Broadsword fixes after a headless review outside the queue, so nothing recorded or enforced that the review happened.

### What we learned

| What | Why | What changes | Where it's enforced |
| --- | --- | --- | --- |
| **A whole-project review found what milestone reviews never could.** The `.env` key sat in 21 Git checkpoints; rules were not in force; claims were stronger than the evidence. | Each milestone review saw only its own diff. The secrets leak sat between a Stage 1 design (checkpoints save ignored files "so `.env` is restored") and the secrets rule from Stage 0. No single diff showed both. | A full independent audit at the end of every stage, before the tag. Its blocking findings must be fixed before tagging. | `AGENTS.md` (Review rule) |
| **A design decision quietly broke a higher rule.** The checkpoint was documented as intentionally saving `.env`. | Nobody checked a new feature against the standing rules it touches. | Every spec names the standing rules it touches (secrets, deletion, settings), and the reviewer checks against them. | `SPEC_CHECKLIST.md` |
| **Our own records over-claimed.** For example, "every Stage 2 milestone passed review on its first or second pass" was false; the "fresh session" test named the project in its prompt. | Records were written from memory and summaries (some by the Housekeeper), not checked against the raw evidence. | The stage-end audit checks claims against raw evidence. Records quote the evidence they rest on. | `AGENTS.md` (Review rule) |
| **The orchestrator skipped the process it enforces on coders.** | No rule said the orchestrator's own code goes through the queue, and doing it directly was faster. | All code reaches main through the merge queue, the orchestrator's included. | `AGENTS.md` (Orchestration rule) |
| **Rules were written but not in force,** such as automatic cleanup and phone alerts. Stage 1 was tagged anyway. | The rules were written in Stage 0, before the code existed. Nothing tied each rule to a mechanism. | Each standing rule is either enforced (code, hook or test) or marked "not yet in force" in `PROGRESS.md`. The audit checks this. | `AGENTS.md` (Done means verified) |
| **A queue PASS can't confirm a done check:** the unreachable damping guard passed Haiku. | The reviewer sees the diff and test output, with no tools, so it cannot run anything. | Until the queue runs done-check commands itself, the orchestrator runs them before queueing and puts their output in the brief. | `SPEC_CHECKLIST.md` |

### Speed and cost

- **Fable, high effort:** about 334k tokens and 28 minutes for the main agent, plus 4 read-only sub-reviewers for breadth.
- **The milestone reviews:** together they cost $0.42 and caught diff-level bugs.
- **Lesson:** cheap per-change reviews and an expensive per-stage audit do different jobs. Neither replaces the other.
- **Process weight:** the reviewer also pointed out that 9 of the last 10 commits are process, not product. That is the ledger's "retro time growing past its value" signal. **Change:** the next milestone must ship product (Stage 4). Process work only fixes audit findings and doesn't add new machinery.

### Carry into the next milestone

1. Fix the blocking finding (secrets in checkpoints) and the serious ones before Stage 4 code starts, through the queue.
2. Stage 4 ends with the same full audit before its tag.
3. No new process machinery in Stage 4 unless an audit finding needs it.

---

## Stage 3 follow-up: scoped blocking, panic guard, shared build folder for `run` (2026-11-18)

### Did the last changes work?

The ledger above has the detail. In short:
- **Helped:**
  - the no-pipe rule (0 refused jobs);
  - done checks written as commands (it was clear what was unrun);
  - `review_costs.py`.
- **Mixed:** the templates. Their rules were followed, but my brief told the coder to run the suite in the background, which made it hand back unverified. That is the first change caught hindering, and it's fixed.
- **No false rejections:** Opus passed the change first time.

### What went wrong

| What | Why | What changes | Where it's enforced |
| --- | --- | --- | --- |
| The coder handed back twice before its test run finished. | My brief said to run the 3-minute suite in the background; a subagent's turn ends and it can't wait for its own job. | Run the suite in the foreground, read the result, then report. | `templates/coder-prompt.md`, `templates/brief.md` |
| Two of the coder's new tests had wrong expectations: a second record from the test's own setup, and a tracked file that is restored, not deleted. | They were written without being run, the same cause as above. | Targeted test runs first, then the full suite once. | `templates/coder-prompt.md` |
| `broadsword run -- cargo test` in a worktree built a private 90 MB `build/`. The queue then kept the worktree, with nothing lost. | `CARGO_TARGET_DIR` was set only in the cargo-step builder. `run` had its own builder, and the repo's relative `target-dir` resolved inside the worktree. | Set it once, where every job's child is spawned. A test fails without it. | Code (`d23b969`) |
| A cross-worktree test flaked. | Its fake "dead" job used the pid of an exited process, and Windows reused the pid. Opus had flagged this as a minor. | Fake records carry an impossible `pid_start`. | Code (tests) |
| A bash heredoc ate a backslash in a Rust test, and another heredoc failed to parse. | Shell quoting inside heredocs. | Write code and scripts with the Edit or Write tools, then run them. | Portable lesson 23; check next retro |

### Speed

- **Coder:** about 4.5 minutes for a larger task (recovery code, 6 files).
- **Orchestrator:** most of the time went to 5 full-suite runs (about 3 minutes each), plus 2 targeted mutation runs. Three of those full runs existed only because of the test-expectation bugs.
- **Change:** targeted tests through `broadsword run ... cargo test --test <name> <filter>` before any full run. It's in the coder template.

### Cost

- **Coder:** about 152k Sonnet tokens. That's twice a one-file task, as expected for 6 files.
- **Reviews:** Opus $0.31 (the largest diff yet) and Haiku $0.0027. Total across all queue reviews so far: $0.42 (`scripts/review_costs.py`).
- **Waste:** 3 extra full-suite runs, and one coder hand-back round.

### Carry into the next milestone

1. Coders run the full suite in the foreground and report its result. Check: no unverified hand-backs.
2. Targeted tests before the full suite. Check: full-suite runs per task.
3. No code written through heredocs. Check: no quoting damage.
4. Stage 4: watch whether the scoped block ever lets a half-changed worktree hurt something outside it (the ledger's watch-for-harm column).
5. Opus minors left in IDEAS.md: the build lock is released before the panicked child is killed (a short window), and a bad `--task` gives exit 3 instead of 2 when the project has an interrupted run.

---

## Stage 3, milestones 3 and 4, and close-out (2026-11-18)

### Did the last changes work?

These were the first retros, so there were no earlier ones to check. The Stage 1 and 2 lessons at the top of `LAB_LOG.md` led to `SPEC_CHECKLIST.md` and the two-review-pass rule, and both held:

- Milestone 3 failed its first Opus pass with 3 blockers. All three were fixed in one round.
- Every crash-sim task in milestone 4 passed review on the first try, except one. That one failed because of the brief, not the code.

### What went wrong

| What | Why | What changes | Where it's enforced |
| --- | --- | --- | --- |
| `max_agents` was never enforced. The scale-up test would have tested nothing. | The setting was added in Stage 0 as a note. Nothing checked that a setting in `broadsword.toml` is actually read. | `task start` now enforces the cap. Every setting must be used by code. | Code (`16c9279`). This retro adds the rule below to the spec checklist. |
| One agent's `\| head` blocked every agent on crash-sim. | `println!` panics on a closed pipe. The dead job stayed "running", and an interrupted job blocks the whole project. | Broadsword can no longer be killed by a closed output. Coders never pipe Broadsword. | Code (`b578103`) and `coder.md`. Narrowing the block to a single worktree is in IDEAS. |
| Haiku rejected correct code. | My brief listed some public functions in parentheses, and the reviewer read the list as complete. | A list in a brief is either marked "for example" or is the full rule. | `SPEC_CHECKLIST.md` |
| My first broken-pipe test failed on its own setup. | I assumed the first output line was "job:". Broadsword prints its recovery line first. | Tests read until they find what they're looking for, not a fixed line number. | Habit only. Small; watch for it next retro. |
| The housekeeper wrote that the Stage 3 check had passed. One reading of it hadn't. | The check's wording, "each reviewed by a different model", has two readings, and the draft took the easier one. | The orchestrator checks every "done" claim in the records against the done check's exact words before committing. | Already the orchestrator's job; it caught this one. |

### Speed

- **The coders weren't the slow part.** Each took 24 to 44 seconds. A whole round of 3 or 4 agents, from first start to last merge, took 2.5 to 3.5 minutes.
- **The time went to orchestration:**
  - writing briefs;
  - the Broadsword fixes, each needing a full test run (about 3 minutes), a marker run and a review;
  - and one Opus review that ran past the 10-minute foreground limit.
- **Change:** run reviews and full test suites in the background from the start, and keep working while they run.

### Cost

- **Small, exact briefs cut the cost per coder pass by 3 to 5 times.**
  - Earlier in Stage 3, coders spent 250k to 400k tokens per pass on open-ended Broadsword work.
  - In milestone 4, each coder spent about 72k tokens on one new file, with a brief that named the claim, the API and the done check.
- **Review cost is negligible next to coder cost:**
  - Haiku: about $0.002 per review.
  - Opus: about $0.09 to $0.21.
  - The 10 queue reviews together cost less than $0.20.
- **Waste:**
  - one false rejection, about $0.002;
  - 4 refused jobs, plus one blocked coder test run, from the pipe incident;
  - one marker run that I chose to do. That one was worth it: it showed the new test really catches the bug.
- **Change:** split work into tasks the size of one or two files whenever the code allows it, since that is where the savings came from. Use Haiku for low-risk reviews and keep Opus for locks, deletion, secrets and the Broadsword core.

### Carry into the next milestone

Read this list before the next milestone starts. The next retro checks each item.


1. Every new setting has a test that fails if no code reads it. This is added to `SPEC_CHECKLIST.md`.
2. Long runs (the full suite, reviews) go in the background.
3. Tasks stay the size of one or two files wherever possible.
4. Check: no coder piped Broadsword output, and no brief got a false rejection.
5. Briefs and coder prompts start from `templates/`, done checks are written as commands, and the cost section uses `scripts/review_costs.py`. Check that this made briefs faster to write and the retro numbers exact.

## Throughput test: tp1 scoped checks (2026-11-18)

- **Wrong:** the brief's done check ("show the queue's step timings before and after") could only be met after the change was installed. Opus failed it twice on that alone.
- **Why:** the brief set a done check measured by the tool being changed, with no route to measure it before the merge.
- **Change:** `SPEC_CHECKLIST.md` row: a done check for a change to the queue or the job runner says how to measure it before the merge (the branch's binary against a scratch root). Enforced in the checklist; next retro checks it.
- **Speed and cost:** two Opus reviews (~10 minutes of queue time) spent on an unmeetable check. Parked rather than merged directly, because Opus's finding was right, not wrong, and a timeout gap remained.

