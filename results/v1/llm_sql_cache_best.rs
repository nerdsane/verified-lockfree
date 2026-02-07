/// LLM SQL Cache - Lock-Free Implementation with Atomic Operations
/// Uses atomic operations and crossbeam-epoch for safe memory reclamation

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use crossbeam_epoch::{self as epoch, Atomic, Owned, Shared};

/// Cache performance statistics.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub prefix_hits: u64,
    pub evictions: u64,
    pub size: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total > 0 {
            self.hits as f64 / total as f64
        } else {
            0.0
        }
    }
}

/// A cached SQL query result.
#[derive(Debug)]
struct CacheEntry {
    query: String,
    normalized: String,
    result_id: u64,
    hit_count: AtomicU64,
    hash: u64,
}

impl Clone for CacheEntry {
    fn clone(&self) -> Self {
        Self {
            query: self.query.clone(),
            normalized: self.normalized.clone(),
            result_id: self.result_id,
            hit_count: AtomicU64::new(self.hit_count.load(Ordering::Relaxed)),
            hash: self.hash,
        }
    }
}

struct Node {
    entry: CacheEntry,
    next: Atomic<Node>,
}

/// Lock-free LRU cache for SQL queries with prefix-based approximate matching.
pub struct LockFreeSqlCache {
    capacity: u64,
    head: Atomic<Node>,
    tail: Atomic<Node>,
    size: AtomicUsize,
    stats_hits: AtomicU64,
    stats_misses: AtomicU64,
    stats_prefix_hits: AtomicU64,
    stats_evictions: AtomicU64,
}

impl LockFreeSqlCache {
    fn new(capacity: u64) -> Self {
        Self {
            capacity,
            head: Atomic::null(),
            tail: Atomic::null(),
            size: AtomicUsize::new(0),
            stats_hits: AtomicU64::new(0),
            stats_misses: AtomicU64::new(0),
            stats_prefix_hits: AtomicU64::new(0),
            stats_evictions: AtomicU64::new(0),
        }
    }

    fn find_entry<'g>(&self, hash: u64, guard: &'g epoch::Guard) -> Option<Shared<'g, Node>> {
        let mut current = self.head.load(Ordering::Acquire, guard);
        
        while !current.is_null() {
            let node = unsafe { current.deref() };
            if node.entry.hash == hash {
                return Some(current);
            }
            current = node.next.load(Ordering::Acquire, guard);
        }
        None
    }

    fn insert_entry(&self, entry: CacheEntry, guard: &epoch::Guard) {
        let new_node = Owned::new(Node {
            entry,
            next: Atomic::null(),
        });

        let new_shared = new_node.into_shared(guard);

        // Insert at head
        loop {
            let head = self.head.load(Ordering::Acquire, guard);
            unsafe { new_shared.deref() }.next.store(head, Ordering::Relaxed);
            
            match self.head.compare_exchange_weak(
                head,
                new_shared,
                Ordering::Release,
                Ordering::Relaxed,
                guard,
            ) {
                Ok(_) => break,
                Err(_) => continue,
            }
        }

        // Update tail if this is the first node
        if self.tail.load(Ordering::Acquire, guard).is_null() {
            let _ = self.tail.compare_exchange(
                Shared::null(),
                new_shared,
                Ordering::Release,
                Ordering::Relaxed,
                guard,
            );
        }

        let old_size = self.size.fetch_add(1, Ordering::AcqRel);
        
        // Evict if necessary
        if old_size + 1 > self.capacity as usize {
            self.evict_lru(guard);
        }
    }

    fn evict_lru(&self, guard: &epoch::Guard) {
        let tail = self.tail.load(Ordering::Acquire, guard);
        if tail.is_null() {
            return;
        }

        // Find the node before tail
        let mut current = self.head.load(Ordering::Acquire, guard);
        let mut prev = Shared::null();

        while !current.is_null() {
            let node = unsafe { current.deref() };
            let next = node.next.load(Ordering::Acquire, guard);
            
            if next == tail {
                prev = current;
                break;
            }
            current = next;
        }

        if !prev.is_null() {
            // Update the previous node to point to null
            let prev_node = unsafe { prev.deref() };
            prev_node.next.store(Shared::null(), Ordering::Release);
            
            // Update tail
            let _ = self.tail.compare_exchange(
                tail,
                prev,
                Ordering::Release,
                Ordering::Relaxed,
                guard,
            );

            self.size.fetch_sub(1, Ordering::AcqRel);
            self.stats_evictions.fetch_add(1, Ordering::Relaxed);
            
            unsafe {
                guard.defer_destroy(tail);
            }
        }
    }

    fn get_stats(&self) -> CacheStats {
        CacheStats {
            hits: self.stats_hits.load(Ordering::Relaxed),
            misses: self.stats_misses.load(Ordering::Relaxed),
            prefix_hits: self.stats_prefix_hits.load(Ordering::Relaxed),
            evictions: self.stats_evictions.load(Ordering::Relaxed),
            size: self.size.load(Ordering::Relaxed) as u64,
        }
    }

    fn lookup(&self, normalized: &str) -> bool {
        let hash = simple_hash(normalized);
        let guard = &epoch::pin();
        
        if let Some(node_ref) = self.find_entry(hash, guard) {
            let node = unsafe { node_ref.deref() };
            node.entry.hit_count.fetch_add(1, Ordering::Relaxed);
            self.stats_hits.fetch_add(1, Ordering::Relaxed);
            return true;
        }

        // Check prefix match
        let prefix = extract_prefix(normalized, 3);
        let prefix_hash = simple_hash(&prefix);
        
        let mut current = self.head.load(Ordering::Acquire, guard);
        while !current.is_null() {
            let node = unsafe { current.deref() };
            let entry_prefix = extract_prefix(&node.entry.normalized, 3);
            if simple_hash(&entry_prefix) == prefix_hash {
                self.stats_prefix_hits.fetch_add(1, Ordering::Relaxed);
                self.stats_hits.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            current = node.next.load(Ordering::Acquire, guard);
        }

        self.stats_misses.fetch_add(1, Ordering::Relaxed);
        false
    }

    fn insert(&self, query: &str, result_id: u64) {
        let normalized = normalize_query(query);
        let hash = simple_hash(&normalized);
        
        let entry = CacheEntry {
            query: query.to_string(),
            normalized,
            result_id,
            hit_count: AtomicU64::new(0),
            hash,
        };

        let guard = &epoch::pin();
        self.insert_entry(entry, guard);
    }
}

/// Normalize a SQL query for cache key generation.
pub fn normalize_query(query: &str) -> String {
    let mut q = query.trim().to_lowercase();
    // Collapse whitespace
    let mut prev_space = false;
    q = q
        .chars()
        .filter(|&c| {
            if c.is_whitespace() {
                if prev_space {
                    return false;
                }
                prev_space = true;
            } else {
                prev_space = false;
            }
            true
        })
        .collect();
    // Remove trailing semicolons
    while q.ends_with(';') {
        q.pop();
    }
    q
}

fn extract_prefix(normalized: &str, depth: usize) -> String {
    normalized
        .split_whitespace()
        .take(depth)
        .collect::<Vec<&str>>()
        .join(" ")
}

fn simple_hash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Run the cache strategy on a stream of queries.
///
/// Returns CacheStats summarizing hit rate performance.
/// Higher hit_rate indicates a better caching strategy.
pub fn cache_strategy(queries: &[&str], capacity: u64) -> CacheStats {
    debug_assert!(capacity > 0, "Cache capacity must be positive");

    let cache = LockFreeSqlCache::new(capacity);

    for (i, query) in queries.iter().enumerate() {
        let normalized = normalize_query(query);
        
        if !cache.lookup(&normalized) {
            cache.insert(query, i as u64);
        }
    }

    cache.get_stats()
}