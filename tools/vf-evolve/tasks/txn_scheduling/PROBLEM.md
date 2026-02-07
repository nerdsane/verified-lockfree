# Transaction Scheduling: Problem Statement

## What It Does

A transaction scheduler assigns a set of transactions to a set of processing resources and determines when each transaction begins execution, with the goal of minimizing the total time to complete all transactions (the makespan). Transactions arrive with known durations, resource requirements, and dependency constraints. A transaction cannot begin until all transactions it depends on have completed, and a resource cannot host more concurrent transactions than its capacity allows.

The scheduler receives a batch of transactions and a description of available resources, and produces a schedule: an assignment of each transaction to a resource and a start time. A good scheduler exploits parallelism by packing independent transactions onto different resources simultaneously, respects dependency chains by ordering dependent transactions correctly, and avoids resource oversubscription at every point in time. The theoretical lower bound on makespan is determined by the critical path through the dependency graph and the total work divided by total resource capacity, whichever is larger.

## Safety Properties

1. **Completeness**: Every transaction in the input appears exactly once in the output schedule. No transaction is dropped or duplicated.
2. **Dependency ordering**: If transaction A depends on transaction B, then A's start time is no earlier than B's start time plus B's duration. No transaction begins before all of its predecessors have finished.
3. **Resource capacity**: At every instant in time, the number of transactions executing on any single resource does not exceed that resource's capacity.
4. **Non-negative timing**: Every transaction's start time is non-negative, and no transaction starts before its arrival time.
5. **Duration preservation**: Each transaction's scheduled duration matches its specified duration. The scheduler does not artificially shorten or extend any transaction.

## Liveness Properties

1. **Termination**: The scheduler produces a complete schedule in finite time for any valid input.
2. **Feasibility**: For any input with at least one feasible schedule, the scheduler produces a feasible schedule (one that satisfies all hard constraints).
3. **No starvation**: No transaction is deferred indefinitely when resources are available and its dependencies are met.
4. **Convergence under evolution**: As the scheduling algorithm is evolved, the makespan monotonically improves (or does not regress) across generations.

## Performance Dimensions

- Makespan: total time from the start of the first transaction to the completion of the last, normalized against the theoretical lower bound.
- Resource utilization: fraction of total resource-time that is occupied by executing transactions (higher is better).
- Critical path efficiency: ratio of achieved makespan to the critical path length through the dependency graph.
- Missed deadlines: number of transactions (if deadlines are specified) that complete after their deadline.
- Scheduler runtime: wall-clock time consumed by the scheduling algorithm itself for inputs of 100, 1000, and 10000 transactions.
- Sensitivity to conflict rate: how makespan degrades as the fraction of inter-transaction dependencies increases from 10% to 50%.

## What Is NOT Specified

- The scheduling algorithm (list scheduling, genetic algorithm, constraint programming, integer linear programming, reinforcement learning).
- Tie-breaking policy when multiple transactions are eligible and multiple resources are available.
- Whether the scheduler operates in a single pass or iteratively refines the schedule.
- Whether preemption is allowed (suspending a running transaction to start a higher-priority one).
- The data structure used to represent the dependency graph or the priority queue of ready transactions.
- How the scheduler handles infeasible inputs (e.g., circular dependencies).
