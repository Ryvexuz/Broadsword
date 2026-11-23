use super::*;
use crate::lock::{self, Lock};
use std::time::Duration;

// ---- the lock ---------------------------------------------------------------------------

pub const EXIT_FAILED: u8 = 1;
pub const EXIT_BUSY: u8 = 3;

/// Take `state/build.lock`. A job holds it while cargo compiles (see the module comment): a build, test or check releases it
/// once its test executables are copied and runs the tests from the copies; a `run` holds it until its command ends. A second job waits up to `build_lock_wait_seconds`, then is
/// refused (exit 3, nothing built, cleaned or recorded) with a message that says how long it waited and who held it.
/// Registered human waiters precede agents; each class is FIFO. Every acquisition, including the nested-run probe,
/// goes through the same priority arbitration. Priority never changes the caller's configured wait budget.
///
/// Nested run (audit 2026-11-21 22Z fix 3): a `run` job holding this lock sets `scrub::RUN_LOCK_MARKER` = `<job id>:<pid>` on
/// its command (job.rs), after `run_proof` proved it. A Broadsword call inside that command that asks for this lock would
/// wait out the whole `build_lock_wait_seconds` for a lock its own parent holds until the call returns: a deadlock. So it is
/// refused at once (exit 3), but only when `nested_in_holder` proves the lock is held by that very run right now. The marker
/// never grants the lock: it is read only after one priority-aware try found the lock unavailable. With an available lock,
/// a malformed marker or any doubt about the holder (a stale marker, a stale record), the call behaves as without it.
pub fn lock(cfg: &Config, priority: lock::Priority) -> Result<Lock, (u8, String)> {
    let started = std::time::Instant::now();
    let wait = cfg.build_lock_wait_seconds;
    let path = cfg.state_dir.join("build.lock");
    let marker = std::env::var(scrub::RUN_LOCK_MARKER).ok();
    if let Some((job, pid)) = marker.as_deref().and_then(scrub::parse_run_lock_marker) {
        if let Some(l) = try_once(&path, priority)? {
            return Ok(l);
        }
        if nested_in_holder(cfg, marker.as_deref().unwrap_or_default()) {
            return Err((EXIT_BUSY, format!("build folder busy: this command runs inside `broadsword run` job {job}, which holds the build lock (pid {pid}); run it directly, not inside another run. Nothing was built, cleaned or recorded")));
        }
    }
    lock::acquire_priority(
        &path,
        Duration::from_secs(wait).saturating_sub(started.elapsed()),
        priority,
        &move |owner| {
            let who = owner.map(|p| format!(" (last recorded holder pid {p})")).unwrap_or_default();
            format!("build folder busy: the build lock stayed unavailable for the whole {wait} s wait (build_lock_wait_seconds){who}; nothing was built, cleaned or recorded. Repeat the call later (the lock frees itself when its holder ends)")
        },
    )
    .map_err(|e| {
        (
            if e.starts_with("build folder busy") {
                EXIT_BUSY
            } else {
                EXIT_FAILED
            },
            e,
        )
    })
}

/// The run proof: `state/build.lock.run.id` holds the marker of the last run that proved it held the build lock, and
/// `state/build.lock.run` is an OS lock that run holds from just AFTER it wrote that id until just BEFORE it lets the build
/// lock go (job.rs `BuildHold` drops its proof first). So the proof lock is held only by a run that holds the build lock, and
/// only one process holds the build lock at a time. (A second file because on Windows a locked file cannot be read.)
fn run_proof_paths(cfg: &Config) -> (PathBuf, PathBuf) {
    (cfg.state_dir.join("build.lock.run.id"), cfg.state_dir.join("build.lock.run"))
}

/// Called by a `run` job that HOLDS the build lock, before it starts its command: write `marker` as the id, then take the proof
/// lock. `Ok` = proved (the caller keeps the lock until just before it releases the build lock, and sets the marker);
/// `Err` = not proved (a write or lock error, named in the message). The caller must then NOT start the command (job.rs fails
/// the run): a command without the marker would bring back the nested-call deadlock silently (review q-20261121-222529).
pub fn run_proof(cfg: &Config, marker: &str) -> Result<Lock, String> {
    let (id, proof) = run_proof_paths(cfg);
    std::fs::write(&id, marker).map_err(|e| format!("cannot write {}: {e}", id.display()))?;
    // a nested call may hold a shared lock on it for an instant while it checks it (`nested_in_holder`)
    lock::acquire(&proof, Duration::from_secs(cfg.build_lock_wait_seconds), &|_| format!("{} stayed busy", proof.display()))
}

/// Is the build lock held right now by the run whose marker is `marker`? Yes only if the id reads `marker`, the proof lock is
/// held exclusively, and the id still reads `marker` afterwards. Why that is exact: only a build-lock holder writes the id, before it
/// takes the proof lock, and the proof holder holds the build lock, so while the proof is held the id is the holder's own.
/// A job id is unique and a run writes it once, so the id cannot read `marker` before and after the proof check unless the
/// run `marker` names held the proof between. A proof lock no run holds (the shared try succeeds), an id written by
/// a run that has let go (with a new holder that writes nothing, as in review q-20261121-220925) or any read error: no.
fn nested_in_holder(cfg: &Config, marker: &str) -> bool {
    let (id, proof) = run_proof_paths(cfg);
    let reads = || std::fs::read_to_string(&id).is_ok_and(|t| t == marker);
    if !reads() {
        return false;
    }
    // a SHARED try: two nested checks never block each other, only a run's exclusive proof blocks it
    let Ok(f) = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&proof) else { return false };
    match f.try_lock_shared() {
        Err(std::fs::TryLockError::WouldBlock) => reads(),
        Ok(()) => false, // released when `f` closes: no run proved it holds the build lock
        Err(std::fs::TryLockError::Error(_)) => false,
    }
}

const TRY_BUSY: &str = "busy";

/// One priority-aware try: `Some` when this waiter could take it (and now holds it), `None` when unavailable.
fn try_once(path: &Path, priority: lock::Priority) -> Result<Option<Lock>, (u8, String)> {
    match lock::acquire_priority(path, Duration::ZERO, priority, &|_| {
        TRY_BUSY.to_string()
    }) {
        Ok(l) => Ok(Some(l)),
        Err(e) if e == TRY_BUSY => Ok(None),
        Err(e) => Err((EXIT_FAILED, e)),
    }
}

