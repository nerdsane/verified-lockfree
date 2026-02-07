# Two-Phase Commit: Problem Statement

## What It Does

A two-phase commit (2PC) coordinator manages a set of resource managers (participants) to atomically commit or abort a distributed transaction. The coordinator drives the protocol in two phases. In the prepare phase, the coordinator sends a Prepare message to every resource manager and waits for each to respond with either Prepared (vote yes) or Aborted (vote no). In the commit phase, if all resource managers voted yes, the coordinator sends Commit to all; if any voted no or timed out, the coordinator sends Abort to all. Each resource manager applies or discards its local changes according to the coordinator's final decision.

The defining characteristic is atomicity across independent failure domains: either every resource manager commits or every resource manager aborts. No intermediate state is observable by external readers after the protocol completes. The protocol tolerates individual resource manager failures (they will recover and learn the decision), but the coordinator is a single point of failure during the commit window. A resource manager that has voted Prepared is in doubt -- it cannot unilaterally decide to commit or abort until it hears the coordinator's decision. This blocking property under coordinator failure is the fundamental limitation of 2PC, and the primary axis of evolution: improved implementations add presumed-abort optimization, timeout-based abort, cooperative recovery, or transition to 3PC/Paxos to reduce or eliminate the blocking window.

## Safety Properties

1. **Atomicity**: If any resource manager is in the Committed state, no resource manager is in the Aborted state, and vice versa. The final states of all resource managers agree.
2. **Validity**: The coordinator commits only if every resource manager voted Prepared. If any resource manager voted to abort (or failed to respond), the coordinator must abort.
3. **Consistency**: A resource manager reaches the Committed state only after the coordinator decided to commit. A resource manager reaches the Aborted state only after the coordinator decided to abort or the resource manager unilaterally aborted before voting.
4. **Agreement**: Once the coordinator makes a decision (Commit or Abort), that decision is irrevocable. No subsequent message or failure changes the outcome.
5. **Prepare-before-commit**: No resource manager receives a Commit message before it has sent a Prepared message. The commit phase does not begin until the prepare phase completes.
6. **No lost votes**: Every Prepared or Aborted vote from a resource manager is eventually delivered to the coordinator (under fair scheduling). Votes are not silently dropped.

## Liveness Properties

1. **Termination**: If the coordinator and a majority of resource managers remain alive, all surviving resource managers eventually reach a terminal state (Committed or Aborted).
2. **Non-blocking progress under coordinator recovery**: If the coordinator crashes and recovers, it replays its decision log and completes the protocol. Resource managers do not wait indefinitely.
3. **Abort progress**: If any resource manager aborts before preparing, the coordinator eventually discovers this and aborts the entire transaction.
4. **No starvation**: A resource manager that is repeatedly involved in transactions that abort eventually participates in a transaction that commits, provided the workload includes non-conflicting transactions.

## Performance Dimensions

- Commit latency: wall-clock time from first Prepare sent to last Commit acknowledged, for 2, 4, 8, and 16 resource managers.
- Abort latency: wall-clock time from abort decision to all resource managers reaching Aborted state.
- Message complexity: total messages per transaction (2PC requires 4N messages for N resource managers in the success case).
- Throughput: transactions committed per second under varying concurrency levels (1, 4, 16, 64 concurrent transactions).
- Blocking window: duration during which in-doubt resource managers cannot make progress, measured under coordinator failure.
- Recovery time: time for a recovering coordinator to replay its log and complete all pending transactions.

## What Is NOT Specified

- The coordinator's failure recovery mechanism (write-ahead log, replicated state, Paxos-based coordinator).
- The transport layer (TCP, in-process channels, gRPC).
- Whether presumed-abort or presumed-commit optimizations are used.
- The timeout strategy for detecting unresponsive resource managers (fixed, adaptive, exponential backoff).
- Whether the protocol supports read-only optimization (resource managers with no writes skip the prepare phase).
- The concurrency control mechanism at each resource manager (locks, MVCC, optimistic validation).
- Whether 3PC or cooperative termination protocols are used to reduce the blocking window.
