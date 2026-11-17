//! A small cross-process lock file, shared by `recover` (`state/recover.lock`) and the task board
//! (`state/tasks/board.lock`).
//!
//! The lock is a file created with `create_new` (exclusive) holding the owner's pid and process creation time.
//! A lock whose owner is gone is stale and is taken over with a rename / re-read / put-back dance, so a racer's
//! fresh lock is never thrown away. A lock with no readable owner counts as stale after 60 s (its creator was
//! killed between creating and writing it). A live lock is waited for briefly, then the caller's message says who
//! holds it. The lock is released when the `Lock` is dropped (only if the file is still exactly ours).

use crate::sys;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub struct Lock {
    path: PathBuf,
    content: String,
}

impl Drop for Lock {
    fn drop(&mut self) {
        if std::fs::read_to_string(&self.path).map(|c| c == self.content).unwrap_or(false) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// What happened when we tried to remove a lock we judged stale.
#[derive(Debug, PartialEq, Eq)]
pub enum Takeover {
    /// The stale lock is gone; try to create ours.
    Removed,
    /// What we moved aside was not the stale lock we had judged (it was someone's fresh lock): put back.
    PutBack,
    /// It was someone's fresh lock and we could not put it back because another holder exists.
    Busy,
    /// Somebody else removed it first.
    Gone,
}

/// Remove a lock judged stale (`seen` is the exact content we judged). The lock is first renamed to a
/// unique name, then the moved file is re-read: only if it is exactly `seen` is it discarded. Otherwise
/// it belongs to a racer who just took the lock, and it goes back with an exclusive create.
pub fn take_over_stale(path: &Path, seen: &str, tag: &str) -> Takeover {
    use std::io::Write;
    let aside = path.with_extension(format!("stale-{tag}"));
    if std::fs::rename(path, &aside).is_err() {
        return Takeover::Gone;
    }
    let moved = std::fs::read_to_string(&aside).unwrap_or_default();
    if moved == seen {
        let _ = std::fs::remove_file(&aside);
        return Takeover::Removed;
    }
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut f) => {
            let _ = f.write_all(moved.as_bytes()).and_then(|_| f.sync_all());
            let _ = std::fs::remove_file(&aside);
            Takeover::PutBack
        }
        Err(_) => Takeover::Busy, // the moved lock stays at `aside`; two holders may exist, so give up
    }
}

/// A lock is stale when its owner is gone (exact pid and creation time), or when it holds no readable
/// owner and is older than 60 s (a holder killed between creating and writing it).
pub fn lock_is_stale(path: &Path, seen: &str) -> bool {
    let mut lines = seen.lines();
    match lines.next().and_then(|l| l.parse::<u32>().ok()) {
        Some(pid) => !sys::is_alive(pid, lines.next().and_then(|l| l.parse::<u64>().ok())),
        None => std::fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(60)),
    }
}

/// Take the lock file at `path` (exclusive create holding pid and creation time), creating its folder if needed.
/// A stale lock is taken over; a live one is waited for up to `wait`, then `busy(owner_pid)` is the error.
pub fn acquire(path: &Path, wait: Duration, busy: &dyn Fn(Option<u32>) -> String) -> Result<Lock, String> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    let me = std::process::id();
    let content = format!("{me}\n{}\n", sys::start_time(me).map(|t| t.to_string()).unwrap_or_default());
    let deadline = Instant::now() + wait;
    let mut takeovers = 0u32;
    let mut denied: Option<String> = None;
    loop {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut f) => {
                f.write_all(content.as_bytes()).and_then(|_| f.sync_all()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
                return Ok(Lock { path: path.to_path_buf(), content });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            // Windows reports a lock file that a racer is just deleting as access denied: wait and retry.
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => denied = Some(format!("cannot create {}: {e}", path.display())),
            Err(e) => return Err(format!("cannot create {}: {e}", path.display())),
        }
        let seen = std::fs::read_to_string(path).unwrap_or_default();
        let owner = seen.lines().next().and_then(|l| l.parse::<u32>().ok());
        if lock_is_stale(path, &seen) && takeovers < 10 {
            takeovers += 1;
            match take_over_stale(path, &seen, &format!("{me}-{takeovers}")) {
                Takeover::Busy => return Err(busy(owner)),
                _ => continue,
            }
        }
        if Instant::now() >= deadline {
            return Err(match denied {
                Some(e) if !path.exists() => e, // really cannot create it (not a held lock)
                _ => busy(owner),
            });
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scratch").join(format!("bs-lockmod-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn busy(o: Option<u32>) -> String {
        format!("busy {o:?}")
    }

    #[test]
    fn acquire_excludes_then_releases_on_drop() {
        let d = dir("excl");
        let p = d.join("sub").join("x.lock");
        let a = acquire(&p, Duration::from_millis(0), &busy).unwrap();
        let e = acquire(&p, Duration::from_millis(50), &busy).err().unwrap();
        assert!(e.starts_with("busy Some("), "{e}");
        drop(a);
        assert!(!p.exists());
        drop(acquire(&p, Duration::from_millis(0), &busy).unwrap());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_dead_owners_lock_is_taken_over() {
        let d = dir("dead");
        let p = d.join("x.lock");
        std::fs::write(&p, "4000000000\n\n").unwrap();
        let l = acquire(&p, Duration::from_millis(0), &busy).unwrap();
        assert!(std::fs::read_to_string(&p).unwrap().starts_with(&std::process::id().to_string()));
        drop(l);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_fresh_empty_lock_is_waited_for_not_taken() {
        let d = dir("empty");
        let p = d.join("x.lock");
        std::fs::write(&p, "").unwrap();
        assert_eq!(acquire(&p, Duration::from_millis(60), &busy).err().as_deref(), Some("busy None"));
        assert!(p.exists());
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
