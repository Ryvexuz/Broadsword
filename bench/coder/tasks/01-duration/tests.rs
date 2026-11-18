mod common;
use common::within;
use solution::*;

fn p(s: &'static str) -> Result<u64, DurationError> {
    within(move || parse_duration(s))
}

#[test]
fn hidden_single_units() {
    assert_eq!(p("1h"), Ok(3600));
    assert_eq!(p("1m"), Ok(60));
    assert_eq!(p("1s"), Ok(1));
    assert_eq!(p("0s"), Ok(0));
    assert_eq!(p("0h0m0s"), Ok(0));
}

#[test]
fn hidden_combined() {
    assert_eq!(p("1h30m15s"), Ok(3600 + 1800 + 15));
    assert_eq!(p("2h5s"), Ok(7205));
    assert_eq!(p("1h30m"), Ok(5400));
    assert_eq!(p("45m10s"), Ok(2710));
}

#[test]
fn hidden_numbers_may_exceed_unit_range() {
    assert_eq!(p("90m"), Ok(5400));
    assert_eq!(p("3600s"), Ok(3600));
    assert_eq!(p("100h100m100s"), Ok(360_000 + 6000 + 100));
}

#[test]
fn hidden_leading_zeros() {
    assert_eq!(p("007s"), Ok(7));
    assert_eq!(p("01h02m03s"), Ok(3723));
}

#[test]
fn hidden_empty() {
    assert_eq!(p(""), Err(DurationError::Empty));
}

#[test]
fn hidden_wrong_order_or_repeated() {
    for s in ["30m1h", "1s1s", "5m5m", "1h1h", "10s5m", "1s1h", "1m1h1s"] {
        assert_eq!(p(s), Err(DurationError::InvalidFormat), "input {s:?}");
    }
}

#[test]
fn hidden_missing_unit_or_number() {
    for s in ["30", "1h30", "h", "m", "s", "1hm", "1h30mm", "hh", "1h2", "s1"] {
        assert_eq!(p(s), Err(DurationError::InvalidFormat), "input {s:?}");
    }
}

#[test]
fn hidden_bad_characters() {
    for s in [
        "1H", "1M30S", " 1s", "1s ", "1 s", "1h 30m", "+5s", "-5s", "1.5s", "5d", "1h,30m", "1h-30m", "abc", "1\u{e9}s", "\u{661}s",
    ] {
        assert_eq!(p(s), Err(DurationError::InvalidFormat), "input {s:?}");
    }
}

#[test]
fn hidden_exactly_max_is_fine() {
    assert_eq!(p("18446744073709551615s"), Ok(u64::MAX));
    assert_eq!(p("5124095576030431h15s"), Ok(u64::MAX));
    assert_eq!(p("5124095576030430h59m60s"), Ok(18446744073709551600 - 3600 + 3540 + 60));
    assert_eq!(p("307445734561825860m15s"), Ok(u64::MAX));
}

#[test]
fn hidden_overflow() {
    // literal does not fit in u64
    assert_eq!(p("18446744073709551616s"), Err(DurationError::Overflow));
    assert_eq!(p("99999999999999999999999999s"), Err(DurationError::Overflow));
    // multiplication overflows
    assert_eq!(p("5124095576030432h"), Err(DurationError::Overflow));
    assert_eq!(p("307445734561825861m"), Err(DurationError::Overflow));
    // addition overflows
    assert_eq!(p("5124095576030431h16s"), Err(DurationError::Overflow));
    assert_eq!(p("5124095576030431h1m"), Err(DurationError::Overflow));
    assert_eq!(p("307445734561825860m16s"), Err(DurationError::Overflow));
}

#[test]
fn hidden_malformed_beats_overflow() {
    assert_eq!(p("99999999999999999999999x"), Err(DurationError::InvalidFormat));
    assert_eq!(p("99999999999999999999999h5"), Err(DurationError::InvalidFormat));
    assert_eq!(p("99999999999999999999999s5s"), Err(DurationError::InvalidFormat));
}

#[test]
fn hidden_very_long_input_does_not_panic() {
    let r = within(|| {
        let s = "1".repeat(100_000) + "s";
        parse_duration(&s)
    });
    assert_eq!(r, Err(DurationError::Overflow));
    let r = within(|| {
        let s = "0".repeat(100_000) + "5s";
        parse_duration(&s)
    });
    assert_eq!(r, Ok(5));
}
