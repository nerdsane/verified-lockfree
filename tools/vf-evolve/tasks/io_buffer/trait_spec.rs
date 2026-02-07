/// Trait specification tests for IoBuffer
///
/// The evolved code must provide:
///   pub struct IoBuffer { ... }
///   impl IoBuffer {
///       pub fn new(capacity: usize) -> Self;
///       pub fn append(&self, data: &[u8]) -> usize;  // returns offset of written data
///       pub fn flush(&self) -> Vec<u8>;               // returns all data and resets
///       pub fn len(&self) -> usize;                   // current bytes written
///   }
///
/// Concurrent append-only buffer. All operations safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::sync::Arc;

    #[test]
    fn test_new_buffer_is_empty() {
        let buf = IoBuffer::new(1024);
        assert_eq!(buf.len(), 0);
        let flushed = buf.flush();
        assert!(flushed.is_empty());
    }

    #[test]
    fn test_append_single() {
        let buf = IoBuffer::new(1024);
        let offset = buf.append(b"hello");
        assert_eq!(offset, 0);
        assert_eq!(buf.len(), 5);
    }

    #[test]
    fn test_append_preserves_order() {
        let buf = IoBuffer::new(1024);
        let off1 = buf.append(b"aaa");
        let off2 = buf.append(b"bbb");
        let off3 = buf.append(b"ccc");

        assert_eq!(off1, 0);
        assert_eq!(off2, 3);
        assert_eq!(off3, 6);
        assert_eq!(buf.len(), 9);

        let data = buf.flush();
        assert_eq!(&data, b"aaabbbccc");
    }

    #[test]
    fn test_flush_resets_buffer() {
        let buf = IoBuffer::new(1024);
        buf.append(b"data1");
        let data = buf.flush();
        assert_eq!(&data, b"data1");
        assert_eq!(buf.len(), 0);

        buf.append(b"data2");
        let data = buf.flush();
        assert_eq!(&data, b"data2");
    }

    #[test]
    fn test_bounded_capacity() {
        let buf = IoBuffer::new(8);
        let off1 = buf.append(b"1234");
        assert_eq!(off1, 0);
        let off2 = buf.append(b"5678");
        assert_eq!(off2, 4);
        assert_eq!(buf.len(), 8);

        // Buffer is full; append should still return a valid offset
        // but the capacity behavior is implementation-defined.
        // At minimum, the first 8 bytes must be intact.
        let data = buf.flush();
        assert!(data.len() >= 8);
        assert_eq!(&data[0..8], b"12345678");
    }

    #[test]
    fn test_concurrent_appends_no_data_loss() {
        const NUM_THREADS: usize = 8;
        const APPENDS_PER_THREAD: usize = 200;
        const CHUNK_SIZE: usize = 4;

        let buf = Arc::new(IoBuffer::new(NUM_THREADS * APPENDS_PER_THREAD * CHUNK_SIZE));

        let mut expected_offsets_per_thread = vec![Vec::new(); NUM_THREADS];

        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for t in 0..NUM_THREADS {
                let buf = Arc::clone(&buf);
                handles.push(s.spawn(move || {
                    let mut offsets = Vec::new();
                    let byte_val = t as u8;
                    let chunk = vec![byte_val; CHUNK_SIZE];
                    for _ in 0..APPENDS_PER_THREAD {
                        let offset = buf.append(&chunk);
                        offsets.push(offset);
                    }
                    offsets
                }));
            }

            for (t, h) in handles.into_iter().enumerate() {
                expected_offsets_per_thread[t] = h.join().unwrap();
            }
        });

        let data = buf.flush();
        let total_expected = NUM_THREADS * APPENDS_PER_THREAD * CHUNK_SIZE;
        assert_eq!(data.len(), total_expected, "No data lost");

        // Verify no offsets overlap: each offset is unique and chunk-aligned
        let mut all_offsets: Vec<usize> = expected_offsets_per_thread
            .iter()
            .flat_map(|v| v.iter().cloned())
            .collect();
        let set: HashSet<usize> = all_offsets.iter().cloned().collect();
        assert_eq!(
            set.len(),
            all_offsets.len(),
            "No overlapping offsets"
        );

        // Verify each chunk has the correct thread's byte value
        for (t, offsets) in expected_offsets_per_thread.iter().enumerate() {
            let byte_val = t as u8;
            for &offset in offsets {
                for j in 0..CHUNK_SIZE {
                    assert_eq!(
                        data[offset + j],
                        byte_val,
                        "Data corruption at offset {} + {}",
                        offset,
                        j
                    );
                }
            }
        }
    }
}
