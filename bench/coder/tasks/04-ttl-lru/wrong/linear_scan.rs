// Wrong: behaves correctly but every operation scans a Vec, so it is O(n) not O(1).
use std::hash::Hash;

pub struct TtlLru<K, V> {
    items: Vec<(K, V, u64)>, // front = least recently used
    capacity: usize,
    ttl: u64,
}

impl<K: Eq + Hash + Clone, V> TtlLru<K, V> {
    pub fn new(capacity: usize, ttl: u64) -> Self {
        assert!(capacity > 0 && ttl > 0);
        TtlLru { items: Vec::new(), capacity, ttl }
    }

    pub fn put(&mut self, key: K, value: V, now: u64) -> Option<V> {
        let expires = now.saturating_add(self.ttl);
        if let Some(p) = self.items.iter().position(|e| e.0 == key) {
            let old = self.items.remove(p);
            self.items.push((key, value, expires));
            return if now < old.2 { Some(old.1) } else { None };
        }
        if self.items.len() == self.capacity {
            self.items.remove(0);
        }
        self.items.push((key, value, expires));
        None
    }

    pub fn get(&mut self, key: &K, now: u64) -> Option<&V> {
        let p = self.items.iter().position(|e| &e.0 == key)?;
        if now >= self.items[p].2 {
            self.items.remove(p);
            return None;
        }
        let e = self.items.remove(p);
        self.items.push(e);
        self.items.last().map(|e| &e.1)
    }

    pub fn peek(&self, key: &K, now: u64) -> Option<&V> {
        let e = self.items.iter().find(|e| &e.0 == key)?;
        if now < e.2 {
            Some(&e.1)
        } else {
            None
        }
    }

    pub fn remove(&mut self, key: &K, now: u64) -> Option<V> {
        let p = self.items.iter().position(|e| &e.0 == key)?;
        let e = self.items.remove(p);
        if now < e.2 {
            Some(e.1)
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
