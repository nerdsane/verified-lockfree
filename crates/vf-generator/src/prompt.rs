//! Prompt generation from TLA+ specs.
//!
//! Transforms TLA+ specifications into prompts that guide LLM code generation.
//! Supports multiple spec types: lock-free stacks and SSI transactions.

use vf_core::TlaSpec;
use vf_evaluators::CascadeResult;

/// Spec type determines which prompts and verification to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecType {
    /// Lock-free data structures (Treiber Stack, MS Queue)
    LockFree,
    /// Serializable Snapshot Isolation transactions
    Ssi,
}

impl SpecType {
    /// Detect spec type from TLA+ spec name or content.
    pub fn detect(spec: &TlaSpec) -> Self {
        let name_lower = spec.name.to_lowercase();
        let content_lower = spec.content.to_lowercase();

        if name_lower.contains("ssi")
            || name_lower.contains("snapshot")
            || name_lower.contains("isolation")
            || content_lower.contains("in_conflict")
            || content_lower.contains("out_conflict")
            || content_lower.contains("siread")
        {
            SpecType::Ssi
        } else {
            SpecType::LockFree
        }
    }
}

/// Template for code generation prompts.
#[derive(Debug, Clone)]
pub struct PromptTemplate {
    /// System prompt establishing context
    pub system: String,
    /// Main prompt template with placeholders
    pub main: String,
    /// Fix prompt template for counterexample feedback
    pub fix: String,
}

impl Default for PromptTemplate {
    fn default() -> Self {
        Self {
            system: DEFAULT_SYSTEM_PROMPT.to_string(),
            main: DEFAULT_MAIN_PROMPT.to_string(),
            fix: DEFAULT_FIX_PROMPT.to_string(),
        }
    }
}

impl PromptTemplate {
    /// Create templates for SSI specs.
    pub fn for_ssi() -> Self {
        Self {
            system: SSI_SYSTEM_PROMPT.to_string(),
            main: SSI_MAIN_PROMPT.to_string(),
            fix: SSI_FIX_PROMPT.to_string(),
        }
    }

    /// Create templates for lock-free specs.
    pub fn for_lockfree() -> Self {
        Self::default()
    }

    /// Create templates based on spec type.
    pub fn for_spec_type(spec_type: SpecType) -> Self {
        match spec_type {
            SpecType::LockFree => Self::for_lockfree(),
            SpecType::Ssi => Self::for_ssi(),
        }
    }
}

/// Builder for generating prompts from TLA+ specs.
pub struct PromptBuilder {
    template: PromptTemplate,
    spec_type: SpecType,
}

impl PromptBuilder {
    /// Create a new prompt builder with default templates (lock-free).
    pub fn new() -> Self {
        Self {
            template: PromptTemplate::default(),
            spec_type: SpecType::LockFree,
        }
    }

    /// Create a prompt builder for a specific spec type.
    pub fn for_spec_type(spec_type: SpecType) -> Self {
        Self {
            template: PromptTemplate::for_spec_type(spec_type),
            spec_type,
        }
    }

    /// Create a prompt builder auto-detecting type from spec.
    pub fn for_spec(spec: &TlaSpec) -> Self {
        let spec_type = SpecType::detect(spec);
        Self::for_spec_type(spec_type)
    }

    /// Create with custom template.
    pub fn with_template(template: PromptTemplate) -> Self {
        Self {
            template,
            spec_type: SpecType::LockFree,
        }
    }

    /// Get the detected spec type.
    pub fn spec_type(&self) -> SpecType {
        self.spec_type
    }

    /// Get the system prompt.
    pub fn system_prompt(&self) -> &str {
        &self.template.system
    }

    /// Build the initial generation prompt from a TLA+ spec.
    pub fn build_generation_prompt(&self, spec: &TlaSpec) -> String {
        let invariants = spec.format_invariants();
        let constants = spec.constants.join(", ");
        let variables = spec.variables.join(", ");

        self.template
            .main
            .replace("{MODULE_NAME}", &spec.name)
            .replace("{INVARIANTS}", &invariants)
            .replace("{CONSTANTS}", &constants)
            .replace("{VARIABLES}", &variables)
            .replace("{TLA_CONTENT}", &spec.content)
    }

    /// Build a fix prompt from a failed verification result.
    pub fn build_fix_prompt(
        &self,
        spec: &TlaSpec,
        previous_code: &str,
        result: &CascadeResult,
    ) -> String {
        let error_info = if let Some(ref failure) = result.first_failure {
            let mut info = format!("Evaluator: {}\n", failure.evaluator);
            if let Some(ref error) = failure.error {
                info.push_str(&format!("Error: {}\n", error));
            }
            if let Some(ref ce) = failure.counterexample {
                info.push_str(&format!("\nCounterexample:\n{}\n", ce.render_diagram()));
            }
            if !failure.output.is_empty() {
                // Include relevant portion of output
                let output_lines: Vec<&str> = failure.output.lines().take(50).collect();
                info.push_str(&format!("\nOutput:\n{}\n", output_lines.join("\n")));
            }
            info
        } else {
            "Unknown error".to_string()
        };

        self.template
            .fix
            .replace("{MODULE_NAME}", &spec.name)
            .replace("{INVARIANTS}", &spec.format_invariants())
            .replace("{PREVIOUS_CODE}", previous_code)
            .replace("{ERROR_INFO}", &error_info)
    }

    /// Build a prompt for a specific evaluator failure.
    pub fn build_targeted_fix_prompt(
        &self,
        evaluator: &str,
        error: &str,
        previous_code: &str,
    ) -> String {
        format!(
            r#"The following Rust code failed at the {} evaluator level:

```rust
{}
```

Error:
{}

Fix the code to pass the {} evaluator. Explain what was wrong and provide the corrected implementation.

Return ONLY the fixed Rust code in a ```rust code block.
"#,
            evaluator, previous_code, error, evaluator
        )
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Default system prompt for code generation.
const DEFAULT_SYSTEM_PROMPT: &str = r#"You are an expert Rust systems programmer specializing in lock-free data structures and formal verification.

Your task is to implement verified lock-free data structures that:
1. Satisfy formal TLA+ specifications
2. Pass a rigorous evaluator cascade (rustc → miri → loom → DST → stateright → kani)
3. Follow TigerStyle coding principles (explicit limits, assertions, big-endian naming)

Key requirements:
- Use std::sync::atomic with correct memory orderings (Acquire/Release/AcqRel)
- Implement proper CAS retry loops
- Use epoch-based garbage collection (crossbeam-epoch) for memory safety
- Include assertions for invariants
- Handle all edge cases

When generating code:
1. Start with the struct definition
2. Implement the core operations (push/pop for stacks, enqueue/dequeue for queues)
3. Implement the Properties trait from vf-core
4. Include basic tests

Return ONLY the Rust code in a ```rust code block. No explanations outside the code."#;

/// Default main prompt template.
const DEFAULT_MAIN_PROMPT: &str = r#"Implement a lock-free {MODULE_NAME} in Rust that satisfies the following TLA+ specification.

## INVARIANTS TO PRESERVE

{INVARIANTS}

## TLA+ SPECIFICATION

Constants: {CONSTANTS}
Variables: {VARIABLES}

```tla
{TLA_CONTENT}
```

## REQUIREMENTS

1. Use `crossbeam_epoch` for safe memory reclamation
2. Use correct atomic orderings:
   - `Acquire` when loading to observe other threads' writes
   - `Release` when storing to publish writes to other threads
   - `AcqRel` for read-modify-write operations
3. Implement retry loops for CAS failures
4. Implement the `StackProperties` trait from `vf_core::invariants::stack`
5. Add `#[cfg(loom)]` conditional compilation for loom compatibility
6. Include at least 2 assertions per function

## TEMPLATE

The code must be SELF-CONTAINED - do not use external crates except crossbeam-epoch.
Do NOT use vf_core or any custom traits - implement tracking directly in the struct.

```rust
use std::sync::atomic::{{AtomicU64, Ordering}};
use std::sync::Mutex;
use std::collections::HashSet;
use crossbeam_epoch::{{self as epoch, Atomic, Owned, Shared}};

pub struct TreiberStack {{
    head: Atomic<Node>,
    // Track pushed/popped for verification
    pushed: Mutex<HashSet<u64>>,
    popped: Mutex<HashSet<u64>>,
}}

struct Node {{
    value: u64,
    next: Atomic<Node>,
}}

impl TreiberStack {{
    pub fn new() -> Self {{ ... }}
    pub fn push(&self, value: u64) {{ ... }}
    pub fn pop(&self) -> Option<u64> {{ ... }}
    pub fn is_empty(&self) -> bool {{ ... }}

    // For testing invariants
    pub fn pushed_elements(&self) -> HashSet<u64> {{ ... }}
    pub fn popped_elements(&self) -> HashSet<u64> {{ ... }}
    pub fn get_contents(&self) -> Vec<u64> {{ ... }}
}}

impl Drop for TreiberStack {{
    fn drop(&mut self) {{ ... }}
}}

// REQUIRED: unsafe impl Send + Sync
unsafe impl Send for TreiberStack {{}}
unsafe impl Sync for TreiberStack {{}}
```

Generate the complete implementation. Do NOT include a tests module - tests will be added separately."#;

/// Default fix prompt template.
const DEFAULT_FIX_PROMPT: &str = r#"The following implementation of {MODULE_NAME} failed verification.

## REQUIRED INVARIANTS

{INVARIANTS}

## FAILED CODE

```rust
{PREVIOUS_CODE}
```

## VERIFICATION FAILURE

{ERROR_INFO}

## TASK

Fix the code to pass verification. Common issues include:
- Missing CAS retry loops (causes lost elements)
- Wrong memory orderings (causes data races)
- Incorrect tracking of pushed/popped elements
- Missing epoch::pin() guards for safe memory access
- Not handling spurious CAS failures

Analyze the error carefully and provide a corrected implementation.

Return ONLY the fixed Rust code in a ```rust code block."#;

// ============================================================================
// SSI-SPECIFIC PROMPTS
// ============================================================================

/// SSI system prompt.
const SSI_SYSTEM_PROMPT: &str = r#"You are an expert Rust systems programmer.

Your task: implement Serializable Snapshot Isolation (SSI) that satisfies the provided TLA+ specification.

The TLA+ spec defines the invariants. Read it carefully - it IS the correctness definition.
Your implementation will be verified against these invariants automatically.

Style requirements:
- Use u64 for IDs, not usize
- Include assertions (at least 2 per public function)
- Code must be self-contained (std only, no external crates)

The internal structure is up to you. Any implementation that satisfies the invariants is correct.

Return ONLY the Rust code in a ```rust code block. No explanations outside the code."#;

/// SSI main prompt template.
const SSI_MAIN_PROMPT: &str = r#"Implement Serializable Snapshot Isolation (SSI) in Rust that satisfies the following TLA+ specification.

## INVARIANTS TO PRESERVE

{INVARIANTS}

## TLA+ SPECIFICATION

Constants: {CONSTANTS}
Variables: {VARIABLES}

```tla
{TLA_CONTENT}
```

## REQUIRED API

Your `SsiStore` struct MUST implement these methods with these exact signatures:

```rust
pub type TxnId = u64;
pub type KeyId = u64;
pub type Value = u64;

impl SsiStore {{
    pub fn new() -> Self;
    pub fn begin(&self) -> TxnId;
    pub fn read(&self, txn: TxnId, key: KeyId) -> Option<Value>;
    pub fn write(&self, txn: TxnId, key: KeyId, value: Value) -> bool;
    pub fn commit(&self, txn: TxnId) -> bool;  // false = aborted due to dangerous structure
    pub fn abort(&self, txn: TxnId);
    pub fn is_active(&self, txn: TxnId) -> bool;
    pub fn committed_txns(&self) -> HashSet<TxnId>;
    pub fn get_current_value(&self, key: KeyId) -> Option<Value>;
    pub fn get_conflict_flags(&self, txn: TxnId) -> (bool, bool);  // (in_conflict, out_conflict)
}}
```

## CONSTRAINTS

1. Code must be SELF-CONTAINED - only `std` crate, no external dependencies
2. Must be thread-safe (`SsiStore` should be `Send + Sync`)
3. Use u64 for IDs and timestamps (not usize)
4. Include at least 2 assertions per public function

## FREEDOM

You decide:
- Internal data structures (how to track transactions, versions, locks)
- Implementation strategy (single lock, fine-grained locking, lock-free)
- How to detect and track dangerous structures

The evaluator cascade only cares about correctness - if your implementation satisfies the TLA+ invariants, any internal design is valid.

Generate the complete implementation. Do NOT include a tests module - tests will be added separately."#;

/// SSI fix prompt template.
const SSI_FIX_PROMPT: &str = r#"The following SSI implementation failed verification.

## REQUIRED INVARIANTS

{INVARIANTS}

## FAILED CODE

```rust
{PREVIOUS_CODE}
```

## VERIFICATION FAILURE

{ERROR_INFO}

## TASK

Fix the code to pass verification. Common SSI issues include:
- Setting conflict flags on committed transactions (they should be frozen)
- Not checking for dangerous structure at commit time
- Missing SIREAD lock persistence after commit
- Wrong visibility logic for snapshot reads
- Not releasing write locks on abort
- Incorrect conflict flag propagation during read/write

Analyze the error carefully and provide a corrected implementation.

Return ONLY the fixed Rust code in a ```rust code block."#;

/// Extract code from a markdown code block.
pub fn extract_code_block(response: &str) -> Option<String> {
    // Look for ```rust ... ``` block
    let rust_start = response.find("```rust")?;
    let code_start = rust_start + 7; // Skip "```rust"

    // Skip any whitespace/newline after ```rust
    let content_after = &response[code_start..];
    let actual_start = content_after
        .find(|c: char| !c.is_whitespace() || c == '\n')
        .map(|i| code_start + i)
        .unwrap_or(code_start);

    let code_end = response[actual_start..].find("```")?;

    Some(response[actual_start..actual_start + code_end].trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_code_block() {
        let response = r#"Here's the implementation:

```rust
fn main() {
    println!("Hello");
}
```

This code prints hello."#;

        let code = extract_code_block(response).unwrap();
        assert!(code.contains("fn main()"));
        assert!(code.contains("println!"));
    }

    #[test]
    fn test_extract_code_block_no_block() {
        let response = "No code here";
        assert!(extract_code_block(response).is_none());
    }

    #[test]
    fn test_prompt_builder() {
        let builder = PromptBuilder::new();
        assert!(!builder.system_prompt().is_empty());
    }

    #[test]
    fn test_build_generation_prompt() {
        let spec_content = r#"
---------------------------- MODULE test_stack ----------------------------
CONSTANTS Elements
VARIABLES head
=============================================================================
"#;
        let spec = vf_core::TlaSpec::parse(spec_content).unwrap();
        let builder = PromptBuilder::new();
        let prompt = builder.build_generation_prompt(&spec);

        assert!(prompt.contains("test_stack"));
        assert!(prompt.contains("Elements"));
    }

    #[test]
    fn test_targeted_fix_prompt() {
        let builder = PromptBuilder::new();
        let prompt = builder.build_targeted_fix_prompt(
            "miri",
            "undefined behavior detected",
            "fn foo() {}",
        );

        assert!(prompt.contains("miri"));
        assert!(prompt.contains("undefined behavior"));
        assert!(prompt.contains("fn foo()"));
    }
}
