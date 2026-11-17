mod cleanup;
mod config;
mod git;
mod job;
mod pause;
mod recover;
mod record;
mod sys;

use config::Config;
use std::io::Read;
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

fn free_now(cfg: &Config) -> u64 {
    sys::disk(&cfg.root).map(|d| d.free).unwrap_or_else(|e| {
        eprintln!("disk check failed: {e}");
        0
    })
}

fn flag(ok: bool) -> &'static str {
    if ok {
        "ok"
    } else {
        "WARN"
    }
}

fn cmd_status(cfg: &Config) -> Result<(), String> {
    let d = sys::disk(&cfg.root)?;
    let r = sys::ram()?;
    let cpu = sys::cpu_percent(Duration::from_secs(1))?;
    println!(
        "disk: {} free of {} (floor {} GB) [{}]",
        pause::gb(d.free),
        pause::gb(d.total),
        cfg.disk_floor_gb,
        flag(!pause::below_floor(d.free, cfg.floor_bytes()))
    );
    let used = (r.total - r.avail) as f64 * 100.0 / r.total as f64;
    println!(
        "ram:  {} free of {} ({:.0}% used, warn at {}%) [{}]",
        pause::gb(r.avail),
        pause::gb(r.total),
        used,
        cfg.ram_warn_percent,
        flag(used < cfg.ram_warn_percent as f64)
    );
    println!("cpu:  {:.0}% over 1s (warn at {}%) [{}]", cpu, cfg.cpu_warn_percent, flag(cpu < cfg.cpu_warn_percent as f64));
    match pause::read_pause(&cfg.root) {
        Some(body) => println!("paused: YES\n{}", body.trim_end()),
        None => println!("paused: no"),
    }
    Ok(())
}

fn run_cleanup(cfg: &Config) {
    let r = cleanup::cleanup_pass(cfg, SystemTime::now(), &mut || free_now(cfg), true);
    cleanup::print_report(&r);
}

fn cmd_watch(cfg: &Config) -> Result<(), String> {
    println!(
        "watching: disk every {}s, cleanup every {}h, floor {} GB (Ctrl+C to stop)",
        cfg.disk_check_every_seconds, cfg.cleanup_every_hours, cfg.disk_floor_gb
    );
    let every = Duration::from_secs(cfg.cleanup_every_hours * 3600);
    let mut last = Instant::now();
    run_cleanup(cfg);
    loop {
        std::thread::sleep(Duration::from_secs(cfg.disk_check_every_seconds.max(1)));
        let free = free_now(cfg);
        if pause::below_floor(free, cfg.floor_bytes()) {
            println!("[{}] free space {} below floor, running cleanup", pause::now_string(), pause::gb(free));
            run_cleanup(cfg);
            last = Instant::now();
        } else if last.elapsed() >= every {
            println!("[{}] scheduled cleanup", pause::now_string());
            run_cleanup(cfg);
            last = Instant::now();
        }
    }
}

fn cmd_resume(cfg: &Config, force: bool) -> Result<ExitCode, String> {
    if !pause::is_paused(&cfg.root) {
        println!("not paused");
        return Ok(ExitCode::SUCCESS);
    }
    let free = free_now(cfg);
    match pause::resume_decision(free, cfg.floor_bytes(), force) {
        Ok(()) => {
            std::fs::remove_file(pause::pause_path(&cfg.root)).map_err(|e| e.to_string())?;
            println!("resumed (free space {})", pause::gb(free));
            Ok(ExitCode::SUCCESS)
        }
        Err(need) => {
            eprintln!(
                "refusing to resume: free space {} is not above the {} GB floor; free at least {} more (or use --force)",
                pause::gb(free),
                cfg.disk_floor_gb,
                pause::gb(need)
            );
            Ok(ExitCode::from(1))
        }
    }
}

fn cmd_hook(cfg: &Config) -> ExitCode {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    if !pause::is_paused(&cfg.root) || pause::allowed_while_paused(&input) {
        return ExitCode::SUCCESS;
    }
    let reason = pause::read_pause(&cfg.root).unwrap_or_default();
    eprintln!(
        "Broadsword is PAUSED (low disk). Only read-only tools and `broadsword status|resume|cleanup` are allowed.\n{}",
        reason.trim_end()
    );
    ExitCode::from(2)
}

/// Run a job after the automatic recovery step. A job is refused (exit 3) while an interrupted `run`
/// job on the same project still waits for `broadsword recover`, because it may have left files half changed.
fn guarded(cfg: &Config, project: &str, job: impl FnOnce() -> ExitCode) -> ExitCode {
    let _ = recover::recover(cfg, recover::Mode::Auto); // kills orphans, resumes read-only jobs; never rolls back
    if let Some(id) = recover::blocked_by_interrupted_run(cfg, project) {
        eprintln!("broadsword: refusing to start: job {id} (run {project}) was interrupted and may have changed files; run `broadsword recover` first");
        return ExitCode::from(job::EXIT_REFUSED);
    }
    job()
}

fn usage() -> ExitCode {
    eprintln!("usage: broadsword <status | cleanup | watch | resume [--force] | hook pre-tool | projects | build|test|check <project> | run <project> -- <command...> | recover [--list | --abandon <id>] | jobs>");
    ExitCode::from(64)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cfg = match config::load() {
        Ok(c) => c,
        Err(e) => {
            // Fail open on purpose: in hook mode exit 1 is a non-blocking error, so a broken
            // or missing broadsword.toml never locks agents out of every tool call.
            eprintln!("broadsword: {e}");
            return ExitCode::from(1);
        }
    };
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["status"] => cmd_status(&cfg).map(|_| ExitCode::SUCCESS),
        ["cleanup"] => {
            run_cleanup(&cfg);
            Ok(ExitCode::SUCCESS)
        }
        ["watch"] => cmd_watch(&cfg).map(|_| ExitCode::SUCCESS),
        ["resume"] => cmd_resume(&cfg, false),
        ["resume", "--force"] => cmd_resume(&cfg, true),
        ["projects"] => Ok(job::cmd_projects(&cfg)),
        [k @ ("build" | "test" | "check"), name] => Ok(guarded(&cfg, name, || job::cmd_job(&cfg, k, name))),
        ["run", name, "--", command @ ..] if !command.is_empty() => Ok(guarded(&cfg, name, || job::cmd_run(&cfg, name, command))),
        ["recover"] => Ok(recover::recover(&cfg, recover::Mode::Explicit)),
        ["recover", "--abandon", id] => Ok(recover::abandon(&cfg, id)),
        ["recover", "--list"] | ["jobs"] => Ok(recover::cmd_list(&cfg)),
        ["hook", "pre-tool"] => Ok(cmd_hook(&cfg)),
        _ => return usage(),
    };
    result.unwrap_or_else(|e| {
        eprintln!("broadsword: {e}");
        ExitCode::from(1)
    })
}
