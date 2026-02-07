//! B+ tree invariants from btree_plus.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | SortedOrder | 45 | Keys in every node are sorted |
//! | BalancedHeight | 58 | All leaves at same depth |
//! | AtomicSplit | 72 | Splits appear atomic to readers |
//! | NoLostKeys | 86 | Every key findable via traversal |

use std::collections::HashSet;

use crate::property::{PropertyChecker, PropertyResult};

const TLA_SPEC: &str = "btree_plus.tla";

/// Properties that any concurrent B+ tree must satisfy.
pub trait BTreePlusProperties {
    /// The logical key-value map (ground truth).
    fn logical_keys(&self) -> HashSet<u64>;

    /// All keys found via tree traversal (leaf scan).
    fn leaf_scan_keys(&self) -> Vec<u64>;

    /// Whether all leaves are at the same depth.
    fn is_balanced(&self) -> bool;

    /// Current tree height.
    fn height(&self) -> u64;
}

/// Property checker for B+ tree implementations.
pub struct BTreePlusPropertyChecker<'a, T: BTreePlusProperties> {
    tree: &'a T,
}

impl<'a, T: BTreePlusProperties> BTreePlusPropertyChecker<'a, T> {
    #[must_use]
    pub fn new(tree: &'a T) -> Self {
        Self { tree }
    }

    /// Line 45: SortedOrder
    fn check_sorted_order(&self) -> PropertyResult {
        let keys = self.tree.leaf_scan_keys();

        for i in 1..keys.len() {
            if keys[i - 1] >= keys[i] {
                return PropertyResult::fail(
                    "SortedOrder",
                    TLA_SPEC,
                    45,
                    format!(
                        "Keys not sorted: {} >= {} at indices {}, {}",
                        keys[i - 1],
                        keys[i],
                        i - 1,
                        i
                    ),
                    None,
                );
            }
        }

        PropertyResult::pass("SortedOrder", TLA_SPEC, 45)
    }

    /// Line 58: BalancedHeight
    fn check_balanced_height(&self) -> PropertyResult {
        if !self.tree.is_balanced() {
            return PropertyResult::fail(
                "BalancedHeight",
                TLA_SPEC,
                58,
                format!("Tree is not balanced (height={})", self.tree.height()),
                None,
            );
        }

        PropertyResult::pass("BalancedHeight", TLA_SPEC, 58)
    }

    /// Line 86: NoLostKeys
    fn check_no_lost_keys(&self) -> PropertyResult {
        let logical = self.tree.logical_keys();
        let leaf_keys: HashSet<u64> = self.tree.leaf_scan_keys().into_iter().collect();

        for key in &logical {
            if !leaf_keys.contains(key) {
                return PropertyResult::fail(
                    "NoLostKeys",
                    TLA_SPEC,
                    86,
                    format!("Key {} exists logically but not found in leaf scan", key),
                    None,
                );
            }
        }

        PropertyResult::pass("NoLostKeys", TLA_SPEC, 86)
    }
}

impl<'a, T: BTreePlusProperties> PropertyChecker for BTreePlusPropertyChecker<'a, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_sorted_order(),
            self.check_balanced_height(),
            self.check_no_lost_keys(),
        ]
    }
}
