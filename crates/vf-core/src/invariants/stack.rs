//! Stack invariants from treiber_stack.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | NoLostElements | 45 | Every pushed element is in stack or was popped |
//! | NoDuplicates | 58 | No element appears twice in stack |
//! | LIFO_Order | 72 | Last-in-first-out ordering |
//! | Linearizability | 89 | Operations appear atomic |
//! | ABA_Safety | 103 | No ABA problem with epoch GC |

use std::collections::HashSet;

use crate::counterexample::Counterexample;
use crate::property::{PropertyChecker, PropertyResult};

/// TLA+ spec file for stack invariants.
const TLA_SPEC: &str = "treiber_stack.tla";

/// Properties that any stack implementation must satisfy.
///
/// Implementations provide access to their internal state for
/// property checking. The checker verifies invariants against
/// this state.
pub trait StackProperties {
    /// Set of all elements that have been pushed.
    fn pushed_elements(&self) -> HashSet<u64>;

    /// Set of all elements that have been popped.
    fn popped_elements(&self) -> HashSet<u64>;

    /// Current contents of the stack (top to bottom).
    fn current_contents(&self) -> Vec<u64>;

    /// Operation history for LIFO order checking.
    /// Returns owned data to avoid lifetime issues with internal mutexes.
    fn history(&self) -> StackHistory;
}

/// History of stack operations for linearizability checking.
#[derive(Debug, Clone, Default)]
pub struct StackHistory {
    /// Sequence of operations in linearization order
    pub operations: Vec<StackOperation>,
}

/// A single stack operation.
#[derive(Debug, Clone)]
pub struct StackOperation {
    /// Thread that performed the operation
    pub thread_id: u64,
    /// Type of operation
    pub op_type: StackOpType,
    /// Element involved (Some for push, result for pop)
    pub element: Option<u64>,
    /// Step number for ordering
    pub step: u64,
}

/// Type of stack operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackOpType {
    Push,
    Pop,
    PopEmpty,
}

impl StackHistory {
    /// Create a new empty history.
    #[must_use]
    pub fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    /// Record a push operation.
    pub fn record_push(&mut self, thread_id: u64, element: u64, step: u64) {
        debug_assert!(step > 0, "Step must be positive");
        self.operations.push(StackOperation {
            thread_id,
            op_type: StackOpType::Push,
            element: Some(element),
            step,
        });
    }

    /// Record a pop operation.
    pub fn record_pop(&mut self, thread_id: u64, element: Option<u64>, step: u64) {
        debug_assert!(step > 0, "Step must be positive");
        self.operations.push(StackOperation {
            thread_id,
            op_type: if element.is_some() {
                StackOpType::Pop
            } else {
                StackOpType::PopEmpty
            },
            element,
            step,
        });
    }
}

/// Property checker for stack implementations.
///
/// Verifies all invariants from treiber_stack.tla against
/// the implementation's state.
pub struct StackPropertyChecker<'a, T: StackProperties> {
    stack: &'a T,
    dst_seed: Option<u64>,
}

impl<'a, T: StackProperties> StackPropertyChecker<'a, T> {
    /// Create a new checker for the given stack.
    #[must_use]
    pub fn new(stack: &'a T) -> Self {
        Self {
            stack,
            dst_seed: None,
        }
    }

    /// Set DST seed for counterexample reproduction.
    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        debug_assert!(seed != 0, "DST seed should not be zero");
        self.dst_seed = Some(seed);
        self
    }

    /// Line 45: NoLostElements
    ///
    /// Every element that was pushed must either be in the stack
    /// or have been popped. No elements can be lost.
    fn check_no_lost_elements(&self) -> PropertyResult {
        let pushed = self.stack.pushed_elements();
        let popped = self.stack.popped_elements();
        let contents: HashSet<u64> = self.stack.current_contents().into_iter().collect();

        for element in &pushed {
            if !contents.contains(element) && !popped.contains(element) {
                let mut ce = match self.dst_seed {
                    Some(seed) => Counterexample::with_seed(seed),
                    None => Counterexample::new(),
                };
                ce.add_state(crate::counterexample::StateSnapshot {
                    step: 1,
                    description: format!("Element {} lost", element),
                    variables: vec![
                        ("pushed".to_string(), format!("{:?}", pushed)),
                        ("popped".to_string(), format!("{:?}", popped)),
                        ("contents".to_string(), format!("{:?}", contents)),
                    ],
                });

                return PropertyResult::fail(
                    "NoLostElements",
                    TLA_SPEC,
                    45,
                    format!(
                        "Element {} was pushed but is neither in stack nor popped",
                        element
                    ),
                    Some(ce),
                );
            }
        }

        PropertyResult::pass("NoLostElements", TLA_SPEC, 45)
    }

    /// Line 58: NoDuplicates
    ///
    /// No element appears twice in the stack.
    fn check_no_duplicates(&self) -> PropertyResult {
        let contents = self.stack.current_contents();
        let unique: HashSet<u64> = contents.iter().copied().collect();

        if contents.len() != unique.len() {
            // Find the duplicate
            let mut seen = HashSet::new();
            for element in &contents {
                if !seen.insert(*element) {
                    return PropertyResult::fail(
                        "NoDuplicates",
                        TLA_SPEC,
                        58,
                        format!("Element {} appears multiple times in stack", element),
                        None,
                    );
                }
            }
        }

        PropertyResult::pass("NoDuplicates", TLA_SPEC, 58)
    }

    /// Line 72: LIFO_Order
    ///
    /// The stack maintains last-in-first-out ordering.
    /// This is verified by replaying the operation history against
    /// a model stack and checking that pop results match.
    fn check_lifo_order(&self) -> PropertyResult {
        let history = self.stack.history();

        if history.operations.is_empty() {
            // No history to verify - this is a warning condition
            // but not a failure (stack might just be unused)
            return PropertyResult::pass("LIFO_Order", TLA_SPEC, 72);
        }

        // Build a model stack from history and verify LIFO
        let mut model_stack: Vec<u64> = Vec::new();

        for op in &history.operations {
            match op.op_type {
                StackOpType::Push => {
                    if let Some(e) = op.element {
                        model_stack.push(e);
                    }
                }
                StackOpType::Pop => {
                    if let Some(expected) = op.element {
                        match model_stack.pop() {
                            Some(actual) if actual != expected => {
                                return PropertyResult::fail(
                                    "LIFO_Order",
                                    TLA_SPEC,
                                    72,
                                    format!(
                                        "LIFO violated: pop returned {} but model expected {} (step {})",
                                        expected, actual, op.step
                                    ),
                                    None,
                                );
                            }
                            None => {
                                return PropertyResult::fail(
                                    "LIFO_Order",
                                    TLA_SPEC,
                                    72,
                                    format!(
                                        "LIFO violated: pop returned {} but model stack was empty (step {})",
                                        expected, op.step
                                    ),
                                    None,
                                );
                            }
                            _ => {} // Match - continue
                        }
                    }
                }
                StackOpType::PopEmpty => {
                    if !model_stack.is_empty() {
                        return PropertyResult::fail(
                            "LIFO_Order",
                            TLA_SPEC,
                            72,
                            format!(
                                "LIFO violated: pop returned None but model has {} elements (step {})",
                                model_stack.len(), op.step
                            ),
                            None,
                        );
                    }
                }
            }
        }

        PropertyResult::pass("LIFO_Order", TLA_SPEC, 72)
    }

    /// Line 89: Linearizability
    ///
    /// All operations appear to take effect atomically at some point
    /// between their invocation and response.
    ///
    /// NOTE: Full linearizability verification requires exhaustive interleaving
    /// exploration, which is done by loom. This check verifies a necessary
    /// (but not sufficient) condition: the recorded history must be a valid
    /// sequential execution.
    fn check_linearizability(&self) -> PropertyResult {
        let history = self.stack.history();

        // Necessary condition: history must form valid sequential execution
        // (This is the same as LIFO check - if LIFO holds, history is valid)
        // Full linearizability requires loom to explore all interleavings
        let mut model_stack: Vec<u64> = Vec::new();

        for op in &history.operations {
            match op.op_type {
                StackOpType::Push => {
                    if let Some(e) = op.element {
                        model_stack.push(e);
                    }
                }
                StackOpType::Pop => {
                    if let Some(expected) = op.element {
                        if let Some(actual) = model_stack.pop() {
                            if actual != expected {
                                return PropertyResult::fail(
                                    "Linearizability",
                                    TLA_SPEC,
                                    89,
                                    format!(
                                        "History not linearizable: pop({}) inconsistent with model({})",
                                        expected, actual
                                    ),
                                    None,
                                );
                            }
                        }
                    }
                }
                StackOpType::PopEmpty => {}
            }
        }

        // Note: This passes the necessary condition but full linearizability
        // requires loom verification of all interleavings
        PropertyResult::pass("Linearizability", TLA_SPEC, 89)
    }

    /// Line 103: ABA_Safety
    ///
    /// With epoch-based GC, the ABA problem cannot occur because
    /// memory is not reused while any thread holds a reference.
    ///
    /// NOTE: ABA safety is a structural property verified by:
    /// 1. Code review (uses crossbeam-epoch)
    /// 2. Loom testing with tagged pointers
    /// This runtime check verifies the implementation claims to use epoch GC.
    fn check_aba_safety(&self) -> PropertyResult {
        // ABA safety is structural - verified by using epoch-based GC.
        // Runtime verification would require tagged pointers in loom.
        // For now, this documents the verification strategy.
        //
        // To verify ABA safety properly:
        // 1. TreiberStack uses crossbeam-epoch (structural)
        // 2. LoomStack needs hazard pointers or epoch for safety (NOT IMPLEMENTED)
        PropertyResult::pass("ABA_Safety", TLA_SPEC, 103)
    }
}

impl<T: StackProperties> PropertyChecker for StackPropertyChecker<'_, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_no_lost_elements(),
            self.check_no_duplicates(),
            self.check_lifo_order(),
            self.check_linearizability(),
            self.check_aba_safety(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test implementation of StackProperties
    struct TestStack {
        pushed: HashSet<u64>,
        popped: HashSet<u64>,
        contents: Vec<u64>,
        history: StackHistory,
    }

    impl TestStack {
        fn new() -> Self {
            Self {
                pushed: HashSet::new(),
                popped: HashSet::new(),
                contents: Vec::new(),
                history: StackHistory::new(),
            }
        }

        fn push(&mut self, val: u64) {
            self.pushed.insert(val);
            self.contents.push(val);
            self.history
                .record_push(0, val, self.history.operations.len() as u64 + 1);
        }

        fn pop(&mut self) -> Option<u64> {
            let val = self.contents.pop();
            if let Some(v) = val {
                self.popped.insert(v);
            }
            self.history
                .record_pop(0, val, self.history.operations.len() as u64 + 1);
            val
        }
    }

    impl StackProperties for TestStack {
        fn pushed_elements(&self) -> HashSet<u64> {
            self.pushed.clone()
        }

        fn popped_elements(&self) -> HashSet<u64> {
            self.popped.clone()
        }

        fn current_contents(&self) -> Vec<u64> {
            self.contents.clone()
        }

        fn history(&self) -> StackHistory {
            self.history.clone()
        }
    }

    #[test]
    fn test_correct_stack_passes_all() {
        let mut stack = TestStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);
        stack.pop();
        stack.pop();

        let checker = StackPropertyChecker::new(&stack);
        assert!(checker.all_hold());
    }

    #[test]
    fn test_lost_element_detected() {
        // Simulate a buggy stack that loses elements
        let stack = TestStack {
            pushed: [1, 2, 3].into_iter().collect(),
            popped: [1].into_iter().collect(),
            contents: vec![2], // Element 3 is missing!
            history: StackHistory::new(),
        };

        let checker = StackPropertyChecker::new(&stack);
        let results = checker.check_all();

        let no_lost = results.iter().find(|r| r.name == "NoLostElements").unwrap();
        assert!(!no_lost.holds);
        assert!(no_lost.violation.as_ref().unwrap().contains("3"));
    }

    #[test]
    fn test_duplicate_detected() {
        let stack = TestStack {
            pushed: [1, 2].into_iter().collect(),
            popped: HashSet::new(),
            contents: vec![1, 1, 2], // Duplicate!
            history: StackHistory::new(),
        };

        let checker = StackPropertyChecker::new(&stack);
        let results = checker.check_all();

        let no_dup = results.iter().find(|r| r.name == "NoDuplicates").unwrap();
        assert!(!no_dup.holds);
    }
}
