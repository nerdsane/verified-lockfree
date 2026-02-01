//! Verifier that tests implementations against the TLA+ spec invariants.
//!
//! This module verifies that actual implementations satisfy the same
//! invariants proven by stateright model checking.

use std::collections::HashSet;

/// Trait that implementations must satisfy for verification.
///
/// This is the bridge between generated code and the TLA+ spec.
pub trait VerifiableStack: Send + Sync {
    /// Create a new empty stack.
    fn new() -> Self;

    /// Push a value onto the stack.
    fn push(&self, value: u64);

    /// Pop a value from the stack.
    fn pop(&self) -> Option<u64>;

    /// Check if empty.
    fn is_empty(&self) -> bool;

    /// Get pushed elements (for verification).
    fn pushed_elements(&self) -> HashSet<u64>;

    /// Get popped elements (for verification).
    fn popped_elements(&self) -> HashSet<u64>;

    /// Get current stack contents.
    fn get_contents(&self) -> Vec<u64>;
}

/// Result of verification.
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Whether all checks passed.
    pub passed: bool,
    /// Number of operations executed.
    pub operations_count: usize,
    /// Invariants checked.
    pub invariants_checked: Vec<String>,
    /// Error message if verification failed.
    pub error: Option<String>,
}

/// Configuration for verification.
#[derive(Debug, Clone)]
pub struct VerifierConfig {
    /// Number of operations to run.
    pub operations_count: usize,
    /// Check invariants after every N operations.
    pub check_interval: usize,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            operations_count: 100,
            check_interval: 10,
        }
    }
}

impl VerifierConfig {
    /// Quick verification.
    pub fn quick() -> Self {
        Self {
            operations_count: 50,
            check_interval: 10,
        }
    }

    /// Thorough verification.
    pub fn thorough() -> Self {
        Self {
            operations_count: 500,
            check_interval: 5,
        }
    }
}

/// Verify an implementation satisfies TLA+ spec invariants.
///
/// Runs a series of operations and verifies:
/// - NoLostElements (TLA+ Line 45): Every pushed element is either in stack or was popped
/// - NoDuplicates (TLA+ Line 58): No element appears twice in the stack
pub fn verify_implementation<S: VerifiableStack>(config: &VerifierConfig) -> VerificationResult {
    let stack = S::new();
    let mut ops = 0;

    // Test 1: Sequential push/pop
    for i in 1..=10 {
        stack.push(i);
        ops += 1;
    }

    if let Err(e) = check_invariants(&stack, "after sequential push") {
        return VerificationResult {
            passed: false,
            operations_count: ops,
            invariants_checked: vec!["NoLostElements".into(), "NoDuplicates".into()],
            error: Some(e),
        };
    }

    for _ in 0..5 {
        stack.pop();
        ops += 1;
    }

    if let Err(e) = check_invariants(&stack, "after sequential pop") {
        return VerificationResult {
            passed: false,
            operations_count: ops,
            invariants_checked: vec!["NoLostElements".into(), "NoDuplicates".into()],
            error: Some(e),
        };
    }

    // Test 2: Interleaved push/pop (simulated concurrent pattern)
    let stack2 = S::new();
    let values: Vec<u64> = (100..150).collect();

    for (i, &val) in values.iter().enumerate() {
        stack2.push(val);
        ops += 1;

        // Occasionally pop
        if i % 3 == 0 && i > 0 {
            stack2.pop();
            ops += 1;
        }

        // Check invariants periodically
        if ops % config.check_interval == 0 {
            if let Err(e) = check_invariants(&stack2, &format!("at operation {}", ops)) {
                return VerificationResult {
                    passed: false,
                    operations_count: ops,
                    invariants_checked: vec!["NoLostElements".into(), "NoDuplicates".into()],
                    error: Some(e),
                };
            }
        }
    }

    // Test 3: Empty all and verify
    while !stack2.is_empty() {
        stack2.pop();
        ops += 1;
    }

    if let Err(e) = check_invariants(&stack2, "after emptying") {
        return VerificationResult {
            passed: false,
            operations_count: ops,
            invariants_checked: vec!["NoLostElements".into(), "NoDuplicates".into()],
            error: Some(e),
        };
    }

    // Test 4: LIFO order verification
    let stack3 = S::new();
    let lifo_values: Vec<u64> = vec![1000, 2000, 3000, 4000, 5000];

    for &val in &lifo_values {
        stack3.push(val);
        ops += 1;
    }

    // Pop should return in reverse order
    for &expected in lifo_values.iter().rev() {
        let actual = stack3.pop();
        ops += 1;

        if actual != Some(expected) {
            return VerificationResult {
                passed: false,
                operations_count: ops,
                invariants_checked: vec!["NoLostElements".into(), "NoDuplicates".into(), "LIFO_Order".into()],
                error: Some(format!(
                    "LIFO order violated: expected {:?}, got {:?}",
                    Some(expected),
                    actual
                )),
            };
        }
    }

    VerificationResult {
        passed: true,
        operations_count: ops,
        invariants_checked: vec![
            "NoLostElements".into(),
            "NoDuplicates".into(),
            "LIFO_Order".into(),
        ],
        error: None,
    }
}

/// Check TLA+ invariants on the implementation.
fn check_invariants<S: VerifiableStack>(stack: &S, context: &str) -> Result<(), String> {
    let pushed = stack.pushed_elements();
    let popped = stack.popped_elements();
    let contents = stack.get_contents();
    let contents_set: HashSet<u64> = contents.iter().copied().collect();

    // NoLostElements (TLA+ Line 45):
    // Every pushed element is either in stack or was popped
    for elem in &pushed {
        if !contents_set.contains(elem) && !popped.contains(elem) {
            return Err(format!(
                "NoLostElements violated {}: element {} was pushed but is neither in stack nor popped. \
                 Pushed: {:?}, Popped: {:?}, Contents: {:?}",
                context, elem, pushed, popped, contents
            ));
        }
    }

    // NoDuplicates (TLA+ Line 58):
    // No element appears twice in the stack
    if contents.len() != contents_set.len() {
        return Err(format!(
            "NoDuplicates violated {}: stack contains duplicate elements. Contents: {:?}",
            context, contents
        ));
    }

    Ok(())
}

/// Verify an implementation with concurrent operations.
///
/// Uses multiple threads to stress test the implementation.
#[cfg(feature = "concurrent")]
pub fn verify_concurrent<S: VerifiableStack + 'static>(
    config: &VerifierConfig,
) -> VerificationResult {
    use std::sync::Arc;
    use std::thread;

    let stack = Arc::new(S::new());
    let threads_count = 4;
    let ops_per_thread = config.operations_count / threads_count;

    let handles: Vec<_> = (0..threads_count)
        .map(|tid| {
            let stack = Arc::clone(&stack);
            thread::spawn(move || {
                for i in 0..ops_per_thread {
                    let value = (tid * 1000 + i) as u64;
                    stack.push(value);
                    if i % 2 == 0 {
                        stack.pop();
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    // Check final state
    match check_invariants(&*stack, "after concurrent operations") {
        Ok(()) => VerificationResult {
            passed: true,
            operations_count: config.operations_count,
            invariants_checked: vec!["NoLostElements".into(), "NoDuplicates".into()],
            error: None,
        },
        Err(e) => VerificationResult {
            passed: false,
            operations_count: config.operations_count,
            invariants_checked: vec!["NoLostElements".into(), "NoDuplicates".into()],
            error: Some(e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock implementation for testing the verifier
    struct MockStack {
        values: std::sync::Mutex<Vec<u64>>,
        pushed: std::sync::Mutex<HashSet<u64>>,
        popped: std::sync::Mutex<HashSet<u64>>,
    }

    impl VerifiableStack for MockStack {
        fn new() -> Self {
            Self {
                values: std::sync::Mutex::new(Vec::new()),
                pushed: std::sync::Mutex::new(HashSet::new()),
                popped: std::sync::Mutex::new(HashSet::new()),
            }
        }

        fn push(&self, value: u64) {
            self.values.lock().unwrap().push(value);
            self.pushed.lock().unwrap().insert(value);
        }

        fn pop(&self) -> Option<u64> {
            let value = self.values.lock().unwrap().pop();
            if let Some(v) = value {
                self.popped.lock().unwrap().insert(v);
            }
            value
        }

        fn is_empty(&self) -> bool {
            self.values.lock().unwrap().is_empty()
        }

        fn pushed_elements(&self) -> HashSet<u64> {
            self.pushed.lock().unwrap().clone()
        }

        fn popped_elements(&self) -> HashSet<u64> {
            self.popped.lock().unwrap().clone()
        }

        fn get_contents(&self) -> Vec<u64> {
            self.values.lock().unwrap().clone()
        }
    }

    #[test]
    fn test_verifier_with_correct_impl() {
        let config = VerifierConfig::quick();
        let result = verify_implementation::<MockStack>(&config);

        assert!(
            result.passed,
            "Verification should pass for correct impl: {:?}",
            result.error
        );
        assert!(result.operations_count > 0);
        assert!(result.invariants_checked.contains(&"NoLostElements".to_string()));
        assert!(result.invariants_checked.contains(&"NoDuplicates".to_string()));
        assert!(result.invariants_checked.contains(&"LIFO_Order".to_string()));
    }

    // Test with a buggy implementation
    struct BuggyStack {
        values: std::sync::Mutex<Vec<u64>>,
        pushed: std::sync::Mutex<HashSet<u64>>,
        popped: std::sync::Mutex<HashSet<u64>>,
    }

    impl VerifiableStack for BuggyStack {
        fn new() -> Self {
            Self {
                values: std::sync::Mutex::new(Vec::new()),
                pushed: std::sync::Mutex::new(HashSet::new()),
                popped: std::sync::Mutex::new(HashSet::new()),
            }
        }

        fn push(&self, value: u64) {
            self.values.lock().unwrap().push(value);
            self.pushed.lock().unwrap().insert(value);
        }

        fn pop(&self) -> Option<u64> {
            let value = self.values.lock().unwrap().pop();
            // BUG: Don't track popped elements - violates NoLostElements
            value
        }

        fn is_empty(&self) -> bool {
            self.values.lock().unwrap().is_empty()
        }

        fn pushed_elements(&self) -> HashSet<u64> {
            self.pushed.lock().unwrap().clone()
        }

        fn popped_elements(&self) -> HashSet<u64> {
            self.popped.lock().unwrap().clone()
        }

        fn get_contents(&self) -> Vec<u64> {
            self.values.lock().unwrap().clone()
        }
    }

    #[test]
    fn test_verifier_catches_buggy_impl() {
        let config = VerifierConfig::quick();
        let result = verify_implementation::<BuggyStack>(&config);

        assert!(
            !result.passed,
            "Verification should fail for buggy impl"
        );
        assert!(
            result.error.as_ref().unwrap().contains("NoLostElements"),
            "Error should mention NoLostElements invariant: {:?}",
            result.error
        );
    }
}
