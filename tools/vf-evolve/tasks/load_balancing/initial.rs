/// Load Balancing (MoE Expert-to-GPU Assignment) - Initial Seed (Round-Robin)
/// ShinkaEvolve will evolve this toward optimal load balancing policies.
///
/// Round-robin assignment of experts to GPUs.

/// An expert with compute requirements.
#[derive(Debug, Clone)]
pub struct Expert {
    pub id: u64,
    pub compute_cost: f64,
    pub memory_bytes: u64,
    pub activation_frequency: f64,
}

/// Expert-to-GPU assignment result.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub expert_id: u64,
    pub gpu_id: u64,
}

/// Result of load balancing.
#[derive(Debug, Clone)]
pub struct BalancingResult {
    pub assignments: Vec<Assignment>,
    pub max_gpu_load: f64,
    pub load_imbalance: f64,
    pub memory_violations: u64,
}

/// Assign experts to `num_gpus` GPUs, each with given compute and memory capacity.
///
/// Returns assignments minimizing max GPU load imbalance.
/// `gpu_memory_bytes` is the memory capacity per GPU.
pub fn balance_load(
    experts: &[Expert],
    num_gpus: u64,
    gpu_memory_bytes: u64,
) -> BalancingResult {
    debug_assert!(num_gpus > 0, "Must have at least one GPU");
    debug_assert!(!experts.is_empty(), "Must have at least one expert");

    let mut gpu_loads = vec![0.0_f64; num_gpus as usize];
    let mut gpu_memory = vec![0_u64; num_gpus as usize];
    let mut assignments = Vec::with_capacity(experts.len());

    // Simple round-robin assignment
    for (i, expert) in experts.iter().enumerate() {
        let gpu_idx = (i as u64) % num_gpus;
        assignments.push(Assignment {
            expert_id: expert.id,
            gpu_id: gpu_idx,
        });
        gpu_loads[gpu_idx as usize] += expert.compute_cost * expert.activation_frequency;
        gpu_memory[gpu_idx as usize] += expert.memory_bytes;
    }

    let max_gpu_load = gpu_loads.iter().copied().fold(0.0_f64, f64::max);
    let avg_load: f64 = gpu_loads.iter().sum::<f64>() / num_gpus as f64;
    let load_imbalance = if avg_load > 0.0 {
        max_gpu_load / avg_load
    } else {
        1.0
    };

    let memory_violations = gpu_memory
        .iter()
        .filter(|&&mem| mem > gpu_memory_bytes)
        .count() as u64;

    BalancingResult {
        assignments,
        max_gpu_load,
        load_imbalance,
        memory_violations,
    }
}
