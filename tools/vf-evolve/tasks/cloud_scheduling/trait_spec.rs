/// Trait specification tests for Cloud Scheduling.
///
/// The evolved code must provide:
///   pub struct Job { id, cpu_cores, memory_gb, duration_hours, deadline_hours, priority, arrival_time }
///   pub struct InstanceType { id, cpu_cores, memory_gb, cost_per_hour, available_count }
///   pub struct SchedulingResult { schedule, total_cost, missed_deadlines, avg_wait_time }
///   pub fn schedule_jobs(jobs: &[Job], instance_types: &[InstanceType]) -> SchedulingResult

#[cfg(test)]
mod tests {
    use super::*;

    fn small_instance() -> InstanceType {
        InstanceType {
            id: 0,
            cpu_cores: 2,
            memory_gb: 4.0,
            cost_per_hour: 0.10,
            available_count: 5,
        }
    }

    fn large_instance() -> InstanceType {
        InstanceType {
            id: 1,
            cpu_cores: 8,
            memory_gb: 32.0,
            cost_per_hour: 0.80,
            available_count: 2,
        }
    }

    fn make_job(id: u64, cores: u64, duration: f64, deadline: f64) -> Job {
        Job {
            id,
            cpu_cores: cores,
            memory_gb: 2.0,
            duration_hours: duration,
            deadline_hours: deadline,
            priority: 0,
            arrival_time: 0.0,
        }
    }

    #[test]
    fn test_all_jobs_scheduled() {
        let jobs: Vec<Job> = (0..5).map(|i| make_job(i, 1, 1.0, 10.0)).collect();
        let instances = vec![small_instance()];
        let result = schedule_jobs(&jobs, &instances);
        assert_eq!(result.schedule.len(), 5);
    }

    #[test]
    fn test_cheapest_instance_preferred() {
        let jobs = vec![make_job(0, 1, 1.0, 10.0)];
        let instances = vec![small_instance(), large_instance()];
        let result = schedule_jobs(&jobs, &instances);
        assert_eq!(result.schedule.len(), 1);
        // Should use the cheap instance
        assert_eq!(result.schedule[0].instance_type_id, 0);
        assert!((result.total_cost - 0.10).abs() < 1e-9);
    }

    #[test]
    fn test_large_job_needs_large_instance() {
        let jobs = vec![make_job(0, 4, 2.0, 10.0)]; // Needs 4 cores
        let instances = vec![small_instance(), large_instance()]; // Small only has 2
        let result = schedule_jobs(&jobs, &instances);
        assert_eq!(result.schedule.len(), 1);
        assert_eq!(result.schedule[0].instance_type_id, 1); // Must use large
    }

    #[test]
    fn test_deadline_violations_counted() {
        let jobs = vec![
            make_job(0, 1, 5.0, 3.0), // Duration > deadline
        ];
        let instances = vec![small_instance()];
        let result = schedule_jobs(&jobs, &instances);
        assert_eq!(result.missed_deadlines, 1);
    }

    #[test]
    fn test_total_cost_nonnegative() {
        let jobs: Vec<Job> = (0..10).map(|i| make_job(i, 1, 1.0, 10.0)).collect();
        let instances = vec![small_instance(), large_instance()];
        let result = schedule_jobs(&jobs, &instances);
        assert!(result.total_cost >= 0.0);
    }

    #[test]
    fn test_cost_quality() {
        // 5 small jobs -> should all use cheap instance = 5 * 0.10 = $0.50
        let jobs: Vec<Job> = (0..5).map(|i| make_job(i, 1, 1.0, 100.0)).collect();
        let instances = vec![small_instance(), large_instance()];
        let result = schedule_jobs(&jobs, &instances);
        assert!(
            result.total_cost <= 1.0,
            "Cost {} too high for 5 small jobs",
            result.total_cost
        );
    }

    #[test]
    fn test_no_time_travel() {
        let jobs = vec![
            Job {
                id: 0,
                cpu_cores: 1,
                memory_gb: 1.0,
                duration_hours: 1.0,
                deadline_hours: 10.0,
                priority: 0,
                arrival_time: 5.0, // Arrives at hour 5
            },
        ];
        let instances = vec![small_instance()];
        let result = schedule_jobs(&jobs, &instances);
        assert!(
            result.schedule[0].start_time >= 5.0 - 1e-9,
            "Job started before arrival"
        );
    }
}
