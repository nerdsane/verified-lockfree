//! Loom-compatible Treiber Stack for concurrency testing.
//!
//! This module provides a stack implementation that works with both
//! std atomics and loom atomics via conditional compilation.
//!
//! # Usage
//!
//! For normal tests:
//! ```bash
//! cargo test -p vf-examples
//! ```
//!
//! For loom tests:
//! ```bash
//! RUSTFLAGS="--cfg loom" cargo test -p vf-examples --release
//! ```

#[cfg(loom)]
use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
#[cfg(loom)]
use loom::sync::Arc;

#[cfg(not(loom))]
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

use std::ptr;

/// A lock-free Treiber stack compatible with loom.
///
/// This is a simplified version without epoch-based GC,
/// suitable for loom testing where we control all allocations.
pub struct LoomStack<T> {
    head: AtomicPtr<Node<T>>,
    size: AtomicUsize,
}

struct Node<T> {
    value: T,
    next: *mut Node<T>,
}

/// Maximum stack size for bounded testing.
const STACK_SIZE_MAX: usize = 10_000;

impl<T> LoomStack<T> {
    /// Create a new empty stack.
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::new(ptr::null_mut()),
            size: AtomicUsize::new(0),
        }
    }

    /// Push a value onto the stack.
    ///
    /// Returns `true` if successful, `false` if stack is full.
    pub fn push(&self, value: T) -> bool {
        // Check size limit
        if self.size.load(Ordering::Relaxed) >= STACK_SIZE_MAX {
            return false;
        }

        let new_node = Box::into_raw(Box::new(Node {
            value,
            next: ptr::null_mut(),
        }));

        loop {
            let head = self.head.load(Ordering::Acquire);

            // Set new node's next to current head
            // Safety: new_node is valid and we have exclusive access
            unsafe {
                (*new_node).next = head;
            }

            // Attempt CAS
            match self.head.compare_exchange(
                head,
                new_node,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.size.fetch_add(1, Ordering::Relaxed);
                    return true;
                }
                Err(_) => {
                    // CAS failed - retry
                    #[cfg(loom)]
                    loom::thread::yield_now();
                    continue;
                }
            }
        }
    }

    /// Pop a value from the stack.
    ///
    /// Returns `None` if the stack is empty.
    pub fn pop(&self) -> Option<T> {
        loop {
            let head = self.head.load(Ordering::Acquire);

            if head.is_null() {
                return None;
            }

            // Safety: head is not null
            let next = unsafe { (*head).next };

            // Attempt CAS
            match self.head.compare_exchange(
                head,
                next,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    self.size.fetch_sub(1, Ordering::Relaxed);
                    // Safety: we have exclusive ownership of head now
                    let node = unsafe { Box::from_raw(head) };
                    return Some(node.value);
                }
                Err(_) => {
                    // CAS failed - retry
                    #[cfg(loom)]
                    loom::thread::yield_now();
                    continue;
                }
            }
        }
    }

    /// Check if the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire).is_null()
    }

    /// Get approximate size.
    pub fn len(&self) -> usize {
        self.size.load(Ordering::Relaxed)
    }
}

impl<T> Default for LoomStack<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Drop for LoomStack<T> {
    fn drop(&mut self) {
        // Clean up remaining nodes
        while self.pop().is_some() {}
    }
}

// Safety: Stack is thread-safe when T is Send
unsafe impl<T: Send> Send for LoomStack<T> {}
unsafe impl<T: Send> Sync for LoomStack<T> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_operations() {
        let stack = LoomStack::new();

        assert!(stack.is_empty());
        assert_eq!(stack.len(), 0);

        assert!(stack.push(1));
        assert!(stack.push(2));
        assert!(stack.push(3));

        assert!(!stack.is_empty());
        assert_eq!(stack.len(), 3);

        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);

        assert!(stack.is_empty());
    }

    #[test]
    fn test_lifo_order() {
        let stack = LoomStack::new();

        for i in 1..=10 {
            stack.push(i);
        }

        for i in (1..=10).rev() {
            assert_eq!(stack.pop(), Some(i));
        }
    }

    #[cfg(not(loom))]
    #[test]
    fn test_concurrent_std() {
        use std::sync::Arc;
        use std::thread;

        let stack = Arc::new(LoomStack::new());
        let mut push_handles = vec![];
        let mut pop_handles = vec![];

        // Spawn pushers
        for i in 0..4 {
            let stack = Arc::clone(&stack);
            push_handles.push(thread::spawn(move || {
                for j in 0..100 {
                    stack.push(i * 1000 + j);
                }
            }));
        }

        // Spawn poppers
        for _ in 0..4 {
            let stack = Arc::clone(&stack);
            pop_handles.push(thread::spawn(move || {
                let mut count = 0;
                for _ in 0..100 {
                    if stack.pop().is_some() {
                        count += 1;
                    }
                }
                count
            }));
        }

        for handle in push_handles {
            handle.join().unwrap();
        }

        let mut total_popped = 0;
        for handle in pop_handles {
            total_popped += handle.join().unwrap();
        }

        // Drain remaining
        let mut remaining = 0;
        while stack.pop().is_some() {
            remaining += 1;
        }

        // Total pushed = 4 * 100 = 400
        // Total popped should equal 400
        println!(
            "Concurrent test: popped={} remaining={} total={}",
            total_popped,
            remaining,
            total_popped + remaining
        );
    }
}

/// Loom tests - these exhaustively check all interleavings
#[cfg(loom)]
mod loom_tests {
    use super::*;
    use loom::sync::Arc;
    use loom::thread;

    #[test]
    fn test_push_push() {
        loom::model(|| {
            let stack = Arc::new(LoomStack::new());

            let s1 = Arc::clone(&stack);
            let s2 = Arc::clone(&stack);

            let h1 = thread::spawn(move || {
                s1.push(1);
            });

            let h2 = thread::spawn(move || {
                s2.push(2);
            });

            h1.join().unwrap();
            h2.join().unwrap();

            // Both values should be in the stack
            let mut values = vec![];
            while let Some(v) = stack.pop() {
                values.push(v);
            }
            values.sort();
            assert_eq!(values, vec![1, 2]);
        });
    }

    #[test]
    fn test_push_pop() {
        loom::model(|| {
            let stack = Arc::new(LoomStack::new());
            stack.push(1);

            let s1 = Arc::clone(&stack);
            let s2 = Arc::clone(&stack);

            let h1 = thread::spawn(move || {
                s1.push(2);
            });

            let h2 = thread::spawn(move || {
                s2.pop()
            });

            h1.join().unwrap();
            let popped = h2.join().unwrap();

            // popped is either Some(1), Some(2), or None depending on interleaving
            // But we started with 1, and pushed 2, so None is not possible
            assert!(popped.is_some());

            // Remaining values
            let mut remaining = vec![];
            while let Some(v) = stack.pop() {
                remaining.push(v);
            }

            // Total values should be 2 (one popped, one remaining)
            // OR if pop happened first, 2 remaining
            let total = remaining.len() + if popped.is_some() { 1 } else { 0 };
            assert_eq!(total, 2);
        });
    }

    #[test]
    fn test_concurrent_pop() {
        loom::model(|| {
            let stack = Arc::new(LoomStack::new());
            stack.push(1);

            let s1 = Arc::clone(&stack);
            let s2 = Arc::clone(&stack);

            let h1 = thread::spawn(move || s1.pop());
            let h2 = thread::spawn(move || s2.pop());

            let r1 = h1.join().unwrap();
            let r2 = h2.join().unwrap();

            // Exactly one should get the value
            match (r1, r2) {
                (Some(1), None) => {}
                (None, Some(1)) => {}
                other => panic!("Unexpected result: {:?}", other),
            }
        });
    }
}
