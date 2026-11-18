//! Drop-height sweep checks: contact time follows free fall, impacts get no gentler with height.
use crash_sim::{run, Config, GRAVITY};

const HEIGHTS: [f64; 5] = [0.5, 1.0, 2.0, 4.0, 8.0];

/// (first_contact_s, peak_strain, bounce_apex_m) for one drop height.
/// Duplicated from examples/sweep.rs on purpose (tests cannot call example code).
fn measure(drop_height: f64) -> (f64, f64, f64) {
    let samples = run(&Config { drop_height, ..Config::default() });
    let contact = samples.iter().position(|s| s.min_y <= 0.0).expect("cube never touched the ground");
    let first_contact_s = samples[contact].t;
    let peak_strain = samples.iter().map(|s| s.max_beam_strain).fold(0.0, f64::max);
    let bounce_apex_m = samples[contact..].iter().map(|s| s.min_y).fold(0.0, f64::max);
    (first_contact_s, peak_strain, bounce_apex_m)
}

#[test]
fn contact_time_matches_free_fall_and_increases() {
    let dt = Config::default().dt;
    let mut prev = 0.0;
    for h in HEIGHTS {
        let (t, _, _) = measure(h);
        let ideal = (2.0 * h / GRAVITY).sqrt();
        assert!(t > prev, "contact time must increase with height: h={h} t={t} prev={prev}");
        // within 2% of sqrt(2h/g), plus one output step of slack
        assert!((t - ideal).abs() <= 0.02 * ideal + dt, "h={h}: t={t} vs ideal {ideal}");
        prev = t;
    }
}

#[test]
fn peak_strain_does_not_decrease_with_height() {
    let low = measure(HEIGHTS[0]).1;
    let high = measure(HEIGHTS[4]).1;
    assert!(high >= low, "peak strain fell from {low} (0.5 m) to {high} (8 m)");
}
