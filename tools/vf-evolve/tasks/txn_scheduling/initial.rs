/// Transaction Scheduling - Initial Seed (Greedy FCFS Scheduler)
/// ShinkaEvolve will evolve this toward optimal scheduling policies.
///
/// Process transactions in arrival order, assign to first available resource.

/// A transaction with resource requirements and timing.
#[derive(Debug, Clone)]
pub struct Transaction {
    pub id: u64,
    pub duration: f64,
    pub resource_requirement: u64,
    pub priority: u64,
    pub arrival_time: f64,
    pub deadline: Option<f64>,
}

/// Result of scheduling a single transaction.
#[derive(Debug, Clone)]
pub struct ScheduleEntry {
    pub txn_id: u64,
    pub resource_id: u64,
    pub start_time: f64,
}

/// Result of scheduling a batch of transactions.
#[derive(Debug, Clone)]
pub struct ScheduleResult {
    pub assignments: Vec<ScheduleEntry>,
    pub makespan: f64,
    pub missed_deadlines: u64,
    pub utilization: f64,
}

/// Schedule transactions across `num_resources` identical resources.
///
/// Returns assignments minimizing makespan. Each resource has capacity 1.
pub fn schedule(transactions: &[Transaction], num_resources: u64) -> ScheduleResult {
    debug_assert!(num_resources > 0, "Must have at least one resource");
    debug_assert!(!transactions.is_empty(), "Must have at least one transaction");

    let mut resource_available_at = vec![0.0_f64; num_resources as usize];
    let mut assignments = Vec::with_capacity(transactions.len());
    let mut missed_deadlines: u64 = 0;
    let mut total_busy_time: f64 = 0.0;

    // Sort by arrival time (FCFS)
    let mut sorted: Vec<&Transaction> = transactions.iter().collect();
    sorted.sort_by(|a, b| a.arrival_time.partial_cmp(&b.arrival_time).unwrap());

    for txn in &sorted {
        // Find resource that becomes available earliest
        let (best_idx, best_available) = resource_available_at
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .unwrap();

        let start_time = best_available.max(txn.arrival_time);
        let finish_time = start_time + txn.duration;
        resource_available_at[best_idx] = finish_time;
        total_busy_time += txn.duration;

        assignments.push(ScheduleEntry {
            txn_id: txn.id,
            resource_id: best_idx as u64,
            start_time,
        });

        if let Some(deadline) = txn.deadline {
            if finish_time > deadline {
                missed_deadlines += 1;
            }
        }
    }

    let makespan = resource_available_at
        .iter()
        .copied()
        .fold(0.0_f64, f64::max);

    let total_capacity_time = if makespan > 0.0 {
        makespan * num_resources as f64
    } else {
        1.0
    };
    let utilization = (total_busy_time / total_capacity_time).min(1.0);

    ScheduleResult {
        assignments,
        makespan,
        missed_deadlines,
        utilization,
    }
}
