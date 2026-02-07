/// Trait specification tests for RadixTree
///
/// The evolved code must provide:
///   pub struct RadixTree { ... }
///   impl RadixTree {
///       pub fn new() -> Self;
///       pub fn insert(&self, key: &[u8], value: u64);
///       pub fn get(&self, key: &[u8]) -> Option<u64>;
///       pub fn remove(&self, key: &[u8]) -> bool;
///       pub fn len(&self) -> usize;
///   }
///
/// Concurrent radix/prefix tree. All operations safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_new_tree_is_empty() {
        let tree = RadixTree::new();
        assert_eq!(tree.len(), 0);
        assert_eq!(tree.get(b"hello"), None);
    }

    #[test]
    fn test_insert_and_get() {
        let tree = RadixTree::new();
        tree.insert(b"hello", 1);
        tree.insert(b"world", 2);
        assert_eq!(tree.get(b"hello"), Some(1));
        assert_eq!(tree.get(b"world"), Some(2));
        assert_eq!(tree.get(b"hell"), None);
        assert_eq!(tree.len(), 2);
    }

    #[test]
    fn test_overwrite_value() {
        let tree = RadixTree::new();
        tree.insert(b"key", 10);
        assert_eq!(tree.get(b"key"), Some(10));
        tree.insert(b"key", 20);
        assert_eq!(tree.get(b"key"), Some(20));
        assert_eq!(tree.len(), 1);
    }

    #[test]
    fn test_remove() {
        let tree = RadixTree::new();
        tree.insert(b"alpha", 1);
        tree.insert(b"beta", 2);

        assert!(tree.remove(b"alpha"));
        assert_eq!(tree.get(b"alpha"), None);
        assert_eq!(tree.get(b"beta"), Some(2));
        assert_eq!(tree.len(), 1);

        assert!(!tree.remove(b"alpha"), "Double remove returns false");
        assert!(!tree.remove(b"gamma"), "Remove non-existent returns false");
    }

    #[test]
    fn test_prefix_sharing() {
        let tree = RadixTree::new();
        tree.insert(b"test", 1);
        tree.insert(b"testing", 2);
        tree.insert(b"tester", 3);
        tree.insert(b"team", 4);

        assert_eq!(tree.get(b"test"), Some(1));
        assert_eq!(tree.get(b"testing"), Some(2));
        assert_eq!(tree.get(b"tester"), Some(3));
        assert_eq!(tree.get(b"team"), Some(4));
        assert_eq!(tree.len(), 4);

        // Prefix that is not a complete key
        assert_eq!(tree.get(b"te"), None);
        assert_eq!(tree.get(b"tes"), None);
    }

    #[test]
    fn test_empty_key() {
        let tree = RadixTree::new();
        tree.insert(b"", 99);
        assert_eq!(tree.get(b""), Some(99));
        tree.insert(b"a", 1);
        assert_eq!(tree.get(b""), Some(99));
        assert_eq!(tree.get(b"a"), Some(1));
    }

    #[test]
    fn test_concurrent_insert_and_get() {
        const NUM_THREADS: usize = 8;
        const OPS_PER_THREAD: usize = 500;

        let tree = Arc::new(RadixTree::new());

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let tree = Arc::clone(&tree);
                s.spawn(move || {
                    for i in 0..OPS_PER_THREAD {
                        let key = format!("thread{}:key{}", t, i);
                        let val = (t * OPS_PER_THREAD + i) as u64;
                        tree.insert(key.as_bytes(), val);
                    }
                });
            }
        });

        assert_eq!(tree.len(), NUM_THREADS * OPS_PER_THREAD);

        // Verify all values are readable
        for t in 0..NUM_THREADS {
            for i in 0..OPS_PER_THREAD {
                let key = format!("thread{}:key{}", t, i);
                let expected = (t * OPS_PER_THREAD + i) as u64;
                assert_eq!(
                    tree.get(key.as_bytes()),
                    Some(expected),
                    "Missing key {}",
                    key
                );
            }
        }
    }

    #[test]
    fn test_concurrent_insert_and_remove() {
        const NUM_KEYS: usize = 500;

        let tree = Arc::new(RadixTree::new());

        // Pre-populate
        for i in 0..NUM_KEYS {
            let key = format!("remove:{}", i);
            tree.insert(key.as_bytes(), i as u64);
        }

        let removed = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            // Removers
            for _ in 0..4 {
                let tree = Arc::clone(&tree);
                let removed = Arc::clone(&removed);
                s.spawn(move || {
                    for i in 0..NUM_KEYS {
                        let key = format!("remove:{}", i);
                        if tree.remove(key.as_bytes()) {
                            removed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }

            // Concurrent inserters adding different keys
            for t in 0..4 {
                let tree = Arc::clone(&tree);
                s.spawn(move || {
                    for i in 0..NUM_KEYS {
                        let key = format!("insert:{}:{}", t, i);
                        tree.insert(key.as_bytes(), (t * NUM_KEYS + i) as u64);
                    }
                });
            }
        });

        // Each pre-populated key removed exactly once
        assert_eq!(
            removed.load(Ordering::Relaxed),
            NUM_KEYS,
            "Each key removed exactly once"
        );
    }
}
