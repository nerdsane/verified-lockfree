/// Cloud Scheduling - Lock-Free Implementation with WaitFree Guarantees
/// Uses atomic operations and wait-free algorithms for deterministic scheduling.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

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

/// Wait-free instance availability tracker using atomic operations
struct AtomicInstanceTracker {
    /// For each instance type, stores packed availability times (simulated via atomic counters)
    type_counters: Vec<Arc<AtomicU64>>,
    type_timings: Vec<Vec<std::sync::Arc<std::sync::atomic::AtomicU64>>>,
}

impl AtomicInstanceTracker {
    fn new(instance_types: &[InstanceType]) -> Self {
        let type_counters = instance_types
            .iter()
            .map(|_| Arc::new(AtomicU64::new(0)))
            .collect();
        
        let type_timings = instance_types
            .iter()
            .map(|it| {
                (0..it.available_count)
                    .map(|_| Arc::new(AtomicU64::new(0)))
                    .collect()
            })
            .collect();

        AtomicInstanceTracker {
            type_counters,
            type_timings,
        }
    }

    fn get_earliest_available(
        &self,
        type_idx: usize,
        instance_count: u64,
    ) -> (usize, f64) {
        let timings = &self.type_timings[type_idx];
        let mut earliest_copy = 0;
        let mut earliest_time = f64::MAX;

        for (copy_idx, timer) in timings.iter().enumerate() {
            let bits = timer.load(Ordering::Acquire);
            let time = f64::from_bits(bits);
            if time < earliest_time {
                earliest_time = time;
                earliest_copy = copy_idx;
            }
        }

        (earliest_copy, earliest_time)
    }

    fn try_schedule(
        &self,
        type_idx: usize,
        copy_idx: usize,
        current_available: f64,
        new_available: f64,
    ) -> bool {
        let timer = &self.type_timings[type_idx][copy_idx];
        let current_bits = current_available.to_bits() as u64;
        let new_bits = new_available.to_bits() as u64;

        timer
            .compare_exchange(current_bits, new_bits, Ordering::Release, Ordering::Acquire)
            .is_ok()
    }
}

/// Schedule jobs across cloud instances minimizing total cost under SLA deadlines.
/// Uses wait-free algorithm for deterministic performance.
pub fn schedule_jobs(jobs: &[Job], instance_types: &[InstanceType]) -> SchedulingResult {
    debug_assert!(!jobs.is_empty(), "Must have at least one job");
    debug_assert!(!instance_types.is_empty(), "Must have at least one instance type");

    // Sort instance types by cost ascending
    let mut sorted_types: Vec<(usize, &InstanceType)> = instance_types
        .iter()
        .enumerate()
        .collect();
    sorted_types.sort_by(|a, b| {
        a.1.cost_per_hour
            .partial_cmp(&b.1.cost_per_hour)
            .unwrap()
    });

    // Initialize atomic availability tracker
    let tracker = AtomicInstanceTracker::new(instance_types);

    let mut schedule = Vec::with_capacity(jobs.len());
    let total_cost = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let missed_deadlines = Arc::new(AtomicUsize::new(0));
    let total_wait = Arc::new(std::sync::atomic::AtomicU64::new(0));

    // Sort jobs by arrival time, then by priority (descending)
    let mut sorted_jobs: Vec<&Job> = jobs.iter().collect();
    sorted_jobs.sort_by(|a, b| {
        match a.arrival_time.partial_cmp(&b.arrival_time).unwrap() {
            std::cmp::Ordering::Equal => b.priority.cmp(&a.priority),
            other => other,
        }
    });

    for job in &sorted_jobs {
        let mut scheduled = false;

        // Find cheapest instance type that fits (wait-free search)
        for (_, inst_type) in &sorted_types {
            if inst_type.cpu_cores >= job.cpu_cores
                && inst_type.memory_gb >= job.memory_gb
            {
                let type_idx = instance_types
                    .iter()
                    .position(|it| it.id == inst_type.id)
                    .unwrap();

                // Wait-free attempt to schedule on earliest available copy
                let (copy_idx, earliest_available) =
                    tracker.get_earliest_available(type_idx, inst_type.available_count);

                let start_time = earliest_available.max(job.arrival_time);
                let end_time = start_time + job.duration_hours;
                let cost = job.duration_hours * inst_type.cost_per_hour;

                // Try to claim this slot with CAS
                if tracker.try_schedule(type_idx, copy_idx, earliest_available, end_time) {
                    schedule.push(ScheduleEntry {
                        job_id: job.id,
                        instance_type_id: inst_type.id,
                        start_time,
                        end_time,
                        cost,
                    });

                    // Update statistics atomically
                    let cost_bits = cost.to_bits() as u64;
                    let _ = total_cost.fetch_add(cost_bits, Ordering::Release);

                    let wait = (start_time - job.arrival_time).to_bits() as u64;
                    let _ = total_wait.fetch_add(wait, Ordering::Release);

                    // Check deadline miss
                    if end_time > job.arrival_time + job.deadline_hours {
                        let _ = missed_deadlines.fetch_add(1, Ordering::Release);
                    }

                    scheduled = true;
                    break;
                }
            }
        }

        if !scheduled {
            let _ = missed_deadlines.fetch_add(1, Ordering::Release);
        }
    }

    // Reconstruct final values from atomic types
    let total_cost_value = f64::from_bits(total_cost.load(Ordering::Acquire));
    let total_wait_bits = total_wait.load(Ordering::Acquire);
    let total_wait_value = if total_wait_bits > 0 {
        f64::from_bits(total_wait_bits)
    } else {
        0.0
    };
    let missed = missed_deadlines.load(Ordering::Acquire);

    let avg_wait = if sorted_jobs.is_empty() {
        0.0
    } else {
        total_wait_value / sorted_jobs.len() as f64
    };

    SchedulingResult {
        schedule,
        total_cost: total_cost_value,
        missed_deadlines: missed as u64,
        avg_wait_time: avg_wait,
    }
}