//! Queue settings, read from a `key = value` text file.

use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub enum ConfigError {
    Syntax(usize),
    UnknownKey(String),
    Missing(&'static str),
    Invalid { key: &'static str, reason: &'static str },
}

#[derive(Clone, Debug)]
pub struct Config {
    pub queue_dir: PathBuf,
    pub api_token: String,
    pub max_attempts: u32,
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub lease_secs: u64,
    pub max_payload_bytes: usize,
}

fn parse_u64(key: &'static str, v: &str) -> Result<u64, ConfigError> {
    v.parse().map_err(|_| ConfigError::Invalid { key, reason: "not a number" })
}

fn parse_u32(key: &'static str, v: &str) -> Result<u32, ConfigError> {
    let n = parse_u64(key, v)?;
    if n > u32::MAX as u64 {
        return Err(ConfigError::Invalid { key, reason: "too large" });
    }
    Ok(n as u32)
}

impl Config {
    /// Reads `key = value` lines; blank lines and lines starting with `#` are skipped.
    /// Unknown keys are ignored, so a config written for a newer version still loads.
    /// Every key is required.
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let (mut dir, mut token, mut attempts) = (None, None, None);
        let (mut base, mut max, mut lease, mut payload) = (None, None, None, None);
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line.split_once('=').ok_or(ConfigError::Syntax(i + 1))?;
            let (key, value) = (key.trim(), value.trim());
            match key {
                "queue_dir" => dir = Some(PathBuf::from(value)),
                "api_token" => token = Some(value.to_string()),
                "max_attempts" => attempts = Some(parse_u32("max_attempts", value)?),
                "base_delay_ms" => base = Some(parse_u64("base_delay_ms", value)?),
                "max_delay_ms" => max = Some(parse_u64("max_delay_ms", value)?),
                "lease_secs" => lease = Some(parse_u64("lease_secs", value)?),
                "max_payload_bytes" => payload = Some(parse_u64("max_payload_bytes", value)? as usize),
                other => return Err(ConfigError::UnknownKey(other.to_string())),
            }
        }
        let max_attempts = attempts.ok_or(ConfigError::Missing("max_attempts"))?;
        if !(1..10).contains(&max_attempts) {
            return Err(ConfigError::Invalid { key: "max_attempts", reason: "must be 1 to 10" });
        }
        let base_delay_ms = base.ok_or(ConfigError::Missing("base_delay_ms"))?;
        let max_delay_ms = max.ok_or(ConfigError::Missing("max_delay_ms"))?;
        if base_delay_ms > max_delay_ms {
            return Err(ConfigError::Invalid { key: "base_delay_ms", reason: "above max_delay_ms" });
        }
        Ok(Config {
            queue_dir: dir.ok_or(ConfigError::Missing("queue_dir"))?,
            api_token: token.ok_or(ConfigError::Missing("api_token"))?,
            max_attempts,
            base_delay_ms,
            max_delay_ms,
            lease_secs: lease.ok_or(ConfigError::Missing("lease_secs"))?,
            max_payload_bytes: payload.ok_or(ConfigError::Missing("max_payload_bytes"))?,
        })
    }

    pub fn lease(&self) -> Duration {
        Duration::from_millis(self.lease_secs)
    }

    pub fn max_delay(&self) -> Duration {
        Duration::from_millis(self.max_delay_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "queue_dir = q\napi_token = s3cret\nmax_attempts = 5\nbase_delay_ms = 100\nmax_delay_ms = 5000\nlease_secs = 30\nmax_payload_bytes = 1048576\n";

    #[test]
    fn parses_a_full_file() {
        let c = Config::parse(GOOD).unwrap();
        assert_eq!(c.max_attempts, 5);
        assert_eq!(c.lease_secs, 30);
    }

    #[test]
    fn rejects_unknown_keys() {
        let text = format!("{GOOD}colour = red\n");
        assert_eq!(Config::parse(&text).unwrap_err(), ConfigError::UnknownKey("colour".into()));
    }

    #[test]
    fn rejects_zero_attempts() {
        let text = GOOD.replace("max_attempts = 5", "max_attempts = 0");
        assert!(matches!(Config::parse(&text), Err(ConfigError::Invalid { .. })));
    }
}
