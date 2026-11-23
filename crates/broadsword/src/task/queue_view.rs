use super::*;

// ---- the merge queue, as `start` sees it ---------------------------------------------------

/// Queue entry statuses (queue.rs) in which the entry is OPEN: `queue run` can still act on it and `queue add` refuses a second
/// entry for its task. This mirrors queue.rs's private `is_open` (queue.rs has no exported list); the unit test
/// `queue_status_lists_match_queue_rs` reads queue.rs and fails if its `is_open` or its status constants change.
/// A `running` entry whose runner died ("interrupted" in `queue list`) is still `running` here.
pub const OPEN_QUEUE_STATUSES: [&str; 5] = [queue::QUEUED, queue::RUNNING, queue::BLOCKED, queue::STALE, queue::APPROVED];
/// The open statuses in which a running task's coder is NOT at work, so `start` does not count it against max_agents: the
/// entry waits for (`queued`) or is in (`running`) a queue run, or has passed review and waits for a merge train
/// (`approved`). A `running` entry whose runner died stays here too: it
/// waits for `queue run` to resume it, not for its coder, so counting it as waiting is accepted. `blocked` (dirty worktree,
/// changes after review) and `stale` (branch moved, no checked commit) need the coder, so they are left out on purpose.
/// Every one of these is still open, so it counts for max_queued.
pub const WAITING_QUEUE_STATUSES: [&str; 3] = [queue::QUEUED, queue::RUNNING, queue::APPROVED];
/// The queue statuses in which an entry has ended: its task, if still running, is back with its coder (a fix round).
pub const ENDED_QUEUE_STATUSES: [&str; 7] = [queue::MERGED, queue::CONFLICT, queue::CHECK_FAILED, queue::PERF_FAILED, queue::REJECTED, queue::MERGED_CHECK_FAILED, queue::CANCELLED];

/// One open merge-queue entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenEntry {
    pub id: String,
    pub task: String,
    pub status: String,
}

/// Why an entry record cannot be used, or `None` if it is well-formed.
fn entry_problem(r: &Record) -> Option<String> {
    if r.id().is_empty() {
        return Some("it has no `id` (empty, half-written or not a queue entry)".into());
    }
    if r.get("task").is_empty() {
        return Some("it has no `task`".into());
    }
    let s = r.status();
    if !OPEN_QUEUE_STATUSES.contains(&s) && !ENDED_QUEUE_STATUSES.contains(&s) {
        return Some(format!("its status `{s}` is not a queue status"));
    }
    None
}

/// The open entries of `<state_dir>/queue/*.entry`, read WITHOUT the queue locks (records are saved atomically by rename).
/// A file that cannot be read or is not a well-formed entry is re-read with backoff (10 ms doubling, at most 250 ms apart)
/// until `queue_entry_read_wait_ms` has passed since the scan began (one budget for the whole scan, so a start never waits
/// longer than that in total while it holds the board lock), then is an error naming the file: never skipped, never counted
/// as closed. Why wait at all: a queue save is a temp file renamed over the entry, which takes milliseconds, but on Windows a
/// read that meets the rename can briefly fail (sharing violation) and an antivirus scan can hold a new file for hundreds of
/// milliseconds; a writer other than record.rs could also leave a short-lived partial file. No queue folder is an empty queue.
pub fn open_queue_entries(cfg: &Config) -> Result<Vec<OpenEntry>, String> {
    let deadline = std::time::Instant::now() + Duration::from_millis(cfg.queue_entry_read_wait_ms);
    let dir = queue::queue_dir(cfg);
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("cannot list {}: {e}", dir.display())),
    };
    let mut out = Vec::new();
    for e in rd {
        let p = e.map_err(|e| format!("cannot list {}: {e}", dir.display()))?.path();
        if p.extension().is_none_or(|x| x != "entry") {
            continue;
        }
        let mut last = String::new();
        let mut good = None;
        let mut pause = Duration::from_millis(10);
        loop {
            if !last.is_empty() {
                let now = std::time::Instant::now();
                if now >= deadline {
                    last.push_str(&format!(" (still so after waiting queue_entry_read_wait_ms = {} ms)", cfg.queue_entry_read_wait_ms));
                    break;
                }
                std::thread::sleep(pause.min(deadline - now));
                pause = (pause * 2).min(Duration::from_millis(250));
            }
            match record::load(&p) {
                Ok(r) => match entry_problem(&r) {
                    None => {
                        good = Some(r);
                        break;
                    }
                    Some(why) => last = format!("queue entry file {} is not usable: {why}", p.display()),
                },
                Err(e) => last = format!("queue entry file {} cannot be read: {e}", p.display()),
            }
        }
        let r = good.ok_or(last)?;
        if OPEN_QUEUE_STATUSES.contains(&r.status()) {
            out.push(OpenEntry { id: r.id().to_string(), task: r.get("task").to_string(), status: r.status().to_string() });
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

