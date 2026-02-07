# Concurrent B+Tree: Problem Statement

## What It Does

A concurrent B+Tree is an ordered key-value dictionary shared among multiple threads. It maps unsigned 64-bit integer keys to unsigned 64-bit integer values. Any thread may insert a key-value pair, look up the value for a key, remove a key, or perform a range scan that returns all key-value pairs whose keys fall within a specified interval, sorted in ascending key order.

The B+Tree maintains keys in sorted order across a balanced tree of internal nodes (which store only keys and child pointers) and leaf nodes (which store key-value pairs and are linked for efficient range scans). Insertions may cause leaf nodes to split, and removals may cause nodes to merge or rebalance. The critical challenge is that these structural modifications -- which rewrite parent pointers, redistribute keys, and allocate or deallocate nodes -- must proceed safely while other threads are concurrently reading, inserting, removing, or scanning overlapping portions of the tree.

## Safety Properties

1. **Key-value integrity**: If an insert of (K, V) completes, a subsequent get of K returns V (or a value from a later insert that overwrites K). No get ever returns a value that was never associated with the queried key.
2. **Sorted order invariant**: At every linearization point, the set of key-value pairs in the tree is totally ordered by key. A range scan returns pairs in strictly ascending key order.
3. **Range scan consistency**: A range scan over [start, end) returns a set of key-value pairs such that every returned pair was present in the tree at some point during the scan, and the returned pairs are sorted.
4. **Atomic overwrite**: Inserting a new value for an existing key replaces the old value atomically. No get observes a partially updated value.
5. **Remove correctness**: A successful remove of key K ensures that subsequent gets of K return None until K is re-inserted. A remove of a non-existent key returns false.
6. **Structural integrity under splits and merges**: Node splits caused by inserts and node merges caused by removes do not leave the tree in a state where any key is unreachable, duplicated, or associated with the wrong value.
7. **Linearizability of point operations**: Every insert, get, and remove appears to take effect at a single atomic instant between invocation and response.

## Liveness Properties

1. **Lock-freedom or deadlock-freedom**: Concurrent operations on the tree make progress. If locks are used, the locking protocol is deadlock-free (e.g., always acquires locks in a top-down or left-to-right order).
2. **Range scan termination**: A range scan completes in finite time, even if concurrent inserts and removes are modifying keys within the scanned range.
3. **Split and merge completion**: Node splits and merges triggered by inserts or removes complete in a finite number of steps and do not cause subsequent operations to loop indefinitely.
4. **Read-write concurrency**: Readers (get, range) are not starved by a continuous stream of writers, and vice versa.

## Performance Dimensions

- Point lookup throughput (gets per second) at 1, 4, 8, 16, and 64 threads.
- Insert throughput at 1, 4, 8, 16, and 64 threads with uniformly random keys.
- Range scan throughput: key-value pairs returned per second for scans of 10, 100, and 1000 keys.
- Mixed workload throughput (80% get, 10% insert, 5% remove, 5% range) scaling from 1 to 32 threads.
- Tree height as a function of the number of stored keys (measures balance).
- Node utilization (average fraction of slots occupied per node).
- Lock hold time or CAS retry rate per operation.

## What Is NOT Specified

- The branching factor (node size / fanout) of internal and leaf nodes.
- The concurrency control protocol (optimistic lock coupling, B-link trees, lock-free node operations, epoch-based protection of nodes).
- Whether leaf nodes are linked in a doubly or singly linked list, or not linked at all.
- The node layout (sorted array, unsorted with periodic compaction, slot array with indirection).
- The rebalancing policy on remove (eager merge, lazy, or no rebalancing).
- Memory allocation strategy for nodes (arena, per-node allocation, slab).
- Whether range scans provide snapshot isolation or observe concurrent modifications.
