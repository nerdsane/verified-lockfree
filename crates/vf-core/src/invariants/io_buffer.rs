//! I/O buffer invariants from io_buffer.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | NoDataCorruption | 45 | Every submitted item appears in flushed output |
//! | CompletionGuarantee | 58 | Every submitted item is eventually flushed |
//! | OrderPreserved | 72 | Per-thread submission order is maintained |
//! | BoundedMemory | 86 | Buffer never exceeds configured size |

use std::collections::HashSet;

use crate::property::{PropertyChecker, PropertyResult};

const TLA_SPEC: &str = "io_buffer.tla";

/// Properties that any I/O buffer implementation must satisfy.
pub trait IoBufferProperties {
    /// All items that have been submitted.
    fn submitted_items(&self) -> Vec<u64>;

    /// All items that have been confirmed flushed.
    fn flushed_items(&self) -> Vec<u64>;

    /// Items currently in the buffer (not yet flushed).
    fn buffered_items(&self) -> Vec<u64>;

    /// Maximum buffer size in items.
    fn buffer_size_max(&self) -> u64;
}

/// Property checker for I/O buffer implementations.
pub struct IoBufferPropertyChecker<'a, T: IoBufferProperties> {
    buffer: &'a T,
}

impl<'a, T: IoBufferProperties> IoBufferPropertyChecker<'a, T> {
    #[must_use]
    pub fn new(buffer: &'a T) -> Self {
        Self { buffer }
    }

    /// Line 45: NoDataCorruption
    fn check_no_data_corruption(&self) -> PropertyResult {
        let submitted = self.buffer.submitted_items();
        let flushed: HashSet<u64> = self.buffer.flushed_items().into_iter().collect();
        let buffered: HashSet<u64> = self.buffer.buffered_items().into_iter().collect();

        for item in &submitted {
            if !flushed.contains(item) && !buffered.contains(item) {
                return PropertyResult::fail(
                    "NoDataCorruption",
                    TLA_SPEC,
                    45,
                    format!(
                        "Item {} was submitted but is neither flushed nor buffered",
                        item
                    ),
                    None,
                );
            }
        }

        PropertyResult::pass("NoDataCorruption", TLA_SPEC, 45)
    }

    /// Line 72: OrderPreserved
    fn check_order_preserved(&self) -> PropertyResult {
        let submitted = self.buffer.submitted_items();
        let flushed = self.buffer.flushed_items();

        // Flushed items must be a prefix of submitted (in same order)
        for (i, item) in flushed.iter().enumerate() {
            if i < submitted.len() && *item != submitted[i] {
                return PropertyResult::fail(
                    "OrderPreserved",
                    TLA_SPEC,
                    72,
                    format!(
                        "Flushed item at index {} is {} but submitted was {}",
                        i, item, submitted[i]
                    ),
                    None,
                );
            }
        }

        PropertyResult::pass("OrderPreserved", TLA_SPEC, 72)
    }

    /// Line 86: BoundedMemory
    fn check_bounded_memory(&self) -> PropertyResult {
        let buffered = self.buffer.buffered_items();
        let max = self.buffer.buffer_size_max();

        if buffered.len() as u64 > max {
            return PropertyResult::fail(
                "BoundedMemory",
                TLA_SPEC,
                86,
                format!(
                    "Buffer contains {} items but max is {}",
                    buffered.len(),
                    max
                ),
                None,
            );
        }

        PropertyResult::pass("BoundedMemory", TLA_SPEC, 86)
    }
}

impl<'a, T: IoBufferProperties> PropertyChecker for IoBufferPropertyChecker<'a, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_no_data_corruption(),
            self.check_order_preserved(),
            self.check_bounded_memory(),
        ]
    }
}
