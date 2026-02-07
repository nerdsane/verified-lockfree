//! Radix tree invariants from radix_tree.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | PrefixConsistency | 45 | Traversal reaches correct value |
//! | NoLostKeys | 58 | Every stored key is retrievable |
//! | AtomicUpdate | 72 | Updates appear atomic |

use std::collections::HashMap;

use crate::property::{PropertyChecker, PropertyResult};

const TLA_SPEC: &str = "radix_tree.tla";

/// Properties that any concurrent radix tree must satisfy.
pub trait RadixTreeProperties {
    /// The logical key-value map (ground truth).
    fn logical_map(&self) -> HashMap<Vec<u8>, u64>;

    /// Look up a key via tree traversal.
    fn lookup(&self, key: &[u8]) -> Option<u64>;

    /// All keys retrievable via traversal.
    fn all_retrievable_keys(&self) -> Vec<Vec<u8>>;
}

/// Property checker for radix tree implementations.
pub struct RadixTreePropertyChecker<'a, T: RadixTreeProperties> {
    tree: &'a T,
}

impl<'a, T: RadixTreeProperties> RadixTreePropertyChecker<'a, T> {
    #[must_use]
    pub fn new(tree: &'a T) -> Self {
        Self { tree }
    }

    /// Line 45: PrefixConsistency
    fn check_prefix_consistency(&self) -> PropertyResult {
        let map = self.tree.logical_map();

        for (key, expected_val) in &map {
            match self.tree.lookup(key) {
                Some(val) if val == *expected_val => {}
                Some(val) => {
                    return PropertyResult::fail(
                        "PrefixConsistency",
                        TLA_SPEC,
                        45,
                        format!(
                            "Key {:?} returned {} but expected {}",
                            key, val, expected_val
                        ),
                        None,
                    );
                }
                None => {
                    return PropertyResult::fail(
                        "PrefixConsistency",
                        TLA_SPEC,
                        45,
                        format!("Key {:?} not found via traversal but exists in map", key),
                        None,
                    );
                }
            }
        }

        PropertyResult::pass("PrefixConsistency", TLA_SPEC, 45)
    }

    /// Line 58: NoLostKeys
    fn check_no_lost_keys(&self) -> PropertyResult {
        let map = self.tree.logical_map();
        let retrievable: std::collections::HashSet<Vec<u8>> =
            self.tree.all_retrievable_keys().into_iter().collect();

        for key in map.keys() {
            if !retrievable.contains(key) {
                return PropertyResult::fail(
                    "NoLostKeys",
                    TLA_SPEC,
                    58,
                    format!("Key {:?} exists in map but is not retrievable", key),
                    None,
                );
            }
        }

        PropertyResult::pass("NoLostKeys", TLA_SPEC, 58)
    }
}

impl<'a, T: RadixTreeProperties> PropertyChecker for RadixTreePropertyChecker<'a, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_prefix_consistency(),
            self.check_no_lost_keys(),
        ]
    }
}
