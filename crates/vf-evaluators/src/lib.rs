//! # vf-evaluators
//!
//! Evaluator cascade for verified lock-free structures.
//!
//! The cascade runs evaluators in order of speed (fastest first):
//!
//! | Level | Tool | Time | Catches |
//! |-------|------|------|---------|
//! | 0 | rustc | instant | Type errors, lifetime issues |
//! | 1 | miri | seconds | Undefined behavior, aliasing |
//! | 2 | loom | seconds | Race conditions, memory ordering |
//! | 3 | DST | seconds | Faults, crashes, delays |
//! | 4 | stateright | seconds | Invariant violations |
//! | 5 | kani | minutes | Bounded proofs |
//! | 6 | verus | minutes | SMT-based theorem proofs |
//!
//! The cascade stops at the first failure, providing a counterexample.

pub mod adrs;
pub mod cascade;
pub mod level0_rustc;
pub mod level1_miri;
pub mod level2_loom;
pub mod level3_dst;
pub mod level4_stateright;
pub mod level5_kani;
pub mod level6_verus;
pub mod result;

pub use cascade::{CascadeConfig, EvaluatorCascade, EvaluatorLevel};
pub use level3_dst::{DstConfig, InlineResult as DstInlineResult};
pub use level4_stateright::{InlineResult as StaterightInlineResult, StaterightConfig};
pub use level5_kani::{KaniConfig, PROOF_HARNESS_TEMPLATE};
pub use level6_verus::{VerusConfig, generate_stack_proof_template as verus_stack_proof_template};
pub use result::{CascadeResult, EvaluatorResult};
