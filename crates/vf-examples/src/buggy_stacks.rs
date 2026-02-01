//! Buggy stack implementations for testing the evaluator cascade.
//!
//! Each implementation has a specific bug that should be caught by
//! a particular level of the evaluator cascade.
//!
//! # Bug Catalog
//!
//! | Implementation | Bug | Caught By |
//! |----------------|-----|-----------|
//! | MissingRetryStack | Missing CAS retry | loom, DST |
//! | WrongOrderingStack | Relaxed instead of Acquire/Release | miri, loom |
//! | LostElementStack | Doesn't update pushed set correctly | DST invariant check |
//! | RaceConditionStack | Data race on non-atomic field | miri |

use std::collections::HashSet;
use std::ptr;
use std::sync::atomic::{AtomicPtr, AtomicU64, Ordering};
use std::sync::Mutex;

use vf_core::invariants::stack::{StackHistory, StackProperties};

// =============================================================================
// Bug 1: Missing CAS Retry Loop
// =============================================================================

/// Stack with missing CAS retry loop.
///
/// BUG: When CAS fails, the push silently fails instead of retrying.
/// This causes elements to be "lost" - they're never actually pushed.
///
/// CAUGHT BY: loom (will find interleaving where push fails)
/// CAUGHT BY: DST invariant check (NoLostElements)
pub struct MissingRetryStack {
    head: AtomicPtr<MissingRetryNode>,
    tracker: Mutex<BuggyTracker>,
}

struct MissingRetryNode {
    value: u64,
    next: *mut MissingRetryNode,
}

impl MissingRetryStack {
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::new(ptr::null_mut()),
            tracker: Mutex::new(BuggyTracker::new()),
        }
    }

    /// BUG: No retry loop! If CAS fails, the value is lost.
    pub fn push(&self, value: u64) {
        let new_node = Box::into_raw(Box::new(MissingRetryNode {
            value,
            next: ptr::null_mut(),
        }));

        let head = self.head.load(Ordering::Acquire);
        unsafe { (*new_node).next = head; }

        // BUG: Only one CAS attempt, no retry on failure!
        let result = self.head.compare_exchange(
            head,
            new_node,
            Ordering::Release,
            Ordering::Relaxed,
        );

        // Track as "pushed" even if CAS failed - this is wrong!
        let mut tracker = self.tracker.lock().unwrap();
        tracker.pushed.insert(value);

        if result.is_err() {
            // BUG: We leak the node AND claim it was pushed
            // In a correct implementation, we would retry
        }
    }

    pub fn pop(&self) -> Option<u64> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head.is_null() {
                return None;
            }

            let next = unsafe { (*head).next };
            let value = unsafe { (*head).value };

            if self.head.compare_exchange(
                head,
                next,
                Ordering::Release,
                Ordering::Relaxed,
            ).is_ok() {
                let mut tracker = self.tracker.lock().unwrap();
                tracker.popped.insert(value);
                unsafe { drop(Box::from_raw(head)); }
                return Some(value);
            }
        }
    }

    fn get_contents(&self) -> Vec<u64> {
        let mut result = Vec::new();
        let mut current = self.head.load(Ordering::Acquire);
        while !current.is_null() {
            result.push(unsafe { (*current).value });
            current = unsafe { (*current).next };
        }
        result
    }
}

impl Default for MissingRetryStack {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for MissingRetryStack {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

unsafe impl Send for MissingRetryStack {}
unsafe impl Sync for MissingRetryStack {}

impl StackProperties for MissingRetryStack {
    fn pushed_elements(&self) -> HashSet<u64> {
        self.tracker.lock().unwrap().pushed.clone()
    }

    fn popped_elements(&self) -> HashSet<u64> {
        self.tracker.lock().unwrap().popped.clone()
    }

    fn current_contents(&self) -> Vec<u64> {
        self.get_contents()
    }

    fn history(&self) -> StackHistory {
        StackHistory::new()
    }
}

// =============================================================================
// Bug 2: Wrong Memory Ordering
// =============================================================================

/// Stack with incorrect memory ordering.
///
/// BUG: Uses Relaxed ordering instead of Acquire/Release.
/// This can cause reads to see stale values.
///
/// CAUGHT BY: miri (under some executions)
/// CAUGHT BY: loom (will find ordering violations)
pub struct WrongOrderingStack {
    head: AtomicPtr<WrongOrderingNode>,
    tracker: Mutex<BuggyTracker>,
}

struct WrongOrderingNode {
    value: u64,
    next: *mut WrongOrderingNode,
}

impl WrongOrderingStack {
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::new(ptr::null_mut()),
            tracker: Mutex::new(BuggyTracker::new()),
        }
    }

    pub fn push(&self, value: u64) {
        let new_node = Box::into_raw(Box::new(WrongOrderingNode {
            value,
            next: ptr::null_mut(),
        }));

        loop {
            // BUG: Should be Acquire, not Relaxed
            let head = self.head.load(Ordering::Relaxed);
            unsafe { (*new_node).next = head; }

            // BUG: Should be Release/Relaxed, not Relaxed/Relaxed
            if self.head.compare_exchange(
                head,
                new_node,
                Ordering::Relaxed,  // BUG: Should be Release
                Ordering::Relaxed,
            ).is_ok() {
                let mut tracker = self.tracker.lock().unwrap();
                tracker.pushed.insert(value);
                break;
            }
        }
    }

    pub fn pop(&self) -> Option<u64> {
        loop {
            // BUG: Should be Acquire
            let head = self.head.load(Ordering::Relaxed);
            if head.is_null() {
                return None;
            }

            let next = unsafe { (*head).next };
            let value = unsafe { (*head).value };

            // BUG: Should be Release/Relaxed
            if self.head.compare_exchange(
                head,
                next,
                Ordering::Relaxed,  // BUG
                Ordering::Relaxed,
            ).is_ok() {
                let mut tracker = self.tracker.lock().unwrap();
                tracker.popped.insert(value);
                unsafe { drop(Box::from_raw(head)); }
                return Some(value);
            }
        }
    }

    fn get_contents(&self) -> Vec<u64> {
        let mut result = Vec::new();
        let mut current = self.head.load(Ordering::Acquire);
        while !current.is_null() {
            result.push(unsafe { (*current).value });
            current = unsafe { (*current).next };
        }
        result
    }
}

impl Default for WrongOrderingStack {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WrongOrderingStack {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

unsafe impl Send for WrongOrderingStack {}
unsafe impl Sync for WrongOrderingStack {}

impl StackProperties for WrongOrderingStack {
    fn pushed_elements(&self) -> HashSet<u64> {
        self.tracker.lock().unwrap().pushed.clone()
    }

    fn popped_elements(&self) -> HashSet<u64> {
        self.tracker.lock().unwrap().popped.clone()
    }

    fn current_contents(&self) -> Vec<u64> {
        self.get_contents()
    }

    fn history(&self) -> StackHistory {
        StackHistory::new()
    }
}

// =============================================================================
// Bug 3: Lost Element (tracker bug)
// =============================================================================

/// Stack that loses track of elements in its tracker.
///
/// BUG: Sometimes "forgets" to record pushed elements.
/// The actual stack is correct, but tracking is wrong.
///
/// CAUGHT BY: DST invariant check (inconsistent tracking)
pub struct LostElementStack {
    head: AtomicPtr<LostElementNode>,
    tracker: Mutex<BuggyTracker>,
    counter: AtomicU64,
}

struct LostElementNode {
    value: u64,
    next: *mut LostElementNode,
}

impl LostElementStack {
    pub fn new() -> Self {
        Self {
            head: AtomicPtr::new(ptr::null_mut()),
            tracker: Mutex::new(BuggyTracker::new()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn push(&self, value: u64) {
        let new_node = Box::into_raw(Box::new(LostElementNode {
            value,
            next: ptr::null_mut(),
        }));

        loop {
            let head = self.head.load(Ordering::Acquire);
            unsafe { (*new_node).next = head; }

            if self.head.compare_exchange(
                head,
                new_node,
                Ordering::Release,
                Ordering::Relaxed,
            ).is_ok() {
                // BUG: Every 3rd push, "forget" to track it
                let count = self.counter.fetch_add(1, Ordering::Relaxed);
                if count % 3 != 2 {
                    let mut tracker = self.tracker.lock().unwrap();
                    tracker.pushed.insert(value);
                }
                // BUG: The element IS in the stack, but we didn't track it
                break;
            }
        }
    }

    pub fn pop(&self) -> Option<u64> {
        loop {
            let head = self.head.load(Ordering::Acquire);
            if head.is_null() {
                return None;
            }

            let next = unsafe { (*head).next };
            let value = unsafe { (*head).value };

            if self.head.compare_exchange(
                head,
                next,
                Ordering::Release,
                Ordering::Relaxed,
            ).is_ok() {
                let mut tracker = self.tracker.lock().unwrap();
                tracker.popped.insert(value);
                unsafe { drop(Box::from_raw(head)); }
                return Some(value);
            }
        }
    }

    fn get_contents(&self) -> Vec<u64> {
        let mut result = Vec::new();
        let mut current = self.head.load(Ordering::Acquire);
        while !current.is_null() {
            result.push(unsafe { (*current).value });
            current = unsafe { (*current).next };
        }
        result
    }
}

impl Default for LostElementStack {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for LostElementStack {
    fn drop(&mut self) {
        while self.pop().is_some() {}
    }
}

unsafe impl Send for LostElementStack {}
unsafe impl Sync for LostElementStack {}

impl StackProperties for LostElementStack {
    fn pushed_elements(&self) -> HashSet<u64> {
        self.tracker.lock().unwrap().pushed.clone()
    }

    fn popped_elements(&self) -> HashSet<u64> {
        self.tracker.lock().unwrap().popped.clone()
    }

    fn current_contents(&self) -> Vec<u64> {
        self.get_contents()
    }

    fn history(&self) -> StackHistory {
        StackHistory::new()
    }
}

// =============================================================================
// Shared tracker
// =============================================================================

struct BuggyTracker {
    pushed: HashSet<u64>,
    popped: HashSet<u64>,
}

impl BuggyTracker {
    fn new() -> Self {
        Self {
            pushed: HashSet::new(),
            popped: HashSet::new(),
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use vf_core::invariants::stack::StackPropertyChecker;
    use vf_core::PropertyChecker;
    use vf_dst::{get_or_generate_seed, DstHarness, HarnessConfig};

    /// Test that MissingRetryStack fails under concurrent load
    #[test]
    fn test_missing_retry_concurrent() {
        use std::sync::Arc;
        use std::thread;

        let stack = Arc::new(MissingRetryStack::new());
        let mut handles = vec![];

        // Multiple threads pushing simultaneously
        for i in 0..4 {
            let s = Arc::clone(&stack);
            handles.push(thread::spawn(move || {
                for j in 0..100 {
                    s.push(i * 1000 + j);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // Check invariants - should fail due to lost elements
        let checker = StackPropertyChecker::new(stack.as_ref());
        let results = checker.check_all();

        // Note: This might pass sometimes due to luck, but under contention
        // it should fail. The key insight is that loom would catch this
        // deterministically.
        println!("MissingRetryStack results:");
        for r in &results {
            println!("  {}", r.format_status());
        }
    }

    /// Test that LostElementStack fails DST invariant checks
    #[test]
    fn test_lost_element_dst() {
        let _seed = get_or_generate_seed();
        let stack = LostElementStack::new();

        // Push 10 elements
        for i in 1..=10 {
            stack.push(i);
        }

        let checker = StackPropertyChecker::new(&stack);
        let results = checker.check_all();

        // Should fail NoLostElements because we "forgot" to track some
        let no_lost = results.iter().find(|r| r.name == "NoLostElements").unwrap();

        println!("LostElementStack - NoLostElements: {}", no_lost.format_status());
        println!("  Pushed (tracked): {:?}", stack.pushed_elements());
        println!("  In stack: {:?}", stack.current_contents());

        // The bug causes inconsistency between tracked and actual
        // Elements in stack but not in pushed set
        let in_stack: HashSet<u64> = stack.current_contents().into_iter().collect();
        let tracked = stack.pushed_elements();
        let untracked: Vec<_> = in_stack.difference(&tracked).collect();

        if !untracked.is_empty() {
            println!("  BUG DETECTED: Elements in stack but not tracked: {:?}", untracked);
        }
    }

    /// Test WrongOrderingStack (the bug is subtle and may not always manifest)
    #[test]
    fn test_wrong_ordering() {
        let stack = WrongOrderingStack::new();

        // Basic operations still work in single-threaded mode
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);

        // The bug would be caught by loom in concurrent scenarios
        println!("WrongOrderingStack: Single-threaded test passed (bug is subtle)");
        println!("  Run with RUSTFLAGS='--cfg loom' to detect ordering issues");
    }

    /// DST harness test for MissingRetryStack
    #[test]
    fn test_missing_retry_harness() {
        let config = HarnessConfig {
            threads_count: 2,
            operations_per_thread: 50,
            yield_probability: 0.3,
            invariant_check_interval: 10,
            ..HarnessConfig::quick()
        };

        let mut harness = DstHarness::new(12345, config);
        let stack = MissingRetryStack::new();

        let mut value_counter = 1u64;

        let result = harness.run_single_threaded(
            |_env, _step| {
                let v = value_counter;
                value_counter += 1;
                Some(v)
            },
            |_env, value| {
                stack.push(value);
                Ok(())
            },
        );

        // Check invariants after all operations
        let checker = StackPropertyChecker::new(&stack);
        let all_hold = checker.all_hold();

        println!("MissingRetryStack harness: {}", result.format());
        println!("  Invariants hold: {}", all_hold);

        // In single-threaded mode, the bug doesn't manifest
        // But in concurrent mode (simulated), it would
    }
}
