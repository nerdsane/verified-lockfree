/// Concurrent Linked List - Initial Seed (Mutex-based blocking implementation)
/// ShinkaEvolve will evolve this from Blocking -> LockFree -> WaitFree
use std::collections::BTreeSet;
use std::sync::Mutex;

pub struct ConcurrentLinkedList<T: Ord> {
    inner: Mutex<BTreeSet<T>>,
}

impl<T: Ord> ConcurrentLinkedList<T> {
    pub fn new() -> Self {
        ConcurrentLinkedList {
            inner: Mutex::new(BTreeSet::new()),
        }
    }

    pub fn insert(&self, value: T) -> bool {
        let mut guard = self.inner.lock().unwrap();
        guard.insert(value)
    }

    pub fn remove(&self, value: &T) -> bool {
        let mut guard = self.inner.lock().unwrap();
        guard.remove(value)
    }

    pub fn contains(&self, value: &T) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.contains(value)
    }

    pub fn len(&self) -> usize {
        let guard = self.inner.lock().unwrap();
        guard.len()
    }

    pub fn is_empty(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.is_empty()
    }
}
