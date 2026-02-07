/// Trait specification tests for EpochGc
///
/// The evolved code must provide:
///   pub struct EpochGc { ... }
///   pub struct Guard { ... }
///   impl EpochGc {
///       pub fn new() -> Self;
///       pub fn pin(&self) -> Guard;
///       pub fn retire<T>(&self, ptr: *mut T);
///       pub fn collect(&self);
///   }
///   impl Guard {
///       pub fn defer(&self, f: impl FnOnce() + Send + 'static);
///   }
///   impl Drop for Guard { ... }   // unpin on drop
///
/// Epoch-based garbage collection. All operations safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_pin_and_unpin() {
        let gc = EpochGc::new();
        let guard = gc.pin();
        // Guard should be droppable without error
        drop(guard);
    }

    #[test]
    fn test_defer_runs_after_collect() {
        let gc = EpochGc::new();
        let ran = Arc::new(AtomicBool::new(false));

        {
            let guard = gc.pin();
            let ran_clone = Arc::clone(&ran);
            guard.defer(move || {
                ran_clone.store(true, Ordering::SeqCst);
            });
        } // guard dropped here (unpin)

        // Collect should eventually run deferred functions
        gc.collect();
        gc.collect(); // May need multiple collects for epoch advancement
        gc.collect();

        assert!(
            ran.load(Ordering::SeqCst),
            "Deferred function must run after collect"
        );
    }

    #[test]
    fn test_retire_and_collect() {
        let gc = EpochGc::new();
        let dropped = Arc::new(AtomicBool::new(false));

        // Allocate a value on the heap
        let dropped_clone = Arc::clone(&dropped);
        let boxed = Box::new(DropDetector(dropped_clone));
        let ptr = Box::into_raw(boxed);

        {
            let _guard = gc.pin();
            gc.retire(ptr);
        }

        gc.collect();
        gc.collect();
        gc.collect();

        assert!(
            dropped.load(Ordering::SeqCst),
            "Retired pointer must be freed after collect"
        );
    }

    #[test]
    fn test_no_premature_free_while_pinned() {
        let gc = Arc::new(EpochGc::new());
        let freed = Arc::new(AtomicBool::new(false));

        let freed_clone = Arc::clone(&freed);
        let boxed = Box::new(DropDetector(freed_clone));
        let ptr = Box::into_raw(boxed);

        let guard = gc.pin(); // Keep pinned

        gc.retire(ptr);
        gc.collect();

        // While a guard is active, the value must NOT be freed
        assert!(
            !freed.load(Ordering::SeqCst),
            "Must not free while any guard is pinned"
        );

        drop(guard); // Now unpin

        gc.collect();
        gc.collect();
        gc.collect();

        assert!(
            freed.load(Ordering::SeqCst),
            "Must free after all guards unpinned and collect called"
        );
    }

    #[test]
    fn test_concurrent_pin_and_defer() {
        const NUM_THREADS: usize = 8;
        const OPS_PER_THREAD: usize = 100;

        let gc = Arc::new(EpochGc::new());
        let total_deferred = Arc::new(AtomicUsize::new(0));
        let total_ran = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            for _ in 0..NUM_THREADS {
                let gc = Arc::clone(&gc);
                let total_deferred = Arc::clone(&total_deferred);
                let total_ran = Arc::clone(&total_ran);
                s.spawn(move || {
                    for _ in 0..OPS_PER_THREAD {
                        let guard = gc.pin();
                        let total_ran = Arc::clone(&total_ran);
                        guard.defer(move || {
                            total_ran.fetch_add(1, Ordering::Relaxed);
                        });
                        total_deferred.fetch_add(1, Ordering::Relaxed);
                        drop(guard);
                    }
                });
            }
        });

        // Collect multiple times to flush all epochs
        for _ in 0..10 {
            gc.collect();
        }

        let deferred = total_deferred.load(Ordering::Relaxed);
        let ran = total_ran.load(Ordering::Relaxed);
        assert_eq!(
            deferred, ran,
            "All deferred functions must eventually run: deferred={}, ran={}",
            deferred, ran
        );
    }

    #[test]
    fn test_concurrent_retire() {
        const NUM_THREADS: usize = 8;
        const RETIRES_PER_THREAD: usize = 100;

        let gc = Arc::new(EpochGc::new());
        let drop_count = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            for _ in 0..NUM_THREADS {
                let gc = Arc::clone(&gc);
                let drop_count = Arc::clone(&drop_count);
                s.spawn(move || {
                    for _ in 0..RETIRES_PER_THREAD {
                        let dc = Arc::clone(&drop_count);
                        let boxed = Box::new(DropCounter(dc));
                        let ptr = Box::into_raw(boxed);
                        let _guard = gc.pin();
                        gc.retire(ptr);
                    }
                });
            }
        });

        for _ in 0..20 {
            gc.collect();
        }

        let expected = NUM_THREADS * RETIRES_PER_THREAD;
        assert_eq!(
            drop_count.load(Ordering::Relaxed),
            expected,
            "All retired values must be dropped"
        );
    }

    // Helper types for drop detection

    struct DropDetector(Arc<AtomicBool>);

    impl Drop for DropDetector {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    struct DropCounter(Arc<AtomicUsize>);

    impl Drop for DropCounter {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}
