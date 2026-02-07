/// Trait specification tests for RingBuffer<T>
///
/// The evolved code must provide:
///   pub struct RingBuffer<T> { ... }
///   impl<T> RingBuffer<T> {
///       pub fn new(capacity: usize) -> Self;
///       pub fn push(&self, val: T) -> bool;       // false if full
///       pub fn pop(&self) -> Option<T>;
///       pub fn len(&self) -> usize;
///       pub fn capacity(&self) -> usize;
///       pub fn is_empty(&self) -> bool;
///       pub fn is_full(&self) -> bool;
///   }
///
/// Bounded MPSC/MPMC ring buffer. All operations must be safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_new_buffer_is_empty() {
        let buf: RingBuffer<i32> = RingBuffer::new(16);
        assert!(buf.is_empty());
        assert!(!buf.is_full());
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.capacity(), 16);
        assert_eq!(buf.pop(), None);
    }

    #[test]
    fn test_push_pop_single() {
        let buf = RingBuffer::new(4);
        assert!(buf.push(42));
        assert_eq!(buf.len(), 1);
        assert!(!buf.is_empty());
        assert_eq!(buf.pop(), Some(42));
        assert!(buf.is_empty());
    }

    #[test]
    fn test_fifo_order() {
        let buf = RingBuffer::new(8);
        buf.push(1);
        buf.push(2);
        buf.push(3);
        assert_eq!(buf.pop(), Some(1));
        assert_eq!(buf.pop(), Some(2));
        assert_eq!(buf.pop(), Some(3));
        assert_eq!(buf.pop(), None);
    }

    #[test]
    fn test_bounded_capacity() {
        let buf = RingBuffer::new(3);
        assert!(buf.push(1));
        assert!(buf.push(2));
        assert!(buf.push(3));
        assert!(buf.is_full());
        assert!(!buf.push(4), "push to full buffer must return false");
        assert_eq!(buf.len(), 3);

        // After popping one, can push again
        assert_eq!(buf.pop(), Some(1));
        assert!(!buf.is_full());
        assert!(buf.push(4));
        assert_eq!(buf.pop(), Some(2));
        assert_eq!(buf.pop(), Some(3));
        assert_eq!(buf.pop(), Some(4));
    }

    #[test]
    fn test_wraparound() {
        let buf = RingBuffer::new(4);
        // Fill and drain multiple times to exercise wraparound
        for round in 0..5 {
            let base = round * 4;
            for i in 0..4 {
                assert!(buf.push(base + i));
            }
            assert!(buf.is_full());
            for i in 0..4 {
                assert_eq!(buf.pop(), Some(base + i));
            }
            assert!(buf.is_empty());
        }
    }

    #[test]
    fn test_concurrent_producer_consumer() {
        const CAPACITY: usize = 64;
        const NUM_ITEMS: usize = 10_000;

        let buf = Arc::new(RingBuffer::new(CAPACITY));

        std::thread::scope(|s| {
            let buf_prod = Arc::clone(&buf);
            let producer = s.spawn(move || {
                let mut sent = 0usize;
                while sent < NUM_ITEMS {
                    if buf_prod.push(sent) {
                        sent += 1;
                    }
                    // Spin-retry if full
                }
            });

            let buf_cons = Arc::clone(&buf);
            let consumer = s.spawn(move || {
                let mut received = Vec::with_capacity(NUM_ITEMS);
                while received.len() < NUM_ITEMS {
                    if let Some(val) = buf_cons.pop() {
                        received.push(val);
                    }
                    // Spin-retry if empty
                }
                received
            });

            producer.join().unwrap();
            let received = consumer.join().unwrap();

            // FIFO: values must arrive in order
            for (i, &val) in received.iter().enumerate() {
                assert_eq!(val, i, "FIFO order violated at index {}", i);
            }
        });
    }

    #[test]
    fn test_concurrent_multi_producer() {
        const CAPACITY: usize = 128;
        const NUM_PRODUCERS: usize = 4;
        const ITEMS_PER_PRODUCER: usize = 1000;

        let buf = Arc::new(RingBuffer::new(CAPACITY));
        let total_consumed = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            // Spawn producers
            for t in 0..NUM_PRODUCERS {
                let buf = Arc::clone(&buf);
                s.spawn(move || {
                    for i in 0..ITEMS_PER_PRODUCER {
                        let val = t * ITEMS_PER_PRODUCER + i;
                        while !buf.push(val) {
                            // Spin until slot available
                        }
                    }
                });
            }

            // Single consumer drains all
            let buf = Arc::clone(&buf);
            let total_consumed = Arc::clone(&total_consumed);
            s.spawn(move || {
                let target = NUM_PRODUCERS * ITEMS_PER_PRODUCER;
                while total_consumed.load(Ordering::Relaxed) < target {
                    if buf.pop().is_some() {
                        total_consumed.fetch_add(1, Ordering::Relaxed);
                    }
                }
            });
        });

        assert_eq!(
            total_consumed.load(Ordering::Relaxed),
            NUM_PRODUCERS * ITEMS_PER_PRODUCER,
            "All items must be consumed"
        );
    }
}
