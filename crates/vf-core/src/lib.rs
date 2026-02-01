//! # vf-core
//!
//! Core types and invariants for verified lock-free structures.
//!
//! This crate provides:
//! - `PropertyResult` and `PropertyChecker` for verifying invariants
//! - `Counterexample` for rendering failure paths
//! - Invariant traits for each data structure (e.g., `StackProperties`)
//!
//! ## TLA+ Traceability
//!
//! Every property is traceable to a specific line in the TLA+ spec.
//! This ensures the Rust verification matches the formal model.

pub mod counterexample;
pub mod invariants;
pub mod property;
pub mod tla_spec;

pub use counterexample::{Counterexample, MemoryIssue, StateSnapshot, ThreadAction};
pub use property::{PropertyChecker, PropertyResult};
pub use tla_spec::{ParseError, TlaInvariant, TlaSpec};
