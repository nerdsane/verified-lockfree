//! Level 4: Stateright model checking evaluator.
//!
//! Runs bounded model checking using Stateright to verify
//! that implementations satisfy TLA+ spec invariants.
//!
//! Stateright explores all reachable states and checks invariants
//! at each state, providing counterexamples when violations are found.

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;
use vf_core::Counterexample;

use crate::result::EvaluatorResult;

/// Stateright configuration.
#[derive(Debug, Clone)]
pub struct StaterightConfig {
    /// Maximum depth for bounded model checking
    pub depth_max: usize,
    /// Number of threads for parallel exploration
    pub threads: usize,
    /// Timeout for model checking
    pub timeout: Duration,
    /// Test filter (regex pattern)
    pub filter: Option<String>,
}

impl Default for StaterightConfig {
    fn default() -> Self {
        Self {
            depth_max: 100,
            threads: num_cpus::get(),
            timeout: Duration::from_secs(120),
            filter: None,
        }
    }
}

impl StaterightConfig {
    /// Quick config for fast iteration.
    pub fn quick() -> Self {
        Self {
            depth_max: 50,
            threads: 2,
            timeout: Duration::from_secs(30),
            filter: None,
        }
    }

    /// Thorough config for CI.
    pub fn thorough() -> Self {
        Self {
            depth_max: 200,
            threads: num_cpus::get(),
            timeout: Duration::from_secs(300),
            filter: None,
        }
    }

    /// Exhaustive config for maximum coverage.
    pub fn exhaustive() -> Self {
        Self {
            depth_max: 500,
            threads: num_cpus::get(),
            timeout: Duration::from_secs(600),
            filter: None,
        }
    }
}

/// Run stateright model checking tests on a crate.
///
/// Looks for tests marked with `#[test]` that use stateright's model checking.
pub async fn run(crate_path: &Path, timeout: Duration, depth_max: usize) -> EvaluatorResult {
    run_with_config(
        crate_path,
        StaterightConfig {
            depth_max,
            timeout,
            ..Default::default()
        },
    )
    .await
}

/// Run stateright with full configuration.
pub async fn run_with_config(crate_path: &Path, config: StaterightConfig) -> EvaluatorResult {
    let start = Instant::now();

    // Build and run tests that use stateright
    let mut cmd = Command::new("cargo");
    cmd.args(["test", "--release"]);

    // Add filter for stateright tests
    if let Some(ref filter) = config.filter {
        cmd.arg(filter);
    } else {
        // Default: run tests with "stateright" or "model" in the name
        cmd.arg("stateright");
    }

    cmd.arg("--");
    cmd.arg("--test-threads=1"); // Stateright tests should run sequentially

    // Set environment for stateright configuration
    cmd.env("STATERIGHT_DEPTH_MAX", config.depth_max.to_string());
    cmd.env("STATERIGHT_THREADS", config.threads.to_string());

    cmd.current_dir(crate_path);

    let result = tokio::time::timeout(config.timeout, cmd.output()).await;

    let duration = start.elapsed();

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let combined = format!("{}\n{}", stdout, stderr);

            if output.status.success() {
                let stats = extract_stateright_stats(&combined);
                EvaluatorResult::pass_with_output(
                    "stateright",
                    duration,
                    format!("{}\n{}", stats, combined),
                )
            } else {
                let (error, counterexample) = extract_stateright_error(&combined);
                if let Some(ce) = counterexample {
                    EvaluatorResult::fail_with_counterexample(
                        "stateright",
                        error,
                        ce,
                        duration,
                        combined,
                    )
                } else {
                    EvaluatorResult::fail("stateright", error, duration, combined)
                }
            }
        }
        Ok(Err(e)) => EvaluatorResult::fail(
            "stateright",
            format!("Failed to run stateright tests: {}", e),
            duration,
            String::new(),
        ),
        Err(_) => EvaluatorResult::fail(
            "stateright",
            format!("Timeout after {:?}", config.timeout),
            duration,
            String::new(),
        ),
    }
}

/// Run inline model checking (for testing the evaluator itself).
///
/// This runs model checking directly in-process using a provided model.
pub fn run_inline<M>(model: M, config: StaterightConfig) -> InlineResult
where
    M: stateright::Model + Send + Sync + 'static,
    M::State: Clone + std::fmt::Debug + std::hash::Hash + Eq + Send + Sync,
    M::Action: Clone + std::fmt::Debug + Send + Sync,
{
    use stateright::Checker;

    let start = Instant::now();

    let checker = model
        .checker()
        .threads(config.threads)
        .spawn_bfs()
        .join();

    let duration = start.elapsed();

    // Check if any properties were violated
    if checker.is_done() {
        // Get discovery statistics
        let state_count = checker.unique_state_count();

        // Try to assert properties (this will panic if violated)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            checker.assert_properties();
        }));

        match result {
            Ok(()) => InlineResult {
                passed: true,
                state_count,
                error: None,
                counterexample: None,
                duration,
            },
            Err(panic_info) => {
                let error_msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_info.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "Property violation detected".to_string()
                };

                InlineResult {
                    passed: false,
                    state_count,
                    error: Some(error_msg),
                    counterexample: None, // TODO: extract from checker
                    duration,
                }
            }
        }
    } else {
        InlineResult {
            passed: false,
            state_count: checker.unique_state_count(),
            error: Some("Model checking did not complete".to_string()),
            counterexample: None,
            duration,
        }
    }
}

/// Result from inline model checking.
#[derive(Debug, Clone)]
pub struct InlineResult {
    pub passed: bool,
    pub state_count: usize,
    pub error: Option<String>,
    pub counterexample: Option<Vec<String>>,
    pub duration: Duration,
}

impl InlineResult {
    pub fn format(&self) -> String {
        if self.passed {
            format!(
                "[PASS] stateright: {} states explored in {:.2}s",
                self.state_count,
                self.duration.as_secs_f64()
            )
        } else {
            format!(
                "[FAIL] stateright: {} states explored in {:.2}s\n  Error: {}",
                self.state_count,
                self.duration.as_secs_f64(),
                self.error.as_deref().unwrap_or("unknown")
            )
        }
    }
}

/// Extract stateright statistics from output.
fn extract_stateright_stats(output: &str) -> String {
    let mut stats = Vec::new();

    for line in output.lines() {
        if line.contains("states discovered")
            || line.contains("unique states")
            || line.contains("model checking")
            || line.contains("BFS")
            || line.contains("DFS")
        {
            stats.push(line.trim().to_string());
        }
    }

    if stats.is_empty() {
        "No stateright stats found in output".to_string()
    } else {
        stats.join("\n")
    }
}

/// Extract error and counterexample from stateright output.
fn extract_stateright_error(output: &str) -> (String, Option<Counterexample>) {
    let mut error = String::new();
    let mut path_lines = Vec::new();
    let mut in_path = false;

    for line in output.lines() {
        // Look for property violation (case-insensitive)
        let lower = line.to_lowercase();
        if lower.contains("violated") || lower.contains("property") && lower.contains("fail") {
            error = line.to_string();
        }

        // Look for counterexample path
        if line.contains("Path:") || line.contains("Counterexample:") {
            in_path = true;
            continue;
        }

        if in_path {
            if line.trim().is_empty() || line.contains("---") {
                in_path = false;
            } else {
                path_lines.push(line.to_string());
            }
        }

        // Capture assertion failures
        if line.contains("assertion failed") || line.contains("panicked at") {
            if error.is_empty() {
                error = line.to_string();
            }
        }
    }

    if error.is_empty() {
        error = "Stateright model checking failed".to_string();
    }

    // Build counterexample from path
    let counterexample = if !path_lines.is_empty() {
        use vf_core::{StateSnapshot, ThreadAction};

        let mut ce = Counterexample::new();

        // Parse path lines into state snapshots
        for (i, line) in path_lines.iter().enumerate() {
            // Try to parse "State N: ..." or "Action: ..." format
            let trimmed = line.trim();
            if trimmed.starts_with("State") || trimmed.starts_with("Action") {
                ce.add_state(StateSnapshot {
                    step: i as u64,
                    description: trimmed.to_string(),
                    variables: Vec::new(),
                });
            } else if !trimmed.is_empty() {
                // Generic action
                ce.add_action(ThreadAction {
                    thread_id: 0,
                    step: i as u64,
                    action: trimmed.to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_presets() {
        let quick = StaterightConfig::quick();
        assert_eq!(quick.depth_max, 50);

        let thorough = StaterightConfig::thorough();
        assert_eq!(thorough.depth_max, 200);

        let exhaustive = StaterightConfig::exhaustive();
        assert_eq!(exhaustive.depth_max, 500);
    }

    #[test]
    fn test_extract_stateright_stats() {
        let output = r#"
running 1 test
Model checking with BFS...
1000 states discovered, 500 unique states
test stateright_test ... ok
"#;
        let stats = extract_stateright_stats(output);
        assert!(stats.contains("states discovered"));
    }

    #[test]
    fn test_extract_stateright_error() {
        let output = r#"
Property "NoLostElements" violated!
Path:
  State 0: head=None
  Action: Push(1)
  State 1: head=Some(0)
  Action: Pop
  State 2: head=None, lost=1
---
"#;
        let (error, ce) = extract_stateright_error(output);
        assert!(error.contains("violated"));
        assert!(ce.is_some());
    }

    #[test]
    fn test_inline_result_format() {
        let result = InlineResult {
            passed: true,
            state_count: 1000,
            error: None,
            counterexample: None,
            duration: Duration::from_secs(2),
        };
        let formatted = result.format();
        assert!(formatted.contains("PASS"));
        assert!(formatted.contains("1000 states"));
    }
}
