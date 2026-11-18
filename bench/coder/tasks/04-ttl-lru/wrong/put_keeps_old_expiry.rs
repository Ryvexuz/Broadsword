use std::collections::HashMap;
use std::hash::Hash;

const NIL: usize = usize::MAX;

struct Node<K, V> {
    key: K,
    value: Option<V>,
    expires: u64,
    prev: usize, // towards the most recently used end (head)
    next: usize, // towards the least recently used end (tail)
}

pub struct TtlLru<K, V> {
    map: HashMap<K, usize>,
    nodes: Vec<Node<K, V>>,
    free: Vec<usize>,
    head: usize, // most recently used
    tail: usize, // least recently used
    capacity: usize,
    ttl: u64,
}

impl<K: Eq + Hash + Clone, V> TtlLru<K, V> {
    pub fn new(capacity: usize, ttl: u64) -> Self {
        assert!(capacity > 0, "capacity must be at least 1");
        assert!(ttl > 0, "ttl must be at least 1");
        TtlLru {
            map: HashMap::new(),
            nodes: Vec::new(),
            free: Vec::new(),
            head: NIL,
            tail: NIL,
            capacity,
            ttl,
        }
    }

    fn unlink(&mut self, i: usize) {
        let (p, n) = (self.nodes[i].prev, self.nodes[i].next);
        if p != NIL {
            self.nodes[p].next = n;
        } else {
            self.head = n;
        }
        if n != NIL {
            self.nodes[n].prev = p;
        } else {
            self.tail = p;
        }
        self.nodes[i].prev = NIL;
        self.nodes[i].next = NIL;
    }

    fn push_front(&mut self, i: usize) {
        self.nodes[i].prev = NIL;
        self.nodes[i].next = self.head;
        if self.head != NIL {
            let h = self.head;
            self.nodes[h].prev = i;
        } else {
            self.tail = i;
        }
        self.head = i;
    }

    /// Unlinks and frees node `i`, dropping it from the map; returns its value.
    fn delete(&mut self, i: usize) -> Option<V> {
        self.unlink(i);
        let key = self.nodes[i].key.clone();
        self.map.remove(&key);
        self.free.push(i);
        self.nodes[i].value.take()
    }

    fn is_live(&self, i: usize, now: u64) -> bool {
        now < self.nodes[i].expires
    }

    pub fn put(&mut self, key: K, value: V, now: u64) -> Option<V> {
        let expires = now.saturating_add(self.ttl);
        if let Some(&i) = self.map.get(&key) {
            let live = self.is_live(i, now);
            let old = self.nodes[i].value.replace(value);
            // WRONG: replacing a live entry keeps its old expiry time
            let _ = expires;
            self.unlink(i);
            self.push_front(i);
            return if live { old } else { None };
        }
        if self.map.len() == self.capacity {
            let t = self.tail;
            self.delete(t);
        }
        let node = Node { key: key.clone(), value: Some(value), expires, prev: NIL, next: NIL };
        let i = if let Some(i) = self.free.pop() {
            self.nodes[i] = node;
            i
        } else {
            self.nodes.push(node);
            self.nodes.len() - 1
        };
        self.map.insert(key, i);
        self.push_front(i);
        None
    }

    pub fn get(&mut self, key: &K, now: u64) -> Option<&V> {
        let i = *self.map.get(key)?;
        if !self.is_live(i, now) {
            self.delete(i);
            return None;
        }
        self.unlink(i);
        self.push_front(i);
        self.nodes[i].value.as_ref()
    }

    pub fn peek(&self, key: &K, now: u64) -> Option<&V> {
        let i = *self.map.get(key)?;
        if self.is_live(i, now) {
            self.nodes[i].value.as_ref()
        } else {
            None
        }
    }

    pub fn remove(&mut self, key: &K, now: u64) -> Option<V> {
        let i = *self.map.get(key)?;
        let live = self.is_live(i, now);
        let v = self.delete(i);
        if live {
            v
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}
