//! Fault injection for lock-free structures.
//!
//! DST injects faults at OPERATION BOUNDARIES, not inside atomic sequences.
//! This respects "code is disposable" - generated code is pure.
//!
//! # What DST Tests (vs Loom)
//!
//! | Concern | Tool | Level |
//! |---------|------|-------|
//! | CAS races | Loom | Instruction (automatic) |
//! | Memory allocation | DST | Operation boundary |
//! | Thread crash | DST | Operation boundary |
//! | Epoch GC timing | DST | Between operations |
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  Test Harness                                                │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
//! │  │ FaultPoint  │───>│ Pure Stack  │───>│ FaultPoint  │     │
//! │  │ (pre-op)    │    │ push()/pop()│    │ (post-op)   │     │
//! │  └─────────────┘    └─────────────┘    └─────────────┘     │
//! │        │                                      │              │
//! │        ▼                                      ▼              │
//! │  "Fail allocation?"              "Crash before return?"     │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! Generated code is UNCHANGED. Faults happen in the test harness.

use crate::fault::FaultInjector;
use crate::random::DeterministicRng;
use std::collections::HashSet;

/// Fault injection points (between operations, not inside).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultPoint {
    /// Before starting an operation
    BeforeOperation,
    /// After operation completes (before returning to caller)
    AfterOperation,
    /// Between retry attempts (if operation has internal retries)
    BetweenRetries,
}

/// Types of faults that can be injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultType {
    /// Memory allocation fails
    AllocationFailure,
    /// Thread "crashes" (operation abandoned)
    ThreadCrash,
    /// Delay (simulates slow thread)
    Delay,
    /// Epoch GC triggered
    EpochGcTrigger,
}

/// DST test runner for lock-free structures.
///
/// Wraps a pure stack implementation and injects faults
/// at operation boundaries. No code instrumentation needed.
pub struct DstRunner<S> {
    stack: S,
    rng: DeterministicRng,
    fault_injector: FaultInjector,
    seed: u64,
    // Tracking for invariant verification
    pushed: HashSet<u64>,
    popped: HashSet<u64>,
    // Statistics
    operations_count: u64,
    faults_injected: u64,
    abandoned_operations: u64,
}

/// Trait for stacks testable with DST.
///
/// MINIMAL interface - no DST knowledge in the implementation.
/// This is the same trait used for verification, just renamed for clarity.
pub trait DstTestableStack: Send + Sync {
    fn new() -> Self;
    fn push(&self, value: u64);
    fn pop(&self) -> Option<u64>;
    fn is_empty(&self) -> bool;
    fn get_contents(&self) -> Vec<u64>;
}

impl<S: DstTestableStack> DstRunner<S> {
    /// Create a new DST runner.
    pub fn new(seed: u64) -> Self {
        let rng = DeterministicRng::new(seed);
        let fault_injector = FaultInjector::new(
            DeterministicRng::new(seed.wrapping_add(1)),
            crate::fault::FaultConfig::default(),
        );

        Self {
            stack: S::new(),
            rng,
            fault_injector,
            seed,
            pushed: HashSet::new(),
            popped: HashSet::new(),
            operations_count: 0,
            faults_injected: 0,
            abandoned_operations: 0,
        }
    }

    /// Get the seed for reproduction.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// Push with fault injection at boundaries.
    ///
    /// The stack.push() call is PURE - no instrumentation.
    /// Faults happen before/after, in the test harness.
    pub fn push(&mut self, value: u64) -> Result<(), FaultType> {
        // Fault point: before operation
        if let Some(fault) = self.maybe_inject_fault(FaultPoint::BeforeOperation) {
            self.faults_injected += 1;
            if fault == FaultType::ThreadCrash {
                self.abandoned_operations += 1;
                return Err(fault);
            }
            // AllocationFailure - operation doesn't start
            if fault == FaultType::AllocationFailure {
                return Err(fault);
            }
        }

        // Execute PURE operation (no instrumentation)
        self.stack.push(value);
        self.operations_count += 1;

        // Fault point: after operation
        if let Some(fault) = self.maybe_inject_fault(FaultPoint::AfterOperation) {
            self.faults_injected += 1;
            if fault == FaultType::ThreadCrash {
                // Operation completed but "caller never sees result"
                // This tests: what if thread dies after push succeeds?
                // The value IS in the stack (push completed)
                self.pushed.insert(value);
                self.abandoned_operations += 1;
                return Err(fault);
            }
        }

        // Normal completion
        self.pushed.insert(value);
        Ok(())
    }

    /// Pop with fault injection at boundaries.
    pub fn pop(&mut self) -> Result<Option<u64>, FaultType> {
        // Fault point: before operation
        if let Some(fault) = self.maybe_inject_fault(FaultPoint::BeforeOperation) {
            self.faults_injected += 1;
            if fault == FaultType::ThreadCrash {
                self.abandoned_operations += 1;
                return Err(fault);
            }
        }

        // Execute PURE operation
        let result = self.stack.pop();
        self.operations_count += 1;

        // Track what was popped
        if let Some(value) = result {
            self.popped.insert(value);
        }

        // Fault point: after operation
        if let Some(fault) = self.maybe_inject_fault(FaultPoint::AfterOperation) {
            self.faults_injected += 1;
            if fault == FaultType::ThreadCrash {
                // Value was popped, but "caller crashes before using it"
                self.abandoned_operations += 1;
                return Err(fault);
            }
        }

        Ok(result)
    }

    /// Maybe inject a fault at the given point.
    fn maybe_inject_fault(&mut self, _point: FaultPoint) -> Option<FaultType> {
        if self.fault_injector.should_fail() {
            // Choose fault type based on RNG
            let fault_type = match self.rng.gen_range(0..4) {
                0 => FaultType::AllocationFailure,
                1 => FaultType::ThreadCrash,
                2 => FaultType::Delay,
                _ => FaultType::EpochGcTrigger,
            };
            Some(fault_type)
        } else {
            None
        }
    }

    /// Check NoLostElements invariant.
    pub fn check_no_lost_elements(&self) -> bool {
        let contents: HashSet<u64> = self.stack.get_contents().into_iter().collect();

        for &elem in &self.pushed {
            if !contents.contains(&elem) && !self.popped.contains(&elem) {
                return false;
            }
        }
        true
    }

    /// Check NoDuplicates invariant.
    pub fn check_no_duplicates(&self) -> bool {
        let contents = self.stack.get_contents();
        let unique: HashSet<_> = contents.iter().collect();
        contents.len() == unique.len()
    }

    /// Get statistics.
    pub fn stats(&self) -> DstStats {
        DstStats {
            seed: self.seed,
            operations_count: self.operations_count,
            faults_injected: self.faults_injected,
            abandoned_operations: self.abandoned_operations,
        }
    }
}

/// Statistics from DST run.
#[derive(Debug, Clone)]
pub struct DstStats {
    pub seed: u64,
    pub operations_count: u64,
    pub faults_injected: u64,
    pub abandoned_operations: u64,
}

impl DstStats {
    pub fn format(&self) -> String {
        format!(
            "DST_SEED={} ops={} faults={} abandoned={}",
            self.seed, self.operations_count, self.faults_injected, self.abandoned_operations
        )
    }
}

/// Run a DST scenario.
///
/// Operations are executed with fault injection. Invariants checked at end.
pub fn run_dst_scenario<S: DstTestableStack>(
    seed: u64,
    operations: Vec<DstOp>,
) -> DstResult {
    let mut runner: DstRunner<S> = DstRunner::new(seed);
    let mut errors = Vec::new();

    for op in operations {
        let result = match op {
            DstOp::Push(v) => runner.push(v).map(|_| ()),
            DstOp::Pop => runner.pop().map(|_| ()),
        };

        // Faults are expected - they're part of the test
        if let Err(fault) = result {
            // Log but continue - we're testing fault tolerance
            errors.push(format!("{:?}", fault));
        }
    }

    // Check invariants
    let no_lost = runner.check_no_lost_elements();
    let no_dups = runner.check_no_duplicates();

    DstResult {
        passed: no_lost && no_dups,
        no_lost_elements: no_lost,
        no_duplicates: no_dups,
        stats: runner.stats(),
        fault_errors: errors,
    }
}

/// DST operation.
#[derive(Debug, Clone)]
pub enum DstOp {
    Push(u64),
    Pop,
}

/// DST result.
#[derive(Debug)]
pub struct DstResult {
    pub passed: bool,
    pub no_lost_elements: bool,
    pub no_duplicates: bool,
    pub stats: DstStats,
    pub fault_errors: Vec<String>,
}

impl DstResult {
    pub fn format(&self) -> String {
        let status = if self.passed { "PASS" } else { "FAIL" };
        let mut result = format!("[{}] {}", status, self.stats.format());

        if !self.no_lost_elements {
            result.push_str("\n  VIOLATION: NoLostElements");
        }
        if !self.no_duplicates {
            result.push_str("\n  VIOLATION: NoDuplicates");
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Simple mock stack for testing the DST framework itself
    struct MockStack {
        values: std::sync::Mutex<Vec<u64>>,
    }

    impl DstTestableStack for MockStack {
        fn new() -> Self {
            Self {
                values: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn push(&self, value: u64) {
            self.values.lock().unwrap().push(value);
        }

        fn pop(&self) -> Option<u64> {
            self.values.lock().unwrap().pop()
        }

        fn is_empty(&self) -> bool {
            self.values.lock().unwrap().is_empty()
        }

        fn get_contents(&self) -> Vec<u64> {
            self.values.lock().unwrap().clone()
        }
    }

    #[test]
    fn test_dst_runner_basic() {
        let mut runner: DstRunner<MockStack> = DstRunner::new(12345);

        // These might fail due to fault injection, and that's OK
        let _ = runner.push(1);
        let _ = runner.push(2);
        let _ = runner.pop();

        // Invariants should still hold
        assert!(runner.check_no_lost_elements());
        assert!(runner.check_no_duplicates());
    }

    #[test]
    fn test_dst_scenario() {
        let ops = vec![
            DstOp::Push(100),
            DstOp::Push(200),
            DstOp::Pop,
            DstOp::Push(300),
        ];

        let result = run_dst_scenario::<MockStack>(12345, ops);

        // Even with faults, invariants should hold
        assert!(result.passed, "DST failed: {}", result.format());
    }

    #[test]
    fn test_determinism() {
        let ops = vec![
            DstOp::Push(1),
            DstOp::Push(2),
            DstOp::Pop,
        ];

        let result1 = run_dst_scenario::<MockStack>(42, ops.clone());
        let result2 = run_dst_scenario::<MockStack>(42, ops);

        // Same seed = same faults = same stats
        assert_eq!(result1.stats.faults_injected, result2.stats.faults_injected);
    }
}
