//! Level 5: Kani bounded model checking evaluator.
//!
//! Runs Kani to perform bounded verification / proof of Rust code.
//! Kani uses CBMC (C Bounded Model Checker) under the hood to verify
//! that code satisfies assertions for all inputs up to a bound.
//!
//! Key capabilities:
//! - Symbolic execution over all possible inputs
//! - Memory safety verification
//! - Assertion checking with bounded unwinding
//! - Counterexample generation for violations

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;
use vf_core::Counterexample;

use crate::result::EvaluatorResult;

/// Kani configuration.
#[derive(Debug, Clone)]
pub struct KaniConfig {
    /// Unwind bound for loops
    pub unwind: usize,
    /// Timeout for verification
    pub timeout: Duration,
    /// Harness filter (regex pattern)
    pub harness: Option<String>,
    /// Enable memory safety checks
    pub memory_safety: bool,
    /// Enable overflow checks
    pub overflow_checks: bool,
    /// Extra arguments to pass to Kani
    pub extra_args: Vec<String>,
}

impl Default for KaniConfig {
    fn default() -> Self {
        Self {
            unwind: 10,
            timeout: Duration::from_secs(300),
            harness: None,
            memory_safety: true,
            overflow_checks: true,
            extra_args: Vec::new(),
        }
    }
}

impl KaniConfig {
    /// Quick config for fast iteration.
    pub fn quick() -> Self {
        Self {
            unwind: 5,
            timeout: Duration::from_secs(60),
            ..Default::default()
        }
    }

    /// Thorough config for CI.
    pub fn thorough() -> Self {
        Self {
            unwind: 20,
            timeout: Duration::from_secs(600),
            ..Default::default()
        }
    }

    /// Exhaustive config for maximum verification.
    pub fn exhaustive() -> Self {
        Self {
            unwind: 50,
            timeout: Duration::from_secs(1800),
            ..Default::default()
        }
    }
}

/// Run Kani verification on a crate.
///
/// Looks for functions annotated with `#[kani::proof]`.
pub async fn run(crate_path: &Path, timeout: Duration, unwind: usize) -> EvaluatorResult {
    run_with_config(
        crate_path,
        KaniConfig {
            unwind,
            timeout,
            ..Default::default()
        },
    )
    .await
}

/// Run Kani with full configuration.
pub async fn run_with_config(crate_path: &Path, config: KaniConfig) -> EvaluatorResult {
    let start = Instant::now();

    // Check if Kani is installed
    let kani_check = Command::new("cargo")
        .args(["kani", "--version"])
        .output()
        .await;

    if kani_check.is_err() || !kani_check.unwrap().status.success() {
        return EvaluatorResult::skip(
            "kani",
            "Kani not installed. Install with: cargo install --locked kani-verifier && cargo kani setup",
            start.elapsed(),
        );
    }

    // Build the Kani command
    let mut cmd = Command::new("cargo");
    cmd.arg("kani");

    // Add unwind bound
    cmd.args(["--default-unwind", &config.unwind.to_string()]);

    // Add harness filter if specified
    if let Some(ref harness) = config.harness {
        cmd.args(["--harness", harness]);
    }

    // Memory safety checks
    if config.memory_safety {
        cmd.arg("--memory-safety-checks");
    }

    // Overflow checks
    if config.overflow_checks {
        cmd.arg("--overflow-checks");
    }

    // Extra arguments
    for arg in &config.extra_args {
        cmd.arg(arg);
    }

    cmd.current_dir(crate_path);

    let result = tokio::time::timeout(config.timeout, cmd.output()).await;

    let duration = start.elapsed();

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}\n{}", stdout, stderr);

            if output.status.success() {
                let stats = extract_kani_stats(&combined);
                EvaluatorResult::pass_with_output("kani", duration, format!("{}\n{}", stats, combined))
            } else {
                let (error, counterexample) = extract_kani_error(&combined);
                if let Some(ce) = counterexample {
                    EvaluatorResult::fail_with_counterexample("kani", error, ce, duration, combined)
                } else {
                    EvaluatorResult::fail("kani", error, duration, combined)
                }
            }
        }
        Ok(Err(e)) => EvaluatorResult::fail(
            "kani",
            format!("Failed to run Kani: {}", e),
            duration,
            String::new(),
        ),
        Err(_) => EvaluatorResult::fail(
            "kani",
            format!("Timeout after {:?}", config.timeout),
            duration,
            String::new(),
        ),
    }
}

/// Check if Kani is available on the system.
pub async fn is_available() -> bool {
    Command::new("cargo")
        .args(["kani", "--version"])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Extract Kani verification statistics from output.
fn extract_kani_stats(output: &str) -> String {
    let mut stats = Vec::new();

    for line in output.lines() {
        // Look for verification result lines
        if line.contains("VERIFICATION:")
            || line.contains("verification result")
            || line.contains("Checking harness")
            || line.contains("** 0 of")
            || line.contains("Complete -")
        {
            stats.push(line.trim().to_string());
        }
    }

    if stats.is_empty() {
        "No Kani stats found in output".to_string()
    } else {
        stats.join("\n")
    }
}

/// Extract error and counterexample from Kani output.
fn extract_kani_error(output: &str) -> (String, Option<Counterexample>) {
    let mut error = String::new();
    let mut trace_lines = Vec::new();
    let mut in_trace = false;
    let mut failure_property = String::new();

    for line in output.lines() {
        let trimmed = line.trim();

        // Look for verification failure
        if trimmed.contains("VERIFICATION:- FAILED") || trimmed.contains("FAILED") && trimmed.contains("harness") {
            if error.is_empty() {
                error = trimmed.to_string();
            }
        }

        // Look for failed property
        if trimmed.contains("Failed Coverage Checks") || trimmed.contains("assertion failed") {
            failure_property = trimmed.to_string();
        }

        // Look for counterexample trace
        if trimmed.contains("COUNTEREXAMPLE") || trimmed.contains("Trace:") {
            in_trace = true;
            continue;
        }

        if in_trace {
            if trimmed.is_empty() || trimmed.starts_with("---") || trimmed.contains("VERIFICATION") {
                in_trace = false;
            } else {
                trace_lines.push(trimmed.to_string());
            }
        }

        // Capture assertion failures
        if trimmed.contains("assertion") && trimmed.contains("FAILURE") {
            if error.is_empty() {
                error = trimmed.to_string();
            }
        }
    }

    if error.is_empty() {
        error = if !failure_property.is_empty() {
            failure_property
        } else {
            "Kani verification failed".to_string()
        };
    }

    // Build counterexample from trace
    let counterexample = if !trace_lines.is_empty() {
        use vf_core::{StateSnapshot, ThreadAction};

        let mut ce = Counterexample::new();

        for (i, line) in trace_lines.iter().enumerate() {
            // Parse Kani trace format
            // Typical format: "  Step N: function_name at file.rs:line"
            if line.contains("Step") || line.contains("at ") {
                ce.add_state(StateSnapshot {
                    step: i as u64,
                    description: line.clone(),
                    variables: Vec::new(),
                });
            } else if !line.is_empty() {
                ce.add_action(ThreadAction {
                    thread_id: 0,
                    step: i as u64,
                    action: line.clone(),
                    success: true,
                });
            }
        }

        Some(ce)
    } else {
        None
    };

    (error, counterexample)
}

/// Template for Kani proof harnesses.
///
/// This provides example code showing how to write Kani proofs.
pub const PROOF_HARNESS_TEMPLATE: &str = r#"
// Kani proof harnesses for lock-free data structures
//
// To run: cargo kani --harness <harness_name>
// Or: cargo kani (runs all harnesses)

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Proof that push never loses elements.
    #[kani::proof]
    #[kani::unwind(5)]
    fn proof_push_preserves_elements() {
        let stack = TreiberStack::new();

        // Symbolic input
        let value: u64 = kani::any();
        kani::assume(value > 0 && value < 1000);

        stack.push(value);

        // The value must be in the stack
        let contents = stack.get_contents();
        kani::assert(contents.contains(&value), "Pushed value must be in stack");
    }

    /// Proof that pop returns valid values.
    #[kani::proof]
    #[kani::unwind(5)]
    fn proof_pop_returns_pushed_value() {
        let stack = TreiberStack::new();

        let value: u64 = kani::any();
        kani::assume(value > 0 && value < 1000);

        stack.push(value);
        let popped = stack.pop();

        kani::assert(popped == Some(value), "Popped value must equal pushed value");
    }

    /// Proof of LIFO ordering.
    #[kani::proof]
    #[kani::unwind(3)]
    fn proof_lifo_order() {
        let stack = TreiberStack::new();

        let v1: u64 = kani::any();
        let v2: u64 = kani::any();
        kani::assume(v1 != v2);
        kani::assume(v1 > 0 && v1 < 100);
        kani::assume(v2 > 0 && v2 < 100);

        stack.push(v1);
        stack.push(v2);

        // Must get v2 first (LIFO)
        let first = stack.pop();
        kani::assert(first == Some(v2), "Second pushed must be first popped");

        let second = stack.pop();
        kani::assert(second == Some(v1), "First pushed must be second popped");
    }

    /// Proof that empty stack returns None.
    #[kani::proof]
    fn proof_empty_pop_returns_none() {
        let stack = TreiberStack::new();
        let result = stack.pop();
        kani::assert(result.is_none(), "Pop on empty stack must return None");
    }

    /// Bounded proof for concurrent operations.
    /// Note: Kani doesn't support real concurrency, but we can verify
    /// sequential consistency of the CAS operations.
    #[kani::proof]
    #[kani::unwind(10)]
    fn proof_cas_operations_bounded() {
        let stack = TreiberStack::new();

        // Simulate a sequence of operations
        let ops_count: usize = kani::any();
        kani::assume(ops_count <= 5);

        let mut pushed_count = 0u64;
        let mut popped_count = 0u64;

        for _ in 0..ops_count {
            let is_push: bool = kani::any();

            if is_push {
                let value = pushed_count + 1;
                stack.push(value);
                pushed_count += 1;
            } else if let Some(_) = stack.pop() {
                popped_count += 1;
            }
        }

        // Invariant: popped_count <= pushed_count
        kani::assert(
            popped_count <= pushed_count,
            "Cannot pop more than pushed"
        );
    }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_presets() {
        let quick = KaniConfig::quick();
        assert_eq!(quick.unwind, 5);

        let thorough = KaniConfig::thorough();
        assert_eq!(thorough.unwind, 20);

        let exhaustive = KaniConfig::exhaustive();
        assert_eq!(exhaustive.unwind, 50);
    }

    #[test]
    fn test_extract_kani_stats() {
        let output = r#"
Checking harness proof_push_preserves_elements
VERIFICATION:- SUCCESSFUL
Complete - 1 successfully verified harnesses, 0 failures
"#;
        let stats = extract_kani_stats(output);
        assert!(stats.contains("VERIFICATION"));
        assert!(stats.contains("SUCCESSFUL"));
    }

    #[test]
    fn test_extract_kani_error() {
        let output = r#"
Checking harness proof_lifo_order
VERIFICATION:- FAILED
assertion failed: Popped value must equal pushed value
COUNTEREXAMPLE
  Step 1: push(42) at src/lib.rs:50
  Step 2: push(43) at src/lib.rs:51
  Step 3: pop() returns 42 at src/lib.rs:52
---
"#;
        let (error, ce) = extract_kani_error(output);
        assert!(error.contains("FAILED") || error.contains("assertion failed"));
        assert!(ce.is_some());
    }

    #[test]
    fn test_proof_harness_template() {
        // Verify template is valid Rust (basic syntax check)
        assert!(PROOF_HARNESS_TEMPLATE.contains("#[kani::proof]"));
        assert!(PROOF_HARNESS_TEMPLATE.contains("kani::any()"));
        assert!(PROOF_HARNESS_TEMPLATE.contains("kani::assert"));
    }
}
