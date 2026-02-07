/// LLM SQL Cache - Initial Seed (LRU Cache with Prefix Matching)
/// ShinkaEvolve will evolve this toward optimal caching/retrieval policies.
///
/// Simple LRU cache keyed by normalized query text, with prefix lookup
/// for structurally similar queries.

use std::collections::HashMap;

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
#[derive(Debug, Clone)]
struct CacheEntry {
    query: String,
    normalized: String,
    result_id: u64,
    hit_count: u64,
}

/// LRU cache for SQL queries with prefix-based approximate matching.
pub struct LruSqlCache {
    capacity: u64,
    /// Ordered map: most recently used at end. Key = normalized query hash.
    entries: Vec<(u64, CacheEntry)>,
    /// Prefix -> list of entry keys for approximate matching.
    prefix_index: HashMap<String, Vec<u64>>,
    stats: CacheStats,
    next_key: u64,
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

    let mut cache = LruSqlCache {
        capacity,
        entries: Vec::new(),
        prefix_index: HashMap::new(),
        stats: CacheStats::default(),
        next_key: 0,
    };

    for (i, query) in queries.iter().enumerate() {
        let normalized = normalize_query(query);
        let hash = simple_hash(&normalized);

        // Exact lookup
        if let Some(pos) = cache.entries.iter().position(|(_, e)| simple_hash(&e.normalized) == hash) {
            cache.entries[pos].1.hit_count += 1;
            // Move to end (most recently used)
            let entry = cache.entries.remove(pos);
            cache.entries.push(entry);
            cache.stats.hits += 1;
            continue;
        }

        // Prefix lookup
        let prefix = extract_prefix(&normalized, 3);
        let prefix_hit = if let Some(keys) = cache.prefix_index.get(&prefix) {
            keys.iter().any(|k| cache.entries.iter().any(|(ek, _)| ek == k))
        } else {
            false
        };

        if prefix_hit {
            cache.stats.prefix_hits += 1;
            cache.stats.hits += 1;
            continue;
        }

        cache.stats.misses += 1;

        // Evict if at capacity
        if cache.entries.len() as u64 >= cache.capacity {
            if let Some((_, evicted)) = cache.entries.first() {
                let evicted_prefix = extract_prefix(&evicted.normalized, 3);
                if let Some(keys) = cache.prefix_index.get_mut(&evicted_prefix) {
                    keys.retain(|k| *k != cache.entries[0].0);
                }
            }
            cache.entries.remove(0);
            cache.stats.evictions += 1;
        }

        // Insert
        let key = cache.next_key;
        cache.next_key += 1;
        let entry = CacheEntry {
            query: query.to_string(),
            normalized: normalized.clone(),
            result_id: i as u64,
            hit_count: 0,
        };
        cache.entries.push((key, entry));

        cache
            .prefix_index
            .entry(prefix)
            .or_default()
            .push(key);
    }

    cache.stats.size = cache.entries.len() as u64;
    cache.stats
}
