// Proof suite for the hard1 case. prove.sh copies this file into tests/ of a copy of each
// crate version, so `jobq` here is whichever version is being checked.
// Test names: bugNN_* must fail on the buggy version and pass on the clean one;
// decoyN_* must pass on both. (The mutation proof for bug 10 lives in prove.sh.)

use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Duration;

use jobq::config::{Config, ConfigError};
use jobq::log::Logger;
use jobq::paths::record_path;
use jobq::retry::backoff;
use jobq::store::{Status, Store, StoreError};
use jobq::Queue;

const TOKEN: &str = "tok-4f9a1c77-SECRET";

fn cfg_text(dir: &PathBuf) -> String {
    format!(
        "queue_dir = {}\napi_token = {TOKEN}\nmax_attempts = 5\nbase_delay_ms = 100\nmax_delay_ms = 5000\nlease_secs = 30\nmax_payload_bytes = 1048576\n",
        dir.display()
    )
}

fn fresh_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("proof-{name}"));
    if dir.exists() {
        fs::remove_dir_all(&dir).unwrap();
    }
    dir
}

fn config(name: &str) -> Config {
    let dir = fresh_dir(name);
    Config::parse(&cfg_text(&dir)).unwrap()
}

#[test]
fn bug01_max_attempts_upper_bound_is_inclusive() {
    let dir = fresh_dir("b01");
    let ten = cfg_text(&dir).replace("max_attempts = 5", "max_attempts = 10");
    assert_eq!(Config::parse(&ten).unwrap().max_attempts, 10);
    let eleven = cfg_text(&dir).replace("max_attempts = 5", "max_attempts = 11");
    assert!(matches!(Config::parse(&eleven), Err(ConfigError::Invalid { .. })));
}

#[test]
fn bug02_duplicate_put_race_has_one_winner() {
    let store = Arc::new(Store::open(&config("b02")).unwrap());
    let threads = 8;
    let rounds = 2000;
    let barrier = Arc::new(Barrier::new(threads));
    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let (store, barrier) = (store.clone(), barrier.clone());
            thread::spawn(move || {
                (0..rounds)
                    .map(|r| {
                        barrier.wait();
                        store.put(&format!("race-{r}"), format!("from {t}").as_bytes()).is_ok()
                    })
                    .collect::<Vec<bool>>()
            })
        })
        .collect();
    let results: Vec<Vec<bool>> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    for r in 0..rounds {
        let winners = results.iter().filter(|v| v[r]).count();
        assert_eq!(winners, 1, "round {r}: {winners} puts of the same id succeeded");
    }
}

#[test]
fn bug03_parent_dir_in_the_middle_is_refused() {
    let root = PathBuf::from("q");
    assert!(record_path(&root, "a/../../escape").is_err());
    assert!(record_path(&root, "a/b/../c").is_err());
    // The queue folder sits three levels below a folder this test owns, so a record that
    // escapes by two levels still lands inside `base`, which is removed before the assert.
    let base = fresh_dir("b03");
    let queue = base.join("x").join("y").join("q");
    let cfg = Config::parse(&cfg_text(&queue)).unwrap();
    let store = Store::open(&cfg).unwrap();
    let result = store.put("a/../../escape", b"x");
    fs::remove_dir_all(&base).unwrap();
    assert!(matches!(result, Err(StoreError::Path(_))));
}

#[test]
fn bug04_token_is_not_logged() {
    let cfg = config("b04");
    assert!(!format!("{:?}", cfg).contains(TOKEN));
    let q = Queue::open(cfg, Logger::memory()).unwrap();
    q.enqueue("a", b"x").unwrap();
    for line in q.log().lines() {
        assert!(!line.contains(TOKEN), "token in log line: {line}");
    }
}

#[test]
fn bug05_ack_reports_a_failed_write() {
    let cfg = config("b05");
    let store = Store::open(&cfg).unwrap();
    store.put("a", b"x").unwrap();
    store.claim("a", 100).unwrap();
    let file = cfg.queue_dir.join("a.job");
    let mut perm = fs::metadata(&file).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(&file, perm.clone()).unwrap();
    let result = store.ack("a");
    perm.set_readonly(false);
    fs::set_permissions(&file, perm).unwrap();
    assert!(result.is_err(), "ack returned Ok although the record could not be written");
    assert_eq!(store.get("a").unwrap().status, Status::Claimed);
}

#[test]
fn bug06_large_payload_round_trips() {
    let store = Store::open(&config("b06")).unwrap();
    let payload = vec![7u8; 70_000];
    store.put("big", &payload).unwrap();
    assert_eq!(store.get("big").unwrap().payload, payload);
}

#[test]
fn bug07_lease_is_in_seconds() {
    assert_eq!(config("b07").lease(), Duration::from_secs(30));
}

#[test]
fn bug08_live_lease_is_refused_and_expired_lease_is_taken() {
    let cfg = config("b08");
    let store = Store::open(&cfg).unwrap();
    fs::write(cfg.queue_dir.join("a.job"), "claimed 1 130 1\nx").unwrap();
    assert!(matches!(store.claim("a", 110), Err(StoreError::AlreadyClaimed(_))));
    let rec = store.claim("a", 140).unwrap();
    assert_eq!(rec.attempts, 2);
}

#[test]
fn bug09_failed_claim_does_not_block_the_next() {
    let store = Store::open(&config("b09")).unwrap();
    store.put("a", b"x").unwrap();
    assert!(matches!(store.claim("missing", 100), Err(StoreError::NotFound(_))));
    assert!(store.claim("a", 100).is_ok(), "store stayed locked after a failed claim");
}

// bug10 (a test that can never fail): checked by mutation in prove.sh.

#[test]
fn bug11_parse_doc_matches_unknown_key_behaviour() {
    let src = include_str!("../src/config.rs");
    let lines: Vec<&str> = src.lines().collect();
    let at = lines.iter().position(|l| l.contains("pub fn parse(")).unwrap();
    let doc: String = lines[..at]
        .iter()
        .rev()
        .take_while(|l| l.trim_start().starts_with("///"))
        .map(|l| l.to_lowercase())
        .collect();
    let accepts = Config::parse(&format!("{}colour = red\n", cfg_text(&fresh_dir("b11")))).is_ok();
    assert!(!accepts, "behaviour changed: unknown keys are accepted");
    assert!(doc.contains("rejected"), "doc does not say unknown keys are rejected: {doc}");
    assert!(!doc.contains("ignored"), "doc says unknown keys are ignored: {doc}");
}

#[test]
fn bug12_backoff_stays_capped_for_large_attempts() {
    let cfg = config("b12");
    for attempt in [63, 64, 65, 70, 1000] {
        assert_eq!(backoff(&cfg, attempt), cfg.max_delay(), "attempt {attempt}");
    }
    assert_eq!(backoff(&cfg, 0), Duration::from_millis(100));
}

#[test]
fn decoy1_peek_unwrap_is_safe() {
    let q = Queue::open(config("d1"), Logger::memory()).unwrap();
    assert!(q.peek().unwrap().is_none());
    q.enqueue("a", b"x").unwrap();
    assert_eq!(q.peek().unwrap().unwrap().payload, b"x");
}

#[test]
fn decoy2_u32_cast_is_range_checked() {
    let dir = fresh_dir("d2");
    let big = cfg_text(&dir).replace("max_attempts = 5", "max_attempts = 4294967296");
    assert!(matches!(Config::parse(&big), Err(ConfigError::Invalid { .. })));
    let max = cfg_text(&dir).replace("max_attempts = 5", "max_attempts = 4294967295");
    assert!(matches!(Config::parse(&max), Err(ConfigError::Invalid { reason: "must be 1 to 10", .. })));
}

#[test]
fn decoy3_log_write_failure_never_fails_the_caller() {
    let dir = fresh_dir("d3");
    fs::create_dir_all(&dir).unwrap();
    let log = Logger::to_file(dir.clone()); // a directory: every file write fails
    log.info("still recorded");
    assert_eq!(log.lines(), vec!["[INFO] still recorded"]);
}
