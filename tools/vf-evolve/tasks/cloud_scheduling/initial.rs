/// Cloud Scheduling - Initial Seed (Greedy Cheapest-First Scheduler)
/// ShinkaEvolve will evolve this toward optimal cloud scheduling policies.
///
/// Assign jobs to the cheapest available instance that meets the deadline.

/// A compute job with requirements and deadline.
#[derive(Debug, Clone)]
pub struct Job {
    pub id: u64,
    pub cpu_cores: u64,
    pub memory_gb: f64,
    pub duration_hours: f64,
    pub deadline_hours: f64,
    pub priority: u64,
    pub arrival_time: f64,
}

/// A cloud instance type with pricing.
#[derive(Debug, Clone)]
pub struct InstanceType {
    pub id: u64,
    pub cpu_cores: u64,
    pub memory_gb: f64,
    pub cost_per_hour: f64,
    pub available_count: u64,
}

/// A single job-to-instance assignment.
#[derive(Debug, Clone)]
pub struct ScheduleEntry {
    pub job_id: u64,
    pub instance_type_id: u64,
    pub start_time: f64,
    pub end_time: f64,
    pub cost: f64,
}

/// Result of cloud scheduling.
#[derive(Debug, Clone)]
pub struct SchedulingResult {
    pub schedule: Vec<ScheduleEntry>,
    pub total_cost: f64,
    pub missed_deadlines: u64,
    pub avg_wait_time: f64,
}

/// Schedule jobs across cloud instances minimizing total cost under SLA deadlines.
pub fn schedule_jobs(jobs: &[Job], instance_types: &[InstanceType]) -> SchedulingResult {
    debug_assert!(!jobs.is_empty(), "Must have at least one job");
    debug_assert!(!instance_types.is_empty(), "Must have at least one instance type");

    // Sort instance types by cost ascending
    let mut sorted_types: Vec<&InstanceType> = instance_types.iter().collect();
    sorted_types.sort_by(|a, b| a.cost_per_hour.partial_cmp(&b.cost_per_hour).unwrap());

    // Track when each instance copy becomes available
    let mut instance_availability: Vec<Vec<f64>> = instance_types
        .iter()
        .map(|it| vec![0.0; it.available_count as usize])
        .collect();

    let mut schedule = Vec::with_capacity(jobs.len());
    let mut total_cost: f64 = 0.0;
    let mut missed_deadlines: u64 = 0;
    let mut total_wait: f64 = 0.0;

    // Sort jobs by arrival time
    let mut sorted_jobs: Vec<&Job> = jobs.iter().collect();
    sorted_jobs.sort_by(|a, b| a.arrival_time.partial_cmp(&b.arrival_time).unwrap());

    for job in &sorted_jobs {
        let mut best_option: Option<(usize, usize, f64)> = None; // (type_idx, copy_idx, start)

        // Find cheapest instance type that fits
        for (type_idx, inst_type) in sorted_types.iter().enumerate() {
            if inst_type.cpu_cores >= job.cpu_cores && inst_type.memory_gb >= job.memory_gb {
                let orig_idx = instance_types
                    .iter()
                    .position(|it| it.id == inst_type.id)
                    .unwrap();
                let avail = &instance_availability[orig_idx];
                if avail.is_empty() {
                    continue;
                }
                let (earliest_copy, earliest_time) = avail
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
                    .unwrap();
                let start = earliest_time.max(job.arrival_time);
                best_option = Some((orig_idx, earliest_copy, start));
                break;
            }
        }

        if let Some((type_idx, copy_idx, start_time)) = best_option {
            let inst_type = &instance_types[type_idx];
            let end_time = start_time + job.duration_hours;
            let cost = job.duration_hours * inst_type.cost_per_hour;

            instance_availability[type_idx][copy_idx] = end_time;

            schedule.push(ScheduleEntry {
                job_id: job.id,
                instance_type_id: inst_type.id,
                start_time,
                end_time,
                cost,
            });

            total_cost += cost;
            total_wait += start_time - job.arrival_time;

            if end_time > job.arrival_time + job.deadline_hours {
                missed_deadlines += 1;
            }
        } else {
            missed_deadlines += 1;
        }
    }

    let avg_wait = if sorted_jobs.is_empty() {
        0.0
    } else {
        total_wait / sorted_jobs.len() as f64
    };

    SchedulingResult {
        schedule,
        total_cost,
        missed_deadlines,
        avg_wait_time: avg_wait,
    }
}
