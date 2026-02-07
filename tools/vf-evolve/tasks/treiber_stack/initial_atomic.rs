/// Treiber Stack - Intermediate Seed (AtomicPtr-based, no epoch GC)
///
/// This starts evolution on the CAS side of the valley (~score 100-160).
/// It compiles and uses lock-free CAS, but has known UB: no safe memory
/// reclamation (nodes are leaked or freed unsafely). Miri will catch this.
///
/// Evolution should fix the memory reclamation to cross the valley.
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

struct Node<T> {
    value: T,
    next: *mut Node<T>,
}

pub struct TreiberStack<T> {
    head: AtomicPtr<Node<T>>,
}

unsafe impl<T: Send> Send for TreiberStack<T> {}
unsafe impl<T: Send> Sync for TreiberStack<T> {}

impl<T> TreiberStack<T> {
    pub fn new() -> Self {
        TreiberStack {
            head: AtomicPtr::new(ptr::null_mut()),
        }
    }

    pub fn push(&self, value: T) {
        let new_node = Box::into_raw(Box::new(Node {
            value,
            next: ptr::null_mut(),
        }));

        loop {
            let old_head = self.head.load(Ordering::Acquire);
            unsafe {
                (*new_node).next = old_head;
            }
            if self
                .head
                .compare_exchange_weak(old_head, new_node, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                break;
            }
        }
    }

    pub fn pop(&self) -> Option<T> {
        loop {
            let old_head = self.head.load(Ordering::Acquire);
            if old_head.is_null() {
                return None;
            }
            let next = unsafe { (*old_head).next };
            if self
                .head
                .compare_exchange_weak(old_head, next, Ordering::Release, Ordering::Relaxed)
                .is_ok()
            {
                // KNOWN UB: We free the node immediately. Another thread may still
                // be reading old_head. This is the classic ABA / use-after-free bug
                // that epoch-based reclamation solves.
                let node = unsafe { Box::from_raw(old_head) };
                return Some(node.value);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire).is_null()
    }

    pub fn len(&self) -> usize {
        let mut count: usize = 0;
        let mut current = self.head.load(Ordering::Acquire);
        while !current.is_null() {
            count += 1;
            current = unsafe { (*current).next };
        }
        count
    }
}

impl<T> Drop for TreiberStack<T> {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}
