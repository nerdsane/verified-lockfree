//! Invariant traits for verified concurrent structures.
//!
//! Each module defines the properties that implementations must satisfy,
//! all traceable to TLA+ specifications.
//!
//! ## Lock-free structures
//! - `stack`: Treiber Stack invariants (NoLostElements, NoDuplicates, LIFO)
//!
//! ## Lock-based protocols
//! - `ssi`: Serializable Snapshot Isolation (FirstCommitterWins, Serializable)

pub mod ssi;
pub mod stack;

pub use ssi::{
    check_all as check_all_ssi, first_committer_wins, is_serializable,
    no_committed_dangerous_structures, no_lost_writes, InvariantResult, SsiHistory,
};
pub use stack::{StackHistory, StackOperation, StackProperties, StackPropertyChecker};
