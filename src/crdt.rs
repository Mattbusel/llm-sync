// SPDX-License-Identifier: MIT
//! CRDT implementations: GCounter, PNCounter, GSet, LWWRegister, ORMap.
//!
//! Every merge operation is commutative, associative, and idempotent.
//! No merge operation ever blocks.

use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};

// ── GCounter (grow-only counter) ─────────────────────────────────────────────

/// A grow-only counter CRDT.
///
/// Per-node counters are merged by taking the component-wise maximum.
/// The aggregate value is the sum of all per-node counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GCounter {
    counts: HashMap<String, u64>,
}

impl GCounter {
    /// Create a new, empty GCounter.
    pub fn new() -> Self { Self::default() }

    /// Increment this node's counter by `amount`.
    ///
    /// # Arguments
    /// * `node` — Identifier of the node performing the increment.
    /// * `amount` — Value to add (must be positive for growth semantics).
    pub fn increment(&mut self, node: impl Into<String>, amount: u64) {
        *self.counts.entry(node.into()).or_insert(0) += amount;
    }

    /// Return the aggregate value across all nodes.
    pub fn value(&self) -> u64 { self.counts.values().sum() }

    /// Merge with another GCounter, taking the component-wise maximum.
    ///
    /// The operation is commutative, associative, and idempotent.
    pub fn merge(&self, other: &GCounter) -> GCounter {
        let mut result = self.counts.clone();
        for (k, &v) in &other.counts {
            let e = result.entry(k.clone()).or_insert(0);
            if v > *e { *e = v; }
        }
        GCounter { counts: result }
    }
}

// ── PNCounter (increment/decrement counter) ──────────────────────────────────

/// A positive-negative counter CRDT that supports both increment and decrement.
///
/// Internally composed of two GCounters: one for increments, one for decrements.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PNCounter {
    increments: GCounter,
    decrements: GCounter,
}

impl PNCounter {
    /// Create a new, zeroed PNCounter.
    pub fn new() -> Self { Self::default() }

    /// Increment by `amount` for `node`.
    pub fn increment(&mut self, node: impl Into<String>, amount: u64) {
        self.increments.increment(node, amount);
    }

    /// Decrement by `amount` for `node`.
    pub fn decrement(&mut self, node: impl Into<String>, amount: u64) {
        self.decrements.increment(node, amount);
    }

    /// Return the net value (increments − decrements).
    pub fn value(&self) -> i64 {
        self.increments.value() as i64 - self.decrements.value() as i64
    }

    /// Merge with another PNCounter.
    pub fn merge(&self, other: &PNCounter) -> PNCounter {
        PNCounter {
            increments: self.increments.merge(&other.increments),
            decrements: self.decrements.merge(&other.decrements),
        }
    }
}

// ── GSet (grow-only set) ─────────────────────────────────────────────────────

/// A grow-only set CRDT. Elements can be added but never removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct GSet {
    items: HashSet<String>,
}

impl GSet {
    /// Create a new, empty GSet.
    pub fn new() -> Self { Self::default() }

    /// Insert an element.
    pub fn insert(&mut self, item: impl Into<String>) { self.items.insert(item.into()); }

    /// Return true if the element is present.
    pub fn contains(&self, item: &str) -> bool { self.items.contains(item) }

    /// Return the number of elements.
    pub fn len(&self) -> usize { self.items.len() }

    /// Return true if the set is empty.
    pub fn is_empty(&self) -> bool { self.items.is_empty() }

    /// Merge with another GSet (union).
    pub fn merge(&self, other: &GSet) -> GSet {
        GSet { items: self.items.union(&other.items).cloned().collect() }
    }

    /// Iterate over elements.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|s| s.as_str())
    }
}

// ── LWWRegister (last-write-wins register) ───────────────────────────────────

/// A last-write-wins register CRDT.
///
/// Writes with a higher logical timestamp overwrite earlier writes.
/// On merge, the replica with the higher timestamp wins.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(deserialize = "T: serde::de::DeserializeOwned"))]
pub struct LWWRegister<T: Clone + Serialize + serde::de::DeserializeOwned> {
    value: Option<T>,
    timestamp: u64,
    writer: String,
}

impl<T: Clone + Serialize + serde::de::DeserializeOwned> LWWRegister<T> {
    /// Create a new, empty register.
    pub fn new() -> Self {
        Self { value: None, timestamp: 0, writer: String::new() }
    }

    /// Write a value with a logical timestamp.
    ///
    /// # Arguments
    /// * `value` — The value to write.
    /// * `timestamp` — Logical timestamp; higher values win.
    /// * `writer` — Identifier of the writing node.
    pub fn write(&mut self, value: T, timestamp: u64, writer: impl Into<String>) {
        if timestamp >= self.timestamp {
            self.value = Some(value);
            self.timestamp = timestamp;
            self.writer = writer.into();
        }
    }

    /// Read the current value. Returns `None` if never written.
    pub fn read(&self) -> Option<&T> { self.value.as_ref() }

    /// Merge with another register. The one with the higher timestamp wins.
    pub fn merge(&self, other: &LWWRegister<T>) -> LWWRegister<T> {
        if other.timestamp > self.timestamp {
            other.clone()
        } else {
            self.clone()
        }
    }

    /// Return the current logical timestamp.
    pub fn timestamp(&self) -> u64 { self.timestamp }
}

impl<T: Clone + Serialize + serde::de::DeserializeOwned> Default for LWWRegister<T> {
    fn default() -> Self { Self::new() }
}

// ── ORMap (observed-remove map, LWW-semantics) ───────────────────────────────

/// An observed-remove map CRDT backed by per-key LWWRegisters.
///
/// Keys can be added and updated. Deletion can be modeled via tombstone
/// conventions at the application layer (not enforced here).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ORMap {
    entries: HashMap<String, LWWRegister<String>>,
}

impl ORMap {
    /// Create a new, empty ORMap.
    pub fn new() -> Self { Self::default() }

    /// Set a key to a value at the given logical timestamp.
    ///
    /// # Arguments
    /// * `key` — The map key.
    /// * `value` — The string value to store.
    /// * `timestamp` — Logical timestamp for LWW resolution.
    /// * `writer` — Identifier of the writing agent.
    pub fn set(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        timestamp: u64,
        writer: impl Into<String>,
    ) {
        let k = key.into();
        self.entries.entry(k).or_default().write(value.into(), timestamp, writer);
    }

    /// Get a value by key. Returns `None` if not set.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key)?.read().map(|s| s.as_str())
    }

    /// Merge with another ORMap.
    pub fn merge(&self, other: &ORMap) -> ORMap {
        let mut result = self.entries.clone();
        for (k, reg) in &other.entries {
            let entry = result.entry(k.clone()).or_default();
            *entry = entry.merge(reg);
        }
        ORMap { entries: result }
    }

    /// Return the number of keys.
    pub fn len(&self) -> usize { self.entries.len() }

    /// Return true if the map is empty.
    pub fn is_empty(&self) -> bool { self.entries.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── GCounter ──────────────────────────────────────────────────────────────

    #[test]
    fn test_gcounter_increment_and_value() {
        let mut c = GCounter::new();
        c.increment("a", 3);
        c.increment("b", 2);
        assert_eq!(c.value(), 5);
    }

    #[test]
    fn test_gcounter_merge_is_commutative() {
        let mut c1 = GCounter::new(); c1.increment("a", 5);
        let mut c2 = GCounter::new(); c2.increment("b", 3);
        assert_eq!(c1.merge(&c2).value(), c2.merge(&c1).value());
    }

    #[test]
    fn test_gcounter_merge_is_idempotent() {
        let mut c = GCounter::new(); c.increment("a", 10);
        assert_eq!(c.merge(&c.clone()).value(), c.value());
    }

    #[test]
    fn test_gcounter_merge_is_associative() {
        let mut a = GCounter::new(); a.increment("x", 1);
        let mut b = GCounter::new(); b.increment("y", 2);
        let mut c = GCounter::new(); c.increment("z", 3);
        assert_eq!(a.merge(&b).merge(&c).value(), a.merge(&b.merge(&c)).value());
    }

    #[test]
    fn test_gcounter_merge_takes_max_per_node() {
        let mut c1 = GCounter::new(); c1.increment("a", 10);
        let mut c2 = GCounter::new(); c2.increment("a", 5);
        assert_eq!(c1.merge(&c2).value(), 10);
    }

    #[test]
    fn test_gcounter_new_is_zero() {
        assert_eq!(GCounter::new().value(), 0);
    }

    // ── PNCounter ─────────────────────────────────────────────────────────────

    #[test]
    fn test_pncounter_increment_decrement() {
        let mut c = PNCounter::new();
        c.increment("a", 10);
        c.decrement("a", 3);
        assert_eq!(c.value(), 7);
    }

    #[test]
    fn test_pncounter_merge_is_commutative() {
        let mut c1 = PNCounter::new(); c1.increment("a", 5);
        let mut c2 = PNCounter::new(); c2.decrement("b", 2);
        assert_eq!(c1.merge(&c2).value(), c2.merge(&c1).value());
    }

    #[test]
    fn test_pncounter_merge_is_idempotent() {
        let mut c = PNCounter::new(); c.increment("x", 7);
        assert_eq!(c.merge(&c.clone()).value(), c.value());
    }

    #[test]
    fn test_pncounter_new_is_zero() {
        assert_eq!(PNCounter::new().value(), 0);
    }

    #[test]
    fn test_pncounter_can_go_negative() {
        let mut c = PNCounter::new();
        c.decrement("a", 5);
        assert_eq!(c.value(), -5);
    }

    // ── GSet ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_gset_insert_and_contains() {
        let mut s = GSet::new();
        s.insert("alpha");
        assert!(s.contains("alpha"));
        assert!(!s.contains("beta"));
    }

    #[test]
    fn test_gset_merge_is_commutative() {
        let mut s1 = GSet::new(); s1.insert("a");
        let mut s2 = GSet::new(); s2.insert("b");
        assert_eq!(s1.merge(&s2).len(), s2.merge(&s1).len());
    }

    #[test]
    fn test_gset_merge_is_idempotent() {
        let mut s = GSet::new(); s.insert("x");
        assert_eq!(s.merge(&s.clone()).len(), s.len());
    }

    #[test]
    fn test_gset_merge_union() {
        let mut s1 = GSet::new(); s1.insert("a"); s1.insert("b");
        let mut s2 = GSet::new(); s2.insert("b"); s2.insert("c");
        assert_eq!(s1.merge(&s2).len(), 3);
    }

    #[test]
    fn test_gset_is_empty_on_new() {
        assert!(GSet::new().is_empty());
    }

    #[test]
    fn test_gset_iter_yields_all_elements() {
        let mut s = GSet::new(); s.insert("a"); s.insert("b");
        let mut items: Vec<&str> = s.iter().collect();
        items.sort();
        assert_eq!(items, vec!["a", "b"]);
    }

    // ── LWWRegister ───────────────────────────────────────────────────────────

    #[test]
    fn test_lww_register_write_and_read() {
        let mut r: LWWRegister<String> = LWWRegister::new();
        r.write("hello".into(), 1, "agent-1");
        assert_eq!(r.read().unwrap(), "hello");
    }

    #[test]
    fn test_lww_register_higher_timestamp_wins() {
        let mut r: LWWRegister<String> = LWWRegister::new();
        r.write("first".into(), 1, "a");
        r.write("second".into(), 2, "b");
        assert_eq!(r.read().unwrap(), "second");
    }

    #[test]
    fn test_lww_register_lower_timestamp_ignored() {
        let mut r: LWWRegister<String> = LWWRegister::new();
        r.write("latest".into(), 10, "a");
        r.write("old".into(), 5, "b");
        assert_eq!(r.read().unwrap(), "latest");
    }

    #[test]
    fn test_lww_register_merge_picks_higher_ts() {
        let mut r1: LWWRegister<String> = LWWRegister::new();
        r1.write("old".into(), 1, "a");
        let mut r2: LWWRegister<String> = LWWRegister::new();
        r2.write("new".into(), 5, "b");
        assert_eq!(r1.merge(&r2).read().unwrap(), "new");
    }

    #[test]
    fn test_lww_register_merge_is_commutative() {
        let mut r1: LWWRegister<String> = LWWRegister::new();
        r1.write("v1".into(), 3, "a");
        let mut r2: LWWRegister<String> = LWWRegister::new();
        r2.write("v2".into(), 7, "b");
        assert_eq!(r1.merge(&r2).read(), r2.merge(&r1).read());
    }

    #[test]
    fn test_lww_register_new_is_empty() {
        let r: LWWRegister<String> = LWWRegister::new();
        assert!(r.read().is_none());
        assert_eq!(r.timestamp(), 0);
    }

    // ── ORMap ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_ormap_set_and_get() {
        let mut m = ORMap::new();
        m.set("key", "value", 1, "a");
        assert_eq!(m.get("key").unwrap(), "value");
    }

    #[test]
    fn test_ormap_get_missing_key_returns_none() {
        let m = ORMap::new();
        assert!(m.get("absent").is_none());
    }

    #[test]
    fn test_ormap_merge_lww_semantics() {
        let mut m1 = ORMap::new(); m1.set("k", "v1", 1, "a");
        let mut m2 = ORMap::new(); m2.set("k", "v2", 2, "b");
        assert_eq!(m1.merge(&m2).get("k").unwrap(), "v2");
    }

    #[test]
    fn test_ormap_merge_is_commutative() {
        let mut m1 = ORMap::new(); m1.set("x", "val1", 3, "a");
        let mut m2 = ORMap::new(); m2.set("y", "val2", 1, "b");
        let merged1 = m1.merge(&m2);
        let merged2 = m2.merge(&m1);
        assert_eq!(merged1.get("x"), merged2.get("x"));
        assert_eq!(merged1.get("y"), merged2.get("y"));
    }

    #[test]
    fn test_ormap_merge_is_idempotent() {
        let mut m = ORMap::new(); m.set("k", "v", 5, "a");
        let m2 = m.merge(&m.clone());
        assert_eq!(m2.get("k"), m.get("k"));
    }

    #[test]
    fn test_ormap_is_empty_on_new() {
        assert!(ORMap::new().is_empty());
    }

    #[test]
    fn test_ormap_len_counts_unique_keys() {
        let mut m = ORMap::new();
        m.set("a", "1", 1, "x");
        m.set("b", "2", 2, "x");
        assert_eq!(m.len(), 2);
    }
}
