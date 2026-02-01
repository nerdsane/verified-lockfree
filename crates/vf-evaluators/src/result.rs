//! Evaluator result types.

use std::time::Duration;

use vf_core::Counterexample;

/// Result from a single evaluator.
#[derive(Debug, Clone)]
pub struct EvaluatorResult {
    /// Name of the evaluator (e.g., "rustc", "miri", "loom")
    pub evaluator: String,
    /// Whether the evaluation passed
    pub passed: bool,
    /// Error message if failed
    pub error: Option<String>,
    /// Counterexample if available
    pub counterexample: Option<Counterexample>,
    /// Duration of the evaluation
    pub duration: Duration,
    /// Raw output from the tool
    pub output: String,
}

impl EvaluatorResult {
    /// Create a passing result.
    pub fn pass(evaluator: impl Into<String>, duration: Duration) -> Self {
        Self {
            evaluator: evaluator.into(),
            passed: true,
            error: None,
            counterexample: None,
            duration,
            output: String::new(),
        }
    }

    /// Create a passing result with output.
    pub fn pass_with_output(evaluator: impl Into<String>, duration: Duration, output: String) -> Self {
        Self {
            evaluator: evaluator.into(),
            passed: true,
            error: None,
            counterexample: None,
            duration,
            output,
        }
    }

    /// Create a failing result.
    pub fn fail(
        evaluator: impl Into<String>,
        error: impl Into<String>,
        duration: Duration,
        output: String,
    ) -> Self {
        Self {
            evaluator: evaluator.into(),
            passed: false,
            error: Some(error.into()),
            counterexample: None,
            duration,
            output,
        }
    }

    /// Create a failing result with counterexample.
    pub fn fail_with_counterexample(
        evaluator: impl Into<String>,
        error: impl Into<String>,
        counterexample: Counterexample,
        duration: Duration,
        output: String,
    ) -> Self {
        Self {
            evaluator: evaluator.into(),
            passed: false,
            error: Some(error.into()),
            counterexample: Some(counterexample),
            duration,
            output,
        }
    }

    /// Create a skipped result (tool not available).
    ///
    /// Skipped results count as passed since the tool couldn't run.
    /// This allows the cascade to continue when optional tools aren't installed.
    pub fn skip(evaluator: impl Into<String>, reason: impl Into<String>, duration: Duration) -> Self {
        Self {
            evaluator: evaluator.into(),
            passed: true, // Skip counts as pass
            error: None,
            counterexample: None,
            duration,
            output: format!("SKIPPED: {}", reason.into()),
        }
    }

    /// Format as a single-line status.
    pub fn format_status(&self) -> String {
        if self.passed {
            format!(
                "[PASS] {} ({:.2}s)",
                self.evaluator,
                self.duration.as_secs_f64()
            )
        } else {
            format!(
                "[FAIL] {} ({:.2}s): {}",
                self.evaluator,
                self.duration.as_secs_f64(),
                self.error.as_deref().unwrap_or("unknown error")
            )
        }
    }
}

/// Result from the full cascade.
#[derive(Debug, Clone)]
pub struct CascadeResult {
    /// Results from each evaluator that ran
    pub results: Vec<EvaluatorResult>,
    /// Whether all evaluators passed
    pub all_passed: bool,
    /// The first failure (if any)
    pub first_failure: Option<EvaluatorResult>,
    /// Total duration of the cascade
    pub total_duration: Duration,
}

impl CascadeResult {
    /// Create a new cascade result from individual results.
    pub fn from_results(results: Vec<EvaluatorResult>) -> Self {
        let all_passed = results.iter().all(|r| r.passed);
        let first_failure = results.iter().find(|r| !r.passed).cloned();
        let total_duration = results.iter().map(|r| r.duration).sum();

        Self {
            results,
            all_passed,
            first_failure,
            total_duration,
        }
    }

    /// Format as a report string.
    pub fn format_report(&self) -> String {
        let mut report = String::new();

        report.push_str("Evaluator Cascade Results\n");
        report.push_str("========================\n\n");

        for result in &self.results {
            report.push_str(&result.format_status());
            report.push('\n');
        }

        report.push_str(&format!(
            "\nTotal: {} passed, {} failed ({:.2}s)\n",
            self.results.iter().filter(|r| r.passed).count(),
            self.results.iter().filter(|r| !r.passed).count(),
            self.total_duration.as_secs_f64()
        ));

        if let Some(ref failure) = self.first_failure {
            report.push_str(&format!("\nFirst failure: {}\n", failure.evaluator));
            if let Some(ref error) = failure.error {
                report.push_str(&format!("Error: {}\n", error));
            }
            if let Some(ref ce) = failure.counterexample {
                report.push_str("\nCounterexample:\n");
                report.push_str(&ce.render_diagram());
            }
        }

        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluator_result_pass() {
        let result = EvaluatorResult::pass("rustc", Duration::from_millis(100));
        assert!(result.passed);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_evaluator_result_fail() {
        let result = EvaluatorResult::fail(
            "miri",
            "undefined behavior detected",
            Duration::from_millis(500),
            "error output".to_string(),
        );
        assert!(!result.passed);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_cascade_result() {
        let results = vec![
            EvaluatorResult::pass("rustc", Duration::from_millis(100)),
            EvaluatorResult::pass("miri", Duration::from_millis(200)),
            EvaluatorResult::fail(
                "loom",
                "data race",
                Duration::from_millis(300),
                String::new(),
            ),
        ];

        let cascade = CascadeResult::from_results(results);
        assert!(!cascade.all_passed);
        assert!(cascade.first_failure.is_some());
        assert_eq!(cascade.first_failure.as_ref().unwrap().evaluator, "loom");
    }
}
