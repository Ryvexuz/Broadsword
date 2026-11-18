# Minimal line diff

Write a function that compares two lists of lines and returns an edit script that turns the first list into the second, using as few edits as possible (Myers' algorithm, or any other method that gives a minimal result).

## Public API

Your file must define exactly these public items (you may add private helpers and private types):

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// The line is present in both lists; it is kept.
    Equal(String),
    /// The line is present only in `old`; it is removed.
    Delete(String),
    /// The line is present only in `new`; it is added.
    Insert(String),
}

pub fn diff(old: &[&str], new: &[&str]) -> Vec<Edit>;
```

## What the edit script means

Read the script from the first edit to the last, keeping a cursor into `old` that starts at the first line:

- `Equal(l)`: the line under the cursor in `old` is `l`; write `l` to the output and move the cursor forward by one.
- `Delete(l)`: the line under the cursor in `old` is `l`; write nothing and move the cursor forward by one.
- `Insert(l)`: write `l` to the output; the cursor does not move.

When the script ends, the cursor must be just past the last line of `old`, and the output must be exactly `new`.
So every line of `old` is consumed by exactly one `Equal` or `Delete`, and every line of `new` is produced by exactly one `Equal` or `Insert`, each in order.

## Minimal

The script must contain the fewest possible `Delete` and `Insert` edits together. Equivalently, the `Equal` lines must form a longest common subsequence of `old` and `new`.
There is often more than one minimal script; any minimal valid script is accepted. Lines are compared by exact string equality (case, spaces and empty lines all count).

Examples (old, new, then one minimal script):

- `["a", "b", "c"]`, `["a", "c"]`: `Equal(a)`, `Delete(b)`, `Equal(c)`.
- `["a", "b"]`, `["b", "a"]`: for example `Delete(a)`, `Equal(b)`, `Insert(a)`. That is 2 edits, and none shorter exists.
- `[]`, `["x"]`: `Insert(x)`.
- `[]`, `[]`: the empty script.

## Constraints

- Use only the Rust standard library. No external crates.
- No `unsafe` code.
- Inputs with a few thousand lines each, differing in only a few places, must be handled quickly, and memory use must stay reasonable (do not allocate a table with one cell for every pair of lines if the inputs are long).
- Provide a single complete Rust source file that compiles as a library crate root (`lib.rs`), edition 2021.
