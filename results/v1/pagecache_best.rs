use std::collections::HashMap;
use std::sync::atomic::Ordering;
use crossbeam_epoch::{self as epoch, Atomic, Owned};

pub struct PageCache {
    pages: Atomic<HashMap<u64, Vec<u8>>>,
    dirty: Atomic<HashMap<u64, bool>>,
    page_size: usize,
}

impl PageCache {
    pub fn new(page_size: usize) -> Self {
        Self {
            pages: Atomic::new(HashMap::new()),
            dirty: Atomic::new(HashMap::new()),
            page_size,
        }
    }

    pub fn read(&self, page_id: u64) -> Option<Vec<u8>> {
        let guard = &epoch::pin();
        let pages_ptr = self.pages.load(Ordering::Acquire, guard);
        if pages_ptr.is_null() {
            return None;
        }
        unsafe {
            pages_ptr.as_ref().and_then(|pages| pages.get(&page_id).cloned())
        }
    }

    pub fn write(&self, page_id: u64, data: Vec<u8>) {
        loop {
            let guard = &epoch::pin();
            let pages_ptr = self.pages.load(Ordering::Acquire, guard);
            let dirty_ptr = self.dirty.load(Ordering::Acquire, guard);
            
            let mut new_pages = if pages_ptr.is_null() {
                HashMap::new()
            } else {
                unsafe { pages_ptr.as_ref().unwrap().clone() }
            };
            
            let mut new_dirty = if dirty_ptr.is_null() {
                HashMap::new()
            } else {
                unsafe { dirty_ptr.as_ref().unwrap().clone() }
            };
            
            new_pages.insert(page_id, data.clone());
            new_dirty.insert(page_id, true);
            
            let new_pages_owned = Owned::new(new_pages);
            
            match self.pages.compare_exchange_weak(
                pages_ptr,
                new_pages_owned,
                Ordering::Release,
                Ordering::Relaxed,
                guard,
            ) {
                Ok(old_pages) => {
                    let new_dirty_owned = Owned::new(new_dirty);
                    match self.dirty.compare_exchange_weak(
                        dirty_ptr,
                        new_dirty_owned,
                        Ordering::Release,
                        Ordering::Relaxed,
                        guard,
                    ) {
                        Ok(old_dirty) => {
                            if !old_pages.is_null() {
                                unsafe { guard.defer_destroy(old_pages) };
                            }
                            if !old_dirty.is_null() {
                                unsafe { guard.defer_destroy(old_dirty) };
                            }
                            break;
                        }
                        Err(_) => continue,
                    }
                }
                Err(_) => continue,
            }
        }
    }

    pub fn flush(&self) {
        loop {
            let guard = &epoch::pin();
            let dirty_ptr = self.dirty.load(Ordering::Acquire, guard);
            
            let new_dirty = HashMap::new();
            let new_dirty_owned = Owned::new(new_dirty);
            
            match self.dirty.compare_exchange_weak(
                dirty_ptr,
                new_dirty_owned,
                Ordering::Release,
                Ordering::Relaxed,
                guard,
            ) {
                Ok(old_dirty) => {
                    if !old_dirty.is_null() {
                        unsafe { guard.defer_destroy(old_dirty) };
                    }
                    break;
                }
                Err(_) => continue,
            }
        }
    }

    pub fn dirty_pages(&self) -> usize {
        let guard = &epoch::pin();
        let dirty_ptr = self.dirty.load(Ordering::Acquire, guard);
        if dirty_ptr.is_null() {
            0
        } else {
            unsafe { dirty_ptr.as_ref().unwrap().len() }
        }
    }
}

impl Drop for PageCache {
    fn drop(&mut self) {
        let guard = &epoch::pin();
        let pages_ptr = self.pages.load(Ordering::Acquire, guard);
        let dirty_ptr = self.dirty.load(Ordering::Acquire, guard);
        
        if !pages_ptr.is_null() {
            unsafe { guard.defer_destroy(pages_ptr) };
        }
        if !dirty_ptr.is_null() {
            unsafe { guard.defer_destroy(dirty_ptr) };
        }
    }
}