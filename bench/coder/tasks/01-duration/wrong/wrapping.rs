// Wrong: arithmetic wraps around instead of reporting Overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {
    Empty,
    InvalidFormat,
    Overflow,
}

pub fn parse_duration(input: &str) -> Result<u64, DurationError> {
    if input.is_empty() {
        return Err(DurationError::Empty);
    }
    let mut total: u64 = 0;
    let mut value: u64 = 0;
    let mut have_digits = false;
    let mut rank = 0;
    for c in input.chars() {
        if let Some(d) = c.to_digit(10) {
            value = value.wrapping_mul(10).wrapping_add(d as u64);
            have_digits = true;
            continue;
        }
        let (r, mult) = match c {
            'h' => (0, 3600u64),
            'm' => (1, 60),
            's' => (2, 1),
            _ => return Err(DurationError::InvalidFormat),
        };
        if !have_digits || r < rank {
            return Err(DurationError::InvalidFormat);
        }
        rank = r + 1;
        total = total.wrapping_add(value.wrapping_mul(mult));
        value = 0;
        have_digits = false;
    }
    if have_digits {
        return Err(DurationError::InvalidFormat);
    }
    Ok(total)
}
