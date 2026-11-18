# Fixed-capacity ring buffer

Write a generic ring buffer: a queue with a fixed capacity that, when full, overwrites (evicts) its oldest element to make room for a new one.

## Public API

Your file must define exactly these public items (you may add private helpers and private fields):

```rust
pub struct RingBuffer<T> { /* your fields */ }

impl<T> RingBuffer<T> {
    /// Creates an empty buffer that holds at most `capacity` elements.
    /// Panics if `capacity == 0`.
    pub fn new(capacity: usize) -> RingBuffer<T>;

    /// The maximum number of elements, as given to `new`.
    pub fn capacity(&self) -> usize;

    /// The number of elements currently stored (0..=capacity).
    pub fn len(&self) -> usize;

    pub fn is_empty(&self) -> bool;

    /// True when `len() == capacity()`.
    pub fn is_full(&self) -> bool;

    /// Adds `item` as the newest element.
    /// If the buffer was full, the oldest element is removed first and returned as `Some(old)`.
    /// Otherwise returns `None`.
    pub fn push(&mut self, item: T) -> Option<T>;

    /// Removes and returns the oldest element, or `None` if empty.
    pub fn pop_oldest(&mut self) -> Option<T>;

    /// The element at `index`, counting from the oldest (`0` is the oldest,
    /// `len() - 1` the newest). `None` if `index >= len()`.
    pub fn get(&self, index: usize) -> Option<&T>;

    /// The oldest element, or `None`.
    pub fn oldest(&self) -> Option<&T>;

    /// The newest element, or `None`.
    pub fn newest(&self) -> Option<&T>;

    /// Iterates over the stored elements from oldest to newest.
    /// The iterator must know its exact length and be iterable from the back as well
    /// (so `.rev()` yields newest to oldest, and `.len()` works).
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator;

    /// Removes every element (dropping them). The capacity is unchanged.
    pub fn clear(&mut self);
}
```

## Behaviour

- Order is always oldest to newest in insertion order, no matter how many times the buffer has wrapped around internally.
- `pop_oldest` frees a slot: after it, a `push` does not evict anything (unless the buffer is full again).
- Every element ever pushed is either still stored, was returned by `push` (evicted), was returned by `pop_oldest`, or was dropped by `clear`. Elements are dropped exactly once and never duplicated (the element type is not necessarily `Clone` or `Copy`).
- `RingBuffer<T>` must work for any `T`, including types that do not implement `Default`, `Clone` or `Copy`.

## Constraints

- Use only the Rust standard library. No external crates.
- No `unsafe` code.
- Provide a single complete Rust source file that compiles as a library crate root (`lib.rs`), edition 2021.
