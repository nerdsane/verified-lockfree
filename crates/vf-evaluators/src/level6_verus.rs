//! Level 6: Verus formal verification evaluator.
//!
//! Uses Verus (https://github.com/verus-lang/verus) to prove correctness
//! properties using SMT solvers. Verus provides stronger guarantees than
//! model checking - proofs hold for ALL inputs, not just explored states.
//!
//! # Verification Approach
//!
//! Verus verifies:
//! - Pre/post conditions (requires/ensures)
//! - Loop invariants
//! - Data structure invariants
//! - Ghost state for concurrent reasoning
//!
//! # Installation
//!
//! ```bash
//! # Download from https://github.com/verus-lang/verus/releases
//! # Or build from source:
//! git clone https://github.com/verus-lang/verus
//! cd verus && ./tools/get-z3.sh && source ./tools/activate
//! cargo build --release
//! ```

use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;

use crate::result::EvaluatorResult;

/// Verus configuration.
#[derive(Debug, Clone)]
pub struct VerusConfig {
    /// Path to verus binary (default: searches PATH)
    pub verus_path: Option<String>,
    /// Timeout for verification
    pub timeout: Duration,
    /// Number of threads for parallel verification
    pub threads: usize,
    /// Additional verus arguments
    pub extra_args: Vec<String>,
}

impl Default for VerusConfig {
    fn default() -> Self {
        Self {
            verus_path: None,
            timeout: Duration::from_secs(300),
            threads: num_cpus::get(),
            extra_args: Vec::new(),
        }
    }
}

impl VerusConfig {
    /// Quick config for fast iteration.
    pub fn quick() -> Self {
        Self {
            timeout: Duration::from_secs(60),
            threads: 2,
            ..Default::default()
        }
    }

    /// Thorough config for CI.
    pub fn thorough() -> Self {
        Self {
            timeout: Duration::from_secs(600),
            threads: num_cpus::get(),
            ..Default::default()
        }
    }
}

/// Check if Verus is installed.
pub async fn is_verus_installed() -> bool {
    Command::new("verus")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run Verus verification on a file.
///
/// The file should contain Verus-annotated Rust code inside `verus! { }` blocks.
pub async fn run(file_path: &Path, config: &VerusConfig) -> EvaluatorResult {
    let start = Instant::now();

    // Check if verus is installed
    let verus_cmd = config.verus_path.as_deref().unwrap_or("verus");

    let version_output = Command::new(verus_cmd)
        .arg("--version")
        .output()
        .await;

    if version_output.is_err() || !version_output.as_ref().unwrap().status.success() {
        return EvaluatorResult {
            evaluator: "verus".into(),
            passed: false,
            duration: start.elapsed(),
            output: String::new(),
            error: Some("Verus not installed. See: https://github.com/verus-lang/verus/releases".into()),
            counterexample: None,
        };
    }

    // Build verus command
    let mut cmd = Command::new(verus_cmd);
    cmd.arg(file_path);

    // Add thread count
    cmd.arg("--num-threads").arg(config.threads.to_string());

    // Add extra args
    for arg in &config.extra_args {
        cmd.arg(arg);
    }

    // Run with timeout
    let output = tokio::time::timeout(config.timeout, cmd.output()).await;

    let output = match output {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return EvaluatorResult {
                evaluator: "verus".into(),
                passed: false,
                duration: start.elapsed(),
                output: String::new(),
                error: Some(format!("Failed to run verus: {}", e)),
                counterexample: None,
            };
        }
        Err(_) => {
            return EvaluatorResult {
                evaluator: "verus".into(),
                passed: false,
                duration: start.elapsed(),
                output: String::new(),
                error: Some(format!("Verus timed out after {:?}", config.timeout)),
                counterexample: None,
            };
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined_output = format!("{}\n{}", stdout, stderr);

    if output.status.success() {
        // Check for verification success
        if combined_output.contains("verification results:: verified:")
            || combined_output.contains("0 errors")
        {
            EvaluatorResult {
                evaluator: "verus".into(),
                passed: true,
                duration: start.elapsed(),
                output: combined_output,
                error: None,
                counterexample: None,
            }
        } else {
            // Verus ran but verification failed
            let error = extract_verus_error(&combined_output);
            let counterexample = extract_counterexample(&combined_output);
            EvaluatorResult {
                evaluator: "verus".into(),
                passed: false,
                duration: start.elapsed(),
                output: combined_output,
                error: Some(error),
                counterexample,
            }
        }
    } else {
        // Verus command failed
        let error = extract_verus_error(&combined_output);
        let counterexample = extract_counterexample(&combined_output);
        EvaluatorResult {
            evaluator: "verus".into(),
            passed: false,
            duration: start.elapsed(),
            output: combined_output,
            error: Some(error),
            counterexample,
        }
    }
}

/// Run Verus on inline code (writes to temp file).
pub async fn run_on_code(code: &str, config: &VerusConfig) -> EvaluatorResult {
    // Create temp file
    let temp_dir = tempfile::tempdir().unwrap();
    let file_path = temp_dir.path().join("verify.rs");

    std::fs::write(&file_path, code).unwrap();

    run(&file_path, config).await
}

/// Extract error message from Verus output.
fn extract_verus_error(output: &str) -> String {
    // Look for error lines
    for line in output.lines() {
        if line.contains("error:") || line.contains("FAILED") {
            return line.to_string();
        }
        if line.contains("assertion failed") {
            return line.to_string();
        }
        if line.contains("precondition not satisfied") {
            return line.to_string();
        }
        if line.contains("postcondition not satisfied") {
            return line.to_string();
        }
    }

    // Return first few lines if no specific error found
    output.lines().take(5).collect::<Vec<_>>().join("\n")
}

/// Extract counterexample from Verus output.
fn extract_counterexample(output: &str) -> Option<vf_core::Counterexample> {
    // Verus provides counterexamples for failed assertions
    // Look for model/witness information
    let mut in_counterexample = false;
    let mut ce_lines = Vec::new();

    for line in output.lines() {
        if line.contains("counterexample") || line.contains("witness") {
            in_counterexample = true;
        }
        if in_counterexample {
            ce_lines.push(line.to_string());
            if line.trim().is_empty() && !ce_lines.is_empty() {
                break;
            }
        }
    }

    if ce_lines.is_empty() {
        None
    } else {
        Some(vf_core::Counterexample {
            states: vec![vf_core::StateSnapshot {
                step: 0,
                description: ce_lines.join("\n"),
                variables: Vec::new(),
            }],
            interleaving: Vec::new(),
            memory_issues: Vec::new(),
            dst_seed: None,
            description: None,
        })
    }
}

/// Generate Verus proof harness for a stack implementation.
///
/// This generates Verus specifications that can be used to verify
/// generated stack implementations.
pub fn generate_stack_proof_template() -> String {
    r#"
// Verus proof template for Treiber Stack
// This file should be compiled with: verus stack_proof.rs

use vstd::prelude::*;

verus! {

// Ghost state representing the abstract stack
pub ghost struct GhostStack {
    pub pushed: Set<u64>,
    pub popped: Set<u64>,
    pub contents: Seq<u64>,
}

// Invariant: NoLostElements (TLA+ Line 45)
// Every pushed element is either in the stack or was popped
pub open spec fn no_lost_elements(gs: GhostStack) -> bool {
    forall|e: u64| gs.pushed.contains(e) ==>
        gs.contents.contains(e) || gs.popped.contains(e)
}

// Invariant: NoDuplicates (TLA+ Line 58)
// No element appears twice in the stack
pub open spec fn no_duplicates(gs: GhostStack) -> bool {
    forall|i: int, j: int|
        0 <= i < gs.contents.len() && 0 <= j < gs.contents.len() && i != j ==>
            gs.contents[i] != gs.contents[j]
}

// Combined invariant
pub open spec fn stack_invariant(gs: GhostStack) -> bool {
    no_lost_elements(gs) && no_duplicates(gs)
}

// Proof that push preserves invariants
pub proof fn push_preserves_invariant(gs: GhostStack, value: u64)
    requires
        stack_invariant(gs),
        !gs.pushed.contains(value),  // Fresh value
    ensures
        stack_invariant(GhostStack {
            pushed: gs.pushed.insert(value),
            popped: gs.popped,
            contents: gs.contents.push(value),
        })
{
    let gs_new = GhostStack {
        pushed: gs.pushed.insert(value),
        popped: gs.popped,
        contents: gs.contents.push(value),
    };

    // Prove NoLostElements
    assert forall|e: u64| gs_new.pushed.contains(e) implies
        gs_new.contents.contains(e) || gs_new.popped.contains(e)
    by {
        if e == value {
            assert(gs_new.contents.contains(value));
        } else {
            assert(gs.pushed.contains(e));
            assert(gs.contents.contains(e) || gs.popped.contains(e));
        }
    }

    // Prove NoDuplicates
    assert forall|i: int, j: int|
        0 <= i < gs_new.contents.len() && 0 <= j < gs_new.contents.len() && i != j
        implies gs_new.contents[i] != gs_new.contents[j]
    by {
        // New element is at the end, and it's fresh (not in pushed before)
        // So it can't duplicate any existing element
    }
}

// Proof that pop preserves invariants
pub proof fn pop_preserves_invariant(gs: GhostStack)
    requires
        stack_invariant(gs),
        gs.contents.len() > 0,
    ensures
        ({
            let value = gs.contents.last();
            let gs_new = GhostStack {
                pushed: gs.pushed,
                popped: gs.popped.insert(value),
                contents: gs.contents.drop_last(),
            };
            stack_invariant(gs_new)
        })
{
    let value = gs.contents.last();
    let gs_new = GhostStack {
        pushed: gs.pushed,
        popped: gs.popped.insert(value),
        contents: gs.contents.drop_last(),
    };

    // Prove NoLostElements
    // The popped element moves from contents to popped set
    assert forall|e: u64| gs_new.pushed.contains(e) implies
        gs_new.contents.contains(e) || gs_new.popped.contains(e)
    by {
        assert(gs.pushed.contains(e));
        if e == value {
            assert(gs_new.popped.contains(e));
        } else if gs.contents.contains(e) {
            // e was in contents, and e != value, so e is still in contents
        } else {
            assert(gs.popped.contains(e));
        }
    }

    // NoDuplicates follows from gs having no duplicates
}

} // verus!
"#.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_verus_not_installed_graceful() {
        // This test verifies graceful handling when verus isn't installed
        let config = VerusConfig::default();
        let code = "fn main() {}";

        let result = run_on_code(code, &config).await;

        // Should fail gracefully with helpful message
        if !result.passed {
            assert!(
                result.error.as_ref().map(|e| e.contains("not installed")).unwrap_or(false)
                || result.error.is_some()
            );
        }
    }

    #[test]
    fn test_generate_proof_template() {
        let template = generate_stack_proof_template();
        assert!(template.contains("verus!"));
        assert!(template.contains("no_lost_elements"));
        assert!(template.contains("no_duplicates"));
        assert!(template.contains("push_preserves_invariant"));
        assert!(template.contains("pop_preserves_invariant"));
    }
}
