// Test helper copied next to the hidden tests by harness.py (tests/common/mod.rs).
// `within` runs a closure on its own thread and panics if it does not finish in time,
// so a looping solution fails its test instead of hanging the run.
#![allow(dead_code)]

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

pub fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let secs: u64 = std::env::var("CODER_BENCH_TEST_TIMEOUT_S")
        .expect("CODER_BENCH_TEST_TIMEOUT_S not set by the harness")
        .parse()
        .expect("CODER_BENCH_TEST_TIMEOUT_S must be a whole number");
    let (tx, rx) = mpsc::channel();
    // 64 MiB stack: a deeply nested input must not crash a correct recursive solution.
    thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let _ = tx.send(r);
        })
        .expect("cannot start the test thread");
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(Ok(v)) => v,
        Ok(Err(p)) => std::panic::resume_unwind(p),
        Err(_) => panic!("timed out after {secs}s"),
    }
}
