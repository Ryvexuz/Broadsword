mod common;
use common::{panics_within, within};
use solution::*;

#[test]
fn hidden_basic_put_get() {
    within(|| {
        let mut c = TtlLru::new(3, 100);
        assert_eq!(c.capacity(), 3);
        assert_eq!(c.len(), 0);
        assert_eq!(c.put("a", 1, 0), None);
        assert_eq!(c.put("b", 2, 0), None);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&"a", 1), Some(&1));
        assert_eq!(c.get(&"b", 1), Some(&2));
        assert_eq!(c.get(&"zzz", 1), None);
    });
}

#[test]
fn hidden_zero_capacity_panics() {
    assert!(panics_within(|| {
        let _c: TtlLru<u8, u8> = TtlLru::new(0, 5);
    }), "TtlLru::new(0, 5) must panic");
}

#[test]
fn hidden_zero_ttl_panics() {
    assert!(panics_within(|| {
        let _c: TtlLru<u8, u8> = TtlLru::new(5, 0);
    }), "TtlLru::new(5, 0) must panic");
}

#[test]
fn hidden_ttl_boundary() {
    within(|| {
        let mut c = TtlLru::new(4, 10);
        c.put("k", 7, 5);
        assert_eq!(c.peek(&"k", 5), Some(&7));
        assert_eq!(c.peek(&"k", 14), Some(&7), "still live at t + ttl - 1");
        assert_eq!(c.peek(&"k", 15), None, "expired at t + ttl");
        assert_eq!(c.get(&"k", 14), Some(&7));
        assert_eq!(c.get(&"k", 15), None);
    });
}

#[test]
fn hidden_get_does_not_extend_ttl() {
    within(|| {
        let mut c = TtlLru::new(4, 10);
        c.put("k", 1, 0);
        assert_eq!(c.get(&"k", 9), Some(&1));
        assert_eq!(c.get(&"k", 10), None);
    });
}

#[test]
fn hidden_expired_get_removes_entry() {
    within(|| {
        let mut c = TtlLru::new(4, 10);
        c.put("a", 1, 0);
        c.put("b", 2, 0);
        assert_eq!(c.len(), 2);
        assert_eq!(c.get(&"a", 10), None);
        assert_eq!(c.len(), 1);
    });
}

#[test]
fn hidden_peek_changes_nothing() {
    within(|| {
        let mut c = TtlLru::new(2, 10);
        c.put("a", 1, 0);
        c.put("b", 2, 1);
        // peek at a must not make it recently used
        assert_eq!(c.peek(&"a", 2), Some(&1));
        c.put("c", 3, 3); // evicts the LRU, which is still a
        assert_eq!(c.peek(&"a", 3), None);
        assert_eq!(c.peek(&"b", 3), Some(&2));
        // peek of an expired entry does not remove it
        assert_eq!(c.len(), 2);
        assert_eq!(c.peek(&"b", 11), None);
        assert_eq!(c.len(), 2);
    });
}

#[test]
fn hidden_evicts_least_recently_used() {
    within(|| {
        let mut c = TtlLru::new(3, 1000);
        c.put(1, "alpha", 0);
        c.put(2, "beta", 0);
        c.put(3, "gamma", 0);
        assert_eq!(c.put(4, "delta", 0), None);
        assert_eq!(c.len(), 3);
        assert_eq!(c.peek(&1, 0), None, "1 was least recently used");
        assert_eq!(c.peek(&2, 0), Some(&"beta"));
        assert_eq!(c.peek(&3, 0), Some(&"gamma"));
        assert_eq!(c.peek(&4, 0), Some(&"delta"));
    });
}

#[test]
fn hidden_get_refreshes_recency() {
    within(|| {
        let mut c = TtlLru::new(3, 1000);
        c.put(1, 'a', 0);
        c.put(2, 'b', 0);
        c.put(3, 'c', 0);
        assert_eq!(c.get(&1, 1), Some(&'a')); // now 2 is the LRU
        c.put(4, 'd', 1);
        assert_eq!(c.peek(&2, 1), None);
        assert_eq!(c.peek(&1, 1), Some(&'a'));
        assert_eq!(c.get(&3, 2), Some(&'c')); // order, LRU first: 1, 4, 3
        c.put(5, 'e', 2); // evicts 1
        assert_eq!(c.peek(&1, 2), None);
        assert_eq!(c.peek(&4, 2), Some(&'d'));
    });
}

#[test]
fn hidden_failed_get_does_not_change_order() {
    within(|| {
        let mut c = TtlLru::new(2, 1000);
        c.put(1, 'a', 0);
        c.put(2, 'b', 0);
        assert_eq!(c.get(&99, 1), None);
        c.put(3, 'c', 1); // evicts 1
        assert_eq!(c.peek(&1, 1), None);
        assert_eq!(c.peek(&2, 1), Some(&'b'));
    });
}

#[test]
fn hidden_put_existing_returns_old_and_refreshes_everything() {
    within(|| {
        let mut c = TtlLru::new(2, 10);
        c.put("a", 1, 0);
        c.put("b", 2, 1);
        assert_eq!(c.put("a", 10, 5), Some(1)); // a is now MRU, expires at 15
        assert_eq!(c.len(), 2);
        c.put("c", 3, 6); // evicts b (LRU)
        assert_eq!(c.peek(&"b", 6), None);
        assert_eq!(c.peek(&"a", 6), Some(&10));
        assert_eq!(c.peek(&"a", 14), Some(&10), "ttl restarted at the second put");
        assert_eq!(c.peek(&"a", 15), None);
    });
}

#[test]
fn hidden_put_over_expired_key_returns_none_and_evicts_nothing() {
    within(|| {
        let mut c = TtlLru::new(2, 10);
        c.put("a", 1, 0);
        c.put("b", 2, 1);
        assert_eq!(c.put("a", 100, 10), None, "old value was expired");
        assert_eq!(c.len(), 2, "slot reused, nothing evicted");
        assert_eq!(c.peek(&"b", 10), Some(&2));
        assert_eq!(c.get(&"a", 10), Some(&100));
        assert_eq!(c.peek(&"a", 19), Some(&100));
    });
}

#[test]
fn hidden_expired_entries_still_count_until_touched() {
    within(|| {
        let mut c = TtlLru::new(2, 10);
        c.put("a", 1, 0);
        c.put("b", 2, 8); // order, LRU first: a, b
        assert_eq!(c.get(&"a", 9), Some(&1)); // order: b, a
        // At t = 11: a is expired (expires 10), b is live (expires 18).
        // The cache is full, so inserting c evicts the LRU entry, b, even though it is live.
        c.put("c", 3, 11);
        assert_eq!(c.len(), 2);
        assert_eq!(c.peek(&"b", 11), None);
        assert_eq!(c.peek(&"c", 11), Some(&3));
        // a is still stored (expired) and counts in len()
        assert_eq!(c.get(&"a", 11), None);
        assert_eq!(c.len(), 1);
    });
}

#[test]
fn hidden_remove_semantics() {
    within(|| {
        let mut c = TtlLru::new(4, 10);
        c.put("a", 1, 0);
        c.put("b", 2, 0);
        assert_eq!(c.remove(&"a", 5), Some(1));
        assert_eq!(c.len(), 1);
        assert_eq!(c.remove(&"a", 5), None);
        assert_eq!(c.remove(&"nope", 5), None);
        // expired: removed but not returned
        assert_eq!(c.remove(&"b", 10), None);
        assert_eq!(c.len(), 0);
        // the freed capacity is usable
        for i in 0..4 {
            c.put(["w", "x", "y", "z"][i], i, 20);
        }
        assert_eq!(c.len(), 4);
    });
}

#[test]
fn hidden_capacity_one() {
    within(|| {
        let mut c = TtlLru::new(1, 5);
        c.put(1, 'a', 0);
        c.put(2, 'b', 0);
        assert_eq!(c.peek(&1, 0), None);
        assert_eq!(c.get(&2, 4), Some(&'b'));
        assert_eq!(c.get(&2, 5), None);
        assert_eq!(c.len(), 0);
        c.put(3, 'c', 6);
        assert_eq!(c.get(&3, 7), Some(&'c'));
    });
}

#[test]
fn hidden_works_with_owned_keys_and_values() {
    within(|| {
        let mut c: TtlLru<String, Vec<u8>> = TtlLru::new(2, 10);
        c.put("hello".to_string(), vec![1, 2, 3], 0);
        assert_eq!(c.get(&"hello".to_string(), 1), Some(&vec![1, 2, 3]));
        assert_eq!(c.put("hello".to_string(), vec![9], 2), Some(vec![1, 2, 3]));
    });
}

#[test]
fn hidden_huge_times_do_not_overflow() {
    within(|| {
        let mut c = TtlLru::new(2, u64::MAX);
        c.put("a", 1, 100);
        assert_eq!(c.get(&"a", u64::MAX - 1), Some(&1));
        let mut c = TtlLru::new(2, 10);
        c.put("a", 1, u64::MAX - 3);
        assert_eq!(c.peek(&"a", u64::MAX - 3), Some(&1));
    });
}

// A deliberately plain model of the spec, used to cross-check random operation sequences.
struct Model {
    cap: usize,
    ttl: u64,
    // front = least recently used
    items: Vec<(u32, u32, u64)>, // key, value, expires
}

impl Model {
    fn pos(&self, k: u32) -> Option<usize> {
        self.items.iter().position(|e| e.0 == k)
    }
    fn live(e: &(u32, u32, u64), now: u64) -> bool {
        now < e.2
    }
    fn put(&mut self, k: u32, v: u32, now: u64) -> Option<u32> {
        let expires = now.saturating_add(self.ttl);
        if let Some(p) = self.pos(k) {
            let old = self.items.remove(p);
            self.items.push((k, v, expires));
            return if Self::live(&old, now) { Some(old.1) } else { None };
        }
        if self.items.len() == self.cap {
            self.items.remove(0);
        }
        self.items.push((k, v, expires));
        None
    }
    fn get(&mut self, k: u32, now: u64) -> Option<u32> {
        let p = self.pos(k)?;
        if !Self::live(&self.items[p], now) {
            self.items.remove(p);
            return None;
        }
        let e = self.items.remove(p);
        self.items.push(e);
        Some(e.1)
    }
    fn peek(&self, k: u32, now: u64) -> Option<u32> {
        let p = self.pos(k)?;
        if Self::live(&self.items[p], now) {
            Some(self.items[p].1)
        } else {
            None
        }
    }
    fn remove(&mut self, k: u32, now: u64) -> Option<u32> {
        let p = self.pos(k)?;
        let e = self.items.remove(p);
        if Self::live(&e, now) {
            Some(e.1)
        } else {
            None
        }
    }
}

#[test]
fn hidden_matches_model_on_random_operations() {
    within(|| {
        let mut x = 42u64;
        let mut rnd = move |n: u64| {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (x >> 33) % n
        };
        for round in 0..30 {
            let cap = 1 + rnd(6) as usize;
            let ttl = 1 + rnd(12);
            let mut c: TtlLru<u32, u32> = TtlLru::new(cap, ttl);
            let mut m = Model { cap, ttl, items: Vec::new() };
            let mut now = 0u64;
            for step in 0..400 {
                now += rnd(4);
                let k = rnd(9) as u32;
                let v = step as u32;
                match rnd(5) {
                    0 | 1 => assert_eq!(c.put(k, v, now), m.put(k, v, now), "round {round} step {step} put"),
                    2 => assert_eq!(c.get(&k, now).copied(), m.get(k, now), "round {round} step {step} get"),
                    3 => assert_eq!(c.peek(&k, now).copied(), m.peek(k, now), "round {round} step {step} peek"),
                    _ => assert_eq!(c.remove(&k, now), m.remove(k, now), "round {round} step {step} remove"),
                }
                assert_eq!(c.len(), m.items.len(), "round {round} step {step} len");
            }
        }
    });
}

// O(1) operations: with a scan or a per-operation O(n) shuffle this takes minutes, not seconds.
#[test]
fn hidden_operations_are_constant_time() {
    within(|| {
        let cap = 20_000usize;
        let mut c: TtlLru<u32, u32> = TtlLru::new(cap, 1_000_000_000);
        for i in 0..cap as u32 {
            c.put(i, i, 0);
        }
        let mut x = 7u64;
        let mut hits = 0u64;
        for step in 0..300_000u64 {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let k = ((x >> 33) % 40_000) as u32;
            match step % 3 {
                0 => {
                    c.put(k, step as u32, step);
                }
                1 => {
                    if c.get(&k, step).is_some() {
                        hits += 1;
                    }
                }
                _ => {
                    c.remove(&k, step);
                }
            }
        }
        assert!(c.len() <= cap);
        assert!(hits > 0);
    });
}
