#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeError {
    InvalidInterval(usize),
}

pub fn merge_intervals(intervals: &[(i64, i64)]) -> Result<Vec<(i64, i64)>, MergeError> {
    for (i, &(s, e)) in intervals.iter().enumerate() {
        if s > e {
            return Err(MergeError::InvalidInterval(i));
        }
    }
    let mut sorted: Vec<(i64, i64)> = intervals.to_vec();
    // WRONG: assumes the input is already sorted
    let _ = &mut sorted;
    let mut out: Vec<(i64, i64)> = Vec::with_capacity(sorted.len());
    for (s, e) in sorted {
        match out.last_mut() {
            // s <= last_end + 1, written without overflow
            Some(last) if s <= last.1 || s - 1 <= last.1 => {
                if e > last.1 {
                    last.1 = e;
                }
            }
            _ => out.push((s, e)),
        }
    }
    Ok(out)
}
