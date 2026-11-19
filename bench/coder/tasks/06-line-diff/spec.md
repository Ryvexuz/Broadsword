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

- `["p", "q", "r"]`, `["p", "r"]`: `Equal(p)`, `Delete(q)`, `Equal(r)`.
- `["m", "n"]`, `["n", "m"]`: for example `Delete(m)`, `Equal(n)`, `Insert(m)`. That is 2 edits, and none shorter exists.
- `[]`, `["k"]`: `Insert(k)`.
- `[]`, `[]`: the empty script.

## Constraints

- Use only the Rust standard library. No external crates.
- No `unsafe` code.
- Speed and memory: the inputs may have tens of thousands of lines each, and then they differ in only a few dozen places at most. Such an input must be diffed within a few seconds. So do not allocate a table with one cell for every pair of lines, and do not do work proportional to the product of the two lengths; the time must depend mainly on the length of the inputs plus the number of changes.
- Provide a single complete Rust source file that compiles as a library crate root (`lib.rs`), edition 2021.
