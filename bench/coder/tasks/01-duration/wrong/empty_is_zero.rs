// Wrong: the empty string parses as 0 instead of Empty, and a trailing number without a unit is ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {
    Empty,
    InvalidFormat,
    Overflow,
}

pub fn parse_duration(input: &str) -> Result<u64, DurationError> {
    let mut total: u64 = 0;
    let mut digits = String::new();
    let mut rank = 0;
    for c in input.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let (r, mult) = match c {
            'h' => (0, 3600u64),
            'm' => (1, 60),
            's' => (2, 1),
            _ => return Err(DurationError::InvalidFormat),
        };
        if digits.is_empty() || r < rank {
            return Err(DurationError::InvalidFormat);
        }
        rank = r + 1;
        let v: u64 = digits.parse().map_err(|_| DurationError::Overflow)?;
        digits.clear();
        total = v
            .checked_mul(mult)
            .and_then(|x| total.checked_add(x))
            .ok_or(DurationError::Overflow)?;
    }
    Ok(total)
}
