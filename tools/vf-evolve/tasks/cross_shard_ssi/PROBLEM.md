# Cross-Shard Serializable Snapshot Isolation: Problem Statement

## What It Does

A cross-shard serializable snapshot isolation (SSI) system manages transactions that read and write keys distributed across multiple independent shards. A transaction begins, performs a series of reads and writes to keys on any combination of shards, and then either commits (making all its writes visible atomically) or aborts (discarding all its writes). The system guarantees that the committed transactions form a history equivalent to some serial execution, even when transactions span multiple shards.

Each shard maintains its own key-value store. A transaction sees a consistent snapshot: it reads the values that were committed at the time the transaction began, unaffected by concurrent uncommitted writes. When two transactions conflict -- one reads a key that the other writes, or both write the same key -- the system detects the conflict and aborts one of them to preserve serializability. The challenge is coordinating this conflict detection across shard boundaries without a single global lock, so that transactions touching disjoint shards commit independently and without interference.

## Safety Properties

1. **Serializability**: The set of committed transactions is equivalent to some serial (one-at-a-time) execution. No committed transaction observes an anomaly (write skew, phantom, non-repeatable read) that would be impossible in a serial schedule.
2. **Snapshot isolation baseline**: Each transaction reads from a consistent snapshot taken at transaction begin time. A read within a transaction never observes a write from a transaction that had not committed before the reading transaction began.
3. **Atomicity**: A committed transaction's writes are all visible or none are. There is no state in which a reader sees some writes from a transaction but not others, even when those writes span multiple shards.
4. **Abort discards all writes**: If a transaction aborts (explicitly or due to conflict), none of its writes are ever visible to any other transaction.
5. **Write-write conflict detection**: If two concurrent transactions both write to the same key, at most one of them commits. The system does not silently lose a write.
6. **Read-write conflict detection (SSI)**: If transaction T1 reads key K and transaction T2 writes key K, and both attempt to commit, the system detects the dangerous structure (rw-antidependency cycle) and aborts at least one to prevent serializability violations.
7. **No false conflicts across shards**: Transactions that touch entirely disjoint sets of shard-key pairs do not conflict with each other and both commit successfully.

## Liveness Properties

1. **Non-conflicting progress**: If a set of transactions have no overlapping read-write or write-write sets, all of them eventually commit.
2. **Conflict resolution progress**: When a conflict is detected, at least one of the conflicting transactions commits. The system does not abort all participants.
3. **No deadlock**: The conflict detection and commit protocol do not deadlock, even when multiple transactions span the same set of shards in different orders.
4. **Abort is immediate**: An abort operation completes in bounded time, releasing all resources held by the transaction.
5. **Starvation freedom under bounded contention**: A transaction that is repeatedly aborted due to conflicts eventually commits if the contending transactions are finite.

## Performance Dimensions

- Commit throughput (transactions per second) for non-conflicting workloads at 1, 4, 8, and 16 threads.
- Commit throughput under varying conflict rates (0%, 10%, 50% of transactions touch overlapping keys).
- Abort rate: fraction of attempted commits that are aborted, as a function of contention.
- Cross-shard commit latency: additional time for a multi-shard commit compared to a single-shard commit.
- Read latency within a transaction (time from read invocation to value returned).
- Scalability with shard count: throughput as the number of shards increases from 2 to 32.
- Garbage collection overhead: cost of cleaning up aborted transaction state.

## What Is NOT Specified

- The data structure used within each shard (hash map, B-tree, skip list).
- The conflict detection algorithm (pessimistic locking, optimistic validation, serialization graph testing).
- The commit protocol for multi-shard transactions (two-phase commit, one-phase for single-shard, Paxos, deterministic ordering).
- The snapshot mechanism (multi-version concurrency control, copy-on-write, undo logs).
- The granularity of conflict detection (key-level, shard-level, predicate-level).
- Whether the system supports read-only transaction optimizations (avoiding validation for read-only transactions).
- The format of transaction IDs (monotonic counter, UUID, timestamp-based).
