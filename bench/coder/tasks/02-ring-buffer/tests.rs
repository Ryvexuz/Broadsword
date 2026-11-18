mod common;
use common::within;
use solution::*;
use std::cell::Cell;
use std::rc::Rc;

fn contents(rb: &RingBuffer<i32>) -> Vec<i32> {
    rb.iter().copied().collect()
}

#[test]
fn hidden_new_is_empty() {
    within(|| {
        let rb: RingBuffer<i32> = RingBuffer::new(3);
        assert_eq!(rb.capacity(), 3);
        assert_eq!(rb.len(), 0);
        assert!(rb.is_empty());
        assert!(!rb.is_full());
        assert_eq!(rb.oldest(), None);
        assert_eq!(rb.newest(), None);
        assert_eq!(rb.get(0), None);
        assert_eq!(rb.iter().count(), 0);
    });
}

#[test]
#[should_panic]
fn hidden_zero_capacity_panics() {
    let _rb: RingBuffer<i32> = RingBuffer::new(0);
}

#[test]
fn hidden_fills_without_eviction() {
    within(|| {
        let mut rb = RingBuffer::new(3);
        assert_eq!(rb.push(1), None);
        assert_eq!(rb.push(2), None);
        assert_eq!(rb.len(), 2);
        assert!(!rb.is_full());
        assert_eq!(rb.push(3), None);
        assert!(rb.is_full());
        assert_eq!(contents(&rb), vec![1, 2, 3]);
        assert_eq!(rb.oldest(), Some(&1));
        assert_eq!(rb.newest(), Some(&3));
    });
}

#[test]
fn hidden_evicts_oldest_when_full() {
    within(|| {
        let mut rb = RingBuffer::new(3);
        for i in 1..=3 {
            rb.push(i);
        }
        assert_eq!(rb.push(4), Some(1));
        assert_eq!(contents(&rb), vec![2, 3, 4]);
        assert_eq!(rb.push(5), Some(2));
        assert_eq!(rb.push(6), Some(3));
        assert_eq!(rb.push(7), Some(4));
        assert_eq!(contents(&rb), vec![5, 6, 7]);
        assert_eq!(rb.len(), 3);
    });
}

#[test]
fn hidden_capacity_one() {
    within(|| {
        let mut rb = RingBuffer::new(1);
        assert_eq!(rb.push('a'), None);
        assert!(rb.is_full());
        assert_eq!(rb.push('b'), Some('a'));
        assert_eq!(rb.iter().collect::<Vec<_>>(), vec![&'b']);
        assert_eq!(rb.pop_oldest(), Some('b'));
        assert!(rb.is_empty());
        assert_eq!(rb.push('c'), None);
    });
}

#[test]
fn hidden_get_is_oldest_first_after_wrapping() {
    within(|| {
        let mut rb = RingBuffer::new(4);
        for i in 0..11 {
            rb.push(i);
        }
        // holds 7, 8, 9, 10
        assert_eq!(rb.get(0), Some(&7));
        assert_eq!(rb.get(1), Some(&8));
        assert_eq!(rb.get(3), Some(&10));
        assert_eq!(rb.get(4), None);
        assert_eq!(rb.get(usize::MAX), None);
        assert_eq!(rb.oldest(), Some(&7));
        assert_eq!(rb.newest(), Some(&10));
    });
}

#[test]
fn hidden_iter_order_len_and_rev() {
    within(|| {
        let mut rb = RingBuffer::new(5);
        for i in 0..13 {
            rb.push(i);
        }
        // holds 8..=12
        assert_eq!(contents(&rb), vec![8, 9, 10, 11, 12]);
        assert_eq!(rb.iter().len(), 5);
        assert_eq!(rb.iter().rev().copied().collect::<Vec<_>>(), vec![12, 11, 10, 9, 8]);
        let mut it = rb.iter();
        assert_eq!(it.next(), Some(&8));
        assert_eq!(it.next_back(), Some(&12));
        assert_eq!(it.len(), 3);
        assert_eq!(it.collect::<Vec<_>>(), vec![&9, &10, &11]);
    });
}

#[test]
fn hidden_pop_oldest_frees_a_slot() {
    within(|| {
        let mut rb = RingBuffer::new(3);
        for i in 1..=5 {
            rb.push(i);
        } // 3, 4, 5
        assert_eq!(rb.pop_oldest(), Some(3));
        assert_eq!(rb.len(), 2);
        assert!(!rb.is_full());
        assert_eq!(rb.push(6), None); // no eviction: there was room
        assert_eq!(contents(&rb), vec![4, 5, 6]);
        assert_eq!(rb.pop_oldest(), Some(4));
        assert_eq!(rb.pop_oldest(), Some(5));
        assert_eq!(rb.pop_oldest(), Some(6));
        assert_eq!(rb.pop_oldest(), None);
        assert!(rb.is_empty());
        assert_eq!(rb.get(0), None);
    });
}

#[test]
fn hidden_interleaved_push_pop_keeps_order() {
    within(|| {
        let mut rb = RingBuffer::new(4);
        let mut model: std::collections::VecDeque<i32> = Default::default();
        let mut x = 12345u32;
        for step in 0..2000 {
            x = x.wrapping_mul(1103515245).wrapping_add(12345);
            if (x >> 16) % 3 == 0 {
                assert_eq!(rb.pop_oldest(), model.pop_front(), "step {step}");
            } else {
                let evicted = if model.len() == 4 { model.pop_front() } else { None };
                model.push_back(step);
                assert_eq!(rb.push(step), evicted, "step {step}");
            }
            assert_eq!(contents(&rb), model.iter().copied().collect::<Vec<_>>(), "step {step}");
            assert_eq!(rb.len(), model.len());
        }
    });
}

#[test]
fn hidden_clear_empties_and_keeps_capacity() {
    within(|| {
        let mut rb = RingBuffer::new(3);
        for i in 0..7 {
            rb.push(i);
        }
        rb.clear();
        assert!(rb.is_empty());
        assert_eq!(rb.capacity(), 3);
        assert_eq!(rb.iter().count(), 0);
        assert_eq!(rb.pop_oldest(), None);
        rb.push(100);
        rb.push(101);
        assert_eq!(contents(&rb), vec![100, 101]);
        assert_eq!(rb.get(0), Some(&100));
    });
}

// Not Clone, not Copy, not Default.
struct Counted(u32, Rc<Cell<u32>>);
impl Drop for Counted {
    fn drop(&mut self) {
        self.1.set(self.1.get() + 1);
    }
}

#[test]
fn hidden_elements_dropped_exactly_once() {
    // Rc is not Send, so this test runs without `within`; its work is tiny and cannot loop forever.
    let drops = Rc::new(Cell::new(0));
    let mut rb = RingBuffer::new(2);
    assert!(rb.push(Counted(1, drops.clone())).is_none());
    assert!(rb.push(Counted(2, drops.clone())).is_none());
    assert_eq!(drops.get(), 0);
    let evicted = rb.push(Counted(3, drops.clone()));
    assert_eq!(evicted.as_ref().map(|c| c.0), Some(1));
    assert_eq!(drops.get(), 0, "evicted element is handed back, not dropped");
    drop(evicted);
    assert_eq!(drops.get(), 1);
    let popped = rb.pop_oldest().unwrap();
    assert_eq!(popped.0, 2);
    drop(popped);
    assert_eq!(drops.get(), 2);
    rb.push(Counted(4, drops.clone()));
    rb.push(Counted(5, drops.clone())); // evicts 3, returned and dropped immediately
    assert_eq!(drops.get(), 3);
    rb.clear();
    assert_eq!(drops.get(), 5);
    rb.push(Counted(6, drops.clone()));
    drop(rb);
    assert_eq!(drops.get(), 6);
}

#[test]
fn hidden_works_with_non_default_non_clone_types() {
    within(|| {
        struct Opaque(#[allow(dead_code)] String);
        let mut rb = RingBuffer::new(2);
        rb.push(Opaque("a".into()));
        rb.push(Opaque("b".into()));
        let e = rb.push(Opaque("c".into()));
        assert!(e.is_some());
        assert_eq!(rb.len(), 2);
    });
}

#[test]
fn hidden_large_capacity_and_many_pushes() {
    within(|| {
        let mut rb = RingBuffer::new(1000);
        for i in 0..100_000u32 {
            rb.push(i);
        }
        assert_eq!(rb.len(), 1000);
        assert_eq!(rb.get(0), Some(&99_000));
        assert_eq!(rb.newest(), Some(&99_999));
        assert_eq!(rb.iter().count(), 1000);
    });
}
