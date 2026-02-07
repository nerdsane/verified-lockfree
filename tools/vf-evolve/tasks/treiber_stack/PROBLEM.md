# Treiber Stack: Problem Statement

## What It Does

A Treiber stack is a concurrent last-in, first-out (LIFO) container shared among multiple threads. Any thread may push a value onto the top of the stack at any time, and any thread may pop the most recently pushed value from the top. The stack begins empty, and a pop on an empty stack returns nothing.

The defining characteristic is that the stack must function correctly without mutual exclusion locks. Threads contend on a single shared top-of-stack pointer, and the implementation must resolve that contention using only atomic compare-and-swap operations (or their equivalent). The result is a data structure whose throughput scales with the number of hardware threads rather than degrading under contention as lock-based stacks do.

## Safety Properties

1. **LIFO ordering**: If a push of value A completes before a push of value B begins, and no intervening pop removes A, then B is popped before A.
2. **No lost values**: Every value that is successfully pushed and not yet popped must eventually be observable by some pop. The total number of values popped equals the total number of values pushed once the stack is drained.
3. **No duplicate values**: A value pushed exactly once is popped at most once. No pop operation ever returns a value that was already returned by a prior pop.
4. **Linearizability**: Every push and pop appears to take effect at a single atomic instant between its invocation and response. The resulting sequential history is a valid stack history.
5. **No use-after-free**: Memory backing a popped node is not accessed by any concurrent operation after the node is logically removed from the stack.
6. **ABA safety**: A compare-and-swap on the top pointer must not succeed spuriously because a node was removed, freed, and a new node was allocated at the same address.

## Liveness Properties

1. **Lock-freedom**: If threads continue to take steps, at least one push or pop operation completes in a finite number of steps, regardless of the scheduling of other threads. No thread can block all others indefinitely.
2. **Eventual completion under fair scheduling**: Under a fair scheduler, every invoked push or pop eventually returns.
3. **Empty-stack progress**: A pop on an empty stack returns immediately rather than spinning or blocking.

## Performance Dimensions

- Throughput (operations per second) under uniform random push/pop workloads at 1, 4, 8, 16, and 64 threads.
- Contention scaling: ratio of throughput at N threads to throughput at 1 thread.
- CAS retry rate: average number of compare-and-swap failures per successful operation.
- Memory overhead per element (bytes beyond the stored value).
- Cache-line contention: number of cross-core invalidations per operation.

## What Is NOT Specified

- The internal node representation (intrusive list, array-based, hybrid).
- The memory reclamation strategy (epoch-based, hazard pointers, reference counting, leaking).
- Whether the implementation uses tagged pointers, double-wide CAS, or pointer packing to solve ABA.
- Backoff strategy under contention (exponential, randomized, none).
- Whether `is_empty` is a linearizable snapshot or a best-effort approximation.
- The allocator used for stack nodes.
