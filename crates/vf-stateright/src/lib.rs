//! # vf-stateright
//!
//! Stateright models that mirror TLA+ specifications.
//!
//! Each model implements the same invariants as the corresponding TLA+ spec,
//! allowing exhaustive model checking in Rust.
//!
//! ## Usage
//!
//! 1. Model checking (verify the spec itself):
//!    ```ignore
//!    cargo test -p vf-stateright
//!    ```
//!
//! 2. Implementation verification (verify code against spec):
//!    ```ignore
//!    use vf_stateright::verifier::{VerifiableStack, verify_implementation};
//!    let result = verify_implementation::<MyStack>(&config);
//!    assert!(result.passed);
//!    ```
//!
//! 3. Oracle extraction for DST:
//!    ```ignore
//!    use vf_stateright::oracle::{OracleExtractor, OracleCategory};
//!    let mut extractor = OracleExtractor::new();
//!    let oracles = extractor.extract(2, vec![1, 2, 3]);
//!    // Use oracles in DST for targeted testing
//!    ```

pub mod oracle;
pub mod treiber_stack;
pub mod verifier;

pub use oracle::{Oracle, OracleAction, OracleActionType, OracleCategory, OracleExtractor};
pub use treiber_stack::{StackAction, StackModel, StackState};
pub use verifier::{VerifiableStack, VerificationResult, VerifierConfig, verify_implementation};
