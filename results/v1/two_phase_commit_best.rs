/// Two-Phase Commit - Lock-Free Implementation
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crossbeam_epoch::{self as epoch, Atomic, Guard, Owned};

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
    tm_state: AtomicUsize,
    rm_states: HashMap<RmId, AtomicUsize>,
    prepared_count: AtomicUsize,
    total_rms: usize,
}

impl TxnInner {
    fn new(rm_ids: &[RmId]) -> Self {
        let rm_states = rm_ids.iter()
            .map(|&id| (id, AtomicUsize::new(RmState::Working as usize)))
            .collect();
        
        TxnInner {
            tm_state: AtomicUsize::new(TmState::Init as usize),
            rm_states,
            prepared_count: AtomicUsize::new(0),
            total_rms: rm_ids.len(),
        }
    }

    fn get_tm_state(&self) -> TmState {
        match self.tm_state.load(Ordering::Acquire) {
            0 => TmState::Init,
            1 => TmState::Preparing,
            2 => TmState::Committed,
            3 => TmState::Aborted,
            _ => unreachable!(),
        }
    }

    fn set_tm_state(&self, from: TmState, to: TmState) -> bool {
        let from_val = from as usize;
        let to_val = to as usize;
        self.tm_state.compare_exchange_weak(from_val, to_val, Ordering::AcqRel, Ordering::Acquire).is_ok()
    }

    fn get_rm_state(&self, rm_id: RmId) -> Option<RmState> {
        self.rm_states.get(&rm_id).map(|atomic| {
            match atomic.load(Ordering::Acquire) {
                0 => RmState::Working,
                1 => RmState::Prepared,
                2 => RmState::Committed,
                3 => RmState::Aborted,
                _ => unreachable!(),
            }
        })
    }

    fn set_rm_state(&self, rm_id: RmId, from: RmState, to: RmState) -> bool {
        if let Some(atomic) = self.rm_states.get(&rm_id) {
            let from_val = from as usize;
            let to_val = to as usize;
            atomic.compare_exchange_weak(from_val, to_val, Ordering::AcqRel, Ordering::Acquire).is_ok()
        } else {
            false
        }
    }
}

struct TxnNode {
    txn_id: TxnId,
    inner: TxnInner,
    next: Atomic<TxnNode>,
}

impl TxnNode {
    fn new(txn_id: TxnId, rm_ids: &[RmId]) -> Self {
        TxnNode {
            txn_id,
            inner: TxnInner::new(rm_ids),
            next: Atomic::null(),
        }
    }
}

pub struct TwoPhaseCommit {
    rm_ids: Vec<RmId>,
    head: Atomic<TxnNode>,
    next_txn_id: AtomicU64,
}

impl TwoPhaseCommit {
    pub fn new(rm_ids: &[RmId]) -> Self {
        debug_assert!(!rm_ids.is_empty(), "Must have at least one resource manager");

        TwoPhaseCommit {
            rm_ids: rm_ids.to_vec(),
            head: Atomic::null(),
            next_txn_id: AtomicU64::new(1),
        }
    }

    pub fn rm_count(&self) -> usize {
        self.rm_ids.len()
    }

    fn find_txn<'g>(&self, txn_id: TxnId, guard: &'g Guard) -> Option<&'g TxnNode> {
        let mut current = self.head.load(Ordering::Acquire, guard);
        
        while !current.is_null() {
            let node = unsafe { current.deref() };
            if node.txn_id == txn_id {
                return Some(node);
            }
            current = node.next.load(Ordering::Acquire, guard);
        }
        None
    }

    pub fn begin(&self) -> TxnId {
        let guard = epoch::pin();
        let txn_id = self.next_txn_id.fetch_add(1, Ordering::Relaxed);
        
        loop {
            let new_node = Owned::new(TxnNode::new(txn_id, &self.rm_ids));
            let head = self.head.load(Ordering::Acquire, &guard);
            new_node.next.store(head, Ordering::Relaxed);
            
            match self.head.compare_exchange_weak(
                head,
                new_node,
                Ordering::Release,
                Ordering::Relaxed,
                &guard,
            ) {
                Ok(_) => break,
                Err(e) => {
                    // Continue with a new node
                    continue;
                }
            }
        }
        
        txn_id
    }

    pub fn get_tm_state(&self, txn_id: TxnId) -> Option<TmState> {
        let guard = epoch::pin();
        self.find_txn(txn_id, &guard).map(|node| node.inner.get_tm_state())
    }

    pub fn get_rm_state(&self, txn_id: TxnId, rm_id: RmId) -> Option<RmState> {
        let guard = epoch::pin();
        self.find_txn(txn_id, &guard)?.inner.get_rm_state(rm_id)
    }

    pub fn tm_prepare(&self, txn_id: TxnId) -> Result<(), TpcError> {
        let guard = epoch::pin();
        let node = self.find_txn(txn_id, &guard).ok_or(TpcError::TxnNotFound)?;
        
        if node.inner.set_tm_state(TmState::Init, TmState::Preparing) {
            Ok(())
        } else {
            Err(TpcError::InvalidState)
        }
    }

    pub fn rm_prepare(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let guard = epoch::pin();
        let node = self.find_txn(txn_id, &guard).ok_or(TpcError::TxnNotFound)?;
        
        if node.inner.set_rm_state(rm_id, RmState::Working, RmState::Prepared) {
            node.inner.prepared_count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        } else {
            let current_state = node.inner.get_rm_state(rm_id).ok_or(TpcError::RmNotFound)?;
            if current_state != RmState::Working {
                Err(TpcError::InvalidState)
            } else {
                Err(TpcError::RmNotFound)
            }
        }
    }

    pub fn rm_choose_to_abort(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let guard = epoch::pin();
        let node = self.find_txn(txn_id, &guard).ok_or(TpcError::TxnNotFound)?;
        
        if node.inner.set_rm_state(rm_id, RmState::Working, RmState::Aborted) {
            Ok(())
        } else {
            let current_state = node.inner.get_rm_state(rm_id).ok_or(TpcError::RmNotFound)?;
            if current_state != RmState::Working {
                Err(TpcError::InvalidState)
            } else {
                Err(TpcError::RmNotFound)
            }
        }
    }

    pub fn tm_commit(&self, txn_id: TxnId) -> Result<(), TpcError> {
        let guard = epoch::pin();
        let node = self.find_txn(txn_id, &guard).ok_or(TpcError::TxnNotFound)?;
        
        if node.inner.get_tm_state() != TmState::Preparing {
            return Err(TpcError::InvalidState);
        }
        
        let prepared_count = node.inner.prepared_count.load(Ordering::Acquire);
        if prepared_count != node.inner.total_rms {
            return Err(TpcError::NotAllPrepared);
        }
        
        if node.inner.set_tm_state(TmState::Preparing, TmState::Committed) {
            Ok(())
        } else {
            Err(TpcError::InvalidState)
        }
    }

    pub fn tm_abort(&self, txn_id: TxnId) -> Result<(), TpcError> {
        let guard = epoch::pin();
        let node = self.find_txn(txn_id, &guard).ok_or(TpcError::TxnNotFound)?;
        
        let current_state = node.inner.get_tm_state();
        if current_state == TmState::Committed || current_state == TmState::Aborted {
            return Err(TpcError::AlreadyDecided);
        }
        
        // Try to abort from any valid state
        match current_state {
            TmState::Init => {
                if node.inner.set_tm_state(TmState::Init, TmState::Aborted) {
                    Ok(())
                } else {
                    Err(TpcError::InvalidState)
                }
            }
            TmState::Preparing => {
                if node.inner.set_tm_state(TmState::Preparing, TmState::Aborted) {
                    Ok(())
                } else {
                    Err(TpcError::InvalidState)
                }
            }
            _ => Err(TpcError::InvalidState)
        }
    }

    pub fn rm_rcv_commit(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let guard = epoch::pin();
        let node = self.find_txn(txn_id, &guard).ok_or(TpcError::TxnNotFound)?;
        
        if node.inner.get_tm_state() != TmState::Committed {
            return Err(TpcError::InvalidState);
        }
        
        if node.inner.set_rm_state(rm_id, RmState::Prepared, RmState::Committed) {
            Ok(())
        } else {
            Err(TpcError::RmNotFound)
        }
    }

    pub fn rm_rcv_abort(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError> {
        let guard = epoch::pin();
        let node = self.find_txn(txn_id, &guard).ok_or(TpcError::TxnNotFound)?;
        
        if node.inner.get_tm_state() != TmState::Aborted {
            return Err(TpcError::InvalidState);
        }
        
        let current_state = node.inner.get_rm_state(rm_id).ok_or(TpcError::RmNotFound)?;
        
        match current_state {
            RmState::Working => {
                node.inner.set_rm_state(rm_id, RmState::Working, RmState::Aborted);
            }
            RmState::Prepared => {
                node.inner.set_rm_state(rm_id, RmState::Prepared, RmState::Aborted);
            }
            RmState::Aborted => {
                // Already aborted, that's fine
            }
            _ => return Err(TpcError::InvalidState),
        }
        
        Ok(())
    }

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

    pub fn run_abort(&self, txn_id: TxnId, aborting_rm: RmId) -> Result<TmDecision, TpcError> {
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

        // Phase 2: Abort
        self.tm_abort(txn_id)?;

        // Deliver abort to all RMs
        for &rm_id in &rm_ids {
            let state = self.get_rm_state(txn_id, rm_id);
            if state != Some(RmState::Aborted) {
                self.rm_rcv_abort(txn_id, rm_id)?;
            }
        }

        Ok(TmDecision::Abort)
    }
}