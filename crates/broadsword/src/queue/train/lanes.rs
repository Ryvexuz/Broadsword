//! Stacked area lanes: only checks run on worker threads; chaining and merging stay on the drain thread.

use super::*;
use std::time::Instant;

type Member<'a> = (Run<'a>, Instant);

pub(super) struct Batch<'a> {
    groups: Vec<Vec<Member<'a>>>,
    order: Vec<String>,
}

/// Keep the existing first ready train, then fill disjoint lanes in queue order.
/// `first` is the valid, non-overlapping group already chosen by `take_group`.
fn pick_lanes(
    items: &[(String, Vec<String>)],
    max: usize,
    lanes: usize,
    first: &[usize],
) -> Vec<Vec<usize>> {
    if max == 0 || lanes == 0 || first.is_empty() {
        return Vec::new();
    }
    let project = &items[first[0]].0;
    let mut out = vec![first.to_vec()];
    let mut used = first.to_vec();
    while out.len() < lanes {
        let mut next = Vec::new();
        for (i, (p, claims)) in items.iter().enumerate() {
            if p != project || used.contains(&i) || next.len() == max {
                continue;
            }
            if used.iter().any(|j| {
                items[*j]
                    .1
                    .iter()
                    .any(|a| claims.iter().any(|b| task::overlaps(a, b)))
            }) {
                continue;
            }
            used.push(i);
            next.push(i);
        }
        if next.is_empty() {
            break;
        }
        out.push(next);
    }
    out
}

/// A failed existence probe counts as a baseline: never bench beside another lane's tests.
pub(super) fn lane_count(cfg: &Config, project: &str) -> usize {
    if cfg.merge_train_lanes == 1 {
        return 1;
    }
    count_for_baseline(
        cfg.merge_train_lanes,
        crate::perfgate::baseline_path(cfg, project).try_exists(),
    )
}

fn count_for_baseline(lanes: u64, probe: std::io::Result<bool>) -> usize {
    if matches!(probe, Ok(false)) {
        usize::try_from(lanes).unwrap_or(usize::MAX)
    } else {
        1
    }
}

pub(super) fn queue_order(approved: &[Member<'_>]) -> Vec<String> {
    approved
        .iter()
        .map(|(r, _)| r.e.id().to_string())
        .collect()
}

pub(super) fn take_more<'a>(
    first: Vec<Member<'a>>,
    approved: &mut Vec<Member<'a>>,
    max: usize,
    count: usize,
    order: Vec<String>,
) -> Batch<'a> {
    let n = first.len();
    let items: Vec<_> = first
        .iter()
        .chain(approved.iter())
        .map(|(r, _)| (r.e.get("project").to_string(), r.claims.clone()))
        .collect();
    let indices = pick_lanes(&items, max, count, &(0..n).collect::<Vec<_>>());
    // Slots preserve indices while entries move into their lanes.
    let mut slots: Vec<_> = first
        .into_iter()
        .chain(approved.drain(..))
        .map(Some)
        .collect();
    let groups = indices
        .into_iter()
        .map(|lane| {
            lane.into_iter()
                .filter_map(|i| slots[i].take())
                .collect()
        })
        .collect();
    approved.extend(slots.into_iter().flatten());
    Batch { groups, order }
}

fn put_back<'a>(
    approved: &mut Vec<Member<'a>>,
    members: impl IntoIterator<Item = Member<'a>>,
    order: &[String],
) {
    approved.extend(members);
    approved.sort_by_key(|(r, _)| order.iter().position(|id| id == r.e.id()));
}

pub(super) fn run_lanes<'a>(
    cfg: &'a Config,
    batch: Batch<'a>,
    approved: &mut Vec<Member<'a>>,
    rows: &mut Vec<Row>,
) -> u8 {
    let Batch { groups, order } = batch;
    let mut worst = 0;
    // Recheck the baseline at the execution boundary as well as at selection.
    if let Some((r, _)) = groups.iter().flatten().next() {
        let count = lane_count(cfg, r.e.get("project"));
        if count == 1 || groups.len() > count {
            let mut pending = groups.into_iter();
            while let Some(group) = pending.next() {
                let code = run_train(cfg, group, rows);
                if code != 0 {
                    leave_approved(
                        cfg,
                        pending.flatten().collect(),
                        "an earlier serial train stopped",
                        rows,
                        &mut worst,
                    );
                    return code;
                }
            }
            return 0;
        }
    }
    let mut ready = Vec::new();
    for group in groups {
        let mut lane = Vec::new();
        for (mut r, t) in group {
            match r.train_ready() {
                Ok(()) => lane.push((r, t)),
                Err(c) => finish_member(cfg, r, code_num(c), t, rows, &mut worst),
            }
        }
        if !lane.is_empty() {
            ready.push(lane);
        }
    }
    let Some((first, _)) = ready.iter().flatten().next() else {
        return 0;
    };
    let head = match trunk_checkout(&first.dir, first.e.get("main_branch")) {
        Ok((_, head)) => head,
        Err(m) => {
            for (mut r, t) in ready.into_iter().flatten() {
                let c = r.leave_train(BLOCKED, &format!("{m}; nothing merged"), EXIT_RETRY);
                finish_member(cfg, r, code_num(c), t, rows, &mut worst);
            }
            return 0;
        }
    };
    static SEQ: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1);
    let id = format!(
        "tr-{}-lanes-{}",
        job::file_stamp(now_secs()),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    );
    let mut prev = head.clone();
    let mut chains: Vec<Vec<Member<'a>>> = Vec::new();
    let mut pending: std::collections::VecDeque<_> = ready.into();
    while let Some(lane) = pending.pop_front() {
        let mut chain = Vec::new();
        let mut members: std::collections::VecDeque<_> = lane.into();
        while let Some((mut r, t)) = members.pop_front() {
            match r.train_link(&prev, &id) {
                Ok(Some(tip)) => {
                    prev = tip;
                    chain.push((r, t));
                }
                Ok(None) => finish_member(cfg, r, EXIT_RETRY, t, rows, &mut worst),
                Err(c) => {
                    let code = code_num(c);
                    finish_member(cfg, r, code, t, rows, &mut worst);
                    if code == EXIT_ERROR {
                        let rest = chains
                            .into_iter()
                            .flatten()
                            .chain(chain)
                            .chain(members)
                            .chain(pending.into_iter().flatten())
                            .collect();
                        leave_approved(
                            cfg,
                            rest,
                            "the merge lanes stopped on an internal error while chaining",
                            rows,
                            &mut worst,
                        );
                        return code;
                    }
                }
            }
        }
        if !chain.is_empty() {
            chains.push(chain);
        }
    }
    let n = chains.len();
    if n == 0 {
        return 0;
    }
    let sizes = chains
        .iter()
        .map(|c| c.len().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    for (k, lane) in chains.iter_mut().enumerate() {
        for (r, _) in lane {
            r.e.set("train_lane", format!("{} of {n}", k + 1));
            if r.save().is_err() {
                return fail_members(
                    cfg,
                    chains.into_iter().flatten().collect(),
                    "could not record train_lane before starting the checks",
                    rows,
                    &mut worst,
                );
            }
        }
    }
    // All workers are started before any is joined. Each borrows only its own last Run.
    let verdicts = std::thread::scope(|scope| {
        let handles: Vec<_> = chains
            .iter_mut()
            .enumerate()
            .map(|(k, lane)| {
                let what = format!("merge lanes {id}: lane {} of {n}", k + 1);
                scope.spawn(move || {
                    let last = lane.len() - 1;
                    let tip = chain_tip(lane, lane.len());
                    lane[last].0.check_commit(&tip, &what)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join().unwrap_or_else(|_| {
                    TrainCheck::Blocked("a merge lane check worker panicked; no verdict".into())
                })
            })
            .collect::<Vec<_>>()
    });
    let passed = verdicts
        .iter()
        .take_while(|v| matches!(v, TrainCheck::Pass { .. }))
        .count();
    let verdict = match verdicts.get(passed) {
        Some(TrainCheck::Fail { .. }) => "failed",
        Some(_) => "no verdict",
        None => "passed",
    };
    // This is printed after the merge below so it describes actual fast-forwards, not an intention.
    let mut rest = chains.split_off(passed);
    if passed > 0 {
        let tip = chain_tip(&chains[passed - 1], chains[passed - 1].len());
        let TrainCheck::Pass { job, summary, tail } = &verdicts[passed - 1] else {
            return fail_members(
                cfg,
                chains
                    .into_iter()
                    .flatten()
                    .chain(rest.into_iter().flatten())
                    .collect(),
                "passing lane prefix lost its verdict",
                rows,
                &mut worst,
            );
        };
        let info = PassInfo {
            job: job.clone(),
            summary: summary.clone(),
            tail: tail.clone(),
            perf: String::new(),
        };
        let dir = chains[0][0].0.dir.clone();
        let trunk = chains[0][0].0.e.get("main_branch").to_string();
        let code = merge_members(
            cfg,
            &id,
            chains.into_iter().flatten().collect(),
            &tip,
            &head,
            &info,
            rows,
            &mut worst,
        );
        if code != 0 {
            leave_approved(
                cfg,
                rest.into_iter().flatten().collect(),
                "the passing prefix's merge stopped the drain",
                rows,
                &mut worst,
            );
            return code;
        }
        // merge_members can return 0 after refusing a fast-forward (dirty/moved/held trunk).
        // In that case do not retry a later lane on top of a prefix that never merged.
        match trunk_checkout(&dir, &trunk) {
            Ok((_, now)) if now == tip => {}
            Ok(_) => {
                leave_approved(
                    cfg,
                    rest.into_iter().flatten().collect(),
                    "the passing prefix did not reach main",
                    rows,
                    &mut worst,
                );
                return 0;
            }
            Err(m) => {
                leave_approved(
                    cfg,
                    rest.into_iter().flatten().collect(),
                    &m,
                    rows,
                    &mut worst,
                );
                eprintln!("broadsword: merge lanes {id}: {m}");
                return EXIT_ERROR;
            }
        }
    }
    println!(
        "merge lanes {id}: {n} lanes checked at once ({sizes} entries); lanes 1..{passed} merged in one fast-forward; lane {} {verdict}",
        passed + 1
    );
    if rest.is_empty() {
        return 0;
    }
    let failed = rest.remove(0);
    match verdicts.into_iter().nth(passed) {
        Some(TrainCheck::Fail { .. }) => {
            // Discard every speculative successor verdict: it included this failed lane.
            // Re-chain/bisect the first failure immediately on the new main, before another round.
            put_back(approved, rest.into_iter().flatten(), &order);
            run_train(cfg, failed, rows)
        }
        Some(TrainCheck::Blocked(msg)) => {
            leave_approved(
                cfg,
                failed.into_iter().chain(rest.into_iter().flatten()).collect(),
                &format!("a merge lane check could not run to a verdict: {msg}"),
                rows,
                &mut worst,
            );
            EXIT_ERROR
        }
        Some(TrainCheck::EntryBlocked(msg)) => {
            let mut others = failed;
            if let Some(member) = others.pop() {
                others.extend(rest.into_iter().flatten());
                entry_leaves_blocked(cfg, &id, member, &msg, others, rows, &mut worst);
            }
            0
        }
        _ => fail_members(
            cfg,
            failed.into_iter().chain(rest.into_iter().flatten()).collect(),
            "a merge lane has no check verdict",
            rows,
            &mut worst,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn item(project: &str, claims: &[&str]) -> (String, Vec<String>) {
        (project.into(), claims.iter().map(|s| s.to_string()).collect())
    }

    #[test]
    fn lanes_never_share_an_overlapping_claim() {
        let items = vec![
            item("p", &["src/a"]),
            item("p", &["broadsword.toml#a"]),
            item("other", &["other"]),
            item("p", &["src/a/child"]),
            item("p", &["broadsword.toml#b"]),
            item("p", &["src/b"]),
            item("p", &["*"]),
        ];
        let lanes = pick_lanes(&items, 2, 3, &[0, 1]);
        assert_eq!(lanes, vec![vec![0, 1], vec![4, 5]]);
        let chosen: Vec<_> = lanes.iter().flatten().copied().collect();
        for (pos, a) in chosen.iter().enumerate() {
            for b in &chosen[pos + 1..] {
                assert!(!items[*a]
                    .1
                    .iter()
                    .any(|x| items[*b].1.iter().any(|y| task::overlaps(x, y))));
            }
        }
    }

    #[test]
    fn a_lane_never_exceeds_merge_train_max() {
        let items: Vec<_> = (0..11)
            .map(|i| item("p", &[&format!("area{i}")]))
            .collect();
        assert_eq!(
            pick_lanes(&items, 2, 3, &[0, 1]),
            vec![vec![0, 1], vec![2, 3], vec![4, 5]]
        );
        assert_eq!(pick_lanes(&items, 2, 1, &[0, 1]), vec![vec![0, 1]]);
        assert!(pick_lanes(&items, 0, 3, &[]).is_empty());
        assert!(pick_lanes(&items, 2, 0, &[0, 1]).is_empty());
        assert_eq!(
            pick_lanes(&items, usize::MAX, usize::MAX, &[0]),
            vec![vec![0], (1..items.len()).collect::<Vec<_>>()]
        );
    }

    #[test]
    fn a_project_with_a_perf_baseline_gets_one_lane() {
        let mut cfg = Config::from_text(
            include_str!("../../../../../broadsword.toml"),
            Path::new("p"),
        )
        .unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../scratch")
            .join(format!("lane-baseline-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        cfg.state_dir = dir.clone();
        cfg.merge_train_lanes = 3;
        assert_eq!(lane_count(&cfg, "p"), 3);
        let baseline = crate::perfgate::baseline_path(&cfg, "p");
        std::fs::create_dir_all(baseline.parent().unwrap()).unwrap();
        std::fs::write(&baseline, "even an invalid baseline disables lanes").unwrap();
        assert_eq!(lane_count(&cfg, "p"), 1);
        assert_eq!(lane_count(&cfg, "other"), 3);
        cfg.merge_train_lanes = 1;
        assert_eq!(lane_count(&cfg, "other"), 1);
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert_eq!(
            count_for_baseline(3, Err(error)),
            1,
            "baseline probe errors fail closed"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
