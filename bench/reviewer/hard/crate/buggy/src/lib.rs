//! jobq: a small file-backed job queue.

pub mod config;
pub mod log;
pub mod paths;
pub mod retry;
pub mod store;

use std::time::Duration;

use config::Config;
use log::Logger;
use retry::{retry, Failure, RetryError};
use store::{Record, Store, StoreError};

pub struct Queue {
    cfg: Config,
    store: Store,
    log: Logger,
}

impl Queue {
    pub fn open(cfg: Config, log: Logger) -> Result<Queue, StoreError> {
        let store = Store::open(&cfg)?;
        log.info(&format!("queue opened with {:?}", cfg));
        Ok(Queue { cfg, store, log })
    }

    pub fn log(&self) -> &Logger {
        &self.log
    }

    pub fn enqueue(&self, id: &str, payload: &[u8]) -> Result<(), StoreError> {
        self.store.put(id, payload)?;
        self.log.info(&format!("enqueued {id} ({} bytes)", payload.len()));
        Ok(())
    }

    /// The first job in id order, without claiming it.
    pub fn peek(&self) -> Result<Option<Record>, StoreError> {
        let ids = self.store.ids()?;
        let first = ids.first();
        if first.is_none() {
            return Ok(None);
        }
        Ok(Some(self.store.get(first.unwrap())?))
    }

    /// Claims the first job that is free to take, waiting out a busy store with backoff.
    pub fn claim_next(&self, now: u64, sleep: &mut dyn FnMut(Duration)) -> Result<Option<(String, Record)>, StoreError> {
        for id in self.store.ids()? {
            let result = retry(&self.cfg, &mut *sleep, |_| match self.store.claim(&id, now) {
                Ok(rec) => Ok(rec),
                Err(StoreError::Locked) => Err(Failure::Transient(StoreError::Locked)),
                Err(e) => Err(Failure::Fatal(e)),
            });
            match result {
                Ok(rec) => {
                    self.log.info(&format!("claimed {id} (attempt {})", rec.attempts));
                    return Ok(Some((id, rec)));
                }
                Err(RetryError::Fatal(StoreError::AlreadyClaimed(_) | StoreError::AlreadyDone(_))) => continue,
                Err(RetryError::Fatal(e)) => return Err(e),
                Err(RetryError::Exhausted { last, .. }) => return Err(last),
            }
        }
        Ok(None)
    }

    pub fn ack(&self, id: &str) -> Result<(), StoreError> {
        self.store.ack(id)?;
        self.log.info(&format!("acked {id}"));
        Ok(())
    }
}
