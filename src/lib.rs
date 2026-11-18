//! Starter crash sim: a cube of 8 nodes and 28 beams falling onto a ground plane.
//!
//! Simple on purpose: spring-damper beams, symplectic Euler, y = 0 ground with
//! restitution and friction. Std only, deterministic (no RNG, fixed step order).

pub mod vec3;

use vec3::Vec3;

pub const GRAVITY: f64 = 9.81;
pub const NODE_MASS: f64 = 1.0;
/// Beam stiffness (N/m). The highest cube mode is roughly sqrt(10 k / m), about
/// 400 rad/s, so at the default 1 ms substep omega*dt is about 0.40, well under
/// the symplectic Euler stability limit of 2.
pub const BEAM_STIFFNESS: f64 = 20_000.0;
/// Axial damping (N*s/m). Damps impact ringing within the 5 s run;
/// c*dt/m = 0.02 per substep, far below the explicit-damping limit of about 1.
pub const BEAM_DAMPING: f64 = 20.0;
/// Fraction of downward speed returned on ground contact.
pub const RESTITUTION: f64 = 0.5;
/// Horizontal speed loss rate (1/s) for a node touching the ground.
pub const FRICTION_RATE: f64 = 5.0;
/// Impact speeds below this (m/s) are treated as resting contact (no bounce),
/// otherwise gravity's per-substep velocity would keep re-bouncing forever.
pub const REST_SPEED: f64 = 0.2;
/// Strain above this means the run has blown up (or the cube is crushed); the sim aborts.
pub const MAX_STRAIN_LIMIT: f64 = 0.5;
/// Initial tilt in degrees about x and z when tilt is on.
pub const TILT_DEG: f64 = 5.0;

/// Largest physics step (s) that keeps symplectic Euler stable for the constants above.
///
/// Each node has 7 beams, so by Gershgorin the stiffness matrix has largest
/// eigenvalue <= 2 * 7 * k / m, i.e. omega_max <= sqrt(14 k / m) (about 529 rad/s;
/// the real cube mode is exactly 400). Symplectic Euler is stable for omega * h < 2.
/// This is a conservative bound, no extra margin needed.
pub fn max_stable_step() -> f64 {
    2.0 / (14.0 * BEAM_STIFFNESS / NODE_MASS).sqrt()
}

#[derive(Clone, Debug)]
pub struct Config {
    pub duration: f64,
    /// Output step in seconds.
    pub dt: f64,
    /// Physics substeps per output step.
    pub substeps: u32,
    /// Height of the lowest corner above the ground, metres.
    pub drop_height: f64,
    pub tilt: bool,
    pub edge: f64,
    /// Gravitational acceleration in m/s^2 (Earth 9.81, Moon 1.62, Mars 3.71).
    pub gravity: f64,
}

impl Default for Config {
    fn default() -> Self {
        Config { duration: 5.0, dt: 0.01, substeps: 10, drop_height: 2.0, tilt: true, edge: 1.0, gravity: GRAVITY }
    }
}

impl Config {
    /// Reject settings that cannot give a valid run, including unstable physics steps.
    pub fn validate(&self) -> Result<(), String> {
        let ok = self.dt.is_finite() && self.dt > 0.0;
        if !(ok && self.duration.is_finite() && self.duration >= 0.0 && self.substeps >= 1
            && self.drop_height.is_finite() && self.drop_height >= 0.0
            && self.gravity.is_finite() && self.gravity > 0.0)
        {
            return Err("dt must be > 0, duration >= 0, substeps >= 1, drop-height >= 0, gravity > 0 (all finite)".into());
        }
        let h = self.dt / self.substeps as f64;
        let limit = max_stable_step();
        if h > limit {
            return Err(format!(
 "physics step h = dt/substeps = {h:.6} s exceeds the stable limit {limit:.6} s (symplectic Euler needs omega_max*h < 2 with omega_max <= sqrt(14*k/m) = {:.0} rad/s, k = {BEAM_STIFFNESS}); raise --substeps or lower --dt",
                (14.0 * BEAM_STIFFNESS / NODE_MASS).sqrt()
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Beam {
    pub a: usize,
    pub b: usize,
    pub rest: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub t: f64,
    pub min_y: f64,
    pub center_y: f64,
    pub max_beam_strain: f64,
}

pub struct Sim {
    pub cfg: Config,
    pub pos: Vec<Vec3>,
    pub vel: Vec<Vec3>,
    pub beams: Vec<Beam>,
    step: u64,
}

impl Sim {
    pub fn new(cfg: Config) -> Sim {
        let e = cfg.edge;
        let mut pos: Vec<Vec3> = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 != 0 { e } else { 0.0 } - e / 2.0,
                    if i & 2 != 0 { e } else { 0.0 } - e / 2.0,
                    if i & 4 != 0 { e } else { 0.0 } - e / 2.0,
                )
            })
            .collect();
        if cfg.tilt {
            let (s, c) = TILT_DEG.to_radians().sin_cos();
            for p in pos.iter_mut() {
                // rotate about x, then about z (cube is centred on the origin)
                let (y, z) = (p.y * c - p.z * s, p.y * s + p.z * c);
                let (x, y) = (p.x * c - y * s, p.x * s + y * c);
                *p = Vec3::new(x, y, z);
            }
        }
        let lowest = pos.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let shift = cfg.drop_height - lowest;
        for p in pos.iter_mut() {
            p.y += shift;
        }
        // Every node pair is a beam: 12 edges + 12 face diagonals + 4 body diagonals = 28.
        let mut beams = Vec::new();
        for a in 0..8 {
            for b in (a + 1)..8 {
                beams.push(Beam { a, b, rest: (pos[a] - pos[b]).length() });
            }
        }
        Sim { cfg, pos, vel: vec![Vec3::ZERO; 8], beams, step: 0 }
    }

    fn substep(&mut self, h: f64) {
        let mut force = vec![Vec3::new(0.0, -self.cfg.gravity * NODE_MASS, 0.0); self.pos.len()];
        for bm in &self.beams {
            let d = self.pos[bm.b] - self.pos[bm.a];
            let len = d.length();
            if len < 1e-12 {
                continue;
            }
            let dir = d * (1.0 / len);
            let rel_v = (self.vel[bm.b] - self.vel[bm.a]).dot(dir);
            let mag = BEAM_STIFFNESS * (len - bm.rest) + BEAM_DAMPING * rel_v;
            let f = dir * mag;
            force[bm.a] += f;
            force[bm.b] += -f;
        }
        for i in 0..self.pos.len() {
            // semi-implicit Euler: velocity first, then position with the new velocity
            self.vel[i] += force[i] * (h / NODE_MASS);
            self.pos[i] += self.vel[i] * h;
            if self.pos[i].y < 0.0 {
                self.pos[i].y = 0.0;
                if self.vel[i].y < 0.0 {
                    let vy = self.vel[i].y;
                    self.vel[i].y = if -vy > REST_SPEED { -vy * RESTITUTION } else { 0.0 };
                }
                let keep = (1.0 - FRICTION_RATE * h).max(0.0);
                self.vel[i].x *= keep;
                self.vel[i].z *= keep;
            }
        }
    }

    pub fn sample(&self) -> Sample {
        let min_y = self.pos.iter().map(|p| p.y).fold(f64::INFINITY, f64::min);
        let center_y = self.pos.iter().map(|p| p.y).sum::<f64>() / self.pos.len() as f64;
        let max_beam_strain = max_strain(&self.beams, &self.pos);
        Sample { t: self.step as f64 * self.cfg.dt, min_y, center_y, max_beam_strain }
    }

    /// Fail if any node position/velocity is non-finite or strain is NaN or above the limit.
    pub fn check(&self) -> Result<(), String> {
        let finite = |v: &Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        if let Some(i) = self.pos.iter().position(|p| !finite(p)) {
            return Err(format!("node {i} position is not finite"));
        }
        if let Some(i) = self.vel.iter().position(|v| !finite(v)) {
            return Err(format!("node {i} velocity is not finite"));
        }
        let strain = max_strain(&self.beams, &self.pos);
        if strain.is_nan() || strain > MAX_STRAIN_LIMIT {
            return Err(format!("max beam strain {strain} exceeds limit {MAX_STRAIN_LIMIT}"));
        }
        Ok(())
    }

    /// Largest node speed, m/s.
    pub fn max_speed(&self) -> f64 {
        self.vel.iter().map(|v| v.length()).fold(0.0, f64::max)
    }

    /// Advance one output step (`substeps` physics substeps).
    pub fn advance(&mut self) {
        let h = self.cfg.dt / self.cfg.substeps as f64;
        for _ in 0..self.cfg.substeps {
            self.substep(h);
        }
        self.step += 1;
    }
}

/// Largest |length - rest| / rest. NaN-propagating: a blown-up state never reads as 0.
pub fn max_strain(beams: &[Beam], pos: &[Vec3]) -> f64 {
    let mut m = 0.0_f64;
    for b in beams {
        let s = ((pos[b.a] - pos[b.b]).length() - b.rest).abs() / b.rest;
        if s.is_nan() {
            return f64::NAN;
        }
        m = m.max(s);
    }
    m
}

/// Run the sim. Returns the samples of all good steps (t = 0, dt, 2dt, ...) and,
/// if the config was refused or the state blew up, the reason (with the time).
/// The step count is `round(duration / dt)`, so the last row can differ from
/// `duration` by up to dt/2 when dt does not divide it.
pub fn run_checked(cfg: &Config) -> (Vec<Sample>, Result<(), String>) {
    if let Err(e) = cfg.validate() {
        return (Vec::new(), Err(e));
    }
    let steps = (cfg.duration / cfg.dt).round() as u64;
    let mut sim = Sim::new(cfg.clone());
    let mut out = Vec::with_capacity(steps as usize + 1);
    out.push(sim.sample());
    for _ in 0..steps {
        sim.advance();
        if let Err(e) = sim.check() {
            return (out, Err(format!("unstable at t = {:.4} s: {e}", sim.sample().t)));
        }
        out.push(sim.sample());
    }
    (out, Ok(()))
}

/// Run the whole sim; panics if the config is refused or the sim blows up.
pub fn run(cfg: &Config) -> Vec<Sample> {
    let (samples, res) = run_checked(cfg);
    res.expect("simulation failed");
    samples
}

pub const CSV_HEADER: &str = "t,min_y,center_y,max_beam_strain";

pub fn to_csv(samples: &[Sample]) -> String {
    let mut s = String::from(CSV_HEADER);
    s.push('\n');
    for r in samples {
        s.push_str(&format!("{:.4},{:.6},{:.6},{:.6}\n", r.t, r.min_y, r.center_y, r.max_beam_strain));
    }
    s
}
