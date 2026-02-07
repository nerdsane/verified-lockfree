# Epoch-Based Garbage Collection: Problem Statement

## What It Does

An epoch-based garbage collector manages the safe reclamation of memory in concurrent data structures where multiple threads may hold references to shared nodes. When a thread removes a node from a data structure, it cannot immediately free that node because other threads may still be reading it. Instead, the thread retires the node to the garbage collector, which defers its destruction until it is provably safe -- that is, until no thread could possibly still hold a reference to it.

The mechanism works through a global epoch counter and per-thread participation. A thread that wishes to access shared data first pins itself (enters a critical section) by acquiring a guard. While pinned, the thread may read shared pointers and defer cleanup functions. When the thread drops its guard, it unpins itself, signaling that it no longer holds references from that access. The collector advances the global epoch and reclaims retired objects only when all threads have moved past the epoch in which those objects were retired.

## Safety Properties

1. **No premature reclamation**: A retired pointer is not freed while any thread holds a guard that was acquired before or during the epoch in which the pointer was retired. As long as any guard is active, objects retired during or before that guard's epoch remain allocated.
2. **No use-after-free**: No thread dereferences a pointer to memory that has already been freed by the collector.
3. **No double free**: Every retired pointer is freed exactly once. Multiple retirements of the same pointer, or retirement combined with manual deallocation, are considered programming errors outside the collector's contract; the collector itself never frees the same block twice.
4. **Deferred function integrity**: Every function passed to `defer` is eventually called exactly once, with the same captured state it was given. No deferred function is lost or called with corrupted closures.
5. **Drop ordering**: Retired objects are dropped (destructors run) only after all guards from the retiring epoch have been released. The collector respects the ownership semantics of Rust's `Drop` trait.
6. **Thread safety of internal state**: The epoch counter, per-thread epoch records, and retirement lists are free from data races under arbitrary concurrent pin, retire, defer, and collect operations.

## Liveness Properties

1. **Guaranteed reclamation**: If all threads eventually unpin (drop their guards), then every retired pointer and every deferred function is eventually reclaimed or executed after a finite number of collect calls.
2. **Collect progress**: A call to `collect` makes progress toward reclamation whenever at least one epoch's worth of retired objects exists and all threads have advanced past that epoch.
3. **Pin/unpin non-blocking**: Pinning (acquiring a guard) and unpinning (dropping a guard) complete in bounded time without waiting on other threads.
4. **No starvation of collection**: A thread that repeatedly calls collect does not starve other threads' ability to pin, and pinned threads do not indefinitely prevent collection as long as they eventually unpin.

## Performance Dimensions

- Pin/unpin latency (nanoseconds per guard acquisition and release).
- Amortized reclamation throughput (objects reclaimed per collect call).
- Peak unreclaimed garbage: maximum number of retired objects that remain unfreed at any point during execution.
- Memory overhead per retired object (bookkeeping bytes beyond the object itself).
- Scalability: reclamation throughput as the number of concurrent threads increases from 1 to 64.
- Epoch advancement frequency: how many collect calls are needed on average to advance the global epoch by one.

## What Is NOT Specified

- The representation of the global epoch (two-bit, three-bit, full counter).
- How per-thread state is stored (thread-local, registered list, hash map by thread ID).
- Whether collect is called explicitly or triggered automatically (e.g., after N retirements).
- The batching strategy for retired objects (per-thread lists, per-epoch bags, centralized queue).
- Whether the implementation supports nested pinning (re-entrant guards).
- The interaction with custom allocators or memory pools.
- Whether the scheme is strictly epoch-based or a hybrid with hazard pointers or quiescent-state tracking.
