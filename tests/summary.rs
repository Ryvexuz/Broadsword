use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_crash-sim")).args(args).output().unwrap()
}

fn summary(args: &[&str]) -> String {
    let mut a = args.to_vec();
    a.push("--summary");
    let o = run(&a);
    assert!(o.status.success());
    String::from_utf8(o.stdout).unwrap()
}

/// (first contact t or None, peak strain, final center_y) from the CSV of the same options.
fn from_csv(args: &[&str]) -> (Option<String>, String, String) {
    let o = run(args);
    assert!(o.status.success());
    let text = String::from_utf8(o.stdout).unwrap();
    let rows: Vec<Vec<&str>> = text.lines().skip(1).map(|l| l.split(',').collect()).collect();
    let contact = rows.iter().find(|r| r[1].parse::<f64>().unwrap() <= 0.0).map(|r| r[0].to_string());
    let peak = rows.iter().map(|r| r[3].parse::<f64>().unwrap()).fold(f64::MIN, f64::max);
    (contact, format!("{peak:.6}"), format!("{:.4}", rows.last().unwrap()[2].parse::<f64>().unwrap()))
}

fn parse(line: &str) -> (String, String, String) {
    assert!(line.ends_with('\n'));
    assert_eq!(line.matches('\n').count(), 1, "exactly one line: {line:?}");
    let f: Vec<&str> = line.trim_end().split(',').collect();
    assert_eq!(f.len(), 3);
    let v = |i: usize, k: &str| f[i].strip_prefix(k).unwrap_or_else(|| panic!("{} missing {k}", f[i])).to_string();
    (v(0, "first_contact_s="), v(1, "peak_strain="), v(2, "final_center_y="))
}

fn decimals(s: &str) -> usize {
    s.split('.').nth(1).unwrap().len()
}

#[test]
fn default_run_has_contact_and_exact_format() {
    let (c, p, y) = parse(&summary(&[]));
    assert_eq!(decimals(&c), 4);
    assert_eq!(decimals(&p), 6);
    assert_eq!(decimals(&y), 4);
    assert!(c.parse::<f64>().unwrap() > 0.0);
}

#[test]
fn short_run_never_touches_ground() {
    let (c, _, _) = parse(&summary(&["--duration", "0.1"]));
    assert_eq!(c, "none");
}

#[test]
fn values_match_the_csv_run() {
    for args in [vec![], vec!["--duration", "0.1"], vec!["--duration", "2", "--drop-height", "3", "--no-tilt"]] {
        let (c, p, y) = parse(&summary(&args));
        let (cc, cp, cy) = from_csv(&args);
        assert_eq!(c, cc.unwrap_or_else(|| "none".into()), "{args:?}");
        assert_eq!(p, cp, "{args:?}");
        assert_eq!(y, cy, "{args:?}");
    }
}

#[test]
fn invalid_option_prints_nothing() {
    for args in [vec!["--summary", "--dt", "-1"], vec!["--summary", "--substeps", "0"], vec!["--summary", "--bogus"], vec!["--summary", "--dt"]] {
        let o = run(&args);
        assert!(!o.status.success(), "{args:?}");
        assert!(o.stdout.is_empty(), "{args:?}");
    }
}

#[test]
fn default_output_is_still_csv() {
    let o = run(&["--duration", "0.1"]);
    assert!(String::from_utf8(o.stdout).unwrap().starts_with("t,min_y,center_y,max_beam_strain\n"));
}
