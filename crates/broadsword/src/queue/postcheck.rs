//! One train's full check of main, settled before another fast-forward or the drain's summary.

use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Instant;

use crate::config::Config;
use crate::job::{self, JobInfo, JobSpec};
use crate::perfgate;

use super::entry::load_entry;
use super::merge::green_revert;
use super::train::{fail_members, finish_member};
use super::view::short;
use super::{
    badness, code_num, row_for, Row, Run, EXIT_ERROR, EXIT_MERGED_BAD, MERGED_CHECK_FAILED, R,
};

struct Pending {
    members: Vec<(String, Instant)>,
    project: String,
    trunk: String,
    sha: String,
    train: String,
    check: JoinHandle<(u8, JobInfo)>,
}

// The drain owns the run lock. This lock additionally makes the single in-process slot indivisible,
// including its join and judgment: no second starter can see an empty slot during settlement.
static PENDING: Mutex<Option<Pending>> = Mutex::new(None);

pub(super) fn enabled(cfg: &Config, project: &str) -> bool {
    let baseline = perfgate::baseline_path(cfg, project);
    cfg.queue_main_check_async
        && cfg.merge_train_max > 1
        // Windows can report Ok(false) even beneath a regular file. Only an existing
        // directory makes absence checkable; metadata errors also keep this synchronous.
        && baseline
            .parent()
            .is_some_and(|p| std::fs::metadata(p).is_ok_and(|m| m.is_dir()))
        && matches!(baseline.try_exists(), Ok(false))
}

pub(super) fn check_spec(project: &str, flaky: Option<crate::flake::Entries>) -> JobSpec {
    JobSpec {
        flaky,
        kind: "check".into(),
        name: project.to_string(),
        command: None,
        first_step: 0,
        resumed_from: None,
        resume_count: 0,
        task: None,
    }
}

fn run_check(cfg: &Config, project: &str, flaky: Option<crate::flake::Entries>) -> (u8, JobInfo) {
    let mut info = JobInfo::default();
    let code = job::run_job_info(cfg, check_spec(project, flaky), &mut info);
    (code, info)
}

pub(super) fn start(
    cfg: &Config,
    train: &str,
    members: Vec<(Run<'_>, Instant)>,
    sha: &str,
    rows: &mut Vec<Row>,
    worst: &mut u8,
) -> u8 {
    let mut slot = PENDING.lock().unwrap_or_else(|p| p.into_inner());
    start_slot(cfg, &mut slot, train, members, sha, rows, worst)
}

fn start_slot(
    cfg: &Config,
    slot: &mut Option<Pending>,
    train: &str,
    mut members: Vec<(Run<'_>, Instant)>,
    sha: &str,
    rows: &mut Vec<Row>,
    worst: &mut u8,
) -> u8 {
    if slot.is_some() {
        return fail_members(
            cfg,
            members,
            "a main check is already pending; no second check was started",
            rows,
            worst,
        );
    }
    // All records, including the merge facts just written by merge_members, must be durable
    // before the worker can run. Even a partial save failure leaves every in-memory Run resumable.
    for (r, _) in &mut members {
        r.e.set("step", "main_check");
    }
    for (r, _) in &mut members {
        if r.set_step("main_check").is_err() {
            return fail_members(
                cfg,
                members,
                "could not save every train member at main_check; no background check was started",
                rows,
                worst,
            );
        }
    }
    let Some((last, _)) = members.last_mut() else {
        eprintln!("broadsword: cannot start a main check without train members");
        return EXIT_ERROR;
    };
    if let Err(c) = last.prepare_main_check() {
        return finish(cfg, train, members, Err(c), rows, worst);
    }
    let project = last.e.get("project").to_string();
    let flaky = crate::flake::for_check(cfg, &project, Some(last.e.get("pre_merge_sha")));
    let trunk = last.e.get("main_branch").to_string();
    let owned_cfg = cfg.clone();
    let owned_project = project.clone();
    let check = match std::thread::Builder::new()
        .spawn(move || run_check(&owned_cfg, &owned_project, flaky))
    {
        Ok(h) => h,
        Err(e) => {
            return fail_members(
                cfg,
                members,
                &format!("could not start the main check thread: {e}"),
                rows,
                worst,
            );
        }
    };
    *slot = Some(Pending {
        members: members
            .iter()
            .map(|(r, t)| (r.e.id().to_string(), *t))
            .collect(),
        project,
        trunk,
        sha: sha.to_string(),
        train: train.to_string(),
        check,
    });
    println!(
        "queue: main check of {} (train {train}) runs in the background; the next fast-forward waits for it",
        short(sha)
    );
    0
}

/// Current train members have not merged. Keep them approved, with rows, when a prior check
/// stopped the drain but none of the existing trunk/dirty/hold checks gave them another outcome.
pub(super) fn stop_members(
    cfg: &Config,
    members: Vec<(Run<'_>, Instant)>,
    rows: &mut Vec<Row>,
    worst: &mut u8,
    code: u8,
) -> u8 {
    for (mut r, t) in members {
        r.e.set(
            "reason",
            "approved, not merged: the preceding main check stopped the drain",
        );
        let c = match r.log(
            "train",
            "the preceding main check stopped the drain; waits for the next drain",
        ) {
            Ok(()) => 0,
            Err(c) => code_num(c),
        };
        finish_member(cfg, r, c, t, rows, worst);
    }
    code
}

pub(super) fn barrier_error(
    cfg: &Config,
    members: Vec<(Run<'_>, Instant)>,
    error: &str,
    rows: &mut Vec<Row>,
    worst: &mut u8,
    settled: u8,
) -> u8 {
    let code = fail_members(cfg, members, error, rows, worst);
    if settled == 0 {
        code
    } else {
        settled
    }
}

pub(super) fn settle(cfg: &Config, rows: &mut Vec<Row>, worst: &mut u8) -> u8 {
    let mut slot = PENDING.lock().unwrap_or_else(|p| p.into_inner());
    settle_slot(cfg, &mut slot, rows, worst)
}

fn settle_slot(
    cfg: &Config,
    slot: &mut Option<Pending>,
    rows: &mut Vec<Row>,
    worst: &mut u8,
) -> u8 {
    let Some(pending) = slot.take() else {
        return 0;
    };
    let result = pending.check.join();
    let mut members = Vec::new();
    let mut read_error = false;
    // Reload independently: one unreadable member must neither hide the first error nor prevent
    // a failed check from being judged (and reverted) by another member of the same train.
    for (id, t) in pending.members {
        let loaded = load_entry(cfg, &id).and_then(|e| {
            let dir = job::resolve_project(&cfg.root, &cfg.projects_dir, &pending.project)?;
            if e.get("project") != pending.project || e.get("main_branch") != pending.trunk {
                return Err(format!(
                    "entry {id} changed project or trunk while its main check was pending"
                ));
            }
            Ok(Run::new(cfg, e, dir))
        });
        match loaded {
            Ok(r) => members.push((r, t)),
            Err(e) => {
                eprintln!(
                    "broadsword: main check of {}: cannot reload {id}: {e}",
                    pending.sha
                );
                rows.push(row_for(cfg, &id, EXIT_ERROR, t));
                read_error = true;
                if badness(EXIT_ERROR) > badness(*worst) {
                    *worst = EXIT_ERROR;
                }
            }
        }
    }
    let code = match result {
        Err(_) => {
            eprintln!(
                "queue: main check of {} FAILED (job none; thread panicked)",
                short(&pending.sha)
            );
            fail_members(
                cfg,
                members,
                "the main check thread panicked; the check did not complete; resume at main_check",
                rows,
                worst,
            )
        }
        Ok((code, info)) => {
            println!(
                "queue: main check of {} {} (job {})",
                short(&pending.sha),
                if code == 0 { "passed" } else { "FAILED" },
                info.id.as_deref().unwrap_or("none")
            );
            if read_error && code == 0 {
                fail_members(
                    cfg,
                    members,
                    "the main check passed, but not every pending member could be reloaded",
                    rows,
                    worst,
                )
            } else if let Some((last, _)) = members.last_mut() {
                // Normally this is already the last member's main_sha. If that record was lost,
                // an earlier member still has the same pre_merge_sha and can judge the full range.
                last.e.set("main_sha", pending.sha);
                let judged = last.judge_main_check(code, info);
                finish(cfg, &pending.train, members, judged, rows, worst)
            } else {
                eprintln!(
                    "broadsword: main check could not be settled: no train member could be reloaded"
                );
                EXIT_ERROR
            }
        }
    };
    if read_error && code == 0 {
        EXIT_ERROR
    } else {
        code
    }
}

/// Same train judgment as merge_members: the last member judges the one result, the rest are
/// covered by it; only a successful check can reach the existing done step.
fn finish(
    cfg: &Config,
    train: &str,
    members: Vec<(Run<'_>, Instant)>,
    judged: R<()>,
    rows: &mut Vec<Row>,
    worst: &mut u8,
) -> u8 {
    let Some((last, _)) = members.last() else {
        return EXIT_ERROR;
    };
    let last_id = last.e.id().to_string();
    let reason = last.e.get("reason").to_string();
    let error = last.e.get("error").to_string();
    let n = members.len();
    let failed = judged.err().map(code_num);
    let mut hard = 0;
    for (i, (mut r, t)) in members.into_iter().enumerate() {
        let code = if let Some(code) = failed {
            if i == n - 1 {
                code
            } else if code == EXIT_MERGED_BAD {
                code_num(r.end(MERGED_CHECK_FAILED, &reason, EXIT_MERGED_BAD))
            } else {
                code_num(r.internal(&format!(
                    "the main check of merge train {train} did not complete: {error}"
                )))
            }
        } else {
            let covered = if i != n - 1 {
                r.log(
                    "main_check",
                    &format!("covered by the one main check of merge train {train} (run by entry {last_id}), which passed"),
                )
            } else {
                Ok(())
            };
            // merge_phase at the final pipeline step calls s_done only: it cannot re-merge or
            // start a second main check. Keep task/worktree completion on that existing path.
            code_num(match covered.and_then(|()| r.merge_phase(super::STEPS.len() - 1)) {
                Ok(c) | Err(c) => c,
            })
        };
        if matches!(code, EXIT_ERROR | EXIT_MERGED_BAD) && badness(code) > badness(hard) {
            hard = code;
        }
        finish_member(cfg, r, code, t, rows, worst);
    }
    if hard == EXIT_MERGED_BAD && green_revert(cfg, &last_id) {
        0
    } else {
        hard
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record;

    fn config() -> Config {
        Config::from_text(
            include_str!("../../../../broadsword.toml"),
            std::path::Path::new("postcheck-unused"),
        )
        .unwrap()
    }

    #[test]
    fn settle_with_nothing_pending_returns_0() {
        let mut rows = Vec::new();
        let mut worst = 0;
        assert_eq!(settle(&config(), &mut rows, &mut worst), 0);
        assert!(rows.is_empty());
        assert_eq!(worst, 0);
    }

    #[test]
    fn a_panicked_check_thread_is_an_internal_error() {
        let root = crate::scrub::planted::config_root("postcheck-panic");
        let mut cfg = Config::load_from(&root).unwrap();
        cfg.projects_dir = root.join("projects");
        std::fs::create_dir_all(cfg.projects_dir.join("p")).unwrap();
        std::fs::write(
            cfg.projects_dir.join("p/Cargo.toml"),
            "[package]\nname = \"p\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        let ids: Vec<_> = ["panic-first", "panic-last"]
            .iter()
            .map(|id| {
                let mut e = record::create_ext(&cfg.state_dir.join("queue"), id, "entry").unwrap();
                e.set("project", "p");
                e.set("main_branch", "main");
                e.set("step", "main_check");
                e.set("status", super::super::APPROVED);
                e.save().unwrap();
                e.id().to_string()
            })
            .collect();
        let mut slot = Some(Pending {
            members: ids.iter().map(|id| (id.clone(), Instant::now())).collect(),
            project: "p".into(),
            trunk: "main".into(),
            sha: "merged-tip".into(),
            train: "panic-train".into(),
            check: std::thread::spawn(|| panic!("a main check did not complete")),
        });
        let mut rows = Vec::new();
        let mut worst = 0;
        assert_eq!(
            settle_slot(&cfg, &mut slot, &mut rows, &mut worst),
            EXIT_ERROR
        );
        assert!(slot.is_none());
        assert_eq!(rows.len(), ids.len());
        assert!(rows.iter().all(|row| row.code == EXIT_ERROR));
        assert_eq!(worst, EXIT_ERROR);
        for id in ids {
            let e = load_entry(&cfg, &id).unwrap();
            assert_eq!(e.get("step"), "main_check");
            assert_ne!(e.status(), super::super::MERGED);
            assert!(e.get("error").contains("thread panicked"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn async_requires_the_setting_a_train_and_a_known_absent_baseline() {
        let root = crate::scrub::planted::config_root("postcheck-baseline");
        let mut cfg = Config::load_from(&root).unwrap();
        cfg.queue_main_check_async = true;
        cfg.merge_train_max = 4;
        let baseline = perfgate::baseline_path(&cfg, "p");
        let parent = baseline.parent().unwrap();
        std::fs::create_dir_all(parent).unwrap();
        assert!(enabled(&cfg, "p"));
        cfg.queue_main_check_async = false;
        assert!(!enabled(&cfg, "p"));
        cfg.queue_main_check_async = true;
        cfg.merge_train_max = 1;
        assert!(!enabled(&cfg, "p"));
        cfg.merge_train_max = 4;
        std::fs::write(&baseline, "{}").unwrap();
        assert!(
            !enabled(&cfg, "p"),
            "even an invalid baseline precludes background work"
        );
        std::fs::remove_file(&baseline).unwrap();
        std::fs::remove_dir(parent).unwrap();
        assert!(
            !enabled(&cfg, "p"),
            "a missing baseline parent must use the synchronous path"
        );
        std::fs::write(parent, "not a directory").unwrap();
        assert!(
            !enabled(&cfg, "p"),
            "a baseline under a regular file must use the synchronous path"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_second_start_cannot_replace_the_pending_check() {
        let root = crate::scrub::planted::config_root("postcheck-slot");
        let cfg = Config::load_from(&root).unwrap();
        let mut e =
            record::create_ext(&cfg.state_dir.join("queue"), "second-member", "entry").unwrap();
        e.set("project", "p");
        e.set("main_branch", "main");
        e.set("step", "merge");
        e.set("status", super::super::APPROVED);
        e.save().unwrap();
        let id = e.id().to_string();
        let mut slot = Some(Pending {
            members: Vec::new(),
            project: "p".into(),
            trunk: "main".into(),
            sha: "first-tip".into(),
            train: "first-train".into(),
            check: std::thread::spawn(|| (0, JobInfo::default())),
        });
        let code = start_slot(
            &cfg,
            &mut slot,
            "second-train",
            vec![(Run::new(&cfg, e, root.clone()), Instant::now())],
            "second-tip",
            &mut Vec::new(),
            &mut 0,
        );
        let pending = slot.take().expect("the first handle must not be discarded");
        assert_eq!(pending.sha, "first-tip");
        assert_eq!(pending.train, "first-train");
        assert_eq!(pending.check.join().unwrap().0, 0);
        assert_eq!(code, EXIT_ERROR);
        assert_eq!(load_entry(&cfg, &id).unwrap().get("step"), "merge");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_partial_member_save_never_starts_the_thread() {
        let root = crate::scrub::planted::config_root("postcheck-save");
        let cfg = Config::load_from(&root).unwrap();
        let mut first =
            record::create_ext(&cfg.state_dir.join("queue"), "saved-member", "entry").unwrap();
        first.set("step", "merge");
        first.set("status", super::super::APPROVED);
        first.save().unwrap();
        let id = first.id().to_string();
        let obstruction = root.join("not-a-directory");
        std::fs::write(&obstruction, "a record cannot be saved beneath this file").unwrap();
        let second = record::Record::parse(
            &obstruction.join("unsaved.entry"),
            "id = unsaved\nstatus = approved\nstep = merge\n",
        );
        let mut slot = None;
        let mut rows = Vec::new();
        let mut worst = 0;
        assert_eq!(
            start_slot(
                &cfg,
                &mut slot,
                "save-train",
                vec![
                    (Run::new(&cfg, first, root.clone()), Instant::now()),
                    (Run::new(&cfg, second, root.clone()), Instant::now()),
                ],
                "merged-tip",
                &mut rows,
                &mut worst,
            ),
            EXIT_ERROR
        );
        assert!(slot.is_none(), "no worker may run after only a partial save");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.code == EXIT_ERROR));
        let e = load_entry(&cfg, &id).unwrap();
        assert_eq!(e.get("step"), "main_check");
        assert_ne!(e.status(), super::super::MERGED);
        assert!(e.get("error").contains("could not save every train member"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
