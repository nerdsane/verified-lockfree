//! Integration test: Stateright → Oracles → DST
//!
//! This test demonstrates the complete verification flow:
//! 1. Run stateright model checking to validate TLA+ spec
//! 2. Extract oracles (interesting CAS interleavings)
//! 3. Replay oracles in DST to test actual implementation

use vf_dst::{OracleScheduler, OracleTrace};
use vf_stateright::{OracleCategory, OracleExtractor};

/// Simulated stack implementation for DST testing.
///
/// In real usage, this would be the actual lock-free stack.
struct SimulatedStack {
    values: Vec<u64>,
    pushed: std::collections::HashSet<u64>,
    popped: std::collections::HashSet<u64>,
    cas_failures: u64,
}

impl SimulatedStack {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            pushed: std::collections::HashSet::new(),
            popped: std::collections::HashSet::new(),
            cas_failures: 0,
        }
    }

    /// Simulate a push with potential CAS failure.
    fn push(&mut self, value: u64, cas_should_fail: bool) -> bool {
        if cas_should_fail {
            self.cas_failures += 1;
            return false;
        }
        self.values.push(value);
        self.pushed.insert(value);
        true
    }

    /// Simulate a pop with potential CAS failure.
    fn pop(&mut self, cas_should_fail: bool) -> Option<u64> {
        if cas_should_fail {
            self.cas_failures += 1;
            return None;
        }
        self.values.pop().map(|v| {
            self.popped.insert(v);
            v
        })
    }

    /// Check NoLostElements invariant.
    fn check_no_lost_elements(&self) -> bool {
        for &elem in &self.pushed {
            if !self.values.contains(&elem) && !self.popped.contains(&elem) {
                return false;
            }
        }
        true
    }

    /// Check NoDuplicates invariant.
    fn check_no_duplicates(&self) -> bool {
        let unique: std::collections::HashSet<_> = self.values.iter().collect();
        self.values.len() == unique.len()
    }
}

#[test]
fn test_stateright_to_oracles() {
    // Step 1: Run stateright model checking and extract oracles
    let mut extractor = OracleExtractor::new();
    let oracles = extractor.extract(2, vec![1, 2, 3]);

    println!("Extracted {} oracles", oracles.len());

    // Verify we got oracles in multiple categories
    let cas_success = extractor.oracles_by_category(OracleCategory::CasSuccess);
    let concurrent_push = extractor.oracles_by_category(OracleCategory::ConcurrentPush);
    let edge_cases = extractor.oracles_by_category(OracleCategory::EdgeCase);

    assert!(!cas_success.is_empty(), "Should have CasSuccess oracles");
    assert!(!concurrent_push.is_empty(), "Should have ConcurrentPush oracles");
    assert!(!edge_cases.is_empty(), "Should have EdgeCase oracles");

    // Print oracle summary
    for oracle in &oracles {
        println!(
            "  [{:?}] {} - {} actions",
            oracle.category,
            oracle.name,
            oracle.actions.len()
        );
    }
}

#[test]
fn test_oracle_replay_in_dst() {
    // Step 1: Create a concurrent push contention oracle
    let oracle = OracleTrace::concurrent_push_contention(0, 1, 100, 200);
    println!("Oracle: {}", oracle.name);
    println!("Description: {}", oracle.description);
    println!("Steps: {}", oracle.steps.len());

    // Step 2: Create DST scheduler with this oracle
    let mut scheduler = OracleScheduler::from_oracle(oracle.clone(), 2);

    // Step 3: Simulate execution following the oracle
    let mut stack = SimulatedStack::new();
    let mut t0_push_value: Option<u64> = None;
    let mut t1_push_value: Option<u64> = None;
    let mut t0_cas_attempt = 0;
    let mut t1_cas_attempt = 0;

    while !scheduler.oracle_complete() {
        let thread = scheduler.current_thread();
        let expected_action = scheduler.expected_action();
        let expected_value = scheduler.expected_value();

        match expected_action {
            Some(vf_dst::oracle_scheduler::OracleActionType::PushAlloc) => {
                // Thread allocates value for push
                let value = expected_value.expect("PushAlloc needs a value");
                if thread == 0 {
                    t0_push_value = Some(value);
                } else {
                    t1_push_value = Some(value);
                }
                println!("T{}: push_alloc({})", thread, value);
            }
            Some(vf_dst::oracle_scheduler::OracleActionType::PushReadHead) => {
                // Thread reads head (preparing for CAS)
                println!("T{}: push_read_head", thread);
            }
            Some(vf_dst::oracle_scheduler::OracleActionType::PushCas) => {
                // Thread attempts CAS
                let value = if thread == 0 {
                    t0_push_value.expect("T0 should have value")
                } else {
                    t1_push_value.expect("T1 should have value")
                };

                // Simulate CAS failure based on oracle pattern
                // T1's first CAS succeeds, T0's first CAS fails, T0's second succeeds
                let should_fail = if thread == 0 {
                    t0_cas_attempt += 1;
                    t0_cas_attempt == 1 // First attempt fails
                } else {
                    t1_cas_attempt += 1;
                    false // T1 always succeeds in this scenario
                };

                let success = stack.push(value, should_fail);
                println!(
                    "T{}: push_cas({}) = {}",
                    thread,
                    value,
                    if success { "SUCCESS" } else { "FAIL" }
                );
            }
            Some(vf_dst::oracle_scheduler::OracleActionType::PopReadHead) => {
                println!("T{}: pop_read_head", thread);
            }
            Some(vf_dst::oracle_scheduler::OracleActionType::PopCas) => {
                let result = stack.pop(false);
                println!("T{}: pop_cas() = {:?}", thread, result);
            }
            None => break,
        }

        scheduler.advance();
    }

    // Step 4: Verify invariants after oracle execution
    assert!(
        stack.check_no_lost_elements(),
        "NoLostElements invariant violated!"
    );
    assert!(
        stack.check_no_duplicates(),
        "NoDuplicates invariant violated!"
    );

    println!("\nFinal state:");
    println!("  Stack: {:?}", stack.values);
    println!("  Pushed: {:?}", stack.pushed);
    println!("  Popped: {:?}", stack.popped);
    println!("  CAS failures: {}", stack.cas_failures);
    println!("\nScheduler stats: {}", scheduler.stats().format());

    // Verify expected behavior
    assert_eq!(stack.cas_failures, 1, "Expected exactly 1 CAS failure");
    assert_eq!(stack.values.len(), 2, "Both values should be pushed");
    assert!(stack.pushed.contains(&100) && stack.pushed.contains(&200));
}

#[test]
fn test_all_cas_scenarios() {
    // Test all predefined CAS scenarios
    let scenarios = vf_dst::oracle_scheduler::scenarios::all_cas_scenarios();

    println!("Testing {} CAS scenarios:", scenarios.len());

    for scenario in scenarios {
        println!("\n=== {} ===", scenario.name);
        println!("Description: {}", scenario.description);

        // Create scheduler and simulate
        let mut scheduler = OracleScheduler::from_oracle(scenario.clone(), 4);
        let mut stack = SimulatedStack::new();

        // Simple simulation - just verify the oracle runs to completion
        let mut step = 0;
        while !scheduler.oracle_complete() && step < 100 {
            scheduler.advance();
            step += 1;
        }

        let stats = scheduler.stats();
        println!(
            "  Completed: {} (executed {}/{} steps)",
            stats.oracle_complete, stats.oracle_steps_executed, stats.oracle_steps_total
        );
        assert!(stats.oracle_complete, "Oracle should complete");
    }
}

#[test]
fn test_oracle_enriches_random_dst() {
    // Demonstrate how oracles can be combined with random exploration
    let oracle = OracleTrace::concurrent_pop_contention(0, 1);

    // First, replay the oracle exactly
    let mut scheduler = OracleScheduler::new(Some(oracle), 12345, 2);

    let mut oracle_steps = 0;
    let mut random_steps = 0;

    // Execute while oracle drives
    while scheduler.is_following_oracle() {
        let _decision = scheduler.decide();
        scheduler.advance();
        oracle_steps += 1;
    }

    // After oracle completes, scheduler falls back to random
    for _ in 0..10 {
        let _decision = scheduler.decide();
        random_steps += 1;
    }

    let stats = scheduler.stats();
    println!("Oracle steps: {}", oracle_steps);
    println!("Random steps: {}", random_steps);
    println!("Stats: {}", stats.format());

    assert!(stats.oracle_complete);
    assert_eq!(stats.fallback_decisions, 10);
}
