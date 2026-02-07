# Ring Buffer: Problem Statement

## What It Does

A bounded ring buffer is a fixed-capacity first-in, first-out (FIFO) channel shared among multiple producers and multiple consumers. Producers append values to the tail of the buffer; consumers remove values from the head. The buffer has a capacity fixed at construction time, and a push into a full buffer fails immediately rather than blocking or overwriting.

The buffer must support concurrent access from any number of producer and consumer threads simultaneously. Internally, the head and tail positions advance around a fixed-size array, wrapping back to the beginning after reaching the end. The critical invariant is that a value pushed before another value is always popped before it (FIFO), and the number of elements in the buffer never exceeds the stated capacity.

## Safety Properties

1. **FIFO ordering**: If producer P1 pushes value A and later producer P1 pushes value B (with the push of A completing before the push of B begins), then A is popped before B.
2. **Bounded capacity**: At no point does the number of elements logically present in the buffer exceed the capacity specified at construction. A push to a full buffer returns false and has no effect.
3. **No lost values**: Every value that is successfully pushed is eventually available to exactly one pop. The total count of successful pops equals the total count of successful pushes once the buffer is drained.
4. **No duplicate delivery**: A value pushed once is returned by pop at most once. No two consumers receive the same value.
5. **No torn reads**: A popped value is identical, byte for byte, to the value that was pushed. Concurrent writes to adjacent slots do not corrupt a value being read.
6. **Consistent length**: The value returned by `len()` is always between zero and capacity, inclusive, and reflects a linearizable snapshot of the buffer occupancy.

## Liveness Properties

1. **Lock-freedom (or wait-freedom)**: If threads continue to take steps, at least one push or pop operation completes in a finite number of steps, regardless of the scheduling or failure of other threads.
2. **Immediate failure on full/empty**: A push to a full buffer and a pop from an empty buffer return immediately without spinning indefinitely.
3. **Wraparound progress**: After the buffer fills and drains, subsequent push/pop cycles proceed without deadlock or degraded performance.

## Performance Dimensions

- Single-producer/single-consumer throughput (messages per second) for varying message sizes.
- Multi-producer/single-consumer throughput at 2, 4, and 8 producers.
- Multi-producer/multi-consumer throughput at 4 producers and 4 consumers.
- Latency percentiles (p50, p99, p99.9) from push to corresponding pop.
- Cache-line false sharing: number of cross-core invalidations when producers and consumers run on different cores.
- Memory footprint: total bytes consumed for a buffer of capacity N holding elements of size S.

## What Is NOT Specified

- Whether the internal storage is a contiguous array, segmented array, or other layout.
- The sequencing mechanism (atomic head/tail indices, sequence numbers per slot, ticket locks).
- Whether capacity is rounded to a power of two for fast modular arithmetic.
- Backoff or yield strategy when a slot is transiently unavailable.
- Whether the buffer supports blocking push/pop variants in addition to try-push/try-pop.
- The alignment or padding strategy for avoiding false sharing.
