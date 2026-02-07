# LLM KV-Cache Prefix Matching for SQL Queries: Problem Statement

## What It Does

An LLM SQL cache manages a fixed-memory pool of precomputed key-value attention states (the "KV-cache") for a large language model that processes SQL queries. When the model receives a new SQL query, it must compute KV states for every token in the query's prompt. If a previously processed query shares a prefix with the new query -- that is, the first N tokens are identical -- the cached KV states for those N tokens can be reused, saving the expensive forward pass for the shared portion.

The cache decides which prefixes to keep in memory and which to evict when the memory budget is exceeded. Incoming queries arrive in a stream with a skewed popularity distribution: some query patterns (the same SELECT structure with different literal values) appear frequently, while others are one-off. The cache must identify that "SELECT count(*) FROM logs WHERE service='nginx'" and "SELECT count(*) FROM logs WHERE service='redis'" share a common prefix up to the point where the literal value diverges, and that caching the KV states for that common prefix benefits both queries. The quality of the caching strategy is measured by the fraction of incoming queries that find a usable prefix in the cache (the hit rate).

## Safety Properties

1. **Memory budget**: The total memory consumed by cached KV states never exceeds the configured budget. Each cached prefix consumes memory proportional to its token count.
2. **Prefix validity**: A cached entry is used as a prefix for a new query only if the cached token sequence is a true prefix of the new query's token sequence. Partial or approximate matches are not used.
3. **Eviction consistency**: When a cache entry is evicted, it is fully removed from all index structures. No lookup ever returns a reference to an evicted or partially deallocated entry.
4. **No double eviction**: Each cache entry is evicted at most once. An entry that has already been evicted is not evicted again or freed a second time.
5. **Cache coherence**: If an entry is inserted and not evicted, a subsequent lookup for the same prefix finds it. Insertions do not silently fail or lose entries.

## Liveness Properties

1. **Termination of lookups**: Every cache lookup completes in bounded time, regardless of the cache size or the number of stored prefixes.
2. **Eviction progress**: When the cache is full and a new entry must be inserted, the eviction policy frees sufficient memory in bounded time. The cache does not deadlock or fail to make room for new entries.
3. **Hit rate convergence**: As the stream of queries continues and the popularity distribution is stationary, the hit rate converges to a stable value. The cache does not oscillate between drastically different states.
4. **Adaptation to distribution shift**: If the query popularity distribution changes, the cache adapts within a bounded number of queries, evicting stale prefixes and caching newly popular ones.

## Performance Dimensions

- Cache hit rate: fraction of incoming queries that find a usable prefix in the cache, normalized against an oracle that knows the future query stream.
- Memory utilization: fraction of the memory budget actually occupied by useful cached prefixes (vs. fragmentation or metadata overhead).
- Prefix sharing efficiency: average number of distinct queries served by each cached prefix entry.
- Lookup latency: time to determine whether a usable prefix exists for an incoming query.
- Eviction quality: cumulative cost of cache misses that would have been hits under an optimal eviction policy (regret).
- Sensitivity to Zipf parameter: hit rate variation as the query popularity skew ranges from alpha=0.8 (mild) to alpha=1.2 (severe).
- Sensitivity to prefix token length: hit rate variation as the average prefix length ranges from 128 to 2048 tokens.

## What Is NOT Specified

- The eviction policy (LRU, LFU, ARC, LIRS, Belady's optimal approximation, learned eviction).
- The prefix identification method (token-level trie, hash-based, normalized SQL AST, n-gram fingerprinting).
- The normalization strategy for SQL queries (literal replacement, whitespace normalization, AST canonicalization).
- Whether the cache supports hierarchical prefixes (caching "SELECT count(*) FROM logs" and separately "SELECT count(*) FROM logs WHERE ...").
- The memory accounting model (fixed per-token, variable per-layer, compressed KV states).
- Whether the cache is shared across model replicas or is per-replica.
- The concurrency model for concurrent lookups and insertions (single-threaded, sharded, lock-free).
