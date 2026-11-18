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
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut next_unit_rank = 0u8; // h=0, m=1, s=2; a unit must have rank >= this
    let mut total: u64 = 0;
    let mut overflow = false;
    while i < bytes.len() {
        let start = i;
        let mut value: u64 = 0;
        let mut value_overflow = false;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            let d = (bytes[i] - b'0') as u64;
            match value.checked_mul(10).and_then(|v| v.checked_add(d)) {
                Some(v) => value = v,
                None => value_overflow = true,
            }
            i += 1;
        }
        if i == start || i >= bytes.len() {
            return Err(DurationError::InvalidFormat);
        }
        let (rank, mult) = match bytes[i] {
            b'h' => (0u8, 3600u64),
            b'm' => (1, 60),
            b's' => (2, 1),
            _ => return Err(DurationError::InvalidFormat),
        };
        if rank < next_unit_rank {
            return Err(DurationError::InvalidFormat);
        }
        next_unit_rank = rank + 1;
        i += 1;
        if value_overflow {
            overflow = true;
        } else {
            match value.checked_mul(mult).and_then(|v| total.checked_add(v)) {
                Some(t) => total = t,
                None => overflow = true,
            }
        }
    }
    if overflow {
        Err(DurationError::Overflow)
    } else {
        Ok(total)
    }
}
