/// Ring Buffer - Initial Seed (Mutex-based blocking implementation)
/// ShinkaEvolve will evolve this from Blocking -> LockFree -> WaitFree
use std::collections::VecDeque;
use std::sync::Mutex;

pub struct RingBuffer<T> {
    inner: Mutex<VecDeque<T>>,
    cap: usize,
}

impl<T> RingBuffer<T> {
    pub fn new(capacity: usize) -> Self {
        debug_assert!(capacity > 0, "capacity must be positive");
        RingBuffer {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            cap: capacity,
        }
    }

    /// Push an item. Returns true if successful, false if buffer is full.
    pub fn push(&self, value: T) -> bool {
        let mut guard = self.inner.lock().unwrap();
        if guard.len() >= self.cap {
            return false;
        }
        guard.push_back(value);
        true
    }

    /// Pop the oldest item from the front.
    pub fn pop(&self) -> Option<T> {
        let mut guard = self.inner.lock().unwrap();
        guard.pop_front()
    }

    pub fn len(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.len()
    }

    pub fn is_empty(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.is_empty()
    }

    pub fn is_full(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.len() >= self.cap
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }
}
