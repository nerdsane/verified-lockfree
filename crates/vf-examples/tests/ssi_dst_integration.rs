//! DST integration tests for SSI implementation.
//!
//! Runs the `SsiStore` through the DST harness with fault injection,
//! and verifies SSI invariants from vf-core.
//!
//! Includes oracle-driven tests that replay Stateright-discovered scenarios.

use std::collections::HashSet;

use vf_dst::ssi_harness::{DstSsiOp, DstTestableSsi, SsiDstRunner, run_ssi_scenario};
use vf_dst::ssi_oracle::{SsiOracleTrace, replay_ssi_oracle, run_all_ssi_oracles};
use vf_examples::SsiStore;

/// Get seed from environment or generate random one.
fn get_or_generate_seed() -> u64 {
    match std::env::var("DST_SEED") {
        Ok(s) => {
            let seed: u64 = s.parse().expect("DST_SEED must be a valid u64");
            println!("DST_SEED={} (from environment)", seed);
            seed
        }
        Err(_) => {
            let seed = rand::random::<u64>();
            println!("DST_SEED={} (randomly generated)", seed);
            seed
        }
    }
}

#[test]
fn test_ssi_store_with_dst_runner() {
    let seed = get_or_generate_seed();
    let store = SsiStore::new();
    let mut runner = SsiDstRunner::new(store, seed);

    // Run some operations, handling faults gracefully
    let mut committed = HashSet::new();
    let mut active_txns = Vec::new();

    for i in 0..100 {
        // Randomly choose an operation
        let op_type = (seed.wrapping_add(i)) % 5;

        match op_type {
            0 => {
                // Begin new transaction
                match runner.begin() {
                    Ok(txn) => active_txns.push(txn),
                    Err(_) => {} // Fault injected, expected
                }
            }
            1 => {
                // Read
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns[idx];
                    let key = (seed.wrapping_add(i) % 5) + 1; // Keys 1-5
                    let _ = runner.read(txn, key);
                }
            }
            2 => {
                // Write
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns[idx];
                    let key = (seed.wrapping_add(i) % 5) + 1;
                    let value = seed.wrapping_add(i);
                    let _ = runner.write(txn, key, value);
                }
            }
            3 => {
                // Commit
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns.remove(idx);
                    if let Ok(true) = runner.commit(txn) {
                        committed.insert(txn);
                    }
                }
            }
            4 => {
                // Abort
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns.remove(idx);
                    runner.abort(txn);
                }
            }
            _ => unreachable!(),
        }
    }

    let stats = runner.stats();
    println!("{}", stats.format());
    println!("Committed transactions: {:?}", committed);

    // Verify we ran some operations
    assert!(stats.operations_count > 0, "Should have run some operations");
}

#[test]
fn test_ssi_store_scenario_replay() {
    let seed = 12345u64;
    let store = SsiStore::new();

    // Define a scenario
    let ops = vec![
        (1, DstSsiOp::Begin),
        (1, DstSsiOp::Write(1, 100)),
        (1, DstSsiOp::Commit),
        (2, DstSsiOp::Begin),
        (2, DstSsiOp::Read(1)),
        (2, DstSsiOp::Write(2, 200)),
        (2, DstSsiOp::Commit),
    ];

    let result = run_ssi_scenario(store, seed, ops);
    println!("{}", result.format());

    // With deterministic seed, should always produce same result
    assert!(result.passed);
}

#[test]
fn test_ssi_determinism() {
    let seed = 42u64;

    // Run same scenario twice
    let ops = vec![
        (1, DstSsiOp::Begin),
        (1, DstSsiOp::Write(1, 10)),
        (1, DstSsiOp::Commit),
        (2, DstSsiOp::Begin),
        (2, DstSsiOp::Read(1)),
        (2, DstSsiOp::Commit),
    ];

    let result1 = run_ssi_scenario(SsiStore::new(), seed, ops.clone());
    let result2 = run_ssi_scenario(SsiStore::new(), seed, ops);

    // Same seed = same fault injection = same stats
    assert_eq!(
        result1.stats.faults_injected,
        result2.stats.faults_injected,
        "Fault injection should be deterministic"
    );
}

#[test]
fn test_ssi_store_dangerous_structure_with_faults() {
    // This test verifies that the SSI implementation correctly
    // handles the dangerous structure pattern even under fault injection.

    for i in 0..10 {
        let seed = 1000 + i;
        let store = SsiStore::new();

        // Setup: create initial values
        let setup = store.begin();
        store.write(setup, 1, 10);
        store.write(setup, 2, 20);
        assert!(store.commit(setup));

        // Create dangerous structure (write skew pattern)
        let t1 = store.begin();
        let t2 = store.begin();

        // T1 reads K1
        store.read(t1, 1);
        // T2 reads K2
        store.read(t2, 2);
        // T2 writes K1
        store.write(t2, 1, 11);
        store.commit(t2);
        // T1 writes K2 (creates dangerous structure)
        store.write(t1, 2, 21);

        // T1's commit should fail
        let committed = store.commit(t1);
        assert!(!committed, "Seed {}: T1 should abort due to dangerous structure", seed);
    }
}

#[test]
fn test_ssi_store_concurrent_writes_different_keys() {
    // Verify that concurrent writes to different keys succeed
    let store = SsiStore::new();

    let t1 = store.begin();
    let t2 = store.begin();

    assert!(store.write(t1, 1, 100));
    assert!(store.write(t2, 2, 200));

    assert!(store.commit(t1));
    assert!(store.commit(t2));

    // Verify values
    let t3 = store.begin();
    assert_eq!(store.read(t3, 1), Some(100));
    assert_eq!(store.read(t3, 2), Some(200));
}

#[test]
fn test_ssi_committed_txns_tracking() {
    let store = SsiStore::new();

    let t1 = store.begin();
    store.write(t1, 1, 100);
    assert!(store.commit(t1));

    let committed = store.committed_txns();
    assert!(committed.contains(&t1), "T1 should be in committed set");

    let t2 = store.begin();
    store.write(t2, 2, 200);
    store.abort(t2);

    let committed = store.committed_txns();
    assert!(!committed.contains(&t2), "T2 should NOT be in committed set (was aborted)");
}

#[test]
fn test_ssi_invariants_after_dst_run() {
    let seed = get_or_generate_seed();
    let store = SsiStore::new();
    let mut runner = SsiDstRunner::new(store, seed);

    // Run random operations
    let mut active_txns = Vec::new();

    for i in 0..50 {
        let op_type = (seed.wrapping_add(i)) % 5;

        match op_type {
            0 => {
                if let Ok(txn) = runner.begin() {
                    active_txns.push(txn);
                }
            }
            1 => {
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns[idx];
                    let key = (seed.wrapping_add(i) % 5) + 1;
                    let _ = runner.read(txn, key);
                }
            }
            2 => {
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns[idx];
                    let key = (seed.wrapping_add(i) % 5) + 1;
                    let value = seed.wrapping_add(i);
                    let _ = runner.write(txn, key, value);
                }
            }
            3 => {
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns.remove(idx);
                    let _ = runner.commit(txn);
                }
            }
            4 => {
                if !active_txns.is_empty() {
                    let idx = ((seed.wrapping_add(i)) as usize) % active_txns.len();
                    let txn = active_txns.remove(idx);
                    runner.abort(txn);
                }
            }
            _ => unreachable!(),
        }
    }

    // Check invariants
    let results = runner.check_invariants();
    for result in &results {
        println!(
            "  {}: {}{}",
            result.name,
            if result.holds { "PASS" } else { "FAIL" },
            result.message.as_ref().map(|m| format!(" - {}", m)).unwrap_or_default()
        );
    }

    // Use assert_invariants for rich failure output
    runner.assert_invariants();

    println!("SSI invariants verified: {}", runner.stats().format());
}

#[test]
fn test_ssi_invariants_serial_execution() {
    // Verify invariants hold for a simple serial execution
    let store = SsiStore::new();
    let mut runner = SsiDstRunner::new(store, 99999);

    // T1: write and commit
    let t1 = runner.begin().unwrap();
    runner.write(t1, 1, 100).unwrap();
    runner.commit(t1).unwrap();

    // T2: read and commit
    let t2 = runner.begin().unwrap();
    runner.read(t2, 1).unwrap();
    runner.commit(t2).unwrap();

    // Check invariants
    let results = runner.check_invariants();
    for result in &results {
        assert!(result.holds, "Invariant {} should hold: {:?}", result.name, result.message);
    }
}

#[test]
fn test_ssi_invariants_dangerous_structure_aborted() {
    // Verify that a dangerous structure transaction is aborted,
    // and the invariant "no committed dangerous structures" holds.
    let store = SsiStore::new();
    let mut runner = SsiDstRunner::new(store, 88888);

    // Setup: create initial values (using store directly to avoid fault injection)
    let setup = runner.begin().unwrap();
    runner.write(setup, 1, 10).unwrap();
    runner.write(setup, 2, 20).unwrap();
    runner.commit(setup).unwrap();

    // Create dangerous structure (write skew pattern)
    let t1 = runner.begin().unwrap();
    let t2 = runner.begin().unwrap();

    // T1 reads K1
    runner.read(t1, 1).unwrap();
    // T2 reads K2
    runner.read(t2, 2).unwrap();
    // T2 writes K1 (conflict with T1's read)
    runner.write(t2, 1, 11).unwrap();
    runner.commit(t2).unwrap();
    // T1 writes K2 (creates dangerous structure in T1)
    runner.write(t1, 2, 21).unwrap();

    // T1's commit should fail due to dangerous structure
    let t1_committed = runner.commit(t1);
    assert!(t1_committed.is_ok() && !t1_committed.unwrap(), "T1 should abort");

    // Invariants should still hold (dangerous structure was aborted, not committed)
    let results = runner.check_invariants();
    for result in &results {
        println!(
            "  {}: {}{}",
            result.name,
            if result.holds { "PASS" } else { "FAIL" },
            result.message.as_ref().map(|m| format!(" - {}", m)).unwrap_or_default()
        );
        assert!(result.holds, "Invariant {} should hold after aborting dangerous structure", result.name);
    }
}

// ============================================================================
// Oracle-Driven Tests (Stateright → DST bridge)
// ============================================================================

#[test]
fn test_ssi_oracle_replay_disjoint_keys() {
    let oracle = SsiOracleTrace::disjoint_keys();
    let store = SsiStore::new();
    let result = replay_ssi_oracle(store, 12345, &oracle);

    println!("{}", result.format());
    assert!(result.invariants_hold, "Invariants should hold for disjoint keys");
    assert_eq!(result.committed_txns.len(), 2, "Both transactions should commit");
}

#[test]
fn test_ssi_oracle_replay_sequential_writes() {
    let oracle = SsiOracleTrace::sequential_writes();
    let store = SsiStore::new();
    let result = replay_ssi_oracle(store, 12345, &oracle);

    println!("{}", result.format());
    assert!(result.invariants_hold, "Invariants should hold for sequential writes");
    assert_eq!(result.committed_txns.len(), 2, "Both transactions should commit");
}

#[test]
fn test_ssi_oracle_replay_dangerous_structure() {
    // This oracle creates a dangerous structure - one txn should abort
    let oracle = SsiOracleTrace::dangerous_structure();
    let store = SsiStore::new();
    let result = replay_ssi_oracle(store, 12345, &oracle);

    println!("{}", result.format());
    // Invariants MUST hold (dangerous structure should be aborted, not committed)
    assert!(result.invariants_hold, "Invariants should hold even with dangerous structure");
    // At least one transaction should be aborted
    assert!(!result.aborted_txns.is_empty(), "Dangerous structure should cause abort");
}

#[test]
fn test_ssi_oracle_replay_single_conflict_flag() {
    let oracle = SsiOracleTrace::single_conflict_flag();
    let store = SsiStore::new();
    let result = replay_ssi_oracle(store, 12345, &oracle);

    println!("{}", result.format());
    assert!(result.invariants_hold, "Invariants should hold with single conflict flag");
    // Both should commit (only out_conflict, no dangerous structure)
    assert_eq!(result.committed_txns.len(), 2, "Both should commit with single conflict flag");
}

#[test]
fn test_ssi_oracle_replay_read_only() {
    let oracle = SsiOracleTrace::read_only();
    let store = SsiStore::new();
    let result = replay_ssi_oracle(store, 12345, &oracle);

    println!("{}", result.format());
    assert!(result.invariants_hold, "Invariants should hold for read-only");
    assert_eq!(result.committed_txns.len(), 2, "Both transactions should commit");
}

#[test]
fn test_ssi_run_all_oracles() {
    let results = run_all_ssi_oracles(SsiStore::new, 42);

    println!("\n=== SSI Oracle Replay Results ===");
    let mut all_invariants_hold = true;
    for result in &results {
        println!("{}", result.format());
        if !result.invariants_hold {
            all_invariants_hold = false;
        }
    }
    println!("=================================\n");

    assert!(all_invariants_hold, "All oracles should have invariants hold");
    assert!(results.len() >= 5, "Should have at least 5 oracles");
}
