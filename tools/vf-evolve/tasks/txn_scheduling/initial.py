"""
Transaction Scheduling - Initial Seed (Simple Greedy Scheduler)
ShinkaEvolve will evolve this toward optimal scheduling policies.

Process transactions in arrival order, assign to first available resource.
"""

from dataclasses import dataclass, field
from typing import List, Optional


@dataclass
class Transaction:
    """A transaction with resource requirements and timing."""
    id: int
    duration: float          # Time units to complete
    resource_requirement: int # Resource slots needed
    priority: int = 0        # Higher = more important
    arrival_time: float = 0.0
    deadline: Optional[float] = None


@dataclass
class Resource:
    """A processing resource with capacity."""
    id: int
    capacity: int           # Total slots available
    used: int = 0           # Currently used slots
    available_at: float = 0.0  # When current work finishes

    @property
    def available_capacity(self) -> int:
        return self.capacity - self.used


@dataclass
class ScheduleResult:
    """Result of scheduling a batch of transactions."""
    assignments: List[tuple]  # (txn_id, resource_id, start_time)
    makespan: float           # Total completion time
    missed_deadlines: int     # Transactions that missed deadline
    utilization: float        # Average resource utilization [0, 1]


class GreedyScheduler:
    """Simple greedy scheduler: assign transactions FCFS to first available resource."""

    def __init__(self, resources: List[Resource]):
        assert len(resources) > 0, "Must have at least one resource"
        self.resources = [Resource(r.id, r.capacity) for r in resources]

    def schedule(self, transactions: List[Transaction]) -> ScheduleResult:
        """Schedule all transactions greedily."""
        assignments = []
        missed_deadlines = 0
        total_busy_time = 0.0

        # Process in arrival order (FCFS)
        sorted_txns = sorted(transactions, key=lambda t: t.arrival_time)

        for txn in sorted_txns:
            best_resource = None
            best_start = float('inf')

            for resource in self.resources:
                if resource.available_capacity >= txn.resource_requirement:
                    start_time = max(txn.arrival_time, resource.available_at)
                    if start_time < best_start:
                        best_start = start_time
                        best_resource = resource

            if best_resource is None:
                # Wait for earliest resource to free up
                self.resources.sort(key=lambda r: r.available_at)
                best_resource = self.resources[0]
                best_start = max(txn.arrival_time, best_resource.available_at)
                # Reset resource
                best_resource.used = 0

            # Assign
            best_resource.used += txn.resource_requirement
            finish_time = best_start + txn.duration
            best_resource.available_at = finish_time
            total_busy_time += txn.duration

            assignments.append((txn.id, best_resource.id, best_start))

            if txn.deadline is not None and finish_time > txn.deadline:
                missed_deadlines += 1

        makespan = max(r.available_at for r in self.resources) if self.resources else 0.0
        total_capacity_time = makespan * len(self.resources) if makespan > 0 else 1.0
        utilization = total_busy_time / total_capacity_time

        return ScheduleResult(
            assignments=assignments,
            makespan=makespan,
            missed_deadlines=missed_deadlines,
            utilization=min(utilization, 1.0),
        )


def schedule(txns, num_resources):
    """Adapter for ShinkaEvolve simulator interface.

    Args:
        txns: list of dicts with id, duration, resource, deps.
        num_resources: number of resources.

    Returns:
        list of dicts with id, start, duration.
    """
    resources = [Resource(id=i, capacity=1) for i in range(num_resources)]
    transactions = [
        Transaction(id=t["id"], duration=t["duration"], resource_requirement=1)
        for t in txns
    ]
    scheduler = GreedyScheduler(resources)
    result = scheduler.schedule(transactions)
    return [
        {"id": txn_id, "start": start, "duration": next(t["duration"] for t in txns if t["id"] == txn_id)}
        for txn_id, _, start in result.assignments
    ]


if __name__ == "__main__":
    resources = [Resource(id=i, capacity=2) for i in range(3)]
    txns = [
        Transaction(id=0, duration=5.0, resource_requirement=1, arrival_time=0.0),
        Transaction(id=1, duration=3.0, resource_requirement=1, arrival_time=1.0),
        Transaction(id=2, duration=4.0, resource_requirement=1, arrival_time=2.0),
        Transaction(id=3, duration=2.0, resource_requirement=2, arrival_time=3.0, deadline=6.0),
        Transaction(id=4, duration=6.0, resource_requirement=1, arrival_time=4.0),
    ]

    scheduler = GreedyScheduler(resources)
    result = scheduler.schedule(txns)
    print(f"Makespan: {result.makespan:.1f}")
    print(f"Missed deadlines: {result.missed_deadlines}")
    print(f"Utilization: {result.utilization:.2%}")
    for txn_id, res_id, start in result.assignments:
        print(f"  Txn {txn_id} -> Resource {res_id} @ t={start:.1f}")
