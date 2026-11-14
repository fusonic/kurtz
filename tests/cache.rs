use chrono::{Duration, Utc};
use kurtz::cache::{LastUsed, LinkCache, Resolved};

fn r(id: i64) -> Option<Resolved> {
    Some(Resolved {
        link_id: id,
        target: format!("https://t/{id}"),
        enabled: true,
    })
}

#[test]
fn lru_evicts_least_recently_used() {
    let c = LinkCache::new(2);
    c.put("x", r(1));
    c.put("y", r(2));
    assert!(c.get("x").is_some()); // x is now most recent
    c.put("z", r(3));
    assert!(c.get("y").is_none(), "y should be evicted");
    assert!(c.get("x").is_some());
    assert!(c.get("z").is_some());
}

#[test]
fn cache_distinguishes_hits_and_misses() {
    let c = LinkCache::new(10);
    c.put("x", r(1));
    c.put("y", None);
    assert_eq!(c.get("x").unwrap().unwrap().link_id, 1);
    assert_eq!(c.get("y"), Some(None));
    assert_eq!(c.get("z"), None);
    c.clear();
    assert!(c.is_empty());
}

#[test]
fn last_used_keeps_latest_and_restores() {
    let l = LastUsed::default();
    let t0 = Utc::now();
    l.record(1, t0);
    l.record(1, t0 + Duration::seconds(5));
    l.record(2, t0);
    let taken = l.take();
    assert_eq!(taken.len(), 2);
    assert_eq!(taken[0], (1, t0 + Duration::seconds(5)));
    assert!(l.is_empty());
    l.record(1, t0 + Duration::seconds(10));
    l.restore(taken);
    assert_eq!(l.take()[0], (1, t0 + Duration::seconds(10)));
}
