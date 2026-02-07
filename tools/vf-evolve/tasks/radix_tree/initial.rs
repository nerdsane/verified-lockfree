use std::collections::HashMap;
use std::sync::Mutex;

pub struct RadixTree {
    inner: Mutex<HashMap<Vec<u8>, u64>>,
}

impl RadixTree {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    pub fn insert(&self, key: &[u8], value: u64) {
        let mut map = self.inner.lock().unwrap();
        map.insert(key.to_vec(), value);
    }

    pub fn get(&self, key: &[u8]) -> Option<u64> {
        let map = self.inner.lock().unwrap();
        map.get(key).copied()
    }

    pub fn remove(&self, key: &[u8]) -> bool {
        let mut map = self.inner.lock().unwrap();
        map.remove(key).is_some()
    }

    pub fn len(&self) -> usize {
        let map = self.inner.lock().unwrap();
        map.len()
    }
}
