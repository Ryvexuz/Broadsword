//! Retry with exponential backoff.

use std::time::Duration;

use crate::config::Config;

pub enum Failure<E> {
    /// Worth another try.
    Transient(E),
    /// Retrying cannot help.
    Fatal(E),
}

#[derive(Debug)]
pub enum RetryError<E> {
    Exhausted { attempts: u32, last: E },
    Fatal(E),
}

/// The wait after failed attempt number `attempt` (counting from 0): `base_delay_ms` doubled
/// once per earlier attempt, never above `max_delay_ms`.
pub fn backoff(cfg: &Config, attempt: u32) -> Duration {
    let factor = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_millis(cfg.base_delay_ms.saturating_mul(factor).min(cfg.max_delay_ms))
}

/// Calls `op` until it succeeds, fails fatally, or has failed `max_attempts` times in total.
pub fn retry<T, E>(
    cfg: &Config,
    sleep: &mut dyn FnMut(Duration),
    mut op: impl FnMut(u32) -> Result<T, Failure<E>>,
) -> Result<T, RetryError<E>> {
    let mut attempt = 0u32;
    loop {
        match op(attempt) {
            Ok(v) => return Ok(v),
            Err(Failure::Fatal(e)) => return Err(RetryError::Fatal(e)),
            Err(Failure::Transient(e)) => {
                attempt += 1;
                if attempt >= cfg.max_attempts {
                    return Err(RetryError::Exhausted { attempts: attempt, last: e });
                }
                sleep(backoff(cfg, attempt - 1));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_config() -> Config {
        Config {
            queue_dir: PathBuf::new(),
            api_token: "t".into(),
            max_attempts: 8,
            base_delay_ms: 100,
            max_delay_ms: 5000,
            lease_secs: 30,
            max_payload_bytes: 1 << 20,
        }
    }

    #[test]
    fn delays_double() {
        let cfg = test_config();
        let ms: Vec<u128> = (0..4).map(|a| backoff(&cfg, a).as_millis()).collect();
        assert_eq!(ms, vec![100, 200, 400, 800]);
    }

    #[test]
    fn backoff_stays_within_cap() {
        let cfg = test_config();
        for attempt in 0..70 {
            assert!(backoff(&cfg, attempt) <= cfg.max_delay());
        }
    }

    #[test]
    fn gives_up_after_max_attempts() {
        let cfg = test_config();
        let (mut calls, mut sleeps) = (0, 0);
        let r: Result<(), _> = retry(&cfg, &mut |_| sleeps += 1, |_| {
            calls += 1;
            Err(Failure::Transient("busy"))
        });
        assert!(matches!(r, Err(RetryError::Exhausted { attempts: 8, .. })));
        assert_eq!((calls, sleeps), (8, 7));
    }
}
