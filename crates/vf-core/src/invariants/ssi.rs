//! SSI (Serializable Snapshot Isolation) invariants.
//!
//! These invariants are shared across all verification layers:
//! - Stateright model checking
//! - DST fault injection
//! - Runtime assertions
//!
//! Mirrors `specs/ssi/serializable_snapshot_isolation.tla`.

use std::collections::{HashMap, HashSet};

/// Transaction identifier.
pub type TxnId = u64;

/// Key identifier.
pub type KeyId = u64;

/// Timestamp (logical clock).
pub type Timestamp = u64;

/// Transaction status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxnStatus {
    Active,
    Committed,
    Aborted,
}

/// A read operation record.
#[derive(Debug, Clone)]
pub struct ReadRecord {
    pub txn: TxnId,
    pub key: KeyId,
    pub version: Option<TxnId>, // Which txn's write we read (None = initial)
    pub timestamp: Timestamp,
}

/// A write operation record.
#[derive(Debug, Clone)]
pub struct WriteRecord {
    pub txn: TxnId,
    pub key: KeyId,
    pub timestamp: Timestamp,
}

/// SSI execution history for invariant checking.
#[derive(Debug, Clone, Default)]
pub struct SsiHistory {
    /// All read operations.
    pub reads: Vec<ReadRecord>,
    /// All write operations.
    pub writes: Vec<WriteRecord>,
    /// Transaction status.
    pub txn_status: HashMap<TxnId, TxnStatus>,
    /// Transaction start timestamps.
    pub txn_start: HashMap<TxnId, Timestamp>,
    /// Transaction commit timestamps.
    pub txn_commit: HashMap<TxnId, Timestamp>,
    /// In-conflict flags (has incoming rw-dependency).
    pub in_conflict: HashMap<TxnId, bool>,
    /// Out-conflict flags (has outgoing rw-dependency).
    pub out_conflict: HashMap<TxnId, bool>,
}

impl SsiHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a transaction start.
    pub fn begin(&mut self, txn: TxnId, timestamp: Timestamp) {
        self.txn_status.insert(txn, TxnStatus::Active);
        self.txn_start.insert(txn, timestamp);
        self.in_conflict.insert(txn, false);
        self.out_conflict.insert(txn, false);
    }

    /// Record a read operation.
    pub fn read(&mut self, txn: TxnId, key: KeyId, version: Option<TxnId>, timestamp: Timestamp) {
        self.reads.push(ReadRecord {
            txn,
            key,
            version,
            timestamp,
        });
    }

    /// Record a write operation.
    pub fn write(&mut self, txn: TxnId, key: KeyId, timestamp: Timestamp) {
        self.writes.push(WriteRecord {
            txn,
            key,
            timestamp,
        });
    }

    /// Record a commit.
    pub fn commit(&mut self, txn: TxnId, timestamp: Timestamp) {
        self.txn_status.insert(txn, TxnStatus::Committed);
        self.txn_commit.insert(txn, timestamp);
    }

    /// Record an abort.
    pub fn abort(&mut self, txn: TxnId) {
        self.txn_status.insert(txn, TxnStatus::Aborted);
        self.in_conflict.insert(txn, false);
        self.out_conflict.insert(txn, false);
    }

    /// Set in_conflict flag.
    pub fn set_in_conflict(&mut self, txn: TxnId) {
        self.in_conflict.insert(txn, true);
    }

    /// Set out_conflict flag.
    pub fn set_out_conflict(&mut self, txn: TxnId) {
        self.out_conflict.insert(txn, true);
    }

    /// Get committed transactions.
    pub fn committed_txns(&self) -> HashSet<TxnId> {
        self.txn_status
            .iter()
            .filter(|(_, &s)| s == TxnStatus::Committed)
            .map(|(&t, _)| t)
            .collect()
    }

    /// Check if two transactions were concurrent.
    pub fn were_concurrent(&self, t1: TxnId, t2: TxnId) -> bool {
        let start1 = self.txn_start.get(&t1).copied().unwrap_or(0);
        let start2 = self.txn_start.get(&t2).copied().unwrap_or(0);
        let commit1 = self.txn_commit.get(&t1).copied();
        let commit2 = self.txn_commit.get(&t2).copied();

        // t1 and t2 are concurrent if their lifetimes overlap
        match (commit1, commit2) {
            (Some(c1), Some(c2)) => start1 < c2 && start2 < c1,
            (Some(c1), None) => start2 < c1,
            (None, Some(c2)) => start1 < c2,
            (None, None) => true, // Both active = concurrent
        }
    }
}

// ============================================================================
// INVARIANTS
// ============================================================================

/// I1: First Committer Wins
///
/// No two concurrent transactions can both commit writes to the same key.
/// This is a fundamental property of Snapshot Isolation.
pub fn first_committer_wins(history: &SsiHistory) -> InvariantResult {
    let committed = history.committed_txns();

    // Group writes by key
    let mut writes_by_key: HashMap<KeyId, Vec<TxnId>> = HashMap::new();
    for w in &history.writes {
        if committed.contains(&w.txn) {
            writes_by_key.entry(w.key).or_default().push(w.txn);
        }
    }

    // Check each key for concurrent committed writers
    for (key, writers) in writes_by_key {
        for i in 0..writers.len() {
            for j in (i + 1)..writers.len() {
                let t1 = writers[i];
                let t2 = writers[j];
                if history.were_concurrent(t1, t2) {
                    return InvariantResult::violated(
                        "FirstCommitterWins",
                        format!(
                            "Concurrent transactions T{} and T{} both committed writes to key {}",
                            t1, t2, key
                        ),
                    );
                }
            }
        }
    }

    InvariantResult::holds("FirstCommitterWins")
}

/// I2: No Committed Dangerous Structures
///
/// No committed transaction has both in_conflict and out_conflict set.
/// A dangerous structure is a potential cycle in the serialization graph.
pub fn no_committed_dangerous_structures(history: &SsiHistory) -> InvariantResult {
    for &txn in &history.committed_txns() {
        let in_c = history.in_conflict.get(&txn).copied().unwrap_or(false);
        let out_c = history.out_conflict.get(&txn).copied().unwrap_or(false);

        if in_c && out_c {
            return InvariantResult::violated(
                "NoCommittedDangerousStructures",
                format!(
                    "Committed transaction T{} has dangerous structure (in_conflict={}, out_conflict={})",
                    txn, in_c, out_c
                ),
            );
        }
    }

    InvariantResult::holds("NoCommittedDangerousStructures")
}

/// I3: Serializable
///
/// The committed history is equivalent to some serial execution.
/// For SSI, this is ensured by I1 + I2.
pub fn is_serializable(history: &SsiHistory) -> InvariantResult {
    let fcw = first_committer_wins(history);
    if !fcw.holds {
        return InvariantResult::violated(
            "Serializable",
            format!("Serializability violated: {}", fcw.message.unwrap_or_default()),
        );
    }

    let nds = no_committed_dangerous_structures(history);
    if !nds.holds {
        return InvariantResult::violated(
            "Serializable",
            format!("Serializability violated: {}", nds.message.unwrap_or_default()),
        );
    }

    InvariantResult::holds("Serializable")
}

/// I4: No Lost Writes
///
/// Every committed write is visible to transactions that start after it commits.
pub fn no_lost_writes(history: &SsiHistory) -> InvariantResult {
    let committed = history.committed_txns();

    for write in &history.writes {
        if !committed.contains(&write.txn) {
            continue;
        }

        let write_commit = history.txn_commit.get(&write.txn).copied().unwrap_or(0);

        // Check that later readers of this key see this write or a later one
        for read in &history.reads {
            if read.key != write.key {
                continue;
            }
            if !committed.contains(&read.txn) {
                continue;
            }

            let read_start = history.txn_start.get(&read.txn).copied().unwrap_or(0);

            // If reader started after writer committed, they should see the write
            if read_start > write_commit {
                if let Some(version) = read.version {
                    // Check if read version is at least as new as write
                    let version_commit = history.txn_commit.get(&version).copied().unwrap_or(0);
                    if version_commit < write_commit {
                        return InvariantResult::violated(
                            "NoLostWrites",
                            format!(
                                "T{} read stale version from T{} for key {}, missing write from T{}",
                                read.txn, version, read.key, write.txn
                            ),
                        );
                    }
                }
            }
        }
    }

    InvariantResult::holds("NoLostWrites")
}

/// Check all SSI invariants.
pub fn check_all(history: &SsiHistory) -> Vec<InvariantResult> {
    vec![
        first_committer_wins(history),
        no_committed_dangerous_structures(history),
        is_serializable(history),
        no_lost_writes(history),
    ]
}

/// Result of checking an invariant.
#[derive(Debug, Clone)]
pub struct InvariantResult {
    pub name: &'static str,
    pub holds: bool,
    pub message: Option<String>,
}

impl InvariantResult {
    pub fn holds(name: &'static str) -> Self {
        Self {
            name,
            holds: true,
            message: None,
        }
    }

    pub fn violated(name: &'static str, message: String) -> Self {
        Self {
            name,
            holds: false,
            message: Some(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_history_passes() {
        let history = SsiHistory::new();
        let results = check_all(&history);
        assert!(results.iter().all(|r| r.holds));
    }

    #[test]
    fn test_simple_transaction_passes() {
        let mut history = SsiHistory::new();

        history.begin(1, 0);
        history.write(1, 100, 1);
        history.read(1, 100, Some(1), 2);
        history.commit(1, 3);

        let results = check_all(&history);
        assert!(results.iter().all(|r| r.holds), "{:?}", results);
    }

    #[test]
    fn test_concurrent_writers_detected() {
        let mut history = SsiHistory::new();

        // T1 and T2 both start
        history.begin(1, 0);
        history.begin(2, 1);

        // Both write to same key
        history.write(1, 100, 2);
        history.write(2, 100, 3);

        // Both commit (this violates FirstCommitterWins)
        history.commit(1, 4);
        history.commit(2, 5);

        let result = first_committer_wins(&history);
        assert!(!result.holds, "Should detect concurrent writers");
    }

    #[test]
    fn test_dangerous_structure_detected() {
        let mut history = SsiHistory::new();

        history.begin(1, 0);
        history.set_in_conflict(1);
        history.set_out_conflict(1);
        history.commit(1, 1);

        let result = no_committed_dangerous_structures(&history);
        assert!(!result.holds, "Should detect dangerous structure");
    }

    #[test]
    fn test_serial_execution_passes() {
        let mut history = SsiHistory::new();

        // T1 completes before T2 starts (serial)
        history.begin(1, 0);
        history.write(1, 100, 1);
        history.commit(1, 2);

        history.begin(2, 3);
        history.write(2, 100, 4);
        history.commit(2, 5);

        let results = check_all(&history);
        assert!(results.iter().all(|r| r.holds), "{:?}", results);
    }
}
