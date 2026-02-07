"""
Load Balancing (MoE Expert-to-GPU Assignment) - Initial Seed (Round-Robin)
ShinkaEvolve will evolve this toward optimal load balancing policies.

Round-robin assignment of experts to GPUs.
"""

from dataclasses import dataclass, field
from typing import List, Dict, Optional


@dataclass
class Expert:
    """A Mixture-of-Experts expert with compute requirements."""
    id: int
    compute_cost: float      # FLOPS required
    memory_bytes: int        # Memory footprint in bytes
    activation_frequency: float = 1.0  # How often this expert is activated [0, 1]


@dataclass
class GPU:
    """A GPU with compute and memory capacity."""
    id: int
    compute_capacity: float  # FLOPS available per step
    memory_capacity: int     # Memory in bytes
    current_load: float = 0.0
    current_memory: int = 0

    @property
    def available_compute(self) -> float:
        return self.compute_capacity - self.current_load

    @property
    def available_memory(self) -> int:
        return self.memory_capacity - self.current_memory


@dataclass
class Assignment:
    """An expert-to-GPU assignment."""
    expert_id: int
    gpu_id: int


@dataclass
class BalancingResult:
    """Result of load balancing."""
    assignments: List[Assignment]
    max_gpu_load: float          # Highest load on any GPU
    load_imbalance: float        # Ratio of max_load / avg_load (1.0 = perfect)
    memory_violations: int       # GPUs exceeding memory capacity
    total_communication: float   # Cross-GPU communication cost estimate


class RoundRobinBalancer:
    """Simple round-robin expert assignment to GPUs."""

    def __init__(self, gpus: List[GPU]):
        assert len(gpus) > 0, "Must have at least one GPU"
        self.gpus = gpus

    def assign(self, experts: List[Expert]) -> BalancingResult:
        """Assign experts to GPUs in round-robin order."""
        assignments = []
        # Reset GPU state
        gpu_loads: Dict[int, float] = {g.id: 0.0 for g in self.gpus}
        gpu_memory: Dict[int, int] = {g.id: 0 for g in self.gpus}
        memory_violations = 0

        for i, expert in enumerate(experts):
            gpu = self.gpus[i % len(self.gpus)]
            assignments.append(Assignment(expert_id=expert.id, gpu_id=gpu.id))
            gpu_loads[gpu.id] += expert.compute_cost * expert.activation_frequency
            gpu_memory[gpu.id] += expert.memory_bytes

        # Check memory violations
        for gpu in self.gpus:
            if gpu_memory[gpu.id] > gpu.memory_capacity:
                memory_violations += 1

        # Compute load statistics
        loads = list(gpu_loads.values())
        max_load = max(loads) if loads else 0.0
        avg_load = sum(loads) / len(loads) if loads else 1.0
        load_imbalance = max_load / avg_load if avg_load > 0 else 1.0

        # Estimate communication cost (experts on different GPUs that interact)
        # Simple model: communication proportional to number of GPUs used
        gpus_used = len(set(a.gpu_id for a in assignments))
        total_communication = float(gpus_used - 1) * len(experts) if gpus_used > 1 else 0.0

        return BalancingResult(
            assignments=assignments,
            max_gpu_load=max_load,
            load_imbalance=load_imbalance,
            memory_violations=memory_violations,
            total_communication=total_communication,
        )


def balance_load(token_counts, num_gpus):
    """Adapter for ShinkaEvolve simulator interface.

    Args:
        token_counts: list of token counts per expert.
        num_gpus: number of GPUs.

    Returns:
        list of GPU indices (one per expert).
    """
    return [i % num_gpus for i in range(len(token_counts))]


if __name__ == "__main__":
    gpus = [
        GPU(id=0, compute_capacity=100.0, memory_capacity=16_000_000_000),
        GPU(id=1, compute_capacity=100.0, memory_capacity=16_000_000_000),
        GPU(id=2, compute_capacity=100.0, memory_capacity=16_000_000_000),
        GPU(id=3, compute_capacity=100.0, memory_capacity=16_000_000_000),
    ]

    experts = [
        Expert(id=i, compute_cost=10.0 + (i % 5) * 5.0,
               memory_bytes=1_000_000_000, activation_frequency=0.3 + (i % 3) * 0.2)
        for i in range(16)
    ]

    balancer = RoundRobinBalancer(gpus)
    result = balancer.assign(experts)
    print(f"Max GPU load: {result.max_gpu_load:.1f}")
    print(f"Load imbalance: {result.load_imbalance:.2f}x")
    print(f"Memory violations: {result.memory_violations}")
    print(f"Communication cost: {result.total_communication:.1f}")
    for a in result.assignments:
        print(f"  Expert {a.expert_id} -> GPU {a.gpu_id}")
