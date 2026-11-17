//! A cross-process lock, shared by `recover` (`state/recover.lock`) and the task board (`state/tasks/board.lock`).
//!
//! It is an OS file lock (`File::try_lock`, exclusive) held on an open handle. The operating system releases it the
//! moment the holder's process ends, however it ends, so there is no stale-lock detection, no takeover and no timeout
//! after a crash. The lock FILE is never deleted and its content is never used for any decision: it only holds the
//! holder's pid, written for diagnostics. A waiter polls `try_lock` with short sleeps up to the wait limit, then the
//! caller's message is the error. The lock is released when the `Lock` is dropped.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::Path;
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
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let mut file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => {
                // Diagnostics only: who holds it. A failure to write is harmless.
                let _ = file.set_len(0).and_then(|_| file.write_all(format!("{}\n", std::process::id()).as_bytes()));
                return Ok(Lock { file });
            }
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(e)) => return Err(format!("cannot lock {}: {e}", path.display())),
        }
        if Instant::now() >= deadline {
            // On Windows the locked bytes cannot be read by another handle, so the owner is often unknown.
            let owner = std::fs::read_to_string(path).ok().and_then(|t| t.trim().parse::<u32>().ok());
            return Err(busy(owner));
        }
        std::thread::sleep(POLL);
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
}
