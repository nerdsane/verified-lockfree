//! DST-instrumented traits for lock-free data structures.
//!
//! Generated code must implement these traits to be testable with DST oracles.
//! The key insight: yield points at CAS operations allow DST to control interleaving.
//!
//! # How it works
//!
//! 1. Generator produces code implementing `InstrumentedStack`
//! 2. Each CAS operation calls `yield_point()` before attempting
//! 3. DST scheduler can force context switches at these points
//! 4. Oracles replay specific interleavings discovered by stateright
//!
//! # Example generated code pattern
//!
//! ```ignore
//! fn push(&self, value: u64, ctx: &mut DstContext) {
//!     let node = self.alloc_node(value);
//!     ctx.yield_point(YieldPoint::PushAlloc);  // DST can switch here
//!
//!     loop {
//!         let head = self.head.load(Ordering::Acquire);
//!         node.next.store(head, Ordering::Relaxed);
//!         ctx.yield_point(YieldPoint::PushReadHead);  // DST can switch here
//!
//!         if self.head.compare_exchange(head, node, ...).is_ok() {
//!             ctx.record_cas_success();
//!             break;
//!         }
//!         ctx.record_cas_failure();  // CAS failed, retry
//!         ctx.yield_point(YieldPoint::PushCasRetry);
//!     }
//! }
//! ```

use std::collections::HashSet;

use crate::oracle_scheduler::{OracleActionType, OracleScheduler};
use crate::scheduler::ScheduleDecision;

/// Yield point types matching oracle actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YieldPoint {
    /// After allocating node for push
    PushAlloc,
    /// After reading head for push
    PushReadHead,
    /// After CAS attempt for push (success or failure)
    PushCas,
    /// After reading head for pop
    PopReadHead,
    /// After CAS attempt for pop
    PopCas,
}

impl YieldPoint {
    /// Convert to oracle action type.
    pub fn to_oracle_action(&self) -> OracleActionType {
        match self {
            YieldPoint::PushAlloc => OracleActionType::PushAlloc,
            YieldPoint::PushReadHead => OracleActionType::PushReadHead,
            YieldPoint::PushCas => OracleActionType::PushCas,
            YieldPoint::PopReadHead => OracleActionType::PopReadHead,
            YieldPoint::PopCas => OracleActionType::PopCas,
        }
    }
}

/// DST context passed to instrumented operations.
///
/// Tracks thread state and interfaces with the oracle scheduler.
pub struct DstContext<'a> {
    /// Thread ID
    pub thread_id: usize,
    /// Oracle scheduler
    scheduler: &'a mut OracleScheduler,
    /// CAS successes
    cas_successes: u64,
    /// CAS failures
    cas_failures: u64,
    /// Current operation value (for push)
    current_value: Option<u64>,
}

impl<'a> DstContext<'a> {
    /// Create a new context.
    pub fn new(thread_id: usize, scheduler: &'a mut OracleScheduler) -> Self {
        Self {
            thread_id,
            scheduler,
            cas_successes: 0,
            cas_failures: 0,
            current_value: None,
        }
    }

    /// Set the value being pushed (for oracle matching).
    pub fn set_value(&mut self, value: u64) {
        self.current_value = Some(value);
    }

    /// Yield point - allows DST to control scheduling.
    ///
    /// Returns true if this thread should continue, false if it should yield.
    pub fn yield_point(&mut self, point: YieldPoint) -> bool {
        // Check if oracle expects this action from this thread
        if self.scheduler.is_following_oracle() {
            let expected_thread = self.scheduler.current_thread();
            let expected_action = self.scheduler.expected_action();

            // Verify we're following the oracle
            if self.thread_id != expected_thread {
                // Wrong thread - should yield
                return false;
            }

            if let Some(expected) = expected_action {
                if expected != point.to_oracle_action() {
                    // Wrong action - oracle mismatch
                    // This shouldn't happen in correct code
                    panic!(
                        "Oracle mismatch: T{} at {:?}, expected {:?}",
                        self.thread_id, point, expected
                    );
                }
            }

            // Advance oracle
            self.scheduler.advance();
        }

        // Make scheduling decision
        let decision = self.scheduler.decide();
        match decision {
            ScheduleDecision::Continue => true,
            ScheduleDecision::SwitchTo(t) if t == self.thread_id => true,
            _ => false,
        }
    }

    /// Record a successful CAS.
    pub fn record_cas_success(&mut self) {
        self.cas_successes += 1;
    }

    /// Record a failed CAS.
    pub fn record_cas_failure(&mut self) {
        self.cas_failures += 1;
    }

    /// Get CAS statistics.
    pub fn cas_stats(&self) -> (u64, u64) {
        (self.cas_successes, self.cas_failures)
    }
}

/// Trait for DST-instrumented stack implementations.
///
/// Generated code must implement this trait to be testable with DST oracles.
pub trait InstrumentedStack: Send + Sync {
    /// Create a new empty stack.
    fn new() -> Self;

    /// Push a value with DST instrumentation.
    fn push_instrumented(&self, value: u64, ctx: &mut DstContext);

    /// Pop a value with DST instrumentation.
    fn pop_instrumented(&self, ctx: &mut DstContext) -> Option<u64>;

    /// Check if empty.
    fn is_empty(&self) -> bool;

    /// Get all pushed elements (for invariant checking).
    fn pushed_elements(&self) -> HashSet<u64>;

    /// Get all popped elements (for invariant checking).
    fn popped_elements(&self) -> HashSet<u64>;

    /// Get current stack contents (for invariant checking).
    fn get_contents(&self) -> Vec<u64>;

    /// Check NoLostElements invariant.
    fn check_no_lost_elements(&self) -> bool {
        let pushed = self.pushed_elements();
        let popped = self.popped_elements();
        let contents: HashSet<_> = self.get_contents().into_iter().collect();

        for elem in pushed {
            if !contents.contains(&elem) && !popped.contains(&elem) {
                return false;
            }
        }
        true
    }

    /// Check NoDuplicates invariant.
    fn check_no_duplicates(&self) -> bool {
        let contents = self.get_contents();
        let unique: HashSet<_> = contents.iter().collect();
        contents.len() == unique.len()
    }
}

/// Run DST with an oracle on an instrumented stack.
///
/// This is the core DST runner that replays oracle scenarios.
pub fn run_oracle_dst<S: InstrumentedStack>(
    oracle: crate::OracleTrace,
    threads_count: usize,
    operations: Vec<DstOperation>,
) -> DstResult {
    let mut scheduler = OracleScheduler::from_oracle(oracle.clone(), threads_count);
    let stack = S::new();

    let mut thread_ops: Vec<_> = (0..threads_count)
        .map(|_| operations.clone().into_iter())
        .collect();

    let mut completed_ops = 0u64;
    let mut invariant_violations = Vec::new();

    // Execute operations following oracle schedule
    while !scheduler.oracle_complete() {
        let thread = scheduler.current_thread();

        if let Some(op) = thread_ops[thread].next() {
            let mut ctx = DstContext::new(thread, &mut scheduler);

            match op {
                DstOperation::Push(value) => {
                    ctx.set_value(value);
                    stack.push_instrumented(value, &mut ctx);
                }
                DstOperation::Pop => {
                    stack.pop_instrumented(&mut ctx);
                }
            }

            completed_ops += 1;

            // Check invariants periodically
            if completed_ops % 10 == 0 {
                if !stack.check_no_lost_elements() {
                    invariant_violations.push("NoLostElements violated".to_string());
                }
                if !stack.check_no_duplicates() {
                    invariant_violations.push("NoDuplicates violated".to_string());
                }
            }
        } else {
            // Thread has no more operations, advance scheduler
            scheduler.advance();
        }
    }

    // Final invariant check
    if !stack.check_no_lost_elements() {
        invariant_violations.push("NoLostElements violated (final)".to_string());
    }
    if !stack.check_no_duplicates() {
        invariant_violations.push("NoDuplicates violated (final)".to_string());
    }

    DstResult {
        oracle_name: oracle.name,
        completed_operations: completed_ops,
        invariant_violations,
        scheduler_stats: scheduler.stats(),
    }
}

/// Operation for DST execution.
#[derive(Debug, Clone)]
pub enum DstOperation {
    Push(u64),
    Pop,
}

/// Result of DST execution.
#[derive(Debug)]
pub struct DstResult {
    pub oracle_name: String,
    pub completed_operations: u64,
    pub invariant_violations: Vec<String>,
    pub scheduler_stats: crate::OracleSchedulerStats,
}

impl DstResult {
    /// Check if DST passed (no invariant violations).
    pub fn passed(&self) -> bool {
        self.invariant_violations.is_empty()
    }

    /// Format for display.
    pub fn format(&self) -> String {
        let status = if self.passed() { "PASS" } else { "FAIL" };
        let mut result = format!(
            "[{}] Oracle '{}': {} ops, {}",
            status,
            self.oracle_name,
            self.completed_operations,
            self.scheduler_stats.format()
        );

        if !self.invariant_violations.is_empty() {
            result.push_str("\nViolations:");
            for v in &self.invariant_violations {
                result.push_str(&format!("\n  - {}", v));
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yield_point_conversion() {
        assert_eq!(
            YieldPoint::PushAlloc.to_oracle_action(),
            OracleActionType::PushAlloc
        );
        assert_eq!(
            YieldPoint::PopCas.to_oracle_action(),
            OracleActionType::PopCas
        );
    }
}
