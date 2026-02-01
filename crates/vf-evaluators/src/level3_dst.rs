//! Level 3: DST evaluator.
//!
//! Runs Deterministic Simulation Testing with fault injection.
//!
//! DST tests are characterized by:
//! - Reproducible via seed (DST_SEED environment variable)
//! - Deterministic scheduling of concurrent operations
//! - Fault injection (delays, failures)
//! - Invariant checking at configurable intervals

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;
use vf_core::Counterexample;

use crate::result::EvaluatorResult;

/// DST configuration.
#[derive(Debug, Clone)]
pub struct DstConfig {
    /// Seed for reproducibility (None = random)
    pub seed: Option<u64>,
    /// Number of iterations
    pub iterations: u64,
    /// Timeout per test
    pub timeout: Duration,
    /// Test filter (regex pattern)
    pub filter: Option<String>,
}

impl Default for DstConfig {
    fn default() -> Self {
        Self {
            seed: None,
            iterations: 1000,
            timeout: Duration::from_secs(60),
            filter: None,
        }
    }
}

impl DstConfig {
    /// Quick config for fast testing.
    pub fn quick() -> Self {
        Self {
            iterations: 100,
            timeout: Duration::from_secs(10),
            ..Default::default()
        }
    }

    /// Thorough config for CI.
    pub fn thorough() -> Self {
        Self {
            iterations: 10000,
            timeout: Duration::from_secs(300),
            ..Default::default()
        }
    }

    /// Stress config for finding rare bugs.
    pub fn stress() -> Self {
        Self {
            iterations: 100000,
            timeout: Duration::from_secs(600),
            ..Default::default()
        }
    }
}

/// Run DST tests on a crate.
///
/// DST tests use the `vf-dst` framework for deterministic simulation.
pub async fn run(
    crate_path: &Path,
    timeout: Duration,
    seed: Option<u64>,
    iterations: u64,
) -> EvaluatorResult {
    run_with_config(crate_path, DstConfig {
        seed,
        iterations,
        timeout,
        filter: None,
    }).await
}

/// Run DST tests with full configuration.
pub async fn run_with_config(crate_path: &Path, config: DstConfig) -> EvaluatorResult {
    let start = Instant::now();

    // Build the test command
    let mut cmd = Command::new("cargo");
    cmd.args(["test", "--release"]);

    // Add filter if specified
    if let Some(ref filter) = config.filter {
        cmd.arg(filter);
    }

    // Run tests sequentially for determinism
    cmd.arg("--");
    cmd.arg("--test-threads=1");

    cmd.current_dir(crate_path);

    // Set DST seed if provided
    if let Some(s) = config.seed {
        cmd.env("DST_SEED", s.to_string());
    }

    // Set iterations
    cmd.env("DST_ITERATIONS", config.iterations.to_string());

    let result = tokio::time::timeout(config.timeout, cmd.output()).await;

    let duration = start.elapsed();

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}\n{}", stdout, stderr);

            if output.status.success() {
                // Extract DST stats from output
                let stats = extract_dst_stats(&combined);
                EvaluatorResult::pass_with_output(
                    "DST",
                    duration,
                    format!("{}\n{}", stats, combined),
                )
            } else {
                let (error, counterexample) = extract_dst_error(&stderr, &stdout);
                if let Some(ce) = counterexample {
                    EvaluatorResult::fail_with_counterexample("DST", error, ce, duration, combined)
                } else {
                    EvaluatorResult::fail("DST", error, duration, combined)
                }
            }
        }
        Ok(Err(e)) => EvaluatorResult::fail(
            "DST",
            format!("Failed to run DST tests: {}", e),
            duration,
            String::new(),
        ),
        Err(_) => EvaluatorResult::fail(
            "DST",
            format!("Timeout after {:?}", config.timeout),
            duration,
            String::new(),
        ),
    }
}

/// Run DST tests directly in-process (for testing the evaluator itself).
pub fn run_inline<F, E>(seed: u64, iterations: u64, mut test_fn: F) -> InlineResult
where
    F: FnMut(u64) -> Result<(), E>,
    E: std::fmt::Display,
{
    let start = Instant::now();

    for i in 0..iterations {
        if let Err(e) = test_fn(i) {
            return InlineResult {
                passed: false,
                seed,
                iterations_completed: i,
                error: Some(format!("{}", e)),
                duration: start.elapsed(),
            };
        }
    }

    InlineResult {
        passed: true,
        seed,
        iterations_completed: iterations,
        error: None,
        duration: start.elapsed(),
    }
}

/// Result from inline DST testing.
#[derive(Debug, Clone)]
pub struct InlineResult {
    pub passed: bool,
    pub seed: u64,
    pub iterations_completed: u64,
    pub error: Option<String>,
    pub duration: Duration,
}

impl InlineResult {
    pub fn format(&self) -> String {
        if self.passed {
            format!(
                "[PASS] DST_SEED={} iterations={} ({:.2}s)",
                self.seed,
                self.iterations_completed,
                self.duration.as_secs_f64()
            )
        } else {
            format!(
                "[FAIL] DST_SEED={} iterations={} ({:.2}s): {}",
                self.seed,
                self.iterations_completed,
                self.duration.as_secs_f64(),
                self.error.as_deref().unwrap_or("unknown")
            )
        }
    }
}

/// Extract DST statistics from output.
fn extract_dst_stats(output: &str) -> String {
    let mut stats = Vec::new();

    for line in output.lines() {
        if line.contains("DST completed:") || line.contains("DST_SEED=") {
            stats.push(line.trim().to_string());
        }
    }

    if stats.is_empty() {
        "No DST stats found in output".to_string()
    } else {
        stats.join("\n")
    }
}

/// Structured invariant failure extracted from DST output.
#[derive(Debug, Clone)]
pub struct InvariantFailure {
    pub name: String,
    pub message: Option<String>,
}

/// Extract DST error and seed for reproduction.
fn extract_dst_error(stderr: &str, stdout: &str) -> (String, Option<Counterexample>) {
    let mut seed: Option<u64> = None;
    let mut error = String::new();
    let mut invariant_failures: Vec<InvariantFailure> = Vec::new();
    let mut operation_trace: Vec<String> = Vec::new();

    let combined = format!("{}\n{}", stderr, stdout);
    let mut in_invariant_section = false;
    let mut in_trace_section = false;
    let mut current_invariant: Option<String> = None;

    for line in combined.lines() {
        // Look for DST_SEED in output
        if line.contains("DST_SEED=") {
            if let Some(seed_str) = line.split("DST_SEED=").nth(1) {
                if let Some(num_str) = seed_str.split_whitespace().next() {
                    // Handle both "12345" and "12345 (randomly generated)"
                    let num_str = num_str.trim_end_matches(|c: char| !c.is_ascii_digit());
                    if let Ok(s) = num_str.parse::<u64>() {
                        seed = Some(s);
                    }
                }
            }
        }

        // Parse structured invariant failure section
        if line.contains("=== DST INVARIANT FAILURES ===") {
            in_invariant_section = true;
            continue;
        }
        if line.contains("=== END INVARIANT FAILURES ===") {
            in_invariant_section = false;
            in_trace_section = false;
            continue;
        }

        if in_invariant_section {
            if line.contains("Operation trace") {
                in_trace_section = true;
                continue;
            }

            if in_trace_section {
                // Capture operation trace lines
                if line.trim().starts_with(|c: char| c.is_ascii_digit()) {
                    operation_trace.push(line.trim().to_string());
                }
            } else if line.starts_with("INVARIANT_FAILED:") {
                // Save previous invariant if any
                if let Some(name) = current_invariant.take() {
                    invariant_failures.push(InvariantFailure { name, message: None });
                }
                // Start new invariant
                let name = line.trim_start_matches("INVARIANT_FAILED:").trim();
                current_invariant = Some(name.to_string());
            } else if line.trim().starts_with("Message:") && current_invariant.is_some() {
                let msg = line.trim().trim_start_matches("Message:").trim();
                let name = current_invariant.take().unwrap();
                invariant_failures.push(InvariantFailure {
                    name,
                    message: Some(msg.to_string()),
                });
            }
        }

        // Look for panic message
        if line.contains("panicked at") {
            error = line.to_string();
        } else if line.contains("assertion failed") || line.contains("Invariant violated") {
            if error.is_empty() {
                error = line.to_string();
            }
        }
    }

    // Handle any remaining invariant
    if let Some(name) = current_invariant {
        invariant_failures.push(InvariantFailure { name, message: None });
    }

    // Build rich error message from invariant failures
    if !invariant_failures.is_empty() {
        let mut rich_error = String::new();
        for failure in &invariant_failures {
            rich_error.push_str(&format!("Invariant '{}' violated", failure.name));
            if let Some(ref msg) = failure.message {
                rich_error.push_str(&format!(": {}", msg));
            }
            rich_error.push('\n');
        }
        if !operation_trace.is_empty() {
            rich_error.push_str("\nRecent operations:\n");
            for op in operation_trace.iter().take(10) {
                rich_error.push_str(&format!("  {}\n", op));
            }
        }
        error = rich_error;
    }

    if error.is_empty() {
        error = "DST test failed".to_string();
    }

    // Build counterexample with invariant details
    let counterexample = seed.map(|s| {
        let ce = Counterexample::with_seed(s);
        // Add invariant failures to counterexample description
        if !invariant_failures.is_empty() {
            let invariant_desc = invariant_failures
                .iter()
                .map(|f| {
                    match &f.message {
                        Some(m) => format!("{}: {}", f.name, m),
                        None => f.name.clone(),
                    }
                })
                .collect::<Vec<_>>()
                .join("; ");
            ce.with_description(format!("Invariant violations: {}", invariant_desc))
        } else {
            ce
        }
    });

    (error, counterexample)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_dst_error() {
        let stdout = r#"
running 1 test
DST_SEED=12345 (randomly generated)
thread 'test_stack_under_faults' panicked at 'assertion failed: checker.all_hold()',
    src/treiber_stack.rs:200:9
test test_stack_under_faults ... FAILED
"#;
        let (error, ce) = extract_dst_error("", stdout);
        assert!(error.contains("panicked") || error.contains("assertion failed"));
        assert!(ce.is_some());
        assert_eq!(ce.unwrap().dst_seed, Some(12345));
    }

    #[test]
    fn test_extract_dst_stats() {
        let output = r#"
running 1 test
DST_SEED=42 (from environment)
DST completed: elapsed=100ms rng_calls=500 faults=10
test test_foo ... ok
"#;
        let stats = extract_dst_stats(output);
        assert!(stats.contains("DST_SEED=42"));
        assert!(stats.contains("DST completed"));
    }

    #[test]
    fn test_run_inline_pass() {
        let result = run_inline(12345, 100, |_i| Ok::<(), &str>(()));
        assert!(result.passed);
        assert_eq!(result.iterations_completed, 100);
    }

    #[test]
    fn test_run_inline_fail() {
        let result = run_inline(12345, 100, |i| {
            if i == 50 {
                Err("Failed at iteration 50")
            } else {
                Ok(())
            }
        });
        assert!(!result.passed);
        assert_eq!(result.iterations_completed, 50);
        assert!(result.error.as_ref().unwrap().contains("50"));
    }

    #[test]
    fn test_config_presets() {
        let quick = DstConfig::quick();
        assert_eq!(quick.iterations, 100);

        let thorough = DstConfig::thorough();
        assert_eq!(thorough.iterations, 10000);

        let stress = DstConfig::stress();
        assert_eq!(stress.iterations, 100000);
    }

    #[test]
    fn test_extract_structured_invariant_failures() {
        let stderr = r#"
=== DST INVARIANT FAILURES ===
DST_SEED=54321
INVARIANT_FAILED: NoCommittedDangerousStructures
  Message: Committed transaction T3 has dangerous structure (in_conflict=true, out_conflict=true)
INVARIANT_FAILED: Serializable
  Message: Serializability violated: Committed transaction T3 has dangerous structure

Operation trace (last 20):
  0: Begin(1)
  1: Read { txn: 1, key: 1, value: None }
  2: Begin(2)
  3: Write { txn: 2, key: 1, value: 100 }
=== END INVARIANT FAILURES ===
thread 'test_ssi' panicked at 'SSI invariants violated at DST_SEED=54321'
"#;
        let (error, ce) = extract_dst_error(stderr, "");

        // Should extract the invariant violations
        assert!(error.contains("NoCommittedDangerousStructures"));
        assert!(error.contains("T3 has dangerous structure"));
        assert!(error.contains("in_conflict=true"));

        // Should have the seed
        assert!(ce.is_some());
        let ce = ce.unwrap();
        assert_eq!(ce.dst_seed, Some(54321));

        // Should have description with invariant details
        assert!(ce.description.is_some());
        let desc = ce.description.unwrap();
        assert!(desc.contains("NoCommittedDangerousStructures"));
    }

    #[test]
    fn test_extract_invariant_failures_includes_trace() {
        let stderr = r#"
=== DST INVARIANT FAILURES ===
DST_SEED=99999
INVARIANT_FAILED: FirstCommitterWins
  Message: Concurrent transactions T1 and T2 both committed writes to key 100

Operation trace (last 20):
  0: Begin(1)
  1: Begin(2)
  2: Write { txn: 1, key: 100, value: 1 }
  3: Write { txn: 2, key: 100, value: 2 }
  4: Commit(1)
  5: Commit(2)
=== END INVARIANT FAILURES ===
"#;
        let (error, _) = extract_dst_error(stderr, "");

        // Should include operation trace
        assert!(error.contains("Recent operations:"));
        assert!(error.contains("Begin(1)"));
    }
}
