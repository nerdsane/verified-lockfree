# Concurrent I/O Buffer: Problem Statement

## What It Does

A concurrent I/O buffer is an append-only byte container shared among multiple writer threads. Writers append variable-length byte slices to the buffer, and each append returns the byte offset at which the data was placed. A flush operation atomically extracts all accumulated bytes and resets the buffer to empty. The buffer has a capacity specified at construction time.

The primary use case is batching many small writes from concurrent producers into a single contiguous byte region that can then be flushed to disk or network in one operation. The critical invariant is that no two appends overlap in the output: each append occupies a unique, contiguous range of bytes, and the flushed output contains every byte that was appended since the last flush, with no gaps and no corruption.

## Safety Properties

1. **No overlapping writes**: Every append receives a unique offset, and the byte ranges [offset, offset + len) assigned to different appends are disjoint. No two appends write to the same byte position.
2. **Data integrity**: The bytes stored at an append's assigned offset are exactly the bytes that were passed to that append. No concurrent append corrupts another append's data.
3. **No data loss**: Every byte passed to a successful append appears in the output of the next flush. The total length of flushed data equals the sum of the lengths of all appends since the previous flush.
4. **Atomic offset assignment**: The offset returned by append reflects a committed reservation. Once an offset is returned, the corresponding bytes will be present in the next flush.
5. **Flush atomicity**: A flush returns a single contiguous byte vector containing all appended data. No append that began after the flush started has its bytes included in the flushed output.
6. **Consistent length**: The value returned by `len()` at any point is equal to the total number of bytes appended since the last flush, and is consistent with the offsets previously returned.

## Liveness Properties

1. **Lock-freedom of appends**: If writer threads continue to take steps, at least one append completes in a finite number of steps, regardless of other threads' scheduling.
2. **Flush completeness**: A flush invoked when no concurrent appends are in progress returns immediately with all previously appended data.
3. **Post-flush progress**: After a flush resets the buffer, subsequent appends proceed starting from offset zero without deadlock or lost capacity.

## Performance Dimensions

- Append throughput (bytes per second) at 1, 4, 8, and 16 concurrent writer threads.
- Append latency (nanoseconds) per operation for small (4-byte) and large (4 KB) appends.
- Flush latency as a function of accumulated buffer size.
- Allocation overhead: whether appends require per-call dynamic allocation or operate on pre-allocated memory.
- Contention cost: ratio of throughput at N writers to throughput at 1 writer.
- Memory utilization: ratio of useful bytes to total buffer capacity consumed.

## What Is NOT Specified

- The internal storage layout (single contiguous array, segmented chunks, linked buffers).
- The mechanism for reserving offsets (atomic fetch-and-add, CAS loop, slot-based).
- The behavior when appending would exceed capacity (block, fail, grow, or auto-flush).
- Whether flush blocks concurrent appends or allows them to proceed into a new buffer.
- The alignment of individual appends within the buffer.
- Thread-local batching or buffering strategies to reduce contention.
