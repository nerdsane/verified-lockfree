# Concurrent Linked List: Problem Statement

## What It Does

A concurrent sorted linked list is an ordered set of unique elements shared among multiple threads. Any thread may insert a new element, remove an existing element, or test whether an element is present, all without acquiring a global lock. The list maintains its elements in sorted order at all times: a traversal from the head to the tail visits elements in strictly ascending order.

The list functions as a concurrent set abstraction. Inserting a value already present has no effect and reports failure. Removing a value not present has no effect and reports failure. The contains operation answers whether a given value is logically in the set. The len operation reports the current number of elements. The critical challenge is that insert and remove must splice and unsplice nodes from the linked structure while other threads are concurrently traversing or modifying adjacent nodes.

## Safety Properties

1. **Sorted order invariant**: At every observable instant, the sequence of values reachable from the head is strictly ascending. No insert or remove operation violates this ordering, even transiently.
2. **Set uniqueness**: The list never contains two nodes with the same value. An insert of a duplicate returns false and does not create a second node.
3. **No lost inserts**: If an insert of value V returns true, then V is present in the list until a subsequent successful remove of V.
4. **No phantom removes**: A remove of value V returns true at most once per corresponding insert. If two threads attempt to remove the same value concurrently, exactly one succeeds.
5. **Linearizability**: Every insert, remove, and contains operation appears to take effect atomically at some point between its invocation and response. The resulting sequential history is that of a valid sorted set.
6. **No use-after-free**: A node that has been logically and physically removed is not dereferenced by any concurrent traversal or modification.
7. **Memory safety under concurrent modification**: A traversal that races with an insert or remove at an adjacent position does not follow a dangling or corrupted pointer.

## Liveness Properties

1. **Lock-freedom**: If threads continue to take steps, at least one insert, remove, or contains operation completes in a finite number of steps, regardless of the scheduling of other threads.
2. **Traversal progress**: A contains query that begins while the list is non-empty eventually terminates, even if concurrent modifications are occurring.
3. **No starvation of removes**: A remove that targets a value known to be present eventually succeeds or observes that another thread has already removed it.

## Performance Dimensions

- Throughput (operations per second) for mixed insert/remove/contains workloads at 1, 4, 8, and 16 threads.
- Scaling ratio: throughput at N threads divided by throughput at 1 thread for read-heavy (90% contains) and write-heavy (50% insert, 50% remove) workloads.
- Average traversal length per operation (nodes visited before the operation completes or restarts).
- CAS retry rate per successful insert or remove.
- Memory overhead per node (bytes beyond the stored value).
- Memory reclamation latency: time from logical removal to physical deallocation.

## What Is NOT Specified

- The marking/flagging scheme for logical deletion (Harris-style mark bits, lazy linking, etc.).
- The memory reclamation strategy (epoch-based, hazard pointers, reference counting).
- Whether the list uses sentinel head and tail nodes.
- The CAS width (single-word, double-word, or descriptor-based).
- Whether helping mechanisms are employed to assist stalled operations.
- The concrete type of the stored elements beyond requiring `Ord`.
