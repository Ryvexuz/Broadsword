use crash_sim::{run_checked, to_csv, Config};
use std::io::Write;
use std::process::exit;

fn usage() -> ! {
    eprintln!("usage: crash-sim [--duration <s>] [--dt <s>] [--substeps <n>] [--drop-height <m>] [--gravity <m/s^2>] [--no-tilt]");
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
            "--gravity" => cfg.gravity = num(&mut args, "--gravity"),
            "--no-tilt" => cfg.tilt = false,
            _ => usage(),
        }
    }
    let (samples, result) = run_checked(&cfg);
    if !samples.is_empty() {
        let _ = std::io::stdout().write_all(to_csv(&samples).as_bytes());
    }
    if let Err(e) = result {
        eprintln!("crash-sim: {e}");
        exit(1);
    }
}
