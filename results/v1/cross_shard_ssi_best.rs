use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crossbeam_epoch::{self as epoch, Atomic, Owned};
use std::ptr;

pub type TxnId = u64;

#[derive(Debug, PartialEq)]
pub enum SsiError {
    WriteConflict,
    ReadConflict,
    ShardOutOfRange,
    TxnNotFound,
}

#[derive(Clone)]
struct TxnState {
    reads: Vec<(usize, u64)>,
    writes: Vec<(usize, u64, u64)>,
    snapshot_versions: Vec<u64>,
}

struct ShardData {
    data: HashMap<u64, u64>,
    version: u64,
}

struct TxnNode {
    id: TxnId,
    state: TxnState,
    next: Atomic<TxnNode>,
}

pub struct CrossShardSsi {
    shards: Vec<Atomic<ShardData>>,
    txn_head: Atomic<TxnNode>,
    next_txn_id: AtomicU64,
    num_shards: AtomicUsize,
}

impl CrossShardSsi {
    pub fn new(num_shards: usize) -> Self {
        debug_assert!(num_shards > 0, "Must have at least one shard");
        let shards = (0..num_shards)
            .map(|_| {
                Atomic::new(ShardData {
                    data: HashMap::new(),
                    version: 0,
                })
            })
            .collect();
        
        CrossShardSsi {
            shards,
            txn_head: Atomic::null(),
            next_txn_id: AtomicU64::new(1),
            num_shards: AtomicUsize::new(num_shards),
        }
    }

    pub fn begin_txn(&self) -> TxnId {
        let id = self.next_txn_id.fetch_add(1, Ordering::Relaxed);
        let guard = &epoch::pin();
        
        let snapshot_versions: Vec<u64> = self.shards
            .iter()
            .map(|s| {
                let shard = s.load(Ordering::Acquire, guard);
                if shard.is_null() {
                    0
                } else {
                    unsafe { shard.deref().version }
                }
            })
            .collect();
        
        let state = TxnState {
            reads: Vec::new(),
            writes: Vec::new(),
            snapshot_versions,
        };
        
        let new_node = Owned::new(TxnNode {
            id,
            state,
            next: Atomic::null(),
        }).into_shared(guard);
        
        loop {
            let head = self.txn_head.load(Ordering::Acquire, guard);
            unsafe { new_node.deref().next.store(head, Ordering::Release); }
            
            match self.txn_head.compare_exchange(
                head,
                new_node,
                Ordering::Release,
                Ordering::Acquire,
                guard,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }
        
        id
    }

    pub fn read(&self, txn: TxnId, shard: usize, key: u64) -> Option<u64> {
        debug_assert!(shard < self.num_shards.load(Ordering::Relaxed), "Shard index out of range");
        let guard = &epoch::pin();
        
        let value = {
            let shard_data = self.shards[shard].load(Ordering::Acquire, guard);
            if shard_data.is_null() {
                None
            } else {
                unsafe { shard_data.deref().data.get(&key).copied() }
            }
        };
        
        self.update_txn_state(txn, |state| {
            state.reads.push((shard, key));
        }, guard);
        
        value
    }

    pub fn write(&self, txn: TxnId, shard: usize, key: u64, value: u64) {
        debug_assert!(shard < self.num_shards.load(Ordering::Relaxed), "Shard index out of range");
        let guard = &epoch::pin();
        
        self.update_txn_state(txn, |state| {
            state.writes.push((shard, key, value));
        }, guard);
    }

    pub fn commit(&self, txn: TxnId) -> Result<(), SsiError> {
        let guard = &epoch::pin();
        
        let state = match self.remove_txn(txn, guard) {
            Some(s) => s,
            None => return Err(SsiError::TxnNotFound),
        };
        
        let mut involved: Vec<usize> = state.reads.iter()
            .map(|(s, _)| *s)
            .chain(state.writes.iter().map(|(s, _, _)| *s))
            .collect();
        involved.sort();
        involved.dedup();
        
        // Try to atomically update all shards
        for &shard_idx in &involved {
            let old_shard = self.shards[shard_idx].load(Ordering::Acquire, guard);
            if old_shard.is_null() {
                continue;
            }
            
            let old_version = unsafe { old_shard.deref().version };
            if old_version != state.snapshot_versions[shard_idx] {
                return Err(SsiError::WriteConflict);
            }
        }
        
        // Apply writes
        for &shard_idx in &involved {
            let has_writes = state.writes.iter().any(|(s, _, _)| *s == shard_idx);
            if !has_writes {
                continue;
            }
            
            loop {
                let old_shard = self.shards[shard_idx].load(Ordering::Acquire, guard);
                if old_shard.is_null() {
                    break;
                }
                
                let mut new_data = unsafe { old_shard.deref().data.clone() };
                let old_version = unsafe { old_shard.deref().version };
                
                if old_version != state.snapshot_versions[shard_idx] {
                    return Err(SsiError::WriteConflict);
                }
                
                for &(s, key, value) in &state.writes {
                    if s == shard_idx {
                        new_data.insert(key, value);
                    }
                }
                
                let new_shard = Owned::new(ShardData {
                    data: new_data,
                    version: old_version + 1,
                });
                
                match self.shards[shard_idx].compare_exchange(
                    old_shard,
                    new_shard,
                    Ordering::Release,
                    Ordering::Acquire,
                    guard,
                ) {
                    Ok(_) => {
                        unsafe { guard.defer_destroy(old_shard) };
                        break;
                    }
                    Err(_) => continue,
                }
            }
        }
        
        Ok(())
    }

    pub fn abort(&self, txn: TxnId) {
        let guard = &epoch::pin();
        self.remove_txn(txn, guard);
    }
    
    fn update_txn_state<F>(&self, txn_id: TxnId, f: F, guard: &epoch::Guard) 
    where F: FnOnce(&mut TxnState) 
    {
        let mut current = self.txn_head.load(Ordering::Acquire, guard);
        
        while !current.is_null() {
            let node = unsafe { current.deref() };
            if node.id == txn_id {
                // Found it - create new state
                let mut new_state = node.state.clone();
                f(&mut new_state);
                
                // Try to update atomically
                let new_node = Owned::new(TxnNode {
                    id: txn_id,
                    state: new_state,
                    next: Atomic::null(),
                }).into_shared(guard);
                
                // Re-insert at head
                loop {
                    let head = self.txn_head.load(Ordering::Acquire, guard);
                    unsafe { new_node.deref().next.store(head, Ordering::Release); }
                    
                    match self.txn_head.compare_exchange(
                        head,
                        new_node,
                        Ordering::Release,
                        Ordering::Acquire,
                        guard,
                    ) {
                        Ok(_) => return,
                        Err(_) => continue,
                    }
                }
            }
            current = node.next.load(Ordering::Acquire, guard);
        }
    }
    
    fn remove_txn(&self, txn_id: TxnId, guard: &epoch::Guard) -> Option<TxnState> {
        let mut prev: *const TxnNode = ptr::null();
        let mut current = self.txn_head.load(Ordering::Acquire, guard);
        
        while !current.is_null() {
            let node = unsafe { current.deref() };
            if node.id == txn_id {
                let state = node.state.clone();
                let next = node.next.load(Ordering::Acquire, guard);
                
                if prev.is_null() {
                    // Remove from head
                    match self.txn_head.compare_exchange(
                        current,
                        next,
                        Ordering::Release,
                        Ordering::Acquire,
                        guard,
                    ) {
                        Ok(_) => {
                            unsafe { guard.defer_destroy(current) };
                            return Some(state);
                        }
                        Err(_) => return self.remove_txn(txn_id, guard),
                    }
                } else {
                    // Remove from middle
                    unsafe {
                        let prev_node = &*prev as &TxnNode;
                        match prev_node.next.compare_exchange(
                            current,
                            next,
                            Ordering::Release,
                            Ordering::Acquire,
                            guard,
                        ) {
                            Ok(_) => {
                                guard.defer_destroy(current);
                                return Some(state);
                            }
                            Err(_) => return self.remove_txn(txn_id, guard),
                        }
                    }
                }
            }
            prev = current.as_raw();
            current = node.next.load(Ordering::Acquire, guard);
        }
        
        None
    }
}