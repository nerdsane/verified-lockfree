"""
Cloud Scheduling - Initial Seed (Greedy Cheapest-First Scheduler)
ShinkaEvolve will evolve this toward optimal cloud scheduling policies.

Assign jobs to the cheapest available instance that meets the deadline.
"""

from dataclasses import dataclass, field
from typing import List, Optional, Dict


@dataclass
class Job:
    """A compute job with requirements and deadline."""
    id: int
    cpu_cores: int           # CPU cores required
    memory_gb: float         # Memory required in GB
    duration_hours: float    # Estimated runtime
    deadline_hours: float    # Must complete within this time
    priority: int = 0        # Higher = more important
    arrival_time: float = 0.0


@dataclass
class Instance:
    """A cloud instance type with pricing."""
    id: str                  # e.g., "m5.xlarge"
    cpu_cores: int
    memory_gb: float
    cost_per_hour: float
    available_count: int     # How many can be provisioned
    startup_time: float = 0.0  # Time to provision in hours


@dataclass
class ScheduleEntry:
    """A single job-to-instance assignment."""
    job_id: int
    instance_id: str
    start_time: float
    end_time: float
    cost: float


@dataclass
class SchedulingResult:
    """Result of cloud scheduling."""
    schedule: List[ScheduleEntry]
    total_cost: float
    missed_deadlines: int
    avg_wait_time: float
    instance_utilization: Dict[str, float]  # instance_id -> utilization ratio


class GreedyCheapestScheduler:
    """Greedy scheduler: assign each job to the cheapest instance that fits."""

    def __init__(self, instances: List[Instance]):
        assert len(instances) > 0, "Must have at least one instance type"
        # Sort by cost ascending
        self.instances = sorted(instances, key=lambda i: i.cost_per_hour)
        self.instance_availability: Dict[str, List[float]] = {}
        for inst in self.instances:
            # Track when each instance copy becomes available
            self.instance_availability[inst.id] = [0.0] * inst.available_count

    def _find_cheapest_fit(self, job: Job) -> Optional[tuple]:
        """Find the cheapest instance that can run this job, and earliest start time."""
        for inst in self.instances:
            if inst.cpu_cores >= job.cpu_cores and inst.memory_gb >= job.memory_gb:
                # Find earliest available copy
                avail_times = self.instance_availability[inst.id]
                if not avail_times:
                    continue
                earliest_idx = min(range(len(avail_times)), key=lambda i: avail_times[i])
                start_time = max(avail_times[earliest_idx], job.arrival_time) + inst.startup_time
                return (inst, earliest_idx, start_time)
        return None

    def schedule(self, jobs: List[Job]) -> SchedulingResult:
        """Schedule all jobs greedily."""
        schedule = []
        missed_deadlines = 0
        total_cost = 0.0
        total_wait = 0.0
        instance_busy_time: Dict[str, float] = {inst.id: 0.0 for inst in self.instances}

        # Process in arrival order
        sorted_jobs = sorted(jobs, key=lambda j: j.arrival_time)

        for job in sorted_jobs:
            result = self._find_cheapest_fit(job)
            if result is None:
                # No instance can run this job - skip with missed deadline
                missed_deadlines += 1
                continue

            inst, copy_idx, start_time = result
            end_time = start_time + job.duration_hours
            cost = job.duration_hours * inst.cost_per_hour

            # Update availability
            self.instance_availability[inst.id][copy_idx] = end_time

            schedule.append(ScheduleEntry(
                job_id=job.id,
                instance_id=inst.id,
                start_time=start_time,
                end_time=end_time,
                cost=cost,
            ))

            total_cost += cost
            total_wait += start_time - job.arrival_time
            instance_busy_time[inst.id] += job.duration_hours

            if end_time > job.arrival_time + job.deadline_hours:
                missed_deadlines += 1

        avg_wait = total_wait / len(sorted_jobs) if sorted_jobs else 0.0

        # Compute utilization
        max_time = max(
            (max(times) for times in self.instance_availability.values() if times),
            default=1.0,
        )
        instance_utilization = {}
        for inst in self.instances:
            total_capacity = max_time * inst.available_count if max_time > 0 else 1.0
            instance_utilization[inst.id] = (
                instance_busy_time[inst.id] / total_capacity
                if total_capacity > 0 else 0.0
            )

        return SchedulingResult(
            schedule=schedule,
            total_cost=total_cost,
            missed_deadlines=missed_deadlines,
            avg_wait_time=avg_wait,
            instance_utilization=instance_utilization,
        )


def schedule_jobs(jobs_dicts, instance_costs):
    """Adapter for ShinkaEvolve simulator interface.

    Args:
        jobs_dicts: list of dicts with id, duration, deadline.
        instance_costs: list of costs per instance type.

    Returns:
        list of dicts with id, instance, cost.
    """
    result = []
    for job in jobs_dicts:
        # Pick cheapest instance
        cheapest_idx = min(range(len(instance_costs)), key=lambda i: instance_costs[i])
        cost = instance_costs[cheapest_idx] * job["duration"]
        result.append({"id": job["id"], "instance": cheapest_idx, "cost": cost})
    return result


if __name__ == "__main__":
    instances = [
        Instance(id="small", cpu_cores=2, memory_gb=4.0, cost_per_hour=0.10, available_count=5),
        Instance(id="medium", cpu_cores=4, memory_gb=16.0, cost_per_hour=0.30, available_count=3),
        Instance(id="large", cpu_cores=8, memory_gb=32.0, cost_per_hour=0.80, available_count=2),
        Instance(id="xlarge", cpu_cores=16, memory_gb=64.0, cost_per_hour=1.50, available_count=1),
    ]

    jobs = [
        Job(id=0, cpu_cores=1, memory_gb=2.0, duration_hours=2.0, deadline_hours=4.0, arrival_time=0.0),
        Job(id=1, cpu_cores=4, memory_gb=8.0, duration_hours=1.0, deadline_hours=2.0, arrival_time=0.5),
        Job(id=2, cpu_cores=2, memory_gb=4.0, duration_hours=3.0, deadline_hours=5.0, arrival_time=1.0),
        Job(id=3, cpu_cores=8, memory_gb=16.0, duration_hours=0.5, deadline_hours=1.0, arrival_time=1.5),
        Job(id=4, cpu_cores=1, memory_gb=1.0, duration_hours=4.0, deadline_hours=8.0, arrival_time=2.0),
    ]

    scheduler = GreedyCheapestScheduler(instances)
    result = scheduler.schedule(jobs)
    print(f"Total cost: ${result.total_cost:.2f}")
    print(f"Missed deadlines: {result.missed_deadlines}")
    print(f"Avg wait time: {result.avg_wait_time:.2f}h")
    for entry in result.schedule:
        print(f"  Job {entry.job_id} -> {entry.instance_id} "
              f"[{entry.start_time:.1f}-{entry.end_time:.1f}h] ${entry.cost:.2f}")
