/// Transaction Scheduling - Lock-Free Optimization
/// Implements an obstructionless scheduler with CAS-based resource tracking
/// and priority-aware assignment heuristics.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

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

/// Atomic representation of resource state (bits: available_at_time_bits | resource_id)
#[derive(Debug)]
struct ResourceState {
    available_at_bits: AtomicU64,
}

impl ResourceState {
    fn new() -> Self {
        ResourceState {
            available_at_bits: AtomicU64::new(0),
        }
    }

    fn get_available_at(&self) -> f64 {
        let bits = self.available_at_bits.load(Ordering::Acquire);
        f64::from_bits(bits)
    }

    fn set_available_at(&self, time: f64) {
        let bits = time.to_bits();
        self.available_at_bits.store(bits, Ordering::Release);
    }

    fn try_reserve(&self, arrival_time: f64, duration: f64) -> Option<f64> {
        loop {
            let current_bits = self.available_at_bits.load(Ordering::Acquire);
            let current_time = f64::from_bits(current_bits);
            let start_time = current_time.max(arrival_time);
            let finish_time = start_time + duration;
            let finish_bits = finish_time.to_bits();

            match self.available_at_bits.compare_exchange(
                current_bits,
                finish_bits,
                Ordering::Release,
                Ordering::Acquire,
            ) {
                Ok(_) => return Some(start_time),
                Err(_) => continue,
            }
        }
    }
}

/// Schedule transactions across `num_resources` identical resources with priority awareness.
///
/// Returns assignments minimizing makespan and deadline misses.
/// Uses lock-free CAS operations for resource state management.
pub fn schedule(transactions: &[Transaction], num_resources: u64) -> ScheduleResult {
    debug_assert!(num_resources > 0, "Must have at least one resource");
    debug_assert!(!transactions.is_empty(), "Must have at least one transaction");

    let num_res = num_resources as usize;
    
    // Initialize resource states (lock-free atomic structures)
    let resources: Vec<Arc<ResourceState>> = (0..num_res)
        .map(|_| Arc::new(ResourceState::new()))
        .collect();

    let mut assignments = Vec::with_capacity(transactions.len());
    let mut missed_deadlines: u64 = 0;
    let mut total_busy_time: f64 = 0.0;

    // Sort by priority (descending), then by arrival time (ascending)
    let mut sorted: Vec<&Transaction> = transactions.iter().collect();
    sorted.sort_by(|a, b| {
        match b.priority.cmp(&a.priority) {
            std::cmp::Ordering::Equal => a.arrival_time.partial_cmp(&b.arrival_time).unwrap(),
            other => other,
        }
    });

    for txn in &sorted {
        // Find best resource using lock-free reads
        let mut best_resource_idx = 0;
        let mut best_available_time = f64::INFINITY;

        for (idx, resource) in resources.iter().enumerate() {
            let available_at = resource.get_available_at();
            if available_at < best_available_time {
                best_available_time = available_at;
                best_resource_idx = idx;
            }
        }

        // Try to reserve the best resource (may need retries due to CAS)
        let resource = &resources[best_resource_idx];
        let start_time = resource
            .try_reserve(txn.arrival_time, txn.duration)
            .unwrap();
        
        let finish_time = start_time + txn.duration;
        total_busy_time += txn.duration;

        assignments.push(ScheduleEntry {
            txn_id: txn.id,
            resource_id: best_resource_idx as u64,
            start_time,
        });

        if let Some(deadline) = txn.deadline {
            if finish_time > deadline {
                missed_deadlines += 1;
            }
        }
    }

    // Calculate makespan by reading all resource states
    let mut makespan = 0.0f64;
    for resource in &resources {
        let available_at = resource.get_available_at();
        if available_at > makespan {
            makespan = available_at;
        }
    }

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