//! Code generator with verification loop.
//!
//! Implements the generate → verify → fix cycle using LLM and the evaluator cascade.

use std::path::Path;
use std::time::{Duration, Instant};

use vf_core::TlaSpec;
use vf_evaluators::{CascadeConfig, CascadeResult, EvaluatorCascade, EvaluatorLevel};

use crate::client::{ClaudeClient, ClientError, Message};
use crate::prompt::{extract_code_block, PromptBuilder, SpecType};

/// Generator configuration.
#[derive(Debug, Clone)]
pub struct GeneratorConfig {
    /// Maximum number of generation attempts
    pub max_attempts: u32,
    /// Cascade configuration for verification
    pub cascade_config: CascadeConfig,
    /// Whether to print verbose output
    pub verbose: bool,
    /// Output directory for generated code
    pub output_dir: Option<String>,
}

impl Default for GeneratorConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            cascade_config: CascadeConfig::default(),
            verbose: false,
            output_dir: None,
        }
    }
}

impl GeneratorConfig {
    /// Quick config for fast iteration.
    pub fn quick() -> Self {
        Self {
            max_attempts: 3,
            cascade_config: CascadeConfig::fast(),
            verbose: true,
            ..Default::default()
        }
    }

    /// Thorough config for production.
    pub fn thorough() -> Self {
        Self {
            max_attempts: 10,
            cascade_config: CascadeConfig::thorough(),
            verbose: true,
            ..Default::default()
        }
    }
}

/// Result from code generation.
#[derive(Debug, Clone)]
pub struct GeneratorResult {
    /// Whether generation succeeded
    pub success: bool,
    /// The generated code (if successful)
    pub code: Option<String>,
    /// Number of attempts made
    pub attempts: u32,
    /// Total duration
    pub duration: Duration,
    /// Cascade result from final attempt
    pub cascade_result: Option<CascadeResult>,
    /// History of all attempts
    pub attempt_history: Vec<AttemptRecord>,
}

/// Record of a single generation attempt.
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    /// Attempt number (1-indexed)
    pub attempt: u32,
    /// Generated code
    pub code: String,
    /// Cascade result
    pub cascade_result: CascadeResult,
    /// Duration of this attempt
    pub duration: Duration,
}

impl GeneratorResult {
    /// Format as a summary string.
    pub fn format_summary(&self) -> String {
        let status = if self.success { "SUCCESS" } else { "FAILED" };
        let mut summary = format!(
            "[{}] Generation completed in {:.2}s ({} attempts)\n",
            status,
            self.duration.as_secs_f64(),
            self.attempts
        );

        if let Some(ref result) = self.cascade_result {
            summary.push_str(&result.format_report());
        }

        if self.success {
            if let Some(ref code) = self.code {
                let lines = code.lines().count();
                summary.push_str(&format!("\nGenerated {} lines of code.\n", lines));
            }
        } else {
            summary.push_str("\nGeneration failed after all attempts.\n");
            for record in &self.attempt_history {
                if let Some(ref failure) = record.cascade_result.first_failure {
                    summary.push_str(&format!(
                        "  Attempt {}: Failed at {} - {}\n",
                        record.attempt,
                        failure.evaluator,
                        failure.error.as_deref().unwrap_or("unknown")
                    ));
                }
            }
        }

        summary
    }
}

/// LLM-powered code generator with verification.
pub struct CodeGenerator {
    client: ClaudeClient,
    config: GeneratorConfig,
}

impl CodeGenerator {
    /// Create a new generator with the given client and config.
    pub fn new(client: ClaudeClient, config: GeneratorConfig) -> Self {
        Self { client, config }
    }

    /// Create from environment variables.
    pub fn from_env(config: GeneratorConfig) -> Result<Self, ClientError> {
        let client = ClaudeClient::from_env()?;
        Ok(Self::new(client, config))
    }

    /// Generate implementation from a TLA+ spec file.
    pub async fn generate_from_file(&self, spec_path: &Path) -> Result<GeneratorResult, GeneratorError> {
        let spec = TlaSpec::from_file(spec_path)
            .map_err(|e| GeneratorError::SpecError(e.to_string()))?;

        self.generate(&spec).await
    }

    /// Generate implementation from a TLA+ spec.
    pub async fn generate(&self, spec: &TlaSpec) -> Result<GeneratorResult, GeneratorError> {
        let start = Instant::now();
        let mut attempt_history = Vec::new();
        let mut current_code: Option<String> = None;

        // Create spec-type-aware prompt builder
        let prompt_builder = PromptBuilder::for_spec(spec);
        let spec_type = prompt_builder.spec_type();

        if self.config.verbose {
            println!("Generating implementation for: {}", spec.name);
            println!("Spec type: {:?}", spec_type);
            println!("Invariants: {}", spec.format_invariants());
            println!();
        }

        for attempt in 1..=self.config.max_attempts {
            let attempt_start = Instant::now();

            if self.config.verbose {
                println!("=== Attempt {}/{} ===", attempt, self.config.max_attempts);
            }

            // Generate or fix code
            let code = if let Some(ref prev_code) = current_code {
                // Fix based on previous failure
                let last_result = attempt_history.last().map(|r: &AttemptRecord| &r.cascade_result);
                self.fix_code(&prompt_builder, spec, prev_code, last_result)
                    .await?
            } else {
                // Initial generation
                self.generate_initial(&prompt_builder, spec).await?
            };

            if self.config.verbose {
                println!("Generated {} lines of code", code.lines().count());
            }

            // Verify with cascade
            let cascade_result = self.verify_code(&code, spec_type).await?;

            let attempt_record = AttemptRecord {
                attempt,
                code: code.clone(),
                cascade_result: cascade_result.clone(),
                duration: attempt_start.elapsed(),
            };
            attempt_history.push(attempt_record);

            if cascade_result.all_passed {
                if self.config.verbose {
                    println!("✅ All evaluators passed!");
                }

                return Ok(GeneratorResult {
                    success: true,
                    code: Some(code),
                    attempts: attempt,
                    duration: start.elapsed(),
                    cascade_result: Some(cascade_result),
                    attempt_history,
                });
            }

            // Log failure
            if self.config.verbose {
                if let Some(ref failure) = cascade_result.first_failure {
                    println!(
                        "❌ Failed at {}: {}",
                        failure.evaluator,
                        failure.error.as_deref().unwrap_or("unknown")
                    );
                }
            }

            current_code = Some(code);
        }

        // All attempts exhausted
        Ok(GeneratorResult {
            success: false,
            code: current_code,
            attempts: self.config.max_attempts,
            duration: start.elapsed(),
            cascade_result: attempt_history.last().map(|r| r.cascade_result.clone()),
            attempt_history,
        })
    }

    /// Generate initial implementation.
    async fn generate_initial(
        &self,
        prompt_builder: &PromptBuilder,
        spec: &TlaSpec,
    ) -> Result<String, GeneratorError> {
        let prompt = prompt_builder.build_generation_prompt(spec);
        let system = prompt_builder.system_prompt().to_string();

        let messages = vec![Message::user(prompt)];

        let response = self
            .client
            .complete_with_system(messages, Some(system))
            .await
            .map_err(|e| GeneratorError::ClientError(e))?;

        extract_code_block(&response)
            .ok_or_else(|| GeneratorError::NoCodeInResponse(response))
    }

    /// Fix code based on verification failure.
    async fn fix_code(
        &self,
        prompt_builder: &PromptBuilder,
        spec: &TlaSpec,
        previous_code: &str,
        previous_result: Option<&CascadeResult>,
    ) -> Result<String, GeneratorError> {
        let prompt = if let Some(result) = previous_result {
            prompt_builder.build_fix_prompt(spec, previous_code, result)
        } else {
            // Fallback if no result
            format!(
                "The following code has bugs. Fix it:\n\n```rust\n{}\n```",
                previous_code
            )
        };

        let system = prompt_builder.system_prompt().to_string();
        let messages = vec![Message::user(prompt)];

        let response = self
            .client
            .complete_with_system(messages, Some(system))
            .await
            .map_err(|e| GeneratorError::ClientError(e))?;

        extract_code_block(&response)
            .ok_or_else(|| GeneratorError::NoCodeInResponse(response))
    }

    /// Verify code using the evaluator cascade.
    async fn verify_code(
        &self,
        code: &str,
        spec_type: SpecType,
    ) -> Result<CascadeResult, GeneratorError> {
        let cascade = EvaluatorCascade::new(self.config.cascade_config.clone());

        // Create test code appropriate for the spec type
        let test_code = match spec_type {
            SpecType::LockFree => generate_stack_test_code(),
            SpecType::Ssi => generate_ssi_test_code(),
        };

        let result = cascade.run_on_code(code, &test_code).await;
        Ok(result)
    }

    /// Get the maximum cascade level being verified.
    pub fn max_level(&self) -> EvaluatorLevel {
        self.config.cascade_config.max_level
    }
}

/// Generate test code for lock-free stacks.
fn generate_stack_test_code() -> String {
    r#"
    #[test]
    fn test_basic_operations() {
        let stack = TreiberStack::new();
        stack.push(1);
        stack.push(2);
        stack.push(3);

        assert_eq!(stack.pop(), Some(3));
        assert_eq!(stack.pop(), Some(2));
        assert_eq!(stack.pop(), Some(1));
        assert_eq!(stack.pop(), None);
    }

    #[test]
    fn test_lifo_order() {
        let stack = TreiberStack::new();
        for i in 1..=5 {
            stack.push(i);
        }
        for i in (1..=5).rev() {
            assert_eq!(stack.pop(), Some(i));
        }
    }

    #[test]
    fn test_is_empty() {
        let stack = TreiberStack::new();
        assert!(stack.is_empty());
        stack.push(42);
        assert!(!stack.is_empty());
        stack.pop();
        assert!(stack.is_empty());
    }

    #[test]
    fn test_no_lost_elements() {
        let stack = TreiberStack::new();
        stack.push(10);
        stack.push(20);
        stack.push(30);

        let pushed = stack.pushed_elements();
        assert!(pushed.contains(&10));
        assert!(pushed.contains(&20));
        assert!(pushed.contains(&30));

        let contents = stack.get_contents();
        // All pushed elements should be in contents (none popped yet)
        for &val in &pushed {
            assert!(contents.contains(&val), "Lost element: {}", val);
        }
    }

    #[test]
    fn test_invariants() {
        // TLA+ Line 45: NoLostElements - every pushed element is either
        // in the stack or has been popped
        let stack = TreiberStack::new();

        // Push elements
        stack.push(100);
        stack.push(200);
        stack.push(300);

        // Pop one
        let popped_val = stack.pop();
        assert_eq!(popped_val, Some(300));

        // Verify NoLostElements invariant
        let pushed = stack.pushed_elements();
        let popped = stack.popped_elements();
        let contents = stack.get_contents();

        for &val in &pushed {
            let in_stack = contents.contains(&val);
            let was_popped = popped.contains(&val);
            assert!(in_stack || was_popped,
                "NoLostElements violated: {} neither in stack nor popped", val);
        }
    }
"#
    .to_string()
}

/// Generate test code for SSI implementations.
fn generate_ssi_test_code() -> String {
    r#"
    #[test]
    fn test_simple_transaction() {
        let store = SsiStore::new();

        // T1: write and commit
        let t1 = store.begin();
        assert!(store.write(t1, 1, 100));
        assert!(store.commit(t1));

        // T2: read committed value
        let t2 = store.begin();
        assert_eq!(store.read(t2, 1), Some(100));
        assert!(store.commit(t2));
    }

    #[test]
    fn test_snapshot_isolation() {
        let store = SsiStore::new();

        // T1: write initial value
        let t1 = store.begin();
        assert!(store.write(t1, 1, 100));
        assert!(store.commit(t1));

        // T2: start and read
        let t2 = store.begin();
        assert_eq!(store.read(t2, 1), Some(100));

        // T3: update value and commit
        let t3 = store.begin();
        assert!(store.write(t3, 1, 200));
        assert!(store.commit(t3));

        // T2 should still see old value (snapshot isolation)
        assert_eq!(store.read(t2, 1), Some(100));
        assert!(store.commit(t2));
    }

    #[test]
    fn test_dangerous_structure_abort() {
        let store = SsiStore::new();

        // Setup: Write initial values
        let setup = store.begin();
        store.write(setup, 1, 10);
        store.write(setup, 2, 20);
        store.commit(setup);

        // T1 and T2: create dangerous structure (write skew pattern)
        let t1 = store.begin();
        let t2 = store.begin();

        // T1 reads K1
        store.read(t1, 1);
        // T2 reads K2
        store.read(t2, 2);
        // T2 writes K1 (conflict with T1's read) - T1 gets out_conflict
        store.write(t2, 1, 11);
        store.commit(t2);
        // T1 writes K2 (conflict with T2's read) - T1 gets in_conflict
        store.write(t1, 2, 21);

        // T1's commit should fail due to dangerous structure
        let committed = store.commit(t1);
        assert!(!committed, "T1 should abort due to dangerous structure");
    }

    #[test]
    fn test_disjoint_keys_commit() {
        let store = SsiStore::new();

        // T1 and T2: write different keys concurrently
        let t1 = store.begin();
        let t2 = store.begin();

        assert!(store.write(t1, 1, 100));
        assert!(store.write(t2, 2, 200));

        // Both should commit (no conflicts)
        assert!(store.commit(t1));
        assert!(store.commit(t2));

        // Verify values
        let t3 = store.begin();
        assert_eq!(store.read(t3, 1), Some(100));
        assert_eq!(store.read(t3, 2), Some(200));
    }

    #[test]
    fn test_write_lock_blocking() {
        let store = SsiStore::new();

        // T1: write K1 (holds lock)
        let t1 = store.begin();
        assert!(store.write(t1, 1, 100));

        // T2: cannot write K1 (lock held)
        let t2 = store.begin();
        assert!(!store.write(t2, 1, 200)); // Should fail

        // T2: can write different key
        assert!(store.write(t2, 2, 300));

        // T1 commits, releases lock
        assert!(store.commit(t1));
        assert!(store.commit(t2));
    }

    #[test]
    fn test_single_conflict_flag_commits() {
        let store = SsiStore::new();

        // Setup
        let setup = store.begin();
        store.write(setup, 1, 10);
        store.commit(setup);

        // T1 reads K1
        let t1 = store.begin();
        store.read(t1, 1);

        // T2 writes K1 (T1 gets out_conflict, but not in_conflict)
        let t2 = store.begin();
        store.write(t2, 1, 20);
        store.commit(t2);

        // T1 should still be able to commit (only out_conflict, no in_conflict)
        assert!(store.commit(t1), "T1 should commit with only out_conflict");
    }

    #[test]
    fn test_committed_txns_tracking() {
        let store = SsiStore::new();

        let t1 = store.begin();
        store.write(t1, 1, 100);
        assert!(store.commit(t1));

        let committed = store.committed_txns();
        assert!(committed.contains(&t1), "T1 should be in committed set");

        let t2 = store.begin();
        store.write(t2, 2, 200);
        store.abort(t2);

        let committed = store.committed_txns();
        assert!(!committed.contains(&t2), "T2 should NOT be in committed set");
    }
"#
    .to_string()
}

/// Generator errors.
#[derive(Debug, thiserror::Error)]
pub enum GeneratorError {
    #[error("Spec error: {0}")]
    SpecError(String),

    #[error("Client error: {0}")]
    ClientError(#[from] ClientError),

    #[error("No code block found in response: {0}")]
    NoCodeInResponse(String),

    #[error("Verification error: {0}")]
    VerificationError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generator_config_presets() {
        let quick = GeneratorConfig::quick();
        assert_eq!(quick.max_attempts, 3);

        let thorough = GeneratorConfig::thorough();
        assert_eq!(thorough.max_attempts, 10);
    }

    #[test]
    fn test_generator_result_format() {
        let result = GeneratorResult {
            success: true,
            code: Some("fn foo() {}".to_string()),
            attempts: 2,
            duration: Duration::from_secs(5),
            cascade_result: None,
            attempt_history: Vec::new(),
        };

        let summary = result.format_summary();
        assert!(summary.contains("SUCCESS"));
        assert!(summary.contains("2 attempts"));
    }

    #[test]
    fn test_generate_stack_test_code() {
        let test_code = generate_stack_test_code();
        assert!(test_code.contains("test_basic_operations"));
        assert!(test_code.contains("test_no_lost_elements"));
    }

    #[test]
    fn test_generate_ssi_test_code() {
        let test_code = generate_ssi_test_code();
        assert!(test_code.contains("test_simple_transaction"));
        assert!(test_code.contains("test_dangerous_structure_abort"));
    }
}
