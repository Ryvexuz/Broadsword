use crash_sim::{run, to_csv, Config, Sample, Sim, BEAM_STIFFNESS, GRAVITY, NODE_MASS};

fn default_run() -> Vec<Sample> {
    run(&Config::default())
}

/// Index of first sample where the lowest node touches the ground.
fn first_contact(s: &[Sample]) -> usize {
    s.iter().position(|r| r.min_y <= 1e-9).expect("cube never reached the ground")
}

/// Local maxima of center_y after first contact, ignoring wiggles under `prominence` metres.
fn bounce_peaks(s: &[Sample], prominence: f64) -> Vec<f64> {
    let mut peaks = Vec::new();
    let mut lo = s[first_contact(s)].center_y; // lowest since last peak
    let mut hi = lo; // highest since last trough
    let mut rising = false;
    for r in &s[first_contact(s)..] {
        let y = r.center_y;
        if rising {
            if y > hi {
                hi = y;
            } else if hi - y > prominence {
                peaks.push(hi);
                rising = false;
                lo = y;
            }
        } else if y < lo {
            lo = y;
        } else if y - lo > prominence {
            rising = true;
            hi = y;
        }
    }
    peaks
}

#[test]
fn free_fall_matches_analytic_height() {
    let cfg = Config::default();
    let s = run(&cfg);
    let c0 = s[0].center_y;
    let contact = first_contact(&s);
    assert!(contact > 10, "contact too early to test free fall");
    // Stop one output step before contact: still airborne.
    for r in &s[..contact - 1] {
        let expect = c0 - 0.5 * GRAVITY * r.t * r.t;
        // symplectic Euler has an O(h) position bias of 0.5*g*h*t (~2.5 mm at 0.5 s, h = 1 ms)
        assert!((r.center_y - expect).abs() < 0.005, "t={} got {} want {}", r.t, r.center_y, expect);
        assert!(r.max_beam_strain < 1e-9, "beams strained in free fall");
    }
    // first sample at 2 m minimum height
    assert!((s[0].min_y - cfg.drop_height).abs() < 1e-9);
}

#[test]
fn cube_has_28_beams_and_8_nodes() {
    let sim = Sim::new(Config::default());
    assert_eq!(sim.pos.len(), 8);
    assert_eq!(sim.beams.len(), 28);
}

#[test]
fn cube_hits_ground_and_bounces() {
    let s = default_run();
    let c = first_contact(&s);
    let peak = s[c..].windows(2).any(|w| w[1].center_y > w[0].center_y + 0.01);
    assert!(peak, "center_y never rose after first contact");
    assert!(!bounce_peaks(&s, 0.02).is_empty(), "no bounce peak found");
}

#[test]
fn bounces_lose_energy_and_cube_settles() {
    let s = default_run();
    let peaks = bounce_peaks(&s, 0.02);
    assert!(!peaks.is_empty());
    for w in peaks.windows(2) {
        assert!(w[1] < w[0], "bounce peak rose: {} -> {}", w[0], w[1]);
    }
    assert!(peaks[0] < s[0].center_y, "first bounce higher than the drop");

    let last = s.last().unwrap();
    assert!((last.center_y - 0.5).abs() < 0.02, "not resting on a face: center_y={}", last.center_y);
    assert!(last.min_y.abs() < 0.01);

    // Velocities small at the end: run again keeping the Sim to inspect it.
    let cfg = Config::default();
    let mut sim = Sim::new(cfg.clone());
    for _ in 0..(cfg.duration / cfg.dt).round() as usize {
        sim.advance();
    }
    assert!(sim.max_speed() < 0.05, "still moving: {} m/s", sim.max_speed());
}

#[test]
fn total_energy_never_exceeds_initial() {
    let cfg = Config::default();
    let mut sim = Sim::new(cfg.clone());
    let energy = |s: &Sim| {
        let ke: f64 = s.vel.iter().map(|v| 0.5 * NODE_MASS * v.dot(*v)).sum();
        let pe: f64 = s.pos.iter().map(|p| NODE_MASS * GRAVITY * p.y).sum();
        let spring: f64 = s
            .beams
            .iter()
            .map(|b| {
                let l = (s.pos[b.a] - s.pos[b.b]).length();
                0.5 * BEAM_STIFFNESS * (l - b.rest) * (l - b.rest)
            })
            .sum();
        ke + pe + spring
    };
    let e0 = energy(&sim);
    for _ in 0..(cfg.duration / cfg.dt).round() as usize {
        sim.advance();
        // 1% slack for symplectic Euler's bounded energy ripple
        assert!(energy(&sim) <= e0 * 1.01, "energy grew: {} > {}", energy(&sim), e0);
    }
}

#[test]
fn stable_no_nan_no_penetration_low_strain() {
    for tilt in [true, false] {
        let s = run(&Config { tilt, ..Config::default() });
        for r in &s {
            assert!(r.t.is_finite() && r.min_y.is_finite() && r.center_y.is_finite() && r.max_beam_strain.is_finite());
            assert!(r.min_y > -0.01, "node below ground: {}", r.min_y);
            assert!(r.max_beam_strain < 0.10, "strain {} at t={}", r.max_beam_strain, r.t);
        }
    }
}

#[test]
fn deterministic_output() {
    let a = to_csv(&default_run());
    let b = to_csv(&default_run());
    assert_eq!(a, b);
    assert!(a.starts_with("t,min_y,center_y,max_beam_strain\n"));
    assert_eq!(a.lines().count(), 502); // header + t = 0..5 s at 0.01
}
