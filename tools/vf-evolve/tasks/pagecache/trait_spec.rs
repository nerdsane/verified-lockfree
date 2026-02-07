/// Trait specification tests for PageCache
///
/// The evolved code must provide:
///   pub struct PageCache { ... }
///   impl PageCache {
///       pub fn new(page_size: usize) -> Self;
///       pub fn read(&self, page_id: u64) -> Option<Vec<u8>>;
///       pub fn write(&self, page_id: u64, data: Vec<u8>);
///       pub fn flush(&self);
///       pub fn dirty_pages(&self) -> usize;
///   }
///
/// Concurrent page cache. All operations safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_new_cache_is_empty() {
        let cache = PageCache::new(4096);
        assert_eq!(cache.read(0), None);
        assert_eq!(cache.read(100), None);
        assert_eq!(cache.dirty_pages(), 0);
    }

    #[test]
    fn test_write_and_read() {
        let cache = PageCache::new(4096);
        let data = vec![0xABu8; 4096];
        cache.write(1, data.clone());

        let read_back = cache.read(1);
        assert_eq!(read_back, Some(data));
    }

    #[test]
    fn test_overwrite_page() {
        let cache = PageCache::new(64);
        cache.write(5, vec![1u8; 64]);
        cache.write(5, vec![2u8; 64]);

        let data = cache.read(5).unwrap();
        assert_eq!(data, vec![2u8; 64], "Overwrite must replace data");
    }

    #[test]
    fn test_dirty_pages_tracking() {
        let cache = PageCache::new(64);
        assert_eq!(cache.dirty_pages(), 0);

        cache.write(1, vec![0u8; 64]);
        assert!(cache.dirty_pages() >= 1);

        cache.write(2, vec![0u8; 64]);
        assert!(cache.dirty_pages() >= 2);

        cache.flush();
        assert_eq!(
            cache.dirty_pages(),
            0,
            "Flush must clear dirty page count"
        );
    }

    #[test]
    fn test_flush_preserves_data() {
        let cache = PageCache::new(64);
        cache.write(1, vec![0xFFu8; 64]);
        cache.flush();

        // Data should still be readable after flush
        let data = cache.read(1);
        assert_eq!(data, Some(vec![0xFFu8; 64]));
    }

    #[test]
    fn test_multiple_pages() {
        let cache = PageCache::new(32);
        for i in 0..10u64 {
            cache.write(i, vec![i as u8; 32]);
        }

        for i in 0..10u64 {
            let data = cache.read(i).unwrap();
            assert_eq!(data, vec![i as u8; 32], "Page {} has wrong data", i);
        }
    }

    #[test]
    fn test_concurrent_reads_and_writes() {
        const NUM_PAGES: u64 = 50;
        const PAGE_SIZE: usize = 128;
        const NUM_THREADS: usize = 8;

        let cache = Arc::new(PageCache::new(PAGE_SIZE));

        // Pre-populate pages
        for i in 0..NUM_PAGES {
            cache.write(i, vec![i as u8; PAGE_SIZE]);
        }

        std::thread::scope(|s| {
            // Reader threads
            for _ in 0..NUM_THREADS {
                let cache = Arc::clone(&cache);
                s.spawn(move || {
                    for i in 0..NUM_PAGES {
                        if let Some(data) = cache.read(i) {
                            // Data should be consistent: all bytes same value
                            let first = data[0];
                            assert!(
                                data.iter().all(|&b| b == first),
                                "Page {} has torn read: mixed byte values",
                                i
                            );
                        }
                    }
                });
            }

            // Writer threads
            for t in 0..NUM_THREADS {
                let cache = Arc::clone(&cache);
                s.spawn(move || {
                    let byte_val = (NUM_PAGES as u8) + (t as u8);
                    for i in 0..NUM_PAGES {
                        cache.write(i, vec![byte_val; PAGE_SIZE]);
                    }
                });
            }
        });

        // After all threads, every page should have consistent data
        for i in 0..NUM_PAGES {
            let data = cache.read(i).unwrap();
            let first = data[0];
            assert!(
                data.iter().all(|&b| b == first),
                "Page {} has inconsistent data after concurrent access",
                i
            );
        }
    }

    #[test]
    fn test_concurrent_flush() {
        const PAGE_SIZE: usize = 64;
        let cache = Arc::new(PageCache::new(PAGE_SIZE));

        std::thread::scope(|s| {
            // Writers
            for t in 0..4 {
                let cache = Arc::clone(&cache);
                s.spawn(move || {
                    for i in 0..100u64 {
                        cache.write(t * 100 + i, vec![(t as u8); PAGE_SIZE]);
                    }
                });
            }

            // Concurrent flushers
            for _ in 0..4 {
                let cache = Arc::clone(&cache);
                s.spawn(move || {
                    for _ in 0..10 {
                        cache.flush();
                        std::thread::yield_now();
                    }
                });
            }
        });

        // Final flush
        cache.flush();
        assert_eq!(cache.dirty_pages(), 0);
    }
}
