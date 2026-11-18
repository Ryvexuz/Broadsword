mod common;
use common::within;
use solution::*;

fn m(v: &[(i64, i64)]) -> Result<Vec<(i64, i64)>, MergeError> {
    let v = v.to_vec();
    within(move || merge_intervals(&v))
}

#[test]
fn hidden_empty_and_single() {
    assert_eq!(m(&[]), Ok(vec![]));
    assert_eq!(m(&[(4, 9)]), Ok(vec![(4, 9)]));
    assert_eq!(m(&[(7, 7)]), Ok(vec![(7, 7)]));
}

#[test]
fn hidden_overlapping() {
    assert_eq!(m(&[(1, 3), (2, 5)]), Ok(vec![(1, 5)]));
    assert_eq!(m(&[(1, 5), (2, 3)]), Ok(vec![(1, 5)]));
    assert_eq!(m(&[(1, 4), (2, 6), (5, 9)]), Ok(vec![(1, 9)]));
}

#[test]
fn hidden_touching_at_one_point_merges() {
    assert_eq!(m(&[(1, 3), (3, 5)]), Ok(vec![(1, 5)]));
    assert_eq!(m(&[(3, 5), (1, 3)]), Ok(vec![(1, 5)]));
}

#[test]
fn hidden_adjacent_integers_merge() {
    assert_eq!(m(&[(1, 3), (4, 6)]), Ok(vec![(1, 6)]));
    assert_eq!(m(&[(4, 6), (1, 3)]), Ok(vec![(1, 6)]));
    assert_eq!(m(&[(0, 0), (1, 1), (2, 2)]), Ok(vec![(0, 2)]));
    assert_eq!(m(&[(-3, -2), (-1, -1), (0, 4)]), Ok(vec![(-3, 4)]));
}

#[test]
fn hidden_gap_keeps_separate() {
    assert_eq!(m(&[(1, 3), (5, 6)]), Ok(vec![(1, 3), (5, 6)]));
    assert_eq!(m(&[(1, 1), (3, 3)]), Ok(vec![(1, 1), (3, 3)]));
    assert_eq!(m(&[(-10, -5), (5, 10)]), Ok(vec![(-10, -5), (5, 10)]));
}

#[test]
fn hidden_nested_intervals_keep_the_larger_end() {
    assert_eq!(m(&[(1, 10), (2, 3)]), Ok(vec![(1, 10)]));
    assert_eq!(m(&[(2, 3), (1, 10)]), Ok(vec![(1, 10)]));
    assert_eq!(m(&[(1, 10), (2, 3), (4, 5), (12, 14), (13, 13)]), Ok(vec![(1, 10), (12, 14)]));
    assert_eq!(m(&[(1, 100), (5, 6), (7, 8), (50, 60)]), Ok(vec![(1, 100)]));
}

#[test]
fn hidden_unsorted_input_gives_sorted_output() {
    assert_eq!(
        m(&[(20, 25), (1, 2), (10, 12), (4, 5), (11, 15)]),
        Ok(vec![(1, 2), (4, 5), (10, 15), (20, 25)])
    );
    assert_eq!(m(&[(9, 9), (1, 1), (5, 5)]), Ok(vec![(1, 1), (5, 5), (9, 9)]));
}

#[test]
fn hidden_duplicates() {
    assert_eq!(m(&[(1, 2), (1, 2), (1, 2)]), Ok(vec![(1, 2)]));
    assert_eq!(m(&[(5, 5), (5, 5), (8, 8)]), Ok(vec![(5, 5), (8, 8)]));
}

#[test]
fn hidden_same_start_different_end() {
    assert_eq!(m(&[(1, 2), (1, 9), (1, 4)]), Ok(vec![(1, 9)]));
}

#[test]
fn hidden_invalid_interval_reports_first_index() {
    assert_eq!(m(&[(5, 1)]), Err(MergeError::InvalidInterval(0)));
    assert_eq!(m(&[(1, 2), (3, 2)]), Err(MergeError::InvalidInterval(1)));
    assert_eq!(m(&[(1, 2), (9, 3), (4, 0)]), Err(MergeError::InvalidInterval(1)));
    assert_eq!(m(&[(2, 1), (1, 2)]), Err(MergeError::InvalidInterval(0)));
    assert_eq!(m(&[(i64::MAX, i64::MIN)]), Err(MergeError::InvalidInterval(0)));
}

#[test]
fn hidden_extreme_values() {
    assert_eq!(m(&[(i64::MIN, 0), (1, i64::MAX)]), Ok(vec![(i64::MIN, i64::MAX)]));
    assert_eq!(m(&[(1, i64::MAX), (i64::MIN, 0)]), Ok(vec![(i64::MIN, i64::MAX)]));
    assert_eq!(m(&[(0, i64::MAX), (i64::MAX, i64::MAX)]), Ok(vec![(0, i64::MAX)]));
    assert_eq!(m(&[(i64::MAX, i64::MAX)]), Ok(vec![(i64::MAX, i64::MAX)]));
    assert_eq!(m(&[(i64::MIN, i64::MIN)]), Ok(vec![(i64::MIN, i64::MIN)]));
    assert_eq!(m(&[(i64::MAX, i64::MAX), (i64::MIN, i64::MIN)]), Ok(vec![(i64::MIN, i64::MIN), (i64::MAX, i64::MAX)]));
    assert_eq!(m(&[(i64::MIN, i64::MIN), (i64::MIN + 1, i64::MIN + 1)]), Ok(vec![(i64::MIN, i64::MIN + 1)]));
    assert_eq!(m(&[(i64::MIN, -5), (i64::MAX - 1, i64::MAX)]), Ok(vec![(i64::MIN, -5), (i64::MAX - 1, i64::MAX)]));
}

#[test]
fn hidden_input_is_not_modified() {
    within(|| {
        let input = vec![(5, 6), (1, 2), (2, 3)];
        let copy = input.clone();
        let _ = merge_intervals(&input);
        assert_eq!(input, copy);
    });
}

// Compare with a brute-force set of integers on random small inputs.
#[test]
fn hidden_matches_brute_force() {
    within(|| {
        let mut x = 987654321u64;
        let mut next = move |n: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) % n
        };
        for case in 0..500 {
            let count = next(8) as usize;
            let offset = next(2000) as i64 - 1000; // shift the whole case around zero
            let mut input = Vec::new();
            for _ in 0..count {
                let a = next(30) as i64;
                let len = next(6) as i64;
                input.push((a + offset, a + len + offset));
            }
            let got = merge_intervals(&input).unwrap();
            // expected: integers covered, then runs of consecutive integers
            let mut covered = std::collections::BTreeSet::new();
            for &(s, e) in &input {
                for v in s..=e {
                    covered.insert(v);
                }
            }
            let mut expected: Vec<(i64, i64)> = Vec::new();
            for v in covered {
                match expected.last_mut() {
                    Some(last) if last.1 + 1 == v => last.1 = v,
                    _ => expected.push((v, v)),
                }
            }
            assert_eq!(got, expected, "case {case}: input {input:?}");
        }
    });
}

#[test]
fn hidden_many_intervals() {
    within(|| {
        // 200_000 disjoint-with-gap intervals in reverse order, then one that bridges everything.
        let mut v: Vec<(i64, i64)> = (0..200_000i64).rev().map(|i| (i * 3, i * 3 + 1)).collect();
        let r = merge_intervals(&v).unwrap();
        assert_eq!(r.len(), 200_000);
        assert_eq!(r[0], (0, 1));
        assert_eq!(r[199_999], (599_997, 599_998));
        v.push((0, 600_000));
        assert_eq!(merge_intervals(&v).unwrap(), vec![(0, 600_000)]);
    });
}
