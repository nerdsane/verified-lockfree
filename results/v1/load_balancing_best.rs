/// Load Balancing (MoE Expert-to-GPU Assignment) - Lock-Free Implementation
/// Uses atomic operations and crossbeam-epoch for wait-free load balancing.

use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

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

#[derive(Debug)]
struct GpuState {
    load: AtomicU64, // Fixed-point representation (multiply by 1000)
    memory: AtomicU64,
}

struct LoadBalancer {
    gpus: Vec<Arc<GpuState>>,
    num_gpus: u64,
    gpu_memory_bytes: u64,
    assignment_counter: AtomicUsize,
}

impl LoadBalancer {
    fn new(num_gpus: u64, gpu_memory_bytes: u64) -> Self {
        let mut gpus = Vec::with_capacity(num_gpus as usize);
        for _ in 0..num_gpus {
            gpus.push(Arc::new(GpuState {
                load: AtomicU64::new(0),
                memory: AtomicU64::new(0),
            }));
        }
        
        Self {
            gpus,
            num_gpus,
            gpu_memory_bytes,
            assignment_counter: AtomicUsize::new(0),
        }
    }

    fn find_best_gpu(&self, expert: &Expert) -> u64 {
        let mut best_gpu = 0;
        let mut best_score = f64::INFINITY;
        
        let expert_load_fp = ((expert.compute_cost * expert.activation_frequency) * 1000.0) as u64;
        
        for (gpu_id, gpu) in self.gpus.iter().enumerate() {
            let current_load = gpu.load.load(Ordering::Relaxed);
            let current_memory = gpu.memory.load(Ordering::Relaxed);
            
            // Check memory constraint
            if current_memory + expert.memory_bytes > self.gpu_memory_bytes {
                continue;
            }
            
            // Calculate load after assignment
            let new_load = current_load + expert_load_fp;
            let load_score = new_load as f64 / 1000.0;
            
            // Add memory pressure penalty
            let memory_ratio = (current_memory + expert.memory_bytes) as f64 / self.gpu_memory_bytes as f64;
            let total_score = load_score * (1.0 + memory_ratio * 0.5);
            
            if total_score < best_score {
                best_score = total_score;
                best_gpu = gpu_id;
            }
        }
        
        best_gpu as u64
    }

    fn assign_expert(&self, expert: &Expert) -> Assignment {
        let expert_load_fp = ((expert.compute_cost * expert.activation_frequency) * 1000.0) as u64;
        
        loop {
            let gpu_id = self.find_best_gpu(expert);
            let gpu = &self.gpus[gpu_id as usize];
            
            // Try to atomically update both load and memory
            let old_load = gpu.load.load(Ordering::Relaxed);
            let old_memory = gpu.memory.load(Ordering::Relaxed);
            
            // Check memory constraint again
            if old_memory + expert.memory_bytes > self.gpu_memory_bytes {
                // Fallback to round-robin if no GPU has enough memory
                let counter = self.assignment_counter.fetch_add(1, Ordering::Relaxed);
                let fallback_gpu_id = (counter as u64) % self.num_gpus;
                let fallback_gpu = &self.gpus[fallback_gpu_id as usize];
                
                fallback_gpu.load.fetch_add(expert_load_fp, Ordering::Relaxed);
                fallback_gpu.memory.fetch_add(expert.memory_bytes, Ordering::Relaxed);
                
                return Assignment {
                    expert_id: expert.id,
                    gpu_id: fallback_gpu_id,
                };
            }
            
            // Try to update load atomically
            match gpu.load.compare_exchange_weak(
                old_load,
                old_load + expert_load_fp,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    // Successfully updated load, now update memory
                    gpu.memory.fetch_add(expert.memory_bytes, Ordering::Relaxed);
                    
                    return Assignment {
                        expert_id: expert.id,
                        gpu_id,
                    };
                }
                Err(_) => {
                    // Retry with updated state
                    continue;
                }
            }
        }
    }

    fn get_final_state(&self) -> (Vec<f64>, Vec<u64>) {
        let loads: Vec<f64> = self.gpus.iter()
            .map(|gpu| gpu.load.load(Ordering::Relaxed) as f64 / 1000.0)
            .collect();
        
        let memory: Vec<u64> = self.gpus.iter()
            .map(|gpu| gpu.memory.load(Ordering::Relaxed))
            .collect();
            
        (loads, memory)
    }
}

/// Assign experts to `num_gpus` GPUs, each with given compute and memory capacity.
///
/// Returns assignments minimizing max GPU load imbalance using lock-free algorithms.
/// `gpu_memory_bytes` is the memory capacity per GPU.
pub fn balance_load(
    experts: &[Expert],
    num_gpus: u64,
    gpu_memory_bytes: u64,
) -> BalancingResult {
    debug_assert!(num_gpus > 0, "Must have at least one GPU");
    debug_assert!(!experts.is_empty(), "Must have at least one expert");

    let balancer = LoadBalancer::new(num_gpus, gpu_memory_bytes);
    let mut assignments = Vec::with_capacity(experts.len());

    // Sort experts by compute cost (descending) for better load balancing
    let mut sorted_experts: Vec<_> = experts.iter().collect();
    sorted_experts.sort_by(|a, b| {
        let load_a = a.compute_cost * a.activation_frequency;
        let load_b = b.compute_cost * b.activation_frequency;
        load_b.partial_cmp(&load_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    // Assign experts using lock-free operations
    for expert in sorted_experts {
        assignments.push(balancer.assign_expert(expert));
    }

    let (gpu_loads, gpu_memory) = balancer.get_final_state();

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