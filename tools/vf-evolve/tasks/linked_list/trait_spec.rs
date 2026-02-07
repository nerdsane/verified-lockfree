/// Trait specification tests for ConcurrentLinkedList<T: Ord>
///
/// The evolved code must provide:
///   pub struct ConcurrentLinkedList<T: Ord> { ... }
///   impl<T: Ord> ConcurrentLinkedList<T> {
///       pub fn new() -> Self;
///       pub fn insert(&self, val: T) -> bool;     // false if already present
///       pub fn remove(&self, val: &T) -> bool;    // false if not found
///       pub fn contains(&self, val: &T) -> bool;
///       pub fn len(&self) -> usize;
///   }
///
/// Sorted, lock-free linked list. All operations safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_new_list_is_empty() {
        let list: ConcurrentLinkedList<i32> = ConcurrentLinkedList::new();
        assert_eq!(list.len(), 0);
        assert!(!list.contains(&0));
    }

    #[test]
    fn test_insert_and_contains() {
        let list = ConcurrentLinkedList::new();
        assert!(list.insert(5));
        assert!(list.insert(3));
        assert!(list.insert(7));
        assert_eq!(list.len(), 3);

        assert!(list.contains(&3));
        assert!(list.contains(&5));
        assert!(list.contains(&7));
        assert!(!list.contains(&4));
    }

    #[test]
    fn test_no_duplicate_insert() {
        let list = ConcurrentLinkedList::new();
        assert!(list.insert(10));
        assert!(!list.insert(10), "Duplicate insert must return false");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_remove() {
        let list = ConcurrentLinkedList::new();
        list.insert(1);
        list.insert(2);
        list.insert(3);

        assert!(list.remove(&2));
        assert!(!list.contains(&2));
        assert_eq!(list.len(), 2);

        assert!(!list.remove(&2), "Double remove must return false");
        assert!(list.contains(&1));
        assert!(list.contains(&3));
    }

    #[test]
    fn test_remove_head_and_tail() {
        let list = ConcurrentLinkedList::new();
        list.insert(10);
        list.insert(20);
        list.insert(30);

        // Remove smallest (head of sorted list)
        assert!(list.remove(&10));
        assert_eq!(list.len(), 2);

        // Remove largest (tail of sorted list)
        assert!(list.remove(&30));
        assert_eq!(list.len(), 1);
        assert!(list.contains(&20));
    }

    #[test]
    fn test_concurrent_insert_no_lost_elements() {
        const NUM_THREADS: usize = 8;
        const OPS_PER_THREAD: usize = 500;

        let list = Arc::new(ConcurrentLinkedList::new());
        let successful_inserts = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let list = Arc::clone(&list);
                let successful_inserts = Arc::clone(&successful_inserts);
                s.spawn(move || {
                    for i in 0..OPS_PER_THREAD {
                        let val = t * OPS_PER_THREAD + i;
                        if list.insert(val as u64) {
                            successful_inserts.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        // All values are unique across threads, so all inserts should succeed
        let expected = NUM_THREADS * OPS_PER_THREAD;
        assert_eq!(
            successful_inserts.load(Ordering::Relaxed),
            expected,
            "All inserts of unique values must succeed"
        );
        assert_eq!(list.len(), expected);

        // Verify all values are present
        for t in 0..NUM_THREADS {
            for i in 0..OPS_PER_THREAD {
                let val = (t * OPS_PER_THREAD + i) as u64;
                assert!(list.contains(&val), "Missing value {}", val);
            }
        }
    }

    #[test]
    fn test_concurrent_insert_and_remove() {
        const NUM_THREADS: usize = 4;
        const OPS_PER_THREAD: usize = 500;

        let list = Arc::new(ConcurrentLinkedList::new());

        // Pre-populate with values that removers will target
        for i in 0..OPS_PER_THREAD {
            list.insert(i as u64);
        }

        let insert_count = Arc::new(AtomicUsize::new(0));
        let remove_count = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            // Inserters: add new values in a different range
            for t in 0..NUM_THREADS {
                let list = Arc::clone(&list);
                let insert_count = Arc::clone(&insert_count);
                s.spawn(move || {
                    let base = OPS_PER_THREAD + t * OPS_PER_THREAD;
                    for i in 0..OPS_PER_THREAD {
                        if list.insert((base + i) as u64) {
                            insert_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }

            // Removers: remove pre-populated values
            for _ in 0..NUM_THREADS {
                let list = Arc::clone(&list);
                let remove_count = Arc::clone(&remove_count);
                s.spawn(move || {
                    for i in 0..OPS_PER_THREAD {
                        if list.remove(&(i as u64)) {
                            remove_count.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        // Each pre-populated value removed exactly once across all remover threads
        assert_eq!(
            remove_count.load(Ordering::Relaxed),
            OPS_PER_THREAD,
            "Each value removed exactly once"
        );

        // Inserted values should all still be present
        let final_len = list.len();
        let inserted = insert_count.load(Ordering::Relaxed);
        assert_eq!(final_len, inserted, "Only inserted values remain");
    }
}
