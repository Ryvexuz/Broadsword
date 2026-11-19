# LRU cache with a time-to-live

Write a generic cache with a fixed capacity, least-recently-used eviction, and a time-to-live (TTL) on every entry.
The cache never reads a clock: the caller passes the current time `now` into each call, so time is just a `u64` number of ticks. Calls are made with non-decreasing `now`.

## Public API

Your file must define exactly these public items (you may add private helpers, private fields and private types):

```rust
use std::hash::Hash;

pub struct TtlLru<K, V> { /* your fields */ }

impl<K: Eq + Hash + Clone, V> TtlLru<K, V> {
    /// A cache holding at most `capacity` entries; each entry lives for `ttl` ticks.
    /// Panics if `capacity == 0` or `ttl == 0`.
    pub fn new(capacity: usize, ttl: u64) -> Self;

    /// Inserts or replaces `key`. See "put" below. Returns the previous live value, if any.
    pub fn put(&mut self, key: K, value: V, now: u64) -> Option<V>;

    /// Looks `key` up and marks it most recently used. See "get" below.
    pub fn get(&mut self, key: &K, now: u64) -> Option<&V>;

    /// Looks `key` up without changing anything. See "peek" below.
    pub fn peek(&self, key: &K, now: u64) -> Option<&V>;

    /// Removes `key`. See "remove" below.
    pub fn remove(&mut self, key: &K, now: u64) -> Option<V>;

    /// The number of entries currently stored. This includes entries that have expired
    /// but have not yet been removed (see "Expiry").
    pub fn len(&self) -> usize;

    pub fn capacity(&self) -> usize;
}
```

## Expiry

An entry written (by `put`) at time `t` expires at time `t + ttl` (use saturating addition so this never overflows).
It is **live** while `now < t + ttl` and **expired** once `now >= t + ttl`.
So with `ttl = 6`, an entry written at time 20 is live at times 20 to 25 and expired from time 26.

Reading an entry does not extend its life: only `put` sets the time.

Expired entries are not removed by the passage of time itself (the cache has no clock). They are removed only when `get`, `put` or `remove` touches that key, or when they are evicted as the least recently used entry. Until then they still count in `len()` and still occupy capacity.

## Operations

- **get(key, now)**
  - Live entry: returns `Some(&value)` and marks the entry as the most recently used.
  - Expired entry: removes it and returns `None`.
  - Absent: returns `None`.
- **peek(key, now)**
  - Returns `Some(&value)` only if the entry is live. It changes nothing: it does not affect recency and does not remove an expired entry.
- **put(key, value, now)**
  - Key present and live: replaces the value, sets its write time to `now` (so it now expires at `now + ttl`), marks it most recently used, and returns `Some(old_value)`.
  - Key present but expired: treated as if it were absent, except that its slot is reused so nothing else is evicted. The new entry is written at `now` and is most recently used. Returns `None`.
  - Key absent: if the cache already holds `capacity` entries, first evict the **least recently used entry, whether or not it has expired**. Then insert the new entry as most recently used. Returns `None`.
- **remove(key, now)**
  - Removes the entry if one is stored, live or not. Returns `Some(value)` if it was live, and `None` if it was expired or absent.

"Recently used" order changes only through a successful `get` (live hit) and through `put`. `peek`, a `get` that misses and a `remove` of another key do not change it.

## Performance

`get`, `peek`, `put` and `remove` must each take O(1) time on average, independent of the number of entries stored. The cache will be used with tens of thousands of entries and hundreds of thousands of operations.

## Constraints

- Use only the Rust standard library. No external crates.
- No `unsafe` code.
- Provide a single complete Rust source file that compiles as a library crate root (`lib.rs`), edition 2021.
