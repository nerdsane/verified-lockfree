# Expert-Parallel Load Balancing: Problem Statement

## What It Does

An expert-parallel load balancer assigns experts to GPUs in a Mixture-of-Experts (MoE) neural network inference system. In MoE architectures, each input token is routed to a small subset (top-k) of a large pool of experts, and the selected experts process the token independently. The experts are distributed across multiple GPUs, and each GPU processes all tokens routed to the experts it hosts. If one GPU receives far more tokens than another, that GPU becomes the bottleneck and all other GPUs sit idle waiting for it.

The load balancer's job is to decide which GPU hosts which expert, given knowledge of how many tokens each expert is expected to receive. The objective is to minimize the maximum load across all GPUs -- that is, to make the busiest GPU as lightly loaded as possible. This is a bin-packing problem with the additional constraint that each GPU has a maximum token capacity, and every token must be routed to at least one expert. The quality of the assignment directly determines the inference throughput of the entire system: a perfect balance means every GPU finishes at the same time, while a poor balance means the slowest GPU dictates the throughput of the whole batch.

## Safety Properties

1. **Complete assignment**: Every expert is assigned to exactly one GPU. No expert is left unassigned.
2. **No replication (base case)**: Each expert appears on exactly one GPU. No expert is duplicated across multiple GPUs.
3. **GPU capacity respected**: The total number of tokens routed to experts on any single GPU does not exceed `capacity_factor * (total_tokens / num_gpus)`.
4. **Routing completeness**: Every token in the batch is routed to at least one expert (determined by the routing function, not the balancer, but the balancer must not create an assignment that makes valid routing impossible).
5. **Assignment stability**: The assignment function is deterministic for a given input. The same token counts and GPU configuration produce the same assignment.

## Liveness Properties

1. **Termination**: The balancer produces a valid assignment in finite time for any valid input configuration.
2. **Feasibility**: For any input where a feasible assignment exists (total expert memory fits within total GPU memory, total tokens within total GPU capacity), the balancer finds one.
3. **Convergence under evolution**: As the balancing algorithm is evolved, the maximum load imbalance monotonically improves across generations.

## Performance Dimensions

- Load imbalance ratio: max GPU load divided by average GPU load (1.0 is perfect; lower is better).
- Absolute maximum GPU load: the token count on the most loaded GPU.
- Communication cost: estimated cross-GPU token transfer volume if tokens must move between the GPU that generated them and the GPU hosting their target expert.
- Memory violations: number of GPUs whose assigned experts exceed the GPU's memory capacity.
- Algorithm runtime: wall-clock time to compute the assignment for 8, 64, and 128 experts across 4 to 16 GPUs.
- Robustness to skewed distributions: load imbalance under Zipf-distributed token-to-expert routing versus uniform routing.

## What Is NOT Specified

- The assignment algorithm (round-robin, greedy bin-packing, integer programming, graph partitioning, simulated annealing).
- Whether expert replication across GPUs is allowed as an advanced optimization.
- The token routing function (top-1, top-2, top-k with auxiliary loss) -- the balancer receives token counts as input, not raw routing logits.
- Whether the assignment is computed once per model or recomputed per batch based on observed routing statistics.
- The communication topology between GPUs (all-to-all, ring, hierarchical).
- How the balancer handles heterogeneous GPUs with different compute capacities or memory sizes.
