//! ADRS (Algorithmic Domain Rule Simulators) — Rust-native domain simulators.
//!
//! Each simulator evaluates a candidate solution against domain-specific criteria
//! and returns a score in [0, 1000]. Higher is better.
//!
//! Simulators:
//! - `TxnSchedulingSim` — minimizes transaction makespan
//! - `TcpCongestionSim` — maximizes throughput/latency ratio
//! - `LoadBalancingSim` — minimizes max GPU load imbalance
//! - `CloudSchedulingSim` — minimizes cost under SLA deadlines
//! - `LlmSqlCacheSim` — maximizes cache hit rate

use std::time::Duration;

use crate::result::EvaluatorResult;

/// Score range: [0.0, 1000.0]. Higher is better.
pub type SimScore = f64;

/// Result of an ADRS simulation run.
#[derive(Debug, Clone)]
pub struct AdrsResult {
    pub score: SimScore,
    pub details: String,
    pub duration: Duration,
}

// ---------------------------------------------------------------------------
// Transaction Scheduling Simulator
// ---------------------------------------------------------------------------

/// Evaluates a transaction scheduling solution.
/// Score = 1000 * (1 - normalized_makespan) + deadline_bonus.
pub fn score_txn_scheduling(
    makespan: f64,
    optimal_makespan: f64,
    missed_deadlines: u64,
    total_txns: u64,
    utilization: f64,
) -> SimScore {
    debug_assert!(optimal_makespan > 0.0);
    debug_assert!(total_txns > 0);

    let makespan_ratio = (makespan / optimal_makespan).max(1.0);
    let makespan_score = (1.0 / makespan_ratio) * 500.0;

    let deadline_ratio = if total_txns > 0 {
        1.0 - (missed_deadlines as f64 / total_txns as f64)
    } else {
        1.0
    };
    let deadline_score = deadline_ratio * 300.0;

    let utilization_score = utilization * 200.0;

    (makespan_score + deadline_score + utilization_score).min(1000.0)
}

// ---------------------------------------------------------------------------
// TCP Congestion Control Simulator
// ---------------------------------------------------------------------------

/// Evaluates a congestion control solution.
/// Score based on throughput, goodput ratio, and loss avoidance.
pub fn score_tcp_congestion(
    throughput: f64,
    max_throughput: f64,
    goodput_ratio: f64,
    total_losses: u64,
    total_events: u64,
) -> SimScore {
    debug_assert!(max_throughput > 0.0);

    let throughput_ratio = (throughput / max_throughput).min(1.0);
    let throughput_score = throughput_ratio * 400.0;

    let goodput_score = goodput_ratio * 400.0;

    let loss_ratio = if total_events > 0 {
        1.0 - (total_losses as f64 / total_events as f64).min(1.0)
    } else {
        1.0
    };
    let loss_score = loss_ratio * 200.0;

    (throughput_score + goodput_score + loss_score).min(1000.0)
}

// ---------------------------------------------------------------------------
// Load Balancing Simulator
// ---------------------------------------------------------------------------

/// Evaluates a load balancing solution.
/// Score based on imbalance minimization and memory constraint satisfaction.
pub fn score_load_balancing(
    load_imbalance: f64,
    memory_violations: u64,
    total_gpus: u64,
) -> SimScore {
    debug_assert!(load_imbalance >= 1.0);

    // Perfect imbalance = 1.0 (perfectly balanced)
    let imbalance_score = (1.0 / load_imbalance) * 700.0;

    let violation_penalty = if total_gpus > 0 {
        (memory_violations as f64 / total_gpus as f64).min(1.0) * 300.0
    } else {
        0.0
    };

    (imbalance_score + 300.0 - violation_penalty).max(0.0).min(1000.0)
}

// ---------------------------------------------------------------------------
// Cloud Scheduling Simulator
// ---------------------------------------------------------------------------

/// Evaluates a cloud scheduling solution.
/// Score based on cost efficiency, deadline compliance, and wait time.
pub fn score_cloud_scheduling(
    total_cost: f64,
    optimal_cost: f64,
    missed_deadlines: u64,
    total_jobs: u64,
    avg_wait_time: f64,
    max_acceptable_wait: f64,
) -> SimScore {
    debug_assert!(optimal_cost > 0.0);
    debug_assert!(total_jobs > 0);

    let cost_ratio = (optimal_cost / total_cost.max(optimal_cost)).min(1.0);
    let cost_score = cost_ratio * 400.0;

    let deadline_ratio = 1.0 - (missed_deadlines as f64 / total_jobs as f64);
    let deadline_score = deadline_ratio * 400.0;

    let wait_ratio = if max_acceptable_wait > 0.0 {
        (1.0 - avg_wait_time / max_acceptable_wait).max(0.0)
    } else {
        1.0
    };
    let wait_score = wait_ratio * 200.0;

    (cost_score + deadline_score + wait_score).min(1000.0)
}

// ---------------------------------------------------------------------------
// LLM SQL Cache Simulator
// ---------------------------------------------------------------------------

/// Evaluates a cache strategy solution.
/// Score based on hit rate and efficient capacity usage.
pub fn score_llm_sql_cache(
    hit_rate: f64,
    evictions: u64,
    total_queries: u64,
    cache_size: u64,
    capacity: u64,
) -> SimScore {
    let hit_score = hit_rate * 700.0;

    let efficiency = if capacity > 0 {
        (cache_size as f64 / capacity as f64).min(1.0)
    } else {
        0.0
    };
    let efficiency_score = efficiency * 100.0;

    let eviction_ratio = if total_queries > 0 {
        1.0 - (evictions as f64 / total_queries as f64).min(1.0)
    } else {
        1.0
    };
    let eviction_score = eviction_ratio * 200.0;

    (hit_score + efficiency_score + eviction_score).min(1000.0)
}

// ---------------------------------------------------------------------------
// Cascade integration
// ---------------------------------------------------------------------------

/// Convert an ADRS simulation result into an EvaluatorResult for cascade scoring.
pub fn adrs_to_evaluator_result(
    simulator_name: &str,
    adrs_result: &AdrsResult,
    pass_threshold: SimScore,
) -> EvaluatorResult {
    if adrs_result.score >= pass_threshold {
        EvaluatorResult::pass_with_output(
            simulator_name,
            adrs_result.duration,
            format!("Score: {:.1}/1000 — {}", adrs_result.score, adrs_result.details),
        )
    } else {
        EvaluatorResult::fail(
            simulator_name,
            format!(
                "Score {:.1} below threshold {:.1}: {}",
                adrs_result.score, pass_threshold, adrs_result.details
            ),
            adrs_result.duration,
            adrs_result.details.clone(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_txn_scheduling_perfect() {
        let score = score_txn_scheduling(10.0, 10.0, 0, 10, 1.0);
        assert!((score - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn test_txn_scheduling_poor_makespan() {
        let score = score_txn_scheduling(100.0, 10.0, 5, 10, 0.3);
        assert!(score < 500.0);
    }

    #[test]
    fn test_tcp_congestion_perfect() {
        let score = score_tcp_congestion(100.0, 100.0, 1.0, 0, 100);
        assert!((score - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn test_load_balancing_perfect() {
        let score = score_load_balancing(1.0, 0, 4);
        assert!((score - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn test_load_balancing_high_imbalance() {
        let score = score_load_balancing(5.0, 2, 4);
        assert!(score < 500.0);
    }

    #[test]
    fn test_cloud_scheduling_perfect() {
        let score = score_cloud_scheduling(10.0, 10.0, 0, 10, 0.0, 5.0);
        assert!((score - 1000.0).abs() < 1e-9);
    }

    #[test]
    fn test_llm_cache_high_hit_rate() {
        let score = score_llm_sql_cache(0.9, 5, 100, 50, 50);
        assert!(score > 700.0);
    }

    #[test]
    fn test_scores_bounded() {
        // All scores should be in [0, 1000]
        let s1 = score_txn_scheduling(1.0, 100.0, 0, 1, 1.0);
        let s2 = score_tcp_congestion(1000.0, 100.0, 1.0, 0, 100);
        let s3 = score_load_balancing(1.0, 0, 1);
        let s4 = score_cloud_scheduling(0.01, 100.0, 0, 1, 0.0, 1.0);
        let s5 = score_llm_sql_cache(1.0, 0, 100, 100, 100);

        for (name, s) in [("txn", s1), ("tcp", s2), ("lb", s3), ("cloud", s4), ("cache", s5)] {
            assert!(
                s >= 0.0 && s <= 1000.0,
                "{} score {} out of bounds",
                name,
                s
            );
        }
    }
}
