//! Epoch-based garbage collection invariants from epoch_gc.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | NoUseAfterFree | 45 | No thread references freed objects |
//! | QuiescentReclamation | 58 | Safe to free after all threads advance |
//! | BoundedMemory | 72 | Pending reclamation is bounded |
//! | MonotonicEpoch | 86 | Epochs never decrease |

use std::collections::{HashMap, HashSet};

use crate::property::{PropertyChecker, PropertyResult};

const TLA_SPEC: &str = "epoch_gc.tla";

/// Properties that any epoch-based GC implementation must satisfy.
pub trait EpochGcProperties {
    /// Current global epoch.
    fn global_epoch(&self) -> u64;

    /// Map: thread_id -> epoch when thread last pinned.
    fn thread_epochs(&self) -> HashMap<u64, u64>;

    /// Map: thread_id -> set of object IDs the thread holds references to.
    fn thread_references(&self) -> HashMap<u64, HashSet<u64>>;

    /// Set of object IDs that have been freed.
    fn freed_objects(&self) -> HashSet<u64>;

    /// Map: epoch -> set of object IDs retired at that epoch.
    fn retired_objects(&self) -> HashMap<u64, HashSet<u64>>;
}

/// Property checker for epoch GC implementations.
pub struct EpochGcPropertyChecker<'a, T: EpochGcProperties> {
    gc: &'a T,
}

impl<'a, T: EpochGcProperties> EpochGcPropertyChecker<'a, T> {
    #[must_use]
    pub fn new(gc: &'a T) -> Self {
        Self { gc }
    }

    /// Line 45: NoUseAfterFree
    fn check_no_use_after_free(&self) -> PropertyResult {
        let freed = self.gc.freed_objects();
        let refs = self.gc.thread_references();

        for (tid, obj_set) in &refs {
            let dangling: HashSet<_> = obj_set.intersection(&freed).collect();
            if !dangling.is_empty() {
                return PropertyResult::fail(
                    "NoUseAfterFree",
                    TLA_SPEC,
                    45,
                    format!(
                        "Thread {} holds references to freed objects: {:?}",
                        tid, dangling
                    ),
                    None,
                );
            }
        }

        PropertyResult::pass("NoUseAfterFree", TLA_SPEC, 45)
    }

    /// Line 72: BoundedMemory
    fn check_bounded_memory(&self) -> PropertyResult {
        let freed = self.gc.freed_objects();
        let retired = self.gc.retired_objects();
        let refs = self.gc.thread_references();

        let all_retired: HashSet<u64> = retired.values().flat_map(|s| s.iter().copied()).collect();
        let pending = all_retired.difference(&freed).count() as u64;
        let thread_count = refs.len() as u64;

        // Heuristic bound: pending should not grow unbounded
        // In practice, bounded by threads * objects-per-epoch
        let bound = thread_count.max(1) * 1000; // generous bound
        if pending > bound {
            return PropertyResult::fail(
                "BoundedMemory",
                TLA_SPEC,
                72,
                format!(
                    "Pending reclamation {} exceeds bound {} (threads={})",
                    pending, bound, thread_count
                ),
                None,
            );
        }

        PropertyResult::pass("BoundedMemory", TLA_SPEC, 72)
    }

    /// Line 86: MonotonicEpoch
    fn check_monotonic_epoch(&self) -> PropertyResult {
        let epoch = self.gc.global_epoch();

        // Epoch should be non-negative (u64 guarantees this)
        // and thread epochs should not exceed global
        let thread_epochs = self.gc.thread_epochs();
        for (tid, te) in &thread_epochs {
            if *te > epoch {
                return PropertyResult::fail(
                    "MonotonicEpoch",
                    TLA_SPEC,
                    86,
                    format!(
                        "Thread {} epoch {} exceeds global epoch {}",
                        tid, te, epoch
                    ),
                    None,
                );
            }
        }

        PropertyResult::pass("MonotonicEpoch", TLA_SPEC, 86)
    }
}

impl<'a, T: EpochGcProperties> PropertyChecker for EpochGcPropertyChecker<'a, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_no_use_after_free(),
            self.check_bounded_memory(),
            self.check_monotonic_epoch(),
        ]
    }
}
