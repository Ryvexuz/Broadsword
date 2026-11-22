//! A cross-process lock, shared by `recover` (`state/recover.lock`) and the task board (`state/tasks/board.lock`).
//!
//! It is an OS file lock (`File::try_lock`, exclusive) held on an open handle. The operating system releases it the
//! moment the holder's process ends, however it ends, so there is no stale-lock detection, no takeover and no timeout
//! after a crash. The lock FILE is never deleted and its content is never used for any decision: it only holds the
//! holder's pid, written for diagnostics. A waiter polls `try_lock` with short sleeps up to the wait limit, then the
//! caller's message is the error. The lock is released when the `Lock` is dropped. Build locks additionally use
//! `acquire_priority`: OS-locked waiter markers and a short-lived arbitration lock provide class priority and FIFO.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How often a waiter retries `try_lock`. Not a setting: it only trades a little CPU for latency, and no limit depends on it.
const POLL: Duration = Duration::from_millis(10);

pub struct Lock {
    file: File,
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.file.unlock(); // closing the handle (or the process ending) would release it anyway
    }
}

/// Take the lock on `path` (created if missing, with its folder), waiting up to `wait`. On timeout `busy(owner_pid)`
/// is the error; the pid is the holder's as written in the file, `None` if it cannot be read.
pub fn acquire(path: &Path, wait: Duration, busy: &dyn Fn(Option<u32>) -> String) -> Result<Lock, String> {
    let mut file = open(path)?;
    let started = Instant::now();
    loop {
        if try_take(&mut file, path)? {
            return Ok(Lock { file });
        }
        if started.elapsed() >= wait {
            return Err(busy(owner(path)));
        }
        std::thread::sleep(POLL);
    }
}

fn open(path: &Path) -> Result<File, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))
}

fn try_take(file: &mut File, path: &Path) -> Result<bool, String> {
    match file.try_lock() {
        Ok(()) => {
            // Diagnostics only: a failure to write is harmless.
            let _ = file
                .set_len(0)
                .and_then(|_| file.write_all(format!("{}\n", std::process::id()).as_bytes()));
            Ok(true)
        }
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(e)) => Err(format!("cannot lock {}: {e}", path.display())),
    }
}

fn owner(path: &Path) -> Option<u32> {
    // On Windows the locked bytes cannot be read by another handle, so the owner is often unknown.
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.trim().parse::<u32>().ok())
}

/// Selection order; tickets preserve arrival order within each class.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Priority {
    Human,
    Agent,
}

/// The arbitration files belong to the lock PATH, not to a package, project or job label.
fn queue_paths(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("lock path has no file name: {}", path.display()))?;
    let mut gate = name.to_os_string();
    gate.push(".queue");
    let mut waiters = name.to_os_string();
    waiters.push(".waiters");
    Ok((path.with_file_name(gate), path.with_file_name(waiters)))
}

fn ticket(name: &std::ffi::OsStr) -> Result<(Priority, u64), String> {
    let bad = || format!("invalid lock waiter marker: {:?}", name.to_string_lossy());
    let s = name.to_str().ok_or_else(bad)?;
    let priority = match s.as_bytes().first() {
        Some(b'h') => Priority::Human,
        Some(b'a') => Priority::Agent,
        _ => return Err(bad()),
    };
    // Fixed-width decimal is the format, not a configurable resource limit.
    if s.len() != 22 || s.as_bytes()[1] != b'-' || !s.as_bytes()[2..].iter().all(u8::is_ascii_digit) {
        return Err(bad());
    }
    let n = s[2..].parse::<u64>().map_err(|_| bad())?;
    if n == 0 {
        return Err(bad());
    }
    Ok((priority, n))
}

/// Scan only under the arbitration lock. Each marker's *OS lock*, not its contents or a pid-liveness guess, proves
/// the waiter is still present. Closing the last handle on death makes the marker stale, including after PID reuse.
/// Never delete a locked marker, and never write through an existing marker.
fn live_waiters(dir: &Path, own: Option<&Path>) -> Result<Vec<(Priority, u64)>, String> {
    let mut live = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("cannot list {}: {e}", dir.display()))?
    {
        let entry = entry.map_err(|e| format!("cannot list {}: {e}", dir.display()))?;
        let path = entry.path();
        let key = ticket(&entry.file_name())?;
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(format!(
                "lock waiter marker is not a regular file: {}",
                path.display()
            ));
        }
        if own == Some(path.as_path()) {
            live.push(key);
            continue;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {
                drop(file);
                std::fs::remove_file(&path).map_err(|e| {
                    format!("cannot remove stale lock waiter {}: {e}", path.display())
                })?;
            }
            Err(TryLockError::WouldBlock) => live.push(key),
            Err(TryLockError::Error(e)) => {
                return Err(format!("cannot probe lock waiter {}: {e}", path.display()))
            }
        }
    }
    Ok(live)
}

/// Take a priority lock. Registration and selection share a short-lived OS arbitration lock. Arrival means registration
/// under that lock: humans precede every registered agent, and each class is FIFO. The holder is never pre-empted.
/// The single wait clock starts before registration and is never reset when another waiter cuts in.
/// All callers of this resource, including zero-wait probes, must use this entry point.
/// Like `acquire`, this assumes a trusted state directory: replacing lock paths while in use is not supported.
pub fn acquire_priority(
    path: &Path,
    wait: Duration,
    priority: Priority,
    busy: &dyn Fn(Option<u32>) -> String,
) -> Result<Lock, String> {
    let started = Instant::now();
    let (gate_path, dir) = queue_paths(path)?;
    let mut file = open(path)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    // The lease closes on every return (or process death). The next scanner removes the now-stale marker.
    let mut lease: Option<(PathBuf, File, u64)> = None;
    loop {
        let gate_file = open(&gate_path)?;
        match gate_file.try_lock() {
            Ok(()) => {
                let _gate = Lock { file: gate_file };
                let mut live = live_waiters(&dir, lease.as_ref().map(|(p, _, _)| p.as_path()))?;
                if lease.is_none() {
                    let n = live
                        .iter()
                        .map(|(_, n)| *n)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)
                        .ok_or("lock waiter ticket overflow")?;
                    let prefix = if priority == Priority::Human { 'h' } else { 'a' };
                    let marker = dir.join(format!("{prefix}-{n:020}"));
                    let f = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create_new(true)
                        .open(&marker)
                        .map_err(|e| {
                            format!("cannot create lock waiter {}: {e}", marker.display())
                        })?;
                    f.try_lock().map_err(|e| {
                        format!("cannot lock new waiter {}: {e}", marker.display())
                    })?;
                    lease = Some((marker, f, n));
                    live.push((priority, n));
                }
                let own = lease.as_ref().map(|(_, _, n)| (priority, *n));
                if live.iter().min().copied() == own && try_take(&mut file, path)? {
                    return Ok(Lock { file });
                }
            }
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => {
                return Err(format!("cannot lock {}: {e}", gate_path.display()))
            }
        }
        if started.elapsed() >= wait {
            return Err(busy(owner(path)));
        }
        std::thread::sleep(POLL.min(wait.saturating_sub(started.elapsed())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn dir(tag: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-lockmod-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn busy(_: Option<u32>) -> String {
        "busy".into()
    }

    #[test]
    fn waiter_ticket_format_is_strict_and_bounded() {
        use std::ffi::OsStr;
        assert_eq!(
            ticket(OsStr::new("h-00000000000000000001")).unwrap(),
            (Priority::Human, 1)
        );
        assert_eq!(
            ticket(OsStr::new("a-18446744073709551615")).unwrap(),
            (Priority::Agent, u64::MAX)
        );
        for bad in [
            "",
            "h",
            "h-00000000000000000000",
            "a-18446744073709551616",
            "a-000000000000000000-1",
            "a-000000000000000000+1",
            "a-0000000000000000001\n",
            "a-0000000000000000001\r",
            "h-0000000000000000001/",
            r"h-0000000000000000001\",
            "C:\\h-00000000000000000001",
            "h-00000000000000000001\nh-00000000000000000002",
            "h-00000000000000000001 # comment",
        ] {
            assert!(
                ticket(OsStr::new(bad))
                    .unwrap_err()
                    .starts_with("invalid lock waiter marker"),
                "{bad:?}"
            );
        }
        assert!(ticket(OsStr::new(&"h".repeat(1_000_000))).is_err());
    }

    #[test]
    fn priority_errors_fail_closed_and_leave_the_resource_available_after_repair() {
        let d = dir("priority-errors");
        let p = d.join("build.lock");
        let (_, markers) = queue_paths(&p).unwrap();
        std::fs::create_dir_all(&markers).unwrap();
        let bad = markers.join("not-a-ticket");
        std::fs::write(&bad, "ignored").unwrap();
        assert!(
            acquire_priority(&p, Duration::ZERO, Priority::Human, &busy)
                .err()
                .unwrap()
                .starts_with("invalid lock waiter marker")
        );
        std::fs::remove_file(bad).unwrap();
        // A failure midway through registration also releases arbitration and never grants the build lock.
        let max = markers.join("h-18446744073709551615");
        let lease = acquire(&max, Duration::ZERO, &busy).unwrap();
        assert_eq!(
            acquire_priority(&p, Duration::ZERO, Priority::Agent, &busy)
                .err()
                .as_deref(),
            Some("lock waiter ticket overflow")
        );
        drop(lease);
        // Duration::MAX is valid: elapsed-time comparisons must not overflow an Instant deadline.
        drop(acquire_priority(&p, Duration::MAX, Priority::Agent, &busy).unwrap());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn arbitration_contention_uses_the_same_wait_budget() {
        let d = dir("priority-gate");
        let p = d.join("build.lock");
        let (gate, _) = queue_paths(&p).unwrap();
        let held = acquire(&gate, Duration::ZERO, &busy).unwrap();
        let wait = Duration::from_millis(50);
        let started = Instant::now();
        assert_eq!(
            acquire_priority(&p, wait, Priority::Human, &busy)
                .err()
                .as_deref(),
            Some("busy")
        );
        assert!(started.elapsed() >= wait);
        drop(held);
        drop(acquire_priority(&p, Duration::ZERO, Priority::Agent, &busy).unwrap());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn acquire_excludes_then_releases_on_drop_and_never_deletes_the_file() {
        let d = dir("excl");
        let p = d.join("sub").join("x.lock");
        let a = acquire(&p, Duration::from_millis(0), &busy).unwrap();
        assert_eq!(acquire(&p, Duration::from_millis(50), &busy).err().as_deref(), Some("busy"));
        drop(a);
        assert!(p.exists(), "the lock file stays");
        drop(acquire(&p, Duration::from_millis(0), &busy).unwrap());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn garbage_in_the_lock_file_means_nothing() {
        let d = dir("garbage");
        let p = d.join("x.lock");
        for junk in ["4000000000\n\n", "", "not a pid", "1\n1\n"] {
            std::fs::write(&p, junk).unwrap();
            drop(acquire(&p, Duration::from_millis(0), &busy).unwrap()); // no stale detection: only the OS lock counts
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn many_threads_never_hold_it_together() {
        let d = dir("threads");
        let p = d.join("x.lock");
        let inside = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hs: Vec<_> = (0..8)
            .map(|_| {
                let (p, inside) = (p.clone(), inside.clone());
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        let l = acquire(&p, Duration::from_secs(20), &busy).unwrap();
                        assert_eq!(inside.fetch_add(1, std::sync::atomic::Ordering::SeqCst), 0, "two holders at once");
                        std::thread::sleep(Duration::from_millis(1));
                        inside.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        drop(l);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn priority_waiters_never_hold_it_together() {
        let d = dir("priority-threads");
        let p = d.join("build.lock");
        let inside = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hs: Vec<_> = (0..8)
            .map(|i| {
                let (p, inside) = (p.clone(), inside.clone());
                std::thread::spawn(move || {
                    let priority = if i % 2 == 0 {
                        Priority::Human
                    } else {
                        Priority::Agent
                    };
                    for _ in 0..20 {
                        let l = acquire_priority(&p, Duration::from_secs(20), priority, &busy).unwrap();
                        assert_eq!(
                            inside.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
                            0,
                            "two priority holders at once"
                        );
                        std::thread::sleep(Duration::from_millis(1));
                        inside.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                        drop(l);
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        std::fs::remove_dir_all(d).unwrap();
    }
}
