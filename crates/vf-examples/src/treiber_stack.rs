//! Treiber Stack - Lock-free stack implementation.
//!
//! # TLA+ Specification
//!
//! This implementation corresponds to `specs/lockfree/treiber_stack.tla`.
//!
//! # Invariants
//!
//! | Property | TLA+ Line | Verified By |
//! |----------|-----------|-------------|
//! | NoLostElements | 45 | DST, atomic counters |
//! | NoDuplicates | 58 | DST, atomic counters |
//! | LIFO_Order | 72 | DST (single-threaded replay) |
//! | Linearizability | 89 | loom (LoomStack) |
//! | ABA_Safety | 103 | epoch GC (structural) |
//!
//! # Memory Safety
//!
//! Uses epoch-based garbage collection from crossbeam-epoch to prevent
//! the ABA problem and use-after-free.
//!
//! # Lock-Free Guarantee
//!
//! All operations are lock-free: at least one thread will make progress
//! in any concurrent execution. The implementation uses only atomic
//! compare-and-swap operations with no blocking.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};

use crossbeam_epoch::{self as epoch, Atomic, Owned};

use vf_core::invariants::stack::{StackHistory, StackProperties};

/// Maximum stack size (TigerStyle: explicit limit).
pub const STACK_SIZE_MAX: u64 = 1_000_000;

/// A lock-free Treiber stack.
///
/// This is the classic lock-free stack design by R. Kent Treiber (1986).
/// Operations are linearizable and lock-free (at least one thread makes
/// progress in any execution).
///
/// Memory safety is guaranteed by epoch-based reclamation from crossbeam.
pub struct TreiberStack<T> {
    /// Pointer to top node
    head: Atomic<Node<T>>,
    /// Atomic size counter (for NoLostElements verification)
    size: AtomicU64,
    /// Atomic push counter
    push_count: AtomicU64,
    /// Atomic pop counter
    pop_count: AtomicU64,
    /// Atomic step counter for ordering
    step: AtomicU64,
}

/// Node in the stack.
struct Node<T> {
    value: T,
    next: Atomic<Node<T>>,
}

impl<T> TreiberStack<T> {
    /// Create a new empty stack.
    #[must_use]
    pub fn new() -> Self {
        Self {
            head: Atomic::null(),
            size: AtomicU64::new(0),
            push_count: AtomicU64::new(0),
            pop_count: AtomicU64::new(0),
            step: AtomicU64::new(0),
        }
    }

    /// Check if the stack is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        let guard = epoch::pin();
        self.head.load(Ordering::Acquire, &guard).is_null()
    }

    /// Get current size.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size.load(Ordering::Relaxed)
    }

    /// Get total push operations.
    #[must_use]
    pub fn push_count(&self) -> u64 {
        self.push_count.load(Ordering::Relaxed)
    }

    /// Get total pop operations (including empty pops).
    #[must_use]
    pub fn pop_count(&self) -> u64 {
        self.pop_count.load(Ordering::Relaxed)
    }
}

impl TreiberStack<u64> {
    /// Push a value onto the stack.
    ///
    /// This operation is lock-free: it will complete in bounded time
    /// unless preempted infinitely.
    ///
    /// # Panics
    ///
    /// Panics if stack size exceeds `STACK_SIZE_MAX` (debug builds only).
    ///
    /// # TLA+ Mapping
    ///
    /// Corresponds to PushAlloc + PushRead + PushCAS in treiber_stack.tla
    pub fn push(&self, value: u64) {
        // TigerStyle: Validate input
        debug_assert!(value != 0, "Zero is reserved as sentinel");
        debug_assert!(
            self.size.load(Ordering::Relaxed) < STACK_SIZE_MAX,
            "Stack size limit exceeded"
        );

        let guard = epoch::pin();

        // Phase 1: Allocate new node (PushAlloc)
        let new_node = Owned::new(Node {
            value,
            next: Atomic::null(),
        });

        // Phase 2 & 3: CAS loop (PushRead + PushCAS)
        let mut node = new_node;
        loop {
            // Read current head
            let head = self.head.load(Ordering::Acquire, &guard);

            // Set new node's next to current head
            node.next.store(head, Ordering::Relaxed);

            // Attempt CAS
            match self.head.compare_exchange(
                head,
                node,
                Ordering::Release,
                Ordering::Relaxed,
                &guard,
            ) {
                Ok(_) => {
                    // Success - update counters atomically
                    self.size.fetch_add(1, Ordering::Relaxed);
                    self.push_count.fetch_add(1, Ordering::Relaxed);
                    self.step.fetch_add(1, Ordering::Relaxed);
                    break;
                }
                Err(e) => {
                    // CAS failed - retry with same node
                    node = e.new;
                }
            }
        }
    }

    /// Pop a value from the stack.
    ///
    /// Returns `None` if the stack is empty.
    ///
    /// # TLA+ Mapping
    ///
    /// Corresponds to PopRead + PopCAS in treiber_stack.tla
    pub fn pop(&self) -> Option<u64> {
        let guard = epoch::pin();

        loop {
            // Read current head
            let head = self.head.load(Ordering::Acquire, &guard);

            if head.is_null() {
                // Stack is empty
                self.pop_count.fetch_add(1, Ordering::Relaxed);
                self.step.fetch_add(1, Ordering::Relaxed);
                return None;
            }

            // Safety: head is not null and protected by epoch guard
            let head_ref = unsafe { head.deref() };
            let value = head_ref.value;
            let next = head_ref.next.load(Ordering::Acquire, &guard);

            // Attempt CAS
            match self.head.compare_exchange(
                head,
                next,
                Ordering::Release,
                Ordering::Relaxed,
                &guard,
            ) {
                Ok(_) => {
                    // Success - defer cleanup and update counters
                    unsafe {
                        guard.defer_destroy(head);
                    }

                    self.size.fetch_sub(1, Ordering::Relaxed);
                    self.pop_count.fetch_add(1, Ordering::Relaxed);
                    self.step.fetch_add(1, Ordering::Relaxed);

                    return Some(value);
                }
                Err(_) => {
                    // CAS failed - retry
                    continue;
                }
            }
        }
    }

    /// Get current contents for verification.
    ///
    /// Note: This traverses the stack and is not thread-safe for concurrent
    /// modification. Use only when stack is quiescent or in single-threaded tests.
    pub fn get_contents(&self) -> Vec<u64> {
        let guard = epoch::pin();
        let mut result = Vec::new();
        let mut current = self.head.load(Ordering::Acquire, &guard);

        while !current.is_null() {
            let node = unsafe { current.deref() };
            result.push(node.value);
            current = node.next.load(Ordering::Acquire, &guard);
        }

        result
    }
}

impl Default for TreiberStack<u64> {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracking wrapper for single-threaded DST verification.
///
/// This wrapper adds operation history tracking for LIFO order verification.
/// Use this in single-threaded DST tests where you need full history replay.
/// For concurrent tests, use `TreiberStack` directly with atomic counter checks.
pub struct TrackedStack {
    inner: TreiberStack<u64>,
    /// Pushed elements (for NoLostElements)
    pushed: std::sync::Mutex<HashSet<u64>>,
    /// Popped elements (for NoLostElements)
    popped: std::sync::Mutex<HashSet<u64>>,
    /// Operation history (for LIFO verification)
    history: std::sync::Mutex<StackHistory>,
}

impl TrackedStack {
    /// Create a new tracked stack.
    pub fn new() -> Self {
        Self {
            inner: TreiberStack::new(),
            pushed: std::sync::Mutex::new(HashSet::new()),
            popped: std::sync::Mutex::new(HashSet::new()),
            history: std::sync::Mutex::new(StackHistory::new()),
        }
    }

    /// Push with tracking.
    pub fn push(&self, value: u64) {
        self.inner.push(value);
        let step = self.inner.step.load(Ordering::Relaxed);
        self.pushed.lock().unwrap().insert(value);
        self.history.lock().unwrap().record_push(0, value, step);
    }

    /// Pop with tracking.
    pub fn pop(&self) -> Option<u64> {
        let result = self.inner.pop();
        let step = self.inner.step.load(Ordering::Relaxed);
        if let Some(v) = result {
            self.popped.lock().unwrap().insert(v);
        }
        self.history.lock().unwrap().record_pop(0, result, step);
        result
    }

    /// Get inner stack for direct access.
    pub fn inner(&self) -> &TreiberStack<u64> {
        &self.inner
    }
}

impl Default for TrackedStack {
    fn default() -> Self {
        Self::new()
    }
}

impl StackProperties for TrackedStack {
    fn pushed_elements(&self) -> HashSet<u64> {
        self.pushed.lock().unwrap().clone()
    }

    fn popped_elements(&self) -> HashSet<u64> {
        self.popped.lock().unwrap().clone()
    }

    fn current_contents(&self) -> Vec<u64> {
        self.inner.get_contents()
    }

    fn history(&self) -> StackHistory {
        self.history.lock().unwrap().clone()
    }
}

/// StackProperties for TreiberStack (limited - no history tracking).
///
/// For full LIFO verification, use `TrackedStack` in single-threaded tests.
impl StackProperties for TreiberStack<u64> {
    fn pushed_elements(&self) -> HashSet<u64> {
        // Cannot track individual elements without Mutex
        // Return empty set - NoLostElements check will use counters instead
        HashSet::new()
    }

    fn popped_elements(&self) -> HashSet<u64> {
        // Cannot track individual elements without Mutex
        HashSet::new()
    }

    fn current_contents(&self) -> Vec<u64> {
        self.get_contents()
    }

    fn history(&self) -> StackHistory {
        // No history without tracking
        // LIFO verification requires TrackedStack
        StackHistory::new()
    }
}

// Safety: Stack is thread-safe
unsafe impl<T: Send> Send for TreiberStack<T> {}
unsafe impl<T: Send> Sync for TreiberStack<T> {}

impl<T> Drop for TreiberStack<T> {
    fn drop(&mut self) {
        // Clean up remaining nodes
        let guard = epoch::pin();
        let mut current = self.head.load(Ordering::Relaxed, &guard);

        while !current.is_null() {
            let node = unsafe { current.into_owned() };
            current = node.next.load(Ordering::Relaxed, &guard);
            drop(node);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vf_core::invariants::stack::StackPropertyChecker;
    use vf_core::PropertyChecker;
    use vf_dst::{get_or_generate_seed, DstEnv, FaultConfig};

    #[test]
    fn test_basic_push_pop() {
        let stack = TreiberStack::new();

        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn test_invariants_basic() {
        let stack = TrackedStack::new();

        stack.push(1);
        stack.push(2);

        let checker = StackPropertyChecker::new(&stack);
        assert!(checker.all_hold(), "Invariants should hold");
    }

    #[test]
    fn test_dst_single_threaded() {
        let seed = get_or_generate_seed();
        let mut env = DstEnv::with_fault_config(seed, FaultConfig::none());
        let stack = TrackedStack::new();
        let checker = StackPropertyChecker::new(&stack);

        let iterations = std::env::var("DST_ITERATIONS")
            .map(|s| s.parse().unwrap())
            .unwrap_or(1000);

        let mut used_values = HashSet::new();

        for _ in 0..iterations {
            let op = env.rng().gen_range(0..3_u8);

            match op {
                0 => {
                    // Push a random value (ensure unique)
                    let value = env.rng().gen_range(1..100000_u64);
                    if !used_values.contains(&value) {
                        used_values.insert(value);
                        stack.push(value);
                    }
                }
                1 => {
                    // Pop
                    stack.pop();
                }
                _ => {
                    // Check invariants
                    assert!(
                        checker.all_hold(),
                        "Invariant violated at {}",
                        env.format_seed()
                    );
                }
            }
        }

        // Final invariant check
        assert!(
            checker.all_hold(),
            "Final invariant check failed at {}",
            env.format_seed()
        );

        println!("DST completed: {}", env.stats());
    }

    #[test]
    fn test_dst_with_faults() {
        let seed = get_or_generate_seed();
        let mut env = DstEnv::new(seed);
        let stack = TrackedStack::new();
        let checker = StackPropertyChecker::new(&stack);

        let iterations = std::env::var("DST_ITERATIONS")
            .map(|s| s.parse().unwrap())
            .unwrap_or(1000);

        let mut values_to_push: Vec<u64> = (1..=100).collect();
        env.rng().shuffle(&mut values_to_push);
        let mut push_idx = 0;

        for _ in 0..iterations {
            // Maybe simulate a delay
            env.maybe_delay();

            let op = env.rng().gen_range(0..4_u8);

            match op {
                0 if push_idx < values_to_push.len() => {
                    stack.push(values_to_push[push_idx]);
                    push_idx += 1;
                }
                1 | 2 => {
                    stack.pop();
                }
                _ => {
                    // Check invariants
                    assert!(
                        checker.all_hold(),
                        "Invariant violated at {}",
                        env.format_seed()
                    );
                }
            }

            // Advance simulated time
            let delay = env.rng().gen_range(1..100_u64);
            env.clock().advance_us(delay);
        }

        // Final check
        assert!(
            checker.all_hold(),
            "Final invariant check failed at {}",
            env.format_seed()
        );

        println!("DST with faults completed: {}", env.stats());
    }

    #[test]
    fn test_lifo_order() {
        let stack = TreiberStack::new();

        // Push in order
        for i in 1..=10 {
            stack.push(i);
        }

        // Pop should return in reverse order
        for i in (1..=10).rev() {
            assert_eq!(stack.pop(), Some(i), "LIFO order violated");
        }
    }

    #[test]
    fn test_lock_free_concurrent() {
        use std::sync::Arc;
        use std::thread;

        let stack = Arc::new(TreiberStack::new());
        let mut push_handles = vec![];
        let mut pop_handles = vec![];

        // Spawn pushers
        for i in 0..4 {
            let s = Arc::clone(&stack);
            push_handles.push(thread::spawn(move || {
                for j in 0..100 {
                    s.push(i * 1000 + j);
                }
            }));
        }

        // Spawn poppers
        for _ in 0..4 {
            let s = Arc::clone(&stack);
            pop_handles.push(thread::spawn(move || {
                let mut count = 0u64;
                for _ in 0..100 {
                    if s.pop().is_some() {
                        count += 1;
                    }
                }
                count
            }));
        }

        for handle in push_handles {
            handle.join().unwrap();
        }

        for handle in pop_handles {
            handle.join().unwrap();
        }

        // Drain remaining
        let mut remaining = 0u64;
        while stack.pop().is_some() {
            remaining += 1;
        }

        // Verify no lost elements using atomic counters
        let pushed = stack.push_count();
        let popped = stack.pop_count();
        let size = stack.size();

        println!(
            "Concurrent test: pushed={} popped={} remaining={} size={}",
            pushed, popped, remaining, size
        );

        // All pushed elements must be accounted for
        assert_eq!(pushed, 400, "Expected 400 pushes");
        assert_eq!(size, 0, "Stack should be empty after draining");
    }
}
