# CONFIG-SPLIT: splitting broadsword.toml so unrelated setting changes stop queueing

Task t-20261121-044024-4. Design note only: no code, no settings, no claim change. Date of the measurement: 2026-11-21 04:40Z.
Approved as a process gap by Lazlo on 2026-11-21 04:39Z (cmsg_019Gy38ApTnA8WHdHwRbXZzH1Qud3JxHUTpDQcLc1gPZFr).

## 1. Measurement: tasks that waited on the broadsword.toml claim

Source: the task records in `state/tasks/*.task` (fields `created`, `started`, `ended`, `claims`, all epoch seconds; read-only), and the
log lines in `state/pc-progress.md` that name `broadsword.toml`. I took every broadsword task whose `claims` list names
`broadsword.toml` and that was created since 2026-11-20 09:00Z (the 092355 row below was created 09:23Z). The numbers come from a
throw-away script, `scratch/tomlwait.py` (scratch, wipeable): it prints, per task, created, started, ended, wait (started, or now,
minus created) and which other toml-claiming tasks were running during that wait.

| task | what it changed | created (Z) | started (Z) | ended (Z) | wait before start | held the toml claim |
|---|---|---|---|---|---|---|
| t-20261120-092355 codex coder route | a new required key | 11-20 09:23 | 11-21 00:02 (see note a) | 11-21 03:34, abandoned | 874 min (note a) | 232 min |
| t-20261120-152939 | (claims the toml) | 11-20 15:29 | 11-20 15:29 | 11-20 16:26 | 0 | 57 min |
| t-20261120-174501 parallel test binaries | `test_parallel_binaries` | 11-20 17:45 | 11-20 17:45 | 11-20 22:58 | 0 | 313 min |
| t-20261120-170450 python tests in parallel | open, never started | 11-20 17:04 | - | - | 695 min and still open (note b) | 0 |
| t-20261120-191952 GPT cap key | abandoned 1 min after creation | 11-20 19:19 | - | 11-20 19:20 | not a wait (note b) | 0 |
| t-20261121-011525 S5 linear re-land | needs its keys | 11-21 01:15 | 11-21 04:36 | running | 201 min (note c) | 3 min so far |
| t-20261121-034547 shadow switch to 6.2 | one value (line 75) | 11-21 03:45 | 11-21 03:45 | 11-21 04:32 | 0 | 47 min |
| t-20261121-035022 codex re-land | a new required key | 11-21 03:50 | - | - | 50 min and open | 0 |
| t-20261121-043519 live parallel-tests trial | `test_parallel_binaries` value | 11-21 04:35 | - | - | 5 min and open | 0 |
| t-20261121-043809 fast-subset lane | queue lane keys | 11-21 04:38 | - | - | 2 min and open | 0 |

Notes.
a. 092355 was created at 09:23Z with a gate on the 00:00Z GPT cap reset (log line 542), so most of its 874 minutes is that gate, not
   the toml. I count it only as a holder: it held `broadsword.toml` for 232 minutes (11-21 00:02Z to 03:34Z), and the log says the S5
   re-land and the queue.rs tasks waited for it (log lines 1178, 1182, 1262, 1264, 1311).
b. 170450 and 191952 are not toml waits that I can prove from the record: one is an open backlog task, the other was abandoned at once.
   I list them so the count is complete and exclude them from the totals.
c. 011525 also claims `queue.rs`, which the Haiku-low refusal task held until its merge (log line 1313, 1346). Its 201 minutes are the
   toml wait and the queue.rs wait added together. I cannot split them from the record. The log says that at 03:59Z the toml was the
   blocker ("held by the queued shadow switch", line 1346), and at 04:03Z again (line 1350).

Totals (window 2026-11-20 09:00Z to 2026-11-21 04:40Z, 19.7 hours):
- 10 tasks claimed the toml; 5 of them started (092355, 152939, 174501, 011525, 034547); 1 was abandoned at once (191952); 4 are still
  open and waiting (035022, 043519, 043809 behind 011525 and earlier holders; 170450 for an unknown reason, note b).
- Toml holding time: 232 + 57 + 313 + 47 = 649 minutes (11 hours of the 19.7), so the claim was busy more than half the window.
- Waiting that the log or the record ties to the toml: S5 re-land up to 201 minutes (shared with queue.rs), codex re-land 50 minutes
  so far, trial 5 and lane 2 minutes so far. Sum 258 minutes of coder-slot-time with a ready task, of which at most 201 is mixed with queue.rs.
- Two of these holders (shadow switch, 47 minutes) changed one number. The same file blocked work that did unrelated things.
- Earlier evidence: `docs/SCALING.md` line 176: 25 of 118 broadsword tasks (21 percent) claimed `broadsword.toml`. Tonight's share
  is 10 of the 137 tasks of 11-20 (the count in log line 741), 7 percent, because the window is only the recent hours; I did not
  count other projects' tasks, which do not claim it.

What this does not show: how many ready tasks the orchestrator never started because it saw the claim taken (the log says it held
the queue.rs tasks for the S5 re-land instead, line 1178). That cost is real but is not in the table.

## 2. Why one file is hot: what the file is and who reads it

- 53 keys in one flat file (`grep -cE "^[a-z_]+ *=" broadsword.toml`), read by a hand-written flat parser, `parse_flat` in
  `crates/broadsword/src/config.rs`, which supports only integers and quoted strings and no `[tables]`.
- One loader builds one `Config` from one file (`Config::load_from`, `find_config_dir`, `main_checkout_root`). Settings are global,
  not per project. Only `python_tests` is per project, and it is already a single string key.
- Every new required key touches three files at once: `broadsword.toml`, `config.rs`, `tests/support/mod.rs` (SCALING.md line 670,
  and 17 test files mention the toml). Today those three are all hot (18 claims for config.rs, per SCALING.md line 176).
- Readers outside Rust: `clients/openai_mcp.py` (`toml_string`: `install_dir`, `scratch_dir`), `clients/openai_review.py`
  (`state_dir`, `gpt_daily_cap_usd` via `tomllib`), `bench/drift/run.py` (`tomllib`, required paths). They read the root file directly.
  Any split must keep every key a Python reader needs in the root file, or change the reader in the same task.
- The every-key test, `every_key_in_the_real_toml_is_required_by_the_loader_and_used_by_code` (config.rs line 809), reads the single
  file with `include_str!`, removes each key in turn and expects the load to fail, then greps the other source files for `.key`.

## 3. Options

A. **Per-project files.** Rejected. Settings are global: of 53 keys exactly one (`python_tests`) is per project. The file that is
   hot (queue, review, test, board settings) would stay one file.

B. **Tables in the one file with claims on a table (`broadsword.toml#queue`).** This is what `docs/SCALING.md` section 3, rule 6
   proposed and Lazlo approved on 2026-11-20 (log line 316, "toml table claims"). It keeps AGENTS.md's "one settings file". It
   needs, in this order: a parser that reads tables (today `parse_flat` fails on `[queue]`: line `[queue]` has no `=`), a change to
   the claim syntax and overlap rule in `task.rs` (claims are plain paths today, "never patterns"), the Python readers taught
   tables, and a way for the merge queue to see that two edits of one file touch different tables (git itself merges two edits
   in different parts of one file; the claim system is what refuses). That is a board change plus a parser change plus a reader
   change, with no benefit until all of them land.

C. **Per-area files named by a required list in the root file** (recommended). `broadsword.toml` keeps the rarely changed core and
   gains one required key, `config_files`, naming the area files. Each area file is flat, uses the same parser, and is a plain path,
   so the existing claim system works with no board change: a task that edits review settings claims `config/review.toml`, not
   `broadsword.toml`.

D. **Auto-discovery (every `config/*.toml` is loaded).** Rejected for the first version: a stray or misspelled file would be loaded
   or ignored silently. The list in the root file makes a missing file and an unlisted file both errors.

## 4. Recommendation: option C

Reasons:
1. It uses what exists. Claims are paths (`task.rs`), the parser is flat (`parse_flat`), the Python readers keep working for root keys.
   B needs three subsystems changed before it helps; C needs the loader and the every-key test changed once.
2. It stops the pattern seen tonight. The shadow switch (review area), the codex timeout key, the lane keys (queue area) and the
   parallel-tests trial (test area) touch different areas. Of the tasks in the table with a real key change, only 174501 and 043519 (both
   `test_parallel_binaries`, test area) and 092355 and 035022 (both codex keys, same new area) would still share a file under the layout
   below; the shadow switch (review), the lane (queue) and S5 would not collide with them.
3. It is cheap to undo: setting `config_files = ""` and moving the keys back restores the old state; every key stays required.
4. The cost is the AGENTS.md sentence "All limits and paths live in one file". That is Lazlo's rule text; the change is his to approve
   (see section 8). The new rule reads: "all settings live in `broadsword.toml` or the files it lists in `config_files`".

What it does not fix. A new required key still touches `config.rs` and `tests/support/mod.rs` besides its area file. Those two are
still hot (18 claims for config.rs). Splitting the toml removes one of three contended files from every key change. A second step,
which is outside this note, is to move each area's keys into its own struct module so `config.rs` stops being one claim; then it can
be measured the same way. I do not claim the split alone cuts waits by a number; section 6 says how to measure it.

## 5. Proposed layout (53 keys)

| file | keys | count | why it lives there |
|---|---|---|---|
| `broadsword.toml` (core, changes rarely) | `root`, `disk_floor_gb`, `cleanup_every_hours`, `scratch_max_age_hours`, `scratch_dir`, `build_dir`, `projects_dir`, `state_dir`, `install_dir`, `worktrees_dir`, `disk_check_every_seconds`, `log_max_mb`, `ram_warn_percent`, `cpu_warn_percent`, `sweep_older_than_days`, `alert_command`, `job_disk_reserve_gb`, `job_disk_poll_seconds`, `checkpoint_ignored_max_mb`, `recover_lock_wait_seconds`, `max_resumes`, `mcp_call_timeout_seconds`, `mcp_output_max_kb`, `redact_min_chars`, plus the new `config_files` | 24 existing + 1 new | paths and safety limits; the folder-apart check and the Python readers need them in one known file |
| `config/board.toml` | `max_agents`, `max_queued`, `queue_entry_read_wait_ms`, `task_lock_wait_seconds` | 4 | slot scaling, edited often by the orchestrator |
| `config/queue.toml` | `queue_lock_wait_seconds`, `merge_train_max`, `merge_train_wait_minutes`, `queue_per_project`, `perf_gate`, `perf_runs` | 6 | merge queue, trains, perf gate |
| `config/review.toml` | `reviewer_backend`, `reviewer_cli`, `reviewer_model_low`, `reviewer_model_high`, `reviewer_model_high_fallback`, `reviewer_model_high_gpt`, `reviewer_effort_high`, `reviewer_timeout_seconds`, `reviewer_diff_max_kb`, `reviewer_brief_max_kb`, `reviewer_check_tail_kb`, `reviewer_max_calls_per_entry`, `shadow_reviewer`, `shadow_reviewer_model`, `shadow_reviewer_timeout_seconds` | 15 | reviewer and shadow reviewer models, edited for model switches |
| `config/test.toml` | `build_lock_wait_seconds`, `python_tests`, `test_parallel_binaries`, `test_output_wait_seconds` | 4 | test and check job settings |

Total 24 + 4 + 6 + 15 + 4 = 53 existing keys, so nothing is added or dropped except `config_files`. (I counted the 15 review keys from
lines 51 to 62 and 73 to 76 of the current file; the sum matches the 53.) The assignment is a proposal: the task that does the move
re-counts with the grep above and stops if the numbers differ.

Python-read keys: `install_dir`, `scratch_dir`, `state_dir` and the paths stay in the root file. The pending `gpt_daily_cap_usd` (task
191952, abandoned) would be read by `openai_review.py` from the root file today. If it is created in `config/review.toml`, the same
task must change that reader. Placing it in the root until the readers use one shared loader avoids a silent miss.

Comments. Each key keeps its explanatory comment when it moves, verbatim (many carry decisions, dates and revert instructions, for
example `merge_train_max`, `test_parallel_binaries`).

## 6. Loader and test design (what the later code task must satisfy)

1. `config_files` is a required string key in `broadsword.toml`: space-separated paths relative to `root`, each ending in `.toml`, each
   inside `root`, none inside `scratch_dir`, `build_dir`, `state_dir`, `worktrees_dir`. `""` means no area files. No default in code.
2. The loader parses the root file, then each listed file with the same `parse_flat`, and merges. Fail closed: an unreadable listed file,
   a key in two files, a key in the wrong place under a rule (see 5 for the root-only keys), or a `config/*.toml` file that exists and
   is not listed are all errors that name the file and the key.
3. Worktrees: the files are read from the main checkout's root, exactly as `broadsword.toml` is today (`main_checkout_root`).
4. The every-key test is extended, not replaced: it enumerates the keys of the root file and of every file it lists, removes each key
   in turn from its own file and expects the load to fail, and keeps the grep for `.key` use. It also gets two new cases: a key in two
   files fails, and an unlisted `config/*.toml` fails.
5. Test configs: every test that writes a hand-written `broadsword.toml` (17 files mention it; the log line 615 shows how a missing
   key broke `git.rs` tests) goes through the shared builder in `tests/support/mod.rs`, which writes the root file and every listed
   file, or sets `config_files = ""` and keeps all keys in the root file. The task states which, and runs `grep -rn "broadsword.toml" crates/broadsword/tests crates/broadsword/src`
   to prove no hand-written config is left.
6. Python readers: `toml_string` in `clients/openai_mcp.py` and the `tomllib` readers read only root keys; the task proves that with a
   grep for every key each of them reads.

Show each new test failing first (the coder prompt rule): remove the guard, run, see it fail, restore.

## 7. Migration path

1. Quiet window. Run the move only when no task holds the toml claim and none is open that claims it (the board lists claims). Tonight
   that means after 011525, 035022, 043519 and 043809 have merged or been re-pointed; the log shows the toml queue order is decided by
   the orchestrator (line 1385).
2. One task, one claim set: `broadsword.toml`, the new `config/*.toml` files, `crates/broadsword/src/config.rs`,
   `crates/broadsword/tests/support/mod.rs`, `crates/broadsword/tests/*.rs` that write a config, and the Python readers if a key moves.
   Risk high (the loader gates every command: fail-closed on a bad config). Opus gate.
3. Step A, a loader that accepts both: `config_files = ""` and everything in the root file, same as today. Check passes, merge. Nothing
   moves yet, so a failure here is a loader bug only.
4. Step B, the move: set `config_files`, move the keys with their comments, merge. The every-key test now covers all files.
5. Step C, update the claim habit: briefs claim `config/<area>.toml` for a setting in that area (BRIEF template and the claim section of
   SPEC_CHECKLIST.md get one line). An edit to `broadsword.toml` stays allowed but is for core keys only.
6. AGENTS.md, Settings: change the sentence to the new rule. This text is Lazlo's; it ships only with his yes (section 8).
7. Revert: put the keys back in the root file and set `config_files = ""`. Steps A and B are separate commits so B reverts alone.

Every key stays required and used. Required: the loader still errors if a key is missing from every file. Used: the every-key test still
removes each key and greps the code.

## 8. Judging the change: before and after

Measure (per 24 hours of orchestrator time, from `state/tasks/*.task`): for tasks whose `claims` name `broadsword.toml` or a `config/*.toml`
file, (a) the number of such tasks, (b) the sum of `started - created` for those whose start was held by a toml-area claim holder (the
overlap column of the script), (c) the hours a toml-area claim was held while another ready task wanted it.

Baseline (section 1, window 2026-11-20 09:00Z to 2026-11-21 04:40Z): 10 tasks, 258 minutes of waiting tied to the toml (201 of them
shared with queue.rs), 649 minutes of claim holding in 19.7 hours (55 percent of wall time), 4 tasks still waiting when measured.

How the change is judged: after the move merges, take the next 20 toml-area tasks or the next 24 hours, whichever is longer, with the
same script. Keep the split if the minutes of waiting behind a holder drop and no new class of failure appears. Revert (section 7 step 7)
if either of these happens:
- waiting does not fall (the area files all get claimed together, because one task still edits several areas), or
- the watch-for-harm signal: a config load failure, a "key in two files" or "unlisted file" error on main, or a test that reads a stale
  root file, in the first 20 tasks.
Also record the number of tasks that still wait on `config.rs` or `tests/support/mod.rs` for a new key, since the section 4 limit says
those stay hot. If most waits move there, the next change is the per-area struct split, not more toml files.

Ledger row to add in RETRO.md when it ships (the orchestrator owns that file; this task does not edit it): change `config_files` split;
measure above; watch-for-harm above; revert rule section 7 step 7.

## 9. Decisions for Lazlo

1. Approve option C (area files named by a required list) over B (tables and table claims, approved earlier on 2026-11-20 as a
   direction). C needs no board change and gives the claim benefit sooner; B stays possible later inside an area file.
2. Approve rewording the AGENTS.md Settings rule from "all limits and paths live in one file" to "in broadsword.toml and the files it lists".
3. Agree the move task runs in a quiet window and its claims (section 7 step 2), risk high.

## 10. What was not verified

- No code or config was changed, so the layout and the loader design were not run. The 53-key count and the group sizes come from
  grep and a hand count of the current file; the extension test the design needs does not exist yet.
- The wait table is a computation over the task records of the time of the measurement; ongoing tasks keep growing. Each wait is the
  time from `created` to `started`, which includes any other cause (a dependency, a held claim on another file, a cap).
- The "claimed the toml" column for 152939 is from the claims field only; I did not open what it changed.
