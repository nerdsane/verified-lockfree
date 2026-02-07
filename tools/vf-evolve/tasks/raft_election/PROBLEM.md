# Raft Leader Election: Problem Statement

## What It Does

A Raft leader election system manages a cluster of servers that must agree on exactly one leader at any time. The cluster begins with all servers as followers. When a follower does not hear from a leader within a randomized timeout, it promotes itself to candidate, increments its term number, votes for itself, and requests votes from all other servers. A server grants its vote to at most one candidate per term, on a first-come-first-served basis. A candidate that receives votes from a strict majority of the cluster (a quorum) promotes itself to leader and begins sending periodic heartbeats to all followers to maintain authority. A heartbeat from a leader with a current or higher term resets the follower's election timeout, preventing unnecessary elections.

The defining characteristic is that the protocol must guarantee at most one leader per term, even under arbitrary message delays, reordering, and server crashes. The quorum intersection property -- any two majorities share at least one member -- ensures that two candidates cannot both collect enough votes in the same term. Terms act as logical clocks: a server that discovers a higher term immediately reverts to follower state, and a candidate or leader that receives a message with a higher term steps down. The result is a leader election protocol that tolerates up to (N-1)/2 simultaneous server failures in a cluster of N servers.

## Safety Properties

1. **Election safety**: At most one server is in the Leader state for any given term. If server A is leader in term T, no other server can become leader in term T.
2. **Vote integrity**: Each server votes for at most one candidate per term. Once a server grants a vote in term T, it does not grant a vote to a different candidate in term T.
3. **Quorum requirement**: A server transitions to Leader only after receiving votes from a strict majority of the cluster (including its own self-vote). No server becomes leader without a quorum.
4. **Term monotonicity**: A server's term number never decreases. Every state transition either preserves or increases the term.
5. **Leader step-down**: A leader that receives a message with a term strictly greater than its own immediately reverts to Follower. No leader persists with a stale term.
6. **Heartbeat consistency**: A follower that receives a valid heartbeat (term >= its own) does not start an election. Heartbeats suppress spurious elections.

## Liveness Properties

1. **Eventual leader**: If a majority of servers are alive and can communicate, a leader is eventually elected. The cluster does not remain leaderless indefinitely.
2. **Timeout progress**: A follower whose election timeout fires eventually becomes a candidate and starts an election, unless it receives a valid heartbeat first.
3. **Election resolution**: An election that starts eventually terminates: either a candidate wins a quorum and becomes leader, or the election times out and a new term begins.
4. **Heartbeat liveness**: A leader periodically sends heartbeats to all followers. Under fair scheduling, followers eventually receive heartbeats.

## Performance Dimensions

- Election convergence time: wall-clock time from leader failure to new leader established, under various cluster sizes (3, 5, 7, 9 servers).
- Spurious election rate: number of elections started per unit time in a stable cluster (should be near zero).
- Message complexity per election: total messages exchanged to elect a leader (RequestVote + VoteResponse + Heartbeat).
- Split-vote probability: fraction of elections that fail due to vote splitting, as a function of timeout randomization range.
- Throughput under churn: leader election rate when servers are continuously joining and leaving the cluster.
- Scalability: how election latency and message count scale as cluster size increases from 3 to 99.

## What Is NOT Specified

- The timeout duration or randomization strategy (uniform, exponential, jittered).
- The mechanism for persisting term and votedFor to stable storage (write-ahead log, file, memory-mapped).
- Whether pre-vote or check-quorum optimizations are used to reduce disruption from partitioned servers.
- The network transport (TCP, UDP, gRPC, in-process channels).
- How log entries or state machine replication interact with the election (this spec covers election only, not log replication).
- Whether the implementation uses async channels, message passing, or shared memory for inter-server communication.
- The backoff strategy when repeated elections fail (linear, exponential, randomized).
