/// Trait specification tests for CrossShardSsi
///
/// The evolved code must provide:
///   pub type TxnId = u64;  // or newtype wrapper
///   pub enum SsiError { WriteConflict, ReadConflict, ShardOutOfRange, TxnNotFound }
///   pub struct CrossShardSsi { ... }
///   impl CrossShardSsi {
///       pub fn new(num_shards: usize) -> Self;
///       pub fn begin_txn(&self) -> TxnId;
///       pub fn read(&self, txn: TxnId, shard: usize, key: u64) -> Option<u64>;
///       pub fn write(&self, txn: TxnId, shard: usize, key: u64, value: u64);
///       pub fn commit(&self, txn: TxnId) -> Result<(), SsiError>;
///       pub fn abort(&self, txn: TxnId);
///   }
///
/// Serializable snapshot isolation across shards. All operations safe for concurrent access.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn test_basic_txn_lifecycle() {
        let ssi = CrossShardSsi::new(4);
        let txn = ssi.begin_txn();

        ssi.write(txn, 0, 1, 100);
        ssi.write(txn, 1, 2, 200);

        let result = ssi.commit(txn);
        assert!(result.is_ok(), "Single txn commit must succeed");
    }

    #[test]
    fn test_read_own_writes() {
        let ssi = CrossShardSsi::new(4);

        // First txn: write and commit
        let txn1 = ssi.begin_txn();
        ssi.write(txn1, 0, 1, 100);
        ssi.commit(txn1).unwrap();

        // Second txn: should see committed data
        let txn2 = ssi.begin_txn();
        let val = ssi.read(txn2, 0, 1);
        assert_eq!(val, Some(100));
        ssi.commit(txn2).unwrap();
    }

    #[test]
    fn test_read_uncommitted_not_visible() {
        let ssi = CrossShardSsi::new(4);

        let txn1 = ssi.begin_txn();
        ssi.write(txn1, 0, 1, 100);
        // txn1 NOT committed

        let txn2 = ssi.begin_txn();
        let val = ssi.read(txn2, 0, 1);
        assert_eq!(
            val, None,
            "Uncommitted writes must not be visible to other txns"
        );

        ssi.abort(txn1);
        ssi.abort(txn2);
    }

    #[test]
    fn test_abort_discards_writes() {
        let ssi = CrossShardSsi::new(4);

        let txn1 = ssi.begin_txn();
        ssi.write(txn1, 0, 1, 999);
        ssi.abort(txn1);

        let txn2 = ssi.begin_txn();
        let val = ssi.read(txn2, 0, 1);
        assert_eq!(val, None, "Aborted writes must be discarded");
        ssi.commit(txn2).unwrap();
    }

    #[test]
    fn test_cross_shard_atomicity() {
        let ssi = CrossShardSsi::new(4);

        // Write to multiple shards atomically
        let txn1 = ssi.begin_txn();
        ssi.write(txn1, 0, 1, 10);
        ssi.write(txn1, 1, 1, 20);
        ssi.write(txn1, 2, 1, 30);
        ssi.commit(txn1).unwrap();

        // Reader sees all-or-nothing
        let txn2 = ssi.begin_txn();
        assert_eq!(ssi.read(txn2, 0, 1), Some(10));
        assert_eq!(ssi.read(txn2, 1, 1), Some(20));
        assert_eq!(ssi.read(txn2, 2, 1), Some(30));
        ssi.commit(txn2).unwrap();
    }

    #[test]
    fn test_write_write_conflict() {
        let ssi = CrossShardSsi::new(4);

        // Setup: initial value
        let setup = ssi.begin_txn();
        ssi.write(setup, 0, 1, 100);
        ssi.commit(setup).unwrap();

        // Two concurrent txns write to same key
        let txn1 = ssi.begin_txn();
        let txn2 = ssi.begin_txn();

        ssi.read(txn1, 0, 1);
        ssi.write(txn1, 0, 1, 200);

        ssi.read(txn2, 0, 1);
        ssi.write(txn2, 0, 1, 300);

        // First to commit wins
        let result1 = ssi.commit(txn1);
        let result2 = ssi.commit(txn2);

        // At least one must fail (SSI)
        assert!(
            result1.is_err() || result2.is_err(),
            "Write-write conflict: at least one txn must abort"
        );

        // At least one must succeed
        assert!(
            result1.is_ok() || result2.is_ok(),
            "At least one conflicting txn should commit"
        );
    }

    #[test]
    fn test_concurrent_non_conflicting_txns() {
        const NUM_THREADS: usize = 8;
        const OPS_PER_THREAD: usize = 100;

        let ssi = Arc::new(CrossShardSsi::new(NUM_THREADS));
        let committed = Arc::new(AtomicUsize::new(0));

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let ssi = Arc::clone(&ssi);
                let committed = Arc::clone(&committed);
                s.spawn(move || {
                    for i in 0..OPS_PER_THREAD {
                        let txn = ssi.begin_txn();
                        // Each thread writes to its own shard with unique keys
                        ssi.write(txn, t, i as u64, (t * OPS_PER_THREAD + i) as u64);
                        if ssi.commit(txn).is_ok() {
                            committed.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });

        // Non-conflicting txns should all commit
        assert_eq!(
            committed.load(Ordering::Relaxed),
            NUM_THREADS * OPS_PER_THREAD,
            "Non-conflicting txns on separate shards must all commit"
        );
    }

    #[test]
    fn test_concurrent_conflicting_txns() {
        const NUM_THREADS: usize = 8;
        const ROUNDS: usize = 50;

        let ssi = Arc::new(CrossShardSsi::new(2));
        let total_committed = Arc::new(AtomicUsize::new(0));
        let total_aborted = Arc::new(AtomicUsize::new(0));

        // Setup initial value
        let setup = ssi.begin_txn();
        ssi.write(setup, 0, 0, 0);
        ssi.commit(setup).unwrap();

        std::thread::scope(|s| {
            for t in 0..NUM_THREADS {
                let ssi = Arc::clone(&ssi);
                let total_committed = Arc::clone(&total_committed);
                let total_aborted = Arc::clone(&total_aborted);
                s.spawn(move || {
                    for r in 0..ROUNDS {
                        let txn = ssi.begin_txn();
                        // All threads read-then-write same key = guaranteed conflicts
                        let _val = ssi.read(txn, 0, 0);
                        ssi.write(txn, 0, 0, (t * ROUNDS + r) as u64);
                        match ssi.commit(txn) {
                            Ok(()) => {
                                total_committed.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(_) => {
                                total_aborted.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                });
            }
        });

        let committed = total_committed.load(Ordering::Relaxed);
        let aborted = total_aborted.load(Ordering::Relaxed);

        assert_eq!(
            committed + aborted,
            NUM_THREADS * ROUNDS,
            "Every txn must either commit or abort"
        );
        assert!(
            aborted > 0,
            "With {} threads on same key, some txns must abort (got 0 aborts)",
            NUM_THREADS
        );
        assert!(
            committed > 0,
            "At least some txns must succeed"
        );
    }
}
