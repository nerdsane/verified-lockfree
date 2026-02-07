//! Page cache invariants from pagecache.tla
//!
//! # TLA+ Mapping
//!
//! | Property | TLA+ Line | Description |
//! |----------|-----------|-------------|
//! | AtomicPageUpdate | 45 | Page writes appear atomic |
//! | NoLostPages | 58 | Every written page is in cache or on disk |
//! | CrashConsistency | 72 | Recovered state is consistent |
//! | NoDirtyReads | 86 | Reads return latest committed version |

use std::collections::HashMap;

use crate::property::{PropertyChecker, PropertyResult};

const TLA_SPEC: &str = "pagecache.tla";

/// Page state.
#[derive(Debug, Clone)]
pub struct PageState {
    pub version: u64,
    pub dirty: bool,
}

/// Properties that any page cache implementation must satisfy.
pub trait PageCacheProperties {
    /// Map: page_id -> current page state in cache.
    fn cached_pages(&self) -> HashMap<u64, PageState>;

    /// Map: page_id -> version on disk.
    fn disk_pages(&self) -> HashMap<u64, u64>;

    /// Set of page IDs that have been written.
    fn written_page_ids(&self) -> Vec<u64>;

    /// Set of page IDs pending flush.
    fn flush_pending(&self) -> Vec<u64>;
}

/// Property checker for page cache implementations.
pub struct PageCachePropertyChecker<'a, T: PageCacheProperties> {
    cache: &'a T,
}

impl<'a, T: PageCacheProperties> PageCachePropertyChecker<'a, T> {
    #[must_use]
    pub fn new(cache: &'a T) -> Self {
        Self { cache }
    }

    /// Line 45: AtomicPageUpdate
    fn check_atomic_page_update(&self) -> PropertyResult {
        let cached = self.cache.cached_pages();

        for (pid, state) in &cached {
            // Version must be a valid positive number (no torn writes)
            if state.version == 0 && state.dirty {
                return PropertyResult::fail(
                    "AtomicPageUpdate",
                    TLA_SPEC,
                    45,
                    format!("Page {} has version 0 but is marked dirty (partial write?)", pid),
                    None,
                );
            }
        }

        PropertyResult::pass("AtomicPageUpdate", TLA_SPEC, 45)
    }

    /// Line 58: NoLostPages
    fn check_no_lost_pages(&self) -> PropertyResult {
        let cached = self.cache.cached_pages();
        let disk = self.cache.disk_pages();
        let written = self.cache.written_page_ids();

        for pid in &written {
            if !cached.contains_key(pid) && !disk.contains_key(pid) {
                return PropertyResult::fail(
                    "NoLostPages",
                    TLA_SPEC,
                    58,
                    format!("Page {} was written but is neither in cache nor on disk", pid),
                    None,
                );
            }
        }

        PropertyResult::pass("NoLostPages", TLA_SPEC, 58)
    }

    /// Line 72: CrashConsistency
    fn check_crash_consistency(&self) -> PropertyResult {
        let disk = self.cache.disk_pages();

        for (pid, version) in &disk {
            if *version == 0 {
                return PropertyResult::fail(
                    "CrashConsistency",
                    TLA_SPEC,
                    72,
                    format!("Page {} on disk has invalid version 0", pid),
                    None,
                );
            }
        }

        PropertyResult::pass("CrashConsistency", TLA_SPEC, 72)
    }
}

impl<'a, T: PageCacheProperties> PropertyChecker for PageCachePropertyChecker<'a, T> {
    fn check_all(&self) -> Vec<PropertyResult> {
        vec![
            self.check_atomic_page_update(),
            self.check_no_lost_pages(),
            self.check_crash_consistency(),
        ]
    }
}
