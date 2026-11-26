use super::*;

/// Test-only hook, like `job::TEST_FREE_HOOK_ENV`: when set to a file path inside this root's scratch folder, `start`
/// waits until that file exists before it touches the board, so a race test can start many processes and release them
/// together. It waits at most 3 x task_lock_wait_seconds (derived from that setting, so no separate limit exists), is
/// ignored for any path outside scratch, and announces itself on stderr. The long name is deliberate: it cannot be
/// set by accident.
pub const TEST_GATE_ENV: &str = "BROADSWORD_INTERNAL_TEST_TASK_START_GATE_FILE";

fn wait_for_test_gate(cfg: &Config) {
    wait_for_gate(cfg, TEST_GATE_ENV, "task start");
}

/// The shared test-hook body: wait for the file named by the env var `env` (scratch only, bounded, announced).
pub fn wait_for_gate(cfg: &Config, env: &str, what: &str) {
    let Ok(gate) = std::env::var(env) else { return };
    let gate = PathBuf::from(gate);
    let inside = match (gate.parent().and_then(|p| p.canonicalize().ok()), cfg.scratch_dir.canonicalize()) {
        (Some(p), Ok(s)) => p.starts_with(s),
        _ => false,
    };
    if !inside {
        return;
    }
    eprintln!("broadsword: TEST HOOK ACTIVE: {what} waits for {} to exist", gate.display());
    let end = std::time::Instant::now() + Duration::from_secs(cfg.task_lock_wait_seconds * 3);
    while !gate.exists() && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Test-only hook for the RAM line of `start`: names a file inside scratch holding a percentage (e.g. `95`). The reading
/// used is `max(hook, real)`, so the hook can only make RAM look fuller (stricter), never emptier. Files outside scratch
/// and unparsable values are ignored; use is announced on stderr. The long name is deliberate.
pub const TEST_RAM_HOOK_ENV: &str = "BROADSWORD_INTERNAL_TEST_RAM_USED_PERCENT_FILE";

/// (percent of RAM used, free bytes), from `sys::ram()` raised by the test hook if one is set.
fn ram_used_percent(cfg: &Config) -> Result<(f64, u64), String> {
    let r = crate::sys::ram()?;
    if r.total == 0 {
        return Err("the OS reported 0 bytes of RAM".into());
    }
    let mut used = (r.total - r.avail.min(r.total)) as f64 * 100.0 / r.total as f64;
    if let Ok(v) = std::env::var(TEST_RAM_HOOK_ENV) {
        let path = PathBuf::from(&v);
        let inside = match (path.canonicalize(), cfg.scratch_dir.canonicalize()) {
            (Ok(p), Ok(s)) => p.starts_with(s),
            _ => false,
        };
        if inside {
            eprintln!("broadsword: TEST HOOK ACTIVE: RAM use is read from {v} (never below the real reading)");
            if let Some(h) = std::fs::read_to_string(&path).ok().and_then(|t| t.trim().parse::<f64>().ok()) {
                used = used.max(h);
            }
        } else {
            eprintln!("broadsword: ignoring {TEST_RAM_HOOK_ENV}: file is not inside the scratch folder");
        }
    }
    Ok((used, r.avail))
}

pub fn cmd_start(cfg: &Config, id: &str, owner: &str, worktree: bool) -> ExitCode {
    let owner = owner.trim();
    if owner.is_empty() || owner.contains(['\n', '\r']) {
        return fail(EXIT_BAD_INPUT, "the owner must be a non-empty single line, e.g. coder-1");
    }
    wait_for_test_gate(cfg);
    let plan = {
        let _lock = match board_lock(cfg) {
            Ok(l) => l,
            Err(c) => return c,
        };
        let mut t = match load_task(cfg, id) {
            Ok(t) => t,
            Err(e) => return fail(EXIT_ERROR, e),
        };
        if let Err(e) = transition(t.status(), Action::Start) {
            return fail(EXIT_ERROR, format!("cannot start {id}: {e}"));
        }
        let all = match load_tasks(cfg) {
            Ok(a) => a,
            Err(e) => return fail(EXIT_ERROR, format!("cannot read the board, so nothing was started: {e}")),
        };
        // The memory line: no new agent while RAM use is at or above ram_warn_percent. Fails closed if RAM can't be read.
        match ram_used_percent(cfg) {
            Err(e) => return fail(EXIT_CONFLICT, format!("cannot start {id}: RAM could not be read, so the ram_warn_percent check cannot be made: {e}")),
            Ok((used, free)) if used >= cfg.ram_warn_percent as f64 => {
                return fail(EXIT_CONFLICT, format!("cannot start {id}: RAM is {used:.0}% used ({} free), at or above ram_warn_percent {}; start it when memory frees up", pause::gb(free), cfg.ram_warn_percent));
            }
            Ok(_) => {}
        }
        if let Err(reason) = crate::slot_gate::check_start(cfg) {
            return fail(
                EXIT_CONFLICT,
                format!("cannot start {id}: {reason}; start it when the merge check ends"),
            );
        }
        // The merge queue, read without its locks (read-only; every entry is written atomically). An entry that cannot be
        // read or makes no sense fails the start: counting it as closed could break either cap below.
        let open_entries = match open_queue_entries(cfg) {
            Ok(v) => v,
            Err(e) => return fail(EXIT_ERROR, format!("cannot start {id}: cannot read the merge queue, so the max_agents and max_queued caps cannot be checked: {e}")),
        };
        // The agent cap: one running task is one coder at work, unless its branch waits in the merge queue, i.e. it has an
        // open entry in a WAITING status (`queued`, `running` or `approved`, see WAITING_QUEUE_STATUSES). A `blocked` or `stale` entry is
        // open but needs its coder to act, so that task counts. Counted under the board lock, across every project.
        // Known design limit: the cap is checked only here, at start. Waiting tasks free their slots, so when many entries
        // end at once (e.g. mass rejections) their coders all return to fix rounds and up to max_agents + max_queued coders
        // can briefly be at work; new starts are refused until the count is back under max_agents.
        let waiting = |o: &Record| open_entries.iter().any(|e| e.task == o.id() && WAITING_QUEUE_STATUSES.contains(&e.status.as_str()));
        let running: Vec<&Record> = all.iter().filter(|o| o.status() == RUNNING).collect(); // this task is still `open` here
        let (queued, working): (Vec<&Record>, Vec<&Record>) = running.iter().partition(|o| waiting(o));
        if working.len() as u64 >= cfg.max_agents {
            let ids: Vec<String> = working.iter().map(|o| format!("{} ({})", o.id(), o.get("project"))).collect();
            return fail(EXIT_CONFLICT, format!("cannot start {id}: {} tasks are running with a working coder, the max_agents cap in broadsword.toml is {} (running tasks whose merge-queue entry is queued, running or approved are not counted; {} are): {}
start it after one of them is queued, done, failed or abandoned (`task abandon` frees the slot of a task whose agent died)", working.len(), cfg.max_agents, queued.len(), ids.join(", ")));
        }
        // The queue cap: so finished-but-unmerged work cannot pile up without limit. An ORPHAN entry (open, but its task is no
        // longer `running`: open, abandoned, done, failed, or not on the board) still counts: it is real open queue work that
        // `queue run` would act on, so skipping it would fail open. The refusal names each orphan and the command that clears it.
        if open_entries.len() as u64 >= cfg.max_queued {
            let ids: Vec<String> = open_entries.iter().map(|e| format!("{} (task {}, {})", e.id, e.task, e.status)).collect();
            let orphans: Vec<String> = open_entries
                .iter()
                .filter_map(|e| {
                    match all.iter().find(|o| o.id() == e.task).map(|o| o.status()) {
                        Some(RUNNING) => None,
                        Some(s) => Some(format!("entry {} is open but its task {} is {s}: clear it with `broadsword queue cancel {}`", e.id, e.task, e.id)),
                        None => Some(format!("entry {} is open but its task {} is not on the board: clear it with `broadsword queue cancel {}`", e.id, e.task, e.id)),
                    }
                })
                .collect();
            let orphan_text = if orphans.is_empty() { String::new() } else { format!("\n{}", orphans.join("\n")) };
            return fail(EXIT_CONFLICT, format!("cannot start {id}: {} merge-queue entries are open, the max_queued cap in broadsword.toml is {}: {}
start it after the queue has merged or ended some of them{orphan_text}", open_entries.len(), cfg.max_queued, ids.join(", ")));
        }
        let conflicts = find_conflicts(t.get("project"), &claims_of(&t), &all, Some(id));
        if !conflicts.is_empty() {
            let lines: Vec<String> = conflicts.iter().map(|c| format!("  {}", describe(c))).collect();
            return fail(EXIT_CONFLICT, format!("cannot start {id}: its claims overlap a running task:\n{}\nstart it after that task is done, failed or abandoned", lines.join("\n")));
        }
        // Every refusal a worktree can have is checked BEFORE the task is marked running (nothing to roll back then).
        let plan = if worktree {
            match worktree::plan(cfg, &t) {
                Ok(p) => Some(p),
                Err(e) => return fail(EXIT_ERROR, format!("cannot start {id} with a worktree: {e}\ntask {id} is still `open`; nothing was changed")),
            }
        } else {
            None
        };
        t.set("status", RUNNING);
        t.set("owner", owner);
        t.set("started", now_secs().to_string());
        if let Some(p) = &plan {
            worktree::mark_creating(&mut t, p); // claims are held from now on; the slow git step runs OUTSIDE the lock
        }
        if let Err(e) = save(&t) {
            return fail(EXIT_ERROR, e);
        }
        if plan.is_none() {
            println!("task {id} is running (owner {owner}); its claims are held until it is done, failed or abandoned");
        }
        plan
    };
    match plan {
        Some(p) => worktree::finish(cfg, id, p, worktree::Mode::Start { owner }),
        None => ExitCode::SUCCESS,
    }
}
