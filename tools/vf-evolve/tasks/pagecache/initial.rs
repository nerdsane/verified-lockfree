use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

pub struct PageCache {
    inner: Mutex<CacheState>,
    page_size: usize,
}

struct CacheState {
    pages: HashMap<u64, Vec<u8>>,
    dirty: HashSet<u64>,
}

impl PageCache {
    pub fn new(page_size: usize) -> Self {
        Self {
            inner: Mutex::new(CacheState {
                pages: HashMap::new(),
                dirty: HashSet::new(),
            }),
            page_size,
        }
    }

    pub fn read(&self, page_id: u64) -> Option<Vec<u8>> {
        let state = self.inner.lock().unwrap();
        state.pages.get(&page_id).cloned()
    }

    pub fn write(&self, page_id: u64, data: Vec<u8>) {
        let mut state = self.inner.lock().unwrap();
        state.pages.insert(page_id, data);
        state.dirty.insert(page_id);
    }

    pub fn flush(&self) {
        let mut state = self.inner.lock().unwrap();
        state.dirty.clear();
    }

    pub fn dirty_pages(&self) -> usize {
        let state = self.inner.lock().unwrap();
        state.dirty.len()
    }
}
