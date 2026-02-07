# Raft Consensus: Full Problem Statement

## What It Does

A full Raft consensus implementation that provides leader election, log replication, and commit safety. The cluster of servers replicates a totally-ordered log of commands. Clients propose entries to the leader; the leader appends entries to its log, replicates them to followers, and commits entries once a majority of servers acknowledge them. Committed entries are durable and will never be overwritten.

This extends the raft_election task with log replication, commit tracking, and the full set of Raft safety invariants from the Ongaro/Ousterhout paper.

## API

```rust
pub struct RaftNode { ... }

impl RaftNode {
    /// Create a new Raft node with the given ID and peer IDs.
    pub fn new(id: u64, peers: Vec<u64>) -> Self;

    /// Advance the internal logical clock by one tick.
    /// Triggers election timeouts and heartbeat intervals.
    pub fn tick(&mut self);

    /// Propose a new entry to be replicated (only valid on leader).
    pub fn propose(&mut self, data: Vec<u8>) -> Result<u64, RaftError>;

    /// Process an incoming message from another node.
    pub fn step(&mut self, message: Message) -> Result<(), RaftError>;

    /// Collect any output ready to be sent: messages, committed entries, etc.
    pub fn ready(&self) -> Ready;

    /// Acknowledge that the Ready output has been processed.
    pub fn advance(&mut self, ready: Ready);
}
```

## Safety Properties (from Raft paper, Figure 3)

1. **Election Safety** — At most one leader per term.
2. **Leader Append-Only** — A leader never overwrites or deletes entries in its log.
3. **Log Matching** — If two logs contain an entry with the same index and term, then all preceding entries are identical.
4. **Leader Completeness** — If an entry is committed in a given term, that entry will be present in the logs of all leaders of higher-numbered terms.
5. **State Machine Safety** — If a server has applied a log entry at a given index, no other server will ever apply a different entry for that index.

## Liveness Properties

1. **Eventual Leader** — If a majority of servers are alive and connected, a leader is eventually elected.
2. **Eventual Commit** — If a leader proposes an entry and a majority of servers are alive, the entry is eventually committed.
3. **Commit Progress** — The commit index monotonically increases over time.

## Performance Dimensions

- Election convergence time under various cluster sizes (3, 5, 7).
- Log replication latency from propose to commit.
- Throughput: committed entries per second under steady-state.
- Recovery time: how quickly a new leader catches up followers after election.
- Message complexity per committed entry.

## What Is NOT Specified

- Persistence mechanism (WAL, mmap, in-memory).
- Log compaction or snapshotting.
- Membership changes (configuration changes).
- Client interaction protocol (linearizability, read indices).
- Network transport mechanism.
- Batch optimization for AppendEntries.
