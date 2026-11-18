use crash_sim::{Config, Sim, BEAM_STIFFNESS, NODE_MASS};

fn energies(sim: &Sim) -> (f64, f64, f64) {
    let kinetic: f64 = sim.vel.iter().map(|v| 0.5 * NODE_MASS * v.length().powi(2)).sum();
    let potential: f64 = sim.pos.iter().map(|p| NODE_MASS * sim.cfg.gravity * p.y).sum();
    let elastic: f64 = sim
        .beams
        .iter()
        .map(|b| {
            let ext = (sim.pos[b.a] - sim.pos[b.b]).length() - b.rest;
            0.5 * BEAM_STIFFNESS * ext * ext
        })
        .sum();
    (kinetic, potential, elastic)
}

fn total(sim: &Sim) -> f64 {
    let (k, p, e) = energies(sim);
    k + p + e
}

#[test]
fn start_has_no_kinetic_or_elastic_energy() {
    let sim = Sim::new(Config::default());
    let (k, p, e) = energies(&sim);
    assert!(k.abs() < 1e-9, "kinetic at t=0 was {k}");
    assert!(e.abs() < 1e-9, "elastic at t=0 was {e}");
    assert!(p > 0.0, "potential at t=0 should be positive, was {p}");
}

#[test]
fn free_fall_conserves_total_energy() {
    let mut sim = Sim::new(Config::default());
    let start = total(&sim);
    let mut checked = 0;
    // Free fall: every node is still above the ground, so no contact has happened yet.
    for _ in 0..1000 {
        // Stop before any node could reach the ground during the next output step,
        // so a contact inside a substep never slips between two checks.
        let min_y = sim.pos.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        if min_y <= sim.max_speed() * sim.cfg.dt + 0.01 {
            break;
        }
        sim.advance();
        let t = total(&sim);
        assert!(((t - start) / start).abs() < 0.01, "total {t} drifted from start {start}");
        checked += 1;
    }
    assert!(checked > 10, "only {checked} free-fall steps were checked");
}

#[test]
fn total_energy_drops_over_the_run() {
    let cfg = Config::default();
    let steps = (cfg.duration / cfg.dt).round() as u64;
    let mut sim = Sim::new(cfg);
    let start = total(&sim);
    for _ in 0..steps {
        sim.advance();
    }
    let end = total(&sim);
    assert!(end < start, "end energy {end} not below start {start}");
}
