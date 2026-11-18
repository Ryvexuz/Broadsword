//! Drop-height sweep: default Config at several heights, CSV table on stdout.
use crash_sim::{run, Config};

/// (first_contact_s, peak_strain, bounce_apex_m) for one drop height.
/// Duplicated in tests/sweep.rs on purpose (tests cannot call example code).
fn measure(drop_height: f64) -> (f64, f64, f64) {
    let samples = run(&Config { drop_height, ..Config::default() });
    let contact = samples.iter().position(|s| s.min_y <= 0.0).expect("cube never touched the ground");
    let first_contact_s = samples[contact].t;
    let peak_strain = samples.iter().map(|s| s.max_beam_strain).fold(0.0, f64::max);
    let bounce_apex_m = samples[contact..].iter().map(|s| s.min_y).fold(0.0, f64::max);
    (first_contact_s, peak_strain, bounce_apex_m)
}

fn main() {
    println!("drop_height,first_contact_s,peak_strain,bounce_apex_m");
    for h in [0.5, 1.0, 2.0, 4.0, 8.0] {
        let (t, strain, apex) = measure(h);
        println!("{h},{t:.4},{strain:.6},{apex:.6}");
    }
}
