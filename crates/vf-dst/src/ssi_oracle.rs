//! Oracle-driven SSI DST testing.
//!
//! Bridges Stateright-discovered SSI scenarios to DST tests.
//! Converts `SsiOracle` from vf-stateright to DST operations.
//!
//! # Architecture
//!
//! ```text
//! Stateright Model Checking (vf-stateright/src/ssi.rs)
//!         │
//!         ▼
//!    SsiOracle (actions: Vec<SsiAction>)
//!         │
//!         ▼
//!    SsiOracleToDst (this module)
//!         │
//!         ▼
//!    SsiDstRunner (ssi_harness.rs)
//!         │
//!         ▼
//!    Actual SsiStore Implementation
//! ```

use std::collections::HashMap;

use crate::ssi_harness::{DstTestableSsi, SsiDstRunner, SsiFaultType, SsiResult};

/// SSI action types from Stateright model.
/// Mirrors vf_stateright::ssi::SsiAction but doesn't require the dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsiOracleAction {
    Begin(u8),
    Read(u8, u8),      // txn, key
    Write(u8, u8),     // txn, key
    Commit(u8),
    Abort(u8),
}

/// An oracle trace for SSI DST replay.
#[derive(Debug, Clone)]
pub struct SsiOracleTrace {
    /// Name for debugging
    pub name: String,
    /// Actions to replay
    pub actions: Vec<SsiOracleAction>,
    /// Description
    pub description: String,
    /// Expected outcome
    pub expected_outcome: SsiExpectedOutcome,
}

/// Expected outcome of an SSI oracle trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsiExpectedOutcome {
    /// All transactions commit successfully
    AllCommit,
    /// At least one transaction aborts (specify which)
    SomeAbort(Vec<u8>),
    /// Specific transaction aborts due to dangerous structure
    DangerousStructureAbort(u8),
    /// Invariants should hold regardless of outcome
    InvariantsHold,
}

impl SsiOracleTrace {
    /// Create a new empty trace.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            actions: Vec::new(),
            description: String::new(),
            expected_outcome: SsiExpectedOutcome::InvariantsHold,
        }
    }

    /// Add description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set expected outcome.
    pub fn with_expected_outcome(mut self, outcome: SsiExpectedOutcome) -> Self {
        self.expected_outcome = outcome;
        self
    }

    /// Add an action.
    pub fn add(&mut self, action: SsiOracleAction) {
        self.actions.push(action);
    }

    // ========================================================================
    // Pre-built SSI oracle traces (mirrors vf_stateright::ssi::SsiOracle)
    // ========================================================================

    /// Dangerous structure (write skew) pattern.
    ///
    /// T1 reads K1, T2 reads K2, T2 writes K1, T1 writes K2.
    /// Creates dangerous structure - one must abort.
    pub fn dangerous_structure() -> Self {
        let mut trace = Self::new("dangerous_structure")
            .with_description("Write skew pattern - both txns get dangerous structure")
            .with_expected_outcome(SsiExpectedOutcome::InvariantsHold);

        trace.add(SsiOracleAction::Begin(1));
        trace.add(SsiOracleAction::Begin(2));
        trace.add(SsiOracleAction::Read(1, 1));  // T1 reads K1
        trace.add(SsiOracleAction::Read(2, 2));  // T2 reads K2
        trace.add(SsiOracleAction::Write(2, 1)); // T2 writes K1 (conflict with T1's read)
        trace.add(SsiOracleAction::Commit(2));   // T2 commits
        trace.add(SsiOracleAction::Write(1, 2)); // T1 writes K2 (conflict with T2's read)
        trace.add(SsiOracleAction::Commit(1));   // T1 should abort due to dangerous structure

        trace
    }

    /// Disjoint keys - concurrent writes to different keys succeed.
    pub fn disjoint_keys() -> Self {
        let mut trace = Self::new("disjoint_keys")
            .with_description("Concurrent writes to different keys - both commit")
            .with_expected_outcome(SsiExpectedOutcome::AllCommit);

        trace.add(SsiOracleAction::Begin(1));
        trace.add(SsiOracleAction::Begin(2));
        trace.add(SsiOracleAction::Write(1, 1)); // T1 writes K1
        trace.add(SsiOracleAction::Write(2, 2)); // T2 writes K2
        trace.add(SsiOracleAction::Commit(1));
        trace.add(SsiOracleAction::Commit(2));

        trace
    }

    /// Sequential writes to same key.
    pub fn sequential_writes() -> Self {
        let mut trace = Self::new("sequential_writes")
            .with_description("T1 commits before T2 starts - serial execution")
            .with_expected_outcome(SsiExpectedOutcome::AllCommit);

        trace.add(SsiOracleAction::Begin(1));
        trace.add(SsiOracleAction::Write(1, 1));
        trace.add(SsiOracleAction::Commit(1));
        trace.add(SsiOracleAction::Begin(2));
        trace.add(SsiOracleAction::Write(2, 1));
        trace.add(SsiOracleAction::Commit(2));

        trace
    }

    /// Single conflict flag (should commit).
    pub fn single_conflict_flag() -> Self {
        let mut trace = Self::new("single_conflict_flag")
            .with_description("T1 has only out_conflict - can commit")
            .with_expected_outcome(SsiExpectedOutcome::AllCommit);

        trace.add(SsiOracleAction::Begin(1));
        trace.add(SsiOracleAction::Begin(2));
        trace.add(SsiOracleAction::Read(1, 1));  // T1 reads K1
        trace.add(SsiOracleAction::Write(2, 1)); // T2 writes K1 (T1 gets out_conflict)
        trace.add(SsiOracleAction::Commit(2));
        trace.add(SsiOracleAction::Commit(1));   // T1 can commit (only out_conflict)

        trace
    }

    /// Read-only transaction.
    pub fn read_only() -> Self {
        let mut trace = Self::new("read_only")
            .with_description("Read-only transaction sees committed write")
            .with_expected_outcome(SsiExpectedOutcome::AllCommit);

        trace.add(SsiOracleAction::Begin(1));
        trace.add(SsiOracleAction::Write(1, 1));
        trace.add(SsiOracleAction::Commit(1));
        trace.add(SsiOracleAction::Begin(2));
        trace.add(SsiOracleAction::Read(2, 1));
        trace.add(SsiOracleAction::Commit(2));

        trace
    }

    /// All pre-built SSI oracles.
    pub fn all_oracles() -> Vec<Self> {
        vec![
            Self::dangerous_structure(),
            Self::disjoint_keys(),
            Self::sequential_writes(),
            Self::single_conflict_flag(),
            Self::read_only(),
        ]
    }
}

/// Result of replaying an SSI oracle trace.
#[derive(Debug)]
pub struct SsiOracleReplayResult {
    pub oracle_name: String,
    pub actions_executed: usize,
    pub actions_total: usize,
    pub committed_txns: Vec<u64>,
    pub aborted_txns: Vec<u64>,
    pub invariants_hold: bool,
    pub expected_outcome_met: bool,
    pub fault_errors: Vec<String>,
}

impl SsiOracleReplayResult {
    /// Format for display.
    pub fn format(&self) -> String {
        let status = if self.invariants_hold && self.expected_outcome_met {
            "PASS"
        } else {
            "FAIL"
        };
        format!(
            "[{}] Oracle '{}': {}/{} actions, committed={:?}, aborted={:?}, invariants={}, expected={}",
            status,
            self.oracle_name,
            self.actions_executed,
            self.actions_total,
            self.committed_txns,
            self.aborted_txns,
            self.invariants_hold,
            self.expected_outcome_met,
        )
    }
}

/// Replay an SSI oracle trace through DST.
///
/// Maps oracle TxnIds (u8) to actual DST TxnIds (u64).
pub fn replay_ssi_oracle<S: DstTestableSsi>(
    ssi: S,
    seed: u64,
    oracle: &SsiOracleTrace,
) -> SsiOracleReplayResult {
    let mut runner = SsiDstRunner::new(ssi, seed);

    // Map oracle txn IDs to actual txn IDs
    let mut txn_map: HashMap<u8, u64> = HashMap::new();
    let mut committed = Vec::new();
    let mut aborted = Vec::new();
    let mut fault_errors = Vec::new();
    let mut actions_executed = 0;

    for action in &oracle.actions {
        let result: SsiResult<()> = match action {
            SsiOracleAction::Begin(txn) => {
                match runner.begin() {
                    Ok(actual_txn) => {
                        txn_map.insert(*txn, actual_txn);
                        actions_executed += 1;
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            SsiOracleAction::Read(txn, key) => {
                if let Some(&actual_txn) = txn_map.get(txn) {
                    match runner.read(actual_txn, *key as u64) {
                        Ok(_) => {
                            actions_executed += 1;
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    Err(SsiFaultType::SystemAbort)
                }
            }
            SsiOracleAction::Write(txn, key) => {
                if let Some(&actual_txn) = txn_map.get(txn) {
                    // Use txn * 100 + key as value for tracing
                    let value = (*txn as u64) * 100 + (*key as u64);
                    match runner.write(actual_txn, *key as u64, value) {
                        Ok(_) => {
                            actions_executed += 1;
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    Err(SsiFaultType::SystemAbort)
                }
            }
            SsiOracleAction::Commit(txn) => {
                if let Some(&actual_txn) = txn_map.get(txn) {
                    match runner.commit(actual_txn) {
                        Ok(success) => {
                            actions_executed += 1;
                            if success {
                                committed.push(*txn as u64);
                            } else {
                                aborted.push(*txn as u64);
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                } else {
                    Err(SsiFaultType::SystemAbort)
                }
            }
            SsiOracleAction::Abort(txn) => {
                if let Some(&actual_txn) = txn_map.get(txn) {
                    runner.abort(actual_txn);
                    aborted.push(*txn as u64);
                    actions_executed += 1;
                }
                Ok(())
            }
        };

        if let Err(fault) = result {
            fault_errors.push(format!("{:?}", fault));
        }
    }

    // Check invariants
    let invariants_hold = runner.invariants_hold();

    // Check expected outcome
    let expected_outcome_met = match &oracle.expected_outcome {
        SsiExpectedOutcome::AllCommit => aborted.is_empty() && fault_errors.is_empty(),
        SsiExpectedOutcome::SomeAbort(expected) => {
            expected.iter().all(|t| aborted.contains(&(*t as u64)))
        }
        SsiExpectedOutcome::DangerousStructureAbort(txn) => {
            aborted.contains(&(*txn as u64))
        }
        SsiExpectedOutcome::InvariantsHold => invariants_hold,
    };

    SsiOracleReplayResult {
        oracle_name: oracle.name.clone(),
        actions_executed,
        actions_total: oracle.actions.len(),
        committed_txns: committed,
        aborted_txns: aborted,
        invariants_hold,
        expected_outcome_met,
        fault_errors,
    }
}

/// Run all pre-built SSI oracles and return results.
pub fn run_all_ssi_oracles<S: DstTestableSsi>(
    ssi_factory: impl Fn() -> S,
    base_seed: u64,
) -> Vec<SsiOracleReplayResult> {
    let oracles = SsiOracleTrace::all_oracles();
    let mut results = Vec::new();

    for (i, oracle) in oracles.iter().enumerate() {
        let ssi = ssi_factory();
        let seed = base_seed.wrapping_add(i as u64);
        let result = replay_ssi_oracle(ssi, seed, oracle);
        results.push(result);
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock SSI for testing the oracle replay mechanism itself
    struct MockSsi {
        next_txn: std::sync::Mutex<u64>,
        active: std::sync::Mutex<std::collections::HashSet<u64>>,
        committed: std::sync::Mutex<std::collections::HashSet<u64>>,
    }

    impl MockSsi {
        fn new() -> Self {
            Self {
                next_txn: std::sync::Mutex::new(1),
                active: std::sync::Mutex::new(std::collections::HashSet::new()),
                committed: std::sync::Mutex::new(std::collections::HashSet::new()),
            }
        }
    }

    impl DstTestableSsi for MockSsi {
        fn begin(&self) -> u64 {
            let mut next = self.next_txn.lock().unwrap();
            let txn = *next;
            *next += 1;
            self.active.lock().unwrap().insert(txn);
            txn
        }

        fn read(&self, _txn: u64, _key: u64) -> Option<u64> {
            None
        }

        fn write(&self, _txn: u64, _key: u64, _value: u64) -> bool {
            true
        }

        fn commit(&self, txn: u64) -> bool {
            let mut active = self.active.lock().unwrap();
            if active.remove(&txn) {
                self.committed.lock().unwrap().insert(txn);
                true
            } else {
                false
            }
        }

        fn abort(&self, txn: u64) {
            self.active.lock().unwrap().remove(&txn);
        }

        fn is_active(&self, txn: u64) -> bool {
            self.active.lock().unwrap().contains(&txn)
        }

        fn committed_txns(&self) -> std::collections::HashSet<u64> {
            self.committed.lock().unwrap().clone()
        }

        fn get_current_value(&self, _key: u64) -> Option<u64> {
            None
        }
    }

    #[test]
    fn test_oracle_trace_creation() {
        let trace = SsiOracleTrace::dangerous_structure();
        assert_eq!(trace.name, "dangerous_structure");
        assert!(!trace.actions.is_empty());
    }

    #[test]
    fn test_all_oracles() {
        let oracles = SsiOracleTrace::all_oracles();
        assert!(oracles.len() >= 5, "Should have at least 5 oracles");

        for oracle in oracles {
            assert!(!oracle.actions.is_empty());
            assert!(!oracle.name.is_empty());
        }
    }

    #[test]
    fn test_replay_disjoint_keys() {
        let ssi = MockSsi::new();
        let oracle = SsiOracleTrace::disjoint_keys();
        let result = replay_ssi_oracle(ssi, 12345, &oracle);

        println!("{}", result.format());
        assert!(result.invariants_hold);
    }

    #[test]
    fn test_replay_sequential_writes() {
        let ssi = MockSsi::new();
        let oracle = SsiOracleTrace::sequential_writes();
        let result = replay_ssi_oracle(ssi, 12345, &oracle);

        println!("{}", result.format());
        assert!(result.invariants_hold);
        // Both transactions should commit in serial execution
        assert_eq!(result.committed_txns.len(), 2);
    }
}
