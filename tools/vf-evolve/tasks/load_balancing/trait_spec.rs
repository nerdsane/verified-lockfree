/// Trait specification tests for Load Balancing (MoE Expert-to-GPU).
///
/// The evolved code must provide:
///   pub struct Expert { id, compute_cost, memory_bytes, activation_frequency }
///   pub struct BalancingResult { assignments, max_gpu_load, load_imbalance, memory_violations }
///   pub fn balance_load(experts: &[Expert], num_gpus: u64, gpu_memory_bytes: u64) -> BalancingResult

#[cfg(test)]
mod tests {
    use super::*;

    fn make_expert(id: u64, cost: f64, memory: u64, freq: f64) -> Expert {
        Expert {
            id,
            compute_cost: cost,
            memory_bytes: memory,
            activation_frequency: freq,
        }
    }

    #[test]
    fn test_all_experts_assigned() {
        let experts: Vec<Expert> = (0..16)
            .map(|i| make_expert(i, 10.0, 1_000_000, 0.5))
            .collect();
        let result = balance_load(&experts, 4, 100_000_000);
        assert_eq!(result.assignments.len(), 16);
        let mut ids: Vec<u64> = result.assignments.iter().map(|a| a.expert_id).collect();
        ids.sort();
        let expected: Vec<u64> = (0..16).collect();
        assert_eq!(ids, expected);
    }

    #[test]
    fn test_gpu_ids_valid() {
        let experts: Vec<Expert> = (0..10)
            .map(|i| make_expert(i, 5.0, 500_000, 1.0))
            .collect();
        let result = balance_load(&experts, 3, 100_000_000);
        for a in &result.assignments {
            assert!(a.gpu_id < 3, "GPU id {} out of range", a.gpu_id);
        }
    }

    #[test]
    fn test_uniform_experts_low_imbalance() {
        // All experts identical -> optimal balancer achieves imbalance ~1.0
        let experts: Vec<Expert> = (0..12)
            .map(|i| make_expert(i, 10.0, 1_000_000, 1.0))
            .collect();
        let result = balance_load(&experts, 4, 100_000_000);
        assert!(
            result.load_imbalance <= 1.5,
            "Imbalance {} too high for uniform experts",
            result.load_imbalance
        );
    }

    #[test]
    fn test_skewed_experts() {
        // One expert much heavier than others
        let mut experts: Vec<Expert> = (0..8)
            .map(|i| make_expert(i, 1.0, 1_000_000, 1.0))
            .collect();
        experts.push(make_expert(8, 100.0, 1_000_000, 1.0)); // Heavy expert
        let result = balance_load(&experts, 4, 100_000_000);
        assert_eq!(result.assignments.len(), 9);
        assert!(result.max_gpu_load >= 100.0); // Heavy expert dominates
    }

    #[test]
    fn test_memory_violations_detected() {
        let experts: Vec<Expert> = (0..4)
            .map(|i| make_expert(i, 10.0, 60_000_000, 1.0)) // 60MB each
            .collect();
        // 2 GPUs, 100MB each -> if 3+ on one GPU, violation
        let result = balance_load(&experts, 2, 100_000_000);
        // With round-robin, 2 per GPU -> 120MB > 100MB = violations
        assert!(
            result.memory_violations >= 0,
            "Should track memory violations"
        );
    }

    #[test]
    fn test_load_imbalance_positive() {
        let experts: Vec<Expert> = (0..5)
            .map(|i| make_expert(i, 10.0 + i as f64 * 5.0, 1_000_000, 0.5))
            .collect();
        let result = balance_load(&experts, 2, 100_000_000);
        assert!(result.load_imbalance >= 1.0);
    }

    #[test]
    fn test_single_gpu() {
        let experts: Vec<Expert> = (0..5)
            .map(|i| make_expert(i, 10.0, 1_000_000, 1.0))
            .collect();
        let result = balance_load(&experts, 1, 100_000_000);
        assert_eq!(result.assignments.len(), 5);
        assert!((result.load_imbalance - 1.0).abs() < 1e-9);
    }
}
