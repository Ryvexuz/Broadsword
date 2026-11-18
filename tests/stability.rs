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
