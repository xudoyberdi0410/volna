//! Small in-memory LRU cache with a time-to-live and an optional byte budget.
//!
//! InnerTube browse calls take 0.3–1.5 s and the UI asks for the same pages
//! repeatedly (artist page → similar artists → bio, album-date enrichment),
//! so api_emu keeps recent results for a few minutes. Capacities are a few
//! hundred entries, so eviction scans linearly instead of keeping a list.
//! Expired entries are swept on every insert, so an idle cache does not keep
//! a whole browsing session alive until it happens to fill up.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Entry<V> {
    value: V,
    weight: usize,
    inserted: Instant,
    last_used: Instant,
}

struct Inner<K, V> {
    map: HashMap<K, Entry<V>>,
    /// Sum of `weight` over `map`.
    weight: usize,
}

/// Thread-safe LRU + TTL map. Values are cloned out, so store `Arc`s for
/// anything large.
pub struct TtlCache<K, V> {
    inner: Mutex<Inner<K, V>>,
    capacity: usize,
    ttl: Duration,
    /// Upper bound on the summed weights; `usize::MAX` for count-only caches.
    max_weight: usize,
    weigh: fn(&V) -> usize,
}

impl<K: Eq + Hash + Clone, V: Clone> TtlCache<K, V> {
    /// At most `capacity` entries, each kept for `ttl`.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self::with_budget(capacity, ttl, usize::MAX, |_| 0)
    }

    /// Like [`Self::new`], and the entries' `weigh`ts (bytes, typically)
    /// stay under `max_weight`. A value heavier than the budget is not kept.
    pub fn with_budget(
        capacity: usize,
        ttl: Duration,
        max_weight: usize,
        weigh: fn(&V) -> usize,
    ) -> Self {
        assert!(capacity > 0, "a cache needs room for one entry");
        Self {
            inner: Mutex::new(Inner {
                map: HashMap::new(),
                weight: 0,
            }),
            capacity,
            ttl,
            max_weight,
            weigh,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner<K, V>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cached value if it is younger than the TTL.
    pub fn get(&self, key: &K) -> Option<V> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&self, key: &K, now: Instant) -> Option<V> {
        let mut inner = self.lock();
        match inner.map.get_mut(key) {
            None => return None,
            Some(e) if now.duration_since(e.inserted) < self.ttl => {
                e.last_used = now;
                return Some(e.value.clone());
            }
            Some(_) => {}
        }
        inner.remove(key);
        None
    }

    pub fn insert(&self, key: K, value: V) {
        self.insert_at(key, value, Instant::now());
    }

    fn insert_at(&self, key: K, value: V, now: Instant) {
        let weight = (self.weigh)(&value);
        let mut inner = self.lock();
        inner.remove(&key);
        if weight > self.max_weight {
            return;
        }
        inner.sweep(now, self.ttl);
        while inner.map.len() >= self.capacity || inner.weight + weight > self.max_weight {
            let Some(oldest) = inner
                .map
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            inner.remove(&oldest);
        }
        inner.weight += weight;
        inner.map.insert(
            key,
            Entry {
                value,
                weight,
                inserted: now,
                last_used: now,
            },
        );
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().map.len()
    }

    #[cfg(test)]
    fn weight(&self) -> usize {
        self.lock().weight
    }
}

impl<K: Eq + Hash, V> Inner<K, V> {
    fn remove(&mut self, key: &K) {
        if let Some(e) = self.map.remove(key) {
            self.weight -= e.weight;
        }
    }

    /// Drops every entry older than `ttl`.
    fn sweep(&mut self, now: Instant, ttl: Duration) {
        let mut freed = 0;
        self.map.retain(|_, e| {
            let keep = now.duration_since(e.inserted) < ttl;
            if !keep {
                freed += e.weight;
            }
            keep
        });
        self.weight -= freed;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(60);

    #[test]
    fn returns_fresh_values() {
        let c = TtlCache::new(4, TTL);
        c.insert("a", 1);
        assert_eq!(c.get(&"a"), Some(1));
        assert_eq!(c.get(&"b"), None);
    }

    #[test]
    fn expires_after_ttl() {
        let c = TtlCache::new(4, TTL);
        let t0 = Instant::now();
        c.insert_at("a", 1, t0);
        assert_eq!(c.get_at(&"a", t0 + TTL / 2), Some(1));
        assert_eq!(c.get_at(&"a", t0 + TTL), None);
        assert_eq!(c.len(), 0, "an expired hit removes the entry");
    }

    #[test]
    fn evicts_least_recently_used() {
        let c = TtlCache::new(2, TTL);
        let t0 = Instant::now();
        c.insert_at("a", 1, t0);
        c.insert_at("b", 2, t0 + Duration::from_secs(1));
        // Touch "a" so "b" becomes the oldest.
        assert_eq!(c.get_at(&"a", t0 + Duration::from_secs(2)), Some(1));
        c.insert_at("c", 3, t0 + Duration::from_secs(3));
        assert_eq!(c.len(), 2);
        assert_eq!(c.get_at(&"b", t0 + Duration::from_secs(4)), None);
        assert_eq!(c.get_at(&"a", t0 + Duration::from_secs(4)), Some(1));
        assert_eq!(c.get_at(&"c", t0 + Duration::from_secs(4)), Some(3));
    }

    #[test]
    fn prefers_dropping_expired_entries() {
        let c = TtlCache::new(2, TTL);
        let t0 = Instant::now();
        c.insert_at("old", 1, t0);
        c.insert_at("recent", 2, t0 + TTL);
        // "old" is expired by now, "recent" is not; "recent" must survive
        // even though "old" was used more recently in wall-clock order.
        c.insert_at("new", 3, t0 + TTL + Duration::from_secs(1));
        assert_eq!(
            c.get_at(&"recent", t0 + TTL + Duration::from_secs(2)),
            Some(2)
        );
        assert_eq!(c.get_at(&"new", t0 + TTL + Duration::from_secs(2)), Some(3));
    }

    #[test]
    fn insert_sweeps_expired_entries() {
        let c = TtlCache::new(8, TTL);
        let t0 = Instant::now();
        c.insert_at("a", 1, t0);
        c.insert_at("b", 2, t0);
        c.insert_at("c", 3, t0 + TTL);
        assert_eq!(c.len(), 1, "a and b expired before c went in");
    }

    #[test]
    fn byte_budget_evicts_least_recently_used() {
        let c: TtlCache<&str, Vec<u8>> = TtlCache::with_budget(8, TTL, 10, Vec::len);
        let t0 = Instant::now();
        c.insert_at("a", vec![0; 4], t0);
        c.insert_at("b", vec![0; 4], t0 + Duration::from_secs(1));
        assert_eq!(c.weight(), 8);
        c.insert_at("c", vec![0; 4], t0 + Duration::from_secs(2));
        assert_eq!(c.weight(), 8);
        assert_eq!(c.get_at(&"a", t0 + Duration::from_secs(3)), None);
        assert!(c.get_at(&"b", t0 + Duration::from_secs(3)).is_some());
        assert!(c.get_at(&"c", t0 + Duration::from_secs(3)).is_some());
    }

    #[test]
    fn oversized_values_are_not_kept() {
        let c: TtlCache<&str, Vec<u8>> = TtlCache::with_budget(8, TTL, 10, Vec::len);
        c.insert("small", vec![0; 4]);
        c.insert("huge", vec![0; 11]);
        assert_eq!(c.get(&"huge"), None);
        assert_eq!(c.get(&"small").map(|v| v.len()), Some(4));
        assert_eq!(c.weight(), 4);
    }

    #[test]
    fn overwriting_replaces_the_weight() {
        let c: TtlCache<&str, Vec<u8>> = TtlCache::with_budget(8, TTL, 10, Vec::len);
        c.insert("a", vec![0; 6]);
        c.insert("a", vec![0; 8]);
        assert_eq!(c.weight(), 8);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn overwriting_does_not_evict() {
        let c = TtlCache::new(1, TTL);
        c.insert("a", 1);
        c.insert("a", 2);
        assert_eq!(c.get(&"a"), Some(2));
    }
}
