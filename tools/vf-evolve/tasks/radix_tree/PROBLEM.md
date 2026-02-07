# Concurrent Radix Tree: Problem Statement

## What It Does

A concurrent radix tree (also called a prefix tree or trie) is a dictionary mapping variable-length byte-string keys to integer values, shared among multiple threads. Any thread may insert a key-value pair, look up the value associated with a key, or remove a key, all without acquiring a global lock. Keys that share a common prefix share the corresponding path from the root of the tree, so storage is proportional to the total distinct key material rather than the number of keys times the maximum key length.

The tree supports arbitrary byte sequences as keys, including the empty key. Inserting a key that already exists overwrites its associated value atomically. Looking up a key that is a prefix of another key (e.g., "test" vs. "testing") returns only if that exact prefix was inserted as a key in its own right; partial-path matches do not produce spurious hits. The critical challenge is that inserts may split existing edges (when a new key diverges mid-edge from an existing key's path), and removes may merge edges, all while concurrent readers are traversing the same paths.

## Safety Properties

1. **Key-value integrity**: If an insert of (K, V) completes, then a subsequent get of K returns V (or a later value if another insert overwrites K). No get ever returns a value that was never associated with the queried key.
2. **No phantom keys**: A get returns Some only if the exact key was previously inserted and has not been removed. Partial prefix matches do not yield results.
3. **Atomic overwrite**: If two threads concurrently insert different values for the same key, the final value is one of the two inserted values, not a corrupted mixture.
4. **Remove correctness**: After a successful remove of key K, a subsequent get of K returns None unless K is re-inserted. A remove of a non-existent key returns false.
5. **No interference across keys**: Inserting or removing one key does not alter the value associated with any other key that was not modified.
6. **Linearizability**: Every insert, get, and remove appears to take effect at a single atomic instant between invocation and response.
7. **Structural safety under edge splitting**: When an insert causes an existing edge to be split (because the new key diverges from an existing key's shared prefix), no concurrent reader observes a state where either key is missing or maps to the wrong value.

## Liveness Properties

1. **Lock-freedom**: If threads continue to take steps, at least one insert, get, or remove completes in a finite number of steps, regardless of the scheduling of other threads.
2. **Read progress**: A get operation completes in time proportional to the length of the queried key, without being delayed by concurrent writes to unrelated keys.
3. **Prefix-path sharing does not cause livelock**: Multiple threads inserting keys with a common prefix do not repeatedly conflict on the shared edge such that none of them make progress.

## Performance Dimensions

- Throughput (operations per second) for mixed insert/get/remove workloads at 1, 4, 8, and 16 threads.
- Lookup latency as a function of key length (bytes).
- Space efficiency: total bytes consumed divided by total distinct key bytes stored.
- Prefix compression ratio: reduction in node count compared to a naive per-byte trie.
- CAS retry rate per successful insert (especially during edge splits).
- Scaling under skewed key distributions (e.g., many keys sharing a common prefix vs. uniformly random keys).

## What Is NOT Specified

- The node representation (array of children, bitmap + sparse array, sorted list, path compression, adaptive sizing a la ART).
- The memory reclamation strategy for removed nodes and split edges.
- Whether path compression (collapsing single-child chains into edges labeled with multi-byte strings) is used.
- The maximum key length, if any.
- The concurrency control mechanism on individual nodes (CAS on pointers, read-write locks per node, optimistic locking with validation).
- Whether the tree supports iteration, range queries, or prefix enumeration beyond point get.
