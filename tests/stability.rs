use crash_sim::*;

#[test]
fn defaults_pass_validate() {
    assert!(Config::default().validate().is_ok());
}

const K: f64 = 20_000.0;
const M: f64 = 1.0;

/// Closed-form root of h^2 w2 + 2 h g = 4 for the stiffest mode (w2 = 8k/m, g = 8c/m).
fn closed_form(k: f64, c: f64, m: f64) -> f64 {
    let (w2, g) = (8.0 * k / m, 8.0 * c / m);
    (-g + (g * g + 4.0 * w2).sqrt()) / w2
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
fn limit_equals_the_closed_form_root() {
    for c in [0.0, 20.0, 28.3, 40.0, 66.0, 200.0] {
        let (limit, binding) = stable_step_limit_with_cause(K, c, M);
        assert!((limit - closed_form(K, c, M)).abs() < 1e-9, "c {c}: {limit} vs {}", closed_form(K, c, M));
        assert_eq!(limit, stable_step_limit(K, c, M));
        assert_eq!(binding, Binding::Coupled, "c {c}");
    }
    // undamped: the plain 2 / omega with omega = 400 rad/s
    assert!((stable_step_limit(K, 0.0, M) - 0.005).abs() < 1e-12);
}

#[test]
fn damper_alone_binds_only_without_a_spring() {
    let (limit, binding) = stable_step_limit_with_cause(0.0, 20.0, M);
    assert_eq!(binding, Binding::DamperAlone);
    assert!((limit - 2.0 / (8.0 * 20.0)).abs() < 1e-15, "{limit}");
    assert!(damped_step_error(limit, 0.0, 20.0, M).unwrap().contains("damper alone"));
    // with any spring the coupled root is below 2/g
    for c in [1.0, 100.0, 1e4] {
        assert!(stable_step_limit(K, c, M) < 2.0 / (8.0 * c / M));
    }
}

#[test]
fn crossover_with_the_gershgorin_bound_is_near_c_28() {
    let h = max_stable_step();
    assert!(stable_step_limit(K, 27.0, M) > h && stable_step_limit(K, 29.0, M) < h);
    // bisect for the crossover
    let (mut lo, mut hi) = (0.0, 100.0);
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if stable_step_limit(K, mid, M) > h { lo = mid } else { hi = mid }
    }
    assert!((lo - 28.3).abs() < 0.1, "crossover at c = {lo}");
}

#[test]
fn simulated_stiffest_mode_is_bounded_below_the_limit_and_diverges_above() {
    for c in [0.0, 20.0, 40.0, 66.0, 200.0] {
        let (w2, g) = (8.0 * K / M, 8.0 * c / M);
        let h = stable_step_limit(K, c, M);
        let below = oscillator_peak(w2, g, h * 0.99, 100_000);
        let above = oscillator_peak(w2, g, h * 1.05, 100_000);
        assert!(below < 10.0, "c {c}: below the limit the peak was {below}");
        assert!(above > 1e6, "c {c}: above the limit the peak was only {above}");
    }
}

#[test]
fn guard_refuses_exactly_the_unstable_steps_for_any_constants() {
    for (k, c, m) in [(K, 20.0, M), (K, 40.0, M), (K, 66.0, M), (5000.0, 100.0, 2.0)] {
        let limit = stable_step_limit(k, c, m);
        let (w2, g) = (8.0 * k / m, 8.0 * c / m);
        assert!(damped_step_error(limit * 0.99, k, c, m).is_none());
        assert!(oscillator_peak(w2, g, limit * 0.99, 100_000) < 10.0);
        let e = damped_step_error(limit * 1.01, k, c, m).expect("over the limit must be refused");
        assert!(e.contains("h = ") && e.contains("limit") && e.contains("h^2*w2"), "{e}");
        assert!(damped_step_error(limit, k, c, m).is_some(), "the limit itself is refused");
    }
}

/// The review's case. The d3c2235 guard accepted h when h <= max_stable_step() and
/// c*h/m < 0.25, separately. At c = 40 that accepts h = 0.0036 s, but the coupled limit is
/// 0.003385 s and the stiffest mode diverges.
#[test]
fn old_separate_checks_accepted_an_unstable_step_at_c_40() {
    let (c, h) = (40.0, 0.0036);
    let old_accepts = h <= max_stable_step() && c * h / M < 0.25;
    assert!(old_accepts, "the old checks should accept h = {h}");
    let limit = stable_step_limit(K, c, M);
    assert!(h > limit, "h {h} vs coupled limit {limit}");
    assert!(damped_step_error(h, K, c, M).is_some(), "the new guard must refuse it");
    let peak = oscillator_peak(8.0 * K / M, 8.0 * c / M, h, 100_000);
    assert!(peak > 1e6, "stiffest mode at h = {h}, c = {c} reached only {peak}");
    eprintln!("c = {c}: old guard accepts h = {h} s; coupled limit {limit:.6} s; stiffest-mode peak |x| = {peak:e}");
}

#[test]
fn defaults_validate_and_a_step_just_over_the_limit_is_refused() {
    let limit = stable_step_limit(BEAM_STIFFNESS, BEAM_DAMPING, NODE_MASS);
    // today the Gershgorin bound is the tighter one, so it is what actually trips
    assert!(max_stable_step() < limit);
    let under = Config { dt: max_stable_step() * 0.99, substeps: 1, ..Config::default() };
    assert!(under.validate().is_ok());
    let over = Config { dt: limit * 1.01, substeps: 1, ..Config::default() };
    assert!(over.validate().is_err());
    // and the step the coupled test alone would refuse is refused by the helper with the same constants
    assert!(damped_step_error(limit * 1.01, BEAM_STIFFNESS, BEAM_DAMPING, NODE_MASS).is_some());
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
