/// Cross-Shard SSI - Initial Seed (Mutex-based blocking implementation)
/// ShinkaEvolve will evolve this from Blocking -> LockFree -> WaitFree
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

pub type TxnId = u64;

#[derive(Debug, PartialEq)]
pub enum SsiError {
    WriteConflict,
    ReadConflict,
    ShardOutOfRange,
    TxnNotFound,
}

struct TxnState {
    reads: Vec<(usize, u64)>,       // (shard, key)
    writes: Vec<(usize, u64, u64)>, // (shard, key, value)
    snapshot_versions: Vec<u64>,
}

struct Shard {
    data: HashMap<u64, u64>,
    version: u64,
}

pub struct CrossShardSsi {
    shards: Vec<Mutex<Shard>>,
    txns: Mutex<HashMap<TxnId, TxnState>>,
    next_txn_id: AtomicU64,
}

impl CrossShardSsi {
    pub fn new(num_shards: usize) -> Self {
        debug_assert!(num_shards > 0, "Must have at least one shard");
        let shards = (0..num_shards)
            .map(|_| {
                Mutex::new(Shard {
                    data: HashMap::new(),
                    version: 0,
                })
            })
            .collect();
        CrossShardSsi {
            shards,
            txns: Mutex::new(HashMap::new()),
            next_txn_id: AtomicU64::new(1),
        }
    }

    pub fn begin_txn(&self) -> TxnId {
        let id = self.next_txn_id.fetch_add(1, Ordering::Relaxed);
        let snapshot_versions: Vec<u64> = self
            .shards
            .iter()
            .map(|s| s.lock().unwrap().version)
            .collect();
        let state = TxnState {
            reads: Vec::new(),
            writes: Vec::new(),
            snapshot_versions,
        };
        self.txns.lock().unwrap().insert(id, state);
        id
    }

    pub fn read(&self, txn: TxnId, shard: usize, key: u64) -> Option<u64> {
        debug_assert!(shard < self.shards.len(), "Shard index out of range");
        let value = {
            let guard = self.shards[shard].lock().unwrap();
            guard.data.get(&key).copied()
        };
        if let Some(state) = self.txns.lock().unwrap().get_mut(&txn) {
            state.reads.push((shard, key));
        }
        value
    }

    pub fn write(&self, txn: TxnId, shard: usize, key: u64, value: u64) {
        debug_assert!(shard < self.shards.len(), "Shard index out of range");
        if let Some(state) = self.txns.lock().unwrap().get_mut(&txn) {
            state.writes.push((shard, key, value));
        }
    }

    pub fn commit(&self, txn: TxnId) -> Result<(), SsiError> {
        let state = {
            let mut txns = self.txns.lock().unwrap();
            match txns.remove(&txn) {
                Some(s) => s,
                None => return Err(SsiError::TxnNotFound),
            }
        };

        // Collect involved shards
        let mut involved: Vec<usize> = state
            .reads
            .iter()
            .map(|(s, _)| *s)
            .chain(state.writes.iter().map(|(s, _, _)| *s))
            .collect();
        involved.sort();
        involved.dedup();

        // Lock in order
        let mut guards: Vec<_> = involved
            .iter()
            .map(|&idx| (idx, self.shards[idx].lock().unwrap()))
            .collect();

        // SSI validation: check versions
        for &(idx, ref guard) in &guards {
            if guard.version != state.snapshot_versions[idx] {
                return Err(SsiError::WriteConflict);
            }
        }

        // Apply writes
        for (idx, guard) in &mut guards {
            let has_writes = state.writes.iter().any(|(s, _, _)| s == idx);
            if has_writes {
                for &(s, key, value) in &state.writes {
                    if s == *idx {
                        guard.data.insert(key, value);
                    }
                }
                guard.version += 1;
            }
        }

        Ok(())
    }

    pub fn abort(&self, txn: TxnId) {
        self.txns.lock().unwrap().remove(&txn);
    }
}
