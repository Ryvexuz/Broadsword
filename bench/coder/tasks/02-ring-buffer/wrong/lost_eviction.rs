pub struct RingBuffer<T> {
    slots: Vec<Option<T>>,
    head: usize, // index of the oldest element
    len: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> RingBuffer<T> {
        assert!(capacity > 0, "capacity must be at least 1");
        let mut slots = Vec::with_capacity(capacity);
        for _ in 0..capacity {
            slots.push(None);
        }
        RingBuffer { slots, head: 0, len: 0 }
    }

    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn is_full(&self) -> bool {
        self.len == self.slots.len()
    }

    pub fn push(&mut self, item: T) -> Option<T> {
        let cap = self.slots.len();
        if self.len == cap {
            // WRONG: the evicted element is dropped instead of being returned
            self.slots[self.head] = Some(item);
            self.head = (self.head + 1) % cap;
            None
        } else {
            let idx = (self.head + self.len) % cap;
            self.slots[idx] = Some(item);
            self.len += 1;
            None
        }
    }

    pub fn pop_oldest(&mut self) -> Option<T> {
        if self.len == 0 {
            return None;
        }
        let item = self.slots[self.head].take();
        self.head = (self.head + 1) % self.slots.len();
        self.len -= 1;
        item
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        if index >= self.len {
            return None;
        }
        self.slots[(self.head + index) % self.slots.len()].as_ref()
    }

    pub fn oldest(&self) -> Option<&T> {
        self.get(0)
    }

    pub fn newest(&self) -> Option<&T> {
        if self.len == 0 {
            None
        } else {
            self.get(self.len - 1)
        }
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> + ExactSizeIterator {
        (0..self.len).map(move |i| self.get(i).unwrap())
    }

    pub fn clear(&mut self) {
        for s in self.slots.iter_mut() {
            *s = None;
        }
        self.head = 0;
        self.len = 0;
    }
}
