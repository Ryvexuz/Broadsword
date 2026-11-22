use crate::config::Config;
use crate::job;
use crate::record::Record;
use crate::task::{self, fail, now_secs};
use crate::{sys, wtgit};
use std::path::PathBuf;
use std::process::ExitCode;
use super::check::{PassInfo, TrainCheck};
use super::entry::{half_merged, load_entries, load_entry, resume_first};
use super::locks::{entries_lock, hold_gate, Lane, lane_for, run_lock};
use super::merge::{green_revert, merge_members, trunk_checkout, trunk_name};
use super::{APPROVED, BLOCKED, CHECK_FAILED, CONFLICT, EXIT_BAD_INPUT, EXIT_ERROR, EXIT_MERGED_BAD, EXIT_REJECTED, EXIT_RETRY, PERF_FAILED, QUEUED, R, REJECTED, Row, Run, RUNNING, STALE, STEP_MERGE, STEPS};
use super::rebase::{chain_bases_of, chained_base, left_train_text};
use super::review::SHADOW_KEYS;
use super::view::{short, some};

/// An exit code that stops `queue run --all`: an internal error, or merged_check_failed unless its revert left main green.
fn stops_drain(cfg: &Config, id: &str, code: u8) -> bool {
    code == EXIT_ERROR || (code == EXIT_MERGED_BAD && !green_revert(cfg, id))
}

// ---- run --------------------------------------------------------------------------------

/// `queue run [--entry <id>] [--project <p>]`: one entry. `project` limits the choice to that project's entries (see `run_lock` for
/// the lock it takes).
pub fn cmd_run(cfg: &Config, entry: Option<&str>, project: Option<&str>) -> ExitCode {
    let lane = match lane_for(cfg, entry, project).and_then(|k| run_lock(cfg, k)) {
        Ok(l) => l,
        Err(c) => return c,
    };
    if let Err(c) = hold_gate(cfg, &lane) {
        return c;
    }
    // Choose and claim the entry under the short lock. Holding the lane's run lock already means no other run of the lane exists,
    // so every `running` entry of the lane is an interrupted one.
    let chosen = claim(cfg, |all| match entry {
        Some(id) => match all.iter().find(|x| x.id() == id) {
            Some(x) if !lane.holds(x) => Err(fail(EXIT_BAD_INPUT, format!("queue entry {id} is of project `{}`, outside this run's project; nothing was run", x.get("project")))),
            Some(x) if matches!(x.status(), QUEUED | RUNNING | STALE | BLOCKED | APPROVED) => Ok(Some(x.clone())),
            Some(x) => Err(not_runnable(x)),
            None => Err(fail(EXIT_BAD_INPUT, format!("no queue entry with the exact id `{id}` (see `broadsword queue list`)"))),
        },
        None => {
            let mine: Vec<&Record> = all.iter().filter(|x| lane.holds(x)).collect();
            Ok(mine.iter().find(|x| resume_first(x)).or_else(|| mine.iter().find(|x| matches!(x.status(), QUEUED | STALE))).or_else(|| mine.iter().find(|x| x.status() == APPROVED)).map(|x| (*x).clone()))
        }
    });
    let e = match chosen {
        Ok(Some(e)) => e,
        Ok(None) => {
            println!("the queue is empty{}: nothing is queued, interrupted or stale", in_project(&lane));
            return ExitCode::SUCCESS;
        }
        Err(c) => return c,
    };
    match begin(cfg, e) {
        Ok(mut run) => run.go(),
        Err((_, code)) => code,
    }
}

/// ` for project <p>` when the lane is one project, for the "queue is empty" line.
fn in_project(lane: &Lane) -> String {
    lane.key.as_ref().map(|k| format!(" for project `{k}`")).unwrap_or_default()
}

/// `queue run` refuses a finished entry: exit 2 with the way on.
fn not_runnable(x: &Record) -> ExitCode {
    let hint = match x.status() {
        REJECTED | CHECK_FAILED | PERF_FAILED | CONFLICT => "fix the code, commit, and queue the task again with `queue add`",
        _ => "it is finished",
    };
    fail(EXIT_BAD_INPUT, format!("queue entry {} is `{}` and cannot be run; {hint}", x.id(), x.status()))
}

/// Under entries.lock: let `pick` choose an entry from all of them (oldest first) and CLAIM it: a fresh attempt (anything but an
/// interrupted `running` entry or an `approved` one) forgets what the last attempt learned (its log and review files stay) and becomes
/// `running`; the claimer's pid is recorded either way. The caller holds queue.lock.
fn claim(cfg: &Config, pick: impl FnOnce(&[Record]) -> Result<Option<Record>, ExitCode>) -> Result<Option<Record>, ExitCode> {
    let _l = entries_lock(cfg)?;
    let all = load_entries(cfg).map_err(|m| fail(EXIT_ERROR, m))?;
    let Some(mut e) = pick(&all)? else { return Ok(None) };
    // An `approved` entry (review PASSed, waiting for a merge train) that is run on its own is RESUMED at its step `review`, like an
    // interrupted one: its verdict stands, nothing is cleared (a deferred check is made before it can merge: see `merge_phase`).
    let resumed = matches!(e.status(), RUNNING | APPROVED);
    if e.status() == APPROVED {
        e.set("status", RUNNING);
    }
    if !resumed {
        // a fresh attempt: forget what the last attempt learned (its log and review file stay)
        for k in [
            "train", "train_pos", "train_job", "train_up", "train_onto", "train_checked", "train_bisect", "alone_tip", "reviewed_orig_tip", "check_deferred", "main_check", "step", "reason", "error", "pre_rebase", "pre_rebase_ref", "base_head", "rebased_tip", "checked_tip", "reviewed_tip", "reviewer_cli", "test_hook", "check_job", "check_exit", "check_summary", "check_tail", "perf", "done_check", "done_check_file",
            "reviewer_model", "gate", "gate_model_used", "verdict", "blocking", "minor", "cost_usd", "tokens_in", "tokens_out", "review_file", "main_sha", "main_branch", "ended", "pre_merge_sha", "revert_sha", "revert_check",
        ] {
            e.set(k, "");
        }
        for k in SHADOW_KEYS {
            e.set(k, "");
        }
        e.set("attempt", (e.get_num("attempt").unwrap_or(0) + 1).to_string());
        e.set("status", RUNNING);
    }
    let me = std::process::id();
    e.set("pid", me.to_string());
    e.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
    e.set("started", now_secs().to_string());
    e.save().map_err(|err| fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display())))?;
    Ok(Some(e))
}

/// A claimed entry, ready to run: its project resolved (an unknown project ends it `blocked`: `Err((id, exit code))`).
fn begin(cfg: &Config, mut e: Record) -> Result<Run<'_>, (String, ExitCode)> {
    let resumed = !e.get("step").is_empty();
    let dir = match job::resolve_project(&cfg.root, &cfg.projects_dir, e.get("project")) {
        Ok(d) => d,
        Err(m) => {
            let id = e.id().to_string();
            let mut run = Run::new(cfg, e, PathBuf::new());
            return Err((id, run.end(BLOCKED, &format!("the project is unknown: {m}"), EXIT_RETRY)));
        }
    };
    println!("queue entry {}: {} (task {}, project {}, risk {}, coder {}, attempt {})", e.id(), if resumed { "RESUMING after an interrupted run, from step ".to_string() + e.get("step") } else { "starting".into() }, e.get("task"), e.get("project"), e.get("risk"), e.get("coder"), e.get("attempt"));
    e.set("error", "");
    Ok(Run::new(cfg, e, dir))
}

// ---- run --all ----------------------------------------------------------------------------
//
// TP4 SEAM: this is the MINIMAL drain the merge train needs (tp4, t-20261118-095351, is not merged at this base). tp4's own
// `cmd_run_all` adds check-ahead (`queue_check_ahead`) and `--blocked`; when tp4 is ported onto this, it keeps the train hooks marked
// "MERGE TRAIN" below (`approved`, `run_trains`, `settle_approved`, the `train` column) and adds its own parts around them.

/// The exit code a `queue run` returned, as a number (ExitCode hides it).
pub(super) fn code_num(c: ExitCode) -> u8 {
    (0..=u8::MAX).find(|n| ExitCode::from(*n) == c).unwrap_or(EXIT_ERROR)
}

/// How bad an outcome is, for "the exit code of `--all` is the worst outcome": merged_check_failed (6) is worst, then an internal
/// error (1), rejected / check_failed (5), blocked / conflict / stale (3), bad input (2), busy (4), merged (0).
fn badness(code: u8) -> u8 {
    match code {
        6 => 7,
        1 => 6,
        5 => 5,
        3 => 4,
        2 => 3,
        4 => 2,
        0 => 0,
        _ => 6,
    }
}

/// The next entry `--all` takes: an interrupted one first, then the oldest queued or stale one; never one this drain already took
/// (`taken`), so an entry that ends stale or blocked is not run again in a loop. An `approved` entry (review PASSed, not merged) is
/// picked last, and only when the merge train is off (`train` false): with the train on, the drain itself collects the approved
/// entries into trains (`take_group`).
fn next_for_all(all: &[Record], taken: &[String], train: bool, lane: &Lane) -> Option<Record> {
    let fresh = |x: &&Record| lane.holds(x) && !taken.iter().any(|t| t == x.id());
    all.iter()
        .filter(fresh)
        .find(|x| resume_first(x))
        .or_else(|| all.iter().filter(fresh).find(|x| matches!(x.status(), QUEUED | STALE)))
        .or_else(|| if train { None } else { all.iter().filter(fresh).find(|x| x.status() == APPROVED) })
        .cloned()
}

/// The exit code of `queue run --all`: the worst (see `badness`) of every row's code and of `stop`, the code that stopped the drain (0
/// when it ran to its end). A stop is counted even when no row carries it, so "the check could not give a verdict" (exit 1) is never
/// reported as the exit 3 of the rows it left `approved`.
fn worst_exit(rows: impl Iterator<Item = u8>, stop: u8) -> u8 {
    rows.chain(std::iter::once(stop)).max_by_key(|c| badness(*c)).unwrap_or(0)
}

/// `queue run --all`: every runnable entry, in order, under ONE hold of queue.lock (so no second runner can race it and lose with
/// exit 4): interrupted entries first, then queued and stale ones oldest first. Each entry is taken at most once per drain. An entry
/// that ends blocked, conflict, stale, rejected or check_failed does not stop the rest; an internal error (exit 1: the entry stays
/// `running`) and merged_check_failed (exit 6: main holds a commit whose check failed) STOP the drain, because what comes next would
/// build on a main nobody has looked at. Prints a summary table; the exit code is the worst outcome (see `badness`).
///
/// MERGE TRAIN (merge_train_max > 1): an entry stops at `approved` after its review PASSed (its own check deferred), and approved
/// entries are merged in trains (see `run_train`): whenever a full train is ready, whenever a project's short train is due
/// (`short_train_why`: none of its entries left to run, or its oldest approved entry waited merge_train_wait_minutes), and at the
/// end of the drain for whatever is left.
///
/// `project` (`--project <p>`): only that project's entries are drained (with queue_per_project on it is required, and the run holds
/// only that project's lock: see `run_lock`).
pub fn cmd_run_all(cfg: &Config, project: Option<&str>) -> ExitCode {
    let lane = match lane_for(cfg, None, project).and_then(|k| run_lock(cfg, k)) {
        Ok(l) => l,
        Err(c) => return c,
    };
    if let Err(c) = hold_gate(cfg, &lane) {
        return c;
    }
    let train = cfg.merge_train_max > 1;
    if !train && cfg.review_parallel > 1 {
        // PARALLEL REVIEW needs the merge train: without it each entry merges on the main it was rebased on, so a batch's later
        // entries would end stale after the first merge and be reviewed again (see `parallel_ok`, `s_merge`)
        println!("queue run --all: parallel reviews are off because the merge train is off (review_parallel = {}, merge_train_max = {}): entries are reviewed one at a time", cfg.review_parallel, cfg.merge_train_max);
    }
    let t0 = std::time::Instant::now();
    let mut taken: Vec<String> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut stopped = String::new();
    // the exit code that stopped the drain (1 or 6), which may be on no row (a train check without a verdict leaves its entries
    // `approved` with exit 3 rows): it counts for the drain's exit code (`worst_exit`)
    let mut stop_code = 0u8;
    let mut approved: Vec<(Run, std::time::Instant)> = Vec::new();
    if train {
        // entries an earlier drain approved and then died before merging them: they join the trains first (they are the oldest)
        let left: Vec<Record> = match claim_approved(cfg, &lane) {
            Ok(l) => l,
            Err(c) => return c,
        };
        for e in left {
            let t = std::time::Instant::now();
            taken.push(e.id().to_string());
            match begin(cfg, e) {
                Ok(mut r) => {
                    r.defer_check = true;
                    r.load_claims();
                    approved.push((r, t));
                }
                Err((id, c)) => rows.push(row_for(cfg, &id, code_num(c), t)),
            }
        }
    }
    loop {
        let e = match claim(cfg, |all| Ok(next_for_all(all, &taken, train, &lane))) {
            Ok(Some(e)) => e,
            Ok(None) => {
                // MERGE TRAIN: nothing else to run: every approved entry left (even a single one) goes into a last train
                if train && !approved.is_empty() {
                    let code = run_trains(cfg, &mut approved, true, &taken, &mut rows);
                    if code == EXIT_ERROR || code == EXIT_MERGED_BAD {
                        stopped = train_stop_note(code);
                        stop_code = code;
                    }
                    settle_approved(cfg, &mut approved, &mut rows);
                }
                break;
            }
            Err(c) => {
                stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
                rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
                break;
            }
        };
        taken.push(e.id().to_string());
        // PARALLEL REVIEW (review_parallel > 1, merge train on): more entries are claimed now, each exactly as the next turn of this
        // loop would claim it, as long as each is one a train member's worker may take (`parallel_ok`). With review_parallel = 1, or
        // when only one entry is ready, the batch is this one entry and everything below runs as it always did.
        let mut batch = vec![e];
        let mut claim_err = None;
        if train && cfg.review_parallel > 1 && parallel_ok(&batch[0]) {
            while (batch.len() as u64) < cfg.review_parallel {
                match claim(cfg, |all| Ok(next_for_all(all, &taken, train, &lane).filter(parallel_ok))) {
                    Ok(Some(x)) => {
                        taken.push(x.id().to_string());
                        batch.push(x);
                    }
                    Ok(None) => break,
                    Err(c) => {
                        claim_err = Some(c);
                        break;
                    }
                }
            }
        }
        let mut slots: Vec<Slot> = Vec::new();
        for e in batch {
            let t = std::time::Instant::now();
            match begin(cfg, e) {
                Ok(mut r) => {
                    r.defer_check = train;
                    let start = r.start_step();
                    slots.push(Slot::Run(r, t, start, 0));
                }
                Err((id, c)) => slots.push(Slot::Ended(id, code_num(c), t)),
            }
        }
        drive_all(&mut slots, train);
        // From here on everything is serial and in queue order, exactly as for one entry: the summary rows, the approved list, then
        // the trains (and so every train check, fast-forward and main check).
        let mut codes: Vec<(String, u8, usize)> = Vec::new();
        for slot in slots {
            match slot {
                Slot::Ended(id, code, t) => rows.push(row_for(cfg, &id, code, t)),
                Slot::Run(run, t, start, code) => {
                    let id = run.e.id().to_string();
                    if run.e.status() == APPROVED {
                        // approved entries keep their Run until their train has finished with them
                        approved.push((run, t));
                    } else {
                        rows.push(row_for(cfg, &id, code, t));
                    }
                    codes.push((id, code, start));
                }
            }
        }
        // a merged_check_failed whose automatic revert left main green (`green_revert`) does not stop the drain: the next entry is
        // rebased onto that green main, and the drain's exit code still counts the 6 (`worst_exit`)
        let Some(stop_at) = codes.iter().position(|(id, c, _)| stops_drain(cfg, id, *c)).or(if codes.is_empty() { None } else { Some(codes.len() - 1) }) else {
            if let Some(c) = claim_err {
                stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
                rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
                break;
            }
            continue; // every entry of the batch ended before it ran (unknown project): as before, no train for them
        };
        let (id, code, start) = codes[stop_at].clone();
        let mut train_code = 0u8;
        // no new train while a half-finished merge is being finished (`resume_first` entries come first): a train only starts once
        // main's last merge has had its main check
        let stop = stops_drain(cfg, &id, code);
        if train && start < STEP_MERGE && !approved.is_empty() && !stop {
            train_code = run_trains(cfg, &mut approved, false, &taken, &mut rows);
        }
        let hard = if stop { code } else { train_code };
        if hard == EXIT_ERROR || hard == EXIT_MERGED_BAD {
            stop_code = hard;
            stopped = if hard == train_code && train_code != 0 { train_stop_note(hard) } else { format!("stopped after entry {id} (exit {code}): {}", if code == EXIT_ERROR { "it stays `running` at its step; fix the cause, then `queue run` resumes it" } else { "main holds a commit whose check failed; a human decides before anything else merges" }) };
            settle_approved(cfg, &mut approved, &mut rows);
            break;
        }
        if let Some(c) = claim_err {
            stopped = format!("the next entry could not be chosen (exit {})", code_num(c));
            rows.push(Row { id: "-".into(), task: "-".into(), status: "-".into(), code: code_num(c), secs: 0.0, note: stopped.clone(), train: String::new() });
            break;
        }
    }
    let worst = worst_exit(rows.iter().map(|r| r.code), stop_code);
    if rows.is_empty() {
        println!("the queue is empty{}: nothing is queued, interrupted or stale", in_project(&lane));
    } else {
        println!("--- queue run --all{}: {} entr{} in {:.1} s ---", lane.key.as_ref().map(|k| format!(" --project {k}")).unwrap_or_default(), rows.len(), if rows.len() == 1 { "y" } else { "ies" }, t0.elapsed().as_secs_f64());
        // the train column only exists when a merge train handled an entry, so a drain without trains prints the plain table
        let trains = rows.iter().any(|r| !r.train.is_empty());
        println!("entry\ttask\tstatus\texit\tseconds\tnote{}", if trains { "\ttrain" } else { "" });
        for r in &rows {
            println!("{}\t{}\t{}\t{}\t{:.1}\t{}{}", r.id, r.task, r.status, r.code, r.secs, r.note, if trains { format!("\t{}", if r.train.is_empty() { "-" } else { r.train.as_str() }) } else { String::new() });
        }
        if !stopped.is_empty() {
            eprintln!("broadsword: queue run --all {stopped}");
        }
        println!("worst exit: {worst}");
    }
    ExitCode::from(worst)
}

// ---- PARALLEL REVIEW (review_parallel) ------------------------------------------------------------------------------
//
// `queue run --all` with the merge train on (merge_train_max > 1) and review_parallel > 1 claims up to review_parallel entries at
// once (each with the same `claim` and `next_for_all` the serial loop uses, so the same entries in the same order) and brings them
// to their review verdict together, one worker thread per entry (`drive_all`):
//   - each worker owns its entry's Run (its record and its files: <entry>.entry, .review, .shadow.review, .donecheck); no worker
//     ever reads or writes another entry's record, so a verdict can only be written to its own entry;
//   - the steps before the review (preconditions, rebase, the deferred check, the done check) run one worker at a time, in queue
//     order (`Turn`): git writes to the shared repository and done-check jobs never overlap, exactly as in the serial drain;
//   - the reviews (gate call, then shadow call, per entry, with the same rules, models, timeouts and reviewer_max_calls_per_entry)
//     run side by side, except that every call through clients/openai_review.py (GPT gate, GPT shadow) holds `GPT_CALL`, so the GPT
//     daily cap is checked call by call with every earlier call already in the ledger, as in the serial drain;
//   - a worker only ever ends its entry `approved` or finished (rejected, blocked, stale, ...): `parallel_ok` admits only entries
//     whose run starts before the merge step, and with the merge train on such a run stops at `approved` (`drive`). No worker
//     merges, builds a train, checks a train or fast-forwards: all of that runs afterwards, on the main thread, serially, in queue order.
// The run lock (queue.lock) is held by the drain for all of it; entries.lock is taken for each claim as before. A drain killed
// mid-review leaves each claimed entry `running` at its recorded step, and the next `queue run` resumes each one as it always does.

/// Can this entry be one of a parallel batch? Only an entry whose run starts before the merge step and that is not a half-finished
/// train merge: those (and everything after a review) run on the main thread only.
fn parallel_ok(e: &Record) -> bool {
    !half_merged(e) && STEPS.iter().position(|s| *s == e.get("step")).unwrap_or(0) < STEP_MERGE
}

/// One claimed entry of a batch: `Run(run, started, start step, exit code once driven)`, or `Ended(id, exit code, started)` for one
/// `begin` already ended (its project is unknown).
enum Slot<'a> {
    Run(Run<'a>, std::time::Instant, usize, u8),
    Ended(String, u8, std::time::Instant),
}

/// Turns in queue order: `wait(i)` returns once turns 0..i are over; the turn ends when the guard drops (also on an early return).
struct Turn {
    pub(super) next: std::sync::Mutex<usize>,
    cv: std::sync::Condvar,
}

struct TurnGuard<'t>(&'t Turn);

impl Turn {
    fn wait(&self, i: usize) -> TurnGuard<'_> {
        let mut n = self.next.lock().unwrap_or_else(|p| p.into_inner());
        while *n != i {
            n = self.cv.wait(n).unwrap_or_else(|p| p.into_inner());
        }
        TurnGuard(self)
    }
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        *self.0.next.lock().unwrap_or_else(|p| p.into_inner()) += 1;
        self.0.cv.notify_all();
    }
}

/// One entry through the steps up to its verdict and, without the merge train, its merge: exactly the serial drain's body. With
/// `turn`, the steps before the review wait for their turn (PARALLEL REVIEW).
fn drive(run: &mut Run<'_>, start: usize, train: bool, turn: Option<(&Turn, usize)>) -> u8 {
    let result = (|| -> R<ExitCode> {
        {
            let _t = turn.map(|(t, i)| t.wait(i));
            run.until_review(start)?;
        }
        run.review_phase(start)?;
        if train && start <= 3 {
            run.approve() // MERGE TRAIN: the entry waits at `approved` for its train
        } else {
            run.merge_phase(start)
        }
    })();
    code_num(match result {
        Ok(c) | Err(c) => c,
    })
}

/// Drive every `Slot::Run` of a batch and record its exit code: one alone on this thread (the serial drain), more side by side,
/// one worker thread each (PARALLEL REVIEW: they are all `parallel_ok` with the train on, so each stops at its verdict).
fn drive_all(slots: &mut [Slot<'_>], train: bool) {
    let runs = slots.iter().filter(|s| matches!(s, Slot::Run(..))).count();
    if runs <= 1 {
        for s in slots.iter_mut() {
            if let Slot::Run(run, _, start, code) = s {
                *code = drive(run, *start, train, None);
            }
        }
        return;
    }
    println!("queue run --all: PARALLEL REVIEW of {runs} entries (review_parallel): their steps up to the review run one after another in queue order, their reviews side by side; trains and merges follow, one at a time");
    let turn = Turn { next: std::sync::Mutex::new(0), cv: std::sync::Condvar::new() };
    std::thread::scope(|sc| {
        let mut i = 0;
        for s in slots.iter_mut() {
            if let Slot::Run(run, _, start, code) = s {
                let (turn, at, start) = (&turn, i, *start);
                i += 1;
                sc.spawn(move || {
                    *code = drive(run, start, train, Some((turn, at)));
                });
            }
        }
    });
}

/// A summary row read back from the entry's record (its final status and one-line reason).
fn row_for(cfg: &Config, id: &str, code: u8, t: std::time::Instant) -> Row {
    let e = load_entry(cfg, id).ok();
    let get = |k: &str| e.as_ref().map(|e| e.get(k).to_string()).unwrap_or_default();
    let mut note = get("reason");
    if note.chars().count() > 120 {
        note = note.chars().take(117).collect::<String>() + "...";
    }
    let train = match (get("train"), get("train_pos")) {
        (t, p) if !t.is_empty() => format!("{t}#{p}"),
        _ => String::new(),
    };
    Row { id: id.to_string(), task: get("task"), status: get("status"), code, secs: t.elapsed().as_secs_f64(), note, train }
}

impl<'a> Run<'a> {
    fn go(&mut self) -> ExitCode {
        match self.pipeline() {
            Ok(code) | Err(code) => code,
        }
    }

    /// The step to start from: the recorded one (a resumed run), else the first.
    fn start_step(&self) -> usize {
        STEPS.iter().position(|s| *s == self.e.get("step")).unwrap_or(0)
    }

    fn pipeline(&mut self) -> R<ExitCode> {
        let start = self.start_step();
        self.until_review(start)?;
        self.review_phase(start)?;
        self.merge_phase(start)
    }

    /// Steps 1 to 3: preconditions, rebase, check (each only when `start` is at or before it).
    fn until_review(&mut self, start: usize) -> R<()> {
        if start < STEP_MERGE {
            self.s_preconditions(start)?;
        }
        if start <= 1 {
            self.s_rebase()?;
        }
        if start <= 2 {
            self.check_step()?;
        }
        Ok(())
    }

    /// Step 4, review.
    fn review_phase(&mut self, start: usize) -> R<()> {
        if start <= 3 {
            self.s_review()?;
        }
        Ok(())
    }

    /// The claims of this entry's task, as a train compares them. A task that cannot be read claims everything (`*`), so it never
    /// shares a train.
    fn load_claims(&mut self) {
        self.claims = match task::load_task(self.cfg, self.e.get("task")) {
            Ok(t) => task::claims_of(&t),
            Err(_) => vec!["*".to_string()],
        };
    }

    /// MERGE TRAIN: the gating review PASSed. The entry stops here as `approved`: nothing is merged, the task stays running, the
    /// worktree is untouched. `run_train` merges it with the other approved entries of its project.
    fn approve(&mut self) -> R<ExitCode> {
        self.load_claims();
        let base = self.e.get("base_head").to_string();
        self.e.set("train_up", base);
        self.e.set("status", APPROVED);
        // when it started to wait: the merge_train_wait_minutes rule (`short_train_why`) measures from here
        self.e.set("approved_at", now_secs().to_string());
        self.e.set("reason", "the review PASSed; it waits for a merge train (nothing is merged yet)");
        println!("queue entry {}: approved (the review PASSed); it waits for a merge train", self.e.id());
        self.log("review", "approved: the entry joins the next merge train of its project (claims that do not overlap, at most merge_train_max entries)")?;
        Ok(ExitCode::SUCCESS)
    }

    /// Before an approved entry joins a train: still exactly the reviewed commit, the worktree on its branch and clean, clean main on
    /// the same branch. A chain rebase that a killed drain left half done is aborted first (the branch is then back at its
    /// `pre_rebase` tip); any OTHER rebase in progress is the coder's own and is never aborted: the entry ends `blocked`.
    fn train_ready(&mut self) -> R<()> {
        self.locate()?;
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        if self.g(wtgit::rebase_in_progress(&wt))? {
            if self.own_chain_rebase()? {
                self.abort_stale_rebase()?;
            } else {
                return Err(self.leave_train(BLOCKED, "a rebase is in progress in the task's worktree that no merge train started (the coder's own, begun after the review); it was left alone. Finish it or abort it (`git rebase --abort`), then run the entry again; nothing merged", EXIT_RETRY));
            }
        }
        // `train_link` rebases whatever the worktree has checked out: it must be the task's own branch (as in `s_preconditions`)
        match self.g(wtgit::current_branch(&wt))? {
            Some(b) if b == branch => {}
            other => {
                let on = other.map(|b| format!("`{b}`")).unwrap_or_else(|| "a detached HEAD".into());
                return Err(self.leave_train(BLOCKED, &format!("the task's worktree is on {on}, not `{branch}`; it cannot join a merge train (its chain rebase would move what is checked out there); nothing merged. Check out `{branch}` again, then run the entry"), EXIT_RETRY));
            }
        }
        let (reviewed, checked) = (self.e.get("reviewed_tip").to_string(), self.e.get("checked_tip").to_string());
        if self.e.get("verdict") != "PASS" || reviewed.is_empty() || reviewed != checked {
            return Err(self.leave_train(STALE, &format!("an approved entry without a recorded PASS for its checked commit (reviewed `{}`, checked `{}`) cannot join a merge train. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&checked), self.e.id()), EXIT_RETRY));
        }
        if let Some(why) = self.test_hook_refusal() {
            return Err(self.leave_train(BLOCKED, &why, EXIT_RETRY));
        }
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        if tip != reviewed {
            return Err(self.leave_train(STALE, &format!("branch changed after the review: `{branch}` was reviewed at {} but is at {} now; nothing was merged. Run `queue run --entry {}` to check and review it again", short(&reviewed), short(&tip), self.e.id()), EXIT_RETRY));
        }
        let wt_status = self.g(wtgit::status_porcelain_plain(&wt))?;
        if !wt_status.is_empty() {
            return Err(self.leave_train(BLOCKED, &format!("the task's worktree got changes after the review ({}); it cannot join a merge train; nothing merged", some(&wt_status)), EXIT_RETRY));
        }
        if let Err(m) = trunk_checkout(&dir, self.e.get("main_branch")) {
            return Err(self.leave_train(BLOCKED, &format!("{m}; nothing merged"), EXIT_RETRY));
        }
        let dirty = self.g(wtgit::status_porcelain_plain(&dir))?;
        if !dirty.is_empty() {
            return Err(self.leave_train(BLOCKED, &format!("the main checkout has uncommitted or untracked changes ({}); commit or discard them, then run again; nothing merged", some(&dirty)), EXIT_RETRY));
        }
        if self.e.get("alone_tip").is_empty() {
            self.e.set("alone_tip", reviewed);
            self.save()?;
        }
        Ok(())
    }

    /// Put this entry's own commits (everything after `train_up`) on top of `onto`, the previous entry of the chain (or main's HEAD
    /// for the first). `Some(new tip)`; `None` when the entry cannot ride in this train and went back to `queued`: git could not apply
    /// its commits (its branch exactly as it was), or the rebased commits are not the same patches as the reviewed ones. The old tip is
    /// saved under refs/broadsword/queue/ first, as for every rebase.
    ///
    /// THE REVIEW BINDS ITS COMMITS: the review and the merge follow the entry to its new commit only when `git patch-id --stable` of
    /// the rebased commits (`onto..new tip`) equals that of the commits that were reviewed (`base_head..reviewed_orig_tip`), commit for
    /// commit. Anything else (a hunk whose context the entries before it changed) sends the entry back to `queued` ("patch changed by
    /// the train rebase"): it is reviewed again and never merges on the old verdict.
    fn train_link(&mut self, onto: &str, train_id: &str) -> R<Option<String>> {
        let (dir, wt, branch) = (self.dir.clone(), self.wt.clone(), self.branch());
        let tip = self.g(wtgit::branch_tip(&dir, &branch))?;
        let up = if self.e.get("train_up").is_empty() { self.e.get("base_head").to_string() } else { self.e.get("train_up").to_string() };
        let orig = if self.e.get("reviewed_orig_tip").is_empty() { self.e.get("reviewed_tip").to_string() } else { self.e.get("reviewed_orig_tip").to_string() };
        let base = self.e.get("base_head").to_string();
        let reviewed_ids = self.g(wtgit::patch_ids(&dir, &format!("{base}..{orig}")))?;
        self.e.set("train", train_id);
        let r = self.keep_pre_rebase(&tip)?;
        self.e.set("pre_rebase", tip.clone());
        self.e.set("pre_rebase_ref", r.clone());
        // KILL WINDOW: `train_onto` is SAVED (by the log line below) before the rebase starts and cleared only in the save that
        // records the result. A drain killed after the rebase moved the branch but before that save leaves it set, so the left-train
        // rule (`chained_base`) still sees the branch as chained on `onto`, and `train_ready` knows a rebase in progress is its own.
        self.e.set("train_onto", onto);
        self.log("rebase", &format!("merge train {train_id}: chaining `{branch}` (tip {}) onto {}; its own commits start after {}; the old tip is saved in {r}", short(&tip), short(onto), short(&up)))?;
        match self.g(wtgit::rebase_onto(&wt, onto, &up))? {
            wtgit::Rebase::Done => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if !self.g(wtgit::is_ancestor(&dir, onto, &now))? {
                    return Err(self.internal(&format!("the chain rebase reported success but `{branch}` ({}) does not contain {}; the old tip {} is saved in {r}", short(&now), short(onto), short(&tip))));
                }
                if self.g(wtgit::rev_count(&dir, &format!("{onto}..{now}")))? == 0 {
                    self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                    return Err(self.leave_train(BLOCKED, &format!("after chaining onto the train `{branch}` has no commit left (its changes are already in the train); the old tip {} is saved in {r}", short(&tip)), EXIT_RETRY));
                }
                let now_ids = self.g(wtgit::patch_ids(&dir, &format!("{onto}..{now}")))?;
                if now_ids != reviewed_ids {
                    self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                    self.e.set("rebased_tip", now.clone());
                    self.requeue(&format!("patch changed by the train rebase: merge train {train_id} chained `{branch}` onto the entries before it, and its rebased commits {} are not the same patches (git patch-id --stable) as the reviewed commits {}..{}; the old verdict does not cover them, so the entry goes back to `queued` and is reviewed again (the pre-train tip is saved in {r})", short(&now), short(&base), short(&orig)));
                    return Ok(None);
                }
                if self.e.get("reviewed_orig_tip").is_empty() {
                    self.e.set("reviewed_orig_tip", orig.clone());
                }
                self.e.set("train_up", onto);
                self.e.set("train_onto", "");
                self.e.set("rebased_tip", now.clone());
                self.e.set("checked_tip", now.clone());
                self.e.set("reviewed_tip", now.clone());
                self.e.set("check_deferred", "yes");
                self.e.set("train_checked", "");
                self.save()?;
                self.log("rebase", &format!("chained: tip {} -> {}; the same {} patch(es) as reviewed (git patch-id --stable)", short(&tip), short(&now), now_ids.len()))?;
                Ok(Some(now))
            }
            wtgit::Rebase::Conflict(files) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("the chain rebase conflicted and was aborted, but `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                self.e.set("train_onto", ""); // the branch is exactly as before: it was never chained onto `onto`
                self.requeue(&format!("merge train {train_id}: chaining `{branch}` onto the entries before it conflicts in: {}. The rebase was aborted and the branch is exactly as before; the entry goes back to `queued` and is reviewed again on its own", files.join(", ")));
                Ok(None)
            }
            wtgit::Rebase::Failed(m) => {
                let now = self.g(wtgit::branch_tip(&dir, &branch))?;
                if now != tip {
                    return Err(self.internal(&format!("{m}; and `{branch}` is at {} instead of {}: the old tip is saved in {r}", short(&now), short(&tip))));
                }
                self.e.set("train_onto", "");
                self.requeue(&format!("merge train {train_id}: `{branch}` could not be chained onto the entries before it: {m}. The branch is unchanged; the entry goes back to `queued`"));
                Ok(None)
            }
        }
    }

    /// An entry a train cannot take goes back to `queued` with the reason; the next `queue run` checks and reviews it afresh. Unless
    /// its branch is chained on other entries (see `leave_train`): then it ends `blocked`.
    /// Both outcomes are exit 3 (the caller records that); a record that cannot be written has printed its own error.
    fn requeue(&mut self, reason: &str) {
        let _ = self.leave_train(QUEUED, reason, EXIT_RETRY);
    }

    /// LEFT-TRAIN RULE, the ONE place it is applied: every way an entry leaves a merge train without merging (back to queued,
    /// check_failed, conflict, stale, blocked) ends here, and so does a solo run of an entry still chained (`s_preconditions`,
    /// `s_merge`). An entry whose branch still carries commits of other entries that are not on main (`chained_onto`) never goes on to
    /// a review or merge of that chained diff: it ends `blocked` ("left a train: ..."), and `left_train_base` keeps the commit it is
    /// chained on across attempts, so a later `queue run` or `queue add` refuses it too until the coder has rebased its own commits
    /// onto main (or those entries reached main). Otherwise the entry gets `status` as asked. A git error counts as chained.
    pub(super) fn leave_train(&mut self, status: &str, reason: &str, code: u8) -> ExitCode {
        let (dir, branch) = (self.dir.clone(), self.branch());
        let base = match chained_base(&dir, &branch, &self.e) {
            Ok(b) => b,
            // a git error counts as chained, on the first recorded base (an error needs one: no base is never chained)
            Err(_) => Some(chain_bases_of(&self.e).into_iter().next().unwrap_or_default()),
        };
        let Some(base) = base else {
            if status == QUEUED {
                let reason = self.secrets.redact(reason);
                self.e.set("status", QUEUED);
                self.e.set("reason", reason.clone());
                let _ = self.log("train", &format!("back to queued: {reason}"));
                eprintln!("broadsword: queue entry {} goes back to `queued`: {reason}", self.e.id());
                return ExitCode::from(EXIT_RETRY);
            }
            return self.end(status, reason, code);
        };
        let main = if self.e.get("main_branch").is_empty() { trunk_name(&dir) } else { self.e.get("main_branch").to_string() };
        self.e.set("left_train_base", base.clone());
        let text = left_train_text(&main, &branch, &base);
        self.blocked(&format!("{text}. Why it left (it would have been `{status}`): {reason}"))
    }
}

/// What a `check_alone` said about the culprit of a failed train.
enum Alone {
    /// It fails on its own commit too: the entry itself is bad.
    Bad(PassInfo),
    /// It passes on its own commit: the failure only shows when it is combined with the entries before it.
    Fine,
    Unknown(String),
    /// Its own worktree kept the check from a verdict (see `TrainCheck::EntryBlocked`).
    EntryBlocked(String),
}

/// The entries the next train takes out of `items` (one `(project, claims)` per approved entry, oldest first), as indices into
/// `items`. Entries of one project only; an entry whose claims overlap an entry already taken is left out (it waits for a later
/// train); at most `max` entries. A train is ready when it is full, or when `flush` is set (nothing else is coming) and it holds at
/// least one entry. Empty when no project has a ready train.
#[cfg(test)]
fn pick_train(items: &[(String, Vec<String>)], max: usize, flush: bool) -> Vec<usize> {
    pick_train_when(items, max, &|_| flush)
}

/// `pick_train` with readiness per project: a project's train is ready when it is full, or when `ready(project)` and it holds at
/// least one entry (the flush train of the drain's end is `ready` for every project; `short_train_projects` says which projects get
/// a short train before the end). Projects are tried in queue order of their oldest approved entry.
fn pick_train_when(items: &[(String, Vec<String>)], max: usize, ready: &dyn Fn(&str) -> bool) -> Vec<usize> {
    let mut seen: Vec<&str> = Vec::new();
    for (first, (project, _)) in items.iter().enumerate() {
        if seen.contains(&project.as_str()) {
            continue;
        }
        seen.push(project.as_str());
        let mut group: Vec<usize> = Vec::new();
        for (i, (p, claims)) in items.iter().enumerate().skip(first) {
            if p != project || group.len() >= max {
                continue;
            }
            let clash = group.iter().any(|g| items[*g].1.iter().any(|a| claims.iter().any(|b| task::overlaps(a, b))));
            if !clash {
                group.push(i);
            }
        }
        if group.len() >= max || (!group.is_empty() && ready(project)) {
            return group;
        }
    }
    Vec::new()
}

/// Take the next train out of `approved` (see `pick_train_when`), in queue order; the others stay.
fn take_group<'a>(approved: &mut Vec<(Run<'a>, std::time::Instant)>, max: usize, ready: &dyn Fn(&str) -> bool) -> Option<Vec<(Run<'a>, std::time::Instant)>> {
    let items: Vec<(String, Vec<String>)> = approved.iter().map(|(r, _)| (r.e.get("project").to_string(), r.claims.clone())).collect();
    let idx = pick_train_when(&items, max, ready);
    if idx.is_empty() {
        return None;
    }
    let mut group = Vec::new();
    for i in idx.iter().rev() {
        group.push(approved.remove(*i));
    }
    group.reverse();
    Some(group)
}

/// Entries an earlier `queue run --all` approved and never merged (it died, or stopped): claimed by this process (their pid), oldest
/// first. Under entries.lock; their status stays `approved`.
fn claim_approved(cfg: &Config, lane: &Lane) -> Result<Vec<Record>, ExitCode> {
    let _l = entries_lock(cfg)?;
    let all: Vec<Record> = load_entries(cfg).map_err(|m| fail(EXIT_ERROR, m))?.into_iter().filter(|x| lane.holds(x)).collect();
    let me = std::process::id();
    let mut out = Vec::new();
    // a half-finished merge is not a train candidate: the drain resumes it on its own first (`resume_first`)
    for mut e in all.into_iter().filter(|x| x.status() == APPROVED && !half_merged(x)) {
        e.set("pid", me.to_string());
        e.set("pid_start", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
        e.save().map_err(|err| fail(EXIT_ERROR, format!("cannot write the queue entry {}: {err}", e.path.display())))?;
        out.push(e);
    }
    Ok(out)
}

fn train_stop_note(code: u8) -> String {
    if code == EXIT_MERGED_BAD {
        "stopped by a merge train (exit 6): main holds the train's commits and the main check failed; a human decides before anything else merges".into()
    } else {
        "stopped by a merge train (exit 1): an internal error or a check that could not run to a verdict; the entries it was working on are `approved` (or `running`), and the next `queue run --all` takes them again".into()
    }
}

/// One entry of a train is finished (merged, rejected, back to queued, ...): its summary row, and the train's worst outcome so far.
pub(super) fn finish_member(cfg: &Config, r: Run<'_>, code: u8, t: std::time::Instant, rows: &mut Vec<Row>, worst: &mut u8) {
    let id = r.e.id().to_string();
    rows.push(row_for(cfg, &id, code, t));
    if badness(code) > badness(*worst) {
        *worst = code;
    }
}

/// The entries stay `approved` (the reason says why), and the next drain takes them again.
fn leave_approved(cfg: &Config, members: Vec<(Run<'_>, std::time::Instant)>, why: &str, rows: &mut Vec<Row>, worst: &mut u8) {
    for (mut r, t) in members {
        r.e.set("reason", format!("approved, not merged: {why}"));
        let _ = r.log("train", why);
        finish_member(cfg, r, EXIT_RETRY, t, rows, worst);
    }
}

/// A train check gave no verdict because of ONE entry's worktree (`TrainCheck::EntryBlocked`): that entry leaves the train and ends
/// `blocked` with the reason (the left-train rule decides the exact text, `leave_train`), as the single-entry path blocks only its own
/// entry. The `others` merged nothing and stay `approved`: the next drain takes them again without it. The drain is NOT stopped.
fn entry_leaves_blocked(cfg: &Config, train_id: &str, member: (Run<'_>, std::time::Instant), msg: &str, others: Vec<(Run<'_>, std::time::Instant)>, rows: &mut Vec<Row>, worst: &mut u8) {
    let (mut r, t) = member;
    let id = r.e.id().to_string();
    println!("merge train {train_id}: entry {id} cannot be checked ({msg}); it leaves the train `blocked`, the other {} entr{} stay approved for the next drain", others.len(), if others.len() == 1 { "y" } else { "ies" });
    let c = r.leave_train(BLOCKED, &format!("merge train {train_id} could not check this entry: {msg}. Put the task's worktree back on its branch at the reviewed commit with nothing else in it, then run the entry again; nothing of it was merged"), EXIT_RETRY);
    finish_member(cfg, r, code_num(c), t, rows, worst);
    leave_approved(cfg, others, &format!("entry {id} left merge train {train_id} (its worktree was not exactly the commit to check); nothing merged, this entry waits for the next drain"), rows, worst);
}

/// An internal error that hits every member (git or the record could not be used): each stays as it is, with the message.
pub(super) fn fail_members(cfg: &Config, members: Vec<(Run<'_>, std::time::Instant)>, msg: &str, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    for (mut r, t) in members {
        let c = r.internal(msg);
        finish_member(cfg, r, code_num(c), t, rows, worst);
    }
    EXIT_ERROR
}

/// Entries still `approved` when the drain ends early (it stopped) or no train took them: a row each; the next drain takes them.
fn settle_approved(cfg: &Config, approved: &mut Vec<(Run<'_>, std::time::Instant)>, rows: &mut Vec<Row>) {
    for (r, t) in approved.drain(..) {
        let id = r.e.id().to_string();
        rows.push(row_for(cfg, &id, 0, t));
    }
}

/// An entry this drain may still run (so its project's approved entries may wait for it to fill their train): not taken by this
/// drain yet (`taken`; a taken entry is never run again in the same drain, so waiting for it would be waiting for the end), and
/// queued, running or stale, or a half-finished train merge (`resume_first`): what `next_for_all` can still pick.
fn still_to_run(x: &Record, taken: &[String]) -> bool {
    !taken.iter().any(|t| t == x.id()) && (resume_first(x) || matches!(x.status(), QUEUED | RUNNING | STALE))
}

/// STARVATION RULE, for one project with approved entries waiting: why it gets a train now although the train is not full and the
/// drain has not ended, or None (it keeps waiting for more of its entries). `approved_at` holds each of its approved entries'
/// approval time (unix seconds; None when the entry has none that can be read). A train is due when
///   (a) `pending` is false: no entry of the project is left for this drain to run, so nothing more can join its train; or
///   (b) its oldest approved entry has waited `wait_minutes` (merge_train_wait_minutes) by `now`.
/// An entry with no readable approval time (approved before this rule recorded one) counts as having waited long enough: a short
/// train only merges earlier, through the same chain, check, merge and main check as the flush train of the drain's end. A clock
/// that went back counts as no wait.
fn short_train_why(pending: bool, approved_at: &[Option<u64>], now: u64, wait_minutes: u64) -> Option<String> {
    if approved_at.is_empty() {
        return None;
    }
    if !pending {
        return Some("no other entry of its project is left to run".into());
    }
    if approved_at.iter().any(|a| a.is_none()) {
        return Some("an approved entry has no readable approval time (`approved_at`), so it counts as having waited merge_train_wait_minutes".into());
    }
    let oldest = approved_at.iter().flatten().min().copied().unwrap_or(now);
    let waited = now.saturating_sub(oldest);
    (waited >= wait_minutes.saturating_mul(60)).then(|| format!("its oldest approved entry has waited {} s (merge_train_wait_minutes = {wait_minutes})", waited))
}

/// The projects that get a short train now (`short_train_why`), each with the reason, in the order of `approved` (project, approval
/// time). `all` is every queue entry (to see what is still to run), `taken` the entries this drain took.
fn short_train_projects_in(all: &[Record], approved: &[(String, Option<u64>)], taken: &[String], now: u64, wait_minutes: u64) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (project, _) in approved {
        if out.iter().any(|(p, _)| p == project) {
            continue;
        }
        let times: Vec<Option<u64>> = approved.iter().filter(|(p, _)| p == project).map(|(_, a)| *a).collect();
        let pending = all.iter().any(|x| x.get("project") == project && still_to_run(x, taken));
        if let Some(why) = short_train_why(pending, &times, now, wait_minutes) {
            out.push((project.clone(), why));
        }
    }
    out
}

/// `short_train_projects_in` on the queue as it is now (read under entries.lock) and the clock.
fn short_train_projects(cfg: &Config, approved: &[(Run<'_>, std::time::Instant)], taken: &[String]) -> Result<Vec<(String, String)>, String> {
    let all = {
        let _l = entries_lock(cfg).map_err(|c| format!("the queue's entries lock could not be taken (exit {})", code_num(c)))?;
        load_entries(cfg)?
    };
    let items: Vec<(String, Option<u64>)> = approved.iter().map(|(r, _)| (r.e.get("project").to_string(), r.e.get_num("approved_at"))).collect();
    Ok(short_train_projects_in(&all, &items, taken, now_secs(), cfg.merge_train_wait_minutes))
}

/// Run trains while `approved` holds a ready one. `flush` (the drain's end): every project is ready. Otherwise a project's train is
/// ready when it is full, or when the starvation rule says so (`short_train_projects`: none of its entries is left to run, or its
/// oldest approved entry waited merge_train_wait_minutes). A short train is exactly a flush train of that project. `taken`: the
/// entries this drain took. Returns 0, or EXIT_ERROR / EXIT_MERGED_BAD when the drain must stop.
fn run_trains<'a>(cfg: &'a Config, approved: &mut Vec<(Run<'a>, std::time::Instant)>, flush: bool, taken: &[String], rows: &mut Vec<Row>) -> u8 {
    let max = cfg.merge_train_max as usize;
    let mut hard = 0u8;
    while hard == 0 {
        let short = if flush {
            Vec::new()
        } else {
            match short_train_projects(cfg, approved, taken) {
                Ok(s) => s,
                Err(m) => {
                    // fail closed: no train is started on a queue that could not be read; the entries stay approved
                    eprintln!("broadsword: queue run --all: cannot tell which merge trains are due: {m}");
                    return EXIT_ERROR;
                }
            }
        };
        let ready = |p: &str| flush || short.iter().any(|(s, _)| s == p);
        let Some(group) = take_group(approved, max, &ready) else { break };
        if !flush && group.len() < max {
            let project = group[0].0.e.get("project").to_string();
            let why = short.iter().find(|(s, _)| *s == project).map(|(_, w)| w.as_str()).unwrap_or("-");
            println!("merge train for project {project}: {} of at most {max} entries, not waiting for more: {why}", group.len());
        }
        hard = run_train(cfg, group, rows);
    }
    hard
}

/// The commit a chain entry's own check proves (its tip in the chain), for the 1-based position `i`.
fn chain_tip(chain: &[(Run<'_>, std::time::Instant)], i: usize) -> String {
    chain[i - 1].0.e.get("checked_tip").to_string()
}

/// ONE MERGE TRAIN. `group` are approved entries of one project with claims that do not overlap (`pick_train`).
///
/// 1. Each entry is checked to still be exactly its reviewed commit (`train_ready`).
/// 2. CHAIN: each entry's own commits are rebased onto the previous entry's tip, the first onto main's HEAD (`train_link`). The result
///    is one straight line `main <- e1 <- e2 <- ... <- en`, every entry's branch tip is on it, and the tip of entry i contains the
///    entries before it. An entry that cannot be chained (a textual conflict), or whose rebased commits are not the same patches as
///    the reviewed ones, goes back to `queued`; the rest go on.
/// 3. ONE CHECK of the chain tip (`Run::check_commit`).
/// 4. PASS: ONE fast-forward of main to the tip (every entry's commits as they are, in order, no squash), ONE main check (`s_main_check`,
///    run by the last entry), then every entry is `merged` (`task done`), recording train id, position and the shared check job.
/// 5. FAIL: BISECT. The check of entry i's tip is a check of the first i entries, so a binary search over the prefixes finds the first
///    entry k whose prefix fails (about log2(n) checks; the prefix before it passed). The culprit is then checked on its own commit
///    (`check_alone`): if it fails alone it is `check_failed` with that check's log, the proven-good entries before it merge as a
///    smaller train, and the entries after it start a new round on the new main. If it passes alone the failure only shows when it is
///    combined with the entries before it: the culprit AND every entry before it in the chain (the partner is not searched for) are
///    held out with the combined failure's log, and NOTHING is merged for them. COMBINED-ONLY PAIR, DECIDED (round 3, kept as the brief
///    says: "neither merges"; the proven prefix is NOT merged): each held entry asks for `conflict`, and the left-train rule
///    (`leave_train`) decides what it ends as. The first entry of the chain sits on main's HEAD and ends a plain `conflict`; every later
///    one still carries the earlier held entries' commits, which are not on main, so it ends `blocked` ("left a train: ... it would
///    have been `conflict`", with the rebase command). So for a pair e1, e2: e1 `conflict`, e2 `blocked`, main unchanged. Merging the
///    proven prefix instead would turn e2 into a plain `conflict` but put e1 on main while its partner is unknown; that is a later
///    choice for the measurement window, not this phase. Every round removes at least one entry, so it ends.
/// 6. A main check that fails after the fast-forward: every entry of the merge is `merged_check_failed` (exit 6), main is NOT reset, the
///    drain stops. A check that cannot run to a verdict leaves all its entries `approved` and stops the drain, unless the reason is
///    one entry's own worktree (`TrainCheck::EntryBlocked`): then only that entry ends `blocked`, the rest stay `approved` for the
///    next drain, and this drain goes on.
///
/// Returns 0, EXIT_ERROR or EXIT_MERGED_BAD (the drain stops on the last two).
fn run_train<'a>(cfg: &'a Config, group: Vec<(Run<'a>, std::time::Instant)>, rows: &mut Vec<Row>) -> u8 {
    static TRAIN_SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let train_id = format!("tr-{}-{}", job::file_stamp(now_secs()), TRAIN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    let started = std::time::Instant::now();
    let size = group.len();
    println!("merge train {train_id}: {size} approved entr{} (merge_train_max = {})", if size == 1 { "y" } else { "ies" }, cfg.merge_train_max);
    let mut worst = 0u8;
    let mut hard = 0u8;
    let mut checks = 0u32;
    let mut todo: Vec<(Run<'a>, std::time::Instant)> = Vec::new();
    for (mut r, t) in group {
        match r.train_ready() {
            Ok(()) => todo.push((r, t)),
            Err(c) => {
                let code = code_num(c);
                finish_member(cfg, r, code, t, rows, &mut worst);
            }
        }
    }
    let mut round = 0u32;
    while !todo.is_empty() && hard == 0 {
        round += 1;
        let dir = todo[0].0.dir.clone();
        // the chain's base is the trunk's tip, and the trunk must be what the main checkout has checked out (`trunk_checkout`)
        let head = match trunk_checkout(&dir, todo[0].0.e.get("main_branch")) {
            Ok((_, h)) => h,
            Err(m) => {
                for (mut r, t) in std::mem::take(&mut todo) {
                    let c = r.leave_train(BLOCKED, &format!("{m}; nothing merged"), EXIT_RETRY);
                    finish_member(cfg, r, code_num(c), t, rows, &mut worst);
                }
                break;
            }
        };
        // ---- chain
        let mut chain: Vec<(Run<'a>, std::time::Instant)> = Vec::new();
        let mut prev = head.clone();
        let mut pending: std::collections::VecDeque<(Run<'a>, std::time::Instant)> = std::mem::take(&mut todo).into();
        while let Some((mut r, t)) = pending.pop_front() {
            match r.train_link(&prev, &train_id) {
                Ok(Some(tip)) => {
                    prev = tip;
                    chain.push((r, t));
                }
                Ok(None) => finish_member(cfg, r, EXIT_RETRY, t, rows, &mut worst),
                Err(c) => {
                    let code = code_num(c);
                    finish_member(cfg, r, code, t, rows, &mut worst);
                    if code == EXIT_ERROR {
                        hard = EXIT_ERROR;
                        break;
                    }
                }
            }
        }
        if hard != 0 {
            let rest: Vec<(Run<'a>, std::time::Instant)> = chain.into_iter().chain(pending).collect();
            leave_approved(cfg, rest, "the merge train stopped on an internal error of an earlier entry", rows, &mut worst);
            break;
        }
        if chain.is_empty() {
            continue;
        }
        let m = chain.len();
        for (i, (r, _)) in chain.iter_mut().enumerate() {
            r.e.set("train", train_id.clone());
            r.e.set("train_pos", (i + 1).to_string());
            let _ = r.save();
        }
        // ---- one check of the whole chain, then a bisect if it fails
        let mut lo = 0usize; // the first `lo` entries are proven good by a check that passed (their tip's)
        let mut hi = m; // the check of the first `hi` entries failed
        let mut good = PassInfo::default();
        let mut bad = PassInfo::default();
        let mut blocked: Option<String> = None;
        // a check that gave no verdict because of ONE entry's worktree: that entry's 1-based position in the chain, and why
        let mut entry_blocked: Option<(usize, String)> = None;
        let mut bisect_checks = 0u32;
        let mut bisect_secs = 0f64;
        let tip = chain_tip(&chain, m);
        let first = chain[m - 1].0.check_commit(&tip, &format!("merge train {train_id}: all {m} entries combined"));
        checks += 1;
        match first {
            TrainCheck::Pass { job, summary, tail } => {
                lo = m;
                good = PassInfo { job, summary, tail, perf: String::new() };
            }
            TrainCheck::Fail { job, summary, tail, perf } => bad = PassInfo { job, summary, tail, perf },
            TrainCheck::Blocked(msg) => blocked = Some(msg),
            TrainCheck::EntryBlocked(msg) => entry_blocked = Some((m, msg)),
        }
        while blocked.is_none() && entry_blocked.is_none() && hi - lo > 1 {
            let mid = (lo + hi) / 2;
            let tip_mid = chain_tip(&chain, mid);
            let tb = std::time::Instant::now();
            let res = chain[mid - 1].0.check_commit(&tip_mid, &format!("bisect of merge train {train_id}: the first {mid} entries"));
            bisect_checks += 1;
            checks += 1;
            bisect_secs += tb.elapsed().as_secs_f64();
            match res {
                TrainCheck::Pass { job, summary, tail } => {
                    lo = mid;
                    good = PassInfo { job, summary, tail, perf: String::new() };
                }
                TrainCheck::Fail { job, summary, tail, perf } => {
                    hi = mid;
                    bad = PassInfo { job, summary, tail, perf };
                }
                TrainCheck::Blocked(msg) => blocked = Some(msg),
                TrainCheck::EntryBlocked(msg) => entry_blocked = Some((mid, msg)),
            }
        }
        if bisect_checks > 0 {
            println!("merge train {train_id}: bisect: {bisect_checks} extra check(s) in {bisect_secs:.1} s; the first {lo} entr{} pass, entry {hi} makes it fail", if lo == 1 { "y" } else { "ies" });
        }
        let stats = format!("round {round}: {} check(s) of the chain, {bisect_checks} of them bisect, {bisect_secs:.1} s of bisect", 1 + bisect_checks);
        for (r, _) in chain.iter_mut() {
            r.e.set("train_bisect", stats.clone());
        }
        if let Some(msg) = blocked {
            println!("merge train {train_id}: the check could not give a verdict ({msg}); the {m} entries stay approved");
            leave_approved(cfg, chain, &format!("the merge train's check could not run to a verdict: {msg}"), rows, &mut worst);
            hard = EXIT_ERROR;
            break;
        }
        if let Some((pos, msg)) = entry_blocked {
            // ONE entry's worktree is the reason (see `TrainCheck::EntryBlocked`): only it leaves, the rest wait, the drain goes on
            let member = chain.remove(pos - 1);
            entry_leaves_blocked(cfg, &train_id, member, &msg, chain, rows, &mut worst);
            break;
        }
        if lo == m {
            // ---- the whole chain passed
            let c = merge_members(cfg, &train_id, chain, &tip, &head, &good, rows, &mut worst);
            if c != 0 {
                hard = c;
            }
            continue;
        }
        // ---- entry `hi` (1-based) is the first whose prefix fails
        let k = hi;
        let after: Vec<(Run<'a>, std::time::Instant)> = chain.split_off(k);
        let Some((mut culprit, ct)) = chain.pop() else {
            hard = EXIT_ERROR;
            break;
        };
        let alone = if k == 1 {
            Alone::Bad(bad.clone()) // its tip IS its own commit on main: the check that failed was already the check on its own
        } else {
            checks += 1;
            match culprit.check_alone() {
                TrainCheck::Fail { job, summary, tail, perf } => Alone::Bad(PassInfo { job, summary, tail, perf }),
                TrainCheck::Pass { job, summary, .. } => {
                    let _ = culprit.log("train", &format!("checked on its own commit: passes (job {job}: {summary}); so the failure only shows when it is combined with the entries before it"));
                    Alone::Fine
                }
                TrainCheck::Blocked(msg) => Alone::Unknown(msg),
                TrainCheck::EntryBlocked(msg) => Alone::EntryBlocked(msg),
            }
        };
        let culprit_id = culprit.e.id().to_string();
        match alone {
            Alone::EntryBlocked(msg) => {
                let mut rest = chain;
                rest.extend(after);
                entry_leaves_blocked(cfg, &train_id, (culprit, ct), &msg, rest, rows, &mut worst);
                break;
            }
            Alone::Unknown(msg) => {
                println!("merge train {train_id}: entry {culprit_id} could not be checked on its own ({msg}); every entry stays approved");
                let mut all = chain;
                all.push((culprit, ct));
                all.extend(after);
                leave_approved(cfg, all, &format!("the merge train's check of {culprit_id} on its own could not run to a verdict: {msg}"), rows, &mut worst);
                hard = EXIT_ERROR;
                break;
            }
            Alone::Bad(info) => {
                let failed = if info.perf.is_empty() { CHECK_FAILED } else { PERF_FAILED };
                println!("merge train {train_id}: entry {culprit_id} fails on its own (job {}): {failed}; the {} entr{} before it merge", info.job, lo, if lo == 1 { "y" } else { "ies" });
                culprit.e.set("check_job", info.job.clone());
                culprit.e.set("check_exit", if info.perf.is_empty() { "1" } else { "0" });
                culprit.e.set("check_summary", info.summary.clone());
                culprit.e.set("check_tail", info.tail.clone());
                // the proven-good entries before it merge FIRST: then a culprit chained only on them sits on main and ends a plain
                // `check_failed`; if they could not merge, its branch still carries them and it ends `blocked` (`leave_train`)
                if lo > 0 {
                    let tip_good = chain_tip(&chain, lo);
                    let c = merge_members(cfg, &train_id, chain, &tip_good, &head, &good, rows, &mut worst);
                    if c != 0 {
                        hard = c;
                    }
                }
                let job_shown = if info.job.is_empty() { "?" } else { info.job.as_str() };
                let why = if info.perf.is_empty() {
                    format!("build or tests failed on this entry on its own (job {job_shown}): {}", info.summary.replace('\n', "; "))
                } else {
                    format!("the build and tests passed on this entry on its own (job {job_shown}) but the perf gate failed: {}", info.perf)
                };
                let c = culprit.leave_train(failed, &format!("{why}. Found by the bisect of merge train {train_id}; nothing of it was merged. Fix it, commit, and queue the task again with `queue add`"), EXIT_REJECTED);
                finish_member(cfg, culprit, code_num(c), ct, rows, &mut worst);
                if hard != 0 {
                    leave_approved(cfg, after, "the merge train stopped before this entry's turn", rows, &mut worst);
                    break;
                }
                todo = after;
            }
            Alone::Fine => {
                // COMBINED-ONLY (see 5 above): all held out; the first ends `conflict`, the ones chained on it `blocked` (left-train rule)
                println!("merge train {train_id}: entry {culprit_id} passes on its own, but fails combined with the entries before it: conflict for it and for those {lo} entr{}; nothing of them is merged", if lo == 1 { "y" } else { "ies" });
                let mut held = chain;
                held.push((culprit, ct));
                let ids: Vec<String> = held.iter().map(|(r, _)| r.e.id().to_string()).collect();
                for (mut r, t) in held {
                    r.e.set("check_job", bad.job.clone());
                    r.e.set("check_exit", "1");
                    r.e.set("check_summary", bad.summary.clone());
                    r.e.set("check_tail", bad.tail.clone());
                    let perf_note = if bad.perf.is_empty() { String::new() } else { format!(" (it was the perf gate that failed: {})", bad.perf) };
                    let reason = format!("the combined check of merge train {train_id} failed (job {}){perf_note} although each of these entries passes on its own: {}. They only fail together, so none of them was merged (main is unchanged). The entry's check summary and tail are the combined failure's. Rebase or fix them yourself, commit, and queue them again with `queue add`", if bad.job.is_empty() { "?" } else { bad.job.as_str() }, ids.join(", "));
                    let c = r.leave_train(CONFLICT, &reason, EXIT_RETRY);
                    finish_member(cfg, r, code_num(c), t, rows, &mut worst);
                }
                todo = after;
            }
        }
    }
    println!("merge train {train_id}: finished in {:.1} s; {size} entr{} taken, {checks} check(s) of the train, worst exit {worst}", started.elapsed().as_secs_f64(), if size == 1 { "y" } else { "ies" });
    hard
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use super::super::entry::is_open;
    use super::super::{CANCELLED, MERGED};

    #[test]
    fn steps_are_in_the_documented_order() {
        assert_eq!(STEPS, ["preconditions", "rebase", "check", "review", "merge", "main_check", "done"]);
        assert_eq!(STEPS[STEP_MERGE], "merge");
        assert!(is_open(APPROVED), "an approved entry is open: its task cannot be queued again");
        assert!(is_open(QUEUED) && is_open(RUNNING) && is_open(BLOCKED) && is_open(STALE) && !is_open(MERGED) && !is_open(CANCELLED) && !is_open(REJECTED) && !is_open(CHECK_FAILED) && !is_open(CONFLICT));
    }

    fn item(project: &str, claims: &[&str]) -> (String, Vec<String>) {
        (project.to_string(), claims.iter().map(|c| c.to_string()).collect())
    }

    #[test]
    fn a_train_takes_only_entries_with_claims_that_do_not_overlap_and_only_from_one_project() {
        let items = vec![item("p", &["src/a.rs"]), item("p", &["src"]), item("q", &["src/a.rs"]), item("p", &["docs"]), item("p", &["src/b.rs"])];
        // `src` overlaps `src/a.rs` (and `src/b.rs`), so the second entry is left out; the other project's entry never joins
        assert_eq!(pick_train(&items, 3, false), vec![0, 3, 4], "full train: a, docs, b");
        assert_eq!(pick_train(&items, 4, false), Vec::<usize>::new(), "only 3 compatible entries: not full, so not ready without flush");
        assert_eq!(pick_train(&items, 4, true), vec![0, 3, 4], "flush takes what there is");
        // a task that could not be read claims everything and rides alone
        let items = vec![item("p", &["*"]), item("p", &["src/a.rs"])];
        assert_eq!(pick_train(&items, 2, false), Vec::<usize>::new());
        assert_eq!(pick_train(&items, 2, true), vec![0]);
        // two projects: the first project with a ready train goes first
        let items = vec![item("p", &["a"]), item("q", &["a"]), item("q", &["b"])];
        assert_eq!(pick_train(&items, 2, false), vec![1, 2]);
        assert_eq!(pick_train(&[], 2, true), Vec::<usize>::new());
    }

    #[test]
    fn a_train_uses_key_claim_overlap_without_a_separate_rule() {
        for file in ["broadsword.toml", "crates/broadsword/src/config.rs"] {
            let alpha = format!("{file}#alpha");
            let beta = format!("{file}#beta");
            let items = vec![item("p", &[&alpha]), item("p", &[&beta]), item("p", &[&alpha]), item("p", &[file]), item("p", &["*"])];
            assert_eq!(pick_train(&items, 2, false), vec![0, 1], "different keys share a full train");
            assert_eq!(pick_train(&items, 3, true), vec![0, 1], "same key, whole file and wildcard stay out");
            let whole_first = vec![item("p", &[file]), item("p", &[&alpha]), item("p", &[&beta])];
            assert_eq!(pick_train(&whole_first, 3, true), vec![0], "a whole-file claim rides alone");
        }
    }

    #[test]
    fn a_train_never_exceeds_merge_train_max() {
        let items: Vec<(String, Vec<String>)> = (0..6)
            .map(|i| {
                let f = format!("f{i}");
                item("p", &[f.as_str()])
            })
            .collect();
        assert_eq!(pick_train(&items, 4, false), vec![0, 1, 2, 3]);
        assert_eq!(pick_train(&items, 1, false), vec![0], "1 = every entry on its own");
    }

    fn entry(id: &str, project: &str, status: &str) -> Record {
        Record::parse(Path::new(&format!("{id}.entry")), &format!("id = {id}\nproject = {project}\nstatus = {status}\n"))
    }

    #[test]
    fn a_project_with_no_entry_left_to_run_gets_a_short_train_while_another_project_still_has_queued_entries() {
        // crash-sim has 2 approved entries and nothing left to run; broadsword has 1 approved and more queued (the 2026-11-20 case)
        let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c2", "crash-sim", APPROVED), entry("q-b1", "broadsword", APPROVED), entry("q-b2", "broadsword", QUEUED)];
        let approved = vec![("crash-sim".to_string(), Some(1000)), ("crash-sim".to_string(), Some(1010)), ("broadsword".to_string(), Some(1020))];
        let taken: Vec<String> = ["q-c1", "q-c2", "q-b1"].iter().map(|s| s.to_string()).collect();
        let short = short_train_projects_in(&all, &approved, &taken, 1030, 20);
        assert_eq!(short.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["crash-sim"], "{short:?}");
        assert!(short[0].1.contains("no other entry"), "{short:?}");
        // the train that forms is BOTH crash-sim entries (a train of 2 of at most 4), not one, and broadsword's waits
        let items = vec![item("crash-sim", &["a"]), item("crash-sim", &["b"]), item("broadsword", &["c"])];
        let ready = |p: &str| short.iter().any(|(s, _)| s == p);
        assert_eq!(pick_train_when(&items, 4, &ready), vec![0, 1]);
        // what still counts as left to run: queued, stale, running (interrupted) and a half-finished train merge, never a taken entry
        for (status, left) in [(QUEUED, true), (STALE, true), (RUNNING, true), (BLOCKED, false), (REJECTED, false), (MERGED, false), (APPROVED, false)] {
            let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c3", "crash-sim", status)];
            let got = short_train_projects_in(&all, &approved[..1], &["q-c1".to_string()], 1030, 20);
            assert_eq!(got.is_empty(), left, "{status}: {got:?}");
            let got = short_train_projects_in(&all, &approved[..1], &["q-c1".to_string(), "q-c3".to_string()], 1030, 20);
            assert_eq!(got.len(), 1, "{status} but already taken by this drain: not waited for: {got:?}");
        }
        let mut half = entry("q-c3", "crash-sim", APPROVED);
        half.set("step", "main_check");
        assert!(short_train_projects_in(&[entry("q-c1", "crash-sim", APPROVED), half], &approved[..1], &["q-c1".to_string()], 1030, 20).is_empty(), "a half-finished train merge is still to run");
        // another project's queued entry never holds a project back
        assert_eq!(short_train_projects_in(&[entry("q-x", "other", QUEUED)], &approved[..1], &[], 1030, 20).len(), 1);
        // no approved entry: no train
        assert!(short_train_projects_in(&all, &[], &taken, 1030, 20).is_empty());
    }

    #[test]
    fn the_wait_rule_fires_once_the_oldest_approved_entry_has_waited_merge_train_wait_minutes() {
        // entries of the project are still queued, so only the wait rule can start its train; `now` is the injected clock
        let at = [Some(10_000u64), Some(10_500)];
        assert_eq!(short_train_why(true, &at, 10_000 + 20 * 60 - 1, 20), None, "one second short of 20 minutes");
        let why = short_train_why(true, &at, 10_000 + 20 * 60, 20).expect("20 minutes after the OLDEST approval");
        assert!(why.contains("1200 s") && why.contains("merge_train_wait_minutes = 20"), "{why}");
        assert!(short_train_why(true, &at, 10_000 + 5 * 60, 5).is_some(), "the setting is read, not a constant");
        assert!(short_train_why(true, &at, 10_000 + 5 * 60 - 1, 5).is_none());
        assert_eq!(short_train_why(true, &[Some(10_000)], 9_000, 20), None, "a clock that went back is no wait");
        assert!(short_train_why(true, &[Some(10_000), None], 10_001, 20).is_some(), "no readable approval time: counts as waited");
        assert_eq!(short_train_why(true, &[], u64::MAX, 20), None, "nothing approved");
        assert!(short_train_why(true, &[Some(0)], u64::MAX - 1, u64::MAX).is_none(), "a huge setting saturates instead of overflowing");
        // through the projects function and the real decision: queued work of the same project, the wait elapsed
        let all = vec![entry("q-c1", "crash-sim", APPROVED), entry("q-c2", "crash-sim", QUEUED)];
        let approved = vec![("crash-sim".to_string(), Some(10_000))];
        let taken = vec!["q-c1".to_string()];
        assert!(short_train_projects_in(&all, &approved, &taken, 10_000 + 19 * 60, 20).is_empty());
        let got = short_train_projects_in(&all, &approved, &taken, 10_000 + 20 * 60, 20);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].1.contains("has waited"), "{got:?}");
    }

    #[test]
    fn run_all_badness_orders_exit_codes_worst_first() {
        let mut codes = vec![0u8, 4, 2, 3, 5, 1, 6];
        codes.sort_by_key(|c| std::cmp::Reverse(badness(*c)));
        assert_eq!(codes, vec![6, 1, 5, 3, 2, 4, 0]);
    }

    #[test]
    fn run_all_exit_counts_the_stop_code_even_when_no_row_carries_it() {
        // a train check without a verdict: its entries stay approved (rows exit 3), the drain stopped with 1
        assert_eq!(worst_exit([3u8, 3, 0].into_iter(), EXIT_ERROR), EXIT_ERROR);
        assert_eq!(worst_exit([5u8, 3].into_iter(), EXIT_ERROR), EXIT_ERROR, "1 is worse than rejected / check_failed");
        assert_eq!(worst_exit([6u8, 3].into_iter(), EXIT_ERROR), EXIT_MERGED_BAD, "merged_check_failed stays the worst");
        assert_eq!(worst_exit([3u8, 0].into_iter(), 0), 3, "no stop: the worst row");
        assert_eq!(worst_exit(std::iter::empty(), 0), 0);
    }
}
