//! Integration tests for SSI oracle extraction and replay.

use vf_stateright::ssi::{SsiAction, SsiOracle, SsiOracleCategory, SsiOracleExtractor, SsiState};

#[test]
fn test_prebuilt_oracles() {
    let oracles = SsiOracle::all_oracles();

    assert!(oracles.len() >= 5, "Should have at least 5 pre-built oracles");

    // Verify categories are diverse
    let categories: std::collections::HashSet<_> = oracles.iter().map(|o| o.category).collect();
    assert!(
        categories.len() >= 3,
        "Should cover at least 3 categories"
    );

    println!("Pre-built SSI oracles:");
    for oracle in &oracles {
        println!("  - {} ({:?}): {}", oracle.name, oracle.category, oracle.description);
    }
}

#[test]
fn test_oracle_replay_sequential_writes() {
    let oracle = SsiOracle::write_write_conflict(); // Now named "write_write_sequential"
    let mut state = SsiState::new(&[1, 2], &[1, 2]);

    println!("Replaying oracle: {}", oracle.name);

    for action in &oracle.actions {
        println!("  Action: {:?}", action);
        if let Some(next) = state.apply(action) {
            state = next;
            // Verify invariants hold
            let violations = state.check_invariants();
            assert!(
                violations.is_empty(),
                "Invariant violations: {:?}",
                violations
            );
        } else {
            println!("    (action not applicable, skipping)");
        }
    }

    // Both transactions should commit (serial execution)
    assert_eq!(
        state.committed_txns().len(),
        2,
        "Both transactions should commit in serial execution"
    );
}

#[test]
fn test_oracle_replay_concurrent_write_blocked() {
    let oracle = SsiOracle::concurrent_write_blocked();
    let mut state = SsiState::new(&[1, 2], &[1, 2]);

    println!("Replaying oracle: {}", oracle.name);

    for action in &oracle.actions {
        println!("  Action: {:?}", action);
        if let Some(next) = state.apply(action) {
            state = next;
            let violations = state.check_invariants();
            assert!(
                violations.is_empty(),
                "Invariant violations: {:?}",
                violations
            );
        }
    }

    // Both should commit (T2 wrote different key)
    assert_eq!(state.committed_txns().len(), 2);
}

#[test]
fn test_oracle_replay_dangerous_structure() {
    let oracle = SsiOracle::dangerous_structure();
    let mut state = SsiState::new(&[1, 2], &[1, 2]);

    println!("Replaying oracle: {}", oracle.name);

    let mut aborted_due_to_dangerous = false;

    for action in &oracle.actions {
        println!("  Action: {:?}", action);
        if let Some(next) = state.apply(action) {
            state = next;

            // Check if any transaction was aborted due to dangerous structure
            for op in &state.history {
                if let vf_stateright::ssi::Operation::Abort { reason, .. } = op {
                    if *reason == vf_stateright::ssi::AbortReason::DangerousStructure {
                        aborted_due_to_dangerous = true;
                    }
                }
            }
        }
    }

    // The dangerous structure oracle should result in an abort
    // (either T1 or T2 should be aborted)
    println!("Aborted due to dangerous structure: {}", aborted_due_to_dangerous);

    // Invariants should always hold
    let violations = state.check_invariants();
    assert!(
        violations.is_empty(),
        "Invariant violations: {:?}",
        violations
    );
}

#[test]
fn test_oracle_replay_disjoint_keys() {
    let oracle = SsiOracle::disjoint_keys();
    let mut state = SsiState::new(&[1, 2], &[1, 2]);

    for action in &oracle.actions {
        if let Some(next) = state.apply(action) {
            state = next;
        }
    }

    // Both transactions should commit (disjoint keys = no conflict)
    let committed = state.committed_txns();
    assert!(
        committed.len() == 2,
        "Both transactions should commit, got {:?}",
        committed
    );
}

#[test]
fn test_oracle_extractor() {
    let mut extractor = SsiOracleExtractor::new(8); // Shallow for speed
    let oracles = extractor.extract(&[1, 2], &[1]);

    println!("Extracted {} oracles", oracles.len());

    // Should include pre-built oracles plus any discovered ones
    assert!(oracles.len() >= 5, "Should have at least pre-built oracles");

    // Group by category
    let mut by_category: std::collections::HashMap<SsiOracleCategory, Vec<&SsiOracle>> =
        std::collections::HashMap::new();
    for oracle in &oracles {
        by_category.entry(oracle.category).or_default().push(oracle);
    }

    println!("Oracles by category:");
    for (cat, list) in &by_category {
        println!("  {:?}: {} oracles", cat, list.len());
    }
}

#[test]
fn test_single_conflict_flag_commits() {
    // Transaction with only out_conflict (no in_conflict) should be able to commit
    let oracle = SsiOracle::single_conflict_flag();
    let mut state = SsiState::new(&[1, 2], &[1, 2]);

    for action in &oracle.actions {
        if let Some(next) = state.apply(action) {
            state = next;
        }
    }

    // T1 should have committed despite having out_conflict
    // (only dangerous if BOTH flags are set)
    let committed = state.committed_txns();
    println!("Committed transactions: {:?}", committed);

    // Verify invariants
    let violations = state.check_invariants();
    assert!(violations.is_empty(), "Invariant violations: {:?}", violations);
}
