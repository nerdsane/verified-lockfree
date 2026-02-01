//! # vf-examples
//!
//! Reference implementations of verified lock-free structures.
//!
//! Each implementation:
//! - Maps to a TLA+ spec in `specs/lockfree/`
//! - Implements the corresponding Properties trait from `vf-core`
//! - Has DST tests that verify invariants
//! - Has loom tests for thread interleavings (under `#[cfg(loom)]`)
//! - Has Kani proofs for bounded verification (under `#[cfg(kani)]`)
//!
//! # Modules
//!
//! - `treiber_stack`: Correct reference implementation with epoch-based GC
//! - `loom_stack`: Loom-compatible stack for concurrency testing
//! - `buggy_stacks`: Intentionally buggy implementations for testing the cascade
//! - `kani_proofs`: Kani bounded model checking proofs

pub mod buggy_stacks;
pub mod kani_proofs;
pub mod loom_stack;
pub mod treiber_stack;

pub use buggy_stacks::{LostElementStack, MissingRetryStack, WrongOrderingStack};
pub use loom_stack::LoomStack;
pub use treiber_stack::TreiberStack;
