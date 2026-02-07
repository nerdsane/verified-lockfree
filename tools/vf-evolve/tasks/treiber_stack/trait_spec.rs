/// Trait specification tests for TreiberStack<T>
///
/// The evolved code must provide:
///   pub struct TreiberStack<T> { ... }
///   impl<T> TreiberStack<T> {
///       pub fn new() -> Self;
///       pub fn push(&self, val: T);
///       pub fn pop(&self) -> Option<T>;
///       pub fn is_empty(&self) -> bool;
///   }
///
/// All operations must be lock-free and safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_new_stack_is_empty() {
        let stack: TreiberStack<i32> = TreiberStack::new();
        assert!(stack.is_empty());
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn test_push_pop_single() {
        let stack = TreiberStack::new();
        stack.push(42);
        assert!(!stack.is_empty());
        assert_eq!(stack.pop(), Some(42));
        assert!(stack.is_empty());
    }

    #[test]
    fn test_lifo_order() {
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
    fn test_interleaved_push_pop() {
        let stack = TreiberStack::new();
        stack.push(10);
        stack.push(20);
        assert_eq!(stack.pop(), Some(20));
        stack.push(30);
        assert_eq!(stack.pop(), Some(30));
        assert_eq!(stack.pop(), Some(10));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn test_is_empty_reflects_state() {
        let stack = TreiberStack::new();
        assert!(stack.is_empty());
        stack.push(1);
        assert!(!stack.is_empty());
        stack.pop();
        assert!(stack.is_empty());
    }

    #[test]
    fn test_concurrent_push_no_lost_elements() {
        const NUM_THREADS: usize = 8;
        const OPS_PER_THREAD: usize = 1000;

        let stack = Arc::new(TreiberStack::new());

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let stack = Arc::clone(&stack);
                s.spawn(move || {
                    for i in 0..OPS_PER_THREAD {
                        stack.push(t * OPS_PER_THREAD + i);
                    }
                });
            }
        });

        let mut popped = Vec::new();
        while let Some(val) = stack.pop() {
            popped.push(val);
        }

        assert_eq!(popped.len(), NUM_THREADS * OPS_PER_THREAD);
        let set: HashSet<usize> = popped.iter().cloned().collect();
        assert_eq!(set.len(), popped.len(), "No duplicates allowed");
    }

    #[test]
    fn test_concurrent_mixed_push_pop() {
        const NUM_THREADS: usize = 8;
        const OPS_PER_THREAD: usize = 500;

        let stack = Arc::new(TreiberStack::new());
        let push_count = Arc::new(AtomicUsize::new(0));
        let pop_count = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let stack = Arc::clone(&stack);
                let push_count = Arc::clone(&push_count);
                let pop_count = Arc::clone(&pop_count);
                s.spawn(move || {
                    for i in 0..OPS_PER_THREAD {
                        if (t + i) % 2 == 0 {
                            stack.push(t * OPS_PER_THREAD + i);
                            push_count.fetch_add(1, Ordering::Relaxed);
                        } else if stack.pop().is_some() {
                            pop_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        let mut remaining = 0usize;
        while stack.pop().is_some() {
            remaining += 1;
        }

        let total_pushed = push_count.load(Ordering::Relaxed);
        let total_popped = pop_count.load(Ordering::Relaxed) + remaining;
        assert_eq!(total_pushed, total_popped, "pushed = popped invariant");
    }
}

/// Property-based tests that scale with compute budget.
/// Set VF_PROPTEST_CASES env var to control number of cases (default 100).
/// Disabled under Miri (too slow for interpretation).
#[cfg(all(test, not(miri)))]
mod prop_tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;

    fn proptest_cases() -> u32 {
        std::env::var("VF_PROPTEST_CASES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(100)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        #[test]
        fn prop_push_pop_conservation(
            values in prop::collection::vec(0u64..10000, 1..200)
        ) {
            let stack = TreiberStack::new();
            for &v in &values {
                stack.push(v);
            }
            let mut popped = Vec::new();
            while let Some(v) = stack.pop() {
                popped.push(v);
            }
            prop_assert_eq!(values.len(), popped.len(), "Element count must match");
        }

        #[test]
        fn prop_lifo_ordering(
            values in prop::collection::vec(0u64..10000, 1..100)
        ) {
            let stack = TreiberStack::new();
            for &v in &values {
                stack.push(v);
            }
            let mut popped = Vec::new();
            while let Some(v) = stack.pop() {
                popped.push(v);
            }
            let expected: Vec<u64> = values.into_iter().rev().collect();
            prop_assert_eq!(popped, expected, "Must be LIFO order");
        }

        #[test]
        fn prop_no_duplicates(
            values in prop::collection::vec(0u64..100000, 1..500)
        ) {
            let stack = TreiberStack::new();
            // Use unique values
            let unique: Vec<u64> = values.into_iter().collect::<HashSet<_>>().into_iter().collect();
            for &v in &unique {
                stack.push(v);
            }
            let mut popped = HashSet::new();
            while let Some(v) = stack.pop() {
                prop_assert!(popped.insert(v), "Duplicate detected: {}", v);
            }
            prop_assert_eq!(popped.len(), unique.len());
        }

        #[test]
        fn prop_interleaved_ops(
            ops in prop::collection::vec(prop::bool::ANY, 1..200),
            seed_val in 0u64..10000
        ) {
            let stack = TreiberStack::new();
            let mut pushed = 0u64;
            let mut popped_count = 0u64;
            let mut val = seed_val;

            for &is_push in &ops {
                if is_push {
                    stack.push(val);
                    pushed += 1;
                    val += 1;
                } else if stack.pop().is_some() {
                    popped_count += 1;
                }
            }

            // Drain remaining
            while stack.pop().is_some() {
                popped_count += 1;
            }

            prop_assert_eq!(pushed, popped_count, "pushed must equal total popped");
        }
    }
}
