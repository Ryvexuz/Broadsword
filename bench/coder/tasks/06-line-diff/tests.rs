mod common;
use common::within;
use solution::Edit::*;
use solution::*;

/// Applies a script to `old` exactly as the spec describes. Err explains what is wrong.
fn apply(old: &[String], script: &[Edit]) -> Result<Vec<String>, String> {
    let mut cursor = 0usize;
    let mut out = Vec::new();
    for (k, edit) in script.iter().enumerate() {
        match edit {
            Equal(l) | Delete(l) => {
                match old.get(cursor) {
                    None => return Err(format!("edit {k} reads past the end of old")),
                    Some(o) if o != l => {
                        return Err(format!("edit {k}: old line {cursor} is {o:?} but the edit says {l:?}"))
                    }
                    _ => {}
                }
                cursor += 1;
                if let Equal(l) = edit {
                    out.push(l.clone());
                }
            }
            Insert(l) => out.push(l.clone()),
        }
    }
    if cursor != old.len() {
        return Err(format!("script ends at old line {cursor} of {}", old.len()));
    }
    Ok(out)
}

fn lcs_len(a: &[String], b: &[String]) -> usize {
    let mut prev = vec![0u32; b.len() + 1];
    for x in a {
        let mut cur = vec![0u32; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { cur[j].max(prev[j + 1]) };
        }
        prev = cur;
    }
    prev[b.len()] as usize
}

fn own(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Runs diff (with the per-test timeout) and checks validity and minimality.
fn check_owned(old: Vec<String>, new: Vec<String>) -> Vec<Edit> {
    let (o2, n2) = (old.clone(), new.clone());
    let script = within(move || {
        let o: Vec<&str> = o2.iter().map(|s| s.as_str()).collect();
        let n: Vec<&str> = n2.iter().map(|s| s.as_str()).collect();
        diff(&o, &n)
    });
    let got = apply(&old, &script).unwrap_or_else(|e| panic!("invalid script for {old:?} -> {new:?}: {e}"));
    assert_eq!(got, new, "applying the script must give the new lines");
    let changes = script.iter().filter(|e| !matches!(e, Equal(_))).count();
    let want = old.len() + new.len() - 2 * lcs_len(&old, &new);
    assert_eq!(changes, want, "not minimal for {old:?} -> {new:?}: {script:?}");
    script
}

fn check(old: &[&str], new: &[&str]) -> Vec<Edit> {
    check_owned(own(old), own(new))
}

#[test]
fn hidden_both_empty() {
    assert_eq!(check(&[], &[]), vec![]);
}

#[test]
fn hidden_only_inserts() {
    let s = check(&[], &["x"]);
    assert_eq!(s, vec![Insert("x".into())]);
    let s = check(&[], &["a", "b", "c"]);
    assert_eq!(s.len(), 3);
}

#[test]
fn hidden_only_deletes() {
    let s = check(&["x"], &[]);
    assert_eq!(s, vec![Delete("x".into())]);
    check(&["a", "b", "c"], &[]);
}

#[test]
fn hidden_identical_is_all_equal() {
    let s = check(&["a", "b", "c"], &["a", "b", "c"]);
    assert_eq!(s, vec![Equal("a".into()), Equal("b".into()), Equal("c".into())]);
}

#[test]
fn hidden_spec_examples() {
    let s = check(&["a", "b", "c"], &["a", "c"]);
    assert_eq!(s, vec![Equal("a".into()), Delete("b".into()), Equal("c".into())]);
    let s = check(&["a", "b"], &["b", "a"]);
    assert_eq!(s.len(), 3);
}

#[test]
fn hidden_completely_different() {
    let s = check(&["a", "b", "c"], &["x", "y"]);
    assert_eq!(s.len(), 5);
    assert!(s.iter().all(|e| !matches!(e, Equal(_))));
}

#[test]
fn hidden_change_in_the_middle() {
    check(&["1", "2", "3", "4", "5"], &["1", "2", "X", "4", "5"]);
    check(&["1", "2", "3", "4", "5"], &["1", "2", "3", "3.5", "4", "5"]);
    check(&["1", "2", "3", "4", "5"], &["1", "5"]);
}

#[test]
fn hidden_first_and_last_lines_change() {
    check(&["a", "b", "c", "d"], &["x", "b", "c", "y"]);
    check(&["a", "b", "c", "d"], &["b", "c"]);
    check(&["b", "c"], &["a", "b", "c", "d"]);
}

#[test]
fn hidden_duplicates() {
    check(&["a", "a", "a"], &["a"]);
    check(&["a"], &["a", "a", "a"]);
    check(&["a", "b", "a", "b", "a"], &["b", "a", "b"]);
    check(&["a", "a", "b", "b"], &["b", "b", "a", "a"]);
    check(&["x", "x", "x", "x"], &["x", "x"]);
}

#[test]
fn hidden_empty_lines_and_whitespace_count() {
    check(&["", "a", ""], &["a"]);
    check(&["a", "", "b"], &["a", "b"]);
    let s = check(&["a ", "b"], &["a", "b"]);
    assert_eq!(s.iter().filter(|e| matches!(e, Equal(_))).count(), 1, "'a ' and 'a' differ");
    let s = check(&["Hello"], &["hello"]);
    assert_eq!(s.len(), 2, "case matters");
    check(&["  x"], &["x"]);
    check(&["\t"], &[" "]);
}

#[test]
fn hidden_reversed() {
    check(&["1", "2", "3", "4", "5", "6"], &["6", "5", "4", "3", "2", "1"]);
}

#[test]
fn hidden_classic_myers_example() {
    // The example from Myers' paper: ABCABBA -> CBABAC (minimal script has 5 edits).
    let old: Vec<&str> = "A B C A B B A".split(' ').collect();
    let new: Vec<&str> = "C B A B A C".split(' ').collect();
    let s = check(&old, &new);
    assert_eq!(s.iter().filter(|e| !matches!(e, Equal(_))).count(), 5);
}

#[test]
fn hidden_unicode_and_long_lines() {
    let long = "x".repeat(10_000);
    check(&["\u{e9}t\u{e9}", &long, "\u{1f600}"], &[&long, "\u{1f600}", "\u{e9}t\u{e9}"]);
}

#[test]
fn hidden_script_has_no_wasted_pairs() {
    // A minimal script never contains a Delete and an Insert of the same line that could have been an Equal.
    // Here "b" can be kept: the only minimal script has one Equal.
    let s = check(&["a", "b", "c"], &["x", "b", "y"]);
    assert_eq!(s.iter().filter(|e| matches!(e, Equal(_))).count(), 1);
}

#[test]
fn hidden_random_small_pairs() {
    let mut x = 20260101u64;
    let mut rnd = move |n: u64| {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (x >> 33) % n
    };
    let alphabet = ["a", "b", "c", "d"];
    for _ in 0..400 {
        let la = rnd(13) as usize;
        let lb = rnd(13) as usize;
        let old: Vec<String> = (0..la).map(|_| alphabet[rnd(4) as usize].to_string()).collect();
        let new: Vec<String> = (0..lb).map(|_| alphabet[rnd(4) as usize].to_string()).collect();
        check_owned(old, new);
    }
}

#[test]
fn hidden_random_pairs_with_small_alphabet_and_more_lines() {
    let mut x = 777u64;
    let mut rnd = move |n: u64| {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (x >> 33) % n
    };
    for alpha in [2u64, 3, 6] {
        for _ in 0..40 {
            let la = 20 + rnd(30) as usize;
            let lb = 20 + rnd(30) as usize;
            let old: Vec<String> = (0..la).map(|_| format!("l{}", rnd(alpha))).collect();
            let new: Vec<String> = (0..lb).map(|_| format!("l{}", rnd(alpha))).collect();
            check_owned(old, new);
        }
    }
}

#[test]
fn hidden_medium_unrelated_inputs() {
    // About 1500 x 1500 lines with many differences: needs an efficient algorithm, not a naive search.
    let mut x = 99u64;
    let mut rnd = move |n: u64| {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (x >> 33) % n
    };
    let old: Vec<String> = (0..1500).map(|_| format!("w{}", rnd(60))).collect();
    let new: Vec<String> = (0..1500).map(|_| format!("w{}", rnd(60))).collect();
    check_owned(old, new);
}

#[test]
fn hidden_large_similar_inputs() {
    // 5000 unique lines; 5 lines replaced, 3 deleted and 3 inserted, all at different places.
    // Lines are unique, so the minimal script has exactly 5*2 + 3 + 3 = 16 changes.
    let old: Vec<String> = (0..5000).map(|i| format!("line {i}")).collect();
    let mut new = old.clone();
    for &i in &[100usize, 1200, 2500, 3300, 4999] {
        new[i] = format!("changed {i}");
    }
    for &i in &[4000usize, 2000, 50] {
        new.remove(i);
    }
    for (k, &i) in [4500usize, 1500, 20].iter().enumerate() {
        new.insert(i, format!("added {k}"));
    }
    let script = check_owned(old, new);
    assert_eq!(script.iter().filter(|e| !matches!(e, Equal(_))).count(), 16);
}

#[test]
fn hidden_large_identical_and_large_one_sided() {
    // Checked by hand: the reference LCS used by check_owned would be too slow at this size.
    fn run(old: Vec<String>, new: Vec<String>) -> (Vec<Edit>, Vec<String>) {
        let (o2, n2) = (old.clone(), new);
        let script = within(move || {
            let o: Vec<&str> = o2.iter().map(|s| s.as_str()).collect();
            let n: Vec<&str> = n2.iter().map(|s| s.as_str()).collect();
            diff(&o, &n)
        });
        let out = apply(&old, &script).unwrap();
        (script, out)
    }
    let old: Vec<String> = (0..6_000).map(|i| format!("line {i}")).collect();
    let (s, out) = run(old.clone(), old.clone());
    assert_eq!(out, old);
    assert_eq!(s.len(), 6_000);
    assert!(s.iter().all(|e| matches!(e, Equal(_))));
    let (s, out) = run(Vec::new(), old.clone());
    assert_eq!(out, old);
    assert_eq!(s.len(), 6_000);
    assert!(s.iter().all(|e| matches!(e, Insert(_))));
    let (s, out) = run(old, Vec::new());
    assert!(out.is_empty());
    assert_eq!(s.len(), 6_000);
    assert!(s.iter().all(|e| matches!(e, Delete(_))));
}

#[test]
fn hidden_very_long_inputs_with_spread_out_changes() {
    // 50,000 unique lines; the first and the last line are among the changes, so trimming the common
    // prefix and suffix removes nothing. Unique lines, so the minimal script has exactly
    // 5*2 (replaced) + 2 (deleted) + 2 (inserted) = 14 changes.
    let old: Vec<String> = (0..50_000).map(|i| format!("line {i}")).collect();
    let mut new = old.clone();
    for &i in &[0usize, 12_000, 25_000, 37_000, 49_999] {
        new[i] = format!("changed {i}");
    }
    new.remove(30_000);
    new.remove(8_000);
    new.insert(44_000, "added 0".to_string());
    new.insert(18_000, "added 1".to_string());
    let (o2, n2) = (old.clone(), new.clone());
    let script = within(move || {
        let o: Vec<&str> = o2.iter().map(|s| s.as_str()).collect();
        let n: Vec<&str> = n2.iter().map(|s| s.as_str()).collect();
        diff(&o, &n)
    });
    let got = apply(&old, &script).unwrap();
    assert_eq!(got, new, "applying the script must give the new lines");
    assert_eq!(script.iter().filter(|e| !matches!(e, Equal(_))).count(), 14);
}
