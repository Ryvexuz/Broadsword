use crash_sim::{Config, Sim, BEAM_STIFFNESS, NODE_MASS};

/// (kinetic, potential, elastic) energy in joules; potential is measured from y = 0.
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

fn main() {
    let cfg = Config::default();
    cfg.validate().expect("default config must be valid");
    let steps = (cfg.duration / cfg.dt).round() as u64;
    let dt = cfg.dt;
    let mut sim = Sim::new(cfg);
    println!("t,kinetic,potential,elastic,total");
    for i in 0..=steps {
        if i > 0 {
            sim.advance();
        }
        let (k, p, e) = energies(&sim);
        println!("{:.4},{:.6},{:.6},{:.6},{:.6}", i as f64 * dt, k, p, e, k + p + e);
    }
}
