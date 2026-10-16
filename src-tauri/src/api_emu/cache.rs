//! Small in-memory LRU cache with a time-to-live.
//!
//! InnerTube browse calls take 0.3–1.5 s and the UI asks for the same pages
//! repeatedly (artist page → similar artists → bio, album-date enrichment),
//! so api_emu keeps recent results for a few minutes. Capacities are a few
//! hundred entries, so eviction scans linearly instead of keeping a list.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Mutex;
use std::time::{Duration, Instant};

struct Entry<V> {
    value: V,
    inserted: Instant,
    last_used: Instant,
}

/// Thread-safe LRU + TTL map. Values are cloned out, so store `Arc`s for
/// anything large.
pub struct TtlCache<K, V> {
    inner: Mutex<HashMap<K, Entry<V>>>,
    capacity: usize,
    ttl: Duration,
}

impl<K: Eq + Hash + Clone, V: Clone> TtlCache<K, V> {
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        assert!(capacity > 0, "a cache needs room for one entry");
        Self {
            inner: Mutex::new(HashMap::new()),
            capacity,
            ttl,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<K, Entry<V>>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The cached value if it is younger than the TTL.
    pub fn get(&self, key: &K) -> Option<V> {
        self.get_at(key, Instant::now())
    }

    fn get_at(&self, key: &K, now: Instant) -> Option<V> {
        let mut map = self.lock();
        let expired = match map.get_mut(key) {
            None => return None,
            Some(e) if now.duration_since(e.inserted) < self.ttl => {
                e.last_used = now;
                return Some(e.value.clone());
            }
            Some(_) => true,
        };
        if expired {
            map.remove(key);
        }
        None
    }

    pub fn insert(&self, key: K, value: V) {
        self.insert_at(key, value, Instant::now());
    }

    fn insert_at(&self, key: K, value: V, now: Instant) {
        let mut map = self.lock();
        if !map.contains_key(&key) && map.len() >= self.capacity {
            // Expired entries go first; otherwise the least recently used one.
            map.retain(|_, e| now.duration_since(e.inserted) < self.ttl);
            if map.len() >= self.capacity
                && let Some(oldest) = map
                    .iter()
                    .min_by_key(|(_, e)| e.last_used)
                    .map(|(k, _)| k.clone())
            {
                map.remove(&oldest);
            }
        }
        map.insert(
            key,
            Entry {
                value,
                inserted: now,
                last_used: now,
            },
        );
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
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
    fn overwriting_does_not_evict() {
        let c = TtlCache::new(1, TTL);
        c.insert("a", 1);
        c.insert("a", 2);
        assert_eq!(c.get(&"a"), Some(2));
    }
}
