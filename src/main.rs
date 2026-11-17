use crash_sim::{run, to_csv, Config};
use std::io::Write;
use std::process::exit;

fn usage() -> ! {
    eprintln!("usage: crash-sim [--duration <s>] [--dt <s>] [--substeps <n>] [--drop-height <m>] [--no-tilt]");
    exit(2)
}

fn num<T: std::str::FromStr>(args: &mut impl Iterator<Item = String>, flag: &str) -> T {
    match args.next().and_then(|v| v.parse().ok()) {
        Some(v) => v,
        None => {
            eprintln!("{flag} needs a numeric value");
            usage()
        }
    }
}

fn main() {
    let mut cfg = Config::default();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--duration" => cfg.duration = num(&mut args, "--duration"),
            "--dt" => cfg.dt = num(&mut args, "--dt"),
            "--substeps" => cfg.substeps = num(&mut args, "--substeps"),
            "--drop-height" => cfg.drop_height = num(&mut args, "--drop-height"),
            "--no-tilt" => cfg.tilt = false,
            _ => usage(),
        }
    }
    if !(cfg.dt > 0.0 && cfg.duration >= 0.0 && cfg.substeps >= 1 && cfg.drop_height >= 0.0) {
        eprintln!("dt must be > 0, duration >= 0, substeps >= 1, drop-height >= 0");
        usage()
    }
    let csv = to_csv(&run(&cfg));
    let _ = std::io::stdout().write_all(csv.as_bytes());
}
