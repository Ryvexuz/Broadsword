# CONFIG-SPLIT: splitting broadsword.toml so unrelated setting changes stop queueing

Task t-20261121-044024-4. Design note only: no code, no settings, no claim change. Date of the measurement: 2026-11-21 04:40Z.
Approved as a process gap by Lazlo on 2026-11-21 04:39Z (cmsg_019Gy38ApTnA8WHdHwRbXZzH1Qud3JxHUTpDQcLc1gPZFr).

## 1. Measurement: tasks that waited on the broadsword.toml claim

Source: the task records in `state/tasks/*.task` (fields `created`, `started`, `ended`, `claims`, epoch seconds; read-only) and the log
lines in `state/pc-progress.md` that name `broadsword.toml`. Every figure below comes from the committed script `scripts/tomlwait.py` (run
from the project root as `build/bin/broadsword.exe run broadsword -- python -I scripts/tomlwait.py`; it reads the main checkout's
`state/tasks`), not from hand arithmetic. It takes every task whose `claims` list names `broadsword.toml` or a `config/*.toml` file and
that was created from 2026-11-20 09:00Z to 2026-11-21 04:40Z. Columns: wait_min = created to started (to ended for an abandoned task
that never started, else to the window end), hold_min = started to ended (or window end), overlap_min = the part of the wait during
which another listed task held the claim. Its output, pasted unchanged:

```
window 11-20 09:00:00 to 11-21 04:40:00 minutes 1180.0
id | status | created | started | ended | wait_min | hold_min | overlap_min
t-20261120-092355 abandoned 11-20 09:23:55 11-20 23:42:33 11-21 03:34:41 wait=858.6 hold=232.1 overlap=370.6
t-20261120-112230 open 11-20 11:22:30 - - wait=1037.5 hold=0.0 overlap=653.2
t-20261120-152939 done 11-20 15:29:39 11-20 15:29:39 11-20 16:26:48 wait=0.0 hold=57.1 overlap=0.0
t-20261120-170450 open 11-20 17:04:50 - - wait=695.2 hold=0.0 overlap=596.1
t-20261120-174501 done 11-20 17:45:01 11-20 17:45:02 11-20 22:58:28 wait=0.0 hold=313.4 overlap=0.0
t-20261120-191952 abandoned 11-20 19:19:52 - 11-20 19:20:15 wait=0.4 hold=0.0 overlap=0.4
t-20261121-011525 running 11-21 01:15:25 11-21 04:36:37 - wait=201.2 hold=3.4 overlap=186.4
t-20261121-034547 done 11-21 03:45:47 11-21 03:45:48 11-21 04:32:56 wait=0.0 hold=47.1 overlap=0.0
t-20261121-035022 open 11-21 03:50:22 - - wait=49.6 hold=0.0 overlap=46.0
t-20261121-043519 open 11-21 04:35:19 - - wait=4.7 hold=0.0 overlap=3.4
t-20261121-043809 open 11-21 04:38:09 - - wait=1.9 hold=0.0 overlap=1.9
tasks 11 started 5 abandoned_unstarted 1 open_unstarted 5
total hold of started+ended tasks: 649.9 min = 55.1% of window
measure_a tasks=11
measure_b overlap_sum_min=1857.9
measure_c contended_hold_min=653.2 hours=10.89
```

What each row is (from the task titles and the log): 092355 the codex coder route (a new required key); 152939 not opened, claims only;
174501 parallel test binaries (`test_parallel_binaries`); 011525 the S5 linear re-land (needs its keys); 034547 the shadow switch to
gpt-6.2-sol (one value); 035022 the codex re-land (a new required key); 043519 the live parallel-tests trial; 043809 the fast-subset lane;
112230 the F7 archive task; 170450 Python tests in parallel; 191952 the GPT cap key.

Notes.
a. 092355 started at 23:42:33Z and ended 03:34:41Z (hold 232.1 min). Its wait of 858.6 min is mostly its own gate on the 00:00Z GPT cap
   reset (log line 542), not the toml. I count it only as a holder; the log says the S5 re-land and the queue.rs tasks waited for it
   (log lines 1178, 1182, 1262, 1264, 1311).
b. 112230 (open, 1037.5 min), 170450 (open, 695.2 min) and 191952 (abandoned 23 s after creation) are not toml waits I can prove from the
   record. They are listed so the count is complete and are excluded from the toml-wait total.
c. 011525 also claims `queue.rs`, which the Haiku-low refusal task held until its merge (log lines 1313, 1346). Its 201.2 minutes are the
   toml wait and the queue.rs wait added together; the record cannot split them. The log says the toml was the blocker at 03:59Z ("held
   by the queued shadow switch", line 1346) and at 04:03Z (line 1350).

Totals, derived from the output above (window 1180.0 min = 19.7 hours):
- 11 tasks claimed the toml. 5 started (092355, 152939, 174501, 011525, 034547). 1 was abandoned unstarted (191952). 5 are open and
  unstarted (112230, 170450, 035022, 043519, 043809).
- Toml holding time of the four finished holders: 232.1 + 57.1 + 313.4 + 47.1 = 649.7 min by the rows; the script's own sum is 649.9 min
  (it adds unrounded seconds). That is 10.8 hours, 55.1 percent of the window.
- Waiting tied to the toml by the log or the record: 011525 201.2, 035022 49.6, 043519 4.7, 043809 1.9: 257.4 minutes, of which the
  201.2 are mixed with queue.rs. 035022, 043519 and 043809 were still waiting when measured, so the figure grows.
- The shadow switch (034547) changed one value and held the claim 47.1 minutes; the same file held back work that did unrelated things.
- Earlier evidence: `docs/SCALING.md` line 176: 25 of 118 broadsword tasks (21 percent) claimed `broadsword.toml`. I do not compare a
  share for tonight: the window is hours, the all-task count for it was not taken.

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
   parallel-tests trial (test area) touch different areas. Of the tasks above with a real key change, only 174501 and 043519 (both
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
7. Revert: put the keys back in the root file, set `config_files = ""` and delete the `config/*.toml` area files in the same commit (loader
   rule 2 makes an unlisted `config/*.toml` that exists an error, so a revert that leaves them behind fails closed). Steps A and B are
   separate commits so B reverts alone; `git revert` of the B commit does all three at once.

Every key stays required and used. Required: the loader still errors if a key is missing from every file. Used: the every-key test still
removes each key and greps the code.

## 8. Judging the change: before and after

Measure, all taken from the output of `scripts/tomlwait.py` (the same committed script, section 1) over the tasks whose `claims` name
`broadsword.toml` or a `config/*.toml` file. Each is a named line or column of that output:
- (a) `measure_a tasks=` : the number of such tasks in the window.
- (b) `measure_b overlap_sum_min=` : the sum of the `overlap_min` column, the minutes tasks waited while another toml-area task held the
  claim. It is an upper bound on the claim-caused wait: a wait can have another cause at the same time (a gate, a cap, queue.rs).
- (c) `measure_c contended_hold_min=` : the minutes in which a toml-area claim was held while at least one other toml-area task waited.

Baseline, the script output of section 1 (window 2026-11-20 09:00Z to 2026-11-21 04:40Z, 1180.0 minutes): (a) 11 tasks, (b) 1857.9
minutes, (c) 653.2 minutes (10.89 hours). Also from that output: 649.9 minutes of claim holding by finished holders, 55.1 percent of the
window, 5 tasks open and unstarted. The 257.4 minutes of section 1 is a different, narrower reading (four tasks the log ties to the
toml); it is not a baseline for these measures.

How the change is judged: after the move merges, run the same committed script with the same arguments over the next 20 toml-area tasks or the
next 24 hours, whichever is longer, with the `config/*.toml` match already in the script, and compare lines (a), (b) and (c).
Divide (b) and (c) by the window minutes before comparing, since the windows differ. Keep the split if (b) and (c) per window minute drop and no new
class of failure appears. Revert (section 7 step 7)
if either of these happens:
- (b) and (c) per window minute do not fall (the area files all get claimed together, because one task still edits several areas), or
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
