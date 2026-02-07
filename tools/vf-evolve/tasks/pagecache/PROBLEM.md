# Concurrent Page Cache: Problem Statement

## What It Does

A concurrent page cache is an in-memory buffer pool that holds fixed-size pages identified by numeric page IDs. Multiple threads may simultaneously read pages, write (create or update) pages, and flush dirty pages. A write marks the affected page as dirty; a flush operation clears all dirty flags, logically persisting the data. The cache retains page data across flushes so that subsequent reads continue to return the most recently written content.

The page cache serves as the layer between a storage engine and its backing store. Readers access pages without blocking writers to unrelated pages, and writers to the same page produce a consistent final state (the value written by whichever write took effect last). The dirty-page count provides the storage engine with an accurate measure of how much data needs to be persisted, enabling it to schedule I/O efficiently.

## Safety Properties

1. **Read-after-write consistency**: If a write of data D to page P completes before a read of page P begins, the read returns D (or a value from a later write).
2. **No torn reads**: A read of page P returns a byte vector that was the complete content of exactly one write to P. No read ever returns a mixture of bytes from two different writes.
3. **No torn writes**: A write of page P replaces the entire content atomically. No concurrent read of P observes a partially updated page.
4. **Dirty tracking accuracy**: After a write to page P and before the next flush, dirty_pages() counts P as dirty. After flush() completes with no concurrent writers, dirty_pages() returns zero.
5. **Flush does not destroy data**: After flush, every page that was written remains readable with its most recently written content. Flush affects only the dirty flag, not the stored data.
6. **Page isolation**: A write to page P does not alter the content of any other page Q (where Q differs from P).
7. **Concurrent flush safety**: If multiple threads call flush concurrently while other threads are writing, no page is lost, and the dirty count eventually reaches zero once all writes and flushes complete.

## Liveness Properties

1. **Read availability**: A read of any page completes in bounded time, without being delayed indefinitely by writers to the same or different pages.
2. **Write availability**: A write to any page completes in bounded time, without being delayed indefinitely by readers or other writers to different pages.
3. **Flush termination**: A flush operation completes in time proportional to the number of dirty pages, and does not deadlock with concurrent reads or writes.
4. **Dirty count convergence**: If no new writes occur, repeated flushes eventually reduce the dirty page count to zero.

## Performance Dimensions

- Read throughput (reads per second) under concurrent read-only workload at 1, 8, and 32 threads.
- Write throughput (writes per second) under concurrent write-only workload at 1, 8, and 32 threads.
- Mixed read-write throughput at various read/write ratios (90/10, 50/50, 10/90).
- Flush latency as a function of the number of dirty pages.
- Per-page memory overhead beyond the stored data itself (metadata, locks, dirty bits).
- Scalability of dirty_pages() reporting: whether it requires scanning all pages or is maintained as a constant-time counter.

## What Is NOT Specified

- The maximum number of pages the cache can hold (bounded vs. unbounded).
- The eviction policy when memory is exhausted (LRU, CLOCK, FIFO, none).
- The internal concurrency mechanism (sharded locks, lock-free hash map, reader-writer locks per page, lock-free per-slot atomics).
- Whether page_size is enforced on write (rejecting data of the wrong length) or advisory.
- Whether the cache supports memory-mapped I/O or purely in-memory buffering.
- The actual persistence mechanism invoked by flush (the specification treats flush as resetting dirty flags only).
