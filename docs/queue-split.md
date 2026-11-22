# Queue split design

## Scope and starting point

Design only: the claim for this task is **`docs/queue-split.md` only**. No Rust,
configuration, claim-parser, formatting-policy, or task-release change is included.
The other fixes in the supplied TOP 3 FIXES remain separate work.

This inventory describes the files in this task worktree, not a verified main
commit. Git metadata is unavailable to this coder. Before implementation, the
orchestrator must confirm that task 020511 has merged, record the starting SHA,
and refresh this inventory against that tree. Do not start the physical split
against an older snapshot. The later 005330-2 key/module-claim work is separate;
the file-level claims below do not depend on it.

Goal: preserve the existing `queue::` API and all queue behaviour while giving
review, check/perf, train, and merge work separate source and test files. This is
not an opportunity to fix existing behaviour or consolidate unrelated fixtures.

## Today's layout

Counts were measured with PowerShell `@(Get-Content -LiteralPath $p).Count`,
including blank and comment lines. Observed output:

```text
crates/broadsword/src/queue.rs lines=3248
crates/broadsword/tests/queue.rs lines=1951
crates/broadsword/tests/queue_train.rs lines=976
crates/broadsword/tests/queue_shadow.rs lines=354
```

The source already has a durable-record state machine, per-project run lanes,
reviewer selection and calls, checks including the perf gate and done check,
single-entry merging, and merge trains. It is not just a review wrapper.

### Source sections

Line anchors below refer to `crates/broadsword/src/queue.rs` in this snapshot.
The destination table later in this document is the complete production-function
inventory; the anchors here identify the main sections.

| Lines | Section, anchored by existing functions |
| --- | --- |
| 1-156 | Module contract, imports, public status/exit constants, step constants, result alias. |
| 157-322 | Storage and locks: `queue_dir()`, `load_entries()`, `run_lock()`, `lane_for()`; display helpers `when()`, `short()`, `some()`. |
| 323-413 | Gate policy: `gating_model()`, `effort_for()`, `same_model()`; resume/open predicates `half_merged()`, `resume_first()`, `is_open()`; secret collection `process_secrets()`. |
| 414-566 | Brief validation and add: `canon_inside()`, `read_brief()`, `cmd_add()`. |
| 567-682 | Display: `shown_status()`, `cmd_list()`, `cmd_show()`. |
| 683-803 | Cancellation: `cancel_refusal()`, `cmd_cancel()`. |
| 804-1107 | Driver/drain: `cmd_run()`, `claim()`, `begin()`, `cmd_run_all()`, `row_for()` and exit aggregation. |
| 1108-1300 | `Run` state, persistence and pipeline: `new()`, `save()`, `pipeline()`, `merge_phase()`, `on_trunk()`. |
| 1301-1530 | Train preparation: `s_defer_check()`, `approve()`, `train_ready()`, `train_link()`, `leave_train()`. |
| 1531-1677 | Check execution and checkout location: `perf_gate()`, `check_commit()`, `check_alone()`, `locate()`, `restore_alone_checkout()`. |
| 1678-1925 | Preconditions/rebase/check/done check: `s_preconditions()`, `s_rebase()`, `s_check()`, `s_done_check()`. |
| 1926-2222 | Review and shadow recording: `s_review()`, `gpt_gate()`, `s_shadow()`, `write_review()`. |
| 2223-2358 | Merge/main check/finish: `s_merge()`, `test_hook_refusal()`, `s_main_check()`, `s_done()`. |
| 2359-3071 | Train result types, grouping, trunk/chain helpers and driver: `pick_train_when()`, `run_trains()`, `run_train()`, `merge_members()`. |
| 3072-3097 | Prompt/log helpers: `with_done_check()`, `read_tail()`. |
| 3098-3248 | Inline unit tests: tail handling, model-family comparison, step order, claim overlap, train size/readiness/waiting, exit severity. |

### Integration-test sections

These anchors refer to `crates/broadsword/tests/queue.rs`. Function spellings
below are existing test functions in that file, not proposed production helpers.
The source and test inventories distinguish the two files both named `queue.rs`.

| Lines | Current contents and proposed test leaf |
| --- | --- |
| 1-323 | Imports, JSON/support modules, scratch-root fixture, Git/process helpers, bounded waits and tiny project text. Keep in the parent test crate initially. |
| 324-389 | Happy path across all stages -> `tests/queue/merge.rs`. |
| 390-448 | Rejection/requeue and malformed review output -> `tests/queue/review.rs`. |
| 449-471 | Failed check stops review -> `tests/queue/check.rs`. |
| 472-641 | Rebase conflict, dirty main and trunk selection -> `tests/queue/merge.rs`, except train-specific case at 586-599 -> `tests/queue/train.rs`. Shared trunk helpers at 530-561 stay in the parent, since both leaves use them. |
| 642-795 | Gate/coder selection, low-risk policy and queue-add validation; local helpers at 671-681 and 698-702 -> `tests/queue/review.rs` for policy and `tests/queue/entry.rs` for add validation. |
| 796-844 | Reviewer timeout/CLI failure -> `tests/queue/review.rs`. |
| 845-869 | Main moves before merge -> `tests/queue/merge.rs`. |
| 870-910 | Reviewer environment/prompt scrubbing -> `tests/queue/review.rs`. |
| 911-965 | Checked/reviewed tip binding -> `tests/queue/merge.rs`. |
| 966-1151 | Secret scrubbing, reviewer provenance, malformed verdicts and call cap -> `tests/queue/review.rs`. |
| 1152-1171 | Duplicate open-entry refusal -> `tests/queue/entry.rs`. |
| 1172-1197 | Effort and model families -> `tests/queue/review.rs`. |
| 1198-1242 | Ignored worktree file and post-merge main-check failure -> `tests/queue/merge.rs`. |
| 1243-1291 | Reviewer death and interrupted review -> `tests/queue/review.rs`. |
| 1292-1364 | Interrupted rebase helper, resume/cancel and already-fast-forwarded resume -> `tests/queue/merge.rs`. |
| 1365-1527 | Global/per-project concurrency and input checks -> `tests/queue/locks.rs`. |
| 1528-1632 | MCP fixture and tool-call test -> `tests/queue/entry.rs`. |
| 1633-1712 | Hand-made interrupted-entry helper, recorded fail/missing tip -> `tests/queue/review.rs`; scratch-only merge refusal and reviewer-path merge evidence -> `tests/queue/merge.rs`. Keep the interrupted-entry helper in the parent if shared. |
| 1713-1775 | Brief path/secret/link validation -> `tests/queue/entry.rs`. |
| 1776-1802 | Existing train fixture helpers, copied in the existing train test crate. Keep parent support initially. |
| 1803-1951 | Fake GPT client fixture extension and GPT/fallback policy tests -> `tests/queue/review.rs`. |

Main test-section function anchors (all in `tests/queue.rs`):

- Happy path: `happy_path_rebases_onto_a_moved_main_checks_reviews_and_merges()`.
- Review/requeue: `a_planted_bug_is_rejected_before_merging_then_the_fix_is_requeued_and_merged()`.
- Check failure: `a_failing_check_stops_before_the_reviewer()`.
- Rebase: `a_rebase_conflict_is_aborted_and_the_branch_tip_is_unchanged()`.
- Preconditions: `a_dirty_main_checkout_blocks_the_entry_and_changes_nothing_then_it_can_be_run_again()`.
- Trunk: `a_merge_train_never_merges_into_a_checked_out_feature_branch()`.
- Add/cancel: `queue_add_refuses_what_cannot_be_merged_and_is_documented_in_exit_codes()`.
- Timeout: `a_reviewer_timeout_blocks_the_entry_with_no_verdict_and_it_can_be_retried()`.
- Commit binding: `a_commit_added_between_check_and_review_is_never_reviewed_or_merged()`.
- Secrets: `every_env_file_value_is_redacted_and_scrubbed_whatever_its_name_and_stored_text_is_redacted_too()`.
- Main-check failure: `a_check_that_fails_on_main_after_the_merge_is_loud_and_main_is_not_reset()`.
- Resume: `a_run_killed_mid_review_is_resumed_by_the_next_queue_run()`.
- Concurrency: `a_second_queue_run_gets_busy_while_one_is_running()`.
- MCP: `queue_add_list_and_run_work_over_mcp_tools_call()`.
- GPT gate: `a_skipped_gpt_gate_falls_back_to_the_sonnet_gate_in_the_same_run()`.

There already are separate `tests/queue_train.rs` and `tests/queue_shadow.rs`
integration crates. Keep them registered and unchanged during this split;
`tests/queue/train.rs` is a child module of the existing `queue` test crate,
not a replacement for the already-discovered `queue_train` target.
There is no dedicated perf section in `tests/queue.rs`; existing
`tests/perfgate.rs` remains authoritative for its own cases. A future
queue/perf-specific regression belongs in `tests/queue/check.rs`, not a new
empty test file pretending to provide coverage.

## Destination module map and API

All source paths in this table are under `crates/broadsword/src/queue/`.
Keep `mod.rs` as the facade and state-machine coordinator. All child modules are
private. Only the facade is crate-facing; preserve existing signatures, return
codes and visibility through explicit re-exports, not a new public submodule API.

| Destination | Functions moved there (methods remain methods on `Run`) | Exposed surface |
| --- | --- | --- |
| `mod.rs` | `cmd_run()`, `cmd_run_all()`, `in_project()`, `not_runnable()`, `claim()`, `begin()`, `code_num()`, `badness()`, `next_for_all()`, `worst_exit()`, `row_for()`; `go()`, `start_step()`, `pipeline()`, `until_review()`, `review_phase()` | Public `cmd_run()`, `cmd_run_all()`, plus explicit re-exports listed below; retain all existing public status/exit constants here. Own `Run`, `Row`, `STEPS`, `STEP_MERGE`, and result alias. |
| `entry.rs` | `queue_dir()`, `entry_path()`, `load_entries()`, `load_entry()`, `half_merged()`, `resume_first()`, `is_open()`, `process_secrets()`; `new()`, `branch()`, `save()`, `log()`, `set_step()`, `end()`, `blocked()`, `internal()`, `g()` | Re-export public `queue_dir()` from facade. Internal storage/status/persistence services only otherwise. Own entry extension and text-redaction key list. |
| `locks.rs` | `take()`, `queue_lock()`, `holds()`, `lane_key()`, `project_key()`, `lane_lock_file()`, `run_lock()`, `lane_for()`, `entries_lock()` | No new public API. Own `Lane`; expose lane/lock operations only within queue. |
| `add.rs` | `canon_inside()`, `read_brief()`, `cmd_add()` | Facade re-exports public `cmd_add()`. Validation stays together, unchanged. |
| `view.rs` | `when()`, `short()`, `some()`, `shown_status()`, `cmd_list()`, `cmd_show()` | Facade re-exports public `cmd_list()`, `cmd_show()`; formatting helpers internal to queue. |
| `cancel.rs` | `cancel_refusal()`, `cmd_cancel()` | Facade re-exports public `cmd_cancel()`; shares the same run/entries locking functions, never a second acquisition path. |
| `rebase.rs` | `locate()`, `restore_alone_checkout()`, `s_preconditions()`, `own_chain_rebase()`, `abort_stale_rebase()`, `keep_pre_rebase()`, `s_rebase()`, `refuse_chained()`, `chain_bases_of()`, `chained_base()`, `chained_onto()`, `left_train_text()` | No public API; checkout/rebase operations and chained-history guards internal to queue. |
| `review.rs` | `gating_model()`, `effort_for()`, `same_model()`, `s_review()`, `take_review_call()`, `gpt_gate()`, `s_shadow()`, `shadow_end()`, `write_shadow_file()`, `recorded_verdict_ok()`, `write_review()`, `with_done_check()` | Facade re-exports public `gating_model()`, `Gate`, `GateBackend`. Own shadow-review keys and GPT-client label. Calls, caps, provenance and output redaction stay unchanged. |
| `check.rs` | `check_step()`, `s_defer_check()`, `perf_gate()`, `check_commit()`, `check_alone()`, `s_check()`, `s_done_check()`, `read_tail()` | No public API. Own `TrainCheck` and `PassInfo`, visible within queue for both single-entry and train consumers. Perf remains part of the same commit-check path. |
| `merge.rs` | `merge_phase()`, `reviewed_on_main()`, `on_trunk()`, `s_merge()`, `test_hook_refusal()`, `s_main_check()`, `s_done()`, `trunk_tip()`, `trunk_checkout()`, `trunk_name()`, `merge_members()` | No public API. Both single-entry and train merge paths continue to share trunk checks and check execution; do not consolidate their control flow during this move. |
| `train.rs` | `load_claims()`, `approve()`, `train_ready()`, `train_link()`, `requeue()`, `leave_train()`, `pick_train()`, `pick_train_when()`, `take_group()`, `claim_approved()`, `train_stop_note()`, `finish_member()`, `leave_approved()`, `entry_leaves_blocked()`, `fail_members()`, `settle_approved()`, `still_to_run()`, `short_train_why()`, `short_train_projects_in()`, `short_train_projects()`, `run_trains()`, `chain_tip()`, `run_train()` | No public API. Own `Alone`. `pick_train()` retains its test-only compilation guard. |

The facade's external surface is exactly the existing public functions in the
table, `Gate`, `GateBackend`, and the existing public status/exit constants.
For example, callers in `src/tools.rs` and `src/task.rs` must not need edits
(current call sites: tools.rs:964-1103, task.rs:308-351).

Use separate inherent implementation blocks for `Run` in child modules.
Cross-module functions, methods, types and required fields get `pub(super)`
only where Rust privacy requires it; none need `pub(crate)` or `pub` except
existing facade exports. Define shared state in the parent so children can
access it. During migration, temporarily queue-visible state in `train.rs`
must remain restricted to the queue parent. Prefer explicit imports; no new
lint suppression or broad wildcard re-export. This is a visibility adjustment,
not a replacement state object or a new trait layer.

Inline unit tests travel with the functions they exercise: tail handling to
check, family comparison to review, train selection/readiness/waiting to train,
exit severity and pipeline-step assertions to the facade. Keep existing
assertions and test-only helpers; no test deletion or replacement by compile-only
checks.

## Move order: small, behaviour-identical tasks

Each numbered stage below is a sequence of independently reviewed tasks, **not
one commit per table row**. Each task has one destination module, with its
source deletion, necessary imports/visibility/facade wiring, and only tests
that fit the same budget. Tests move into the corresponding area; if they
do not fit alongside the source move, the next task moves only that area's
tests. Do not silently stretch a stage into a whole-area rewrite.

Budget: aim below approximately 300 added-plus-deleted lines, including
comments, visibility changes, wiring and tests. A moved line counts on both
sides unless Git records a whole-file rename. Pack complete declarations,
normally at most 120 source lines per task, leaving room for wiring. This is
a review-size target, not a runtime limit or configuration setting.

The large driver makes the initial destination important. PowerShell calculation
on its declaration through closing brace:

```text
run_train range=2718-2947
actual_driver_lines=230
actual_driver_add_delete=460
```

The outputs above were computed as `(2947 - 2718 + 1)` and
`(2 * (2947 - 2718 + 1))`, respectively.

Do **not** first rename the monolith to `mod.rs` and later delete/add that
driver. Instead:

1. **Seed train by rename.** Rename `src/queue.rs` to `src/queue/train.rs`
   without reformatting its contents, and add a small `src/queue/mod.rs`
   facade with explicit re-exports for the current public surface. The
   existing crate-level module declaration still resolves to `queue/mod.rs`.
   The train child temporarily owns everything. Require rename-aware diff
   output; reject a deletion/addition presentation of the entire file.
   Tests stay where they are and keep running through the unchanged API.
2. **Expose internal seams, then storage and locks.** Separate tiny tasks
   make only the visibility/import changes needed by the next move, keeping
   their destination to one module. Move the `Run`/`Row` definitions and
   shared constants to the facade in bounded batches; import them back into
   train. Extract `entry.rs`, then `locks.rs`, in complete-function batches.
   Persistence methods follow the state definition. Do not move the
   1-323 integration-test fixture wholesale: leave it at the parent.
3. **Add, view, cancel.** Extract `add.rs`, `view.rs`, `cancel.rs`, each as
   repeated single-destination tasks if needed. Move brief validation before
   the add command, and refusal checking before the cancel command.
   Repoint facade exports in the task that moves each exported command.
   Route corresponding add/cancel/MCP tests to the entry leaf.
4. **Check, then rebase.** Extract `check.rs` types/helpers before check
   methods; split the methods across tasks rather than moving the whole
   1531-1925 region. Move the tail unit test with its helper if budget allows.
   Extract `rebase.rs` the same way. Integrations go to check and merge
   leaves respectively. Preserve calls from the still-unmoved train methods.
5. **Review.** Move selection policy and gate types first; then review
   call accounting/GPT helpers, shadow helpers, review-file helpers, and
   finally the review step, as separate tasks. Retarget facade gate exports.
   Move the review leaf tests in bounded batches, including the fake-GPT
   fixture extension; keep any shared fixture declarations in the parent.
6. **Merge.** Move trunk helpers, then merge-phase helpers, the single-entry
   merge, main-check/finish, and train-member merge as separate tasks.
   `merge_members()` is a complete-function task, not bundled with its
   integration tests. Move merge tests in subsequent bounded batches.
7. **Facade driver and residual train tests.** Move the drain's supporting
   functions before `cmd_run_all()`; move `cmd_run()` and the pipeline
   methods separately. The large `run_train()` body never moves again:
   `train.rs` has become its intended module by subtraction. Move remaining
   inline train/exit tests to their owners. Wire `tests/queue/train.rs` and
   `tests/queue/locks.rs` in separate bounded test moves.
8. **Finish the test harness, without fixture redesign.** Keep
   `tests/queue.rs` as the Cargo-discovered parent containing shared fixtures
   and explicit path attributes for `queue/review.rs`, `queue/train.rs`,
   `queue/check.rs`, `queue/merge.rs`, `queue/entry.rs`, `queue/locks.rs`.
   Child modules can access parent-private fixtures. Do not add a competing
   `tests/queue/mod.rs` target. A shared-support extraction across integration
   crates is optional later work with a fresh claim and design, not a
   prerequisite for independent leaf-file edits.

Before queueing **each** move, the orchestrator attaches the exact function/test
batch, claimed paths, starting SHA, rename-aware stat and measured added/deleted
line total to its brief. If complete declarations plus wiring cannot fit, split
the batch again; never use disabled copies, stub bodies, include-fragment tricks,
behavioural refactors, or new allow attributes to satisfy the size target.
Do not run whole-crate formatting as part of a move. Check only changed Rust
files with `rustfmt --check --edition 2021 <changed .rs files>` through the
approved Broadsword run route; formatting-only cleanup is separate work.

For implementation verification, compare before/after discovered test names,
normalising only the newly added module prefix. Fail on an empty listing,
lost/duplicate test, or missing existing integration target. Full tests run
only through Broadsword. Preserve behavioural assertions: for example, if the
checked-tip equality refusal were disabled, the existing tip-change cases at
tests/queue.rs:911-965 must observe an unexpected review/merge instead of the
asserted stale/refused outcome. Moving a test and merely compiling it is not
evidence that this safety contract holds. No mutation test was run here.

## Claims and parallel work

Use exact repo-relative file claims, not the whole `src/queue` or `tests`
directory. After the split, independent examples are:

| Area task | Exact claims |
| --- | --- |
| Reviewer policy/calls | `crates/broadsword/src/queue/review.rs`, `crates/broadsword/tests/queue/review.rs` |
| Train scheduling | `crates/broadsword/src/queue/train.rs`, `crates/broadsword/tests/queue/train.rs`, and `crates/broadsword/tests/queue_train.rs` only if edited |
| Commit checks/perf integration | `crates/broadsword/src/queue/check.rs`, `crates/broadsword/tests/queue/check.rs` |
| Merge/trunk | `crates/broadsword/src/queue/merge.rs`, `crates/broadsword/tests/queue/merge.rs` |
| Storage/add/cancel | Only the applicable `entry.rs`, `add.rs`, `cancel.rs` source leaves, plus `crates/broadsword/tests/queue/entry.rs` |
| Run-lane locking | `crates/broadsword/src/queue/locks.rs`, `crates/broadsword/tests/queue/locks.rs` |

These are examples, not blanket permission to edit every listed path. A
reviewer task that edits shadow integration tests must additionally claim
`crates/broadsword/tests/queue_shadow.rs`. A check task editing perf tests
must claim `crates/broadsword/tests/perfgate.rs`. A fixture change must claim
`crates/broadsword/tests/queue.rs` and any shared support it edits. A shared
state or API change must claim `crates/broadsword/src/queue/mod.rs`. Such tasks
serialize with one another; ordinary leaf changes need neither parent file.
If a new setting is required, stop and expand the brief to cover config,
`broadsword.toml` and all fixture/parser sites; this design adds no setting.

During extraction, tasks necessarily share the seeded `train.rs` donor and
often a facade/test-parent claim, so run those moves sequentially. The seed
task claims the old source, renamed destination and facade. A later move
claims its donor, destination, facade if rewired, test parent if registering
a leaf, and every test file it actually changes. Do not promise parallelism
until those shared edits have merged. Independent leaf tasks can then run
side by side without waiting for the later key/module-claim feature.

## Risks and invariants to review at every move

- **Private helpers and method visibility:** `Run` methods cross every boundary.
  Preserve one shared state object and the persistence/redaction path in
  `save()`; do not copy helpers into multiple modules. Inherent implementations
  must retain lifetime parameters. Shared check result types must not drift.
- **Locks and ordering:** keep every run/cancel acquisition through `run_lock()`,
  the existing global/per-project resource key, then entries lock. Preserve
  `Lane`'s lifetime around the drain. A split must not drop a lock early or
  introduce stale-lock takeover. Existing concurrency cases remain active.
- **Resume and durable records:** keep status strings, step order, field names,
  write order and half-finished-train precedence byte-for-byte in logic.
  Preserve both `half_merged()` and `resume_first()` consumers; do not infer
  interruption from a different lock or change cancellation eligibility.
- **Checks, secrets and merge:** keep check/review binding to the exact commit,
  deferred-check fallback, perf checks during train/bisect checks, redaction
  before record/prompt output, caps, and scratch-only reviewer hooks.
  Both merge paths must keep trunk-ref checks, fast-forward-only history and
  loud main-check failure without reset. No claim-release fix is bundled here.
- **Test support and discovery:** leave scratch containment, per-test naming,
  copied fake reviewers, timeout/wait behaviour, and child-process cleanup
  unchanged. Do not unify the existing copied fixtures just for elegance.
  Explicit child path attributes avoid Cargo target-name collisions.
  Move imports with tests; avoid unused imports and newly added lint allows.
- **Rebases and rollback:** refresh anchors after 020511 and after each move;
  pending tasks must rebase before their done checks. Revert an individual
  mechanical move through the queue if it changes behaviour or cannot be
  reviewed cleanly; never discard work or reset history. The brief's
  rebase-conflict warning remains a monitoring item for the orchestrator,
  not a measured result of this design.

## Design done checks (orchestrator)

The requested checks are: the file exists, every function named here exists
in its current `queue.rs` file (grep), and `git diff --stat` shows only this
document. No build, test, grep process or Git command was run by this coder.
Local reads and the in-memory checks below are not a substitute for the
orchestrator's Git/done-check evidence. Run on the task tree being reviewed,
before any implementation split; record that tree's SHA with the evidence.

PowerShell existence and exhaustive name check, using only built-in operations
(no scratch files). Every referenced function uses an identifier followed by
empty parentheses. Check production names against `src/queue.rs` and the test
anchors against `tests/queue.rs`; neither permits inventing a future helper.
The negative control inserts only an in-memory nonexistent name and must be
detected; it does not mutate source or documentation.

```powershell
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath docs/queue-split.md -PathType Leaf)) {
    throw 'FAIL: design file missing'
}
'PASS: design file exists'
$doc = Get-Content -LiteralPath docs/queue-split.md -Raw
$src = Get-Content -LiteralPath crates/broadsword/src/queue.rs -Raw
$test = Get-Content -LiteralPath crates/broadsword/tests/queue.rs -Raw
$names = @([regex]::Matches($doc, '\b([A-Za-z_][A-Za-z_0-9]*)\(\)') |
    ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
if ($names.Count -eq 0) { throw 'FAIL: empty function inventory' }
$pattern = '(?m)^\s*(?:pub(?:\([^)]*\))? )?fn\s+([A-Za-z_][A-Za-z_0-9]*)'
$sourceNames = @([regex]::Matches($src, $pattern) |
    ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
$testNames = @([regex]::Matches($test, $pattern) |
    ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
if ($sourceNames.Count -eq 0 -or $testNames.Count -eq 0) {
    throw 'FAIL: empty source or test declarations'
}
$declared = @(($sourceNames + $testNames) | Sort-Object -Unique)
$testSection = ($doc -split 'Main test-section function anchors', 2)[1]
if (-not $testSection) { throw 'FAIL: missing test anchors' }
$testSection = ($testSection -split 'There already are separate', 2)[0]
$testRefs = @([regex]::Matches($testSection, '\b([A-Za-z_][A-Za-z_0-9]*)\(\)') |
    ForEach-Object { $_.Groups[1].Value } | Sort-Object -Unique)
if ($testRefs.Count -eq 0) { throw 'FAIL: empty test anchors' }
if (@($testRefs | Where-Object { $_ -notin $testNames }).Count -ne 0) {
    throw 'FAIL: test anchor absent from tests/queue.rs'
}
if (@($names | Where-Object { $_ -notin $testRefs -and $_ -notin $sourceNames }).Count -ne 0) {
    throw 'FAIL: production reference absent from src/queue.rs'
}
$missing = @($names | Where-Object { $_ -notin $declared })
$probe = '__queue_split_missing_probe__'
if ($probe -in $declared) { throw 'FAIL: negative-control name collision' }
$negative = @(($names + $probe) | Where-Object { $_ -notin $declared })
"referenced_functions=$($names.Count) missing=$($missing.Count)"
"negative_control_missing=$($negative.Count)"
if ($missing.Count -ne 0) { throw "FAIL: absent functions: $missing" }
if ($negative.Count -ne 1 -or $negative[0] -ne $probe) {
    throw 'FAIL: name-check negative control'
}
'PASS: every referenced function is declared; negative control detected'
```

For the literal grep inventory and Git stat, run these in the orchestrator's
POSIX shell at the task-worktree root (external runs have timeouts):

```sh
timeout 30s grep -nE '^[[:space:]]*(pub(\([^)]*\))? )?fn[[:space:]]+' crates/broadsword/src/queue.rs
timeout 30s grep -nE '^[[:space:]]*(pub(\([^)]*\))? )?fn[[:space:]]+' crates/broadsword/tests/queue.rs
timeout 30s git diff --stat
```

Require successful exits and nonempty labelled evidence: the grep output must
correspond to the PowerShell-validated references, and the stat must name only
`docs/queue-split.md`. Git stat omits untracked files, so an empty stat is not
a pass. If the new file has already been staged by the orchestrator, also
inspect the complete task delta:

```sh
timeout 30s git diff HEAD --stat
timeout 30s git status --short
```

The complete delta and status must show no changed path except this document.
Do not claim the design done check passed until the orchestrator records these
results. If its normal validation requires the project test route, the exact
foreground command for this task is
`broadsword test broadsword --task t-20261122-040443-2`; it was not run here.
