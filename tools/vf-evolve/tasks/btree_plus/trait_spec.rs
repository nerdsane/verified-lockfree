/// Trait specification tests for ConcurrentBTree
///
/// The evolved code must provide:
///   pub struct ConcurrentBTree { ... }
///   impl ConcurrentBTree {
///       pub fn new() -> Self;
///       pub fn insert(&self, key: u64, value: u64);
///       pub fn get(&self, key: &u64) -> Option<u64>;
///       pub fn remove(&self, key: &u64) -> bool;
///       pub fn range(&self, start: u64, end: u64) -> Vec<(u64, u64)>;
///       pub fn len(&self) -> usize;
///   }
///
/// Concurrent B+Tree with sorted key order. All operations safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_new_tree_is_empty() {
        let tree = ConcurrentBTree::new();
        assert_eq!(tree.len(), 0);
        assert_eq!(tree.get(&0), None);
        assert!(tree.range(0, 100).is_empty());
    }

    #[test]
    fn test_insert_and_get() {
        let tree = ConcurrentBTree::new();
        tree.insert(10, 100);
        tree.insert(20, 200);
        tree.insert(5, 50);

        assert_eq!(tree.get(&10), Some(100));
        assert_eq!(tree.get(&20), Some(200));
        assert_eq!(tree.get(&5), Some(50));
        assert_eq!(tree.get(&15), None);
        assert_eq!(tree.len(), 3);
    }

    #[test]
    fn test_overwrite() {
        let tree = ConcurrentBTree::new();
        tree.insert(1, 10);
        tree.insert(1, 20);
        assert_eq!(tree.get(&1), Some(20));
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn test_remove() {
        let tree = ConcurrentBTree::new();
        tree.insert(1, 10);
        tree.insert(2, 20);
        tree.insert(3, 30);

        assert!(tree.remove(&2));
        assert_eq!(tree.get(&2), None);
        assert_eq!(tree.len(), 2);

        assert!(!tree.remove(&2), "Double remove returns false");
        assert!(!tree.remove(&99), "Remove non-existent returns false");
    }

    #[test]
    fn test_sorted_range_scan() {
        let tree = ConcurrentBTree::new();
        tree.insert(50, 500);
        tree.insert(10, 100);
        tree.insert(30, 300);
        tree.insert(20, 200);
        tree.insert(40, 400);

        let range = tree.range(15, 45);
        assert_eq!(
            range,
            vec![(20, 200), (30, 300), (40, 400)],
            "Range must return sorted entries in [start, end)"
        );

        let range = tree.range(10, 51);
        assert_eq!(range.len(), 5);
        assert_eq!(range[0], (10, 100));
        assert_eq!(range[4], (50, 500));

        let range = tree.range(100, 200);
        assert!(range.is_empty(), "Range with no matches returns empty");
    }

    #[test]
    fn test_range_boundary() {
        let tree = ConcurrentBTree::new();
        tree.insert(10, 1);
        tree.insert(20, 2);
        tree.insert(30, 3);

        // start inclusive, end exclusive
        let range = tree.range(10, 30);
        assert_eq!(range, vec![(10, 1), (20, 2)]);

        let range = tree.range(10, 31);
        assert_eq!(range, vec![(10, 1), (20, 2), (30, 3)]);
    }

    #[test]
    fn test_concurrent_inserts_no_lost_keys() {
        const NUM_THREADS: usize = 8;
        const OPS_PER_THREAD: usize = 1000;

        let tree = Arc::new(ConcurrentBTree::new());

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let tree = Arc::clone(&tree);
                s.spawn(move || {
                    for i in 0..OPS_PER_THREAD {
                        let key = (t * OPS_PER_THREAD + i) as u64;
                        tree.insert(key, key * 10);
                    }
                });
            }
        });

        let expected = NUM_THREADS * OPS_PER_THREAD;
        assert_eq!(tree.len(), expected, "No keys lost during concurrent insert");

        // Verify all keys present
        for t in 0..NUM_THREADS {
            for i in 0..OPS_PER_THREAD {
                let key = (t * OPS_PER_THREAD + i) as u64;
                assert_eq!(
                    tree.get(&key),
                    Some(key * 10),
                    "Missing key {}",
                    key
                );
            }
        }
    }

    #[test]
    fn test_concurrent_range_scan_during_inserts() {
        const INITIAL_KEYS: u64 = 1000;
        let tree = Arc::new(ConcurrentBTree::new());

        // Pre-populate
        for i in 0..INITIAL_KEYS {
            tree.insert(i, i * 10);
        }

        std::thread::scope(|s| {
            // Readers doing range scans
            for _ in 0..4 {
                let tree = Arc::clone(&tree);
                s.spawn(move || {
                    for _ in 0..100 {
                        let range = tree.range(100, 200);
                        // Range must be sorted
                        for w in range.windows(2) {
                            assert!(
                                w[0].0 < w[1].0,
                                "Range scan returned unsorted keys: {} >= {}",
                                w[0].0,
                                w[1].0
                            );
                        }
                    }
                });
            }

            // Writers inserting new keys
            for t in 0..4 {
                let tree = Arc::clone(&tree);
                s.spawn(move || {
                    for i in 0..500 {
                        let key = INITIAL_KEYS + (t as u64) * 500 + (i as u64);
                        tree.insert(key, key * 10);
                    }
                });
            }
        });
    }
}
