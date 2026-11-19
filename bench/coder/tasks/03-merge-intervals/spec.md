# Merge intervals

Write a function that merges overlapping and adjacent closed integer intervals.

## Public API

Your file must define exactly these public items (you may add private helpers):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeError {
    /// The interval at this index (in the input slice) has `start > end`.
    InvalidInterval(usize),
}

/// Each interval `(start, end)` is closed: it contains every integer `x` with `start <= x <= end`.
pub fn merge_intervals(intervals: &[(i64, i64)]) -> Result<Vec<(i64, i64)>, MergeError>;
```

## Behaviour

- The input may be in any order and may contain duplicates. It is not modified.
- Two intervals are merged when they share at least one integer **or are adjacent**, meaning no integer lies between them. For example:
  - `(100, 300)` and `(200, 500)` overlap, so they merge into `(100, 500)`.
  - `(100, 300)` and `(300, 500)` share the integer 300, so they merge into `(100, 500)`.
  - `(100, 300)` and `(301, 600)` are adjacent (300 and 301 are consecutive integers), so they merge into `(100, 600)`.
  - `(100, 300)` and `(305, 600)` are not adjacent (the integers 301 to 304 lie between them), so they stay separate.
- An interval fully inside another disappears into it: `(100, 1000)` and `(200, 300)` give `(100, 1000)`.
- A single-point interval such as `(77, 77)` is valid.
- The result is sorted by start, ascending. No two intervals in the result overlap or are adjacent, so the result is the unique smallest list covering exactly the same integers as the input.
- An empty input gives `Ok(vec![])`.

## Errors

- If any interval has `start > end`, return `Err(MergeError::InvalidInterval(i))` where `i` is the index of the **first** such interval in the input slice. This is checked before any merging, so it applies whatever the other intervals are.

## Edge cases to handle

- Values may be anywhere in the `i64` range, including `i64::MIN` and `i64::MAX`. The function must never panic or overflow: `(i64::MIN, -7)` and `(-6, i64::MAX)` are adjacent and merge into one interval that runs from `i64::MIN` to `i64::MAX`, while `(7, i64::MAX)` and `(i64::MAX - 3, i64::MAX)` overlap and merge into one interval from 7 to `i64::MAX`.

## Constraints

- Use only the Rust standard library. No external crates.
- No `unsafe` code.
- Provide a single complete Rust source file that compiles as a library crate root (`lib.rs`), edition 2021.
