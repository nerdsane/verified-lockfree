//! Serializable Snapshot Isolation (SSI) implementation.
//!
//! A correct implementation of SSI following Cahill's algorithm.
//! This is the algorithm behind PostgreSQL's SERIALIZABLE isolation level.
//!
//! # Key Concepts
//!
//! - **Snapshot**: Each transaction sees data from its start time
//! - **SIREAD locks**: Track reads (persist after commit for conflict detection)
//! - **Conflict flags**: `in_conflict` and `out_conflict` per transaction
//! - **Dangerous structure**: Transaction with BOTH flags = potential cycle
//!
//! # Invariants (from `specs/ssi/serializable_snapshot_isolation.tla`)
//!
//! 1. `FirstCommitterWins`: No concurrent commits to same key
//! 2. `SnapshotConsistency`: Reads see consistent snapshot
//! 3. `Serializable`: No dangerous structures at commit time
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  SsiStore                                                    │
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
//! │  │ Txn Manager │    │ Lock Manager│    │ Data Store  │     │
//! │  │ (snapshots) │    │ (conflicts) │    │ (versions)  │     │
//! │  └─────────────┘    └─────────────┘    └─────────────┘     │
//! └─────────────────────────────────────────────────────────────┘
//! ```

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;

use vf_dst::ssi_harness::{DstTestableSsi, KeyId, TxnId, Value};

/// Transaction status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxnStatus {
    Active,
    Committed,
    Aborted,
}

/// A versioned value with writer transaction ID.
#[derive(Debug, Clone)]
pub struct Version {
    pub value: Value,
    pub writer_txn: TxnId,
    pub commit_timestamp: u64,
}

/// Transaction state.
#[derive(Debug, Clone)]
pub struct TxnState {
    pub status: TxnStatus,
    /// Snapshot timestamp (start time).
    pub snapshot_ts: u64,
    /// Keys written by this transaction.
    pub write_set: BTreeSet<KeyId>,
    /// Keys read by this transaction.
    pub read_set: BTreeSet<KeyId>,
    /// Incoming rw-conflict flag.
    pub in_conflict: bool,
    /// Outgoing rw-conflict flag.
    pub out_conflict: bool,
}

impl TxnState {
    fn new(snapshot_ts: u64) -> Self {
        Self {
            status: TxnStatus::Active,
            snapshot_ts,
            write_set: BTreeSet::new(),
            read_set: BTreeSet::new(),
            in_conflict: false,
            out_conflict: false,
        }
    }

    /// Check if transaction has dangerous structure.
    fn has_dangerous_structure(&self) -> bool {
        self.in_conflict && self.out_conflict
    }
}

/// All SSI state in a single lock to avoid deadlocks.
///
/// This is simpler than fine-grained locking and sufficient for testing.
#[derive(Debug)]
struct SsiInner {
    /// Current timestamp.
    timestamp: u64,
    /// Next transaction ID.
    next_txn: TxnId,
    /// Transaction states.
    txns: HashMap<TxnId, TxnState>,
    /// Write locks: Key -> Option<TxnId holding lock>.
    write_locks: HashMap<KeyId, Option<TxnId>>,
    /// SIREAD locks: Key -> Set of TxnIds that read.
    siread_locks: HashMap<KeyId, BTreeSet<TxnId>>,
    /// Versioned data: Key -> List of versions (newest first).
    data: HashMap<KeyId, Vec<Version>>,
    /// Committed transaction IDs.
    committed: HashSet<TxnId>,
}

impl SsiInner {
    fn new() -> Self {
        Self {
            timestamp: 1,
            next_txn: 1,
            txns: HashMap::new(),
            write_locks: HashMap::new(),
            siread_locks: HashMap::new(),
            data: HashMap::new(),
            committed: HashSet::new(),
        }
    }

    fn tick(&mut self) -> u64 {
        let ts = self.timestamp;
        self.timestamp += 1;
        ts
    }

    /// Find the version visible at a snapshot timestamp.
    fn visible_version(&self, key: KeyId, snapshot_ts: u64) -> Option<Version> {
        if let Some(versions) = self.data.get(&key) {
            for ver in versions {
                // Version is visible if committed before or at snapshot time
                if ver.commit_timestamp <= snapshot_ts && ver.commit_timestamp != u64::MAX {
                    return Some(ver.clone());
                }
            }
        }
        None
    }

    /// Find concurrent readers of a key (for conflict detection during write).
    fn concurrent_readers(&self, txn: TxnId, key: KeyId) -> BTreeSet<TxnId> {
        let mut readers = BTreeSet::new();

        if let Some(holders) = self.siread_locks.get(&key) {
            for &holder in holders {
                if holder == txn {
                    continue;
                }

                if let Some(holder_state) = self.txns.get(&holder) {
                    match holder_state.status {
                        TxnStatus::Active | TxnStatus::Committed => {
                            readers.insert(holder);
                        }
                        TxnStatus::Aborted => {}
                    }
                }
            }
        }

        readers
    }

    /// Find writers that wrote after our snapshot (for conflict detection during read).
    fn newer_writers(&self, txn: TxnId, key: KeyId, snapshot_ts: u64) -> BTreeSet<TxnId> {
        let mut writers = BTreeSet::new();

        // Check committed versions
        if let Some(versions) = self.data.get(&key) {
            for ver in versions {
                if ver.writer_txn == txn {
                    continue;
                }
                // Uncommitted writes (timestamp = MAX) or writes after our snapshot
                if ver.commit_timestamp > snapshot_ts {
                    if let Some(writer_state) = self.txns.get(&ver.writer_txn) {
                        if writer_state.status == TxnStatus::Committed
                            || writer_state.status == TxnStatus::Active
                        {
                            writers.insert(ver.writer_txn);
                        }
                    }
                }
            }
        }

        writers
    }
}

/// SSI Store - the core data structure.
///
/// Thread-safe storage with SSI concurrency control.
pub struct SsiStore {
    inner: Mutex<SsiInner>,
}

impl SsiStore {
    /// Create a new SSI store.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(SsiInner::new()),
        }
    }
}

impl Default for SsiStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DstTestableSsi for SsiStore {
    fn begin(&self) -> TxnId {
        let mut inner = self.inner.lock().unwrap();

        let txn = inner.next_txn;
        inner.next_txn += 1;

        let snapshot_ts = inner.timestamp;
        inner.tick();

        inner.txns.insert(txn, TxnState::new(snapshot_ts));

        txn
    }

    fn read(&self, txn: TxnId, key: KeyId) -> Option<Value> {
        let mut inner = self.inner.lock().unwrap();

        // Check transaction is active
        let txn_state = match inner.txns.get(&txn) {
            Some(state) if state.status == TxnStatus::Active => state.clone(),
            _ => return None,
        };

        let snapshot_ts = txn_state.snapshot_ts;

        // Check for rw-conflicts with newer writers
        let newer_writers = inner.newer_writers(txn, key, snapshot_ts);

        // Update conflict flags
        for &writer in &newer_writers {
            if let Some(writer_state) = inner.txns.get_mut(&writer) {
                writer_state.in_conflict = true;
            }
        }

        if !newer_writers.is_empty() {
            if let Some(state) = inner.txns.get_mut(&txn) {
                state.out_conflict = true;
            }
        }

        // Record the read
        if let Some(state) = inner.txns.get_mut(&txn) {
            state.read_set.insert(key);
        }

        // Add SIREAD lock
        inner.siread_locks.entry(key).or_default().insert(txn);

        // Get visible version
        inner.visible_version(key, snapshot_ts).map(|v| v.value)
    }

    fn write(&self, txn: TxnId, key: KeyId, value: Value) -> bool {
        let mut inner = self.inner.lock().unwrap();

        // Check transaction is active
        match inner.txns.get(&txn) {
            Some(state) if state.status == TxnStatus::Active => {}
            _ => return false,
        };

        // Check write lock
        let lock_holder = inner.write_locks.get(&key).copied().flatten();
        if lock_holder.is_some() && lock_holder != Some(txn) {
            return false; // Lock held by another transaction
        }

        // Check for rw-conflicts with concurrent readers
        let concurrent_readers = inner.concurrent_readers(txn, key);

        // Update conflict flags
        // SSI rule: When T writes K:
        //   - For each ACTIVE T' with SIREAD on K: T'.out_conflict = true
        //   - If any T' (active OR committed) has SIREAD on K: T.in_conflict = true
        // Committed transactions are frozen - we don't modify their flags.
        for &reader in &concurrent_readers {
            if let Some(reader_state) = inner.txns.get_mut(&reader) {
                // Only set out_conflict on ACTIVE transactions (committed are frozen)
                if reader_state.status == TxnStatus::Active {
                    reader_state.out_conflict = true;
                }
            }
        }

        // Writer gets in_conflict if ANY reader (active or committed) exists
        // This is correct SSI: writing where someone else read creates rw-dependency
        if !concurrent_readers.is_empty() {
            if let Some(state) = inner.txns.get_mut(&txn) {
                state.in_conflict = true;
            }
        }

        // Acquire write lock
        inner.write_locks.insert(key, Some(txn));

        // Record the write
        if let Some(state) = inner.txns.get_mut(&txn) {
            state.write_set.insert(key);
        }

        // Store pending write (commit_timestamp = MAX until commit)
        let versions = inner.data.entry(key).or_default();
        // Remove any previous pending write from this txn
        versions.retain(|v| !(v.writer_txn == txn && v.commit_timestamp == u64::MAX));
        versions.insert(
            0,
            Version {
                value,
                writer_txn: txn,
                commit_timestamp: u64::MAX, // Pending
            },
        );

        true
    }

    fn commit(&self, txn: TxnId) -> bool {
        let mut inner = self.inner.lock().unwrap();

        // Check transaction is active
        let txn_state = match inner.txns.get(&txn) {
            Some(state) if state.status == TxnStatus::Active => state.clone(),
            _ => return false,
        };

        // Check for dangerous structure - CANNOT commit
        if txn_state.has_dangerous_structure() {
            // Abort instead
            if let Some(state) = inner.txns.get_mut(&txn) {
                state.status = TxnStatus::Aborted;
            }

            // Release write locks
            for &key in &txn_state.write_set {
                if inner.write_locks.get(&key) == Some(&Some(txn)) {
                    inner.write_locks.insert(key, None);
                }
            }

            // Remove pending writes
            for &key in &txn_state.write_set {
                if let Some(versions) = inner.data.get_mut(&key) {
                    versions.retain(|v| v.writer_txn != txn);
                }
            }

            return false;
        }

        // Commit the transaction
        let commit_ts = inner.tick();

        // Update pending writes with real commit timestamp
        for &key in &txn_state.write_set {
            if let Some(versions) = inner.data.get_mut(&key) {
                for ver in versions.iter_mut() {
                    if ver.writer_txn == txn && ver.commit_timestamp == u64::MAX {
                        ver.commit_timestamp = commit_ts;
                    }
                }
                // Sort by commit timestamp (newest first)
                versions.sort_by(|a, b| b.commit_timestamp.cmp(&a.commit_timestamp));
            }
        }

        // Release write locks
        for &key in &txn_state.write_set {
            if inner.write_locks.get(&key) == Some(&Some(txn)) {
                inner.write_locks.insert(key, None);
            }
        }

        // Mark as committed
        if let Some(state) = inner.txns.get_mut(&txn) {
            state.status = TxnStatus::Committed;
        }

        // Track committed transactions
        inner.committed.insert(txn);

        // SIREAD locks persist after commit (for conflict detection)

        true
    }

    fn abort(&self, txn: TxnId) {
        let mut inner = self.inner.lock().unwrap();

        // Check transaction is active
        let txn_state = match inner.txns.get(&txn) {
            Some(state) if state.status == TxnStatus::Active => state.clone(),
            _ => return,
        };

        // Mark as aborted
        if let Some(state) = inner.txns.get_mut(&txn) {
            state.status = TxnStatus::Aborted;
        }

        // Release write locks
        for &key in &txn_state.write_set {
            if inner.write_locks.get(&key) == Some(&Some(txn)) {
                inner.write_locks.insert(key, None);
            }
        }

        // Remove pending writes
        for &key in &txn_state.write_set {
            if let Some(versions) = inner.data.get_mut(&key) {
                versions.retain(|v| v.writer_txn != txn);
            }
        }

        // Remove SIREAD locks
        for locks in inner.siread_locks.values_mut() {
            locks.remove(&txn);
        }
    }

    fn is_active(&self, txn: TxnId) -> bool {
        self.inner
            .lock()
            .unwrap()
            .txns
            .get(&txn)
            .map(|s| s.status == TxnStatus::Active)
            .unwrap_or(false)
    }

    fn committed_txns(&self) -> HashSet<TxnId> {
        self.inner.lock().unwrap().committed.clone()
    }

    fn get_current_value(&self, key: KeyId) -> Option<Value> {
        let inner = self.inner.lock().unwrap();
        inner.visible_version(key, inner.timestamp).map(|v| v.value)
    }

    fn get_conflict_flags(&self, txn: TxnId) -> (bool, bool) {
        let inner = self.inner.lock().unwrap();
        inner
            .txns
            .get(&txn)
            .map(|state| (state.in_conflict, state.out_conflict))
            .unwrap_or((false, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_transaction() {
        let store = SsiStore::new();

        // T1: write and commit
        let t1 = store.begin();
        assert!(store.write(t1, 1, 100));
        assert!(store.commit(t1));

        // T2: read committed value
        let t2 = store.begin();
        assert_eq!(store.read(t2, 1), Some(100));
        assert!(store.commit(t2));
    }

    #[test]
    fn test_snapshot_isolation() {
        let store = SsiStore::new();

        // T1: write initial value
        let t1 = store.begin();
        assert!(store.write(t1, 1, 100));
        assert!(store.commit(t1));

        // T2: start and read
        let t2 = store.begin();
        assert_eq!(store.read(t2, 1), Some(100));

        // T3: update value and commit
        let t3 = store.begin();
        assert!(store.write(t3, 1, 200));
        assert!(store.commit(t3));

        // T2 should still see old value (snapshot isolation)
        assert_eq!(store.read(t2, 1), Some(100));
        assert!(store.commit(t2));
    }

    #[test]
    fn test_write_lock_blocking() {
        let store = SsiStore::new();

        // T1: write K1 (holds lock)
        let t1 = store.begin();
        assert!(store.write(t1, 1, 100));

        // T2: cannot write K1 (lock held)
        let t2 = store.begin();
        assert!(!store.write(t2, 1, 200)); // Should fail

        // T2: can write different key
        assert!(store.write(t2, 2, 300));

        // T1 commits, releases lock
        assert!(store.commit(t1));

        // T2 can now write K1 (but we already wrote K2)
        assert!(store.commit(t2));
    }

    #[test]
    fn test_dangerous_structure_abort() {
        let store = SsiStore::new();

        // Setup: Write initial values
        let setup = store.begin();
        store.write(setup, 1, 10);
        store.write(setup, 2, 20);
        store.commit(setup);

        // T1 and T2: create dangerous structure (write skew pattern)
        let t1 = store.begin();
        let t2 = store.begin();

        // T1 reads K1
        store.read(t1, 1);

        // T2 reads K2
        store.read(t2, 2);

        // T2 writes K1 (conflict with T1's read) - T1 gets out_conflict
        store.write(t2, 1, 11);
        store.commit(t2);

        // T1 writes K2 (conflict with T2's read) - T1 gets in_conflict
        // Now T1 has BOTH flags = dangerous structure
        store.write(t1, 2, 21);

        // T1's commit should fail due to dangerous structure
        let committed = store.commit(t1);
        assert!(!committed, "T1 should abort due to dangerous structure");
    }

    #[test]
    fn test_disjoint_keys_commit() {
        let store = SsiStore::new();

        // T1 and T2: write different keys concurrently
        let t1 = store.begin();
        let t2 = store.begin();

        assert!(store.write(t1, 1, 100));
        assert!(store.write(t2, 2, 200));

        // Both should commit (no conflicts)
        assert!(store.commit(t1));
        assert!(store.commit(t2));

        // Verify values
        let t3 = store.begin();
        assert_eq!(store.read(t3, 1), Some(100));
        assert_eq!(store.read(t3, 2), Some(200));
    }

    #[test]
    fn test_single_conflict_flag_commits() {
        let store = SsiStore::new();

        // Setup
        let setup = store.begin();
        store.write(setup, 1, 10);
        store.commit(setup);

        // T1 reads K1
        let t1 = store.begin();
        store.read(t1, 1);

        // T2 writes K1 (T1 gets out_conflict, but not in_conflict)
        let t2 = store.begin();
        store.write(t2, 1, 20);
        store.commit(t2);

        // T1 should still be able to commit (only out_conflict, no in_conflict)
        assert!(store.commit(t1), "T1 should commit with only out_conflict");
    }
}
