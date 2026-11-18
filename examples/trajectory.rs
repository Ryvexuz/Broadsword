use crash_sim::{Config, Sim};

fn main() {
    let stride: u64 = match std::env::args().nth(1) {
        None => 1,
        Some(a) => match a.parse::<u64>() {
            Ok(n) if n >= 1 => n,
            _ => {
                eprintln!("error: stride must be a whole number >= 1, got '{a}'");
                std::process::exit(2);
            }
        },
    };
    let cfg = Config::default();
    cfg.validate().expect("default config must be valid");
    let steps = (cfg.duration / cfg.dt).round() as u64;
    let dt = cfg.dt;
    let mut sim = Sim::new(cfg);
    println!("t,node,x,y,z");
    for i in 0..=steps {
        if i > 0 {
            sim.advance();
        }
        if i % stride == 0 {
            for (n, p) in sim.pos.iter().enumerate() {
                println!("{:.4},{},{:.4},{:.4},{:.4}", i as f64 * dt, n, p.x, p.y, p.z);
            }
        }
    }
}
