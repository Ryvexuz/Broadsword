use super::*;

pub(super) const QUEUE_ADD: Tool = Tool {
        name: "queue_add",
        summary: "Put a finished task branch in the merge queue, to be rebased, checked, reviewed by another model and merged.",
        description: concat!("Queues the work of a `running` task for `queue_run`: records one queue entry (state/queue/<entry id>.entry) with the task, project, branch `task/<id>`, the risk (`low` or `high`, which picks the reviewer model: the reviewer_model_low or reviewer_model_high setting, `haiku` and `opus` as shipped), the coder's model name (the reviewer must be a different model) and a brief for the reviewer: the contents of the file given as `brief` (state the task and its done check there; at most reviewer_brief_max_kb KB; the file must be inside the project root after resolving `..` and links, and is refused (exit code 2) if it is outside the root, inside state_dir or build_dir, or is a secret file (", SECRET_FILES!(), "), because the brief goes to a model), or, without one, a brief made from the task's title and claims. It only RECORDS: nothing is rebased, built, reviewed or merged until `queue_run`, and no model is called here. It refuses (exit code 3, nothing queued) unless the task is `running` and has a finished worktree that really is the task's, that worktree is on its branch and CLEAN with ignored files counted (the same rule that guards worktree removal: commit or remove everything first), the branch has at least one commit beyond the task's recorded base, and the task is not already in the queue as a `queued` or `running` entry (a task is queued once at a time; an entry that ended as rejected, check_failed, perf_failed, conflict or merged does not count, so a coder whose work was rejected fixes it, commits, and calls `queue_add` again). It takes a short lock on the queue's entry table (state/queue/entries.lock, waiting at most queue_lock_wait_seconds, then exit code 4); it never waits for a running `queue_run`."),
        params: &[QUEUE_TASK, QUEUE_RISK, QUEUE_CODER, QUEUE_BRIEF],
        outputs: "The new entry's id on the first line, for example `q-20261117-120000`, then one line saying what will happen.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "the queue could not be read or the entry could not be written" },
            ExitDoc { code: "2", meaning: concat!("bad input: risk is not `low` or `high`, the coder is empty, the brief file cannot be read, is not UTF-8 text, is empty, is over reviewer_brief_max_kb KB or is a secret file (", SECRET_FILES!(), "), or there is no task with that id") },
            ExitDoc { code: "3", meaning: "refused, nothing queued: the task is not running or has no usable worktree, the worktree is not on its branch or not clean (ignored files count), the branch has no commit beyond its base, or the task is already queued (the output says which and names the entry)" },
            ExitDoc { code: "4", meaning: "queue busy: the entry-table lock could not be taken in time; repeat the call" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: false,
        cli: "broadsword queue add <task> --risk low|high --coder <model name> [--brief <file>]",
        cli_examples: &["queue add t-1 --risk low --coder sonnet", "queue add t-1 --risk high --coder sonnet --brief brief.md"],
        parse: |a| parse_sub(a, "queue", "add", Some("task"), &[("--risk", "risk", false), ("--coder", "coder", false), ("--brief", "brief", false)], &["task", "risk", "coder", "brief"]),
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["queue".to_string(), "add".into(), i.str("task").unwrap_or_default().into(), "--risk".into(), i.str("risk").unwrap_or_default().into(), "--coder".into(), i.str("coder").unwrap_or_default().into()];
            if let Some(b) = i.str("brief") {
                v.push("--brief".into());
                v.push(b.into());
            }
            v
        },
        handler: |cfg, i| Ok(queue::cmd_add(cfg, i.require("task")?, i.require("risk")?, i.require("coder")?, i.str("brief"))),
};

pub(super) const QUEUE_LIST: Tool = Tool {
        name: "queue_list",
        summary: "List the merge queue: every entry with its status. (read-only)",
        description: "Lists every queue entry, oldest first, with its status: queued (waiting), running (a `queue_run` is on it, or it says `interrupted` when that run died and a later `queue_run` will resume it), approved (merge train only: the review PASSed; it waits for a train, or is a half-finished train merge that `queue_run` resumes first), merged, blocked, conflict, stale, check_failed, perf_failed (the build and tests passed but the perf gate found a bench case slower than its baseline allows, or the baseline or bench could not be used), rejected, merged_check_failed or cancelled; `queue_show` explains one. It only reads entry records and returns immediately; it calls no model.",
        params: &[],
        outputs: "One tab-separated line per entry: id, status, task id, project, risk, coder model, and the reason (or the current step); or `no queue entries`.",
        exit_codes: &[E0, ExitDoc { code: "1", meaning: "an entry record could not be read" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword queue list",
        cli_examples: &["queue list"],
        parse: |a| (a == ["queue", "list"]).then(Inputs::default),
        mcp_callable: true,
        to_args: |_| vec!["queue".into(), "list".into()],
        handler: |cfg, _| Ok(queue::cmd_list(cfg)),
};

pub(super) const QUEUE_SHOW: Tool = Tool {
        name: "queue_show",
        summary: "Show one queue entry in full: status, reason, every step, the check summary and the review findings. (read-only)",
        description: "Shows one queue entry: its status and the reason for it, the task, branch, risk, coder and reviewer model, the attempt number, the commits involved (the trunk's tip at the rebase, the pre-rebase tip and the ref that keeps it, the rebased tip, the reviewed commit, the merge commit), the check summary, the reviewer's verdict with its blocking and minor findings, cost and tokens, the path of the review file that holds the reviewer's full raw output, the brief and the time-stamped log of every step. Use it to see why an entry did not merge and what to fix. It only reads and returns immediately.",
        params: &[QUEUE_ENTRY],
        outputs: "Lines of `key: value` (id, status, reason, task, project, branch, risk, coder, reviewer, attempt, step, times, commits, verdict, review file ...), then the check summary, the findings, the brief and the step log as indented blocks.",
        exit_codes: &[E0, ExitDoc { code: "2", meaning: "no queue entry with that exact id" }],
        read_only: true,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword queue show <entry>",
        cli_examples: &["queue show q-1"],
        parse: |a| parse_sub(a, "queue", "show", Some("entry"), &[], &["entry"]),
        mcp_callable: true,
        to_args: |i| vec!["queue".into(), "show".into(), i.str("entry").unwrap_or_default().into()],
        handler: |cfg, i| Ok(queue::cmd_show(cfg, i.require("entry")?)),
};

pub(super) const QUEUE_RUN: Tool = Tool {
        name: "queue_run",
        summary: "Process one queue entry (or, with `all`, drain the queue in order, merging approved entries in merge trains): rebase onto the trunk (`main` or `master`, which the project's checkout must have checked out), check, review by another model, fast-forward merge into the trunk, check the trunk, finish the task. (destructive, spends money)",
        description: "ALL: with `all`, every runnable entry is run, in order, one after another under ONE hold of the run lock (state/queue/queue.lock; with queue_per_project \"on\", the project's own state/queue/<project>.queue.lock: see PER-PROJECT LOCK) (so no second runner races it and loses with exit 4): interrupted entries first, then the queued and stale ones oldest first (blocked ones are skipped: they wait on a fix). Each entry is taken at most once per call. An entry that ends blocked, conflict, stale, rejected, check_failed or perf_failed does not stop the rest; an internal error (exit 1) or merged_check_failed (exit 6) STOPS the drain. A summary table follows (entry, task, status, exit, seconds, note), and the exit code is the WORST outcome, worst first: 6, 1, 5, 3, 2, 4, 0. MERGE TRAIN, with `all` and when the merge_train_max setting is above 1 (4 as shipped; 1 = off, every entry is then checked, merged and main-checked on its own exactly as described below): every entry is still reviewed on its own diff, brief and gate reviewer (its shadow too), but it does NOT run its own check (the review prompt says so) and stops as `approved` when its review PASSes, with nothing merged. PARALLEL REVIEW (setting review_parallel; 1 = one entry at a time, as before; ignored with the train off): up to review_parallel entries are taken at once and brought to their verdict together: their steps before the review (preconditions, rebase, done check) run one entry after another in queue order, their reviews (gate, then shadow, per entry, same rules, models, timeouts and call cap) side by side, every GPT call (gate or shadow) one at a time so the daily cap is checked per call as before; each entry's verdict is written only to its own entry; the trains, train checks, fast-forwards and main checks after them stay one at a time, in queue order. A call killed mid-review leaves each of those entries `running` at its step, and the next call resumes each. Approved entries of one project whose task claims do not overlap form a TRAIN of up to merge_train_max entries (a lone one at the end of the drain too; and a SHORT train, even of 1, as soon as no entry of that project is left for the drain to run, or its oldest approved entry has waited merge_train_wait_minutes, so one project's approved entries never wait behind another project's stream): their branches are chained one on top of the other with a rebase (a textual conflict sends that entry back to `queued`; so does a rebase that changed the entry's patches, compared by `git patch-id --stable` with the reviewed commits: it is reviewed again, never merged on the old verdict), ONE check runs on the chain tip, main is fast-forwarded ONCE to it (every entry's commits kept in order, no squash) and checked ONCE, and every entry is `merged`, recording its train id, position and the shared check job. If the combined check fails it is BISECTED (the check of the first i entries, by halves): an entry that fails on its own is `check_failed` with the log of that check (`perf_failed` when it was the perf gate that failed), the entries proven good before it merge as a smaller train and the entries after it start a new round on the new main; if the failing entry passes on its own, the failure only shows combined: the failing entry and every entry before it in the chain are held out and none of them merges (main unchanged), the first of them is `conflict`, the ones chained on it are `blocked` with the left-train reason below. If the main check fails after a train merge the exit is 6, every entry of that merge is `merged_check_failed`, main is NOT reset and the drain stops. A check that cannot run to a verdict leaves the train's entries `approved` (the next call takes them again) and stops the drain, unless the reason is one entry's own worktree: then only that entry ends `blocked`, the rest stay `approved` and the drain goes on. TRAINS FORM ONLY UNDER `--all`; `merge_train_max` sets the most entries one train takes. SHORT-TRAIN RULE: a project with no entry left to run, or whose oldest approved entry has waited merge_train_wait_minutes (20 as shipped), gets a train now, even of 1, so its approved entries never wait behind another project's stream. An approved entry that is run on its own (without `all`) is checked first and then merged as a single entry, with one exception: a HALF-FINISHED TRAIN MERGE, an `approved` entry that a train already took into its merge (its step is merge, main_check or done, or it has a main_sha), whose commit a train may already have fast-forwarded into main. It is RESUMED FIRST, by `queue_run` with or without `all`, before any other entry and at its recorded step: the fast-forward is recorded as done (not repeated), only the check of the merged main and `task done` remain, and it is finished as `merged`; it never joins a new train and is not checked alone again. `all` starts no train while one is being finished, and `queue_cancel` refuses it (\"is a half-finished merge ... Not cancelled; finish it with `broadsword queue run --all` or `broadsword queue run --entry <id>`\": main may already hold its commits). AN ENTRY THAT LEAVES A TRAIN WHILE ITS BRANCH IS STILL CHAINED on commits of other entries of a train that are not on main ends `blocked` with a reason that starts `left a train: rebase your own commits onto main and queue again.` and names `git rebase --onto <main> <base> <branch>`. The summary table gets a `train` column when a train ran. WITHOUT `all`: takes the oldest entry that is interrupted (`running` with no process behind it), else the oldest `queued` or `stale` one (or the one named by `entry`: queued, running-but-interrupted, stale or blocked) and runs it through the pipeline, one entry at a time (an OS file lock, state/queue/queue.lock; a second call waits queue_lock_wait_seconds, then exits 4). PER-PROJECT LOCK (setting queue_per_project, \"off\" as shipped; \"off\" = the one global state/queue/queue.lock above, exactly as before, and `project` then only narrows which entries a run takes): with \"on\", a run holds only its project's own run lock, state/queue/<project>.queue.lock (the same OS file lock, freed by the operating system when its holder ends however it ends, and the same queue_lock_wait_seconds), so runs of DIFFERENT projects (`queue run --all --project <p>`) go at the same time, while a second run of the SAME project exits 4 with a `queue busy:` message that names the project (\"another `queue run` of project `<p>`\") and the holder's pid when it can be read; `project` (or `entry`, whose own project is then used) is REQUIRED, without either the call exits 2 and runs nothing, because a run of no named project cannot tell another project's live entries from interrupted ones. The entries lock (state/queue/entries.lock, milliseconds) stays ONE for all projects, and merge trains stay within one project. Do not switch the setting while a `queue run` is going: a run started under the other value does not see its lock. STEPS, each recorded in the entry before it is relied on: (1) preconditions: the task is still running, the project's trunk (the one of the local branches `main` and `master` that exists; both or neither is an error) can be told and is what the main checkout of the project has checked out (a feature branch or a detached HEAD there ends the entry `blocked`, and the checkout is not touched, moved or merged into), that checkout is clean (`git status --porcelain`), the task's worktree is clean (uncommitted and untracked changes; ignored files such as an ignored .env do not count, they only make `task done` keep the worktree), the reviewer model for the risk differs from the entry's coder model, and the branch has a commit main lacks; if one fails the entry is `blocked` and nothing was changed. (2) rebase: `git rebase <trunk tip>` (the tip of `refs/heads/<trunk>`, never HEAD) inside the task's worktree, skipped if the branch already contains that tip; the pre-rebase tip is first saved in the entry and, create-only, under refs/broadsword/queue/<entry>/pre-rebase; on a conflict the rebase is ABORTED, the branch tip is verified to be exactly what it was, and the entry is `conflict` naming the files. (3) check: `check <project> --task <id>` (the same job as the `check` tool: disk guard, build lock, notes, log); if it fails the entry is `check_failed` and NO model is asked. PERF GATE (setting perf_gate, \"on\" as shipped; part of every check, the single entry's and every train check, bisect and own check alike): once the build and tests pass, the entry's own project is benched on the same commit when it has a baseline state/perf/<project>-baseline.json (a project without one is skipped and the skip is logged; no other project is ever benched): `cargo run --release --offline --example bench -- --runs <runs_per_case from the baseline>` runs perf_runs times (5 as shipped) as `run` jobs in the task's worktree, each holding the build lock; a case whose median of the runs' medians is slower than the baseline by more than that case's own threshold_pct (from the baseline file) makes the entry `perf_failed` (exit 5, NO model is asked), with each case's baseline, now and change in % in the reason, and the output says whether the baseline is LOADED. FAIL CLOSED: a baseline file that exists but cannot be read or lacks a needed field, a bench that exits non-zero, and output with a missing or invalid case are `perf_failed` too, never a pass; a bench job that never ran (refused) gives no verdict: `blocked`. With a merge train the gate runs ONCE on the chain tip, with its check, and a perf failure is bisected exactly like a failed check. (4) review: the reviewer model (reviewer_model_low or reviewer_model_high by risk, through the reviewer_backend setting, `claude` as shipped = the Claude Code CLI run headless, one turn, no tools, in a folder under scratch with no CLAUDE.md, with every environment variable named *_API_KEY, *_KEY, *_TOKEN, *_SECRET, *_PAT, *_PASSWORD, *_PASS, *_CREDENTIAL* or *_AUTH* removed, and every one whose value equals or contains ANY value of the root's .env, whatever its name) gets a fixed reviewer prompt, the brief, the check summary and the end of its output, and the whole diff <trunk tip at the rebase>...the checked commit (never cut: a diff over reviewer_diff_max_kb makes the entry `blocked`); EVERY value of the root's .env (redact_min_chars characters or longer, 6 as shipped; shorter ones are only kept out of the environment) and every secret-named variable's value are replaced by [REDACTED] first, in the prompt and in everything stored in the entry and review file. Limits: a secret split across two diff lines is not redacted, and a .env value that is also ordinary code (REGION=return) is blanked everywhere, hiding that code from the reviewer, so keep .env values distinctive. The check, review and merge are bound to ONE commit: the one the check passed on; a branch that moved after the check makes the entry `stale` (\"branch changed after check\") and nothing is reviewed or merged. THIS CALLS A MODEL: it costs money (about $0.006 for haiku, $0.18 for opus per the benchmark) and uses the network; the CLI program is the reviewer_cli setting, or the env var BROADSWORD_REVIEWER_CLI (a machine-specific path); the RESOLVED PATH is printed and recorded in the entry and the review file every time, the override must point OUTSIDE the project root (like the real Claude install) or inside the root's scratch_dir (a TEST HOOK: it prints `TEST HOOK ACTIVE: reviewer CLI is <path>` and the review is marked test_hook: true), and any other path inside the root, build/ included, is refused; the answer must be the CLI's JSON and its modelUsage must name the requested model family (haiku, sonnet, opus), else it is a FAIL; at most reviewer_max_calls_per_entry calls are made per entry (then `blocked`, \"review call cap reached\"); a call longer than reviewer_timeout_seconds is killed and the entry is `blocked` (no verdict). The answer must follow `VERDICT: PASS|FAIL`, `BLOCKING:` lines and `MINOR:` lines; an answer that does not is a FAIL, and a PASS needs an explicit `BLOCKING: none` and no blocking finding (an empty `BLOCKING:` header followed by findings is a FAIL). The full raw output, cost and tokens are saved in state/queue/<entry>.review. A FAIL makes the entry `rejected`: the task stays running and its worktree is kept so the coder can fix, commit and `queue_add` again. (5) merge, on PASS only (a review made by a TEST HOOK reviewer never merges into a project whose folder is outside scratch_dir: the entry is `blocked`; the reviewer CLI's resolved path is printed and logged at the merge; LIMIT: an override that points outside the root is trusted as a real reviewer, because nothing can prove a program there is not a stand-in; a resumed entry whose recorded verdict is not PASS ends `rejected`, and one whose PASS does not name the checked commit ends `stale`): `git merge --ff-only` of the exact reviewed commit in the main checkout, and only while that checkout has the trunk checked out (never a merge commit, never into another branch); if the trunk or the branch moved since the rebase the entry is `stale` and nothing is merged (call again to rebase again). (6) `check <project>` in the main checkout; it is REFUSED (the entry is `merged_check_failed`, naming why) unless that checkout still has the trunk checked out and the trunk holds the merged commit, and if it fails, which should not happen, the trunk is NOT reset: the entry is `merged_check_failed` with a loud message naming the commit, and a human decides. (7) `task done`, which removes the worktree and branch under its usual safety rules; the entry is `merged` with main's commit. NEVER uses --force, -D, reset --hard or push. NOT IDEMPOTENT, DESTRUCTIVE (it rewrites the task branch, moves main's branch forward and removes the worktree), and LONG-RUNNING (a check plus a review take minutes; over MCP the call blocks and is killed after the mcp_call_timeout_seconds setting, 1800 s as shipped, leaving the entry `running` for the next call to resume). CRASH AND RESUME: every step is recorded, so if the process dies the entry stays `running`; the next `queue_run` resumes it from the recorded step (a half-done rebase is aborted first, a recorded verdict is not asked for again, a fast-forward that already happened is recorded, not repeated). The task-board lock is never held during any of this; `task done` takes it for a moment. Every outcome except `merged` exits non-zero with a one-line reason and the entry id on stderr. SHADOW REVIEW (when `shadow_reviewer` is `openai`; `off` disables it): after the gating review is recorded, the SAME prompt goes once to a second model (`shadow_reviewer_model`, run as `python -I -B clients/openai_review.py`, key from the root .env, `shadow_reviewer_timeout_seconds`). Its answer is only RECORDED in the entry (`shadow_verdict` PASS, FAIL, ERROR or SKIPPED, `shadow_blocking`, `shadow_model_used`, `shadow_cost_usd`, `shadow_tokens`, `shadow_secs`) and in state/queue/<id>.shadow.review; it NEVER changes the outcome: the gating review alone decides the merge, and a shadow timeout or error is recorded as ERROR and the merge goes on. SKIPPED when the coder is a GPT model. The rule that no reviewer program may live inside the project root does not apply to the shadow, because it never gates a merge.",
        params: &[QUEUE_RUN_ENTRY, QUEUE_RUN_ALL, QUEUE_RUN_PROJECT],
        outputs: "Progress lines `queue entry <id>: step <name>`, the check job's own output, then one final line: `queue entry <id>: merged: ...` on stdout, or `broadsword: queue entry <id> is <status>: <reason>` on stderr (`broadsword: LOUD: ...` for merged_check_failed). The reviewer's full answer is in state/queue/<id>.review (see `queue_show`). `the queue is empty` and exit 0 when there is nothing to run. With `all`: each entry's own lines, `merge train <id>: ...` lines for every train (its size, the chain, the check, a bisect with its extra checks and seconds), then `--- queue run --all: N entries in S s ---`, a tab-separated table `entry task status exit seconds note` (plus `train`, as `<train id>#<position>`, when a train ran), and `worst exit: N`; a stopped drain says why on stderr.",
        exit_codes: &[
            ExitDoc { code: "0", meaning: "merged (the entry is `merged`), or the queue is empty" },
            ExitDoc { code: "1", meaning: "internal error (git or a record could not be read or written, a verification failed): the entry stays `running` at its step and the next call resumes it; or the entry record could not be written" },
            ExitDoc { code: "2", meaning: "bad input: no entry with that id, or the entry is finished (merged, rejected, check_failed, perf_failed, conflict, merged_check_failed, cancelled) and cannot be run; or `entry` was given with `all`; or `project` is not a project, or `entry` is not of that project; or queue_per_project is \"on\" and neither `project` nor `entry` was given; nothing was run" },
            ExitDoc { code: "3", meaning: "not merged, retryable: the entry is `blocked` (a precondition failed, the reviewer model equals the coder, the diff is too big, the check could not run, the reviewer could not be used or timed out), `conflict` (the rebase conflicted and was aborted) or `stale` (main or the branch moved); the reason names the fix" },
            ExitDoc { code: "4", meaning: "queue busy: another `queue_run` holds the run lock (state/queue/queue.lock; with queue_per_project \"on\", that project's state/queue/<project>.queue.lock, and the message names the project) for longer than queue_lock_wait_seconds; nothing was changed" },
            ExitDoc { code: "5", meaning: "not merged, the code needs changes: the entry is `rejected` (the reviewer said FAIL, or gave no usable verdict) or `check_failed` (build or tests failed on the rebased branch) or `perf_failed` (the perf gate failed)" },
            ExitDoc { code: "6", meaning: "LOUD: the commit IS on main but `check` failed there (`merged_check_failed`); main was not reset, a human decides" },
        ],
        read_only: false,
        long_running: true,
        changes_files: true,
        destructive: true,
        idempotent: false,
        cli: "broadsword queue run [--entry <id> | --all] [--project <p>]",
        cli_examples: &["queue run", "queue run --entry q-1", "queue run --all", "queue run --all --project crash-sim", "queue run --project crash-sim"],
        parse: |a| {
            // `--entry <id>` once, `--all` once, `--project <p>` once; inputs in parameter order (entry, all, project). The handler
            // refuses entry with all, so every combination still round-trips.
            let rest = match a {
                ["queue", "run", rest @ ..] => rest,
                _ => return None,
            };
            let (mut entry, mut all, mut project): (Option<&str>, bool, Option<&str>) = (None, false, None);
            let mut it = rest.iter().copied();
            while let Some(w) = it.next() {
                match w {
                    "--entry" if entry.is_none() => entry = Some(it.next()?),
                    "--all" if !all => all = true,
                    "--project" if project.is_none() => project = Some(it.next()?),
                    _ => return None,
                }
            }
            let mut i = Inputs::default();
            if let Some(e) = entry {
                i = i.with("entry", Val::Str(e.into()));
            }
            if all {
                i = i.with("all", Val::Bool(true));
            }
            if let Some(p) = project {
                i = i.with("project", Val::Str(p.into()));
            }
            Some(i)
        },
        mcp_callable: true,
        to_args: |i| {
            let mut v = vec!["queue".to_string(), "run".into()];
            if let Some(e) = i.str("entry") {
                v.push("--entry".into());
                v.push(e.into());
            }
            if i.bool("all") {
                v.push("--all".into());
            }
            if let Some(p) = i.str("project") {
                v.push("--project".into());
                v.push(p.into());
            }
            v
        },
        handler: |cfg, i| match (i.str("entry"), i.bool("all")) {
            (Some(_), true) => {
                eprintln!("broadsword: give `entry` (run that one entry) or `all` (drain the queue), not both; nothing was run");
                Ok(ExitCode::from(queue::EXIT_BAD_INPUT))
            }
            (_, true) => Ok(queue::cmd_run_all(cfg, i.str("project"))),
            (e, false) => Ok(queue::cmd_run(cfg, e, i.str("project"))),
        },
};

pub(super) const QUEUE_CANCEL: Tool = Tool {
        name: "queue_cancel",
        summary: "Drop an entry from the merge queue; nothing else is touched.",
        description: "Marks a `queued`, `blocked`, `stale` or interrupted `running` entry as `cancelled`, so `queue_run` no longer picks it. The task, its worktree and branch, and main are not touched, with one exception: for an interrupted run that died in the middle of its rebase, the half-done rebase left in the worktree is aborted (and the branch verified to be back at its pre-rebase tip), because a worktree stuck in a rebase is unusable. An interrupted entry that already reached the merge step is refused: main may already hold the commit, so resume it with `queue_run` instead. An entry that is running right now in another process cannot be cancelled (the call waits for the run lock that guards the entry, state/queue/queue.lock, or with the queue_per_project setting \"on\" the entry's project's state/queue/<project>.queue.lock, then exits 4). An entry that is already cancelled succeeds without change; one that is finished (merged, rejected, check_failed, perf_failed, conflict, merged_check_failed) is refused. The record is kept for good.",
        params: &[QUEUE_ENTRY],
        outputs: "`queue entry <id> is cancelled; nothing else was touched` (plus a note when a leftover rebase was aborted), or `queue entry <id> was already cancelled`.",
        exit_codes: &[
            E0,
            ExitDoc { code: "1", meaning: "the entry record could not be written, or a leftover rebase could not be aborted or did not restore the tip, or a running entry's project is not a plain name so no run lock can guard it (the entry was not cancelled)" },
            ExitDoc { code: "2", meaning: "no queue entry with that exact id" },
            ExitDoc { code: "3", meaning: "not cancelled: the entry is finished, or was interrupted at or after the merge step" },
            ExitDoc { code: "4", meaning: "queue busy: a lock could not be taken in time (for a running entry: its run lock, which names the project when queue_per_project is \"on\"); repeat the call" },
        ],
        read_only: false,
        long_running: false,
        changes_files: false,
        destructive: false,
        idempotent: true,
        cli: "broadsword queue cancel <entry>",
        cli_examples: &["queue cancel q-1"],
        parse: |a| parse_sub(a, "queue", "cancel", Some("entry"), &[], &["entry"]),
        mcp_callable: true,
        to_args: |i| vec!["queue".into(), "cancel".into(), i.str("entry").unwrap_or_default().into()],
        handler: |cfg, i| Ok(queue::cmd_cancel(cfg, i.require("entry")?)),
};

const QUEUE_TASK: Param = Param { name: "task", ty: Ty::String, required: true, description: concat!("The id of the task whose branch `task/<id>` is queued (see `task_list`). It must be `running` with a clean worktree and at least one commit beyond its base.", NO_DASH!()) };
const QUEUE_RISK: Param = Param {
    name: "risk",
    ty: Ty::String,
    required: true,
    description: concat!("`low` or `high`: picks the reviewer model (the reviewer_model_low or reviewer_model_high setting). Use `high` for deleting or rewriting files, locks, secrets, or anything that runs before or instead of a safety check; `low` for the rest. Anything else fails with exit code 2.", NO_DASH!()),
};
const QUEUE_CODER: Param = Param { name: "coder", ty: Ty::String, required: true, description: concat!("The model name that wrote the code, for example `sonnet`. The reviewer model must differ from it (compared case-insensitively), or `queue_run` blocks the entry.", NO_DASH!()) };
const QUEUE_BRIEF: Param = Param {
    name: "brief",
    ty: Ty::String,
    required: false,
    description: concat!("Path of a UTF-8 text file with the brief the reviewer gets: what the task is and its done check. At most reviewer_brief_max_kb KB; a secret file (", SECRET_FILES!(), ") is refused. Default: a brief made from the task's title and claims.", NO_DASH!()),
};
const QUEUE_ENTRY: Param = Param { name: "entry", ty: Ty::String, required: true, description: concat!("The exact queue entry id, as printed by `queue_add` and shown in the first column of `queue_list`.", NO_DASH!()) };
const QUEUE_RUN_ALL: Param = Param {
    name: "all",
    ty: Ty::Boolean,
    required: false,
    description: "Drain the queue: run every runnable entry in order (interrupted first, then queued and stale ones oldest first) under one hold of the run lock (with queue_per_project \"on\": the project's own, and `project` is required), each at most once, then print a summary table; the exit code is the worst outcome. With merge_train_max above 1 (4 as shipped), approved entries with non-overlapping claims are merged together by ONE check of their chained tip, one fast-forward and one main check (bisected when the check fails; see the main description). A project with no entry left to run, or whose oldest approved entry has waited merge_train_wait_minutes, gets a train now, even of 1. Not together with `entry`. Default false.",
};

const QUEUE_RUN_PROJECT: Param = Param {
    name: "project",
    ty: Ty::String,
    required: false,
    description: concat!("Take only this project's entries (with `all`: drain only this project). It must be a project (see the `projects` tool), else exit code 2 and nothing runs; with `entry`, that entry must be of this project (else exit code 2). When the queue_per_project setting is \"on\" (\"off\" as shipped), every project has its own run lock, state/queue/<project>.queue.lock, so runs of different projects go at the same time, and `project` (or `entry`) is REQUIRED: without either, exit code 2 and nothing runs. When it is \"off\", every run holds the one state/queue/queue.lock as before and `project` only narrows which entries the run takes.", NO_DASH!()),
};

const QUEUE_RUN_ENTRY: Param = Param {
    name: "entry",
    ty: Ty::String,
    required: false,
    description: concat!("Run this entry (it must be queued, interrupted, stale or blocked) instead of the oldest one. Omit it to take the oldest interrupted entry, else the oldest queued or stale one. An unknown id or a finished entry fails with exit code 2.", NO_DASH!()),
};

