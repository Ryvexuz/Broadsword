use crash_sim::*;

#[test]
fn defaults_pass_validate() {
    assert!(Config::default().validate().is_ok());
}

#[test]
fn damping_limit_enforced_directly() {
    let edge = max_damping_step();
    assert!(damping_limit_error(edge * 0.99).is_none());
    let e = damping_limit_error(edge).expect("h at the limit must be rejected");
    assert!(e.contains("c*h/m"), "{e}");
    assert!(e.contains("--substeps") && e.contains("--dt"), "{e}");
    assert!(damping_limit_error(edge * 2.0).is_some());
}

#[test]
fn damping_limit_is_above_stiffness_limit_so_validate_hits_stiffness_first() {
    // No config breaks only the damping limit with the current constants.
    assert!(max_damping_step() > max_stable_step());
    let cfg = Config { dt: 0.1, substeps: 1, ..Config::default() };
    let e = cfg.validate().unwrap_err();
    assert!(e.contains("stable limit"), "{e}");
}

#[test]
fn stiffness_limit_still_enforced() {
    let h = max_stable_step();
    let ok = Config { dt: h * 0.99, substeps: 1, ..Config::default() };
    assert!(ok.validate().is_ok());
    let bad = Config { dt: h * 1.01, substeps: 1, ..Config::default() };
    assert!(bad.validate().is_err());
}

/// One damped oscillator x'' = -w2 x - g v stepped exactly like `Sim::substep`
/// (velocity first, then position with the new velocity). Returns the largest |x| seen.
fn oscillator_peak(w2: f64, g: f64, h: f64, steps: usize) -> f64 {
    let (mut x, mut v, mut peak) = (1.0_f64, 0.0_f64, 1.0_f64);
    for _ in 0..steps {
        v += (-w2 * x - g * v) * h;
        x += v * h;
        peak = peak.max(x.abs());
        if !peak.is_finite() || peak > 1e12 {
            break;
        }
    }
    peak
}

#[test]
fn damping_limit_is_a_quarter_not_one() {
    assert_eq!(DAMPING_STABILITY_LIMIT, 0.25);
    assert!((max_damping_step() - 0.25 * NODE_MASS / BEAM_DAMPING).abs() < 1e-15);
}

#[test]
fn pure_damping_limit_is_bounded_below_and_unbounded_above() {
    // Stiffest mode with the spring nearly off: g = 8 c / m, so h g < 2 is c h / m < 0.25.
    let g = STIFFEST_MODE_EIGENVALUE * BEAM_DAMPING / NODE_MASS;
    let w2 = 1.0; // tiny spring so the position still moves
    let h = max_damping_step();
    assert!(oscillator_peak(w2, g, h * 0.98, 5000) <= 1.0 + 1e-9, "below the limit must decay");
    assert!(oscillator_peak(w2, g, h * 1.02, 5000) > 1e6, "above the limit must blow up");
}

#[test]
fn coupled_limit_is_bounded_below_and_unbounded_above() {
    let w2 = STIFFEST_MODE_EIGENVALUE * BEAM_STIFFNESS / NODE_MASS;
    let g = STIFFEST_MODE_EIGENVALUE * BEAM_DAMPING / NODE_MASS;
    let h = coupled_stability_limit();
    assert!(oscillator_peak(w2, g, h * 0.98, 20000) < 2.0, "below the coupled limit must stay bounded");
    assert!(oscillator_peak(w2, g, h * 1.02, 20000) > 1e6, "above the coupled limit must blow up");
}

#[test]
fn everything_validate_accepts_is_inside_the_coupled_limit() {
    assert!(max_stable_step() < coupled_stability_limit());
    assert!(max_stable_step() < max_damping_step());
}

/// The stiffest-mode eigenvalue used above is the real one: power iteration on the
/// 24x24 matrix K = sum over beams of dir dir^T (spring k = 1) at the start shape.
#[test]
fn stiffest_mode_eigenvalue_is_eight() {
    let sim = Sim::new(Config { tilt: false, ..Config::default() });
    let mut m = [[0.0_f64; 24]; 24];
    for b in &sim.beams {
        let d = sim.pos[b.b] - sim.pos[b.a];
        let u = [d.x / d.length(), d.y / d.length(), d.z / d.length()];
        for i in 0..3 {
            for j in 0..3 {
                let w = u[i] * u[j];
                m[3 * b.a + i][3 * b.a + j] += w;
                m[3 * b.b + i][3 * b.b + j] += w;
                m[3 * b.a + i][3 * b.b + j] -= w;
                m[3 * b.b + i][3 * b.a + j] -= w;
            }
        }
    }
    let mut x: Vec<f64> = (0..24).map(|i| 1.0 + 0.37 * i as f64).collect();
    let mut lambda = 0.0;
    for _ in 0..500 {
        let y: Vec<f64> = (0..24).map(|i| (0..24).map(|j| m[i][j] * x[j]).sum()).collect();
        lambda = y.iter().map(|v| v * v).sum::<f64>().sqrt() / x.iter().map(|v| v * v).sum::<f64>().sqrt();
        let n = y.iter().map(|v| v * v).sum::<f64>().sqrt();
        x = y.iter().map(|v| v / n).collect();
    }
    assert!((lambda - STIFFEST_MODE_EIGENVALUE).abs() < 1e-6, "largest eigenvalue {lambda}");
}
