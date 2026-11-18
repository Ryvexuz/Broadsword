//! Physical invariants checked across a grid of configurations.

use crash_sim::{run_checked, Config, MAX_STRAIN_LIMIT};

/// 20 s: at low gravity and 4 m drop the cube is still rocking at 8 s (0.057 m of center_y motion in the last 0.5 s).
fn check_config(drop_height: f64, tilt: bool, gravity: f64) {
    let cfg = Config { duration: 20.0, drop_height, tilt, gravity, ..Config::default() };
    let name = format!("drop_height={drop_height} tilt={tilt} gravity={gravity} duration={}", cfg.duration);

    let (samples, res) = run_checked(&cfg);
    assert!(res.is_ok(), "[{name}] run_checked failed: {:?}", res.err());
    assert!(!samples.is_empty(), "[{name}] no samples");

    for s in &samples {
        assert!(s.min_y >= -0.05, "[{name}] ground penetration at t={:.3}: min_y={}", s.t, s.min_y);
        assert!(
            s.max_beam_strain < MAX_STRAIN_LIMIT,
            "[{name}] strain {} >= limit {MAX_STRAIN_LIMIT} at t={:.3}",
            s.max_beam_strain,
            s.t
        );
    }

    let last = samples.last().unwrap();
    let window = (0.5 / cfg.dt).round() as usize;
    assert!(samples.len() > window, "[{name}] too few samples ({}) for a 0.5 s window", samples.len());
    let tail = &samples[samples.len() - 1 - window..];
    let (lo, hi) = tail.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
        (lo.min(s.center_y), hi.max(s.center_y))
    });
    assert!(
        hi - lo < 1e-3,
        "[{name}] not at rest: center_y varies by {} m over last 0.5 s (t={:.3}..{:.3})",
        hi - lo,
        tail[0].t,
        last.t
    );
    assert!(
        (last.center_y - cfg.edge / 2.0).abs() < 0.05,
        "[{name}] final center_y={} not within 0.05 of {}",
        last.center_y,
        cfg.edge / 2.0
    );
}

#[test]
fn invariants_hold_across_configurations() {
    for &drop_height in &[0.5, 1.0, 2.0, 4.0] {
        for &tilt in &[true, false] {
            for &gravity in &[1.62, 9.81] {
                check_config(drop_height, tilt, gravity);
            }
        }
    }
}
