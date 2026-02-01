//! DST harness for Serializable Snapshot Isolation testing.
//!
//! Provides fault injection at transaction boundaries, not inside operations.
//! Respects the "code is disposable" principle - the SSI implementation
//! doesn't know it's being tested.
//!
//! # Fault Points
//!
//! SSI has different fault points than lock-free structures:
//!
//! | Fault Point | What it simulates |
//! |-------------|-------------------|
//! | BeforeBegin | Connection failure before transaction starts |
//! | AfterBegin | Crash after begin, before any operations |
//! | BeforeRead | Network timeout before read |
//! | AfterRead | Crash after read, SIREAD lock acquired |
//! | BeforeWrite | Allocation failure before write |
//! | AfterWrite | Crash after write, lock held |
//! | BeforeCommit | Network partition before commit |
//! | AfterCommit | Crash after commit, before client notified |
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  DST Harness                                                 │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
//! │  │ FaultPoint  │───>│ Pure SSI    │───>│ FaultPoint  │     │
//! │  │ (pre-op)    │    │ Impl        │    │ (post-op)   │     │
//! │  └─────────────┘    └─────────────┘    └─────────────┘     │
//! │        │                                      │              │
//! │        ▼                                      ▼              │
//! │  "Network timeout?"              "Crash before return?"     │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use crate::fault::{FaultConfig, FaultInjector};
use crate::random::DeterministicRng;
use std::collections::{HashMap, HashSet};

use vf_core::invariants::ssi::{SsiHistory, InvariantResult};

/// Transaction identifier.
pub type TxnId = u64;

/// Key identifier.
pub type KeyId = u64;

/// Value type.
pub type Value = u64;

/// Fault injection points for SSI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsiFaultPoint {
    BeforeBegin,
    AfterBegin,
    BeforeRead,
    AfterRead,
    BeforeWrite,
    AfterWrite,
    BeforeCommit,
    AfterCommit,
}

/// Types of faults that can be injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsiFaultType {
    /// Network timeout
    NetworkTimeout,
    /// Connection dropped
    ConnectionDropped,
    /// Transaction aborted by system
    SystemAbort,
    /// Delay (simulates slow operation)
    Delay,
    /// Out of memory
    OutOfMemory,
}

/// Result of an SSI operation.
pub type SsiResult<T> = Result<T, SsiFaultType>;

/// Trait for SSI implementations testable with DST.
///
/// MINIMAL interface - no DST knowledge in the implementation.
pub trait DstTestableSsi: Send + Sync {
    /// Begin a new transaction, returns transaction ID.
    fn begin(&self) -> TxnId;

    /// Read a key within a transaction.
    fn read(&self, txn: TxnId, key: KeyId) -> Option<Value>;

    /// Write a key within a transaction.
    fn write(&self, txn: TxnId, key: KeyId, value: Value) -> bool;

    /// Commit a transaction.
    fn commit(&self, txn: TxnId) -> bool;

    /// Abort a transaction.
    fn abort(&self, txn: TxnId);

    /// Check if transaction is active.
    fn is_active(&self, txn: TxnId) -> bool;

    /// Get all committed transaction IDs.
    fn committed_txns(&self) -> HashSet<TxnId>;

    /// Get current value of a key (for invariant checking).
    fn get_current_value(&self, key: KeyId) -> Option<Value>;

    /// Get conflict flags for a transaction (for invariant checking).
    /// Returns (in_conflict, out_conflict). Default: (false, false)
    fn get_conflict_flags(&self, _txn: TxnId) -> (bool, bool) {
        (false, false)
    }
}

/// DST runner for SSI implementations.
pub struct SsiDstRunner<S> {
    ssi: S,
    rng: DeterministicRng,
    fault_injector: FaultInjector,
    seed: u64,

    // Tracking for invariant verification
    operations: Vec<SsiOperation>,
    active_txns: HashSet<TxnId>,

    // Timestamps for history building
    timestamp: u64,
    txn_start_ts: HashMap<TxnId, u64>,
    txn_commit_ts: HashMap<TxnId, u64>,

    // Statistics
    operations_count: u64,
    faults_injected: u64,
    txns_started: u64,
    txns_committed: u64,
    txns_aborted: u64,
}

/// Recorded operation for history.
#[derive(Debug, Clone)]
pub enum SsiOperation {
    Begin(TxnId),
    Read { txn: TxnId, key: KeyId, value: Option<Value> },
    Write { txn: TxnId, key: KeyId, value: Value },
    Commit(TxnId),
    Abort(TxnId),
    FaultInjected { txn: TxnId, fault: SsiFaultType, point: SsiFaultPoint },
}

impl<S: DstTestableSsi> SsiDstRunner<S> {
    /// Create a new DST runner.
    pub fn new(ssi: S, seed: u64) -> Self {
        let rng = DeterministicRng::new(seed);
        let fault_injector = FaultInjector::new(
            DeterministicRng::new(seed.wrapping_add(1)),
            FaultConfig::default(),
        );

        Self {
            ssi,
            rng,
            fault_injector,
            seed,
            operations: Vec::new(),
            active_txns: HashSet::new(),
            timestamp: 1,
            txn_start_ts: HashMap::new(),
            txn_commit_ts: HashMap::new(),
            operations_count: 0,
            faults_injected: 0,
            txns_started: 0,
            txns_committed: 0,
            txns_aborted: 0,
        }
    }

    /// Advance timestamp.
    fn tick(&mut self) -> u64 {
        let ts = self.timestamp;
        self.timestamp += 1;
        ts
    }

    /// Get the seed for reproduction.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Begin a transaction with fault injection.
    pub fn begin(&mut self) -> SsiResult<TxnId> {
        // Fault point: before begin
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::BeforeBegin) {
            self.faults_injected += 1;
            return Err(fault);
        }

        // Execute PURE operation
        let txn = self.ssi.begin();
        let ts = self.tick();
        self.txn_start_ts.insert(txn, ts);
        self.operations_count += 1;
        self.txns_started += 1;
        self.active_txns.insert(txn);
        self.operations.push(SsiOperation::Begin(txn));

        // Fault point: after begin
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::AfterBegin) {
            self.faults_injected += 1;
            self.operations.push(SsiOperation::FaultInjected {
                txn,
                fault,
                point: SsiFaultPoint::AfterBegin,
            });
            // Transaction started but client doesn't know
            return Err(fault);
        }

        Ok(txn)
    }

    /// Read with fault injection.
    pub fn read(&mut self, txn: TxnId, key: KeyId) -> SsiResult<Option<Value>> {
        if !self.active_txns.contains(&txn) {
            return Err(SsiFaultType::SystemAbort);
        }

        // Fault point: before read
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::BeforeRead) {
            self.faults_injected += 1;
            self.operations.push(SsiOperation::FaultInjected {
                txn,
                fault,
                point: SsiFaultPoint::BeforeRead,
            });
            return Err(fault);
        }

        // Execute PURE operation
        let value = self.ssi.read(txn, key);
        self.operations_count += 1;
        self.operations.push(SsiOperation::Read { txn, key, value });

        // Fault point: after read
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::AfterRead) {
            self.faults_injected += 1;
            self.operations.push(SsiOperation::FaultInjected {
                txn,
                fault,
                point: SsiFaultPoint::AfterRead,
            });
            // Read completed, SIREAD lock acquired, but client crashes
            return Err(fault);
        }

        Ok(value)
    }

    /// Write with fault injection.
    pub fn write(&mut self, txn: TxnId, key: KeyId, value: Value) -> SsiResult<bool> {
        if !self.active_txns.contains(&txn) {
            return Err(SsiFaultType::SystemAbort);
        }

        // Fault point: before write
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::BeforeWrite) {
            self.faults_injected += 1;
            self.operations.push(SsiOperation::FaultInjected {
                txn,
                fault,
                point: SsiFaultPoint::BeforeWrite,
            });
            if fault == SsiFaultType::OutOfMemory {
                return Err(fault);
            }
        }

        // Execute PURE operation
        let success = self.ssi.write(txn, key, value);
        self.operations_count += 1;
        self.operations.push(SsiOperation::Write { txn, key, value });

        // Fault point: after write
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::AfterWrite) {
            self.faults_injected += 1;
            self.operations.push(SsiOperation::FaultInjected {
                txn,
                fault,
                point: SsiFaultPoint::AfterWrite,
            });
            // Write completed, lock held, but client crashes
            return Err(fault);
        }

        Ok(success)
    }

    /// Commit with fault injection.
    pub fn commit(&mut self, txn: TxnId) -> SsiResult<bool> {
        if !self.active_txns.contains(&txn) {
            return Err(SsiFaultType::SystemAbort);
        }

        // Fault point: before commit
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::BeforeCommit) {
            self.faults_injected += 1;
            self.operations.push(SsiOperation::FaultInjected {
                txn,
                fault,
                point: SsiFaultPoint::BeforeCommit,
            });
            if fault == SsiFaultType::NetworkTimeout {
                // Network partition - transaction state unknown
                return Err(fault);
            }
        }

        // Execute PURE operation
        let success = self.ssi.commit(txn);
        self.operations_count += 1;

        if success {
            let ts = self.tick();
            self.txn_commit_ts.insert(txn, ts);
            self.txns_committed += 1;
            self.active_txns.remove(&txn);
            self.operations.push(SsiOperation::Commit(txn));
        } else {
            self.txns_aborted += 1;
            self.active_txns.remove(&txn);
            self.operations.push(SsiOperation::Abort(txn));
        }

        // Fault point: after commit
        if let Some(fault) = self.maybe_inject_fault(SsiFaultPoint::AfterCommit) {
            self.faults_injected += 1;
            self.operations.push(SsiOperation::FaultInjected {
                txn,
                fault,
                point: SsiFaultPoint::AfterCommit,
            });
            // Commit succeeded but client doesn't know
            return Err(fault);
        }

        Ok(success)
    }

    /// Abort a transaction.
    pub fn abort(&mut self, txn: TxnId) {
        if self.active_txns.contains(&txn) {
            self.ssi.abort(txn);
            self.active_txns.remove(&txn);
            self.txns_aborted += 1;
            self.operations.push(SsiOperation::Abort(txn));
        }
    }

    /// Maybe inject a fault at the given point.
    fn maybe_inject_fault(&mut self, _point: SsiFaultPoint) -> Option<SsiFaultType> {
        if self.fault_injector.should_fail() {
            let fault_type = match self.rng.gen_range(0..5) {
                0 => SsiFaultType::NetworkTimeout,
                1 => SsiFaultType::ConnectionDropped,
                2 => SsiFaultType::SystemAbort,
                3 => SsiFaultType::Delay,
                _ => SsiFaultType::OutOfMemory,
            };
            Some(fault_type)
        } else {
            None
        }
    }

    /// Get statistics.
    pub fn stats(&self) -> SsiDstStats {
        SsiDstStats {
            seed: self.seed,
            operations_count: self.operations_count,
            faults_injected: self.faults_injected,
            txns_started: self.txns_started,
            txns_committed: self.txns_committed,
            txns_aborted: self.txns_aborted,
        }
    }

    /// Get the operation history.
    pub fn history(&self) -> &[SsiOperation] {
        &self.operations
    }

    /// Convert operation history to SsiHistory for invariant checking.
    ///
    /// This bridges the DST operation log to the vf-core invariant checker.
    pub fn to_ssi_history(&self) -> SsiHistory {
        let mut history = SsiHistory::new();

        // Track which txn wrote which version of each key
        let mut key_versions: HashMap<KeyId, TxnId> = HashMap::new();

        for op in &self.operations {
            match op {
                SsiOperation::Begin(txn) => {
                    let ts = self.txn_start_ts.get(txn).copied().unwrap_or(0);
                    history.begin(*txn, ts);
                }
                SsiOperation::Read { txn, key, value: _ } => {
                    // Find the version we read (which txn wrote it)
                    let version = key_versions.get(key).copied();
                    let ts = self.tick_for_op();
                    history.read(*txn, *key, version, ts);
                }
                SsiOperation::Write { txn, key, value: _ } => {
                    key_versions.insert(*key, *txn);
                    let ts = self.tick_for_op();
                    history.write(*txn, *key, ts);
                }
                SsiOperation::Commit(txn) => {
                    // Get conflict flags from the implementation
                    let (in_c, out_c) = self.ssi.get_conflict_flags(*txn);
                    if in_c {
                        history.set_in_conflict(*txn);
                    }
                    if out_c {
                        history.set_out_conflict(*txn);
                    }
                    let ts = self.txn_commit_ts.get(txn).copied().unwrap_or(0);
                    history.commit(*txn, ts);
                }
                SsiOperation::Abort(txn) => {
                    history.abort(*txn);
                }
                SsiOperation::FaultInjected { .. } => {
                    // Faults don't affect history
                }
            }
        }

        history
    }

    /// Helper for timestamp in history building (doesn't actually tick).
    fn tick_for_op(&self) -> u64 {
        self.timestamp
    }

    /// Check all SSI invariants against the current history.
    ///
    /// Returns a vector of results for each invariant.
    pub fn check_invariants(&self) -> Vec<InvariantResult> {
        let history = self.to_ssi_history();
        vf_core::invariants::ssi::check_all(&history)
    }

    /// Check if all invariants hold.
    pub fn invariants_hold(&self) -> bool {
        self.check_invariants().iter().all(|r| r.holds)
    }

    /// Format invariant failures for DST evaluator parsing.
    ///
    /// Outputs a structured format that the evaluator can extract:
    /// ```text
    /// === DST INVARIANT FAILURES ===
    /// INVARIANT_FAILED: NoCommittedDangerousStructures
    ///   Message: Committed transaction T3 has dangerous structure (in_conflict=true, out_conflict=true)
    /// INVARIANT_FAILED: Serializable
    ///   Message: Serializability violated: ...
    /// === END INVARIANT FAILURES ===
    /// ```
    pub fn format_invariant_failures(&self) -> Option<String> {
        let results = self.check_invariants();
        let failures: Vec<_> = results.iter().filter(|r| !r.holds).collect();

        if failures.is_empty() {
            return None;
        }

        let mut output = String::new();
        output.push_str("=== DST INVARIANT FAILURES ===\n");
        output.push_str(&format!("DST_SEED={}\n", self.seed));

        for failure in &failures {
            output.push_str(&format!("INVARIANT_FAILED: {}\n", failure.name));
            if let Some(ref msg) = failure.message {
                output.push_str(&format!("  Message: {}\n", msg));
            }
        }

        // Include operation trace for debugging
        output.push_str("\nOperation trace (last 20):\n");
        let start = self.operations.len().saturating_sub(20);
        for (i, op) in self.operations.iter().skip(start).enumerate() {
            output.push_str(&format!("  {}: {:?}\n", start + i, op));
        }

        output.push_str("=== END INVARIANT FAILURES ===\n");
        Some(output)
    }

    /// Assert invariants with rich failure output.
    ///
    /// Use this instead of `assert!(runner.invariants_hold())` to get
    /// actionable feedback in DST test failures.
    pub fn assert_invariants(&self) {
        if !self.invariants_hold() {
            if let Some(failures) = self.format_invariant_failures() {
                // Print structured output for evaluator extraction
                eprintln!("{}", failures);
            }
            panic!(
                "SSI invariants violated at DST_SEED={}. See above for details.",
                self.seed
            );
        }
    }
}

/// Statistics from DST run.
#[derive(Debug, Clone)]
pub struct SsiDstStats {
    pub seed: u64,
    pub operations_count: u64,
    pub faults_injected: u64,
    pub txns_started: u64,
    pub txns_committed: u64,
    pub txns_aborted: u64,
}

impl SsiDstStats {
    pub fn format(&self) -> String {
        format!(
            "DST_SEED={} ops={} faults={} txns(started={} committed={} aborted={})",
            self.seed,
            self.operations_count,
            self.faults_injected,
            self.txns_started,
            self.txns_committed,
            self.txns_aborted
        )
    }
}

/// DST operation for scenario replay.
#[derive(Debug, Clone)]
pub enum DstSsiOp {
    Begin,
    Read(KeyId),
    Write(KeyId, Value),
    Commit,
    Abort,
}

/// Run a DST scenario.
pub fn run_ssi_scenario<S: DstTestableSsi>(
    ssi: S,
    seed: u64,
    operations: Vec<(TxnId, DstSsiOp)>,
) -> SsiDstResult {
    let mut runner = SsiDstRunner::new(ssi, seed);
    let mut txn_map: HashMap<TxnId, TxnId> = HashMap::new(); // scenario txn -> actual txn
    let mut errors = Vec::new();

    for (scenario_txn, op) in operations {
        let result = match op {
            DstSsiOp::Begin => {
                match runner.begin() {
                    Ok(actual_txn) => {
                        txn_map.insert(scenario_txn, actual_txn);
                        Ok(())
                    }
                    Err(e) => Err(e),
                }
            }
            DstSsiOp::Read(key) => {
                if let Some(&actual_txn) = txn_map.get(&scenario_txn) {
                    runner.read(actual_txn, key).map(|_| ())
                } else {
                    Err(SsiFaultType::SystemAbort)
                }
            }
            DstSsiOp::Write(key, value) => {
                if let Some(&actual_txn) = txn_map.get(&scenario_txn) {
                    runner.write(actual_txn, key, value).map(|_| ())
                } else {
                    Err(SsiFaultType::SystemAbort)
                }
            }
            DstSsiOp::Commit => {
                if let Some(&actual_txn) = txn_map.get(&scenario_txn) {
                    runner.commit(actual_txn).map(|_| ())
                } else {
                    Err(SsiFaultType::SystemAbort)
                }
            }
            DstSsiOp::Abort => {
                if let Some(&actual_txn) = txn_map.get(&scenario_txn) {
                    runner.abort(actual_txn);
                    Ok(())
                } else {
                    Ok(())
                }
            }
        };

        if let Err(fault) = result {
            errors.push(format!("{:?}", fault));
        }
    }

    SsiDstResult {
        passed: true, // Faults are expected, not failures
        stats: runner.stats(),
        fault_errors: errors,
    }
}

/// DST result.
#[derive(Debug)]
pub struct SsiDstResult {
    pub passed: bool,
    pub stats: SsiDstStats,
    pub fault_errors: Vec<String>,
}

impl SsiDstResult {
    pub fn format(&self) -> String {
        let status = if self.passed { "PASS" } else { "FAIL" };
        format!("[{}] {}", status, self.stats.format())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Simple mock SSI for testing the DST framework itself.
    struct MockSsi {
        next_txn: Mutex<TxnId>,
        active: Mutex<HashSet<TxnId>>,
        committed: Mutex<HashSet<TxnId>>,
        data: Mutex<HashMap<KeyId, Value>>,
    }

    impl MockSsi {
        fn new() -> Self {
            Self {
                next_txn: Mutex::new(1),
                active: Mutex::new(HashSet::new()),
                committed: Mutex::new(HashSet::new()),
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    impl DstTestableSsi for MockSsi {
        fn begin(&self) -> TxnId {
            let mut next = self.next_txn.lock().unwrap();
            let txn = *next;
            *next += 1;
            self.active.lock().unwrap().insert(txn);
            txn
        }

        fn read(&self, _txn: TxnId, key: KeyId) -> Option<Value> {
            self.data.lock().unwrap().get(&key).copied()
        }

        fn write(&self, _txn: TxnId, key: KeyId, value: Value) -> bool {
            self.data.lock().unwrap().insert(key, value);
            true
        }

        fn commit(&self, txn: TxnId) -> bool {
            let mut active = self.active.lock().unwrap();
            if active.remove(&txn) {
                self.committed.lock().unwrap().insert(txn);
                true
            } else {
                false
            }
        }

        fn abort(&self, txn: TxnId) {
            self.active.lock().unwrap().remove(&txn);
        }

        fn is_active(&self, txn: TxnId) -> bool {
            self.active.lock().unwrap().contains(&txn)
        }

        fn committed_txns(&self) -> HashSet<TxnId> {
            self.committed.lock().unwrap().clone()
        }

        fn get_current_value(&self, key: KeyId) -> Option<Value> {
            self.data.lock().unwrap().get(&key).copied()
        }
    }

    #[test]
    fn test_ssi_dst_runner_basic() {
        let ssi = MockSsi::new();
        let mut runner = SsiDstRunner::new(ssi, 12345);

        // These might fail due to fault injection, and that's OK
        let txn = runner.begin();
        if let Ok(t) = txn {
            let _ = runner.write(t, 1, 100);
            let _ = runner.read(t, 1);
            let _ = runner.commit(t);
        }

        println!("{}", runner.stats().format());
    }

    #[test]
    fn test_ssi_scenario() {
        let ssi = MockSsi::new();
        let ops = vec![
            (1, DstSsiOp::Begin),
            (1, DstSsiOp::Write(100, 42)),
            (1, DstSsiOp::Read(100)),
            (1, DstSsiOp::Commit),
            (2, DstSsiOp::Begin),
            (2, DstSsiOp::Read(100)),
            (2, DstSsiOp::Commit),
        ];

        let result = run_ssi_scenario(ssi, 12345, ops);
        println!("{}", result.format());
    }

    #[test]
    fn test_determinism() {
        let ops = vec![
            (1, DstSsiOp::Begin),
            (1, DstSsiOp::Write(1, 1)),
            (1, DstSsiOp::Commit),
        ];

        let result1 = run_ssi_scenario(MockSsi::new(), 42, ops.clone());
        let result2 = run_ssi_scenario(MockSsi::new(), 42, ops);

        // Same seed = same faults = same stats
        assert_eq!(result1.stats.faults_injected, result2.stats.faults_injected);
    }
}
