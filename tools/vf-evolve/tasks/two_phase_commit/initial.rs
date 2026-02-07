/// Two-Phase Commit - Initial Seed (Mutex-based blocking implementation)
/// ShinkaEvolve will evolve this from Blocking -> LockFree -> WaitFree
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub type RmId = u64;
pub type TxnId = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RmState {
    Working,
    Prepared,
    Committed,
    Aborted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmState {
    Init,
    Preparing,
    Committed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TpcError {
    RmNotFound,
    TxnNotFound,
    InvalidState,
    AlreadyDecided,
    NotAllPrepared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TmDecision {
    Commit,
    Abort,
}

struct TxnInner {
    tm_state: TmState,
    rm_states: HashMap<RmId, RmState>,
    prepared_set: HashSet<RmId>,
}

pub struct TwoPhaseCommit {
    rm_ids: Vec<RmId>,
    txns: Mutex<HashMap<TxnId, TxnInner>>,
    next_txn_id: AtomicU64,
}

impl TwoPhaseCommit {
    pub fn new(rm_ids: &[RmId]) -> Self {
        debug_assert!(!rm_ids.is_empty(), "Must have at least one resource manager");

        TwoPhaseCommit {
            rm_ids: rm_ids.to_vec(),
            txns: Mutex::new(HashMap::new()),
            next_txn_id: AtomicU64::new(1),
        }
    }

    pub fn rm_count(&self) -> usize {
        self.rm_ids.len()
    }

    /// Begin a new transaction. Returns the transaction ID.
    pub fn begin(&self) -> TxnId {
        let txn_id = self.next_txn_id.fetch_add(1, Ordering::Relaxed);
        let inner = TxnInner {
            tm_state: TmState::Init,
            rm_states: self.rm_ids.iter().map(|&id| (id, RmState::Working)).collect(),
            prepared_set: HashSet::new(),
        };
        self.txns.lock().unwrap().insert(txn_id, inner);
        txn_id
    }

    /// Get the TM state for a transaction.
    pub fn get_tm_state(&self, txn_id: TxnId) -> Option<TmState> {
        self.txns
            .lock()
            .unwrap()
            .get(&txn_id)
            .map(|t| t.tm_state)
    }

    /// Get the RM state for a specific resource manager in a transaction.
    pub fn get_rm_state(&self, txn_id: TxnId, rm_id: RmId) -> Option<RmState> {
        self.txns
            .lock()
            .unwrap()
            .get(&txn_id)
            .and_then(|t| t.rm_states.get(&rm_id).copied())
    }

    /// TM initiates prepare phase: sends Prepare to all RMs.
    pub fn tm_prepare(&self, txn_id: TxnId) -> Result<(), TpcError> {
        let mut txns = self.txns.lock().unwrap();
        let txn = txns.get_mut(&txn_id).ok_or(TpcError::TxnNotFound)?;

        if txn.tm_state != TmState::Init {
            return Err(TpcError::InvalidState);
        }

        txn.tm_state = TmState::Preparing;
        Ok(())
    }

    /// RM votes to prepare (yes vote).
    pub fn rm_prepare(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let mut txns = self.txns.lock().unwrap();
        let txn = txns.get_mut(&txn_id).ok_or(TpcError::TxnNotFound)?;

        if !txn.rm_states.contains_key(&rm_id) {
            return Err(TpcError::RmNotFound);
        }

        let rm_state = txn.rm_states.get(&rm_id).copied().unwrap();
        if rm_state != RmState::Working {
            return Err(TpcError::InvalidState);
        }

        txn.rm_states.insert(rm_id, RmState::Prepared);
        txn.prepared_set.insert(rm_id);
        Ok(())
    }

    /// RM unilaterally chooses to abort (no vote, before preparing).
    pub fn rm_choose_to_abort(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let mut txns = self.txns.lock().unwrap();
        let txn = txns.get_mut(&txn_id).ok_or(TpcError::TxnNotFound)?;

        if !txn.rm_states.contains_key(&rm_id) {
            return Err(TpcError::RmNotFound);
        }

        let rm_state = txn.rm_states.get(&rm_id).copied().unwrap();
        if rm_state != RmState::Working {
            return Err(TpcError::InvalidState);
        }

        txn.rm_states.insert(rm_id, RmState::Aborted);
        Ok(())
    }

    /// TM decides to commit. Only valid if all RMs are prepared.
    pub fn tm_commit(&self, txn_id: TxnId) -> Result<(), TpcError> {
        let mut txns = self.txns.lock().unwrap();
        let txn = txns.get_mut(&txn_id).ok_or(TpcError::TxnNotFound)?;

        if txn.tm_state != TmState::Preparing {
            return Err(TpcError::InvalidState);
        }

        // Check all RMs are prepared
        let all_rm_ids: HashSet<RmId> = txn.rm_states.keys().copied().collect();
        if txn.prepared_set != all_rm_ids {
            return Err(TpcError::NotAllPrepared);
        }

        txn.tm_state = TmState::Committed;
        Ok(())
    }

    /// TM decides to abort.
    pub fn tm_abort(&self, txn_id: TxnId) -> Result<(), TpcError> {
        let mut txns = self.txns.lock().unwrap();
        let txn = txns.get_mut(&txn_id).ok_or(TpcError::TxnNotFound)?;

        if txn.tm_state == TmState::Committed || txn.tm_state == TmState::Aborted {
            return Err(TpcError::AlreadyDecided);
        }

        txn.tm_state = TmState::Aborted;
        Ok(())
    }

    /// RM receives Commit message from TM.
    pub fn rm_rcv_commit(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let mut txns = self.txns.lock().unwrap();
        let txn = txns.get_mut(&txn_id).ok_or(TpcError::TxnNotFound)?;

        if !txn.rm_states.contains_key(&rm_id) {
            return Err(TpcError::RmNotFound);
        }

        if txn.tm_state != TmState::Committed {
            return Err(TpcError::InvalidState);
        }

        txn.rm_states.insert(rm_id, RmState::Committed);
        Ok(())
    }

    /// RM receives Abort message from TM.
    pub fn rm_rcv_abort(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let mut txns = self.txns.lock().unwrap();
        let txn = txns.get_mut(&txn_id).ok_or(TpcError::TxnNotFound)?;

        if !txn.rm_states.contains_key(&rm_id) {
            return Err(TpcError::RmNotFound);
        }

        if txn.tm_state != TmState::Aborted {
            return Err(TpcError::InvalidState);
        }

        txn.rm_states.insert(rm_id, RmState::Aborted);
        Ok(())
    }

    /// Run a complete successful 2PC: prepare all, commit all.
    pub fn run_commit(&self, txn_id: TxnId) -> Result<TmDecision, TpcError> {
        // Phase 1: Prepare
        self.tm_prepare(txn_id)?;

        // All RMs vote yes
        let rm_ids = self.rm_ids.clone();
        for &rm_id in &rm_ids {
            self.rm_prepare(txn_id, rm_id)?;
        }

        // Phase 2: Commit
        self.tm_commit(txn_id)?;

        // Deliver commit to all RMs
        for &rm_id in &rm_ids {
            self.rm_rcv_commit(txn_id, rm_id)?;
        }

        Ok(TmDecision::Commit)
    }

    /// Run 2PC where a specific RM aborts.
    pub fn run_abort(
        &self,
        txn_id: TxnId,
        aborting_rm: RmId,
    ) -> Result<TmDecision, TpcError> {
        // Phase 1: Prepare
        self.tm_prepare(txn_id)?;

        // RMs vote; aborting_rm votes no
        let rm_ids = self.rm_ids.clone();
        for &rm_id in &rm_ids {
            if rm_id == aborting_rm {
                self.rm_choose_to_abort(txn_id, rm_id)?;
            } else {
                self.rm_prepare(txn_id, rm_id)?;
            }
        }

        // Phase 2: Abort (not all prepared)
        self.tm_abort(txn_id)?;

        // Deliver abort to all RMs
        for &rm_id in &rm_ids {
            // Only abort RMs that haven't already aborted
            let state = self.get_rm_state(txn_id, rm_id);
            if state != Some(RmState::Aborted) {
                self.rm_rcv_abort(txn_id, rm_id)?;
            }
        }

        Ok(TmDecision::Abort)
    }
}
