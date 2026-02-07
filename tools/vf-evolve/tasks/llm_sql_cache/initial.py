"""
LLM SQL Cache - Initial Seed (LRU Cache with Prefix Matching)
ShinkaEvolve will evolve this toward optimal caching/retrieval policies.

Simple LRU cache that stores SQL query results keyed by normalized query text,
with prefix-based lookup for similar queries.
"""

from collections import OrderedDict
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Tuple
import hashlib
import re


@dataclass
class CacheEntry:
    """A cached SQL query result."""
    query: str
    normalized: str
    result: Any
    hit_count: int = 0
    creation_time: float = 0.0


@dataclass
class CacheStats:
    """Cache performance statistics."""
    hits: int = 0
    misses: int = 0
    prefix_hits: int = 0
    evictions: int = 0
    size: int = 0

    @property
    def hit_rate(self) -> float:
        total = self.hits + self.misses
        return self.hits / total if total > 0 else 0.0


class LruSqlCache:
    """LRU cache with prefix matching for SQL queries."""

    def __init__(self, capacity: int = 1000):
        assert capacity > 0, "capacity must be positive"
        self.capacity = capacity
        self.cache: OrderedDict[str, CacheEntry] = OrderedDict()
        self.prefix_index: Dict[str, List[str]] = {}  # prefix -> [cache_keys]
        self.stats = CacheStats()

    @staticmethod
    def normalize_query(query: str) -> str:
        """Normalize SQL query for cache key generation."""
        q = query.strip().lower()
        # Collapse whitespace
        q = re.sub(r'\s+', ' ', q)
        # Remove trailing semicolons
        q = q.rstrip(';')
        # Normalize literal values to placeholders for prefix matching
        q = re.sub(r"'[^']*'", "'?'", q)
        q = re.sub(r'\b\d+\b', '?', q)
        return q

    @staticmethod
    def _extract_prefix(normalized: str, depth: int = 3) -> str:
        """Extract a prefix from normalized query (first N tokens)."""
        tokens = normalized.split()
        prefix_tokens = tokens[:depth]
        return ' '.join(prefix_tokens)

    def _cache_key(self, normalized: str) -> str:
        """Generate a cache key from normalized query."""
        return hashlib.md5(normalized.encode()).hexdigest()

    def get(self, query: str) -> Optional[Any]:
        """Look up a query in cache. Returns result or None."""
        normalized = self.normalize_query(query)
        key = self._cache_key(normalized)

        if key in self.cache:
            # Move to end (most recently used)
            self.cache.move_to_end(key)
            entry = self.cache[key]
            entry.hit_count += 1
            self.stats.hits += 1
            return entry.result

        self.stats.misses += 1
        return None

    def get_prefix_match(self, query: str) -> Optional[Tuple[str, Any]]:
        """Find a cached result with matching prefix (approximate match).
        Returns (original_query, result) or None.
        """
        normalized = self.normalize_query(query)
        prefix = self._extract_prefix(normalized)

        if prefix in self.prefix_index:
            for key in self.prefix_index[prefix]:
                if key in self.cache:
                    self.cache.move_to_end(key)
                    entry = self.cache[key]
                    entry.hit_count += 1
                    self.stats.prefix_hits += 1
                    return (entry.query, entry.result)

        return None

    def put(self, query: str, result: Any, timestamp: float = 0.0) -> None:
        """Insert a query result into the cache."""
        normalized = self.normalize_query(query)
        key = self._cache_key(normalized)
        prefix = self._extract_prefix(normalized)

        # Evict if at capacity
        if key not in self.cache and len(self.cache) >= self.capacity:
            self._evict()

        # Insert or update
        if key in self.cache:
            self.cache.move_to_end(key)
            self.cache[key].result = result
        else:
            entry = CacheEntry(
                query=query,
                normalized=normalized,
                result=result,
                creation_time=timestamp,
            )
            self.cache[key] = entry

            # Update prefix index
            if prefix not in self.prefix_index:
                self.prefix_index[prefix] = []
            self.prefix_index[prefix].append(key)

        self.stats.size = len(self.cache)

    def _evict(self) -> None:
        """Evict the least recently used entry."""
        if not self.cache:
            return
        key, entry = self.cache.popitem(last=False)  # Remove oldest
        prefix = self._extract_prefix(entry.normalized)
        if prefix in self.prefix_index:
            self.prefix_index[prefix] = [
                k for k in self.prefix_index[prefix] if k != key
            ]
            if not self.prefix_index[prefix]:
                del self.prefix_index[prefix]
        self.stats.evictions += 1
        self.stats.size = len(self.cache)

    def clear(self) -> None:
        """Clear all cached entries."""
        self.cache.clear()
        self.prefix_index.clear()
        self.stats.size = 0

    def get_stats(self) -> CacheStats:
        """Return current cache statistics."""
        return self.stats


def cache_strategy(queries, memory_budget, distinct_prefixes):
    """Adapter for ShinkaEvolve simulator interface.

    Args:
        queries: list of query strings.
        memory_budget: memory budget in GB.
        distinct_prefixes: number of distinct prefixes.

    Returns:
        dict with hit_rate.
    """
    cache = LruSqlCache(capacity=min(distinct_prefixes, memory_budget * 100))
    hits = 0
    for i, query in enumerate(queries):
        if cache.get(query) is not None:
            hits += 1
        else:
            cache.put(query, i)
    return {"hit_rate": hits / len(queries) if queries else 0.0}


if __name__ == "__main__":
    cache = LruSqlCache(capacity=5)

    # Store some queries
    cache.put("SELECT count(*) FROM logs WHERE service='nginx'", 42)
    cache.put("SELECT count(*) FROM logs WHERE service='redis'", 17)
    cache.put("SELECT avg(duration) FROM spans WHERE service='api'", 0.23)

    # Exact hit
    result = cache.get("SELECT count(*) FROM logs WHERE service='nginx'")
    print(f"Exact hit: {result}")  # 42

    # Miss
    result = cache.get("SELECT max(duration) FROM spans")
    print(f"Miss: {result}")  # None

    # Prefix match (different literal value but same structure)
    match = cache.get_prefix_match("SELECT count(*) FROM logs WHERE env='prod'")
    if match:
        print(f"Prefix match: query='{match[0]}' result={match[1]}")

    stats = cache.get_stats()
    print(f"\nStats: hits={stats.hits}, misses={stats.misses}, "
          f"prefix_hits={stats.prefix_hits}, hit_rate={stats.hit_rate:.2%}")
