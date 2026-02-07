//! Ring buffer invariants from ring_buffer.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | NoLostMessages | 45 | Every produced message is in buffer or consumed |
//! | FIFO_Order | 58 | Messages consumed in production order |
//! | BoundedCapacity | 72 | Buffer never exceeds capacity |
//! | ProducerConsumerProgress | 86 | Non-blocking progress |

use std::collections::HashSet;

use crate::counterexample::Counterexample;
use crate::property::{PropertyChecker, PropertyResult};

const TLA_SPEC: &str = "ring_buffer.tla";

/// Properties that any ring buffer implementation must satisfy.
pub trait RingBufferProperties {
    /// All messages that have been produced (in order).
    fn produced_messages(&self) -> Vec<u64>;

    /// All messages that have been consumed (in order).
    fn consumed_messages(&self) -> Vec<u64>;

    /// Current messages in the buffer (head to tail order).
    fn current_contents(&self) -> Vec<u64>;

    /// Maximum capacity of the buffer.
    fn capacity(&self) -> u64;
}

/// Property checker for ring buffer implementations.
pub struct RingBufferPropertyChecker<'a, T: RingBufferProperties> {
    buffer: &'a T,
    dst_seed: Option<u64>,
}

impl<'a, T: RingBufferProperties> RingBufferPropertyChecker<'a, T> {
    #[must_use]
    pub fn new(buffer: &'a T) -> Self {
        Self {
            buffer,
            dst_seed: None,
        }
    }

    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        debug_assert!(seed != 0, "DST seed should not be zero");
        self.dst_seed = Some(seed);
        self
    }

    /// Line 45: NoLostMessages
    fn check_no_lost_messages(&self) -> PropertyResult {
        let produced = self.buffer.produced_messages();
        let consumed: HashSet<u64> = self.buffer.consumed_messages().into_iter().collect();
        let contents: HashSet<u64> = self.buffer.current_contents().into_iter().collect();

        for msg in &produced {
            if !consumed.contains(msg) && !contents.contains(msg) {
                let mut ce = match self.dst_seed {
                    Some(seed) => Counterexample::with_seed(seed),
                    None => Counterexample::new(),
                };
                ce.add_state(crate::counterexample::StateSnapshot {
                    step: 1,
                    description: format!("Message {} lost", msg),
                    variables: vec![
                        ("produced".to_string(), format!("{:?}", produced)),
                        ("consumed".to_string(), format!("{:?}", consumed)),
                        ("contents".to_string(), format!("{:?}", contents)),
                    ],
                });
                return PropertyResult::fail(
                    "NoLostMessages",
                    TLA_SPEC,
                    45,
                    format!("Message {} was produced but is neither in buffer nor consumed", msg),
                    Some(ce),
                );
            }
        }

        PropertyResult::pass("NoLostMessages", TLA_SPEC, 45)
    }

    /// Line 58: FIFO_Order
    fn check_fifo_order(&self) -> PropertyResult {
        let produced = self.buffer.produced_messages();
        let consumed = self.buffer.consumed_messages();

        for (i, msg) in consumed.iter().enumerate() {
            if i < produced.len() && *msg != produced[i] {
                return PropertyResult::fail(
                    "FIFO_Order",
                    TLA_SPEC,
                    58,
                    format!(
                        "Consumed message at index {} is {} but produced was {}",
                        i, msg, produced[i]
                    ),
                    None,
                );
            }
        }

        PropertyResult::pass("FIFO_Order", TLA_SPEC, 58)
    }

    /// Line 72: BoundedCapacity
    fn check_bounded_capacity(&self) -> PropertyResult {
        let contents = self.buffer.current_contents();
        let capacity = self.buffer.capacity();

        if contents.len() as u64 > capacity {
            return PropertyResult::fail(
                "BoundedCapacity",
                TLA_SPEC,
                72,
                format!(
                    "Buffer contains {} items but capacity is {}",
                    contents.len(),
                    capacity
                ),
                None,
            );
        }

        PropertyResult::pass("BoundedCapacity", TLA_SPEC, 72)
    }
}

impl<'a, T: RingBufferProperties> PropertyChecker for RingBufferPropertyChecker<'a, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_no_lost_messages(),
            self.check_fifo_order(),
            self.check_bounded_capacity(),
        ]
    }
}
