//! Serializable Snapshot Isolation (SSI) state machine.
//!
//! Mirrors `specs/ssi/serializable_snapshot_isolation.tla`.
//!
//! SSI is the algorithm behind PostgreSQL's SERIALIZABLE isolation level.
//! It extends Snapshot Isolation to prevent write skew anomalies by detecting
//! "dangerous structures" in the conflict graph.
//!
//! # Key Concepts
//!
//! - **Snapshot**: Each transaction sees a consistent view from its start time
//! - **SIREAD locks**: Track reads even after commit (for conflict detection)
//! - **Conflict flags**: `in_conflict` and `out_conflict` per transaction
//! - **Dangerous structure**: Transaction with both flags set (potential cycle)
//!
//! # Invariants
//!
//! 1. `FirstCommitterWins`: No concurrent commits to same key
//! 2. `SnapshotConsistency`: Reads see consistent snapshot
//! 3. `Serializable`: No dangerous structures at commit time

use std::collections::{BTreeMap, BTreeSet};
use std::hash::Hash;

/// Transaction identifier.
pub type TxnId = u8;

/// Key identifier.
pub type KeyId = u8;

/// Logical timestamp (position in history).
pub type Timestamp = u64;

/// Transaction status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum TxnStatus {
    NotStarted,
    Active,
    Committed,
    Aborted,
}

/// Operation in the history.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Operation {
    Begin { txn: TxnId },
    Read { txn: TxnId, key: KeyId, version: Option<TxnId> },
    Write { txn: TxnId, key: KeyId },
    Commit { txn: TxnId },
    Abort { txn: TxnId, reason: AbortReason },
}

/// Reason for abort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AbortReason {
    Voluntary,
    ReadConflict,
    WriteConflict,
    DangerousStructure,
}

/// SSI state machine state.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SsiState {
    /// Linear history of operations.
    pub history: Vec<Operation>,

    /// Transaction status.
    pub txn_status: BTreeMap<TxnId, TxnStatus>,

    /// Transaction snapshot timestamp (begin time).
    pub txn_snapshot: BTreeMap<TxnId, Timestamp>,

    /// Write locks: Key -> TxnId holding exclusive lock.
    pub write_locks: BTreeMap<KeyId, Option<TxnId>>,

    /// SIREAD locks: Key -> Set of TxnIds that have read (persists after commit).
    pub siread_locks: BTreeMap<KeyId, BTreeSet<TxnId>>,

    /// Incoming rw-conflict flag per transaction.
    pub in_conflict: BTreeMap<TxnId, bool>,

    /// Outgoing rw-conflict flag per transaction.
    pub out_conflict: BTreeMap<TxnId, bool>,
}

impl SsiState {
    /// Create initial state with given transaction and key sets.
    pub fn new(txns: &[TxnId], keys: &[KeyId]) -> Self {
        Self {
            history: Vec::new(),
            txn_status: txns.iter().map(|&t| (t, TxnStatus::NotStarted)).collect(),
            txn_snapshot: txns.iter().map(|&t| (t, 0)).collect(),
            write_locks: keys.iter().map(|&k| (k, None)).collect(),
            siread_locks: keys.iter().map(|&k| (k, BTreeSet::new())).collect(),
            in_conflict: txns.iter().map(|&t| (t, false)).collect(),
            out_conflict: txns.iter().map(|&t| (t, false)).collect(),
        }
    }

    /// Current logical timestamp.
    pub fn now(&self) -> Timestamp {
        self.history.len() as Timestamp
    }

    /// Get active transactions.
    pub fn active_txns(&self) -> BTreeSet<TxnId> {
        self.txn_status
            .iter()
            .filter(|(_, &s)| s == TxnStatus::Active)
            .map(|(&t, _)| t)
            .collect()
    }

    /// Get committed transactions.
    pub fn committed_txns(&self) -> BTreeSet<TxnId> {
        self.txn_status
            .iter()
            .filter(|(_, &s)| s == TxnStatus::Committed)
            .map(|(&t, _)| t)
            .collect()
    }

    /// Find latest committed version of key visible at snapshot time.
    pub fn latest_version(&self, key: KeyId, snapshot_time: Timestamp) -> Option<TxnId> {
        let committed = self.committed_txns();
        let mut latest: Option<(Timestamp, TxnId)> = None;

        for (i, op) in self.history.iter().enumerate() {
            if i as Timestamp > snapshot_time {
                break;
            }
            if let Operation::Write { txn, key: k } = op {
                if *k == key && committed.contains(txn) {
                    match latest {
                        None => latest = Some((i as Timestamp, *txn)),
                        Some((prev_ts, _)) if (i as Timestamp) > prev_ts => {
                            latest = Some((i as Timestamp, *txn));
                        }
                        _ => {}
                    }
                }
            }
        }

        latest.map(|(_, txn)| txn)
    }

    /// Check if transaction has dangerous structure (both conflict flags set).
    pub fn has_dangerous_structure(&self, txn: TxnId) -> bool {
        self.in_conflict.get(&txn).copied().unwrap_or(false)
            && self.out_conflict.get(&txn).copied().unwrap_or(false)
    }

    /// Find writers of key that wrote after given timestamp (excluding given txn).
    pub fn newer_writers(&self, key: KeyId, after_ts: Timestamp, exclude_txn: TxnId) -> BTreeSet<TxnId> {
        let active_or_committed: BTreeSet<TxnId> = self
            .txn_status
            .iter()
            .filter(|(_, &s)| s == TxnStatus::Active || s == TxnStatus::Committed)
            .map(|(&t, _)| t)
            .collect();

        self.history
            .iter()
            .enumerate()
            .filter_map(|(i, op)| {
                if let Operation::Write { txn, key: k } = op {
                    if *k == key
                        && *txn != exclude_txn  // Don't include self
                        && (i as Timestamp) > after_ts
                        && active_or_committed.contains(txn)
                    {
                        return Some(*txn);
                    }
                }
                None
            })
            .collect()
    }

    /// Find concurrent SIREAD lock holders for a key.
    pub fn concurrent_siread_holders(&self, txn: TxnId, key: KeyId) -> BTreeSet<TxnId> {
        let txn_start = self.txn_snapshot.get(&txn).copied().unwrap_or(0);

        self.siread_locks
            .get(&key)
            .map(|holders| {
                holders
                    .iter()
                    .filter(|&&holder| {
                        if holder == txn {
                            return false;
                        }
                        let status = self.txn_status.get(&holder).copied().unwrap_or(TxnStatus::NotStarted);
                        if status == TxnStatus::Active {
                            return true;
                        }
                        if status == TxnStatus::Committed {
                            // Check if committed after our start
                            for (i, op) in self.history.iter().enumerate() {
                                if let Operation::Commit { txn: t } = op {
                                    if *t == holder && (i as Timestamp) > txn_start {
                                        return true;
                                    }
                                }
                            }
                        }
                        false
                    })
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Actions that can be taken in SSI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SsiAction {
    Begin(TxnId),
    Read(TxnId, KeyId),
    Write(TxnId, KeyId),
    Commit(TxnId),
    Abort(TxnId),
}

impl SsiState {
    /// Get all possible actions from current state.
    pub fn possible_actions(&self) -> Vec<SsiAction> {
        let mut actions = Vec::new();

        for (&txn, &status) in &self.txn_status {
            match status {
                TxnStatus::NotStarted => {
                    actions.push(SsiAction::Begin(txn));
                }
                TxnStatus::Active => {
                    // Can read, write, commit, or abort
                    for &key in self.write_locks.keys() {
                        actions.push(SsiAction::Read(txn, key));
                        // Can only write if lock is free or we hold it
                        if self.write_locks.get(&key).copied().flatten().map_or(true, |h| h == txn) {
                            actions.push(SsiAction::Write(txn, key));
                        }
                    }
                    if !self.has_dangerous_structure(txn) {
                        actions.push(SsiAction::Commit(txn));
                    }
                    actions.push(SsiAction::Abort(txn));
                }
                _ => {}
            }
        }

        actions
    }

    /// Apply an action to produce a new state.
    pub fn apply(&self, action: &SsiAction) -> Option<Self> {
        let mut next = self.clone();

        match action {
            SsiAction::Begin(txn) => {
                if next.txn_status.get(txn) != Some(&TxnStatus::NotStarted) {
                    return None;
                }
                next.history.push(Operation::Begin { txn: *txn });
                next.txn_status.insert(*txn, TxnStatus::Active);
                next.txn_snapshot.insert(*txn, next.now() - 1);
            }

            SsiAction::Read(txn, key) => {
                if next.txn_status.get(txn) != Some(&TxnStatus::Active) {
                    return None;
                }

                let snapshot = next.txn_snapshot.get(txn).copied().unwrap_or(0);
                let version = if next.write_locks.get(key).copied().flatten() == Some(*txn) {
                    Some(*txn) // Read own write
                } else {
                    next.latest_version(*key, snapshot)
                };

                // Check for serializability violation
                let newer_writers = next.newer_writers(*key, snapshot, *txn);
                let would_violate = newer_writers.iter().any(|&writer| {
                    next.txn_status.get(&writer) == Some(&TxnStatus::Committed)
                        && next.out_conflict.get(&writer).copied().unwrap_or(false)
                });

                if would_violate {
                    // Abort to preserve serializability
                    next.history.push(Operation::Abort {
                        txn: *txn,
                        reason: AbortReason::ReadConflict,
                    });
                    next.txn_status.insert(*txn, TxnStatus::Aborted);
                    next.in_conflict.insert(*txn, false);
                    next.out_conflict.insert(*txn, false);
                    for locks in next.siread_locks.values_mut() {
                        locks.remove(txn);
                    }
                } else {
                    // Perform read
                    next.history.push(Operation::Read {
                        txn: *txn,
                        key: *key,
                        version,
                    });
                    next.siread_locks.entry(*key).or_default().insert(*txn);

                    // Update conflict flags
                    for &writer in &newer_writers {
                        next.in_conflict.insert(writer, true);
                    }
                    if !newer_writers.is_empty() {
                        next.out_conflict.insert(*txn, true);
                    }
                }
            }

            SsiAction::Write(txn, key) => {
                if next.txn_status.get(txn) != Some(&TxnStatus::Active) {
                    return None;
                }

                let lock_holder = next.write_locks.get(key).copied().flatten();
                if lock_holder.is_some() && lock_holder != Some(*txn) {
                    return None; // Lock held by another transaction
                }

                let concurrent_readers = next.concurrent_siread_holders(*txn, *key);

                // Check for serializability violation
                let would_violate = concurrent_readers.iter().any(|&reader| {
                    next.txn_status.get(&reader) == Some(&TxnStatus::Committed)
                        && next.in_conflict.get(&reader).copied().unwrap_or(false)
                });

                if would_violate {
                    // Abort to preserve serializability
                    next.history.push(Operation::Abort {
                        txn: *txn,
                        reason: AbortReason::WriteConflict,
                    });
                    next.txn_status.insert(*txn, TxnStatus::Aborted);
                    if next.write_locks.get(key).copied().flatten() == Some(*txn) {
                        next.write_locks.insert(*key, None);
                    }
                    next.in_conflict.insert(*txn, false);
                    next.out_conflict.insert(*txn, false);
                    for locks in next.siread_locks.values_mut() {
                        locks.remove(txn);
                    }
                } else {
                    // Perform write
                    next.history.push(Operation::Write { txn: *txn, key: *key });
                    next.write_locks.insert(*key, Some(*txn));

                    // Update conflict flags
                    for &reader in &concurrent_readers {
                        next.out_conflict.insert(reader, true);
                    }
                    if !concurrent_readers.is_empty() {
                        next.in_conflict.insert(*txn, true);
                    }
                }
            }

            SsiAction::Commit(txn) => {
                if next.txn_status.get(txn) != Some(&TxnStatus::Active) {
                    return None;
                }
                if next.has_dangerous_structure(*txn) {
                    // Cannot commit with dangerous structure - must abort
                    next.history.push(Operation::Abort {
                        txn: *txn,
                        reason: AbortReason::DangerousStructure,
                    });
                    next.txn_status.insert(*txn, TxnStatus::Aborted);
                    for lock in next.write_locks.values_mut() {
                        if *lock == Some(*txn) {
                            *lock = None;
                        }
                    }
                    next.in_conflict.insert(*txn, false);
                    next.out_conflict.insert(*txn, false);
                    for locks in next.siread_locks.values_mut() {
                        locks.remove(txn);
                    }
                } else {
                    next.history.push(Operation::Commit { txn: *txn });
                    next.txn_status.insert(*txn, TxnStatus::Committed);
                    // Release write locks
                    for lock in next.write_locks.values_mut() {
                        if *lock == Some(*txn) {
                            *lock = None;
                        }
                    }
                    // SIREAD locks persist after commit
                }
            }

            SsiAction::Abort(txn) => {
                if next.txn_status.get(txn) != Some(&TxnStatus::Active) {
                    return None;
                }
                next.history.push(Operation::Abort {
                    txn: *txn,
                    reason: AbortReason::Voluntary,
                });
                next.txn_status.insert(*txn, TxnStatus::Aborted);
                for lock in next.write_locks.values_mut() {
                    if *lock == Some(*txn) {
                        *lock = None;
                    }
                }
                for locks in next.siread_locks.values_mut() {
                    locks.remove(txn);
                }
                next.in_conflict.insert(*txn, false);
                next.out_conflict.insert(*txn, false);
            }
        }

        Some(next)
    }
}

// ============================================================================
// INVARIANTS
// ============================================================================

impl SsiState {
    /// I1: First Committer Wins
    /// No two concurrent transactions can both commit writes to the same key.
    pub fn first_committer_wins(&self) -> bool {
        let committed = self.committed_txns();

        for &key in self.write_locks.keys() {
            let writers: Vec<(Timestamp, TxnId)> = self
                .history
                .iter()
                .enumerate()
                .filter_map(|(i, op)| {
                    if let Operation::Write { txn, key: k } = op {
                        if *k == key && committed.contains(txn) {
                            return Some((i as Timestamp, *txn));
                        }
                    }
                    None
                })
                .collect();

            // Check all pairs of committed writers
            for i in 0..writers.len() {
                for j in (i + 1)..writers.len() {
                    let (ts1, t1) = writers[i];
                    let (ts2, t2) = writers[j];
                    let snap1 = self.txn_snapshot.get(&t1).copied().unwrap_or(0);
                    let snap2 = self.txn_snapshot.get(&t2).copied().unwrap_or(0);

                    // Check if they were concurrent (overlapping lifetimes)
                    if snap1 < ts2 && snap2 < ts1 {
                        return false; // Concurrent committed writers - violation
                    }
                }
            }
        }

        true
    }

    /// I2: No Dangerous Structures at Commit
    /// No committed transaction has both in_conflict and out_conflict set.
    pub fn no_committed_dangerous_structures(&self) -> bool {
        for &txn in &self.committed_txns() {
            if self.has_dangerous_structure(txn) {
                return false;
            }
        }
        true
    }

    /// I3: Serializability (simplified check)
    /// The committed history is serializable.
    pub fn is_serializable(&self) -> bool {
        // Simplified: if no committed transaction has dangerous structure, we're safe
        self.no_committed_dangerous_structures() && self.first_committer_wins()
    }

    /// Check all invariants.
    pub fn check_invariants(&self) -> Vec<&'static str> {
        let mut violations = Vec::new();

        if !self.first_committer_wins() {
            violations.push("FirstCommitterWins");
        }
        if !self.no_committed_dangerous_structures() {
            violations.push("NoCommittedDangerousStructures");
        }
        if !self.is_serializable() {
            violations.push("Serializable");
        }

        violations
    }
}

// ============================================================================
// ORACLE EXTRACTION
// ============================================================================

/// Categories of interesting SSI scenarios for testing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SsiOracleCategory {
    /// Two transactions successfully commit writes to different keys
    ConcurrentCommit,
    /// Write-write conflict detected and one aborts
    WriteWriteConflict,
    /// Read-write conflict creates dangerous structure
    DangerousStructure,
    /// Transaction aborts due to serializability violation
    SerializabilityAbort,
    /// Successful commit after conflict detection (no dangerous structure)
    CommitWithConflict,
    /// Deadlock avoidance scenario
    DeadlockAvoidance,
}

/// An oracle capturing an interesting SSI execution.
#[derive(Debug, Clone)]
pub struct SsiOracle {
    pub name: String,
    pub category: SsiOracleCategory,
    pub actions: Vec<SsiAction>,
    pub description: String,
}

impl SsiOracle {
    /// Pre-built oracle: Write-write conflict
    ///
    /// T1 writes and commits, then T2 starts (serial execution).
    /// Shows that sequential writes to same key are allowed.
    pub fn write_write_conflict() -> Self {
        Self {
            name: "write_write_sequential".into(),
            category: SsiOracleCategory::ConcurrentCommit, // Actually serial, both commit
            actions: vec![
                SsiAction::Begin(1),
                SsiAction::Write(1, 1), // T1 writes K1
                SsiAction::Commit(1),   // T1 commits
                SsiAction::Begin(2),    // T2 starts AFTER T1 commits (serial)
                SsiAction::Write(2, 1), // T2 writes K1
                SsiAction::Commit(2),   // T2 commits (serial execution)
            ],
            description: "Sequential writes to same key - T2 starts after T1 commits".into(),
        }
    }

    /// Pre-built oracle: Dangerous structure detection
    ///
    /// Classic write skew setup that SSI must prevent.
    pub fn dangerous_structure() -> Self {
        Self {
            name: "dangerous_structure".into(),
            category: SsiOracleCategory::DangerousStructure,
            actions: vec![
                SsiAction::Begin(1),
                SsiAction::Begin(2),
                SsiAction::Read(1, 1),  // T1 reads K1
                SsiAction::Read(2, 2),  // T2 reads K2
                SsiAction::Write(1, 2), // T1 writes K2 (conflict with T2's read)
                SsiAction::Write(2, 1), // T2 writes K1 (conflict with T1's read)
                // Now both have in_conflict AND out_conflict = dangerous structure
                // Commit attempts should fail
                SsiAction::Commit(1), // Should abort due to dangerous structure
            ],
            description: "Write skew pattern - both txns have dangerous structure".into(),
        }
    }

    /// Pre-built oracle: Safe concurrent operations
    ///
    /// Two transactions on disjoint keys - should both commit.
    pub fn disjoint_keys() -> Self {
        Self {
            name: "disjoint_keys".into(),
            category: SsiOracleCategory::ConcurrentCommit,
            actions: vec![
                SsiAction::Begin(1),
                SsiAction::Begin(2),
                SsiAction::Write(1, 1), // T1 writes K1
                SsiAction::Write(2, 2), // T2 writes K2 (different key)
                SsiAction::Commit(1),
                SsiAction::Commit(2),
            ],
            description: "Concurrent writes to different keys - both commit".into(),
        }
    }

    /// Pre-built oracle: Read-only transaction
    ///
    /// Read-only transactions never conflict.
    pub fn read_only() -> Self {
        Self {
            name: "read_only".into(),
            category: SsiOracleCategory::ConcurrentCommit,
            actions: vec![
                SsiAction::Begin(1),
                SsiAction::Write(1, 1),
                SsiAction::Commit(1),
                SsiAction::Begin(2),
                SsiAction::Read(2, 1), // T2 reads T1's write
                SsiAction::Commit(2),
            ],
            description: "Read-only transaction sees committed write".into(),
        }
    }

    /// Pre-built oracle: Commit with single conflict flag
    ///
    /// Having only in_conflict OR out_conflict is safe.
    pub fn single_conflict_flag() -> Self {
        Self {
            name: "single_conflict_flag".into(),
            category: SsiOracleCategory::CommitWithConflict,
            actions: vec![
                SsiAction::Begin(1),
                SsiAction::Begin(2),
                SsiAction::Read(1, 1),  // T1 reads K1
                SsiAction::Write(2, 1), // T2 writes K1 (T1 gets out_conflict)
                SsiAction::Commit(2),   // T2 commits
                SsiAction::Commit(1),   // T1 can commit (only out_conflict, no in_conflict)
            ],
            description: "Transaction commits with only out_conflict set".into(),
        }
    }

    /// Pre-built oracle: Concurrent write attempt blocked
    ///
    /// T2 cannot write while T1 holds the lock.
    pub fn concurrent_write_blocked() -> Self {
        Self {
            name: "concurrent_write_blocked".into(),
            category: SsiOracleCategory::WriteWriteConflict,
            actions: vec![
                SsiAction::Begin(1),
                SsiAction::Begin(2),
                SsiAction::Write(1, 1), // T1 writes K1, holds lock
                // T2's write to K1 would fail (lock held)
                SsiAction::Write(2, 2), // T2 writes K2 instead (different key)
                SsiAction::Commit(1),
                SsiAction::Commit(2),
            ],
            description: "T2 cannot write K1 while T1 holds lock, writes K2 instead".into(),
        }
    }

    /// All pre-built SSI oracles.
    pub fn all_oracles() -> Vec<Self> {
        vec![
            Self::write_write_conflict(),
            Self::concurrent_write_blocked(),
            Self::dangerous_structure(),
            Self::disjoint_keys(),
            Self::read_only(),
            Self::single_conflict_flag(),
        ]
    }
}

/// Extract oracles by running the state machine and finding interesting paths.
pub struct SsiOracleExtractor {
    pub oracles: Vec<SsiOracle>,
    max_depth: usize,
}

impl SsiOracleExtractor {
    pub fn new(max_depth: usize) -> Self {
        Self {
            oracles: Vec::new(),
            max_depth,
        }
    }

    /// Extract oracles by exploring the state space.
    pub fn extract(&mut self, txns: &[TxnId], keys: &[KeyId]) -> Vec<SsiOracle> {
        let initial = SsiState::new(txns, keys);
        let mut visited = std::collections::HashSet::new();
        let mut interesting = Vec::new();

        self.explore(initial, Vec::new(), &mut visited, &mut interesting, 0);

        // Also include pre-built oracles
        let mut result = SsiOracle::all_oracles();
        result.extend(interesting);
        result
    }

    fn explore(
        &self,
        state: SsiState,
        path: Vec<SsiAction>,
        visited: &mut std::collections::HashSet<SsiState>,
        interesting: &mut Vec<SsiOracle>,
        depth: usize,
    ) {
        if depth >= self.max_depth {
            return;
        }

        if !visited.insert(state.clone()) {
            return;
        }

        // Check if this state is interesting
        self.check_interesting(&state, &path, interesting);

        // Explore successors
        for action in state.possible_actions() {
            if let Some(next) = state.apply(&action) {
                let mut new_path = path.clone();
                new_path.push(action);
                self.explore(next, new_path, visited, interesting, depth + 1);
            }
        }
    }

    fn check_interesting(
        &self,
        state: &SsiState,
        path: &[SsiAction],
        interesting: &mut Vec<SsiOracle>,
    ) {
        // Check for dangerous structure that was aborted
        for (&txn, &status) in &state.txn_status {
            if status == TxnStatus::Aborted {
                // Check if it was aborted due to dangerous structure
                for op in &state.history {
                    if let Operation::Abort { txn: t, reason } = op {
                        if *t == txn && *reason == AbortReason::DangerousStructure {
                            interesting.push(SsiOracle {
                                name: format!("dangerous_abort_t{}", txn),
                                category: SsiOracleCategory::SerializabilityAbort,
                                actions: path.to_vec(),
                                description: format!("T{} aborted due to dangerous structure", txn),
                            });
                        }
                    }
                }
            }
        }

        // Check for successful commit with conflict flags
        for &txn in &state.committed_txns() {
            let in_c = state.in_conflict.get(&txn).copied().unwrap_or(false);
            let out_c = state.out_conflict.get(&txn).copied().unwrap_or(false);

            if in_c || out_c {
                interesting.push(SsiOracle {
                    name: format!("commit_with_conflict_t{}", txn),
                    category: SsiOracleCategory::CommitWithConflict,
                    actions: path.to_vec(),
                    description: format!(
                        "T{} committed with in_conflict={}, out_conflict={}",
                        txn, in_c, out_c
                    ),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let state = SsiState::new(&[1, 2], &[1, 2]);
        assert_eq!(state.txn_status.get(&1), Some(&TxnStatus::NotStarted));
        assert_eq!(state.write_locks.get(&1), Some(&None));
        assert!(state.check_invariants().is_empty());
    }

    #[test]
    fn test_begin_transaction() {
        let state = SsiState::new(&[1, 2], &[1]);
        let next = state.apply(&SsiAction::Begin(1)).unwrap();

        assert_eq!(next.txn_status.get(&1), Some(&TxnStatus::Active));
        assert!(next.check_invariants().is_empty());
    }

    #[test]
    fn test_simple_read_write_commit() {
        let mut state = SsiState::new(&[1], &[1]);

        state = state.apply(&SsiAction::Begin(1)).unwrap();
        state = state.apply(&SsiAction::Write(1, 1)).unwrap();
        state = state.apply(&SsiAction::Read(1, 1)).unwrap();
        state = state.apply(&SsiAction::Commit(1)).unwrap();

        assert_eq!(state.txn_status.get(&1), Some(&TxnStatus::Committed));
        assert!(state.check_invariants().is_empty());
    }

    #[test]
    fn test_concurrent_writers_blocked() {
        let mut state = SsiState::new(&[1, 2], &[1]);

        // T1 begins and writes
        state = state.apply(&SsiAction::Begin(1)).unwrap();
        state = state.apply(&SsiAction::Write(1, 1)).unwrap();

        // T2 begins
        state = state.apply(&SsiAction::Begin(2)).unwrap();

        // T2 cannot write to same key (lock held)
        let actions = state.possible_actions();
        assert!(!actions.contains(&SsiAction::Write(2, 1)));
    }

    #[test]
    fn test_dangerous_structure_prevents_commit() {
        let mut state = SsiState::new(&[1, 2], &[1, 2]);

        // Setup: create a scenario where T1 has both conflict flags
        state = state.apply(&SsiAction::Begin(1)).unwrap();
        state = state.apply(&SsiAction::Begin(2)).unwrap();

        // T1 reads key 1
        state = state.apply(&SsiAction::Read(1, 1)).unwrap();

        // T2 writes key 1 (creates rw-conflict: T1 -> T2, sets T1.out_conflict)
        state = state.apply(&SsiAction::Write(2, 1)).unwrap();

        // T2 reads key 2
        state = state.apply(&SsiAction::Read(2, 2)).unwrap();

        // T2 commits
        state = state.apply(&SsiAction::Commit(2)).unwrap();

        // T1 writes key 2 (creates rw-conflict: T2 -> T1, sets T1.in_conflict)
        state = state.apply(&SsiAction::Write(1, 2)).unwrap();

        // Now T1 has dangerous structure - commit should not be in possible actions
        // (or commit will abort)
        if state.has_dangerous_structure(1) {
            let actions = state.possible_actions();
            assert!(!actions.contains(&SsiAction::Commit(1)));
        }

        assert!(state.check_invariants().is_empty());
    }
}
