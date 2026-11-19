//! File-backed record store: one small file per job.

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use crate::config::Config;
use crate::paths::{record_path, PathError};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Status {
    Pending,
    Claimed,
    Done,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Claimed => "claimed",
            Status::Done => "done",
        }
    }

    fn parse(s: &str) -> Option<Status> {
        [Status::Pending, Status::Claimed, Status::Done].into_iter().find(|v| v.as_str() == s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub status: Status,
    pub attempts: u32,
    pub lease_until: u64,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub enum StoreError {
    Path(PathError),
    Duplicate(String),
    NotFound(String),
    TooLarge { len: usize, max: usize },
    AlreadyClaimed(String),
    AlreadyDone(String),
    NotClaimed(String),
    Locked,
    Corrupt(String),
    Io(io::Error),
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<PathError> for StoreError {
    fn from(e: PathError) -> Self {
        StoreError::Path(e)
    }
}

/// Held while a record is being changed; the lock file goes away when it is dropped.
struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Drop cannot return an error, so the result of the removal is ignored.
        let _ = fs::remove_file(&self.path);
    }
}

fn encode(rec: &Record) -> Vec<u8> {
    let head = format!("{} {} {} {}\n", rec.status.as_str(), rec.attempts, rec.lease_until, rec.payload.len() as u16);
    let mut out = head.into_bytes();
    out.extend_from_slice(&rec.payload);
    out
}

fn decode(id: &str, bytes: &[u8]) -> Result<Record, StoreError> {
    let bad = || StoreError::Corrupt(id.to_string());
    let nl = bytes.iter().position(|b| *b == b'\n').ok_or_else(bad)?;
    let head = std::str::from_utf8(&bytes[..nl]).map_err(|_| bad())?;
    let f: Vec<&str> = head.split(' ').collect();
    if f.len() != 4 {
        return Err(bad());
    }
    let len: usize = f[3].parse().map_err(|_| bad())?;
    let payload = bytes[nl + 1..].to_vec();
    if payload.len() != len {
        return Err(bad());
    }
    Ok(Record {
        status: Status::parse(f[0]).ok_or_else(bad)?,
        attempts: f[1].parse().map_err(|_| bad())?,
        lease_until: f[2].parse().map_err(|_| bad())?,
        payload,
    })
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) -> io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect(root, &path, out)?;
        } else if path.extension().map_or(false, |e| e == "job") {
            if let Ok(rel) = path.strip_prefix(root) {
                let parts: Vec<String> =
                    rel.with_extension("").components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                out.push(parts.join("/"));
            }
        }
    }
    Ok(())
}

pub struct Store {
    root: PathBuf,
    cfg: Config,
}

impl Store {
    pub fn open(cfg: &Config) -> Result<Store, StoreError> {
        fs::create_dir_all(&cfg.queue_dir)?;
        Ok(Store { root: cfg.queue_dir.clone(), cfg: cfg.clone() })
    }

    /// Adds a new pending job. Fails with `Duplicate` if the id is already in use.
    pub fn put(&self, id: &str, payload: &[u8]) -> Result<(), StoreError> {
        if payload.len() > self.cfg.max_payload_bytes {
            return Err(StoreError::TooLarge { len: payload.len(), max: self.cfg.max_payload_bytes });
        }
        let path = self.path(id)?;
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let rec = Record { status: Status::Pending, attempts: 0, lease_until: 0, payload: payload.to_vec() };
        if path.exists() {
            return Err(StoreError::Duplicate(id.to_string()));
        }
        fs::write(&path, encode(&rec))?;
        Ok(())
    }

    /// Returns the stored record, or `Err(NotFound)` if the id is unknown.
    pub fn get(&self, id: &str) -> Result<Record, StoreError> {
        let path = self.path(id)?;
        self.load(&path, id)
    }

    /// Leases a job to a worker until `now + lease`. A live lease or a finished job is refused.
    pub fn claim(&self, id: &str, now: u64) -> Result<Record, StoreError> {
        self.acquire()?;
        let path = self.path(id)?;
        let mut rec = self.load(&path, id)?;
        match rec.status {
            Status::Done => return Err(StoreError::AlreadyDone(id.to_string())),
            Status::Claimed if now > rec.lease_until => return Err(StoreError::AlreadyClaimed(id.to_string())),
            _ => {}
        }
        rec.status = Status::Claimed;
        rec.attempts += 1;
        rec.lease_until = now + self.cfg.lease().as_secs();
        self.save(&path, &rec)?;
        self.release();
        Ok(rec)
    }

    /// Marks a claimed job as finished.
    pub fn ack(&self, id: &str) -> Result<(), StoreError> {
        let _lock = self.lock()?;
        let path = self.path(id)?;
        let mut rec = self.load(&path, id)?;
        if rec.status != Status::Claimed {
            return Err(StoreError::NotClaimed(id.to_string()));
        }
        rec.status = Status::Done;
        let _ = self.save(&path, &rec);
        Ok(())
    }

    /// Every job id, sorted.
    pub fn ids(&self) -> Result<Vec<String>, StoreError> {
        let mut out = Vec::new();
        collect(&self.root, &self.root, &mut out)?;
        out.sort();
        Ok(out)
    }

    fn path(&self, id: &str) -> Result<PathBuf, StoreError> {
        Ok(record_path(&self.root, id)?)
    }

    fn load(&self, path: &Path, id: &str) -> Result<Record, StoreError> {
        match fs::read(path) {
            Ok(bytes) => decode(id, &bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(StoreError::NotFound(id.to_string())),
            Err(e) => Err(e.into()),
        }
    }

    fn save(&self, path: &Path, rec: &Record) -> Result<(), StoreError> {
        fs::write(path, encode(rec))?;
        Ok(())
    }

    fn acquire(&self) -> Result<(), StoreError> {
        self.lock().map(std::mem::forget)
    }

    fn release(&self) {
        let _ = fs::remove_file(self.root.join(".lock"));
    }

    fn lock(&self) -> Result<LockGuard, StoreError> {
        let path = self.root.join(".lock");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => Ok(LockGuard { path }),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Err(StoreError::Locked),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static N: AtomicUsize = AtomicUsize::new(0);

    fn store() -> Store {
        let dir = std::env::temp_dir().join(format!("jobq-unit-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let cfg = Config {
            queue_dir: dir,
            api_token: "t".into(),
            max_attempts: 3,
            base_delay_ms: 10,
            max_delay_ms: 100,
            lease_secs: 30,
            max_payload_bytes: 1 << 20,
        };
        Store::open(&cfg).unwrap()
    }

    #[test]
    fn put_then_get() {
        let s = store();
        s.put("a", b"hello").unwrap();
        assert_eq!(s.get("a").unwrap().payload, b"hello");
    }

    #[test]
    fn second_put_is_a_duplicate() {
        let s = store();
        s.put("a", b"1").unwrap();
        assert!(matches!(s.put("a", b"2"), Err(StoreError::Duplicate(_))));
    }

    #[test]
    fn claim_then_ack() {
        let s = store();
        s.put("a", b"x").unwrap();
        assert_eq!(s.claim("a", 100).unwrap().attempts, 1);
        s.ack("a").unwrap();
        assert!(matches!(s.claim("a", 500), Err(StoreError::AlreadyDone(_))));
    }
}
