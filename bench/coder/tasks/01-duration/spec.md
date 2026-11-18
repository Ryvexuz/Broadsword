# Parse a duration

Write a function that turns a duration string such as `2h10m5s` into a number of seconds.

## Public API

Your file must define exactly these public items (you may add private helpers):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurationError {
    /// The input is the empty string.
    Empty,
    /// The input is not a well-formed duration (see the grammar).
    InvalidFormat,
    /// The input is well-formed but the total does not fit in a `u64`.
    Overflow,
}

pub fn parse_duration(input: &str) -> Result<u64, DurationError>;
```

## Grammar

A duration is one or more components written one after another with nothing between them.
A component is one or more ASCII digits followed by one unit letter:

| unit | meaning | seconds |
|------|---------|---------|
| `h`  | hours   | 3600    |
| `m`  | minutes | 60      |
| `s`  | seconds | 1       |

Rules:

- Unit letters are lowercase only.
- Each unit may appear at most once, and the units must appear in the order `h`, `m`, `s`.
  So `4h20m`, `15s` and `3h9s` are valid; `20m4h`, `7s7s` and `9m9m` are not.
- A component's number may be any size in its own right: `75m` is valid and means 4500 seconds. Leading zeros are allowed (`0042s` is 42).
- Zero is valid: `0m` means 0.
- No whitespace, no sign characters (`+`, `-`), no decimal points, no other characters.
- A number without a unit (`45`, `2h10`) is not valid. A unit without a number (`m`, `3hs`) is not valid.

The result is the sum of all components, in seconds.

## Errors

- The empty string gives `DurationError::Empty`.
- Any other input that does not follow the grammar gives `DurationError::InvalidFormat`.
  This holds even if some number in the input is also too large: malformed input is always `InvalidFormat`.
- Input that follows the grammar but whose total (or any single component's value in seconds) does not fit in a `u64` gives `DurationError::Overflow`.
  A total of exactly `u64::MAX` is fine.

## Constraints

- Use only the Rust standard library. No external crates.
- No `unsafe` code.
- The function must never panic, whatever the input.
- Provide a single complete Rust source file that compiles as a library crate root (`lib.rs`), edition 2021.
