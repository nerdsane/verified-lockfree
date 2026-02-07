# Cloud Batch Scheduling: Problem Statement

## What It Does

A cloud batch scheduler assigns compute jobs to cloud instances with the goal of minimizing total dollar cost while ensuring every job completes before its deadline. Jobs arrive with known CPU, memory, and duration requirements, and each has a deadline by which it must finish. The scheduler chooses from a catalog of instance types that vary in size and price, and must respect per-type account limits on how many instances can run simultaneously.

The scheduler operates in an environment of uncertainty: spot instances are significantly cheaper than on-demand instances but may be preempted (terminated) by the cloud provider at any time. When a spot instance is preempted, the job running on it must be rescheduled onto another instance, incurring both the cost of the new instance and the delay of re-execution. The scheduler must decide not only which instance to assign each job to, but also whether to gamble on cheap spot pricing at the risk of preemption or pay more for the reliability of on-demand instances. Data locality adds a further dimension: scheduling a job in a region far from its data incurs cross-region transfer costs and latency.

## Safety Properties

1. **Deadline satisfaction**: Every job completes execution before its deadline. A job that cannot meet its deadline due to preemption must be rescheduled immediately, and the penalty for a missed deadline is 10x the job's compute cost.
2. **Resource limits**: The total number of instances of any single type running at any point in time does not exceed the account limit for that type.
3. **Resource adequacy**: Each job is assigned to an instance whose CPU cores and memory meet or exceed the job's requirements. No job runs on an instance too small for it.
4. **Preemption handling**: When a spot instance is preempted, the affected job is rescheduled on an alternative instance. No job is silently lost to preemption.
5. **Schedule completeness**: Every job in the input appears in the output schedule. No job is dropped.
6. **Non-overlapping assignment**: No single instance runs two jobs whose execution intervals overlap, unless the instance has sufficient remaining capacity (for multi-tenant instance types).

## Liveness Properties

1. **Termination**: The scheduler produces a complete schedule in finite time for any valid input.
2. **Feasibility**: If a feasible schedule exists (enough instance capacity to complete all jobs by their deadlines), the scheduler finds one.
3. **Preemption recovery**: After a spot preemption, the rescheduling of the affected job begins within bounded time. The system does not wait indefinitely for the preempted spot instance to return.
4. **Convergence under evolution**: As the scheduling algorithm is evolved, total cost monotonically improves (or does not regress) across generations for the same workload and preemption scenario.

## Performance Dimensions

- Total dollar cost: sum of (instance cost per hour * hours used) across all instances, normalized against the cost of running every job on the cheapest on-demand instance that fits.
- Missed deadline count: number of jobs that finish after their deadline, weighted by penalty cost.
- Spot utilization: fraction of total compute-hours served by spot instances (higher means more savings, but more preemption risk).
- Average job wait time: mean time from job arrival to job start.
- Instance utilization: average fraction of each instance's uptime that is spent executing a job (vs. idle).
- Scheduler runtime: wall-clock time consumed by the scheduling algorithm itself for 50, 200, and 1000 jobs.
- Preemption resilience: degradation in cost and deadline satisfaction as the spot preemption rate increases from 5% to 30%.

## What Is NOT Specified

- The scheduling algorithm (greedy, constraint programming, reinforcement learning, simulated annealing, priority queues).
- The spot pricing model (fixed discount, variable market price, auction-based).
- Whether the scheduler uses job checkpointing to reduce re-execution cost after preemption.
- The instance provisioning delay model (constant startup time, variable by region, instant).
- Whether the scheduler considers job dependencies or treats all jobs as independent.
- The tie-breaking policy when multiple instance types have the same cost and fit.
- Whether the scheduler operates in an online (streaming arrivals) or offline (full batch known upfront) mode.
