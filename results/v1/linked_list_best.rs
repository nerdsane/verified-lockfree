use crossbeam_epoch::{self as epoch, Atomic, Guard, Owned, Shared};
use std::cmp::Ordering;
use std::sync::atomic::Ordering::{Acquire, Relaxed, Release};

struct Node<T> {
    data: T,
    next: Atomic<Node<T>>,
}

pub struct ConcurrentLinkedList<T> {
    head: Atomic<Node<T>>,
}

impl<T: Ord> ConcurrentLinkedList<T> {
    pub fn new() -> Self {
        ConcurrentLinkedList {
            head: Atomic::null(),
        }
    }

    pub fn insert(&self, value: T) -> bool {
        let guard = &epoch::pin();
        let mut new_node = Owned::new(Node {
            data: value,
            next: Atomic::null(),
        });

        loop {
            let (prev, curr) = self.find(&new_node.data, guard);
            
            if !curr.is_null() {
                unsafe {
                    if curr.deref().data == new_node.data {
                        return false;
                    }
                }
            }

            new_node.next.store(curr, Relaxed);

            match if prev.is_null() {
                self.head.compare_exchange(curr, new_node, Release, Acquire, guard)
            } else {
                unsafe {
                    prev.deref().next.compare_exchange(curr, new_node, Release, Acquire, guard)
                }
            } {
                Ok(_) => return true,
                Err(e) => {
                    new_node = e.new;
                }
            }
        }
    }

    pub fn remove(&self, value: &T) -> bool {
        let guard = &epoch::pin();
        
        loop {
            let (prev, curr) = self.find(value, guard);
            
            if curr.is_null() {
                return false;
            }

            let curr_node = unsafe { curr.deref() };
            if curr_node.data != *value {
                return false;
            }

            let next = curr_node.next.load(Acquire, guard);

            match if prev.is_null() {
                self.head.compare_exchange(curr, next, Release, Acquire, guard)
            } else {
                unsafe {
                    prev.deref().next.compare_exchange(curr, next, Release, Acquire, guard)
                }
            } {
                Ok(_) => {
                    unsafe { guard.defer_destroy(curr) };
                    return true;
                }
                Err(_) => continue,
            }
        }
    }

    pub fn contains(&self, value: &T) -> bool {
        let guard = &epoch::pin();
        let (_, curr) = self.find(value, guard);
        
        if curr.is_null() {
            false
        } else {
            unsafe { curr.deref().data == *value }
        }
    }

    pub fn len(&self) -> usize {
        let guard = &epoch::pin();
        let mut count = 0;
        let mut curr = self.head.load(Acquire, guard);
        
        while !curr.is_null() {
            count += 1;
            curr = unsafe { curr.deref().next.load(Acquire, guard) };
        }
        
        count
    }

    pub fn is_empty(&self) -> bool {
        let guard = &epoch::pin();
        self.head.load(Acquire, guard).is_null()
    }

    fn find<'g>(&self, value: &T, guard: &'g Guard) -> (Shared<'g, Node<T>>, Shared<'g, Node<T>>) {
        let mut prev = Shared::null();
        let mut curr = self.head.load(Acquire, guard);

        while !curr.is_null() {
            let curr_node = unsafe { curr.deref() };
            
            match curr_node.data.cmp(value) {
                Ordering::Less => {
                    prev = curr;
                    curr = curr_node.next.load(Acquire, guard);
                }
                _ => break,
            }
        }

        (prev, curr)
    }
}

impl<T> Drop for ConcurrentLinkedList<T> {
    fn drop(&mut self) {
        let guard = &epoch::pin();
        let mut curr = self.head.load(Relaxed, guard);
        
        while !curr.is_null() {
            let next = unsafe { curr.deref().next.load(Relaxed, guard) };
            unsafe {
                drop(curr.into_owned());
            }
            curr = next;
        }
    }
}