/// Concurrent B+ Tree - Initial Seed (Mutex-based blocking implementation)
/// ShinkaEvolve will evolve this from Blocking -> LockFree -> WaitFree
use std::collections::BTreeMap;
use std::sync::Mutex;

pub struct ConcurrentBTree {
    inner: Mutex<BTreeMap<u64, u64>>,
}

impl ConcurrentBTree {
    pub fn new() -> Self {
        ConcurrentBTree {
            inner: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn insert(&self, key: u64, value: u64) {
        let mut guard = self.inner.lock().unwrap();
        guard.insert(key, value);
    }

    pub fn get(&self, key: &u64) -> Option<u64> {
        let guard = self.inner.lock().unwrap();
        guard.get(key).copied()
    }

    pub fn remove(&self, key: &u64) -> bool {
        let mut guard = self.inner.lock().unwrap();
        guard.remove(key).is_some()
    }

    /// Range scan: return all key-value pairs in [start, end).
    pub fn range(&self, start: u64, end: u64) -> Vec<(u64, u64)> {
        let guard = self.inner.lock().unwrap();
        guard
            .range(start..end)
            .map(|(&k, &v)| (k, v))
            .collect()
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
