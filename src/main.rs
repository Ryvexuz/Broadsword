use crash_sim::{run_checked_with_contact, to_csv, Config};
use std::io::Write;
use std::process::exit;

fn usage() -> ! {
    eprintln!("usage: crash-sim [--duration <s>] [--dt <s>] [--substeps <n>] [--drop-height <m>] [--gravity <m/s^2>] [--no-tilt] [--summary]");
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
    let mut summary = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--duration" => cfg.duration = num(&mut args, "--duration"),
            "--dt" => cfg.dt = num(&mut args, "--dt"),
            "--substeps" => cfg.substeps = num(&mut args, "--substeps"),
            "--drop-height" => cfg.drop_height = num(&mut args, "--drop-height"),
            "--gravity" => cfg.gravity = num(&mut args, "--gravity"),
            "--no-tilt" => cfg.tilt = false,
            "--summary" => summary = true,
            _ => usage(),
        }
    }
    let (samples, first_contact, result) = run_checked_with_contact(&cfg);
    if summary {
        if result.is_ok() {
            if let (Some(last), true) = (samples.last(), !samples.is_empty()) {
                // From the substep loop, not from the output samples, so a contact
                // that starts and ends between two samples is still reported.
                let contact = match first_contact {
                    Some(t) => format!("{t:.4}"),
                    None => "none".to_string(),
                };
                let peak = samples.iter().map(|s| s.max_beam_strain).fold(f64::MIN, f64::max);
                println!(
                    "first_contact_s={contact},peak_strain={peak:.6},final_center_y={:.4}",
                    last.center_y
                );
            }
        }
    } else if !samples.is_empty() {
        let _ = std::io::stdout().write_all(to_csv(&samples).as_bytes());
    }
    if let Err(e) = result {
        eprintln!("crash-sim: {e}");
        exit(1);
    }
}
