//! In-memory LRU cache for the redirect path and the batched last-used tracker.

use std::{collections::HashMap, num::NonZeroUsize, sync::Arc};

use chrono::{DateTime, Utc};
use lru::LruCache;
use std::sync::Mutex;

/// What a redirect needs to know about a link.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub link_id: i64,
    pub target: String,
    pub enabled: bool,
}

/// slug → resolved link, or `None` for a cached miss.
///
/// The database is the source of truth. Writes are rare (one admin), so every
/// write simply clears the cache; that keeps invalidation trivially correct.
type Entries = LruCache<String, Option<Arc<Resolved>>>;

pub struct LinkCache {
    inner: Mutex<Entries>,
}

impl LinkCache {
    pub fn new(capacity: usize) -> Self {
        let cap = NonZeroUsize::new(capacity.max(1)).expect("capacity >= 1");
        Self {
            inner: Mutex::new(LruCache::new(cap)),
        }
    }

    /// `None` = not cached. `Some(None)` = cached "no such link".
    pub fn get(&self, slug: &str) -> Option<Option<Arc<Resolved>>> {
        let mut c = self.inner.lock().expect("cache lock");
        c.get(slug).cloned()
    }

    pub fn put(&self, slug: &str, value: Option<Resolved>) {
        let mut c = self.inner.lock().expect("cache lock");
        c.put(slug.to_string(), value.map(Arc::new));
    }

    pub fn clear(&self) {
        self.inner.lock().expect("cache lock").clear();
    }

    pub fn len(&self) -> usize {
        self.inner.lock().expect("cache lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Last-used timestamps collected on the redirect path and written to the
/// database in batches, never in the request path.
#[derive(Default)]
pub struct LastUsed {
    pending: Mutex<HashMap<i64, DateTime<Utc>>>,
}

impl LastUsed {
    pub fn record(&self, link_id: i64, at: DateTime<Utc>) {
        self.pending.lock().expect("last-used lock").insert(link_id, at);
    }

    /// Takes everything pending, leaving the tracker empty.
    pub fn take(&self) -> Vec<(i64, DateTime<Utc>)> {
        let mut map = self.pending.lock().expect("last-used lock");
        let mut out: Vec<_> = map.drain().collect();
        out.sort_by_key(|(id, _)| *id);
        out
    }

    /// Puts entries back after a failed flush, keeping newer values.
    pub fn restore(&self, entries: Vec<(i64, DateTime<Utc>)>) {
        let mut map = self.pending.lock().expect("last-used lock");
        for (id, at) in entries {
            map.entry(id).and_modify(|cur| *cur = (*cur).max(at)).or_insert(at);
        }
    }

    pub fn len(&self) -> usize {
        self.pending.lock().expect("last-used lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
