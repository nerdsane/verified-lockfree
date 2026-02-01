//! Oracle-driven scheduler for DST.
//!
//! Replays specific interleavings discovered by stateright model checking.
//! This allows DST to test actual implementations with known-interesting scenarios.
//!
//! # Architecture
//!
//! ```text
//! Stateright Model Checking
//!         │
//!         ▼
//!    Oracle Extraction
//!         │
//!         ▼
//!    OracleScheduler
//!         │
//!         ▼
//!    DST Test Harness
//!         │
//!         ▼
//!    Actual Implementation
//! ```

use crate::scheduler::{ScheduleDecision, Scheduler};
use crate::DeterministicRng;

/// Action type for oracle scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleActionType {
    /// Allocate node for push
    PushAlloc,
    /// Read head for push
    PushReadHead,
    /// CAS for push
    PushCas,
    /// Read head for pop
    PopReadHead,
    /// CAS for pop
    PopCas,
}

/// A single step in an oracle trace.
#[derive(Debug, Clone)]
pub struct OracleStep {
    /// Thread that should execute
    pub thread: usize,
    /// Expected action type
    pub action_type: OracleActionType,
    /// Optional value (for push)
    pub value: Option<u64>,
}

/// Oracle trace to replay.
#[derive(Debug, Clone)]
pub struct OracleTrace {
    /// Name for debugging
    pub name: String,
    /// Steps to replay
    pub steps: Vec<OracleStep>,
    /// Description
    pub description: String,
}

impl OracleTrace {
    /// Create a new oracle trace.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            steps: Vec::new(),
            description: String::new(),
        }
    }

    /// Add a step.
    pub fn add_step(&mut self, thread: usize, action_type: OracleActionType, value: Option<u64>) {
        self.steps.push(OracleStep {
            thread,
            action_type,
            value,
        });
    }

    /// Set description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Build a concurrent push contention oracle.
    ///
    /// Two threads push concurrently, one CAS fails and retries.
    pub fn concurrent_push_contention(t0: usize, t1: usize, v0: u64, v1: u64) -> Self {
        let mut trace = Self::new("concurrent_push_contention")
            .with_description("T0 and T1 push, T1 wins, T0 retries");

        // T0 starts push
        trace.add_step(t0, OracleActionType::PushAlloc, Some(v0));
        trace.add_step(t0, OracleActionType::PushReadHead, None);

        // T1 completes push (interrupting T0)
        trace.add_step(t1, OracleActionType::PushAlloc, Some(v1));
        trace.add_step(t1, OracleActionType::PushReadHead, None);
        trace.add_step(t1, OracleActionType::PushCas, None);

        // T0's CAS fails, retry
        trace.add_step(t0, OracleActionType::PushCas, None); // Will fail
        trace.add_step(t0, OracleActionType::PushReadHead, None);
        trace.add_step(t0, OracleActionType::PushCas, None); // Succeeds

        trace
    }

    /// Build a concurrent pop contention oracle.
    pub fn concurrent_pop_contention(t0: usize, t1: usize) -> Self {
        let mut trace = Self::new("concurrent_pop_contention")
            .with_description("T0 and T1 pop, T1 wins, T0 retries");

        // Both read head
        trace.add_step(t0, OracleActionType::PopReadHead, None);
        trace.add_step(t1, OracleActionType::PopReadHead, None);

        // T1 wins
        trace.add_step(t1, OracleActionType::PopCas, None);

        // T0 fails, retry
        trace.add_step(t0, OracleActionType::PopCas, None);
        trace.add_step(t0, OracleActionType::PopReadHead, None);
        trace.add_step(t0, OracleActionType::PopCas, None);

        trace
    }

    /// Build a push-pop race oracle.
    pub fn push_pop_race(pusher: usize, popper: usize, value: u64) -> Self {
        let mut trace = Self::new("push_pop_race")
            .with_description("Push and pop racing on same element");

        // Pusher starts
        trace.add_step(pusher, OracleActionType::PushAlloc, Some(value));
        trace.add_step(pusher, OracleActionType::PushReadHead, None);

        // Popper tries to pop (empty or stale head)
        trace.add_step(popper, OracleActionType::PopReadHead, None);

        // Pusher completes
        trace.add_step(pusher, OracleActionType::PushCas, None);

        // Popper's CAS may fail or succeed depending on initial state
        trace.add_step(popper, OracleActionType::PopCas, None);

        trace
    }
}

/// Scheduler that replays oracle traces.
///
/// Combines oracle-driven scheduling with fallback to random scheduling.
pub struct OracleScheduler {
    /// Current oracle being replayed
    oracle: Option<OracleTrace>,
    /// Current position in oracle
    oracle_position: usize,
    /// Fallback scheduler for non-oracle decisions
    fallback: Scheduler,
    /// Number of oracle steps executed
    oracle_steps_executed: u64,
    /// Number of fallback decisions
    fallback_decisions: u64,
}

impl OracleScheduler {
    /// Create a new oracle scheduler.
    ///
    /// # Arguments
    /// - `oracle`: Optional oracle trace to replay
    /// - `seed`: Seed for fallback scheduler
    /// - `threads_count`: Number of threads
    pub fn new(oracle: Option<OracleTrace>, seed: u64, threads_count: usize) -> Self {
        let rng = DeterministicRng::new(seed);
        let fallback = Scheduler::new(rng, threads_count, 0.2);

        Self {
            oracle,
            oracle_position: 0,
            fallback,
            oracle_steps_executed: 0,
            fallback_decisions: 0,
        }
    }

    /// Create with just an oracle (no fallback randomness).
    pub fn from_oracle(oracle: OracleTrace, threads_count: usize) -> Self {
        Self::new(Some(oracle), 12345, threads_count)
    }

    /// Get current thread according to oracle or fallback.
    pub fn current_thread(&self) -> usize {
        if let Some(ref oracle) = self.oracle {
            if self.oracle_position < oracle.steps.len() {
                return oracle.steps[self.oracle_position].thread;
            }
        }
        self.fallback.current_thread()
    }

    /// Get the next expected action type (if following oracle).
    pub fn expected_action(&self) -> Option<OracleActionType> {
        self.oracle.as_ref().and_then(|o| {
            o.steps.get(self.oracle_position).map(|s| s.action_type)
        })
    }

    /// Get the expected value for current step (if any).
    pub fn expected_value(&self) -> Option<u64> {
        self.oracle.as_ref().and_then(|o| {
            o.steps.get(self.oracle_position).and_then(|s| s.value)
        })
    }

    /// Advance to next oracle step.
    ///
    /// Call this after an action completes that matches the oracle.
    pub fn advance(&mut self) {
        if self.oracle.is_some() && self.oracle_position < self.oracle.as_ref().unwrap().steps.len()
        {
            self.oracle_position += 1;
            self.oracle_steps_executed += 1;
        }
    }

    /// Check if we're still following the oracle.
    pub fn is_following_oracle(&self) -> bool {
        self.oracle.as_ref().map_or(false, |o| {
            self.oracle_position < o.steps.len()
        })
    }

    /// Check if oracle is complete.
    pub fn oracle_complete(&self) -> bool {
        self.oracle.as_ref().map_or(true, |o| {
            self.oracle_position >= o.steps.len()
        })
    }

    /// Make a scheduling decision.
    ///
    /// If following oracle, returns the thread specified by oracle.
    /// Otherwise, uses fallback scheduler.
    pub fn decide(&mut self) -> ScheduleDecision {
        if self.is_following_oracle() {
            let thread = self.oracle.as_ref().unwrap().steps[self.oracle_position].thread;
            ScheduleDecision::SwitchTo(thread)
        } else {
            self.fallback_decisions += 1;
            self.fallback.decide()
        }
    }

    /// Get statistics.
    pub fn stats(&self) -> OracleSchedulerStats {
        OracleSchedulerStats {
            oracle_name: self.oracle.as_ref().map(|o| o.name.clone()),
            oracle_steps_total: self.oracle.as_ref().map_or(0, |o| o.steps.len()),
            oracle_steps_executed: self.oracle_steps_executed,
            fallback_decisions: self.fallback_decisions,
            oracle_complete: self.oracle_complete(),
        }
    }
}

/// Statistics from oracle scheduler.
#[derive(Debug, Clone)]
pub struct OracleSchedulerStats {
    /// Name of the oracle (if any)
    pub oracle_name: Option<String>,
    /// Total steps in oracle
    pub oracle_steps_total: usize,
    /// Oracle steps executed
    pub oracle_steps_executed: u64,
    /// Fallback decisions made
    pub fallback_decisions: u64,
    /// Whether oracle completed
    pub oracle_complete: bool,
}

impl OracleSchedulerStats {
    /// Format for display.
    pub fn format(&self) -> String {
        match &self.oracle_name {
            Some(name) => format!(
                "Oracle '{}': {}/{} steps, {} fallback, complete={}",
                name,
                self.oracle_steps_executed,
                self.oracle_steps_total,
                self.fallback_decisions,
                self.oracle_complete
            ),
            None => format!(
                "Random scheduling: {} decisions",
                self.fallback_decisions
            ),
        }
    }
}

/// Pre-built oracles for common CAS scenarios.
pub mod scenarios {
    use super::*;

    /// All standard CAS contention scenarios.
    pub fn all_cas_scenarios() -> Vec<OracleTrace> {
        vec![
            OracleTrace::concurrent_push_contention(0, 1, 100, 200),
            OracleTrace::concurrent_pop_contention(0, 1),
            OracleTrace::push_pop_race(0, 1, 100),
            multiple_cas_failures(),
            aba_pattern(),
        ]
    }

    /// Multiple CAS failures scenario.
    pub fn multiple_cas_failures() -> OracleTrace {
        let mut trace = OracleTrace::new("multiple_cas_failures")
            .with_description("Thread 0 fails CAS 3 times before succeeding");

        // T0 starts
        trace.add_step(0, OracleActionType::PushAlloc, Some(100));
        trace.add_step(0, OracleActionType::PushReadHead, None);

        // T1 interrupts
        trace.add_step(1, OracleActionType::PushAlloc, Some(200));
        trace.add_step(1, OracleActionType::PushReadHead, None);
        trace.add_step(1, OracleActionType::PushCas, None);

        // T0 fails, retry
        trace.add_step(0, OracleActionType::PushCas, None);
        trace.add_step(0, OracleActionType::PushReadHead, None);

        // T2 interrupts
        trace.add_step(2, OracleActionType::PushAlloc, Some(300));
        trace.add_step(2, OracleActionType::PushReadHead, None);
        trace.add_step(2, OracleActionType::PushCas, None);

        // T0 fails again, retry
        trace.add_step(0, OracleActionType::PushCas, None);
        trace.add_step(0, OracleActionType::PushReadHead, None);

        // T3 interrupts (if available, otherwise T1 again)
        trace.add_step(1, OracleActionType::PushAlloc, Some(400));
        trace.add_step(1, OracleActionType::PushReadHead, None);
        trace.add_step(1, OracleActionType::PushCas, None);

        // T0 fails third time
        trace.add_step(0, OracleActionType::PushCas, None);
        trace.add_step(0, OracleActionType::PushReadHead, None);
        trace.add_step(0, OracleActionType::PushCas, None); // Finally succeeds

        trace
    }

    /// ABA problem pattern (for testing epoch GC protection).
    pub fn aba_pattern() -> OracleTrace {
        let mut trace = OracleTrace::new("aba_pattern")
            .with_description("Classic ABA: push A, pop A, push B at same address");

        // T0 starts pop, reads head pointing to A
        trace.add_step(0, OracleActionType::PopReadHead, None);

        // T1 completes pop of A
        trace.add_step(1, OracleActionType::PopReadHead, None);
        trace.add_step(1, OracleActionType::PopCas, None);

        // T2 pushes B (might reuse A's address without epoch GC)
        trace.add_step(2, OracleActionType::PushAlloc, Some(999));
        trace.add_step(2, OracleActionType::PushReadHead, None);
        trace.add_step(2, OracleActionType::PushCas, None);

        // T0's CAS - with epoch GC, address won't match
        // Without epoch GC, this could succeed incorrectly
        trace.add_step(0, OracleActionType::PopCas, None);

        trace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oracle_trace_creation() {
        let trace = OracleTrace::concurrent_push_contention(0, 1, 42, 43);
        assert_eq!(trace.name, "concurrent_push_contention");
        assert!(!trace.steps.is_empty());
    }

    #[test]
    fn test_oracle_scheduler_follows_trace() {
        let trace = OracleTrace::concurrent_push_contention(0, 1, 100, 200);
        let mut scheduler = OracleScheduler::from_oracle(trace, 2);

        // Should start with T0 (push alloc)
        assert_eq!(scheduler.current_thread(), 0);
        assert_eq!(
            scheduler.expected_action(),
            Some(OracleActionType::PushAlloc)
        );
        assert_eq!(scheduler.expected_value(), Some(100));

        // Advance
        scheduler.advance();
        assert_eq!(scheduler.expected_action(), Some(OracleActionType::PushReadHead));
    }

    #[test]
    fn test_oracle_scheduler_completion() {
        let mut trace = OracleTrace::new("simple");
        trace.add_step(0, OracleActionType::PushAlloc, Some(1));
        trace.add_step(0, OracleActionType::PushCas, None);

        let mut scheduler = OracleScheduler::from_oracle(trace, 2);

        assert!(!scheduler.oracle_complete());
        scheduler.advance();
        assert!(!scheduler.oracle_complete());
        scheduler.advance();
        assert!(scheduler.oracle_complete());
    }

    #[test]
    fn test_all_scenarios() {
        let scenarios = scenarios::all_cas_scenarios();
        assert!(scenarios.len() >= 4);

        for scenario in scenarios {
            assert!(!scenario.steps.is_empty());
        }
    }

    #[test]
    fn test_stats() {
        let trace = OracleTrace::concurrent_push_contention(0, 1, 1, 2);
        let steps = trace.steps.len();
        let mut scheduler = OracleScheduler::from_oracle(trace, 2);

        for _ in 0..3 {
            scheduler.advance();
        }

        let stats = scheduler.stats();
        assert_eq!(stats.oracle_steps_total, steps);
        assert_eq!(stats.oracle_steps_executed, 3);
    }
}
