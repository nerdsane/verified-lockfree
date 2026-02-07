use std::sync::atomic::{AtomicPtr, Ordering};
use crossbeam_epoch::{self as epoch, Atomic, Guard, Owned, Shared};
use std::ptr;

struct Node<T> {
    data: T,
    next: Atomic<Node<T>>,
}

pub struct TreiberStack<T> {
    head: Atomic<Node<T>>,
}

impl<T> TreiberStack<T> {
    pub fn new() -> Self {
        TreiberStack {
            head: Atomic::null(),
        }
    }

    pub fn push(&self, value: T) {
        let guard = &epoch::pin();
        let mut new_node = Owned::new(Node {
            data: value,
            next: Atomic::null(),
        });

        loop {
            let head = self.head.load(Ordering::Acquire, guard);
            new_node.next.store(head, Ordering::Relaxed);
            
            match self.head.compare_exchange(
                head,
                new_node,
                Ordering::Release,
                Ordering::Acquire,
                guard,
            ) {
                Ok(_) => break,
                Err(e) => new_node = e.new,
            }
        }
    }

    pub fn pop(&self) -> Option<T> {
        let guard = &epoch::pin();
        loop {
            let head = self.head.load(Ordering::Acquire, guard);
            match unsafe { head.as_ref() } {
                None => return None,
                Some(node) => {
                    let next = node.next.load(Ordering::Acquire, guard);
                    match self.head.compare_exchange(
                        head,
                        next,
                        Ordering::Release,
                        Ordering::Acquire,
                        guard,
                    ) {
                        Ok(_) => {
                            let data = unsafe { ptr::read(&node.data) };
                            unsafe {
                                guard.defer_destroy(head);
                            }
                            return Some(data);
                        }
                        Err(_) => continue,
                    }
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        let guard = &epoch::pin();
        self.head.load(Ordering::Acquire, guard).is_null()
    }

    pub fn len(&self) -> usize {
        let guard = &epoch::pin();
        let mut count = 0;
        let mut current = self.head.load(Ordering::Acquire, guard);
        
        while let Some(node) = unsafe { current.as_ref() } {
            count += 1;
            current = node.next.load(Ordering::Acquire, guard);
        }
        
        count
    }
}

unsafe impl<T: Send> Send for TreiberStack<T> {}
unsafe impl<T: Send> Sync for TreiberStack<T> {}