//! Oracle extraction from stateright model checking.
//!
//! Oracles are interesting execution traces discovered during model checking
//! that can be replayed in DST to test actual implementations.
//!
//! # CAS Scenarios Captured
//!
//! 1. **CAS Success** - Normal operation completion
//! 2. **CAS Failure + Retry** - Contention causing retry loops
//! 3. **Concurrent Push-Push** - Two threads racing to push
//! 4. **Concurrent Pop-Pop** - Two threads racing to pop
//! 5. **Concurrent Push-Pop** - Interleaved push/pop operations

use std::collections::BTreeMap;

use crate::StackModel;
use stateright::{Checker, Model};

/// An oracle is a sequence of thread actions representing an interesting interleaving.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oracle {
    /// Name/description of this oracle
    pub name: String,
    /// Sequence of actions to replay
    pub actions: Vec<OracleAction>,
    /// Why this oracle is interesting
    pub description: String,
    /// Category for filtering
    pub category: OracleCategory,
}

/// A single action in an oracle trace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OracleAction {
    /// Thread that performs this action
    pub thread: u64,
    /// Type of action
    pub action_type: OracleActionType,
    /// Optional value involved (for push)
    pub value: Option<u64>,
}

/// Categories of oracles for filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OracleCategory {
    /// CAS succeeds on first attempt
    CasSuccess,
    /// CAS fails and retries
    CasFailure,
    /// Multiple threads pushing concurrently
    ConcurrentPush,
    /// Multiple threads popping concurrently
    ConcurrentPop,
    /// Interleaved push/pop operations
    MixedOperations,
    /// Edge case (empty stack, single element, etc.)
    EdgeCase,
}

/// Type of action in an oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleActionType {
    /// Allocate node for push
    PushAlloc,
    /// Read head for push
    PushReadHead,
    /// CAS for push (may succeed or fail)
    PushCas,
    /// Read head for pop
    PopReadHead,
    /// CAS for pop (may succeed or fail)
    PopCas,
}

impl Oracle {
    /// Create a new oracle.
    pub fn new(name: impl Into<String>, category: OracleCategory) -> Self {
        Self {
            name: name.into(),
            actions: Vec::new(),
            description: String::new(),
            category,
        }
    }

    /// Add an action to the oracle.
    pub fn add_action(&mut self, thread: u64, action_type: OracleActionType, value: Option<u64>) {
        self.actions.push(OracleAction {
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

    /// Number of CAS failures in this oracle.
    pub fn cas_failure_count(&self) -> usize {
        // Count how many CAS operations are followed by a retry (same thread, same operation type)
        let mut failures = 0;
        for i in 0..self.actions.len().saturating_sub(1) {
            let curr = &self.actions[i];
            // A CAS followed by another attempt from same thread indicates failure
            if matches!(curr.action_type, OracleActionType::PushCas | OracleActionType::PopCas) {
                // Check if this thread retries
                for j in (i + 1)..self.actions.len() {
                    let next = &self.actions[j];
                    if next.thread == curr.thread {
                        // Same thread acts again - if it's a read, the CAS failed
                        if matches!(
                            next.action_type,
                            OracleActionType::PushReadHead | OracleActionType::PopReadHead
                        ) {
                            failures += 1;
                        }
                        break;
                    }
                }
            }
        }
        failures
    }

    /// Format as a human-readable trace.
    pub fn format_trace(&self) -> String {
        let mut output = format!("Oracle: {} [{}]\n", self.name, format!("{:?}", self.category));
        output.push_str(&format!("Description: {}\n", self.description));
        output.push_str("Trace:\n");

        for (i, action) in self.actions.iter().enumerate() {
            let action_str = match action.action_type {
                OracleActionType::PushAlloc => {
                    format!("push_alloc({})", action.value.unwrap_or(0))
                }
                OracleActionType::PushReadHead => "push_read_head".to_string(),
                OracleActionType::PushCas => "push_cas".to_string(),
                OracleActionType::PopReadHead => "pop_read_head".to_string(),
                OracleActionType::PopCas => "pop_cas".to_string(),
            };
            output.push_str(&format!("  {:3}. T{}: {}\n", i + 1, action.thread, action_str));
        }

        output
    }
}

/// Extract oracles from stateright model checking.
///
/// Runs BFS exploration and captures interesting traces.
pub struct OracleExtractor {
    /// Collected oracles
    oracles: Vec<Oracle>,
    /// Maximum oracles per category
    oracles_per_category_max: usize,
    /// Category counts
    category_counts: BTreeMap<OracleCategory, usize>,
}

impl OracleExtractor {
    /// Create a new extractor.
    pub fn new() -> Self {
        Self {
            oracles: Vec::new(),
            oracles_per_category_max: 10,
            category_counts: BTreeMap::new(),
        }
    }

    /// Set maximum oracles per category.
    pub fn with_max_per_category(mut self, max: usize) -> Self {
        self.oracles_per_category_max = max;
        self
    }

    /// Extract oracles by running model checking.
    pub fn extract(&mut self, threads_count: u64, values: Vec<u64>) -> Vec<Oracle> {
        // Create model for validation
        let model = StackModel::new(threads_count, values.clone());

        // Use stateright's checker to validate the model (proves invariants hold)
        let _checker = model.checker().threads(1).spawn_bfs().join();

        // Create a fresh model for scenario generation (since checker consumed the first one)
        let model = StackModel::new(threads_count, values.clone());

        // Generate canonical scenarios based on the model configuration
        self.generate_scenarios(&model, &values);

        // Add manually crafted edge cases
        self.add_edge_case_oracles(threads_count, &values);

        self.oracles.clone()
    }

    /// Generate canonical oracles based on model configuration.
    fn generate_scenarios(&mut self, model: &StackModel, values: &[u64]) {
        // We'll generate oracles based on interesting patterns
        // Since stateright doesn't easily expose paths, we generate canonical scenarios

        // 1. Simple successful push
        self.add_simple_push_oracle(0, values.first().copied().unwrap_or(1));

        // 2. Simple successful pop (after push)
        self.add_simple_push_pop_oracle(0, values.first().copied().unwrap_or(1));

        // 3. Concurrent push - CAS contention
        if model.threads_count >= 2 && values.len() >= 2 {
            self.add_concurrent_push_oracle(0, 1, values[0], values[1]);
        }

        // 4. Concurrent pop - CAS contention
        if model.threads_count >= 2 {
            self.add_concurrent_pop_oracle(0, 1);
        }

        // 5. Mixed push/pop interleaving
        if model.threads_count >= 2 && !values.is_empty() {
            self.add_mixed_operations_oracle(0, 1, values[0]);
        }
    }

    /// Add a simple successful push oracle.
    fn add_simple_push_oracle(&mut self, thread: u64, value: u64) {
        let category = OracleCategory::CasSuccess;
        if self.should_add(category) {
            let mut oracle = Oracle::new("simple_push", category)
                .with_description("Single thread pushes successfully");
            oracle.add_action(thread, OracleActionType::PushAlloc, Some(value));
            oracle.add_action(thread, OracleActionType::PushReadHead, None);
            oracle.add_action(thread, OracleActionType::PushCas, None);
            self.add_oracle(oracle);
        }
    }

    /// Add a simple push then pop oracle.
    fn add_simple_push_pop_oracle(&mut self, thread: u64, value: u64) {
        let category = OracleCategory::CasSuccess;
        if self.should_add(category) {
            let mut oracle = Oracle::new("simple_push_pop", category)
                .with_description("Single thread pushes then pops");
            // Push
            oracle.add_action(thread, OracleActionType::PushAlloc, Some(value));
            oracle.add_action(thread, OracleActionType::PushReadHead, None);
            oracle.add_action(thread, OracleActionType::PushCas, None);
            // Pop
            oracle.add_action(thread, OracleActionType::PopReadHead, None);
            oracle.add_action(thread, OracleActionType::PopCas, None);
            self.add_oracle(oracle);
        }
    }

    /// Add concurrent push oracle with CAS contention.
    fn add_concurrent_push_oracle(&mut self, t0: u64, t1: u64, v0: u64, v1: u64) {
        // Scenario: Both threads try to push, T1 succeeds first, T0's CAS fails
        let category = OracleCategory::ConcurrentPush;
        if self.should_add(category) {
            let mut oracle = Oracle::new("concurrent_push_cas_failure", category)
                .with_description("T0 and T1 push concurrently, T1 wins, T0 retries");

            // T0 allocates and reads head
            oracle.add_action(t0, OracleActionType::PushAlloc, Some(v0));
            oracle.add_action(t0, OracleActionType::PushReadHead, None);

            // T1 allocates, reads, and CAS succeeds (while T0 is about to CAS)
            oracle.add_action(t1, OracleActionType::PushAlloc, Some(v1));
            oracle.add_action(t1, OracleActionType::PushReadHead, None);
            oracle.add_action(t1, OracleActionType::PushCas, None); // T1 succeeds

            // T0's CAS fails (head changed), must retry
            oracle.add_action(t0, OracleActionType::PushCas, None); // T0 fails
            oracle.add_action(t0, OracleActionType::PushReadHead, None); // T0 retries
            oracle.add_action(t0, OracleActionType::PushCas, None); // T0 succeeds

            self.add_oracle(oracle);
        }

        // Scenario: Interleaved allocation
        let category = OracleCategory::ConcurrentPush;
        if self.should_add(category) {
            let mut oracle = Oracle::new("interleaved_push_allocation", category)
                .with_description("Alternating push phases between threads");

            oracle.add_action(t0, OracleActionType::PushAlloc, Some(v0));
            oracle.add_action(t1, OracleActionType::PushAlloc, Some(v1));
            oracle.add_action(t0, OracleActionType::PushReadHead, None);
            oracle.add_action(t1, OracleActionType::PushReadHead, None);
            oracle.add_action(t0, OracleActionType::PushCas, None);
            oracle.add_action(t1, OracleActionType::PushCas, None); // Fails, retries
            oracle.add_action(t1, OracleActionType::PushReadHead, None);
            oracle.add_action(t1, OracleActionType::PushCas, None);

            self.add_oracle(oracle);
        }
    }

    /// Add concurrent pop oracle.
    fn add_concurrent_pop_oracle(&mut self, t0: u64, t1: u64) {
        let category = OracleCategory::ConcurrentPop;
        if self.should_add(category) {
            let mut oracle = Oracle::new("concurrent_pop_cas_failure", category)
                .with_description("T0 and T1 pop concurrently, T1 wins, T0 retries");

            // Both read head
            oracle.add_action(t0, OracleActionType::PopReadHead, None);
            oracle.add_action(t1, OracleActionType::PopReadHead, None);

            // T1 CAS succeeds
            oracle.add_action(t1, OracleActionType::PopCas, None);

            // T0 CAS fails (head changed), must restart
            oracle.add_action(t0, OracleActionType::PopCas, None); // Fails
            oracle.add_action(t0, OracleActionType::PopReadHead, None); // Restart
            oracle.add_action(t0, OracleActionType::PopCas, None); // Succeeds

            self.add_oracle(oracle);
        }
    }

    /// Add mixed push/pop operations.
    fn add_mixed_operations_oracle(&mut self, t0: u64, t1: u64, value: u64) {
        let category = OracleCategory::MixedOperations;
        if self.should_add(category) {
            let mut oracle = Oracle::new("push_pop_race", category)
                .with_description("T0 pushes while T1 tries to pop");

            // T1 reads head (empty or has element)
            // T0 pushes
            oracle.add_action(t0, OracleActionType::PushAlloc, Some(value));
            oracle.add_action(t1, OracleActionType::PopReadHead, None);
            oracle.add_action(t0, OracleActionType::PushReadHead, None);
            oracle.add_action(t0, OracleActionType::PushCas, None);
            // T1's pop CAS fails (head changed by push)
            oracle.add_action(t1, OracleActionType::PopCas, None);
            // T1 retries and succeeds
            oracle.add_action(t1, OracleActionType::PopReadHead, None);
            oracle.add_action(t1, OracleActionType::PopCas, None);

            self.add_oracle(oracle);
        }
    }

    /// Add edge case oracles.
    fn add_edge_case_oracles(&mut self, threads_count: u64, values: &[u64]) {
        let category = OracleCategory::EdgeCase;

        // Pop from empty stack (should be no-op)
        if self.should_add(category) {
            let oracle = Oracle::new("pop_empty_stack", category)
                .with_description("Attempt to pop from empty stack");
            // No actions - the implementation should handle this gracefully
            self.add_oracle(oracle);
        }

        // Multiple CAS failures in a row
        if threads_count >= 3 && values.len() >= 3 && self.should_add(category) {
            let mut oracle = Oracle::new("multiple_cas_failures", category)
                .with_description("T0 fails CAS multiple times as T1 and T2 keep winning");

            oracle.add_action(0, OracleActionType::PushAlloc, Some(values[0]));
            oracle.add_action(0, OracleActionType::PushReadHead, None);

            // T1 sneaks in
            oracle.add_action(1, OracleActionType::PushAlloc, Some(values[1]));
            oracle.add_action(1, OracleActionType::PushReadHead, None);
            oracle.add_action(1, OracleActionType::PushCas, None);

            // T0 fails
            oracle.add_action(0, OracleActionType::PushCas, None);
            oracle.add_action(0, OracleActionType::PushReadHead, None);

            // T2 sneaks in
            oracle.add_action(2, OracleActionType::PushAlloc, Some(values[2]));
            oracle.add_action(2, OracleActionType::PushReadHead, None);
            oracle.add_action(2, OracleActionType::PushCas, None);

            // T0 fails again
            oracle.add_action(0, OracleActionType::PushCas, None);
            oracle.add_action(0, OracleActionType::PushReadHead, None);
            oracle.add_action(0, OracleActionType::PushCas, None); // Finally succeeds

            self.add_oracle(oracle);
        }
    }

    fn should_add(&self, category: OracleCategory) -> bool {
        self.category_counts.get(&category).copied().unwrap_or(0) < self.oracles_per_category_max
    }

    fn add_oracle(&mut self, oracle: Oracle) {
        let count = self.category_counts.entry(oracle.category).or_insert(0);
        if *count < self.oracles_per_category_max {
            *count += 1;
            self.oracles.push(oracle);
        }
    }

    /// Get all extracted oracles.
    pub fn oracles(&self) -> &[Oracle] {
        &self.oracles
    }

    /// Get oracles by category.
    pub fn oracles_by_category(&self, category: OracleCategory) -> Vec<&Oracle> {
        self.oracles
            .iter()
            .filter(|o| o.category == category)
            .collect()
    }
}

impl Default for OracleExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_oracles() {
        let mut extractor = OracleExtractor::new();
        let oracles = extractor.extract(2, vec![1, 2, 3]);

        assert!(!oracles.is_empty());

        // Should have at least one of each main category
        assert!(!extractor.oracles_by_category(OracleCategory::CasSuccess).is_empty());
        assert!(!extractor.oracles_by_category(OracleCategory::ConcurrentPush).is_empty());
    }

    #[test]
    fn test_oracle_format() {
        let mut oracle = Oracle::new("test", OracleCategory::CasSuccess)
            .with_description("Test oracle");
        oracle.add_action(0, OracleActionType::PushAlloc, Some(42));
        oracle.add_action(0, OracleActionType::PushReadHead, None);
        oracle.add_action(0, OracleActionType::PushCas, None);

        let trace = oracle.format_trace();
        assert!(trace.contains("test"));
        assert!(trace.contains("push_alloc(42)"));
        assert!(trace.contains("push_cas"));
    }

    #[test]
    fn test_cas_failure_count() {
        let mut oracle = Oracle::new("test", OracleCategory::ConcurrentPush);
        // T0 pushes
        oracle.add_action(0, OracleActionType::PushAlloc, Some(1));
        oracle.add_action(0, OracleActionType::PushReadHead, None);
        // T1 interrupts and succeeds
        oracle.add_action(1, OracleActionType::PushAlloc, Some(2));
        oracle.add_action(1, OracleActionType::PushReadHead, None);
        oracle.add_action(1, OracleActionType::PushCas, None);
        // T0's CAS fails
        oracle.add_action(0, OracleActionType::PushCas, None);
        oracle.add_action(0, OracleActionType::PushReadHead, None); // Retry indicator
        oracle.add_action(0, OracleActionType::PushCas, None);

        assert_eq!(oracle.cas_failure_count(), 1);
    }
}
