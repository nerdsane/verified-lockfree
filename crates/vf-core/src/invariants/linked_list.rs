//! Linked list invariants from linked_list.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | NoLostElements | 45 | Every inserted-not-removed key is reachable |
//! | NoDuplicates | 58 | No key appears twice in list |
//! | Reachability | 72 | All live nodes reachable from head |
//! | InsertOrderPreserved | 86 | Keys in sorted order |

use std::collections::HashSet;

use crate::counterexample::Counterexample;
use crate::property::{PropertyChecker, PropertyResult};

const TLA_SPEC: &str = "linked_list.tla";

/// Properties that any concurrent linked list must satisfy.
pub trait LinkedListProperties {
    /// Set of all keys that have been inserted.
    fn inserted_keys(&self) -> HashSet<u64>;

    /// Set of all keys that have been removed.
    fn removed_keys(&self) -> HashSet<u64>;

    /// Current keys reachable from head (in traversal order).
    fn reachable_keys(&self) -> Vec<u64>;
}

/// Property checker for linked list implementations.
pub struct LinkedListPropertyChecker<'a, T: LinkedListProperties> {
    list: &'a T,
    dst_seed: Option<u64>,
}

impl<'a, T: LinkedListProperties> LinkedListPropertyChecker<'a, T> {
    #[must_use]
    pub fn new(list: &'a T) -> Self {
        Self {
            list,
            dst_seed: None,
        }
    }

    #[must_use]
    pub fn with_seed(mut self, seed: u64) -> Self {
        debug_assert!(seed != 0, "DST seed should not be zero");
        self.dst_seed = Some(seed);
        self
    }

    /// Line 45: NoLostElements
    fn check_no_lost_elements(&self) -> PropertyResult {
        let inserted = self.list.inserted_keys();
        let removed = self.list.removed_keys();
        let reachable: HashSet<u64> = self.list.reachable_keys().into_iter().collect();

        let live = inserted.difference(&removed).copied().collect::<HashSet<_>>();

        for key in &live {
            if !reachable.contains(key) {
                let mut ce = match self.dst_seed {
                    Some(seed) => Counterexample::with_seed(seed),
                    None => Counterexample::new(),
                };
                ce.add_state(crate::counterexample::StateSnapshot {
                    step: 1,
                    description: format!("Key {} lost", key),
                    variables: vec![
                        ("inserted".to_string(), format!("{:?}", inserted)),
                        ("removed".to_string(), format!("{:?}", removed)),
                        ("reachable".to_string(), format!("{:?}", reachable)),
                    ],
                });
                return PropertyResult::fail(
                    "NoLostElements",
                    TLA_SPEC,
                    45,
                    format!("Key {} was inserted but is not reachable from head", key),
                    Some(ce),
                );
            }
        }

        PropertyResult::pass("NoLostElements", TLA_SPEC, 45)
    }

    /// Line 58: NoDuplicates
    fn check_no_duplicates(&self) -> PropertyResult {
        let reachable = self.list.reachable_keys();
        let unique: HashSet<u64> = reachable.iter().copied().collect();

        if reachable.len() != unique.len() {
            let mut seen = HashSet::new();
            for key in &reachable {
                if !seen.insert(*key) {
                    return PropertyResult::fail(
                        "NoDuplicates",
                        TLA_SPEC,
                        58,
                        format!("Key {} appears multiple times in list", key),
                        None,
                    );
                }
            }
        }

        PropertyResult::pass("NoDuplicates", TLA_SPEC, 58)
    }

    /// Line 86: InsertOrderPreserved (sorted order)
    fn check_insert_order_preserved(&self) -> PropertyResult {
        let reachable = self.list.reachable_keys();

        for i in 1..reachable.len() {
            if reachable[i - 1] >= reachable[i] {
                return PropertyResult::fail(
                    "InsertOrderPreserved",
                    TLA_SPEC,
                    86,
                    format!(
                        "Keys not sorted: {} >= {} at indices {}, {}",
                        reachable[i - 1],
                        reachable[i],
                        i - 1,
                        i
                    ),
                    None,
                );
            }
        }

        PropertyResult::pass("InsertOrderPreserved", TLA_SPEC, 86)
    }
}

impl<'a, T: LinkedListProperties> PropertyChecker for LinkedListPropertyChecker<'a, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_no_lost_elements(),
            self.check_no_duplicates(),
            self.check_insert_order_preserved(),
        ]
    }
}
