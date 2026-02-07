/// Trait specification tests for TwoPhaseCommit
///
/// The evolved code must provide:
///   pub type RmId = u64;
///   pub type TxnId = u64;
///   pub enum RmState { Working, Prepared, Committed, Aborted }
///   pub enum TmState { Init, Preparing, Committed, Aborted }
///   pub enum TpcError { RmNotFound, TxnNotFound, InvalidState, AlreadyDecided, NotAllPrepared }
///   pub enum TmDecision { Commit, Abort }
///   pub struct TwoPhaseCommit { ... }
///   impl TwoPhaseCommit {
///       pub fn new(rm_ids: &[RmId]) -> Self;
///       pub fn rm_count(&self) -> usize;
///       pub fn begin(&self) -> TxnId;
///       pub fn get_tm_state(&self, txn_id: TxnId) -> Option<TmState>;
///       pub fn get_rm_state(&self, txn_id: TxnId, rm_id: RmId) -> Option<RmState>;
///       pub fn tm_prepare(&self, txn_id: TxnId) -> Result<(), TpcError>;
///       pub fn rm_prepare(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError>;
///       pub fn rm_choose_to_abort(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError>;
///       pub fn tm_commit(&self, txn_id: TxnId) -> Result<(), TpcError>;
///       pub fn tm_abort(&self, txn_id: TxnId) -> Result<(), TpcError>;
///       pub fn rm_rcv_commit(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError>;
///       pub fn rm_rcv_abort(&self, txn_id: TxnId, rm_id: RmId) -> Result<(), TpcError>;
///       pub fn run_commit(&self, txn_id: TxnId) -> Result<TmDecision, TpcError>;
///       pub fn run_abort(&self, txn_id: TxnId, aborting_rm: RmId) -> Result<TmDecision, TpcError>;
///   }
///
/// All operations must be safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_initial_state() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();

        assert_eq!(tpc.get_tm_state(txn), Some(TmState::Init));
        for rm_id in 1..=3 {
            assert_eq!(
                tpc.get_rm_state(txn, rm_id),
                Some(RmState::Working),
                "All RMs must start in Working state"
            );
        }
    }

    #[test]
    fn test_successful_commit() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();

        let decision = tpc.run_commit(txn).unwrap();
        assert_eq!(decision, TmDecision::Commit);
        assert_eq!(tpc.get_tm_state(txn), Some(TmState::Committed));

        for rm_id in 1..=3 {
            assert_eq!(
                tpc.get_rm_state(txn, rm_id),
                Some(RmState::Committed),
                "All RMs must be Committed after successful 2PC"
            );
        }
    }

    #[test]
    fn test_abort_on_rm_failure() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();

        let decision = tpc.run_abort(txn, 2).unwrap();
        assert_eq!(decision, TmDecision::Abort);
        assert_eq!(tpc.get_tm_state(txn), Some(TmState::Aborted));

        for rm_id in 1..=3 {
            assert_eq!(
                tpc.get_rm_state(txn, rm_id),
                Some(RmState::Aborted),
                "All RMs must be Aborted when any RM votes no"
            );
        }
    }

    #[test]
    fn test_atomicity_no_mixed_states() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3, 4]);

        // Successful commit
        let txn1 = tpc.begin();
        tpc.run_commit(txn1).unwrap();

        let has_committed = (1..=4).any(|rm| tpc.get_rm_state(txn1, rm) == Some(RmState::Committed));
        let has_aborted = (1..=4).any(|rm| tpc.get_rm_state(txn1, rm) == Some(RmState::Aborted));
        assert!(
            !(has_committed && has_aborted),
            "Atomicity: cannot have both Committed and Aborted RMs"
        );

        // Aborted transaction
        let txn2 = tpc.begin();
        tpc.run_abort(txn2, 3).unwrap();

        let has_committed = (1..=4).any(|rm| tpc.get_rm_state(txn2, rm) == Some(RmState::Committed));
        let has_aborted = (1..=4).any(|rm| tpc.get_rm_state(txn2, rm) == Some(RmState::Aborted));
        assert!(
            !(has_committed && has_aborted),
            "Atomicity: cannot have both Committed and Aborted RMs"
        );
    }

    #[test]
    fn test_validity_commit_only_if_all_prepared() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();

        tpc.tm_prepare(txn).unwrap();

        // Only RM 1 and RM 2 prepare (not RM 3)
        tpc.rm_prepare(txn, 1).unwrap();
        tpc.rm_prepare(txn, 2).unwrap();

        // TM should not be able to commit
        let result = tpc.tm_commit(txn);
        assert!(
            result.is_err(),
            "Validity: TM must not commit unless all RMs are prepared"
        );
    }

    #[test]
    fn test_consistency_committed_rm_requires_tm_commit() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();

        tpc.tm_prepare(txn).unwrap();
        tpc.rm_prepare(txn, 1).unwrap();

        // RM 1 should not be able to receive commit without TM deciding
        let result = tpc.rm_rcv_commit(txn, 1);
        assert!(
            result.is_err(),
            "Consistency: RM cannot commit without TM decision"
        );
    }

    #[test]
    fn test_agreement_irrevocable_decision() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();

        // Commit the transaction
        tpc.run_commit(txn).unwrap();
        assert_eq!(tpc.get_tm_state(txn), Some(TmState::Committed));

        // TM should not be able to abort after committing
        let result = tpc.tm_abort(txn);
        assert!(
            result.is_err(),
            "Agreement: decision must be irrevocable after commit"
        );
    }

    #[test]
    fn test_prepare_before_commit_ordering() {
        let tpc = TwoPhaseCommit::new(&[1, 2]);
        let txn = tpc.begin();

        // Cannot commit without preparing first
        let result = tpc.tm_commit(txn);
        assert!(
            result.is_err(),
            "Must prepare before commit"
        );
    }

    #[test]
    fn test_rm_abort_before_prepare() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();

        tpc.tm_prepare(txn).unwrap();

        // RM 2 aborts before preparing
        tpc.rm_choose_to_abort(txn, 2).unwrap();

        // RM 2 should be Aborted
        assert_eq!(tpc.get_rm_state(txn, 2), Some(RmState::Aborted));

        // Other RMs prepare normally
        tpc.rm_prepare(txn, 1).unwrap();
        tpc.rm_prepare(txn, 3).unwrap();

        // TM should not be able to commit
        let result = tpc.tm_commit(txn);
        assert!(
            result.is_err(),
            "TM cannot commit when an RM has aborted"
        );
    }

    #[test]
    fn test_cannot_prepare_after_abort() {
        let tpc = TwoPhaseCommit::new(&[1, 2]);
        let txn = tpc.begin();

        tpc.tm_prepare(txn).unwrap();
        tpc.rm_choose_to_abort(txn, 1).unwrap();

        // RM 1 cannot then prepare
        let result = tpc.rm_prepare(txn, 1);
        assert!(
            result.is_err(),
            "RM that aborted cannot subsequently prepare"
        );
    }

    #[test]
    fn test_multiple_independent_transactions() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);

        let txn1 = tpc.begin();
        let txn2 = tpc.begin();
        let txn3 = tpc.begin();

        assert_ne!(txn1, txn2);
        assert_ne!(txn2, txn3);

        tpc.run_commit(txn1).unwrap();
        tpc.run_abort(txn2, 1).unwrap();
        tpc.run_commit(txn3).unwrap();

        assert_eq!(tpc.get_tm_state(txn1), Some(TmState::Committed));
        assert_eq!(tpc.get_tm_state(txn2), Some(TmState::Aborted));
        assert_eq!(tpc.get_tm_state(txn3), Some(TmState::Committed));
    }

    #[test]
    fn test_concurrent_transactions_atomicity() {
        const NUM_THREADS: usize = 8;
        const TXNS_PER_THREAD: usize = 50;

        let tpc = Arc::new(TwoPhaseCommit::new(&[1, 2, 3, 4]));
        let committed_count = Arc::new(AtomicUsize::new(0));
        let aborted_count = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let tpc = Arc::clone(&tpc);
                let committed_count = Arc::clone(&committed_count);
                let aborted_count = Arc::clone(&aborted_count);
                s.spawn(move || {
                    for i in 0..TXNS_PER_THREAD {
                        let txn = tpc.begin();
                        if (t + i) % 3 == 0 {
                            // Some transactions abort
                            if tpc.run_abort(txn, 2).is_ok() {
                                aborted_count.fetch_add(1, Ordering::Relaxed);
                            }
                        } else {
                            // Most transactions commit
                            if tpc.run_commit(txn).is_ok() {
                                committed_count.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });

        let total = committed_count.load(Ordering::Relaxed)
            + aborted_count.load(Ordering::Relaxed);
        assert_eq!(
            total,
            NUM_THREADS * TXNS_PER_THREAD,
            "Every transaction must either commit or abort"
        );
    }

    #[test]
    fn test_concurrent_rm_votes_on_same_txn() {
        let tpc = Arc::new(TwoPhaseCommit::new(&[1, 2, 3, 4]));
        let txn = tpc.begin();
        tpc.tm_prepare(txn).unwrap();

        // All RMs vote concurrently
        std::thread::scope(|s| {
            for rm_id in 1..=4 {
                let tpc = Arc::clone(&tpc);
                s.spawn(move || {
                    tpc.rm_prepare(txn, rm_id).unwrap();
                });
            }
        });

        // All should be prepared
        for rm_id in 1..=4 {
            assert_eq!(
                tpc.get_rm_state(txn, rm_id),
                Some(RmState::Prepared),
                "All RMs must reach Prepared after concurrent votes"
            );
        }

        // Now TM can commit
        tpc.tm_commit(txn).unwrap();
        assert_eq!(tpc.get_tm_state(txn), Some(TmState::Committed));
    }

    #[test]
    fn test_txn_not_found() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        assert!(tpc.get_tm_state(999).is_none());
        assert!(tpc.get_rm_state(999, 1).is_none());
        assert!(tpc.tm_prepare(999).is_err());
    }

    #[test]
    fn test_rm_not_found() {
        let tpc = TwoPhaseCommit::new(&[1, 2, 3]);
        let txn = tpc.begin();
        tpc.tm_prepare(txn).unwrap();
        assert!(tpc.rm_prepare(txn, 99).is_err());
        assert!(tpc.get_rm_state(txn, 99).is_none());
    }
}
