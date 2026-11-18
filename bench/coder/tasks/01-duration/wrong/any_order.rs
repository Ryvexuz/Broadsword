// Wrong: accepts units in any order and repeated units.
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
    let mut digits = String::new();
    for c in input.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let mult = match c {
            'h' => 3600u64,
            'm' => 60,
            's' => 1,
            _ => return Err(DurationError::InvalidFormat),
        };
        if digits.is_empty() {
            return Err(DurationError::InvalidFormat);
        }
        let v: u64 = digits.parse().map_err(|_| DurationError::Overflow)?;
        digits.clear();
        total = v
            .checked_mul(mult)
            .and_then(|x| total.checked_add(x))
            .ok_or(DurationError::Overflow)?;
    }
    if !digits.is_empty() {
        return Err(DurationError::InvalidFormat);
    }
    Ok(total)
}
