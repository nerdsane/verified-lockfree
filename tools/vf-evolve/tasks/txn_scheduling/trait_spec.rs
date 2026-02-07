/// Trait specification tests for Transaction Scheduling.
///
/// The evolved code must provide:
///   pub struct Transaction { id, duration, resource_requirement, priority, arrival_time, deadline }
///   pub struct ScheduleResult { assignments, makespan, missed_deadlines, utilization }
///   pub fn schedule(transactions: &[Transaction], num_resources: u64) -> ScheduleResult

#[cfg(test)]
mod tests {
    use super::*;

    fn make_txn(id: u64, duration: f64, arrival: f64, deadline: Option<f64>) -> Transaction {
        Transaction {
            id,
            duration,
            resource_requirement: 1,
            priority: 0,
            arrival_time: arrival,
            deadline,
        }
    }

    #[test]
    fn test_single_transaction() {
        let txns = vec![make_txn(0, 5.0, 0.0, None)];
        let result = schedule(&txns, 1);
        assert_eq!(result.assignments.len(), 1);
        assert!((result.makespan - 5.0).abs() < 1e-9);
        assert_eq!(result.missed_deadlines, 0);
    }

    #[test]
    fn test_parallel_on_multiple_resources() {
        let txns = vec![
            make_txn(0, 10.0, 0.0, None),
            make_txn(1, 10.0, 0.0, None),
        ];
        let result = schedule(&txns, 2);
        assert_eq!(result.assignments.len(), 2);
        // Both should run in parallel -> makespan = 10
        assert!(result.makespan <= 10.0 + 1e-9);
    }

    #[test]
    fn test_sequential_on_one_resource() {
        let txns = vec![
            make_txn(0, 5.0, 0.0, None),
            make_txn(1, 3.0, 0.0, None),
        ];
        let result = schedule(&txns, 1);
        // Must run sequentially -> makespan = 8
        assert!((result.makespan - 8.0).abs() < 1e-9);
    }

    #[test]
    fn test_all_transactions_assigned() {
        let txns: Vec<Transaction> = (0..20)
            .map(|i| make_txn(i, 1.0 + (i as f64) * 0.1, 0.0, None))
            .collect();
        let result = schedule(&txns, 4);
        assert_eq!(result.assignments.len(), 20);
        // Every transaction ID should appear exactly once
        let mut ids: Vec<u64> = result.assignments.iter().map(|a| a.txn_id).collect();
        ids.sort();
        let expected: Vec<u64> = (0..20).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn test_deadline_tracking() {
        let txns = vec![
            make_txn(0, 5.0, 0.0, Some(3.0)),  // Will miss deadline
            make_txn(1, 2.0, 0.0, Some(10.0)),  // Will meet deadline
        ];
        let result = schedule(&txns, 1);
        assert!(result.missed_deadlines >= 1);
    }

    #[test]
    fn test_utilization_bounded() {
        let txns: Vec<Transaction> = (0..10)
            .map(|i| make_txn(i, 2.0, 0.0, None))
            .collect();
        let result = schedule(&txns, 3);
        assert!(result.utilization >= 0.0);
        assert!(result.utilization <= 1.0);
    }

    #[test]
    fn test_arrival_time_respected() {
        let txns = vec![
            make_txn(0, 1.0, 10.0, None),  // Arrives at t=10
            make_txn(1, 1.0, 0.0, None),   // Arrives at t=0
        ];
        let result = schedule(&txns, 1);
        // Transaction 1 arrives first, should start at 0.0
        // Transaction 0 arrives at 10.0, must start at or after 10.0
        for entry in &result.assignments {
            let txn = txns.iter().find(|t| t.id == entry.txn_id).unwrap();
            assert!(
                entry.start_time >= txn.arrival_time - 1e-9,
                "Transaction {} started at {} before arrival {}",
                entry.txn_id,
                entry.start_time,
                txn.arrival_time
            );
        }
    }

    #[test]
    fn test_makespan_quality() {
        // 10 identical jobs on 5 resources -> optimal makespan = 2.0
        let txns: Vec<Transaction> = (0..10)
            .map(|i| make_txn(i, 1.0, 0.0, None))
            .collect();
        let result = schedule(&txns, 5);
        // Makespan should be at most 2.0 for a reasonable scheduler
        assert!(
            result.makespan <= 2.0 + 1e-9,
            "Makespan {} too high for 10 unit jobs on 5 resources",
            result.makespan
        );
    }
}
